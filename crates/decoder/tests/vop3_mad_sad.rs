// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal opcode/operand expectations, independent of production metadata.
use xenora_decoder::{
    Architecture::Gfx1030, DecodeErrorKind, Instruction, VectorMadSadOp, VectorSource, decode,
    decode_at,
};
const CASES: [(u32, bool); 12] = [
    (322, false),
    (322, true),
    (323, false),
    (323, true),
    (346, false),
    (346, true),
    (347, false),
    (347, true),
    (348, false),
    (348, true),
    (349, false),
    (349, true),
];
const FLOAT: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
fn arity(_op: u32) -> usize {
    3
}
fn code(op: u32, clamp: bool, dst: u8, raw: [u32; 3], literal: u32) -> Vec<u8> {
    [
        0xd4000000 | op << 16 | u32::from(dst) | if clamp { 0x8000 } else { 0 },
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
fn expected(op: u32, clamp: bool, dst: u8, src: [VectorSource; 3]) -> Instruction {
    let [src0, src1, src2] = src;
    Instruction::VectorMadSad {
        op: match op {
            322 => VectorMadSadOp::MadI32I24,
            323 => VectorMadSadOp::MadU32U24,
            346 => VectorMadSadOp::SadU8,
            347 => VectorMadSadOp::SadHiU8,
            348 => VectorMadSadOp::SadU16,
            349 => VectorMadSadOp::SadU32,
            _ => unreachable!(),
        },
        clamp,
        dst,
        src0,
        src1,
        src2,
    }
}

fn baseline(op: u32) -> [u32; 3] {
    [257, 258, if arity(op) == 2 { 128 } else { 259 }]
}
#[test]
fn every_active_selector_and_destination_boundary() {
    for (op, clamp) in CASES {
        for position in 0..arity(op) {
            for raw in 0..512 {
                for dst in [0, 5, 255] {
                    let mut fields = baseline(op);
                    fields[position] = raw;
                    let bytes = code(op, clamp, dst, fields, 0x7fc12345);
                    let actual = decode_at(Gfx1030, &bytes, 0);
                    if let Some(value) = source(raw, 0x7fc12345) {
                        let mut sources = fields.map(|field| source(field, 0x7fc12345).unwrap());
                        sources[position] = value;
                        let actual = actual.unwrap();
                        assert_eq!(actual.instruction, expected(op, clamp, dst, sources));
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
    for (op, clamp) in CASES {
        let bytes = code(op, clamp, 5, baseline(op), 0);
        assert!(decode_at(Gfx1030, &bytes, 0).is_ok());
        for bit in (8..15).chain(59..64) {
            let mut modified = bytes.clone();
            modified[bit / 8] |= 1 << (bit % 8);
            assert!(matches!(
                decode_at(Gfx1030, &modified, 0).unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { .. }
            ));
        }
    }
    // Nearby full-width/ternary opcodes are not admitted by a family catch-all.
    for op in [335, 336, 337, 340, 343, 350, 837, 840, 878, 880] {
        assert!(decode_at(Gfx1030, &code(op, false, 5, [257, 258, 259], 0), 0).is_err());
    }
}
#[test]
fn distinct_scalar_identity_budget_precedes_immediate_collapse() {
    // Exhaust all triples over two normal SGPRs, VCC, literal, inline and VGPR.
    for (op, clamp) in CASES {
        for a in [0, 105, 106, 255, 129, 248, 511] {
            for b in [0, 105, 106, 255, 129, 248, 511] {
                for c in [0, 105, 106, 255, 129, 248, 511] {
                    let raw = [a, b, c];
                    let mut keys = std::collections::BTreeSet::new();
                    for value in raw {
                        if value <= 106 || value == 255 {
                            keys.insert(value);
                        }
                    }
                    let actual = decode_at(Gfx1030, &code(op, clamp, 255, raw, 1), 0);
                    if keys.len() <= 2 {
                        let actual = actual.unwrap();
                        assert_eq!(
                            actual.instruction,
                            expected(op, clamp, 255, raw.map(|v| source(v, 1).unwrap()))
                        );
                        assert_eq!(actual.byte_len, if raw.contains(&255) { 12 } else { 8 });
                    } else {
                        match actual.unwrap_err().kind {
                            DecodeErrorKind::UnsupportedForm { detail, .. } => {
                                assert!(detail.contains("scalar-bus"))
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn shared_literals_all_positions_truncation_and_stream_boundaries() {
    for (op, clamp) in CASES {
        for mask in 0..(1 << arity(op)) {
            for literal in [0, 1, 0xffffffff, 0x80000000, 0x7fc12345, 0xbf810000] {
                let mut raw = baseline(op);
                for (index, value) in raw.iter_mut().enumerate().take(arity(op)) {
                    if mask & (1 << index) != 0 {
                        *value = 255;
                    }
                }
                let len = if mask == 0 { 8 } else { 12 };
                let bytes = code(op, clamp, 255, raw, literal);
                for cut in 0..len {
                    assert!(matches!(
                        decode_at(Gfx1030, &bytes[..cut], 0).unwrap_err().kind,
                        DecodeErrorKind::Truncated { .. }
                    ));
                }
                let mut stream = code(op, clamp, 0, baseline(op), 0)[..8].to_vec();
                stream.extend_from_slice(&bytes[..len]);
                stream.extend_from_slice(&0xbf810000u32.to_le_bytes());
                let actual = decode(Gfx1030, &stream).unwrap();
                assert_eq!(actual.len(), 3);
                assert_eq!((actual[1].offset, actual[1].byte_len), (8, len as u8));
                assert_eq!(
                    actual[1].instruction,
                    expected(op, clamp, 255, raw.map(|v| source(v, literal).unwrap()))
                );
                assert_eq!(actual[2].offset, 8 + len);
                assert_eq!(actual[2].instruction, Instruction::EndProgram);
            }
        }
    }
}
#[test]
fn display_both_modes_and_unique_metadata() {
    for (opcode, name) in [
        (322, "v_mad_i32_i24"),
        (323, "v_mad_u32_u24"),
        (346, "v_sad_u8"),
        (347, "v_sad_hi_u8"),
        (348, "v_sad_u16"),
        (349, "v_sad_u32"),
    ] {
        for clamp in [false, true] {
            let instruction = decode_at(Gfx1030, &code(opcode, clamp, 255, [106, 511, 105], 0), 0)
                .unwrap()
                .instruction;
            assert_eq!(instruction.mnemonic(), name);
            assert_eq!(
                instruction.to_string(),
                format!(
                    "{name} v255, vcc_lo, v255, s105{}",
                    if clamp { " clamp" } else { "" }
                )
            );
        }
        assert_eq!(
            xenora_decoder::SEMANTIC_FORMS
                .iter()
                .filter(|&&(id, family)| id == name.to_uppercase()
                    && family == xenora_isa::meta::EncodingFamily::Vop3)
                .count(),
            1
        );
    }
    assert_eq!(xenora_decoder::SEMANTIC_FORMS.len(), 190);
}
