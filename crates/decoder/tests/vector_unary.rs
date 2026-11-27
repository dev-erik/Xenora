// SPDX-License-Identifier: GPL-3.0-or-later
//! Original VOP1 field and source-domain checks, independent of generated dispatch.
use xenora_decoder::{
    Architecture::Gfx1030, Instruction, VectorSource, VectorUnaryOp, decode, decode_at,
};
const FORMS: [(u32, VectorUnaryOp, &str); 5] = [
    (55, VectorUnaryOp::NotB32, "v_not_b32_e32"),
    (56, VectorUnaryOp::BfrevB32, "v_bfrev_b32_e32"),
    (57, VectorUnaryOp::FfbhU32, "v_ffbh_u32_e32"),
    (58, VectorUnaryOp::FfblB32, "v_ffbl_b32_e32"),
    (59, VectorUnaryOp::FfbhI32, "v_ffbh_i32_e32"),
];
const FLOATS: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
#[test]
fn vector_unary_all_nine_bit_source_fields_have_exact_domains_and_words() {
    for (opcode, op, name) in FORMS {
        for selector in 0..512u32 {
            let code: Vec<_> = [0x7e000000 | 255 << 17 | opcode << 9 | selector, 0x12345678]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            let expected = match selector {
                0..=107 | 126..=127 => Some(VectorSource::Scalar(selector as u8)),
                128..=192 => Some(VectorSource::Immediate(selector - 128)),
                193..=208 => Some(VectorSource::Immediate(0u32.wrapping_sub(selector - 192))),
                240..=248 => Some(VectorSource::Immediate(FLOATS[(selector - 240) as usize])),
                255 => Some(VectorSource::Immediate(0x12345678)),
                256..=511 => Some(VectorSource::Register((selector - 256) as u8)),
                _ => None,
            };
            let actual = decode_at(Gfx1030, &code, 0);
            if let Some(src) = expected {
                let decoded = actual.unwrap();
                assert_eq!(decoded.byte_len, if selector == 255 { 8 } else { 4 });
                assert_eq!(
                    decoded.instruction,
                    Instruction::VectorUnary { op, dst: 255, src }
                );
                assert_eq!(decoded.instruction.mnemonic(), name);
            } else {
                assert!(actual.is_err(), "op={opcode} source={selector}");
            }
        }
    }
}
#[test]
fn vector_unary_literals_keep_following_offsets_and_reject_truncation() {
    for (opcode, _, _) in FORMS {
        let code: Vec<_> = [
            0x7e000000 | 5 << 17 | opcode << 9 | 255,
            0x12345678,
            0xbf810000,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        for length in 0..8 {
            assert!(decode_at(Gfx1030, &code[..length], 0).is_err());
        }
        let decoded = decode(Gfx1030, &code).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].offset, 8);
    }
    // Noncanonical unused SRC1 remains rejected for unary e64 aliases.
    for opcode in 439..=443 {
        let code: Vec<_> = [0xd4000000u32 | opcode << 16 | 5, 0x02020101]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(decode_at(Gfx1030, &code, 0).is_err());
    }
}
