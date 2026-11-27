// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal opcode/operand expectations, independent of production metadata.
use xenora_decoder::{
    Architecture::Gfx1030, DecodeErrorKind, Instruction, VectorAluOp, VectorBinaryOp, VectorSource,
    VectorUnaryOp, decode, decode_at,
};
const OPS: [u32; 23] = [
    265, 266, 267, 268, 273, 274, 275, 276, 286, 293, 294, 295, 283, 284, 285, 282, 278, 280, 439,
    440, 441, 442, 443,
];
const FLOAT: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
fn arity(op: u32) -> usize {
    if (439..=443).contains(&op) { 1 } else { 2 }
}
fn code(op: u32, dst: u8, raw: [u32; 3], literal: u32) -> Vec<u8> {
    [
        0xd4000000 | op << 16 | u32::from(dst),
        raw[0] | raw[1] << 9 | raw[2] << 18,
        literal,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
fn source(raw: u32, literal: u32) -> Option<VectorSource> {
    Some(match raw {
        0..=106 => VectorSource::Scalar(raw as u8),
        128..=192 => VectorSource::Immediate(raw - 128),
        193..=208 => VectorSource::Immediate((192i32 - raw as i32) as u32),
        240..=248 => VectorSource::Immediate(FLOAT[(raw - 240) as usize]),
        255 => VectorSource::Immediate(literal),
        256..=511 => VectorSource::Register((raw - 256) as u8),
        _ => return None,
    })
}
fn expected(op: u32, dst: u8, src: [VectorSource; 3]) -> Instruction {
    let [src0, src1, _] = src;
    if op == 293
        && let (VectorSource::Register(src0), VectorSource::Register(src1)) = (src0, src1)
    {
        return Instruction::AddNoCarryU32 { dst, src0, src1 };
    }
    match op {
        439 => Instruction::VectorUnaryE64 {
            op: VectorUnaryOp::NotB32,
            dst,
            src: src0,
        },
        440 => Instruction::VectorUnaryE64 {
            op: VectorUnaryOp::BfrevB32,
            dst,
            src: src0,
        },
        441 => Instruction::VectorUnaryE64 {
            op: VectorUnaryOp::FfbhU32,
            dst,
            src: src0,
        },
        442 => Instruction::VectorUnaryE64 {
            op: VectorUnaryOp::FfblB32,
            dst,
            src: src0,
        },
        443 => Instruction::VectorUnaryE64 {
            op: VectorUnaryOp::FfbhI32,
            dst,
            src: src0,
        },
        _ => Instruction::VectorBinaryE64 {
            op: match op {
                265 => VectorBinaryOp::Alu(VectorAluOp::MulI32I24),
                266 => VectorBinaryOp::Alu(VectorAluOp::MulHiI32I24),
                267 => VectorBinaryOp::Alu(VectorAluOp::MulU32U24),
                268 => VectorBinaryOp::Alu(VectorAluOp::MulHiU32U24),
                273 => VectorBinaryOp::Alu(VectorAluOp::MinI32),
                274 => VectorBinaryOp::Alu(VectorAluOp::MaxI32),
                275 => VectorBinaryOp::Alu(VectorAluOp::MinU32),
                276 => VectorBinaryOp::Alu(VectorAluOp::MaxU32),
                286 => VectorBinaryOp::Alu(VectorAluOp::XnorB32),
                293 => VectorBinaryOp::Alu(VectorAluOp::AddNcU32),
                294 => VectorBinaryOp::Alu(VectorAluOp::SubNcU32),
                295 => VectorBinaryOp::Alu(VectorAluOp::SubrevNcU32),
                283 => VectorBinaryOp::AndB32,
                284 => VectorBinaryOp::OrB32,
                285 => VectorBinaryOp::XorB32,
                282 => VectorBinaryOp::LshlrevB32,
                278 => VectorBinaryOp::LshrrevB32,
                280 => VectorBinaryOp::AshrrevI32,
                _ => unreachable!(),
            },
            dst,
            src0,
            src1,
        },
    }
}

fn baseline(op: u32) -> [u32; 3] {
    [257, if arity(op) == 1 { 128 } else { 258 }, 128]
}
#[test]
fn every_active_selector_and_destination_boundary() {
    for op in OPS {
        for position in 0..arity(op) {
            for raw in 0..512 {
                for dst in [0, 5, 255] {
                    let mut fields = baseline(op);
                    fields[position] = raw;
                    let bytes = code(op, dst, fields, 0x7fc12345);
                    let actual = decode_at(Gfx1030, &bytes, 0);
                    if let Some(value) = source(raw, 0x7fc12345) {
                        let mut sources = fields.map(|field| source(field, 0x7fc12345).unwrap());
                        sources[position] = value;
                        let actual = actual.unwrap();
                        assert_eq!(actual.instruction, expected(op, dst, sources));
                        assert_eq!(actual.byte_len, if raw == 255 { 12 } else { 8 });
                        assert_eq!(
                            decode_at(Gfx1030, &bytes[..actual.byte_len as usize], 0).unwrap(),
                            actual
                        );
                    } else {
                        assert!(
                            matches!(
                                actual.unwrap_err().kind,
                                DecodeErrorKind::UnsupportedForm { .. }
                            ),
                            "op={op} position={position} raw={raw}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn exact_opcode_scope_and_each_modifier_bit() {
    for op in OPS {
        let bytes = code(op, 5, baseline(op), 0);
        assert!(decode_at(Gfx1030, &bytes, 0).is_ok());
        for bit in (8..16).chain(59..64) {
            let mut modified = bytes.clone();
            modified[bit / 8] |= 1 << (bit % 8);
            assert!(matches!(
                decode_at(Gfx1030, &modified, 0).unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { mnemonic, .. }
                    if mnemonic == expected(op, 5, baseline(op).map(|v| source(v, 0).unwrap())).mnemonic()
            ));
        }
    }
    // Nearby full-width/ternary opcodes are not admitted by a family catch-all.
    for op in [335, 336, 337, 340, 343, 346, 837, 840, 878, 880] {
        assert!(decode_at(Gfx1030, &code(op, 5, [257, 258, 259], 0), 0).is_err());
    }
}
#[test]
fn all_unused_fields_require_inline_zero() {
    for op in OPS {
        for position in arity(op)..3 {
            for selector in 0..512 {
                let mut raw = baseline(op);
                raw[position] = selector;
                assert_eq!(
                    decode_at(Gfx1030, &code(op, 0, raw, 0), 0).is_ok(),
                    selector == 128
                );
            }
        }
    }
}
#[test]
fn two_scalar_keys_and_shared_literal_are_accepted() {
    for op in OPS {
        for a in [0, 105, 106, 255, 129, 248, 511] {
            for b in [0, 105, 106, 255, 129, 248, 511] {
                let raw = [a, if arity(op) == 1 { 128 } else { b }, 128];
                let actual = decode_at(Gfx1030, &code(op, 255, raw, 1), 0).unwrap();
                assert_eq!(
                    actual.instruction,
                    expected(op, 255, raw.map(|v| source(v, 1).unwrap()))
                );
                assert_eq!(
                    actual.byte_len,
                    if raw[..arity(op)].contains(&255) {
                        12
                    } else {
                        8
                    }
                );
            }
        }
    }
}
#[test]
fn shared_literals_all_positions_truncation_and_stream_boundaries() {
    for op in OPS {
        for mask in 0..(1 << arity(op)) {
            for literal in [0, 1, 0xffffffff, 0x80000000, 0x7fc12345, 0xbf810000] {
                let mut raw = baseline(op);
                for (index, value) in raw.iter_mut().enumerate().take(arity(op)) {
                    if mask & (1 << index) != 0 {
                        *value = 255;
                    }
                }
                let len = if mask == 0 { 8 } else { 12 };
                let bytes = code(op, 255, raw, literal);
                for cut in 0..len {
                    assert!(matches!(
                        decode_at(Gfx1030, &bytes[..cut], 0).unwrap_err().kind,
                        DecodeErrorKind::Truncated { .. }
                    ));
                }
                let mut stream = code(op, 0, baseline(op), 0)[..8].to_vec();
                stream.extend_from_slice(&bytes[..len]);
                stream.extend_from_slice(&0xbf810000u32.to_le_bytes());
                let actual = decode(Gfx1030, &stream).unwrap();
                assert_eq!(actual.len(), 3);
                assert_eq!((actual[1].offset, actual[1].byte_len), (8, len as u8));
                assert_eq!(
                    actual[1].instruction,
                    expected(op, 255, raw.map(|v| source(v, literal).unwrap()))
                );
                assert_eq!(actual[2].offset, 8 + len);
                assert_eq!(actual[2].instruction, Instruction::EndProgram);
            }
        }
    }
}
#[test]
fn exact_names_and_metadata_unique() {
    for (opcode, name, identity) in [
        (265, "v_mul_i32_i24_e64", "V_MUL_I32_I24"),
        (266, "v_mul_hi_i32_i24_e64", "V_MUL_HI_I32_I24"),
        (267, "v_mul_u32_u24_e64", "V_MUL_U32_U24"),
        (268, "v_mul_hi_u32_u24_e64", "V_MUL_HI_U32_U24"),
        (273, "v_min_i32_e64", "V_MIN_I32"),
        (274, "v_max_i32_e64", "V_MAX_I32"),
        (275, "v_min_u32_e64", "V_MIN_U32"),
        (276, "v_max_u32_e64", "V_MAX_U32"),
        (286, "v_xnor_b32_e64", "V_XNOR_B32"),
        (293, "v_add_nc_u32_e64", "V_ADD_NC_U32"),
        (294, "v_sub_nc_u32_e64", "V_SUB_NC_U32"),
        (295, "v_subrev_nc_u32_e64", "V_SUBREV_NC_U32"),
        (283, "v_and_b32_e64", "V_AND_B32"),
        (284, "v_or_b32_e64", "V_OR_B32"),
        (285, "v_xor_b32_e64", "V_XOR_B32"),
        (282, "v_lshlrev_b32_e64", "V_LSHLREV_B32"),
        (278, "v_lshrrev_b32_e64", "V_LSHRREV_B32"),
        (280, "v_ashrrev_i32_e64", "V_ASHRREV_I32"),
        (439, "v_not_b32_e64", "V_NOT_B32"),
        (440, "v_bfrev_b32_e64", "V_BFREV_B32"),
        (441, "v_ffbh_u32_e64", "V_FFBH_U32"),
        (442, "v_ffbl_b32_e64", "V_FFBL_B32"),
        (443, "v_ffbh_i32_e64", "V_FFBH_I32"),
    ] {
        let instruction = decode_at(Gfx1030, &code(opcode, 5, baseline(opcode), 0), 0)
            .unwrap()
            .instruction;
        assert_eq!(instruction.mnemonic(), name);
        assert_eq!(
            instruction.to_string(),
            format!(
                "{name} v5, v1{}",
                if arity(opcode) == 1 { "" } else { ", v2" }
            )
        );
        assert_eq!(
            xenora_decoder::SEMANTIC_FORMS
                .iter()
                .filter(|&&(name, family)| name == identity
                    && family == xenora_isa::meta::EncodingFamily::Vop3)
                .count(),
            1
        );
    }
}
