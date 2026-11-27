// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{
    Architecture::Gfx1030, BufferLoadKind, DecodeErrorKind, Instruction, decode, decode_at,
};
const FORMS: [(u32, BufferLoadKind, &str); 4] = [
    (0xe0202000, BufferLoadKind::Ubyte, "buffer_load_ubyte"),
    (0xe0242000, BufferLoadKind::Sbyte, "buffer_load_sbyte"),
    (0xe0282000, BufferLoadKind::Ushort, "buffer_load_ushort"),
    (0xe02c2000, BufferLoadKind::Sshort, "buffer_load_sshort"),
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
#[test]
fn exact_types_lengths_and_display_for_register_boundaries() {
    for (first, kind, name) in FORMS {
        for data in [0, 1, 127, 255] {
            for index in [0, 1, 127, 255] {
                for resource in 0..32 {
                    let code = bytes(&[first, 0x80000000 | resource << 16 | data << 8 | index]);
                    let result = decode_at(Gfx1030, &code, 0);
                    if resource > 25 {
                        assert!(result.is_err());
                        continue;
                    }
                    let result = result.unwrap();
                    assert_eq!(
                        result.instruction,
                        Instruction::BufferLoadNarrow {
                            kind,
                            data: data as u8,
                            index: index as u8,
                            resource: (resource * 4) as u8
                        }
                    );
                    assert_eq!(result.byte_len, 8);
                    assert_eq!(
                        result.instruction.to_string(),
                        format!(
                            "{name} v{data}, v{index}, s[{}:{}], 0 idxen",
                            resource * 4,
                            resource * 4 + 3
                        )
                    );
                }
            }
        }
    }
    assert!(matches!(
        decode_at(Gfx1030, &bytes(&[0xe0302000, 0x80000100]), 0)
            .unwrap()
            .instruction,
        Instruction::BufferLoadDword {
            data: 1,
            index: 0,
            resource: 0
        }
    ));
}
#[test]
fn each_address_modifier_reserved_bit_and_soffset_is_checked() {
    for (first, _, _) in FORMS {
        // Flip every non-opcode bit of the first word: offsets, enables, cache policy, LDS, reserved.
        for bit in 0..18 {
            let result = decode_at(Gfx1030, &bytes(&[first ^ (1 << bit), 0x80000100]), 0);
            assert!(matches!(
                result.unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { .. }
            ));
        }
        for bit in 21..24 {
            assert!(decode_at(Gfx1030, &bytes(&[first, 0x80000100 | 1 << bit]), 0).is_err());
        }
        for soffset in 0..256 {
            let result = decode_at(Gfx1030, &bytes(&[first, soffset << 24 | 0x100]), 0);
            assert_eq!(result.is_ok(), soffset == 128);
        }
    }
}
#[test]
fn truncated_second_words_and_branch_into_load_payload_reject() {
    for (first, _, _) in FORMS {
        let code = bytes(&[first, 0x80000100]);
        for len in 0..8 {
            assert!(matches!(
                decode_at(Gfx1030, &code[..len], 0).unwrap_err().kind,
                DecodeErrorKind::Truncated { .. }
            ));
        }
        let decoded = decode(Gfx1030, &bytes(&[first, 0x80000100, 0xbf810000])).unwrap();
        assert_eq!(decoded[1].offset, 8);
        assert!(
            decode(
                Gfx1030,
                &bytes(&[0xbf820001, first, 0x80000100, 0xbf810000])
            )
            .is_err()
        );
    }
}
#[test]
fn no_neighboring_load_or_store_family_is_admitted() {
    for opcode in [0, 7, 13, 14, 15, 24, 25, 26, 32, 33, 34, 35, 36, 37] {
        assert!(decode_at(Gfx1030, &bytes(&[0xe0002000 | opcode << 18, 0x80000100]), 0).is_err());
    }
    for (_, _, name) in FORMS {
        assert_eq!(
            xenora_decoder::SEMANTIC_FORMS
                .iter()
                .filter(|(id, _)| *id == name.to_uppercase())
                .count(),
            1
        );
    }
}
