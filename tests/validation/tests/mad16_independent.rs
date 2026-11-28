// SPDX-License-Identifier: GPL-3.0-or-later
//! Handwritten answers from public AMD RDNA2 70648 p185.
//! Nonzero modifiers remain outside the admitted subset.
//! https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture
//! Encoding checked against amdgpu_isa_rdna2.xml SHA256
//! d671ecbc36543674ab59e9b2feffd56718fd2137e7ad913c314f2e2508b9f4f7.
//! No production evaluator or generated opcode table supplies expected results.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};

// AMD RDNA2 p185 and Table82: opcode883/885, zero modifiers.
const UNSIGNED: u32 = 0xd7730000;
const SIGNED: u32 = 0xd7750000;
const ANSWERS: &[(u32, u32, u32, u32, u32)] = &[
    (UNSIGNED, 0xffff, 0xffff, 0, 0xfffe0001),
    (UNSIGNED, 0xffff, 0xffff, 0x20000, 1),
    (UNSIGNED, 0xdead0002, 0xbeef0003, 0x12345678, 0x1234567e),
    (UNSIGNED, 0x8000, 2, 0xffff0000, 0),
    (SIGNED, 0xffff, 2, 0, 0xfffffffe),
    (SIGNED, 0x8000, 0xffff, 0, 0x8000),
    (SIGNED, 0x8000, 0x8000, 0, 0x40000000),
    (SIGNED, 0x7fff, 0x7fff, 0, 0x3fff0001),
    (SIGNED, 1, 1, 0x7fffffff, 0x80000000),
    (SIGNED, 0xffff, 1, 0x80000000, 0x7fffffff),
    (SIGNED, 0x1234ffff, 0xabcd0002, 0x12345678, 0x12345676),
    (SIGNED, 0xffff, 0xffff, 0xffffffff, 0),
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn instruction(op: u32, dst: u32, src: [u32; 3]) -> [u32; 2] {
    [op | dst, src[0] | src[1] << 9 | src[2] << 18]
}
fn program(body: &[u32], dst: u32) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
struct Case {
    code: Vec<u8>,
    input: Vec<u32>,
    expected: Vec<u32>,
}
fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for &(op, a, b, c, want) in ANSWERS {
        for dst in [1, 2, 3, 4, 255] {
            for mask in [u32::MAX, 0x80000005, 0] {
                let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
                let old = match dst {
                    1 => a,
                    2 => b,
                    3 => c,
                    _ => 0x13579bdf,
                };
                if dst > 3 {
                    body.extend([0x7e0002ff | dst << 17, old]);
                }
                body.extend([0xbefe03ff, mask]);
                body.extend(instruction(op, dst, [257, 258, 259]));
                body.push(0xbefe03c1);
                out.push(Case {
                    code: program(&body, dst),
                    input: vec![a; 35],
                    expected: (0..35)
                        .map(|i| {
                            if mask & (1 << (i % 32)) != 0 {
                                want
                            } else {
                                old
                            }
                        })
                        .collect(),
                });
            }
        }
        // Each source position independently uses SGPR or literal while all
        // other operands remain VGPRs; at most one scalar bus identity.
        for pos in 0..3 {
            for literal in [false, true] {
                let values = [a, b, c];
                let mut src = [257, 258, 259];
                let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
                if literal {
                    src[pos] = 255;
                } else {
                    src[pos] = 8;
                    body.extend([0xbe8803ff, values[pos]]);
                }
                body.extend(instruction(op, 4, src));
                if literal {
                    body.push(values[pos]);
                }
                out.push(Case {
                    code: program(&body, 4),
                    input: vec![a; 32],
                    expected: vec![want; 32],
                });
            }
        }
    }
    for op in [UNSIGNED, SIGNED] {
        for flag in [false, true] {
            let mut body = vec![0x7e0a0280, if flag { 0xbf068080 } else { 0xbf068180 }];
            body.extend(instruction(op, 4, [129, 129, 128]));
            body.extend([0xbf850002, 0x7e0a02ff, 1]);
            out.push(Case {
                code: program(&body, 5),
                input: vec![0; 32],
                expected: vec![u32::from(!flag); 32],
            });
        }
        for mask in [0, 0x87654321, u32::MAX] {
            let mut body = vec![0xbeea03ff, mask];
            body.extend(instruction(op, 4, [129, 129, 128]));
            body.push(0x7e0a026a);
            out.push(Case {
                code: program(&body, 5),
                input: vec![0; 32],
                expected: vec![mask; 32],
            });
        }
    }
    assert_eq!(out.len(), 262);
    out
}
// Table 20 and p185: columns are unsigned/signed HALF times one,
// FP32 raw addend plus one, and unsigned/signed HALF squared plus FP32.
// These fixed answers do not call either evaluator or a production table.
const FLOAT_ANSWERS: &[(u32, u32, u32, u32, u32)] = &[
    (0x00003800, 0x00003800, 0x3f000001, 0x4b400000, 0x4b400000),
    (0x0000b800, 0xffffb800, 0xbf000001, 0x43400000, 0xd3400000),
    (0x00003c00, 0x00003c00, 0x3f800001, 0x4d900000, 0x4d900000),
    (0x0000bc00, 0xffffbc00, 0xbf800001, 0x49900000, 0xd1900000),
    (0x00004000, 0x00004000, 0x40000001, 0x50000000, 0x50000000),
    (0x0000c000, 0xffffc000, 0xc0000001, 0x50000000, 0xd0000000),
    (0x00004400, 0x00004400, 0x40800001, 0x52900000, 0x52900000),
    (0x0000c400, 0xffffc400, 0xc0800001, 0x56900000, 0xce900000),
    (0x00003118, 0x00003118, 0x3e22f984, 0x478d2bc3, 0x478d2bc3),
];
fn float_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for (index, &(u, s, add, mixed_u, mixed_s)) in FLOAT_ANSWERS.iter().enumerate() {
        let selector = 240 + index as u32;
        for op in [UNSIGNED, SIGNED] {
            for position in 0..4 {
                let mut src = [129, 129, 128];
                let expected = if position == 3 {
                    src = [selector; 3];
                    if op == UNSIGNED { mixed_u } else { mixed_s }
                } else {
                    src[position] = selector;
                    if position == 2 {
                        add
                    } else if op == UNSIGNED {
                        u
                    } else {
                        s
                    }
                };
                out.push(Case {
                    code: program(&instruction(op, 4, src), 4),
                    input: vec![0; 32],
                    expected: vec![expected; 32],
                });
            }
        }
    }
    assert_eq!(out.len(), 72);
    out
}
#[test]
fn named_float_sources_use_each_operand_width() {
    for case in float_cases() {
        check(&case, None);
    }
}
fn check(case: &Case, gpu: Option<&xenora_metal::NativeCompute>) {
    assert_eq!(
        xenora_ref::execute(&case.code, &case.input).unwrap(),
        case.expected,
        "primary {:02x?}",
        case.code
    );
    assert_eq!(
        independent_ref::execute(&case.code, &case.input).unwrap(),
        case.expected,
        "secondary {:02x?}",
        case.code
    );
    let shader = compile(Gfx1030, &case.code).unwrap();
    if let Some(gpu) = gpu {
        assert_eq!(
            gpu.run_compiled_words(&shader, &case.input).unwrap(),
            case.expected
        );
    }
}
#[test]
fn known_answers_aliases_operand_classes_and_retained_lanes() {
    for case in cases() {
        check(&case, None);
    }
}
#[test]
fn shared_literal_and_inline_sources_are_single_values() {
    for op in [UNSIGNED, SIGNED] {
        for mask in 0..8 {
            let src = std::array::from_fn(|i| if mask & (1 << i) != 0 { 255 } else { 129 });
            let mut body = instruction(op, 4, src).to_vec();
            if mask != 0 {
                body.push(1);
            }
            check(
                &Case {
                    code: program(&body, 4),
                    input: vec![0; 1],
                    expected: vec![2; 1],
                },
                None,
            );
        }
    }
}
#[test]
fn zero_modifiers_lengths_scalar_bus_and_unready_sources_are_enforced() {
    for op in [UNSIGNED, SIGNED] {
        for bit in 8..16 {
            let mut pair = instruction(op, 4, [257, 258, 259]);
            pair[0] |= 1 << bit;
            let raw = bytes(&pair);
            assert!(xenora_ref::decode(&raw).is_err());
            assert!(compile(Gfx1030, &program(&pair, 4)).is_err());
            assert!(independent_ref::execute(&program(&pair, 4), &[0]).is_err());
        }
        for bit in 27..32 {
            let mut pair = instruction(op, 4, [257, 258, 259]);
            pair[1] |= 1 << bit;
            assert!(xenora_ref::decode(&bytes(&pair)).is_err());
            assert!(compile(Gfx1030, &program(&pair, 4)).is_err());
            assert!(independent_ref::execute(&program(&pair, 4), &[0]).is_err());
        }
        assert!(xenora_ref::decode(&bytes(&instruction(op, 4, [8, 9, 10]))).is_err());
        let literal = bytes(&instruction(op, 4, [255, 128, 128]));
        for n in [4, 8] {
            assert!(xenora_ref::decode(&literal[..n]).is_err());
        }
        let unready = program(&instruction(op, 4, [257, 258, 259]), 4);
        assert!(xenora_ref::execute(&unready, &[0]).is_err());
        assert!(compile(Gfx1030, &unready).is_err());
    }
}
#[test]
#[ignore = "lead opt-in only: native Metal API and shader validation required"]
fn native_mad16_compute() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    check(&fixture_case(), Some(&gpu));
    for case in cases().into_iter().chain(float_cases()) {
        check(&case, Some(&gpu));
    }
}

#[test]
fn width_specific_inline_constants_and_full_width_addend() {
    for op in [UNSIGNED, SIGNED] {
        // Float inline in the 32-bit addend is raw F32 bits: 1.0 + integer product1.
        check(
            &Case {
                code: program(&instruction(op, 4, [129, 129, 242]), 4),
                input: vec![0],
                expected: vec![0x3f800001],
            },
            None,
        );
        // Integer inline -1: unsigned lowhalf65535 versus signed -1.
        check(
            &Case {
                code: program(&instruction(op, 4, [193, 130, 128]), 4),
                input: vec![0],
                expected: vec![if op == SIGNED { 0xfffffffe } else { 0x1fffe }],
            },
            None,
        );
    }
}

fn fixture_case() -> Case {
    // For lowhalf x: unsigned product65535*x plus signed product(-1)*signed16(x),
    // with two accumulators0x7fffffff. Hand-reduced values cross the sign boundary.
    Case {
        code: include_bytes!("../../../fixtures/rdna2/vop3-mad16.bin").to_vec(),
        input: vec![0, 1, 0x8000, 0xffff, 0xdead0002],
        expected: vec![0xfffffffe, 0xfffc, 0x7ffffffe, 0xfffe0000, 0x1fffa],
    }
}
#[test]
fn assembled_fixture_known_answers() {
    check(&fixture_case(), None);
}

// Literal, unsigned answers for source masks1..7, signed answers for masks1..7.
// Unselected source words are 2,3,5. Multiplicands use low16; accumulator uses full32.
const WIDE_LITERAL_ANSWERS: &[(u32, [u32; 7], [u32; 7])] = &[
    (
        0xffff8000,
        [
            0x00018005, 0x00010005, 0x40000005, 0xffff8006, 0x00010000, 0x00008000, 0x3fff8000,
        ],
        [
            0xfffe8005, 0xffff0005, 0x40000005, 0xffff8006, 0xfffe0000, 0xfffe8000, 0x3fff8000,
        ],
    ),
    (
        0xffffffff,
        [
            0x00030002, 0x00020003, 0xfffe0006, 0x00000005, 0x0002fffc, 0x0001fffd, 0xfffe0000,
        ],
        [
            0x00000002, 0x00000003, 0x00000006, 0x00000005, 0xfffffffc, 0xfffffffd, 0x00000000,
        ],
    ),
    (
        0x12345678,
        [
            0x0001036d, 0x0000acf5, 0x1d34d845, 0x1234567e, 0x123559e0, 0x12350368, 0x2f692eb8,
        ],
        [
            0x0001036d, 0x0000acf5, 0x1d34d845, 0x1234567e, 0x123559e0, 0x12350368, 0x2f692eb8,
        ],
    ),
    (
        0x80000000,
        [
            0x00000005, 0x00000005, 0x00000005, 0x80000006, 0x80000000, 0x80000000, 0x80000000,
        ],
        [
            0x00000005, 0x00000005, 0x00000005, 0x80000006, 0x80000000, 0x80000000, 0x80000000,
        ],
    ),
    (
        0x00010000,
        [
            0x00000005, 0x00000005, 0x00000005, 0x00010006, 0x00010000, 0x00010000, 0x00010000,
        ],
        [
            0x00000005, 0x00000005, 0x00000005, 0x00010006, 0x00010000, 0x00010000, 0x00010000,
        ],
    ),
];
fn wide_literal_cases() -> Vec<Case> {
    let mut out = Vec::new();
    for &(literal, unsigned, signed) in WIDE_LITERAL_ANSWERS {
        for (op, answers) in [(UNSIGNED, unsigned), (SIGNED, signed)] {
            for mask in 1..8 {
                let mut sources = [257, 258, 259];
                for (i, source) in sources.iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        *source = 255;
                    }
                }
                for dst in [1, 2, 3, 255] {
                    for exec in [u32::MAX, 0x80000005, 0] {
                        let old = match dst {
                            1 => 2,
                            2 => 3,
                            3 => 5,
                            _ => 0x13579bdf,
                        };
                        let mut body = vec![0x7e040283, 0x7e060285];
                        if dst == 255 {
                            body.extend([0x7ffe02ff, old]);
                        }
                        body.extend([0xbefe03ff, exec]);
                        body.extend(instruction(op, dst, sources));
                        body.extend([literal, 0xbefe03c1]);
                        out.push(Case {
                            code: program(&body, dst),
                            input: vec![2; 35],
                            expected: (0..35)
                                .map(|i| {
                                    if exec & (1 << (i % 32)) != 0 {
                                        answers[mask - 1]
                                    } else {
                                        old
                                    }
                                })
                                .collect(),
                        });
                    }
                }
            }
        }
    }
    assert_eq!(out.len(), 840);
    out
}
#[test]
fn shared_full_word_literals_keep_operand_widths() {
    for case in wide_literal_cases() {
        check(&case, None);
    }
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_mad16_wide_literals() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for case in wide_literal_cases() {
        check(&case, Some(&gpu));
    }
    check(&wide_literal_fixture(), Some(&gpu));
}

fn wide_literal_fixture() -> Case {
    Case {
        code: include_bytes!("../../../fixtures/rdna2/vop3-mad16-wide-literal.bin").to_vec(),
        input: vec![0, 1, 0x8000, 0xffff, 0xdead0002],
        expected: vec![0xffff8000, 0xffff0000, 0x3fff8000, 0, 0xfffe8000],
    }
}
#[test]
fn assembled_negative_shared_literal_keeps_full_addend() {
    check(&wide_literal_fixture(), None);
}
