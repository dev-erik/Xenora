// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal opcode/operand expectations, independent of production metadata.
use xenora_decoder::{
    Architecture::Gfx1030, DecodeErrorKind, Instruction, VectorClampOp, VectorSource, decode,
    decode_at,
};
const OPS: [u32; 5] = [265, 267, 293, 294, 295];
const FLOAT: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
fn arity(_op: u32) -> usize {
    2
}
fn code(op: u32, dst: u8, raw: [u32; 3], literal: u32) -> Vec<u8> {
    [
        0xd4008000 | op << 16 | u32::from(dst),
        raw[0] | raw[1] << 9 | raw[2] << 18,
        literal,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
fn source(raw: u32, literal: u32) -> Option<VectorSource> {
    Some(match raw {
        0..=107 | 126..=127 => VectorSource::Scalar(raw as u8),
        128..=192 => VectorSource::Immediate(raw - 128),
        193..=208 => VectorSource::Immediate((192i32 - raw as i32) as u32),
        240..=248 => VectorSource::Immediate(FLOAT[(raw - 240) as usize]),
        255 => VectorSource::Immediate(literal),
        256..=511 => VectorSource::Register((raw - 256) as u8),
        _ => return None,
    })
}
fn expected(op: u32, dst: u8, src: [VectorSource; 3]) -> Instruction {
    Instruction::VectorClampedE64 {
        op: match op {
            265 => VectorClampOp::MulI24,
            267 => VectorClampOp::MulU24,
            293 => VectorClampOp::AddU32,
            294 => VectorClampOp::SubU32,
            295 => VectorClampOp::SubrevU32,
            _ => unreachable!(),
        },
        dst,
        src0: src[0],
        src1: src[1],
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
        for bit in (8..15).chain(59..64) {
            let mut modified = bytes.clone();
            modified[bit / 8] |= 1 << (bit % 8);
            assert!(matches!(
                decode_at(Gfx1030, &modified, 0).unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { mnemonic, .. }
                    if mnemonic == expected(op, 5, baseline(op).map(|v| source(v, 0).unwrap())).mnemonic()
            ));
        }
    }
    assert!(matches!(
        decode_at(Gfx1030, &code(346, 5, [257, 258, 259], 0), 0)
            .unwrap()
            .instruction,
        Instruction::VectorMadSad {
            op: xenora_decoder::VectorMadSadOp::SadU8,
            ..
        }
    ));
    // Nearby full-width/ternary opcodes are not admitted by a family catch-all.
    for op in [335, 336, 337, 340, 343, 350, 834, 840, 878, 880] {
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
        for a in [0, 105, 106, 107, 126, 127, 255, 129, 248, 511] {
            for b in [0, 105, 106, 107, 126, 127, 255, 129, 248, 511] {
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
fn only_five_clamp_modes_and_old_add_representation() {
    for op in [
        266, 268, 273, 274, 275, 276, 278, 280, 282, 283, 284, 285, 286, 334, 439, 440, 441, 442,
        443,
    ] {
        assert!(decode_at(Gfx1030, &code(op, 5, baseline(op), 0), 0).is_err());
    }
    let mut add = code(293, 5, baseline(293), 0);
    assert!(matches!(
        decode_at(Gfx1030, &add, 0).unwrap().instruction,
        Instruction::VectorClampedE64 {
            op: VectorClampOp::AddU32,
            ..
        }
    ));
    add[1] &= 0x7f;
    assert_eq!(
        decode_at(Gfx1030, &add, 0).unwrap().instruction,
        Instruction::AddNoCarryU32 {
            dst: 5,
            src0: 1,
            src1: 2
        }
    );
}
#[test]
fn names_display_and_existing_identity_count() {
    for (op, name) in [
        (265, "v_mul_i32_i24_e64"),
        (267, "v_mul_u32_u24_e64"),
        (293, "v_add_nc_u32_e64"),
        (294, "v_sub_nc_u32_e64"),
        (295, "v_subrev_nc_u32_e64"),
    ] {
        let instruction = decode_at(Gfx1030, &code(op, 255, [106, 255, 128], 0xffffffff), 0)
            .unwrap()
            .instruction;
        assert_eq!(instruction.mnemonic(), name);
        assert_eq!(
            instruction.to_string(),
            format!("{name} v255, vcc_lo, 0xffffffff clamp")
        );
    }
    // Includes the bounded control, wait and Wave32 MBCNT forms.
    assert_eq!(xenora_decoder::SEMANTIC_FORMS.len(), 208);
}
