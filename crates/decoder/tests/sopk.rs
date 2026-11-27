//! SOPK encodings from AMD RDNA2 sections 12.2 and 13.1.2.
use xenora_decoder::{
    Architecture, DecodeErrorKind, Instruction, SEMANTIC_FORMS, ScalarAluOp, ScalarSource, decode,
    decode_at,
};
use xenora_isa::meta::{EncodingFamily, FORMS, identify};

#[test]
fn sopk_identity_operands_and_four_byte_length() {
    for (base, name, op) in [
        (0xb000_0000u32, "S_MOVK_I32", None),
        (0xb780_0000, "S_ADDK_I32", Some(ScalarAluOp::AddI32)),
        (0xb800_0000, "S_MULK_I32", Some(ScalarAluOp::MulI32)),
    ] {
        let form = FORMS.iter().find(|form| form.name == name).unwrap();
        assert_eq!(form.mask, 0xff80_0000);
        assert_eq!(form.bits, u64::from(base));
        assert!(SEMANTIC_FORMS.contains(&(name, EncodingFamily::Sopk)));
        for dst in 0..=105u8 {
            for (imm, expected) in [
                (0u32, 0),
                (1, 1),
                (0x7fff, 0x7fff),
                (0x8000, 0xffff_8000),
                (0xffff, 0xffff_ffff),
                (0xff, 0xff),
            ] {
                let bytes = (base | (u32::from(dst) << 16) | imm).to_le_bytes();
                assert_eq!(identify(&bytes).unwrap().form.name, name);
                let decoded = decode(Architecture::Gfx1030, &bytes).unwrap();
                assert_eq!(decoded.len(), 1);
                assert_eq!(decoded[0].offset, 0);
                assert_eq!(decoded[0].byte_len, 4);
                let src = ScalarSource::Immediate(expected);
                assert_eq!(
                    decoded[0].instruction,
                    match op {
                        None => Instruction::ScalarMoveB32 { dst, src },
                        Some(op) => Instruction::ScalarAlu {
                            op,
                            dst,
                            src0: ScalarSource::Register(dst),
                            src1: src,
                        },
                    }
                );
            }
        }
        for dst in 106..=127u32 {
            let bytes = (base | (dst << 16)).to_le_bytes();
            assert!(matches!(
                decode(Architecture::Gfx1030, &bytes).unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { .. }
            ));
        }
    }
}

#[test]
fn neighboring_opcodes_stay_unsupported() {
    for op in 1..=26u32 {
        if matches!(op, 15 | 16 | 23) {
            continue;
        }
        let word = 0xb000_0000 | (op << 23);
        assert_eq!(
            decode(Architecture::Gfx1030, &word.to_le_bytes())
                .unwrap_err()
                .kind,
            DecodeErrorKind::UnsupportedOpcode { word }
        );
    }
}

#[test]
fn sopk_does_not_consume_a_literal_or_capture_waits() {
    let words = [
        0xb000_ffffu32,
        0xb780_8000,
        0xb800_00ff,
        0xbbfd_0000,
        0xbf8c_3f70,
        0xbf81_0000,
    ];
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    let decoded = decode(Architecture::Gfx1030, &bytes).unwrap();
    assert_eq!(decoded.len(), words.len());
    for (index, instruction) in decoded.iter().enumerate() {
        assert_eq!(instruction.offset, index * 4);
        assert_eq!(instruction.byte_len, 4);
    }
    assert_eq!(decoded[3].instruction, Instruction::WaitStore);
    assert_eq!(decoded[4].instruction, Instruction::WaitLoad);
    assert_eq!(decoded[5].instruction, Instruction::EndProgram);
    for word in [0xbb80_0000u32, 0xbbfd_0001, 0xbf8c_0000] {
        assert!(matches!(
            decode(Architecture::Gfx1030, &word.to_le_bytes())
                .unwrap_err()
                .kind,
            DecodeErrorKind::UnsupportedForm { .. }
        ));
    }
    for available in 0..4 {
        assert_eq!(
            decode_at(Architecture::Gfx1030, &bytes[..available], 0)
                .unwrap_err()
                .kind,
            DecodeErrorKind::Truncated {
                needed: 4,
                available
            }
        );
    }
}
