// SPDX-License-Identifier: GPL-3.0-or-later
//! Original gfx1030 words checked against public AMD metadata and native LLVM.
use xenora_decoder::{Architecture::Gfx1030, DecodeErrorKind, Instruction, SEMANTIC_FORMS, decode};
use xenora_isa::meta::EncodingFamily;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn one(words: &[u32]) -> Instruction {
    let decoded = decode(Gfx1030, &bytes(words)).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(usize::from(decoded[0].byte_len), words.len() * 4);
    decoded[0].instruction
}
fn rejects(words: &[u32], name: &'static str) {
    assert!(matches!(decode(Gfx1030, &bytes(words)).unwrap_err().kind,
        DecodeErrorKind::UnsupportedForm { mnemonic, .. } if mnemonic == name));
}

#[test]
fn ds_registers_offsets_and_exact_identities() {
    for register in 0..=255u32 {
        for offset in [0, 1, 255, 256, 0x7fff, 0x801b, 0xc020, 0xe000, 0xffff] {
            let data = 255 - register;
            assert_eq!(
                one(&[0xdacc0000 | offset, register << 24 | data << 8 | register]),
                Instruction::DsBpermuteB32 {
                    dst: register as u8,
                    addr: register as u8,
                    data: data as u8,
                    offset: offset as u16
                }
            );
            assert_eq!(
                one(&[0xd8d40000 | offset, register << 24 | data]),
                Instruction::DsSwizzleB32 {
                    dst: register as u8,
                    src: data as u8,
                    offset: offset as u16
                }
            );
        }
    }
    assert_eq!(
        one(&[0xdacc0000, 0x03000504]).to_string(),
        "ds_bpermute_b32 v3, v4, v5"
    );
    assert_eq!(
        one(&[0xd8d4801b, 0x03000004]).to_string(),
        "ds_swizzle_b32 v3, v4 offset:32795"
    );
    for name in ["DS_BPERMUTE_B32", "DS_SWIZZLE_B32"] {
        assert!(SEMANTIC_FORMS.contains(&(name, EncodingFamily::Ds)));
    }
}

#[test]
fn ds_rejects_gds_reserved_and_unused_fields() {
    for (first, second, name, unused) in [
        (0xdacc0000u32, 0x03000504u32, "ds_bpermute_b32", 16..24),
        (0xd8d40000, 0x03000004, "ds_swizzle_b32", 8..24),
    ] {
        for bit in [16, 17] {
            rejects(&[first | 1 << bit, second], name);
        }
        for bit in unused {
            rejects(&[first, second | 1 << bit], name);
        }
    }
}

#[test]
fn permlane_normal_scalar_selectors_and_all_fi_bc_modes() {
    for (first, cross_row, name) in [
        (0xd7770000u32, false, "V_PERMLANE16_B32"),
        (0xd7780000, true, "V_PERMLANEX16_B32"),
    ] {
        assert!(SEMANTIC_FORMS.contains(&(name, EncodingFamily::Vop3)));
        for lo in 0..=105u32 {
            for hi in 0..=105u32 {
                for flags in 0..4u32 {
                    assert_eq!(
                        one(&[first | 255 | flags << 11, 511 | lo << 9 | hi << 18]),
                        Instruction::Permlane16B32 {
                            dst: 255,
                            src: 255,
                            select_lo: lo as u8,
                            select_hi: hi as u8,
                            cross_row,
                            fetch_inactive: flags & 1 != 0,
                            bound_control: flags & 2 != 0
                        }
                    );
                }
            }
        }
        for reg in 0..=255u32 {
            assert!(
                matches!(one(&[first | reg, 256 | reg]), Instruction::Permlane16B32 { dst, src, .. }
                if dst == reg as u8 && src == reg as u8)
            );
        }
    }
    assert_eq!(
        one(&[0xd7771803, 0x00180b04]).to_string(),
        "v_permlane16_b32 v3, v4, s5, s6 op_sel:[1,1]"
    );
}

#[test]
fn permlane_rejects_non_vgpr_data_non_normal_selectors_and_modifiers() {
    for (first, name) in [
        (0xd7770000u32, "v_permlane16_b32"),
        (0xd7780000, "v_permlanex16_b32"),
    ] {
        for src in 0..256 {
            rejects(&[first, src], name);
        }
        for selector in 106..512u32 {
            for shift in [9, 18] {
                rejects(&[first, 256 | selector << shift], name);
            }
        }
        for bit in [8, 9, 10, 13, 14, 15] {
            rejects(&[first | 1 << bit, 256], name);
        }
        for bit in 27..32 {
            rejects(&[first, 256 | 1 << bit], name);
        }
        // A literal word cannot make the unsupported scalar selector admissible.
        rejects(&[first, 256 | 255 << 9, 0x12345678], name);
    }
}

#[test]
fn transfers_require_eight_bytes_and_preserve_following_instruction() {
    for words in [
        [0xdacc0000, 0x03000504],
        [0xd8d4801b, 0x03000004],
        [0xd7770003, 0x00180b04],
        [0xd7781803, 0x00180b04],
    ] {
        let code = bytes(&words);
        for length in 1..8 {
            assert!(decode(Gfx1030, &code[..length]).is_err());
        }
        let stream = bytes(&[words[0], words[1], 0xbf8cc07f, 0xbf810000]);
        let decoded = decode(Gfx1030, &stream).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[1].offset, 8);
        assert_eq!(decoded[1].instruction, Instruction::WaitLgkm);
        assert_eq!(decoded[2].offset, 12);
    }
}

#[test]
fn lgkm_zero_is_a_distinct_wait_operand_mode_only() {
    assert_eq!(one(&[0xbf8cc07f]), Instruction::WaitLgkm);
    assert_eq!(one(&[0xbf8cc07f]).to_string(), "s_waitcnt lgkmcnt(0)");
    assert_eq!(one(&[0xbf8c3f70]), Instruction::WaitLoad);
    for immediate in 0..=65535u32 {
        if matches!(immediate, 0xc07f | 0x3f70) {
            continue;
        }
        rejects(&[0xbf8c0000 | immediate], "s_waitcnt");
    }
    assert_eq!(
        SEMANTIC_FORMS
            .iter()
            .filter(|(name, _)| *name == "S_WAITCNT")
            .count(),
        1
    );
}

// Spec-derived only: AMD XML VOP3 opcode386 and Table82 pp257-258.
// LLVM omits this encoding; these are not LLVM-validated fixture words.
#[test]
fn readfirstlane_vop3_spec_derived_register_domain_and_identity() {
    assert!(SEMANTIC_FORMS.contains(&("V_READFIRSTLANE_B32", EncodingFamily::Vop3)));
    for dst in 0..=105u32 {
        for src in 0..=255u32 {
            assert_eq!(
                one(&[0xd5820000 | dst, 256 | src]),
                Instruction::ReadFirstLaneB32 {
                    dst: dst as u8,
                    src: src as u8
                }
            );
        }
    }
    assert_eq!(
        one(&[0xd5820069, 511]).to_string(),
        "v_readfirstlane_b32 s105, v255"
    );
}

#[test]
fn readfirstlane_vop3_rejects_reserved_registers_and_non_vgpr_sources() {
    for dst in 106..=255u32 {
        rejects(&[0xd5820000 | dst, 256], "v_readfirstlane_b32");
    }
    for src in 0..256u32 {
        rejects(&[0xd5820000, src], "v_readfirstlane_b32");
    }
    rejects(&[0xd5820000, 255, 0x12345678], "v_readfirstlane_b32");
}

#[test]
fn readfirstlane_vop3_project_zero_padding_and_modifier_policy() {
    // All nonzero values of each unused 9-bit selector are outside this slice.
    // Inline-zero selector128 is deliberately NOT our selected padding here.
    for padding in 1..512u32 {
        for shift in [9, 18] {
            rejects(&[0xd5820000, 256 | padding << shift], "v_readfirstlane_b32");
        }
    }
    for modifiers in 1..256u32 {
        rejects(&[0xd5820000 | modifiers << 8, 256], "v_readfirstlane_b32");
    }
    for modifiers in 1..32u32 {
        rejects(&[0xd5820000, 256 | modifiers << 27], "v_readfirstlane_b32");
    }
}

#[test]
fn readfirstlane_vop3_requires_exact_eight_bytes_before_next_instruction() {
    let code = bytes(&[0xd5820008, 257]);
    for length in 1..8 {
        assert!(decode(Gfx1030, &code[..length]).is_err());
    }
    let stream = bytes(&[0xd5820008, 257, 0xbf810000]);
    let decoded = decode(Gfx1030, &stream).unwrap();
    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].byte_len, 8);
    assert_eq!(decoded[1].offset, 8);
    assert_eq!(decoded[1].instruction, Instruction::EndProgram);
}
