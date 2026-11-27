// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte SRC0 programs and literal answers, AMD RDNA2 12.7/13.3.1.
//! Production compilation checks admission; neither reference supplies the oracle.
#[path = "../independent_ref.rs"]
mod independent_ref;

const OPS: [u32; 6] = [22, 24, 26, 27, 28, 29];
// A, B, answers in OPS order: logical right, arithmetic right, left, AND, OR, XOR.
const CASES: &[(u32, u32, [u32; 6])] = &[
    (
        0,
        0x80000001,
        [
            0x80000001, 0x80000001, 0x80000001, 0, 0x80000001, 0x80000001,
        ],
    ),
    (
        1,
        0x80000001,
        [0x40000000, 0xc0000000, 2, 1, 0x80000001, 0x80000000],
    ),
    (
        31,
        0x80000001,
        [1, 0xffffffff, 0x80000000, 1, 0x8000001f, 0x8000001e],
    ),
    (
        32,
        0x80000001,
        [
            0x80000001, 0x80000001, 0x80000001, 0, 0x80000021, 0x80000021,
        ],
    ),
    (
        33,
        0x80000001,
        [0x40000000, 0xc0000000, 2, 1, 0x80000021, 0x80000020],
    ),
    (
        63,
        0x80000001,
        [1, 0xffffffff, 0x80000000, 1, 0x8000003f, 0x8000003e],
    ),
    (
        0xffffffff,
        0x80000001,
        [
            1, 0xffffffff, 0x80000000, 0x80000001, 0xffffffff, 0x7ffffffe,
        ],
    ),
    (
        0x00ff00ff,
        0x12345678,
        [0, 0, 0, 0x00340078, 0x12ff56ff, 0x12cb5687],
    ),
    (0, 0, [0; 6]),
    (
        1,
        0xffffffff,
        [
            0x7fffffff, 0xffffffff, 0xfffffffe, 1, 0xffffffff, 0xfffffffe,
        ],
    ),
    (
        0,
        0x7fa12345,
        [
            0x7fa12345, 0x7fa12345, 0x7fa12345, 0, 0x7fa12345, 0x7fa12345,
        ],
    ),
];
fn op(opcode: u32, dst: u32, src0: u32, src1: u32) -> u32 {
    opcode << 25 | dst << 17 | src1 << 9 | src0
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn program(body: &[u32], dst: u32) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(body);
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
fn check(code: &[u8], input: &[u32], expected: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code)
        .unwrap_or_else(|e| panic!("production rejected positive {code:02x?}: {e:?}"));
    assert_eq!(
        xenora_ref::execute(code, input).unwrap(),
        expected,
        "primary {code:02x?}"
    );
    assert_eq!(
        independent_ref::execute(code, input).unwrap(),
        expected,
        "independent {code:02x?}"
    );
}
fn reject(code: &[u8], input: &[u32], at: usize, decode_error: bool) {
    let error = match xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code) {
        Ok(_) => panic!("production accepted {code:02x?}"),
        Err(e) => e,
    };
    match error {
        xenora_shader::CompileError::Decode(e) if decode_error => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } if !decode_error => {
            assert_eq!(offset, at)
        }
        e => panic!("wrong rejection kind/intent at {at:#x}: {e:?}"),
    }
    let error = xenora_ref::execute(code, input).unwrap_err();
    assert_eq!(error.offset, at, "{error:?}");
    let error = independent_ref::execute(code, input).unwrap_err();
    assert!(error.starts_with(&format!("{at:#x}:")), "{error}");
}

#[test]
fn literal_answers_cover_expanded_sources_endpoints_and_aliases() {
    for &(a, b, answers) in CASES {
        for (opcode, expected) in OPS.into_iter().zip(answers) {
            for dst in [1, 2, 3, 255] {
                for selector in [8, 105, 106, 255, 259, 496, 504, 511] {
                    let mut body = vec![0x7e0602ff, a]; // v3=A
                    if selector <= 106 {
                        body.extend([0xbe8003ff | selector << 16, a]);
                    }
                    if selector >= 496 {
                        body.extend([0x7e0002ff | (selector - 256) << 17, a]);
                    }
                    body.push(op(opcode, dst, selector, 1));
                    if selector == 255 {
                        body.push(a);
                    }
                    check(&program(&body, dst), &[b], &[expected]);
                }
            }
        }
    }
    // Both operands alias the destination. The one-bit count/data is symmetric
    // in value but source order is independently covered above.
    for (opcode, answer) in OPS.into_iter().zip([0, 0, 2, 1, 1, 0]) {
        check(&program(&[op(opcode, 1, 257, 1)], 1), &[1], &[answer]);
    }
}

#[test]
fn inline_integer_boundaries_and_all_float_word_selectors_are_raw_bits() {
    // B=ffffffff. Every shift result is literal; bitwise AND returns A,
    // OR returns ffffffff, XOR complements A.
    let cases: &[(u32, u32, [u32; 3])] = &[
        (128, 0, [0xffffffff, 0xffffffff, 0xffffffff]),
        (129, 1, [0x7fffffff, 0xffffffff, 0xfffffffe]),
        (159, 31, [1, 0xffffffff, 0x80000000]),
        (160, 32, [0xffffffff, 0xffffffff, 0xffffffff]),
        (161, 33, [0x7fffffff, 0xffffffff, 0xfffffffe]),
        (191, 63, [1, 0xffffffff, 0x80000000]),
        (192, 64, [0xffffffff, 0xffffffff, 0xffffffff]),
        (193, 0xffffffff, [1, 0xffffffff, 0x80000000]),
        (208, 0xfffffff0, [0x0000ffff, 0xffffffff, 0xffff0000]),
        (240, 0x3f000000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (241, 0xbf000000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (242, 0x3f800000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (243, 0xbf800000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (244, 0x40000000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (245, 0xc0000000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (246, 0x40800000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (247, 0xc0800000, [0xffffffff, 0xffffffff, 0xffffffff]),
        (248, 0x3e22f983, [0x1fffffff, 0xffffffff, 0xfffffff8]),
    ];
    for &(selector, a, shifts) in cases {
        for (opcode, answer) in OPS
            .into_iter()
            .zip([shifts[0], shifts[1], shifts[2], a, 0xffffffff, !a])
        {
            check(
                &program(&[op(opcode, 2, selector, 1)], 2),
                &[0xffffffff],
                &[answer],
            );
        }
    }
}

#[test]
fn exec_and_flags_preserve_old_words_and_unset_state() {
    for (opcode, answer) in OPS
        .into_iter()
        .zip([0x40000000, 0xc0000000, 2, 1, 0x80000021, 0x80000020])
    {
        for mask in [0u32, 0xffffffff, 1, 0x80000000, 0x55555555, 0xaaaaaaaa] {
            for selector in [8, 106, 161, 255, 259] {
                let mut body = vec![
                    0xbe8803a1, 0xbeea03a1, 0x7e0602a1, 0x7e0402ff, 0x13579bdf, 0xbefe03ff, mask,
                ];
                body.push(op(opcode, 2, selector, 1));
                if selector == 255 {
                    body.push(33);
                }
                body.push(0xbefe03c1);
                let expected: Vec<_> = (0..32)
                    .map(|i| {
                        if mask & (1 << i) != 0 {
                            answer
                        } else {
                            0x13579bdf
                        }
                    })
                    .collect();
                check(&program(&body, 2), &[0x80000001; 32], &expected);
            }
        }
        for flag in [false, true] {
            let code = program(
                &[
                    if flag { 0xbf068080 } else { 0xbf068180 },
                    op(opcode, 2, 161, 1),
                    0x7e060287,
                    0xbf840001,
                    0x7e060289,
                ],
                3,
            );
            check(&code, &[0], &[if flag { 9 } else { 7 }]);
        }
        reject(
            &program(&[op(opcode, 2, 161, 1), 0xbf840001, 0x7e040280], 2),
            &[0],
            16,
            false,
        );
        // SCC unset remains legal without a consumer; VCC read is unchanged.
        check(
            &program(
                &[0xbeea03ff, 0x55aa55aa, op(opcode, 2, 161, 1), 0x7e06026a],
                3,
            ),
            &[0],
            &[0x55aa55aa],
        );
        check(
            &program(&[0x7d8a0280, op(opcode, 2, 106, 1), 0x7e06026a], 3),
            &[0, 1, 0, 1],
            &[10; 4],
        );
        reject(
            &program(&[op(opcode, 2, 161, 1), 0x7e06026a], 3),
            &[0],
            16,
            false,
        );
    }
}

#[test]
fn zero_exec_skips_unknown_sources_nonzero_checks_sources_and_old_destination() {
    for opcode in OPS {
        for src in [20, 106, 259] {
            check(
                &program(
                    &[0x7e040281, 0xbefe0380, op(opcode, 2, src, 9), 0xbefe03c1],
                    2,
                ),
                &[0],
                &[1],
            );
            for mask in [1u32, 0x80000000, 0xffffffff] {
                reject(
                    &program(
                        &[
                            0x7e040281,
                            0xbefe03ff,
                            mask,
                            op(opcode, 2, src, 1),
                            0xbefe03c1,
                        ],
                        2,
                    ),
                    &[0],
                    24,
                    false,
                );
            }
        }
        reject(
            &program(&[0xbefe0381, op(opcode, 2, 129, 1), 0xbefe03c1], 2),
            &[0],
            16,
            false,
        );
        reject(&program(&[op(opcode, 2, 129, 9)], 2), &[0], 12, false);
        reject(&program(&[op(opcode, 0, 129, 1)], 1), &[0], 12, false);
        reject(&program(&[op(opcode, 2, 256, 1)], 2), &[0], 12, false);
        for word in [op(opcode, 2, 257, 1), op(opcode, 1, 129, 2)] {
            // Initialize v2 before loading v1, then read/write pending v1.
            let code = bytes(&[
                0x7e040281, 0xe0302000, 0x80000100, word, 0xbf8c3f70, 0xe0702000, 0x80010200,
                0xbbfd0000, 0xbf810000,
            ]);
            reject(&code, &[0], 12, false);
        }
    }
}

#[test]
fn reserved_selectors_modifiers_and_fp_domains_reject_while_e64_aliases_execute() {
    for (index, opcode) in OPS.into_iter().enumerate() {
        for src in (108..=125).chain(209..=239).chain(249..=254) {
            reject(&program(&[op(opcode, 2, src, 1)], 2), &[0], 12, true);
            reject(
                &program(&[0xbefe0380, op(opcode, 2, src, 1), 0xbefe03c1], 1),
                &[0],
                16,
                true,
            );
        }
        // Admitted high mask halves still require initialization under full EXEC.
        for src in [107, 127] {
            reject(&program(&[op(opcode, 2, src, 1)], 2), &[0], 12, false);
        }
        // New mask operands are outside the legacy third reference's domain.
        // Fixed answers check compiler admission and the primary byte interpreter;
        // every other existing three-way check remains intact.
        let positive_mask = |code: Vec<u8>, expected: u32| {
            xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
            assert_eq!(
                xenora_ref::execute(&code, &[0x80000001]).unwrap(),
                [expected]
            );
        };
        let exec_answers = [
            1, 0xffffffff, 0x80000000, 0x80000001, 0xffffffff, 0x7ffffffe,
        ];
        positive_mask(program(&[op(opcode, 2, 126, 1)], 2), exec_answers[index]);
        let one_answers = [0x40000000, 0xc0000000, 2, 1, 0x80000001, 0x80000000];
        for src in [107, 127] {
            positive_mask(
                program(&[0xbe800381 | src << 16, op(opcode, 2, src, 1)], 2),
                one_answers[index],
            );
        }
        for src in [107, 126, 127] {
            positive_mask(
                program(&[0xbefe0380, op(opcode, 2, src, 1), 0xbefe03c1], 1),
                0x80000001,
            );
        }
        for src in [233, 234, 249, 250] {
            reject(&bytes(&[op(opcode, 2, src, 1)]), &[0], 0, true);
            reject(&program(&[op(opcode, 2, src, 1), 0], 2), &[0], 12, true);
        }
        reject(&bytes(&[op(opcode, 2, 255, 1)]), &[0], 0, true);
        // Canonical integer e64 aliases are now supported; clamp stays excluded.
        let expected = match opcode {
            22 | 24 | 29 => 0,
            26 => 2,
            27 | 28 => 1,
            _ => unreachable!(),
        };
        check(
            &program(&[0xd5000002 | opcode << 16, 0x02020301], 2),
            &[1],
            &[expected],
        );
        reject(
            &program(&[0xd5008002 | opcode << 16, 0x02020301], 2),
            &[0],
            12,
            true,
        );
    }
    for opcode in [3, 8] {
        // FP add/mul must retain VGPR-only source decoding.
        for src in [8, 106, 129, 240, 255] {
            let mut body = vec![op(opcode, 2, src, 1)];
            if src == 255 {
                body.push(0x3f800000);
            }
            reject(&program(&body, 2), &[0], 12, true);
        }
    }
}

#[test]
fn runtime_vcc_regions_keep_old_vgpr_sources_but_reject_expanded_classes() {
    for opcode in OPS {
        for mask in [0u32, 1, 0xffffffff] {
            // Compare before changing EXEC, so both runtime branch outcomes
            // remain possible even when the integer instruction has zero EXEC.
            let old = program(
                &[
                    0x7e040287,
                    0x7e060281,
                    0x7d8a0280,
                    0xbefe03ff,
                    mask,
                    0xbf860001,
                    op(opcode, 2, 259, 1),
                    0xbefe03c1,
                ],
                2,
            );
            check(&old, &[0], &[7]);
            let answer = match opcode {
                22 | 24 => 0,
                26 => 2,
                27 | 28 => 1,
                _ => 0,
            };
            check(&old, &[1], &[if mask == 0 { 7 } else { answer }]);
            for selector in [8, 106, 129, 240, 255] {
                // All operands and the old destination are initialized; only
                // SRC0's class violates the runtime-region contract.
                let mut body = vec![
                    0xbe880381,
                    0x7e040287,
                    0x7d8a0280,
                    0xbefe03ff,
                    mask,
                    if selector == 255 {
                        0xbf860002
                    } else {
                        0xbf860001
                    },
                    op(opcode, 2, selector, 1),
                ];
                if selector == 255 {
                    body.push(1);
                }
                body.push(0xbefe03c1);
                let code = program(&body, 2);
                for input in [0, 1] {
                    reject(&code, &[input], 36, false);
                    let first = xenora_ref::execute(&code, &[input]).unwrap_err();
                    assert!(
                        first
                            .detail
                            .contains("runtime VCC region requires VGPR SRC0")
                    );
                    let second = independent_ref::execute(&code, &[input]).unwrap_err();
                    assert!(second.contains("runtime VCC region requires VGPR SRC0"));
                    let production =
                        xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code)
                            .unwrap_err();
                    assert!(
                        production
                            .to_string()
                            .contains("runtime s_cbranch_vccz integer SRC0 must be a VGPR"),
                        "{production}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_legacy_sources_batch_runtime_words_masks_and_flag_observation() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    let anchors = [0u32, 1, 0x80000000, 0xffffffff, 0x12345678, 0x7fa12345];
    let inputs: Vec<_> = (0..32).map(|lane| anchors[lane % anchors.len()]).collect();
    // Literal answers for A=1. Rows follow OPS; columns follow runtime inputs.
    let answers = [
        [0u32, 0, 0x40000000, 0x7fffffff, 0x091a2b3c, 0x3fd091a2],
        [0, 0, 0xc0000000, 0xffffffff, 0x091a2b3c, 0x3fd091a2],
        [0, 2, 0, 0xfffffffe, 0x2468acf0, 0xff42468a],
        [0, 1, 0, 1, 0, 1],
        [1, 1, 0x80000001, 0xffffffff, 0x12345679, 0x7fa12345],
        [1, 0, 0x80000001, 0xfffffffe, 0x12345679, 0x7fa12344],
    ];
    // 72 bounded dispatches: every source gets full EXEC plus one mask.
    // A full wave includes lane 31; six literal input anchors repeat within it.
    for (column, opcode) in OPS.into_iter().enumerate() {
        for (index, selector) in [8, 105, 106, 129, 255, 259].into_iter().enumerate() {
            for mask in [
                0xffffffffu32,
                [0, 1, 0xaaaaaaaa, 0x55555555, 0x80000000, 0][index],
            ] {
                let flag = index % 2 == 0;
                let mut body = vec![
                    0xbe880381,
                    0xbee90381,
                    0xbeea0381,
                    0x7e060281,
                    0x7e0402ff,
                    0x13579bdf,
                    if flag { 0xbf068080 } else { 0xbf068180 },
                    0xbefe03ff,
                    mask,
                    op(opcode, 2, selector, 1),
                ];
                if selector == 255 {
                    body.push(1);
                }
                body.extend([
                    0xbefe03c1,
                    0x7e08026a, // v4=preserved VCC (1)
                    0xbf840001,
                    op(29, 2, 260, 2),
                ]); // if SCC was set, XOR with VCC
                let code = program(&body, 2);
                let expected: Vec<_> = (0..inputs.len())
                    .map(|lane| {
                        let answer = answers[column][lane % anchors.len()];
                        let value = if mask & (1 << lane) != 0 {
                            answer
                        } else {
                            0x13579bdf
                        };
                        if flag { value ^ 1 } else { value }
                    })
                    .collect();
                check(&code, &inputs, &expected);
                let compiled =
                    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
                assert_eq!(
                    gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &inputs)
                        .unwrap(),
                    expected,
                    "op={opcode} selector={selector} mask={mask:#x}"
                );
            }
        }
    }
}
