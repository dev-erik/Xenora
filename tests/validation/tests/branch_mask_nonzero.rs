// SPDX-License-Identifier: GPL-3.0-or-later
//! Original expectations from AMD RDNA2 ISA 70648 sections 3.7/3.9 and 12.5:
//! https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf
//! SOPP op7 VCCNZ and op9 EXECNZ branch when the Wave32 low mask is nonzero.
//! Target = next PC + signed SIMM16 * 4. Forward-only/region admission is the
//! project contract, not an architectural restriction. Native ARM64 LLVM 23.1.6
//! gfx1030 confirms +1 encodings [01,00,87,bf] and [01,00,89,bf].
#[path = "../independent_ref.rs"]
mod independent_ref;
const VCCNZ: u32 = 0xbf870000;
const EXECNZ: u32 = 0xbf890000;
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const WAIT: u32 = 0xbf8c3f70;
const END: u32 = 0xbf810000;
const CMP: u32 = 0x7d840280; // v_cmp_eq_u32 vcc_lo,0,v1
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/mask-nonzero-branch.bin");
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn finish(mut words: Vec<u32>, dst: u32) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, END]);
    bytes(&words)
}
fn scalar(body: &[u32]) -> Vec<u8> {
    let mut words = body.to_vec();
    words.extend([0xbefe03c1, 0x7e040208]);
    finish(words, 2)
}
fn runtime(region: &[u32], joined: bool) -> Vec<u8> {
    let mut words = LOAD.to_vec();
    words.extend([WAIT, 0x7e040287, CMP, VCCNZ | region.len() as u32]);
    words.extend(region);
    let dst = if joined {
        words.extend([0x7d840487, 0x7e06026a]);
        3
    } else {
        2
    };
    finish(words, dst)
}
fn expected(data: &[u32], joined: bool) -> Vec<u32> {
    data.chunks(32)
        .flat_map(|wave| {
            let taken = wave.contains(&0);
            let value = if joined {
                // All present lanes retain 7 when any lane compares equal to zero.
                // Enumerate present bits; do not shift a u32 by the wave width.
                if taken {
                    (0..wave.len()).fold(0, |m, lane| m | (1 << lane))
                } else {
                    0
                }
            } else if taken {
                7
            } else {
                9
            };
            vec![value; wave.len()]
        })
        .collect()
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for (branch, low, high) in [(VCCNZ, 106, 107), (EXECNZ, 126, 127)] {
        for mask in [0, 1, 0x80000000, u32::MAX] {
            for high_word in [0, u32::MAX] {
                // VCC scalar branching must operate even while EXEC is empty.
                let body = [
                    0xbe880387,
                    0xbefe0380,
                    0xbe8003ff | low << 16,
                    mask,
                    0xbe8003ff | high << 16,
                    high_word,
                    branch | 1,
                    0xbe880389,
                ];
                cases.push((
                    scalar(&body),
                    vec![0; 33],
                    vec![if mask != 0 { 7 } else { 9 }; 33],
                ));
                // SIMM16=0 targets the next instruction, not the current branch.
                let mut zero = body;
                zero[6] = branch;
                cases.push((scalar(&zero), vec![0; 1], vec![9]));
            }
        }
    }
    for len in [1, 31, 32, 33, 63, 65] {
        for pattern in 0..4 {
            let data: Vec<u32> = (0..len)
                .map(|i| match pattern {
                    0 => 0,
                    1 => 1,
                    2 => u32::from(i != 31 && i != 64),
                    _ => u32::from(i != 0 && i != 32),
                })
                .collect();
            for joined in [false, true] {
                cases.push((
                    runtime(&[0x7e040289], joined),
                    data.clone(),
                    expected(&data, joined),
                ));
            }
            cases.push((FIXTURE.to_vec(), data.clone(), expected(&data, true)));
        }
    }
    cases.extend(partial_exec_cases());
    cases
}
// Compare under full EXEC first, then change EXEC without replacing runtime VCC.
fn partial_runtime(region: &[u32], exec: u32, joined: bool) -> Vec<u8> {
    let mut words = LOAD.to_vec();
    words.extend([
        WAIT,
        0x7e040287,
        CMP,
        0xbefe03ff,
        exec,
        VCCNZ | region.len() as u32,
    ]);
    words.extend(region);
    words.push(0xbefe03c1); // restore full EXEC for compare/readback
    let dst = if joined {
        words.extend([0x7d840487, 0x7e06026a]);
        3
    } else {
        2
    };
    finish(words, dst)
}
fn partial_exec_cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in [1, 31, 32, 33, 65] {
        for pattern in 0..4 {
            let data: Vec<u32> = (0..len)
                .map(|i| match pattern {
                    0 => 0,
                    1 => 1,
                    2 => u32::from(i != 31 && i != 64),
                    _ => u32::from(i != 0 && i != 32),
                })
                .collect();
            for exec in [0, 0x80000002, 0x55555555] {
                for joined in [false, true] {
                    let want = data
                        .chunks(32)
                        .flat_map(|wave| {
                            // The saved comparison includes lanes made inactive later.
                            // Taken: all lanes retain 7. Fall-through: only EXEC lanes
                            // write 9; inactive destinations keep the seeded sentinel 7.
                            let taken = wave.contains(&0);
                            let values: Vec<u32> = (0..wave.len())
                                .map(|lane| {
                                    if taken || exec & (1 << lane) == 0 {
                                        7
                                    } else {
                                        9
                                    }
                                })
                                .collect();
                            if joined {
                                let mask = values
                                    .iter()
                                    .enumerate()
                                    .fold(0, |m, (lane, &v)| m | (u32::from(v == 7) << lane));
                                vec![mask; wave.len()]
                            } else {
                                values
                            }
                        })
                        .collect();
                    cases.push((
                        partial_runtime(&[0x7e040289], exec, joined),
                        data.clone(),
                        want,
                    ));
                }
            }
        }
    }
    cases
}
#[test]
fn partial_exec_runtime_retains_sentinels_and_recomputes_join_comparisons() {
    assert_eq!(partial_exec_cases().len(), 120);
    assert_eq!(cases().len(), 224);
    for (code, data, want) in partial_exec_cases() {
        check(&code, &data, &want);
    }
}
#[test]
fn empty_exec_does_not_bypass_runtime_region_validation() {
    for region in [
        vec![0x7e060289], // new destination remains forbidden even without writes
        vec![0xbe880389], // unsupported scalar instruction
        vec![VCCNZ],
        vec![EXECNZ],
        LOAD.to_vec(),
        vec![WAIT],
    ] {
        for value in [0, 1] {
            rejects(&partial_runtime(&region, 0, false), &[value; 32], 32);
        }
    }
    // Compare before clearing EXEC keeps VCC lane-derived; a pending load
    // must be rejected at the branch for both outcomes even with no live lanes.
    for value in [0x80, 0x81] {
        let mut words = vec![0x7e040200 | value];
        words.extend(LOAD);
        words.extend([
            0x7d840480,
            0xbefe0380,
            VCCNZ | 1,
            0x7e040289,
            0xbefe03c1,
            WAIT,
        ]);
        rejects(&finish(words, 2), &[0; 32], 20);
    }
}
fn check(code: &[u8], data: &[u32], want: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, data).unwrap(), want);
    assert_eq!(independent_ref::execute(code, data).unwrap(), want);
}
fn rejects(code: &[u8], data: &[u32], at: usize) {
    let compiler = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match compiler {
        xenora_shader::CompileError::Decode(e) => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } => assert_eq!(offset, at),
        other => panic!("unexpected rejection {other:?}"),
    }
    let primary = xenora_ref::execute(code, data).unwrap_err();
    assert_eq!(primary.offset, at, "{primary}");
    let independent = independent_ref::execute(code, data).unwrap_err();
    assert!(
        independent.starts_with(&format!("{at:#x}:")),
        "{independent}"
    );
}
#[test]
fn exact_encoding_and_fixture_words() {
    assert_eq!((VCCNZ | 1).to_le_bytes(), [1, 0, 0x87, 0xbf]);
    assert_eq!((EXECNZ | 1).to_le_bytes(), [1, 0, 0x89, 0xbf]);
    let mut words = vec![0xbe880387, EXECNZ | 1, 0xbe88038b];
    words.extend(LOAD);
    words.extend([
        WAIT,
        0x7e040208,
        CMP,
        VCCNZ | 1,
        0x7e040289,
        0x7d840487,
        0x7e06026a,
    ]);
    assert_eq!(FIXTURE, finish(words, 3));
}
#[test]
fn known_masks_high_halves_empty_exec_runtime_waves_and_comparison_joins() {
    for (code, data, want) in cases() {
        check(&code, &data, &want);
    }
}
#[test]
fn independent_reference_matches_locally_derived_answers() {
    for (code, data, want) in cases() {
        assert_eq!(independent_ref::execute(&code, &data).unwrap(), want);
    }
}
#[test]
fn unknown_vcc_rejected_even_empty_exec_or_zero_displacement() {
    for exec in [0x80, 0xc1] {
        for displacement in [0, 1] {
            let code = scalar(&[
                0xbe880387,
                0xbefe0300 | exec,
                VCCNZ | displacement,
                0xbe880389,
            ]);
            rejects(&code, &[0; 32], 8);
        }
    }
}
#[test]
fn signed_malformed_targets_rejected_for_both_branch_outcomes() {
    for (branch, reg) in [(VCCNZ, 106), (EXECNZ, 126)] {
        for mask in [0x80, 0xc1] {
            for displacement in [1, 3, 0x7fff, 0xffff, 0xfffe, 0x8000] {
                let code = bytes(&[
                    0xbe880387,
                    0xbe800300 | reg << 16 | mask,
                    branch | displacement,
                    0xbe8803ff,
                    0x12345678,
                    0xe0702000,
                    0x80010100,
                    0xbbfd0000,
                    END,
                ]);
                rejects(&code, &[0], 8);
            }
            rejects(
                &bytes(&[0xbe800300 | reg << 16 | mask, branch | 1, END]),
                &[0],
                4,
            );
        }
    }
}
#[test]
fn invalid_runtime_regions_rejected_identically_taken_and_untaken() {
    for region in [
        vec![0x7e060289], // uninitialized destination v3
        vec![0xbe880389], // scalar opcode unsupported inside runtime region
        vec![VCCNZ],      // nested runtime branch
        vec![EXECNZ],     // scalar mask branch inside runtime region
        LOAD.to_vec(),    // memory inside region
        vec![WAIT],       // wait inside region
        vec![0x7e040203], // uninitialized scalar source s3 (resource value absent)
    ] {
        let code = runtime(&region, false);
        for value in [0, 1] {
            rejects(&code, &[value; 32], 24);
        }
    }
    // The compare uses initialized v2 so the outstanding v1 load is the sole
    // invalidity. Check both actual comparison outcomes, before region entry.
    for value in [0x80, 0x81] {
        let mut words = vec![0x7e040200 | value];
        words.extend(LOAD);
        words.extend([0x7d840480, VCCNZ | 1, 0x7e040289, WAIT]);
        rejects(&finish(words, 2), &[0; 32], 16);
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_mask_nonzero_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, data, want) in cases() {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &data)
                .unwrap(),
            want
        );
    }
}
