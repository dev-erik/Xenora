// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, Instruction, decode_at};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
#[test]
fn readfirst_vop1_exact_source_and_destination_domains() {
    for dst in 0..256 {
        for src in 0..512 {
            let code = bytes(&[0x7e000400 | dst << 17 | src]);
            let result = decode_at(Gfx1030, &code, 0);
            if dst <= 105 && src >= 256 {
                let decoded = result.unwrap();
                assert_eq!(
                    decoded.instruction,
                    Instruction::ReadFirstLaneB32 {
                        dst: dst as u8,
                        src: src as u8
                    }
                );
                assert_eq!(decoded.byte_len, 4);
                assert_eq!(
                    decoded.instruction.to_string(),
                    format!("v_readfirstlane_b32 s{dst}, v{}", src - 256)
                );
            } else {
                assert!(result.is_err());
            }
        }
    }
}
#[test]
fn readlane_requires_normal_sgpr_index_and_canonical_padding() {
    for src in 0..512 {
        for lane in [0, 8, 105, 106, 124, 126, 128, 255, 256, 511] {
            let result = decode_at(
                Gfx1030,
                &bytes(&[0xd7600008, src | lane << 9 | 128 << 18]),
                0,
            );
            assert_eq!(result.is_ok(), src >= 256 && lane <= 105);
        }
    }
    for dst in 0..256 {
        assert_eq!(
            decode_at(Gfx1030, &bytes(&[0xd7600000 | dst, 0x02001301]), 0).is_ok(),
            dst <= 105
        );
    }
    for bit in (8..16).chain(59..64) {
        let mut code = bytes(&[0xd7600008, 0x02001301]);
        code[bit / 8] ^= 1 << (bit % 8);
        assert!(decode_at(Gfx1030, &code, 0).is_err());
    }
    for padding in 0..512 {
        assert_eq!(
            decode_at(
                Gfx1030,
                &bytes(&[0xd7600008, 257 | 9 << 9 | padding << 18]),
                0
            )
            .is_ok(),
            padding == 128
        );
    }
}
#[test]
fn literal_truncation_and_unverified_e64_readfirst_remain_rejected() {
    for words in [vec![0x7e100501], vec![0xd7600008, 0x02001301]] {
        let code = bytes(&words);
        for cut in 0..code.len() {
            assert!(decode_at(Gfx1030, &code[..cut], 0).is_err());
        }
    }
    // AMD XML has this VOP3 identity; LLVM rejects its encoding. Not admitted yet.
    assert!(decode_at(Gfx1030, &bytes(&[0xd5820008, 0x02010101]), 0).is_err());
    for src in [233, 234, 249, 250, 254, 255] {
        assert!(decode_at(Gfx1030, &bytes(&[0x7e100400 | src, 0]), 0).is_err());
    }
}
