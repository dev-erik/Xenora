// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_decoder::{
    Architecture, DecodeError, DecodeErrorKind, DecodedInstruction, ExportTarget, Instruction,
    InterpolationPhase, decode,
};

#[test]
fn decodes_export_registers_and_interpolation_fields_in_byte_order() {
    // Derived with llvm-mc -triple=amdgcn -mcpu=gfx1030 -show-encoding:
    // exp param0, v255, v128, v15, v1
    // v_interp_p1_f32 v255, v254, attr31.z
    // v_interp_p2_f32 v128, v255, attr1.w
    let bytes = [
        0x0f, 0x02, 0x00, 0xf8, 0xff, 0x80, 0x0f, 0x01, 0xfe, 0x7e, 0xfc, 0xcb, 0xff, 0x07, 0x01,
        0xca,
    ];
    assert_eq!(
        decode(Architecture::Gfx1030, &bytes).unwrap(),
        [
            DecodedInstruction {
                offset: 0,
                byte_len: 8,
                instruction: Instruction::Export {
                    target: ExportTarget::Parameter0,
                    sources: [255, 128, 15, 1],
                    done: false,
                },
            },
            DecodedInstruction {
                offset: 8,
                byte_len: 4,
                instruction: Instruction::Interpolate {
                    phase: InterpolationPhase::First,
                    dst: 255,
                    coordinate: 254,
                    attribute: 31,
                    component: 2,
                },
            },
            DecodedInstruction {
                offset: 12,
                byte_len: 4,
                instruction: Instruction::Interpolate {
                    phase: InterpolationPhase::Second,
                    dst: 128,
                    coordinate: 255,
                    attribute: 1,
                    component: 3,
                },
            },
        ],
    );
}

#[test]
fn rejects_truncated_export_and_each_invalid_operand_field() {
    let valid = 0xf800_180fu32;
    let bytes = valid.to_le_bytes();
    assert_eq!(
        decode(Architecture::Gfx1030, &bytes).unwrap_err(),
        DecodeError {
            offset: 0,
            kind: DecodeErrorKind::Truncated {
                needed: 8,
                available: 4
            },
        },
    );
    for bit in 0..26 {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&(valid ^ (1 << bit)).to_le_bytes());
        if matches!(bit, 4 | 5 | 6 | 11) {
            let decoded = decode(Architecture::Gfx1030, &bytes).unwrap();
            let index = if bit == 11 { 0 } else { 1 << (bit - 4) };
            assert_eq!(
                decoded[0].instruction,
                Instruction::Export {
                    target: ExportTarget::RenderTarget(index),
                    sources: [0; 4],
                    done: bit != 11,
                }
            );
            continue;
        }
        assert!(
            matches!(
                decode(Architecture::Gfx1030, &bytes),
                Err(DecodeError {
                    offset: 0,
                    kind: DecodeErrorKind::UnsupportedForm {
                        mnemonic: "exp",
                        ..
                    },
                }),
            ),
            "export bit {bit}"
        );
    }
}

#[test]
fn runtime_uniform_fixtures_decode_register_conditions_and_diamond_boundaries() {
    use xenora_decoder::ScalarSource;
    for (bytes, register, compare_offset, branch_offset, arm, join) in [
        (
            include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-vs.bin").as_slice(),
            24,
            0,
            4,
            20,
            28,
        ),
        (
            include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-ps.bin").as_slice(),
            25,
            8,
            12,
            28,
            36,
        ),
    ] {
        let decoded = decode(Architecture::Gfx1030, bytes).unwrap();
        let compare = decoded.iter().find(|i| i.offset == compare_offset).unwrap();
        match compare.instruction {
            Instruction::ScalarCompare { op, src0, src1 } => {
                assert_eq!(op.mnemonic(), "s_cmp_eq_u32");
                assert_eq!(src0, ScalarSource::Register(register));
                assert_eq!(src1, ScalarSource::Immediate(0));
            }
            other => panic!("expected scalar compare, got {other:?}"),
        }
        assert_eq!(
            decoded
                .iter()
                .find(|i| i.offset == branch_offset)
                .unwrap()
                .instruction,
            Instruction::BranchSccZero { target: arm }
        );
        assert!(
            decoded
                .iter()
                .any(|i| i.instruction == Instruction::Branch { target: join })
        );
        assert!(decoded.iter().any(|i| i.offset == arm));
        assert!(decoded.iter().any(|i| i.offset == join));
    }
}

#[test]
fn scc1_uses_signed_dword_offsets_and_instruction_boundaries() {
    let bytes = [0xbf85_0000u32, 0xbf85_fffe, 0xbf81_0000]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let decoded = decode(Architecture::Gfx1030, &bytes).unwrap();
    assert_eq!(
        decoded[0].instruction,
        Instruction::BranchSccOne { target: 4 }
    );
    assert_eq!(
        decoded[1].instruction,
        Instruction::BranchSccOne { target: 0 }
    );
    assert_eq!(decoded[1].branch_target(), Some(0));
    assert_eq!(decoded[0].byte_len, 4);
    assert_eq!(decoded[0].instruction.mnemonic(), "s_cbranch_scc1");
    for words in [
        vec![0xbf85_fffeu32], // Target before stream.
        vec![0xbf85_0001, 0x7e04_02ff, 0x1234_5678, 0xbf81_0000], // Literal.
        vec![0xbf85_0001, 0xbf81_0000], // Past stream.
    ] {
        let bytes = words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(decode(Architecture::Gfx1030, &bytes).is_err());
    }
}
