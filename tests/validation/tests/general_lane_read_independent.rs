// SPDX-License-Identifier: GPL-3.0-or-later
//! Original full-wave expectations from AMD RDNA2 70648 sections12.8/12.12:
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! READFIRST selects lowest EXEC bit (zero for empty EXEC); READLANE masks
//! its scalar selector to five bits and ignores EXEC. FullWavesRequired is an
//! explicit project launch contract. No expected output calls production code.
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const WAIT: u32 = 0xbf8c3f70;
const END: u32 = 0xbf810000;
struct Case {
    code: Vec<u8>,
    input: Vec<u32>,
    expected: Vec<u32>,
}
fn first(dst: u32, src: u32) -> [u32; 1] {
    [0x7e000500 | dst << 17 | src]
}
fn lane(dst: u32, src: u32, index: u32) -> [u32; 2] {
    [0xd7600000 | dst, 0x02000100 | index << 9 | src]
}
fn write(dst: u32, src: u32, index: u32) -> [u32; 2] {
    [0xd7610000 | dst, 0x02000000 | index << 9 | src]
}
fn finish(mut words: Vec<u32>, dst: u32) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, END]);
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| [0x80001000, 0xfffffe00, 0x7fffff00][i / 32] + (i % 32) as u32)
        .collect()
}
fn broadcast(data: &[u32], selected: usize) -> Vec<u32> {
    data.as_chunks::<32>()
        .0
        .iter()
        .flat_map(|wave| vec![wave[selected]; 32])
        .collect()
}
fn equal_mask(data: &[u32], selected: usize) -> u32 {
    data.iter().enumerate().fold(0, |mask, (i, &x)| {
        mask | (u32::from(x == data[selected]) << i)
    })
}
fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    // 64 programs: every selected lane both direct and wrapped, distributing
    // complete-wave lengths pairwise. Inactive selected lanes retain input.
    for selected in 0..32usize {
        for wrapped in [false, true] {
            let len = [32, 64, 96][(selected + usize::from(wrapped)) % 3];
            let data = input(len);
            let mut w = LOAD.to_vec();
            w.push(WAIT);
            let index = if wrapped {
                [32, 64, 0xffffffe0][selected % 3] + selected as u32
            } else {
                selected as u32
            };
            if wrapped {
                w.extend([0xbefe03ff, !(1u32 << selected), 0x7e020289, 0xbefe0380]);
            }
            w.extend([0xbe8903ff, index]);
            w.extend(lane(8, 1, 9));
            w.extend([0xbefe03c1, 0x7e020280, 0x7e060208]);
            cases.push(Case {
                code: finish(w, 3),
                expected: broadcast(&data, selected),
                input: data,
            });
        }
    }
    // 35 programs: every lowest-set-bit position plus EXEC0 at each length.
    for selected in 0..32usize {
        let len = [32, 64, 96][selected % 3];
        let data = input(len);
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbefe03ff, (1u32 << selected) | 0x80000000]);
        w.extend(first(8, 1));
        w.extend([0xbefe03c1, 0x7e060208]);
        cases.push(Case {
            code: finish(w, 3),
            expected: broadcast(&data, selected),
            input: data,
        });
    }
    for len in [32, 64, 96] {
        let data = input(len);
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbefe0380]);
        w.extend(first(8, 1));
        w.extend([0xbefe03c1, 0x7e060208]);
        cases.push(Case {
            code: finish(w, 3),
            expected: broadcast(&data, 0),
            input: data,
        });
    }
    // Nine programs: a captured scalar chooses a different lane in every wave.
    // Direct, comparison replay, and nested read with subsequent source overwrite.
    for len in [32, 64, 96] {
        let mut data = input(len);
        for (wave, chunk) in data.as_chunks_mut::<32>().0.iter_mut().enumerate() {
            chunk[0] |= [31, 16, 1][wave];
        }
        for mode in 0..3 {
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0x7e040301]);
            w.extend(first(9, 1));
            w.extend(lane(8, 1, 9));
            w.push(0x7e060208);
            let expected = data
                .as_chunks::<32>()
                .0
                .iter()
                .flat_map(|wave| {
                    let selected = (wave[0] % 32) as usize;
                    vec![
                        if mode == 1 {
                            equal_mask(wave, selected)
                        } else {
                            wave[selected]
                        };
                        32
                    ]
                })
                .collect();
            let dst = if mode == 1 {
                w.extend([0x7d840503, 0x7e08026a]);
                4
            } else if mode == 2 {
                w.extend(lane(10, 3, 9));
                w.extend([0x7e020280, 0x7e060280, 0xbe890380, 0x7e08020a]);
                4
            } else {
                3
            };
            cases.push(Case {
                code: finish(w, dst),
                input: data.clone(),
                expected,
            });
        }
    }
    // Three programs: MBCNT replay at lane31 gives LO(-1,0)=31 and HI=31;
    // later input loads must not substitute invocation lane for selected lane.
    for len in [32, 64, 96] {
        let mut w = LOAD.to_vec();
        w.extend([
            WAIT, 0xd7650002, 0x020100c1, 0xd7660002, 0x020204c1, 0xbe89039f,
        ]);
        w.extend(lane(8, 2, 9));
        w.push(0x7e060208);
        cases.push(Case {
            code: finish(w, 3),
            input: input(len),
            expected: vec![31; len],
        });
    }
    // Nine programs: before/after write snapshots and a later scalar overwrite.
    // Read lane31, write captured value into lane16, read lane16, overwrite both
    // scalar and vector sources; earlier captures must remain immutable.
    for len in [32, 64, 96] {
        let data = input(len);
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe89039f]);
        w.extend(lane(8, 1, 9));
        w.extend([0xbe890390]);
        w.extend(lane(10, 1, 9));
        w.extend(write(1, 8, 9));
        w.extend(lane(11, 1, 9));
        w.extend([0xbe88038d, 0x7e020280, 0x7e06020a, 0x7e08020b]);
        cases.push(Case {
            code: finish(w.clone(), 3),
            input: data.clone(),
            expected: broadcast(&data, 16),
        });
        cases.push(Case {
            code: finish(w.clone(), 4),
            input: data.clone(),
            expected: broadcast(&data, 31),
        });
        w.push(0x7e0a0208);
        cases.push(Case {
            code: finish(w, 5),
            input: data,
            expected: vec![13; len],
        });
    }
    // Six programs: lane31 capture -> runtime VCCNZ join -> comparison replay.
    // Full waves alternate branch outcomes within one dispatch.
    for len in [32, 64, 96] {
        for flip in [false, true] {
            let mut data = input(len);
            for (wave, chunk) in data.as_chunks_mut::<32>().0.iter_mut().enumerate() {
                if (wave % 2 == 0) != flip {
                    chunk[0] = 0;
                }
            }
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0x7e040301, 0xbe89039f]);
            w.extend(lane(8, 1, 9));
            w.extend([
                0x7e060208, 0x7d840280, 0xbf870001, 0x7e060289, 0x7d840503, 0x7e08026a,
            ]);
            let expected = data
                .as_chunks::<32>()
                .0
                .iter()
                .flat_map(|wave| vec![if wave.contains(&0) { 0x80000000 } else { 0 }; 32])
                .collect();
            cases.push(Case {
                code: finish(w, 4),
                input: data,
                expected,
            });
        }
    }
    // Destination aliases the runtime selector: consume the old s9 before
    // publishing its newly selected word. Three waves choose 31,16,1.
    let mut data = input(96);
    for (wave, chunk) in data.as_chunks_mut::<32>().0.iter_mut().enumerate() {
        chunk[0] |= [31, 16, 1][wave];
    }
    let mut w = LOAD.to_vec();
    w.extend([WAIT, 0x7e040301]); // v2 preserves the selector source
    w.extend(first(9, 2));
    w.extend(lane(9, 1, 9));
    w.push(0x7e060209);
    let expected = [0x8000101f, 0xfffffe10, 0x7fffff01]
        .into_iter()
        .flat_map(|value| vec![value; 32])
        .collect();
    cases.push(Case {
        code: finish(w, 3),
        input: data,
        expected,
    });
    cases
}
#[test]
fn exact_encodings_and_independent_corpus_size() {
    assert_eq!(first(8, 1), [0x7e100501]);
    assert_eq!(lane(8, 1, 9), [0xd7600008, 0x02001301]);
    assert_eq!(cases().len(), 127);
    for case in cases() {
        assert!(matches!(case.input.len(), 32 | 64 | 96));
        assert_eq!(case.input.len(), case.expected.len());
    }
}
fn compile(code: &[u8]) -> xenora_shader::CompiledShader {
    xenora_shader::compile_with_options(
        xenora_shader::Architecture::Gfx1030,
        code,
        xenora_shader::CompileOptions {
            wave32_launch: xenora_shader::Wave32Launch::FullWavesRequired,
        },
    )
    .unwrap()
}
fn execute(code: &[u8], data: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_options(
        code,
        data,
        xenora_ref::ExecuteOptions {
            wave32_launch: xenora_ref::Wave32Launch::FullWavesRequired,
        },
    )
}
#[test]
fn full_wave_values_snapshots_selectors_and_comparison_joins() {
    for (n, case) in cases().into_iter().enumerate() {
        let compiled = compile(&case.code);
        assert_eq!(
            compiled.wave32_launch(),
            xenora_shader::Wave32Launch::FullWavesRequired
        );
        compiled.validate_launch(case.input.len()).unwrap();
        assert_eq!(
            execute(&case.code, &case.input).unwrap(),
            case.expected,
            "case {n}"
        );
    }
}
#[test]
fn launch_contract_rejects_empty_partial_and_oversized_dispatches() {
    let case = cases().remove(62); // explicit selected lane31, complete input
    let compiled = compile(&case.code);
    for count in 0..=4097 {
        assert_eq!(
            compiled.validate_launch(count).is_ok(),
            (32..=4096).contains(&count) && count % 32 == 0,
            "count {count}"
        );
    }
    for count in [0, 1, 15, 16, 17, 31, 33, 63, 65, 95, 97, 4095, 4097, 4128] {
        assert!(
            execute(&case.code, &vec![0; count]).is_err(),
            "count {count}"
        );
    }
    // The upper accepted boundary has 128 complete waves; every selected value
    // is explicitly present and identical here, independently of evaluator code.
    assert_eq!(
        execute(&case.code, &[0x80000001; 4096]).unwrap(),
        vec![0x80000001; 4096]
    );
}
#[test]
fn default_api_keeps_lane_zero_and_rejects_nonzero_or_runtime_selection() {
    for selected in [1, 15, 16, 31] {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe8903ff, selected]);
        w.extend(lane(8, 1, 9));
        w.push(0x7e060208);
        let code = finish(w, 3);
        assert!(xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &input(32)).is_err());
    }
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(first(9, 1));
    w.extend(lane(8, 1, 9));
    w.push(0x7e060208);
    let code = finish(w, 3);
    assert!(xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).is_err());
    assert!(xenora_ref::execute(&code, &[0; 32]).is_err());
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(first(8, 1));
    w.push(0x7e060208);
    let code = finish(w, 3);
    let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
    for count in [1, 31, 33, 63, 65] {
        compiled.validate_launch(count).unwrap();
        assert_eq!(
            xenora_ref::execute(&code, &vec![0x80000001; count]).unwrap(),
            vec![0x80000001; count]
        );
    }
}
#[test]
fn explicit_full_waves_do_not_admit_unknown_or_pending_operands() {
    let mut programs = Vec::new();
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(lane(8, 1, 9));
    w.push(0x7e060208);
    programs.push(finish(w, 3)); // uninitialized selector
    let mut w = vec![0xbe89039f];
    w.extend(lane(8, 1, 9));
    w.push(0x7e060208);
    programs.push(finish(w, 3)); // uninitialized vector source
    let mut w = LOAD.to_vec();
    w.push(0xbe89039f);
    w.extend(lane(8, 1, 9));
    w.extend([WAIT, 0x7e060208]);
    programs.push(finish(w, 3)); // source load still pending
    for code in programs {
        assert!(
            xenora_shader::compile_with_options(
                xenora_shader::Architecture::Gfx1030,
                &code,
                xenora_shader::CompileOptions {
                    wave32_launch: xenora_shader::Wave32Launch::FullWavesRequired
                }
            )
            .is_err()
        );
        assert!(execute(&code, &[0; 32]).is_err());
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_general_lane_read_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (n, case) in cases().into_iter().enumerate() {
        let compiled = compile(&case.code);
        assert_eq!(
            gpu.run_compiled_words(&compiled, &case.input).unwrap(),
            case.expected,
            "case {n}"
        );
        if n == 0 {
            for count in [0, 1, 31, 33, 4097] {
                let error = gpu
                    .run_compiled_words(&compiled, &vec![0; count])
                    .unwrap_err();
                assert!(
                    error.to_string().contains("compiled shader launch"),
                    "{error}"
                );
            }
        }
        if [63, 101, 125].contains(&n) {
            // Raw group-width access is used only after validating the compiled
            // full-wave contract. Guest wave boundaries cannot follow Metal groups.
            compiled.validate_launch(case.input.len()).unwrap();
            for width in [2, 17, 32, 64] {
                assert_eq!(
                    gpu.run_words_with_group_width(
                        &compiled.msl,
                        xenora_shader::ENTRY_POINT,
                        &case.input,
                        width
                    )
                    .unwrap()
                    .output,
                    case.expected,
                    "case {n}, group width {width}"
                );
            }
        }
    }
}
