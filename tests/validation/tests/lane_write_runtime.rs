// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent AMD RDNA2 ISA 70648, section12.12 p184 expectations:
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! WRITELANE selects S1 modulo wave size, ignores EXEC, and preserves other lanes.
//! Full-wave admission of captured selectors is a project contract. No production
//! evaluator supplies expected values; these tests are not AMD hardware evidence.
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const WAIT: u32 = 0xbf8c3f70;
fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn write(data: u32) -> [u32; 2] {
    [0xd7610001, 0x02001200 | data]
}
fn read(dst: u32, src: u32, index: u32) -> [u32; 2] {
    [0xd7600000 | dst, 0x02000100 | index << 9 | src]
}
fn scalar(w: &mut Vec<u32>, dst: u32, value: u32) {
    w.extend([0xbe8003ff | dst << 16, value]);
}
fn exec(w: &mut Vec<u32>, size: usize, mask: u64) {
    scalar(w, 126, mask as u32);
    if size == 64 {
        scalar(w, 127, (mask >> 32) as u32);
    }
}
fn finish(mut w: Vec<u32>, dst: u32) -> Vec<u8> {
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
fn compile(
    code: &[u8],
    size: usize,
) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    xenora_shader::compile_with_wave_config(
        xenora_shader::Architecture::Gfx1030,
        code,
        xenora_shader::WaveConfig {
            size: if size == 64 {
                xenora_shader::WaveSize::Wave64
            } else {
                xenora_shader::WaveSize::Wave32
            },
            launch: xenora_shader::WaveLaunch::FullWavesRequired,
        },
    )
}
fn reference(
    code: &[u8],
    data: &[u32],
    size: usize,
) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_wave_config(
        code,
        data,
        xenora_ref::WaveConfig {
            size: if size == 64 {
                xenora_ref::WaveSize::Wave64
            } else {
                xenora_ref::WaveSize::Wave32
            },
            launch: xenora_ref::WaveLaunch::FullWavesRequired,
        },
    )
}
struct Case {
    code: Vec<u8>,
    data: Vec<u32>,
    expected: Vec<u32>,
    size: usize,
}
fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for size in [32, 64] {
        for use_readlane in [false, true] {
            for mask in [0, 0x8000000100000002, u64::MAX] {
                for mode in 0..4 {
                    let mut data: Vec<u32> =
                        (0..size * 3).map(|i| 0x80001000 + i as u32 * 17).collect();
                    let selectors = [size as u32 - 1, size as u32, u32::MAX];
                    for (wave, selector) in data.chunks_mut(size).zip(selectors) {
                        wave[usize::from(use_readlane)] = selector;
                    }
                    let mut w = LOAD.to_vec();
                    w.extend([WAIT, 0x7e040301, 0x7e080301]); // v2 and v4 snapshot input
                    if use_readlane {
                        scalar(&mut w, 10, 1);
                        w.extend(read(9, 2, 10));
                    } else {
                        w.push(0x7e120502);
                    } // READFIRST s9,v2
                    scalar(&mut w, 10, 2);
                    w.extend(read(8, 2, 10)); // source data captured independently from lane2
                    w.push(0x7e040287); // overwrite original capture source
                    exec(&mut w, size, mask);
                    w.extend(write(if mode == 1 { 9 } else { 8 })); // data/index SGPR alias
                    exec(&mut w, size, u64::MAX);
                    let mut written = data.clone();
                    for (wave, selector) in written.chunks_mut(size).zip(selectors) {
                        let value = if mode == 1 { selector } else { wave[2] };
                        wave[selector as usize % size] = value;
                    }
                    let (dst, expected) = match mode {
                        2 => {
                            w.extend(read(11, 1, 9)); // nested read replays runtime write
                            scalar(&mut w, 8, 13);
                            scalar(&mut w, 9, 0);
                            w.push(0x7e020289); // overwrite destination after nested capture
                            w.push(0x7e06020b);
                            (
                                3,
                                data.chunks(size).flat_map(|v| vec![v[2]; size]).collect(),
                            )
                        }
                        3 => {
                            w.extend([0x7d840901, 0x7e06026a]); // EQ v1,v4 -> low VCC snapshot
                            let expected = written
                                .chunks(size)
                                .zip(data.chunks(size))
                                .flat_map(|(a, b)| {
                                    let mut bits = 0u32;
                                    for lane in 0..32 {
                                        if a[lane] == b[lane] {
                                            bits |= 1 << lane;
                                        }
                                    }
                                    vec![bits; size]
                                })
                                .collect();
                            (3, expected)
                        }
                        _ => (1, written),
                    };
                    out.push(Case {
                        code: finish(w, dst),
                        data,
                        expected,
                        size,
                    });
                }
            }
        }
    }
    out
}
#[test]
fn captured_selectors_snapshots_aliases_and_replay() {
    assert_eq!(write(9), [0xd7610001, 0x02001209]);
    assert_eq!(read(9, 2, 10), [0xd7600009, 0x02001502]);
    let cases = cases();
    assert_eq!(cases.len(), 48);
    for (i, case) in cases.into_iter().enumerate() {
        compile(&case.code, case.size).unwrap_or_else(|e| panic!("case{i}: {e:?}"));
        assert_eq!(
            reference(&case.code, &case.data, case.size).unwrap(),
            case.expected,
            "case{i}"
        );
    }
}
#[test]
fn partial_launch_and_default_contract_still_reject_runtime_selectors() {
    for size in [32, 64] {
        let case = cases().into_iter().find(|c| c.size == size).unwrap();
        let shader = compile(&case.code, size).unwrap();
        for len in [0, 1, size - 1, size + 1, 4097] {
            assert!(shader.validate_launch(len).is_err());
            assert!(reference(&case.code, &vec![0; len], size).is_err());
        }
    }
    let case = cases().remove(0);
    assert!(xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &case.code).is_err());
    assert!(xenora_ref::execute(&case.code, &case.data).is_err());
}
#[test]
fn unknown_pending_and_runtime_region_rejections() {
    for size in [32, 64] {
        let mut programs = Vec::new();
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe88038d]); // selector never initialized
        w.extend(write(8));
        programs.push(finish(w, 1));
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0x7e120501]); // data never initialized
        w.extend(write(8));
        programs.push(finish(w, 1));
        let mut w = LOAD.to_vec();
        w.extend([0x7e120501, WAIT, 0xbe88038d]); // capture before load wait
        w.extend(write(8));
        programs.push(finish(w, 1));
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0x7e120501, 0xbe88038d]);
        w.extend(LOAD); // writing old destination while second load pending
        w.extend(write(8));
        w.push(WAIT);
        programs.push(finish(w, 1));
        for predicate in [0x7d840280, 0x7d850280] {
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0x7e120501, 0xbe88038d, predicate, 0xbf870002]);
            w.extend(write(8)); // forbidden lane transfer in runtime branch region
            programs.push(finish(w, 1));
        }
        for (i, code) in programs.iter().enumerate() {
            assert!(compile(code, size).is_err(), "size{size} case{i}");
            assert!(
                reference(code, &vec![0; size], size).is_err(),
                "size{size} case{i}"
            );
        }
    }
}
#[test]
fn graphics_rejects_runtime_lane_write_in_both_stages() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    xenora_shader::compile_graphics(xenora_shader::Architecture::Gfx1030, vs, ps).unwrap();
    let prefix = bytes(&[0x7e120502, 0xd7610001, 0x02001209]);
    for vertex in [false, true] {
        let mut code = prefix.clone();
        code.extend_from_slice(if vertex { vs } else { ps });
        assert!(
            xenora_shader::compile_graphics(
                xenora_shader::Architecture::Gfx1030,
                if vertex { &code } else { vs },
                if vertex { ps } else { &code }
            )
            .is_err()
        );
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_runtime_lane_write_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    let cases = cases();
    assert_eq!(cases.len(), 48);
    for (i, case) in cases.into_iter().enumerate() {
        let shader = compile(&case.code, case.size).unwrap();
        assert_eq!(
            gpu.run_compiled_words(&shader, &case.data).unwrap(),
            case.expected,
            "case{i}"
        );
    }
    eprintln!("runtime WRITELANE: 48 independent native dispatches");
}
