// SPDX-License-Identifier: GPL-3.0-or-later
//! Handwritten answers from public AMD RDNA2 70648 pp170,178,181.
//! https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture
//! Encoding checked against amdgpu_isa_rdna2.xml SHA256
//! d671ecbc36543674ab59e9b2feffd56718fd2137e7ad913c314f2e2508b9f4f7.
//! No production evaluator or generated opcode table supplies expected results.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};

// First words independently derived from ENC_VOP3 OP[25:16], VDST[7:0].
const XAD: u32 = 0xd7450000;
const LERP: u32 = 0xd54d0000;
const MSAD: u32 = 0xd5710000;
// (opcode word, a, b, c, complete result), including asymmetric zero masking.
const ANSWERS: &[(u32, u32, u32, u32, u32)] = &[
    (XAD, 0xffffffff, 0, 1, 0),
    (XAD, 0xaaaaaaaa, 0x55555555, 2, 1),
    (XAD, 0x80000000, 1, 0x7fffffff, 0),
    (XAD, 0x12345678, 0x00ff00ff, 0x01010101, 0x13cc5788),
    (LERP, 0, 0xffffffff, 0, 0x7f7f7f7f),
    (LERP, 0, 0xffffffff, 0x01010101, 0x80808080),
    (LERP, 0, 0xffffffff, 0xfefefefe, 0x7f7f7f7f),
    (LERP, 0, 0xffffffff, 0x01000001, 0x807f7f80),
    (LERP, 0x00ff01fe, 0xff000201, 0x00010100, 0x7f80027f),
    (LERP, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff),
    (MSAD, 0xffffffff, 0, 7, 7),
    (MSAD, 0, 0xffffffff, 7, 1027),
    (MSAD, 0, 0xffffffff, 0xffffffff, 1019),
    (MSAD, 0x04030201, 0x00030001, 9, 9),
    (MSAD, 0x00030001, 0x04030201, 9, 15),
    (MSAD, 0xff00ff00, 0x00010001, 0xfffffffe, 0),
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
    // Isolate every byte and every rounding bit. Bits other than 0/8/16/24
    // must not influence the average; bytes may not carry into neighbours.
    for byte in 0..4 {
        let b = 1u32 << (byte * 8);
        for round_bit in 0..32 {
            let round = 1 << round_bit;
            let want = if round_bit == byte * 8 { b } else { 0 };
            let mut body = vec![0x7e0402ff, b, 0x7e0602ff, round];
            body.extend(instruction(LERP, 4, [257, 258, 259]));
            out.push(Case {
                code: program(&body, 4),
                input: vec![0; 32],
                expected: vec![want; 32],
            });
        }
        for (a, reference, want) in [(0, b, 1), (b, 0, 0)] {
            let mut body = vec![0x7e0402ff, reference];
            body.extend(instruction(MSAD, 4, [257, 258, 128]));
            out.push(Case {
                code: program(&body, 4),
                input: vec![a; 32],
                expected: vec![want; 32],
            });
        }
    }
    out
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
    for op in [XAD, LERP, MSAD] {
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
                    expected: vec![1; 1],
                },
                None,
            );
        }
    }
}
#[test]
fn zero_modifiers_lengths_scalar_bus_and_unready_sources_are_enforced() {
    for op in [XAD, LERP, MSAD] {
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
fn native_packed_integer_compute() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for case in cases() {
        check(&case, Some(&gpu));
    }
    check(&fixture_case(), Some(&gpu));
}

fn fixture_case() -> Case {
    Case {
        code: include_bytes!("../../../fixtures/rdna2/vop3-byte-arithmetic.bin").to_vec(),
        input: vec![0, 0x0100ff00, 0xffffffff, 0x01010101],
        expected: vec![0x04038202, 0x03030102, 0x8182037f, 0x03058102],
    }
}

#[test]
fn assembled_fixture_has_literal_whole_program_answers() {
    check(&fixture_case(), None);
}
