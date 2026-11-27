// SPDX-License-Identifier: GPL-3.0-or-later
//! Original exact-bit cases for RDNA2 inline selectors, independent of production tables.
use xenora_decoder::{Architecture::Gfx1030, Instruction, ScalarSource, VectorSource, decode_at};

const WORDS: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
// (instruction with empty source selector, source-field shift).
const FORMS: [(u32, u32); 9] = [
    (0xbea00300, 0),
    (0xbea00700, 0),
    (0x80202100, 0),
    (0x80200021, 8),
    (0xbf062100, 0),
    (0xbf060021, 8),
    (0x7e100200, 0),
    (0x4a101200, 0),
    (0x7d841200, 0),
];

#[test]
fn inline_float_sources_have_exact_words_and_do_not_consume_literals() {
    for (index, expected) in WORDS.into_iter().enumerate() {
        let selector = 240 + index as u32;
        for (form, shift) in FORMS {
            let bytes = (form | selector << shift).to_le_bytes();
            let decoded = decode_at(Gfx1030, &bytes, 0).unwrap();
            assert_eq!(decoded.byte_len, 4);
            let actual = match decoded.instruction {
                Instruction::ScalarMoveB32 {
                    src: ScalarSource::Immediate(x),
                    ..
                }
                | Instruction::ScalarUnary {
                    src: ScalarSource::Immediate(x),
                    ..
                } => x,
                Instruction::ScalarAlu { src0, src1, .. }
                | Instruction::ScalarCompare { src0, src1, .. } => {
                    let ScalarSource::Immediate(x) = (if shift == 0 { src0 } else { src1 }) else {
                        panic!("missing scalar immediate")
                    };
                    x
                }
                Instruction::MoveB32 {
                    src: VectorSource::Immediate(x),
                    ..
                }
                | Instruction::VectorAlu {
                    src0: VectorSource::Immediate(x),
                    ..
                }
                | Instruction::VectorCompare {
                    src0: VectorSource::Immediate(x),
                    ..
                } => x,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(actual, expected, "form {form:x} selector {selector}");
            let literal: Vec<_> = [form | 255 << shift, expected]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            let decoded_literal = decode_at(Gfx1030, &literal, 0).unwrap();
            assert_eq!(decoded_literal.byte_len, 8);
            assert_eq!(decoded.instruction, decoded_literal.instruction);
            assert!(decode_at(Gfx1030, &literal[..4], 0).is_err());
        }
    }
}

#[test]
fn inline_selectors_do_not_alias_vector_registers_or_reserved_selectors() {
    for selector in 240..=248u32 {
        for form in [0x7e100200u32, 0x4a101200, 0x7d841200] {
            let decoded = decode_at(Gfx1030, &(form | 256 | selector).to_le_bytes(), 0).unwrap();
            let register = match decoded.instruction {
                Instruction::MoveB32 {
                    src: VectorSource::Register(r),
                    ..
                }
                | Instruction::VectorAlu {
                    src0: VectorSource::Register(r),
                    ..
                }
                | Instruction::VectorCompare {
                    src0: VectorSource::Register(r),
                    ..
                } => r,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(register, selector as u8);
        }
    }
    for selector in [209, 232, 233, 234, 235, 239, 249, 250, 251, 252, 253, 254] {
        for (form, shift) in FORMS {
            assert!(decode_at(Gfx1030, &(form | selector << shift).to_le_bytes(), 0).is_err());
        }
    }
}

#[test]
fn inline_and_literal_sources_keep_their_distinct_lengths() {
    for (i, expected) in WORDS.into_iter().enumerate() {
        let selector = 240 + i as u32;
        for (a, b, length) in [
            (selector, selector, 4),
            (selector, 255, 8),
            (255, selector, 8),
        ] {
            let bytes: Vec<_> = [0x80200000 | b << 8 | a, expected]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            let decoded = decode_at(Gfx1030, &bytes, 0).unwrap();
            assert_eq!(decoded.byte_len, length);
            let Instruction::ScalarAlu { src0, src1, .. } = decoded.instruction else {
                panic!()
            };
            assert_eq!(src0, ScalarSource::Immediate(expected));
            assert_eq!(src1, ScalarSource::Immediate(expected));
        }
    }
    let both: Vec<_> = [0x8020ffffu32, 1, 2]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    assert!(decode_at(Gfx1030, &both, 0).is_err());
}
