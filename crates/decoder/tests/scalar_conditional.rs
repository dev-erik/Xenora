// SPDX-License-Identifier: GPL-3.0-or-later
//! Independently specified scalar bit/conditional encodings and operand bounds.
use xenora_decoder::{Architecture::Gfx1030, Instruction, ScalarSource, decode, decode_at};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn source(selector: u32) -> Option<ScalarSource> {
    Some(match selector {
        0..=105 => ScalarSource::Register(selector as u8),
        128..=192 => ScalarSource::Immediate(selector - 128),
        193..=208 => ScalarSource::Immediate((192i32 - selector as i32) as u32),
        240..=248 => ScalarSource::Immediate(
            [
                0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
                0xc0800000, 0x3e22f983,
            ][(selector - 240) as usize],
        ),
        255 => ScalarSource::Immediate(0x89abcdef),
        _ => return None,
    })
}
fn sop1(op: u32, dst: u8, src: ScalarSource) -> Instruction {
    match op {
        5 => Instruction::ScalarConditionalMoveB32 { dst, src },
        27 | 29 => Instruction::ScalarBitSetB32 {
            dst,
            index: src,
            value: op == 29,
        },
        _ => unreachable!(),
    }
}
#[test]
fn sop1_selectors_and_all_destination_bounds() {
    for op in [5u32, 27, 29] {
        for dst in 0..128u32 {
            for selector in 0..256u32 {
                let code = bytes(&[0xbe800000 | dst << 16 | op << 8 | selector, 0x89abcdef]);
                let actual = decode_at(Gfx1030, &code, 0);
                if let Some(src) = source(selector).filter(|_| dst <= 105) {
                    let actual = actual.unwrap();
                    assert_eq!(actual.instruction, sop1(op, dst as u8, src));
                    assert_eq!(actual.byte_len, if selector == 255 { 8 } else { 4 });
                    assert_eq!(decode_at(Gfx1030, &code[..4], 0).is_ok(), selector != 255);
                } else {
                    assert!(actual.is_err(), "op={op} dst={dst} selector={selector}");
                }
            }
        }
    }
}
#[test]
fn bitcompare_both_source_domains_and_dual_literal_rejection() {
    for op in [12u32, 13] {
        for a in 0..256u32 {
            for b in 0..256u32 {
                let code = bytes(&[0xbf000000 | op << 16 | b << 8 | a, 0x89abcdef]);
                let actual = decode_at(Gfx1030, &code, 0);
                if let (Some(src), Some(index)) = (source(a), source(b))
                    && (a != 255 || b != 255)
                {
                    let actual = actual.unwrap();
                    assert_eq!(
                        actual.instruction,
                        Instruction::ScalarBitCompareB32 {
                            src,
                            index,
                            value: op == 13
                        }
                    );
                    assert_eq!(actual.byte_len, if a == 255 || b == 255 { 8 } else { 4 });
                    continue;
                }
                assert!(actual.is_err(), "op={op} src={a} index={b}");
            }
        }
    }
}
#[test]
fn cmovk_keeps_signed_immediate_and_four_byte_encoding() {
    for dst in 0..128u32 {
        for immediate in [0u16, 1, 0x7fff, 0x8000, 0xffff] {
            let code = bytes(&[0xb1000000 | dst << 16 | u32::from(immediate), 0xbf810000]);
            let actual = decode_at(Gfx1030, &code, 0);
            if dst <= 105 {
                let actual = actual.unwrap();
                assert_eq!(actual.byte_len, 4);
                assert_eq!(
                    actual.instruction,
                    Instruction::ScalarConditionalMoveImmediateI32 {
                        dst: dst as u8,
                        immediate: immediate as i16
                    }
                );
                assert_eq!(
                    actual.instruction.to_string(),
                    format!("s_cmovk_i32 s{dst}, {}", immediate as i16)
                );
                assert_eq!(decode(Gfx1030, &code).unwrap()[1].offset, 4);
            } else {
                assert!(actual.is_err());
            }
        }
    }
}
#[test]
fn literals_are_required_even_for_conditional_encodings_and_keep_offsets() {
    for first in [
        0xbea005ffu32,
        0xbea01bff,
        0xbea01dff,
        0xbf0c20ff,
        0xbf0cff20,
        0xbf0d20ff,
        0xbf0dff20,
    ] {
        let code = bytes(&[first, 0xbf810000, 0xbf810000]);
        for len in 0..8 {
            assert!(decode_at(Gfx1030, &code[..len], 0).is_err());
        }
        let decoded = decode(Gfx1030, &code).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!((decoded[0].offset, decoded[0].byte_len), (0, 8));
        assert_eq!((decoded[1].offset, decoded[1].byte_len), (8, 4));
        assert_eq!(decoded[1].instruction, Instruction::EndProgram);
    }
    for word in [
        0xbea00620u32,
        0xbea01c20,
        0xbea01e20,
        0xbf0e2120,
        0xbf0f2120,
    ] {
        assert!(decode_at(Gfx1030, &word.to_le_bytes(), 0).is_err());
    }
}
#[test]
fn literal_opcode_anchors_and_display() {
    for (word, assembly) in [
        (0xbea01b9fu32, "s_bitset0_b32 s32, 31"),
        (0xbea01d9f, "s_bitset1_b32 s32, 31"),
        (0xbf0c9f20, "s_bitcmp0_b32 s32, 31"),
        (0xbf0d9f20, "s_bitcmp1_b32 s32, 31"),
        (0xbea00521, "s_cmov_b32 s32, s33"),
        (0xb120ffff, "s_cmovk_i32 s32, -1"),
    ] {
        assert_eq!(
            decode_at(Gfx1030, &word.to_le_bytes(), 0)
                .unwrap()
                .instruction
                .to_string(),
            assembly
        );
    }
}
