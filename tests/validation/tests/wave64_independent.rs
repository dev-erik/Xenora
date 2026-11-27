// SPDX-License-Identifier: GPL-3.0-or-later
//! Independently authored Wave64 expectations, public AMD RDNA2 ISA 70648:
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! Sections6.2.4(p44),10.4.4(pp96-97), instruction tables pp137,183-187,205-209.
//! Specific DS half-wave restrictions govern generic modulo64 opcode pseudocode.
//! DS offsets below are eight-bit, avoiding the separate high-offset ambiguity.
//! SGPR read/write overlap is rejected in Wave64, without an invented READLANE
//! exemption. e64 READFIRST zero padding is a project subset, not LLVM evidence.
//! Collision highest-writer results are project policy; not an AMD hardware claim.
//! No production helper computes expected outputs. Native tests are lead-run only.

const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const VM: u32 = 0xbf8c3f70;
const LGKM: u32 = 0xbf8cc07f;
const SENTINEL: u32 = 0xa5a55a5a;
struct Case {
    name: String,
    code: Vec<u8>,
    input: Vec<u32>,
    expected: Vec<u32>,
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| [0x80001000, 0xfffffe00, 0x7fffff00][i / 64] + (i % 64) as u32)
        .collect()
}
fn start() -> Vec<u32> {
    vec![LOAD[0], LOAD[1], VM]
}
fn finish(mut w: Vec<u32>, dst: u32) -> Vec<u8> {
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn scalar(w: &mut Vec<u32>, dst: u32, value: u32) {
    w.extend([0xbe8003ff | dst << 16, value]);
}
fn exec(w: &mut Vec<u32>, mask: u64) {
    scalar(w, 126, mask as u32);
    scalar(w, 127, (mask >> 32) as u32);
}
fn readlane(dst: u32, src: u32, selector: u32) -> [u32; 2] {
    [0xd7600000 | dst, 0x02000100 | selector << 9 | src]
}
fn broadcast(data: &[u32], lane: usize) -> Vec<u32> {
    data.as_chunks::<64>()
        .0
        .iter()
        .flat_map(|wave| vec![wave[lane]; 64])
        .collect()
}
fn push(
    out: &mut Vec<Case>,
    name: impl Into<String>,
    w: Vec<u32>,
    dst: u32,
    data: Vec<u32>,
    expected: Vec<u32>,
) {
    out.push(Case {
        name: name.into(),
        code: finish(w, dst),
        input: data,
        expected,
    });
}
fn lane_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for (n, lane) in [0, 1, 15, 16, 30, 31, 32, 33, 47, 48, 62, 63]
        .into_iter()
        .enumerate()
    {
        for wrapped in [false, true] {
            let data = input([64, 128, 192][(n + usize::from(wrapped)) % 3]);
            let mut w = start();
            let selector = if wrapped {
                if lane == 63 {
                    u32::MAX
                } else {
                    lane as u32 + 64
                }
            } else {
                lane as u32
            };
            scalar(&mut w, 9, selector);
            // Even inactive selected lanes retain initialized input snapshots.
            if wrapped {
                exec(&mut w, 0);
            }
            w.extend(readlane(8, 1, 9));
            exec(&mut w, u64::MAX);
            w.extend([0x7e020280, 0x7e060208]);
            let expected = broadcast(&data, lane);
            push(
                &mut out,
                format!("readlane {selector:x}"),
                w,
                3,
                data,
                expected,
            );
        }
    }
    for (n, (mask, lane)) in [
        (0, 0),
        (1, 0),
        (1u64 << 31, 31),
        (1u64 << 32, 32),
        (1u64 << 63, 63),
        ((1u64 << 32) | (1u64 << 63), 32),
        ((1u64 << 31) | (1u64 << 32), 31),
        (u64::MAX, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let data = input([64, 128, 192][n % 3]);
        let mut w = start();
        exec(&mut w, mask);
        if n % 2 == 0 {
            w.push(0x7e100501);
        } else {
            w.extend([0xd5820008, 0x00000101]);
        }
        exec(&mut w, u64::MAX);
        w.extend([0x7e020280, 0x7e060208]);
        let expected = broadcast(&data, lane);
        push(
            &mut out,
            format!("readfirst exec{mask:016x}"),
            w,
            3,
            data,
            expected,
        );
    }
    for len in [64, 128, 192] {
        for after in [false, true] {
            let data = input(len);
            let mut w = start();
            scalar(&mut w, 9, 63);
            w.extend(readlane(8, 1, 9));
            scalar(&mut w, 9, 32);
            w.extend(readlane(10, 1, 9));
            exec(&mut w, 0);
            w.extend([0xd7610001, 0x02001208]); // write captured lane63 into lane32, ignores EXEC
            w.extend(readlane(11, 1, 9));
            exec(&mut w, u64::MAX);
            scalar(&mut w, 8, 13);
            w.extend([0x7e020280, if after { 0x7e06020b } else { 0x7e06020a }]);
            let expected = broadcast(&data, if after { 63 } else { 32 });
            push(
                &mut out,
                format!("capture-write length{len} after{after}"),
                w,
                3,
                data,
                expected,
            );
        }
    }
    for first in [false, true] {
        let data = input(192);
        let mut w = start();
        exec(&mut w, 1u64 << 32);
        w.push(0x7e020289); // only lane32 becomes9
        exec(&mut w, 0);
        if first {
            w.push(0x7e100501);
        } else {
            scalar(&mut w, 9, 32);
            w.extend(readlane(8, 1, 9));
        }
        exec(&mut w, u64::MAX);
        w.push(0x7e060208);
        let expected = if first {
            broadcast(&data, 0)
        } else {
            vec![9; 192]
        };
        push(
            &mut out,
            format!("inactive retained first{first}"),
            w,
            3,
            data,
            expected,
        );
    }
    out
}
fn mbcnt_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for mode in 0..7 {
        let len: usize = [64, 128, 192][mode % 3];
        let mut w = start();
        let mask = if matches!(mode, 3 | 4) {
            0x80000001
        } else {
            u32::MAX
        };
        let initial = if mode >= 5 {
            0xfffffff0
        } else if mode >= 3 {
            7
        } else {
            0
        };
        scalar(&mut w, 8, mask);
        scalar(&mut w, 9, initial);
        let high = matches!(mode, 1 | 4 | 6);
        w.extend([if high { 0xd7660002 } else { 0xd7650002 }, 0x02001208]);
        if mode == 2 {
            w.extend([0xd7660002, 0x02020408]);
        }
        let expected = (0..len)
            .map(|i| {
                let lane = i % 64;
                let count = match mode {
                    0 | 5 => lane.min(32) as u32,
                    1 | 6 => lane.saturating_sub(32) as u32,
                    2 => lane as u32,
                    3 => u32::from(lane > 0) + u32::from(lane > 31),
                    _ => u32::from(lane > 32), // bit31 is never below a Wave64 lane
                };
                initial.wrapping_add(count)
            })
            .collect();
        push(
            &mut out,
            format!("mbcnt mode{mode}"),
            w,
            2,
            input(len),
            expected,
        );
    }
    out
}
fn static_branch_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for op in 6..=9u32 {
        for (n, mask) in [0, 1u64 << 31, 1u64 << 32, 1u64 << 63]
            .into_iter()
            .enumerate()
        {
            let len = [64, 128, 192][n % 3];
            let mut w = start();
            if op < 8 {
                scalar(&mut w, 106, mask as u32);
                scalar(&mut w, 107, (mask >> 32) as u32);
            } else {
                exec(&mut w, mask);
            }
            w.extend([0xbe880381, 0xbf800001 | op << 16, 0xbe880382]);
            exec(&mut w, u64::MAX);
            w.push(0x7e060208);
            let taken = if op % 2 == 0 { mask == 0 } else { mask != 0 };
            push(
                &mut out,
                format!("branch op{op} mask{mask:016x}"),
                w,
                3,
                input(len),
                vec![if taken { 1 } else { 2 }; len],
            );
        }
    }
    out
}
fn runtime_vcc_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for rotation in 0..3 {
        let mut data = input(192);
        let mut halves = [[0u32; 2]; 3];
        for wave in 0..3 {
            let lane = [31, 32, 63][(wave + rotation) % 3];
            data[wave * 64 + lane] = 0;
            halves[wave][lane / 32] = 1u32 << (lane % 32);
        }
        for half in 0..2 {
            let mut w = start();
            scalar(&mut w, 106, u32::MAX);
            scalar(&mut w, 107, u32::MAX);
            w.extend([0x7d840280, 0x7e06026a | half as u32]);
            let expected = halves
                .iter()
                .flat_map(|pair| vec![pair[half]; 64])
                .collect();
            push(
                &mut out,
                format!("compare replace rotation{rotation} half{half}"),
                w,
                3,
                data.clone(),
                expected,
            );
        }
    }
    // Wave0 compares true only at32; wave1 only at31; wave2 nowhere.
    let mut data = input(192);
    data[32] = 0;
    data[64 + 31] = 0;
    for (written, value) in [(0, 0), (1, 0), (0, 0x80000000), (1, 0x80000000)] {
        let mut pairs = [[0, 1], [0x80000000, 0], [0, 0]];
        for pair in &mut pairs {
            pair[written] = value;
        }
        let mut base = start();
        base.push(0x7d840280);
        scalar(&mut base, 106 + written as u32, value);
        for half in 0..2 {
            let mut w = base.clone();
            w.push(0x7e06026a | half as u32);
            let expected = pairs.iter().flat_map(|pair| vec![pair[half]; 64]).collect();
            push(
                &mut out,
                format!("runtime VCC write{written}={value:x} read{half}"),
                w,
                3,
                data.clone(),
                expected,
            );
        }
        for nz in [false, true] {
            let mut w = base.clone();
            w.extend([
                0x7e060287,
                if nz { 0xbf870001 } else { 0xbf860001 },
                0x7e060289,
            ]);
            // A second comparison consumes the joined value and replaces BOTH
            // VCC halves. Observe the upper half to catch shared loop memo state.
            w.extend([0x7d840687, 0x7e08026b]);
            let expected = pairs
                .iter()
                .flat_map(|pair| {
                    let nonzero = pair[0] != 0 || pair[1] != 0;
                    let taken = if nz { nonzero } else { !nonzero };
                    vec![if taken { u32::MAX } else { 0 }; 64]
                })
                .collect();
            push(
                &mut out,
                format!("VCC halfwrite branch/recompare write{written}={value:x} nz{nz}"),
                w,
                4,
                data.clone(),
                expected,
            );
        }
    }
    out
}
fn transfer_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for (n, (address, offset)) in [(0u32, 0u16), (124, 4), (128, 0), (252, 3)]
        .into_iter()
        .enumerate()
    {
        for scatter in [false, true] {
            let len = [64, 128, 192][n % 3];
            let data = input(len);
            let mut w = start();
            w.extend([
                0x7e0402ff,
                address,
                0x7e0602ff,
                SENTINEL,
                if scatter { 0xdac80000 } else { 0xdacc0000 } | u32::from(offset),
                0x03000102,
                LGKM,
            ]);
            let selected = ((address + u32::from(offset)) / 4 % 32) as usize;
            let expected = (0..len)
                .map(|i| {
                    let base = i / 32 * 32;
                    if scatter {
                        if i % 32 == selected {
                            data[base + 31]
                        } else {
                            0
                        }
                    } else {
                        data[base + selected]
                    }
                })
                .collect();
            push(
                &mut out,
                format!("DS scatter{scatter} address{address} offset{offset}"),
                w,
                3,
                data,
                expected,
            );
        }
    }
    let fft = [
        0, 16, 8, 24, 4, 20, 12, 28, 2, 18, 10, 26, 6, 22, 14, 30, 1, 17, 9, 25, 5, 21, 13, 29, 3,
        19, 11, 27, 7, 23, 15, 31,
    ];
    for (n, offset) in [0x7c1f, 0x401f, 0xe000].into_iter().enumerate() {
        let len = [64, 128, 192][n];
        let data = input(len);
        let mut w = start();
        w.extend([0xd8d40000 | offset, 0x01000001, LGKM]); // VGPR source/destination alias
        let expected = (0..len)
            .map(|i| {
                data[i / 32 * 32
                    + match n {
                        0 => 31 - i % 32,
                        1 => (i % 32) ^ 16,
                        _ => fft[i % 32],
                    }]
            })
            .collect();
        push(
            &mut out,
            format!("swizzle {offset:x}"),
            w,
            1,
            data,
            expected,
        );
    }
    for cross in [false, true] {
        let data = input(192);
        let mut w = start();
        scalar(&mut w, 8, if cross { 0x76543210 } else { 0x89abcdef });
        scalar(&mut w, 9, if cross { 0xfedcba98 } else { 0x01234567 });
        w.extend([if cross { 0xd7781801 } else { 0xd7771801 }, 0x00241101]); // FI/BC set, full EXEC, v1 alias
        let expected = (0..192)
            .map(|i| {
                data[if cross {
                    i ^ 16
                } else {
                    i / 16 * 16 + 15 - i % 16
                }]
            })
            .collect();
        push(
            &mut out,
            format!("permlane cross{cross}"),
            w,
            1,
            data,
            expected,
        );
    }
    for scatter in [false, true] {
        let data = input(192);
        let mut w = start();
        w.extend([0x7e040280, 0x7e060301]);
        exec(&mut w, 0x8000000100000000);
        w.extend([
            if scatter { 0xdac80000 } else { 0xdacc0000 },
            0x03000102,
            LGKM,
        ]);
        exec(&mut w, u64::MAX);
        let expected = (0..192)
            .map(|i| match i % 64 {
                32 => data[i / 64 * 64 + if scatter { 63 } else { 32 }],
                63 => {
                    if scatter {
                        0
                    } else {
                        data[i / 64 * 64 + 32]
                    }
                }
                _ => data[i],
            })
            .collect();
        push(
            &mut out,
            format!("DS high EXEC scatter{scatter}"),
            w,
            3,
            data,
            expected,
        );
    }
    out
}
fn nested_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for len in [64, 128, 192] {
        for address_gather in [false, true] {
            let data = input(len);
            let mut w = start();
            // Markers' low bits carry lane position; DS discards bits above6.
            w.extend([0x34040282, 0x7e060280, 0x7e080280, 0x7e0a0280, 0x7e0c0280]);
            if address_gather {
                w.extend([0xd8d47c1f, 0x05000002, LGKM, 0xdac80004, 0x04000105, LGKM]);
            } else {
                w.extend([0xd8d4041f, 0x03000001, LGKM, 0xdac80004, 0x04000302, LGKM]);
            }
            w.extend([
                0xd8d4401f, 0x06000004, LGKM, 0x7e020280, 0x7e040280, 0x7e060280, 0x7e080280,
                0x7e0a0280,
            ]);
            let mut expected = Vec::new();
            // Materialize each stage per half, preserving distinct upper-half
            // input values even when every local lane index is the same.
            for half in data.as_chunks::<32>().0 {
                let source: Vec<_> = if address_gather {
                    half.to_vec()
                } else {
                    half.as_chunks::<2>()
                        .0
                        .iter()
                        .flat_map(|pair| [pair[1], pair[0]])
                        .collect()
                };
                let mut scattered = [0; 32];
                for (lane, &value) in source.iter().enumerate() {
                    let target = if address_gather {
                        (32 - lane) % 32
                    } else {
                        (lane + 1) % 32
                    };
                    scattered[target] = value;
                }
                expected.extend_from_slice(&scattered[16..]);
                expected.extend_from_slice(&scattered[..16]);
            }
            push(
                &mut out,
                format!("nested length{len} address{address_gather}"),
                w,
                6,
                data,
                expected,
            );
        }
    }
    out
}
fn cases() -> Vec<Case> {
    lane_cases()
        .into_iter()
        .chain(mbcnt_cases())
        .chain(static_branch_cases())
        .chain(runtime_vcc_cases())
        .chain(transfer_cases())
        .chain(nested_cases())
        .collect()
}
fn config() -> xenora_shader::WaveConfig {
    xenora_shader::WaveConfig {
        size: xenora_shader::WaveSize::Wave64,
        launch: xenora_shader::WaveLaunch::FullWavesRequired,
    }
}
fn ref_config() -> xenora_ref::WaveConfig {
    xenora_ref::WaveConfig {
        size: xenora_ref::WaveSize::Wave64,
        launch: xenora_ref::WaveLaunch::FullWavesRequired,
    }
}
fn compile(code: &[u8]) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    xenora_shader::compile_with_wave_config(xenora_shader::Architecture::Gfx1030, code, config())
}
fn reference(code: &[u8], data: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_wave_config(code, data, ref_config())
}
#[test]
fn wave32_variant_import_source_compatibility() {
    // A type alias breaks these imports; a public re-export preserves them.
    use xenora_ref::Wave32Launch::FullWavesRequired as ReferenceLegacy;
    use xenora_shader::Wave32Launch::FullWavesRequired as CompilerLegacy;
    let _: xenora_shader::WaveLaunch = CompilerLegacy;
    let _: xenora_ref::WaveLaunch = ReferenceLegacy;
}
#[test]
fn literal_encodings_and_handwritten_boundaries() {
    assert_eq!(readlane(8, 1, 9), [0xd7600008, 0x02001301]);
    assert_eq!(0xdac80000u32.to_le_bytes(), [0, 0, 0xc8, 0xda]);
    let mb = mbcnt_cases();
    assert_eq!(
        [mb[0].expected[31], mb[0].expected[32], mb[0].expected[63]],
        [31, 32, 32]
    );
    assert_eq!(
        [mb[1].expected[31], mb[1].expected[32], mb[1].expected[63]],
        [0, 0, 31]
    );
    assert_eq!(
        [mb[2].expected[31], mb[2].expected[32], mb[2].expected[63]],
        [31, 32, 63]
    );
    assert_eq!(
        [mb[3].expected[31], mb[3].expected[32], mb[3].expected[63]],
        [8, 9, 9]
    );
    assert_eq!(
        [mb[4].expected[31], mb[4].expected[32], mb[4].expected[63]],
        [7, 7, 8]
    );
    assert_eq!(lane_cases().len(), 40);
    assert_eq!(runtime_vcc_cases().len(), 22);
    assert_eq!(transfer_cases().len(), 15);
    assert_eq!(cases().len(), 106);
    assert_eq!(wave32_hi_cases().len(), 6);
}
#[test]
fn independent_wave64_outputs_masks_snapshots_and_replay() {
    for case in cases() {
        assert!(case.code.len() <= 1024, "{}", case.name);
        let shader = compile(&case.code).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert!(matches!(
            shader.wave_config().size,
            xenora_shader::WaveSize::Wave64
        ));
        assert!(matches!(
            shader.launch_policy(),
            xenora_shader::WaveLaunch::FullWavesRequired
        ));
        shader.validate_launch(case.input.len()).unwrap();
        assert_eq!(
            reference(&case.code, &case.input).unwrap_or_else(|e| panic!("{}: {e}", case.name)),
            case.expected,
            "{}",
            case.name
        );
    }
}
fn negative_cases() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = Vec::new();
    for selector in [0, 32, 63, 64] {
        let mut w = start();
        scalar(&mut w, 9, selector);
        w.extend(readlane(9, 1, 9));
        w.push(0x7e060209);
        out.push((
            "READLANE original scalar source/destination overlap",
            finish(w, 3),
        ));
    }
    for source in [106, 107] {
        for value in [0, u32::MAX] {
            let mut w = start();
            scalar(&mut w, 106, value);
            scalar(&mut w, 107, value);
            w.extend([0x7d840200 | source, 0x7e06026a]);
            out.push((
                "VCC original compare operand overlap even if constant",
                finish(w, 3),
            ));
        }
    }
    for half in [106, 107] {
        for branch in [0xbf860001, 0xbf870001] {
            let mut w = start();
            scalar(&mut w, half, 0);
            w.extend([0x7e060287, branch, 0x7e060289]);
            out.push(("untouched VCC half remains unknown", finish(w, 3)));
        }
    }
    let mut w = start();
    w.extend(readlane(8, 1, 9));
    w.push(0x7e060208);
    out.push(("undefined selector", finish(w, 3)));
    let mut w = LOAD.to_vec();
    scalar(&mut w, 9, 63);
    w.extend(readlane(8, 1, 9));
    w.extend([VM, 0x7e060208]);
    out.push(("pending lane source", finish(w, 3)));
    for op in [0xdac80000, 0xdacc0000] {
        let mut w = start();
        w.extend([0x7e040280, 0x7e060280, op, 0x03000102, VM]);
        out.push(("VM wait cannot release DS", finish(w, 3)));
    }
    out
}
#[test]
fn overlap_unknown_half_and_pending_source_rejections() {
    for (name, code) in negative_cases() {
        assert!(compile(&code).is_err(), "compiler {name}");
        assert!(reference(&code, &input(64)).is_err(), "reference {name}");
    }
}
#[test]
fn explicit_launch_and_wave32_compatibility() {
    let case = lane_cases().remove(22); // READLANE63 with distinct selector/destination
    let shader = compile(&case.code).unwrap();
    for count in 0..=4097 {
        assert_eq!(
            shader.validate_launch(count).is_ok(),
            (64..=4096).contains(&count) && count % 64 == 0,
            "count{count}"
        );
    }
    for count in [0, 1, 32, 63, 65, 96, 129, 4097] {
        assert!(reference(&case.code, &vec![0; count]).is_err());
    }
    assert_eq!(
        reference(&case.code, &vec![0x80000001; 4096]).unwrap(),
        vec![0x80000001; 4096]
    );
    assert!(
        xenora_shader::compile_with_wave_config(
            xenora_shader::Architecture::Gfx1030,
            &case.code,
            xenora_shader::WaveConfig {
                size: xenora_shader::WaveSize::Wave64,
                launch: xenora_shader::WaveLaunch::PartialAllowed
            }
        )
        .is_err()
    );
    assert!(
        xenora_ref::execute_with_wave_config(
            &case.code,
            &input(64),
            xenora_ref::WaveConfig {
                size: xenora_ref::WaveSize::Wave64,
                launch: xenora_ref::WaveLaunch::PartialAllowed
            }
        )
        .is_err()
    );
    let mut w = start();
    scalar(&mut w, 9, 32);
    w.extend(readlane(8, 1, 9));
    w.push(0x7e060208);
    let code = finish(w, 3);
    let data = input(64);
    let expected = data
        .as_chunks::<32>()
        .0
        .iter()
        .flat_map(|wave| vec![wave[0]; 32])
        .collect::<Vec<_>>();
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
    assert_eq!(xenora_ref::execute(&code, &data).unwrap(), expected);
    assert_ne!(reference(&code, &data).unwrap(), expected); // Wave64 selects32, not local0
}
fn wave32_hi_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for register in [107, 127] {
        for value in [0, 0x80000001] {
            let mut w = start();
            if register == 107 {
                w.push(0x7d840280);
            } // compare invalidates HI, explicit write reinitializes it
            scalar(&mut w, register, value);
            w.push(0x7e060200 | register);
            push(
                &mut out,
                format!("Wave32 explicit high register{register} value{value:x}"),
                w,
                3,
                input(64),
                vec![value; 64],
            );
        }
    }
    let mut w = start();
    // No input is zero. Compare's high half is UNKNOWN, but VCCZ observes LO
    // only in Wave32, so it must take the branch and retain7 in every lane.
    w.extend([0x7e060287, 0x7d840280, 0xbf860001, 0x7e060289]);
    push(
        &mut out,
        "Wave32 VCCZ ignores unknown HI",
        w,
        3,
        input(64),
        vec![7; 64],
    );
    let mut w = start();
    scalar(&mut w, 126, 0);
    scalar(&mut w, 127, 1);
    w.extend([0xbe880381, 0xbf890001, 0xbe880382, 0xbefe03c1, 0x7e060208]);
    push(
        &mut out,
        "Wave32 EXECNZ ignores explicitly nonzero HI",
        w,
        3,
        input(64),
        vec![2; 64],
    );
    out
}
fn compile32(code: &[u8]) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    xenora_shader::compile_with_wave_config(
        xenora_shader::Architecture::Gfx1030,
        code,
        xenora_shader::WaveConfig {
            size: xenora_shader::WaveSize::Wave32,
            launch: xenora_shader::WaveLaunch::PartialAllowed,
        },
    )
}
fn reference32(code: &[u8], data: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_wave_config(
        code,
        data,
        xenora_ref::WaveConfig {
            size: xenora_ref::WaveSize::Wave32,
            launch: xenora_ref::WaveLaunch::PartialAllowed,
        },
    )
}
#[test]
fn wave32_unknown_high_halves_require_explicit_initialization() {
    let mut negatives = Vec::new();
    for mask in [0, u32::MAX] {
        let mut w = start();
        scalar(&mut w, 107, 0x80000001); // a prior known value must not survive compare
        scalar(&mut w, 126, mask);
        w.extend([0x7d840280, 0xbefe03c1, 0x7e06026b]);
        negatives.push(finish(w, 3));
    }
    for register in [107, 127] {
        let mut w = start();
        w.push(0x7e060200 | register);
        negatives.push(finish(w, 3));
    }
    for (n, code) in negatives.into_iter().enumerate() {
        assert!(
            compile32(&code).is_err(),
            "Wave32 unknown high compiler {n}"
        );
        assert!(
            reference32(&code, &input(64)).is_err(),
            "Wave32 unknown high reference {n}"
        );
    }
    for case in wave32_hi_cases() {
        compile32(&case.code).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        assert_eq!(
            reference32(&case.code, &case.input).unwrap(),
            case.expected,
            "{}",
            case.name
        );
    }
}

#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_wave64_independent() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (n, case) in cases().into_iter().enumerate() {
        let shader = compile(&case.code).unwrap();
        assert_eq!(
            gpu.run_compiled_words(&shader, &case.input).unwrap(),
            case.expected,
            "{}",
            case.name
        );
        if [0, 103, 105].contains(&n) {
            shader.validate_launch(case.input.len()).unwrap();
            for width in [3, 17, 64] {
                assert_eq!(
                    gpu.run_words_with_group_width(
                        &shader.msl,
                        xenora_shader::ENTRY_POINT,
                        &case.input,
                        width
                    )
                    .unwrap()
                    .output,
                    case.expected,
                    "{} width{width}",
                    case.name
                );
            }
        }
        if n == 0 {
            for count in [0, 1, 32, 63, 65, 96, 4097] {
                let error = gpu
                    .run_compiled_words(&shader, &vec![1; count])
                    .unwrap_err();
                assert!(
                    error.to_string().contains("compiled shader launch"),
                    "{error}"
                );
            }
            // Deliberately bypass metadata to test the generated early guard.
            // The native backend pre-fills output with A5A5A5A5; every slot must
            // stay untouched. Counts fit backend limits but violate Wave64.
            for count in [1, 32, 63, 65, 96] {
                assert_eq!(
                    gpu.run_words(&shader.msl, xenora_shader::ENTRY_POINT, &vec![1; count])
                        .unwrap(),
                    vec![0xa5a5a5a5; count],
                    "raw guard count{count}"
                );
            }
        }
    }
    for case in wave32_hi_cases() {
        let shader = compile32(&case.code).unwrap();
        assert_eq!(
            gpu.run_compiled_words(&shader, &case.input).unwrap(),
            case.expected,
            "{}",
            case.name
        );
    }
    // 106 Wave64 +6 Wave32 +9 group-width +5 raw-guard =126 GPU dispatches.
}
