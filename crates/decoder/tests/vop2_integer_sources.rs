// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal VOP2 fields and source expectations, independent of generated metadata.
use xenora_decoder::{Architecture::Gfx1030, Instruction, VectorSource, decode, decode_at};

const OPS: [u32; 6] = [22, 24, 26, 27, 28, 29];
const FLOAT_WORDS: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn instruction(op: u32, dst: u8, src0: VectorSource, src1: u8) -> Instruction {
    match op {
        22 => Instruction::VectorLshrrevB32 { dst, src0, src1 },
        24 => Instruction::VectorAshrrevI32 { dst, src0, src1 },
        26 => Instruction::VectorLshlrevB32 { dst, src0, src1 },
        27 => Instruction::VectorAndB32 { dst, src0, src1 },
        28 => Instruction::VectorOrB32 { dst, src0, src1 },
        29 => Instruction::VectorXorB32 { dst, src0, src1 },
        _ => unreachable!(),
    }
}

#[test]
fn every_selector_has_exact_class_value_and_length() {
    for op in OPS {
        for selector in 0..512u32 {
            let expected = match selector {
                0..=107 | 126..=127 => Some(VectorSource::Scalar(selector as u8)),
                128..=192 => Some(VectorSource::Immediate(selector - 128)),
                193..=208 => Some(VectorSource::Immediate((192i32 - selector as i32) as u32)),
                240..=248 => Some(VectorSource::Immediate(
                    FLOAT_WORDS[(selector - 240) as usize],
                )),
                255 => Some(VectorSource::Immediate(0x7fc12345)),
                256..=511 => Some(VectorSource::Register((selector - 256) as u8)),
                _ => None,
            };
            for (dst, src1) in [(0u8, 0u8), (5, 2), (255, 255)] {
                let word = op << 25 | u32::from(dst) << 17 | u32::from(src1) << 9 | selector;
                let code = bytes(&[word, 0x7fc12345]);
                let actual = decode_at(Gfx1030, &code, 0);
                if let Some(src0) = expected {
                    let actual = actual.unwrap();
                    assert_eq!(actual.instruction, instruction(op, dst, src0, src1));
                    assert_eq!(actual.byte_len, if selector == 255 { 8 } else { 4 });
                    if selector != 255 {
                        assert_eq!(decode_at(Gfx1030, &code[..4], 0).unwrap(), actual);
                    }
                } else {
                    assert!(actual.is_err(), "op={op} selector={selector}");
                    assert!(decode_at(Gfx1030, &code[..4], 0).is_err());
                }
            }
        }
    }
}

#[test]
fn literal_payloads_truncation_and_stream_offsets() {
    for op in OPS {
        for payload in [0, 1, 0xffffffff, 0x80000000, 0x7fc12345, 0xbf810000] {
            let word = op << 25 | 5 << 17 | 2 << 9 | 255;
            let code = bytes(&[word, payload, 0xbf810000]);
            for length in 0..8 {
                assert!(decode_at(Gfx1030, &code[..length], 0).is_err());
            }
            let decoded = decode(Gfx1030, &code).unwrap();
            assert_eq!(decoded.len(), 2);
            assert_eq!(
                decoded[0].instruction,
                instruction(op, 5, VectorSource::Immediate(payload), 2)
            );
            assert_eq!((decoded[1].offset, decoded[1].byte_len), (8, 4));
            assert_eq!(decoded[1].instruction, Instruction::EndProgram);
        }
    }
}

#[test]
fn extension_and_e64_modifiers_remain_rejected() {
    for op in OPS {
        for selector in [233, 234, 249, 250, 254] {
            let first = op << 25 | 5 << 17 | 2 << 9 | selector;
            for extension in [0, 0xff00e401, 0x06060601, 0xffffffff] {
                let code = bytes(&[first, extension]);
                for length in 4..=8 {
                    assert!(decode_at(Gfx1030, &code[..length], 0).is_err());
                }
            }
        }
        // These six pinned e64 aliases admit canonical unmodified fields.
        let first = 0xd4000000 | (256 + op) << 16 | 5;
        let second = 128 << 18 | 258 << 9 | 257;
        for first_modifier in [0, 1 << 8, 1 << 15] {
            for second_modifier in [0, 1 << 27, 1 << 29] {
                if first_modifier == 0 && second_modifier == 0 {
                    assert!(decode_at(Gfx1030, &bytes(&[first, second]), 0).is_ok());
                    continue;
                }
                assert!(
                    decode_at(
                        Gfx1030,
                        &bytes(&[first | first_modifier, second | second_modifier]),
                        0
                    )
                    .is_err()
                );
            }
        }
    }
}

#[test]
fn floating_vop2_sources_stay_vgpr_only() {
    for op in [3u32, 8] {
        for selector in 0..512u32 {
            let code = bytes(&[op << 25 | 5 << 17 | 2 << 9 | selector, 0x12345678]);
            let decoded = decode_at(Gfx1030, &code, 0);
            if selector < 256 {
                assert!(decoded.is_err(), "FP op={op} selector={selector}");
            } else {
                let decoded = decoded.unwrap();
                assert_eq!(decoded.byte_len, 4);
                let src0 = (selector - 256) as u8;
                let expected = if op == 3 {
                    Instruction::VectorAddF32 {
                        dst: 5,
                        src0,
                        src1: 2,
                    }
                } else {
                    Instruction::VectorMulF32 {
                        dst: 5,
                        src0,
                        src1: 2,
                    }
                };
                assert_eq!(decoded.instruction, expected);
            }
        }
    }
}

#[test]
fn assembly_preserves_source_register_class_and_shift_order() {
    for (op, name) in OPS.into_iter().zip([
        "v_lshrrev_b32_e32",
        "v_ashrrev_i32_e32",
        "v_lshlrev_b32_e32",
        "v_and_b32_e32",
        "v_or_b32_e32",
        "v_xor_b32_e32",
    ]) {
        for (selector, spelling) in [
            (0, "s0"),
            (105, "s105"),
            (106, "vcc_lo"),
            (256, "v0"),
            (511, "v255"),
        ] {
            let decoded = decode_at(
                Gfx1030,
                &(op << 25 | 5 << 17 | 2 << 9 | selector).to_le_bytes(),
                0,
            )
            .unwrap();
            assert_eq!(
                decoded.instruction.to_string(),
                format!("{name} v5, {spelling}, v2")
            );
        }
    }
}
