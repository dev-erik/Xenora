// SPDX-License-Identifier: GPL-3.0-or-later
//! Mask-half encodings are wave-size independent. Configured consumers enforce
//! AMD RDNA2 section 6.2.4 Wave64 read/write overlap and runtime-source limits.
use xenora_decoder::{
    Architecture::Gfx1030, DecodeErrorKind, Instruction, ScalarSource, VectorSource, decode,
    decode_at,
};
const HALVES: [(u32, &str); 4] = [
    (106, "vcc_lo"),
    (107, "vcc_hi"),
    (126, "exec_lo"),
    (127, "exec_hi"),
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn one(words: &[u32]) -> Instruction {
    let decoded = decode(Gfx1030, &bytes(words)).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(usize::from(decoded[0].byte_len), words.len() * 4);
    decoded[0].instruction
}

#[test]
fn scalar_mask_half_reads_writes_and_spelling_are_exact() {
    for (selector, name) in HALVES {
        assert_eq!(
            one(&[0xbe880300 | selector]),
            Instruction::ScalarMoveB32 {
                dst: 8,
                src: ScalarSource::Register(selector as u8)
            }
        );
        assert_eq!(
            one(&[0xbe880300 | selector]).to_string(),
            format!("s_mov_b32 s8, {name}")
        );
        assert_eq!(
            one(&[0xbe800381 | selector << 16]).to_string(),
            format!("s_mov_b32 {name}, 1")
        );
        for second in [false, true] {
            let word = 0xbf060000
                | if second {
                    selector << 8 | 128
                } else {
                    128 << 8 | selector
                };
            let Instruction::ScalarCompare { src0, src1, .. } = one(&[word]) else {
                panic!("expected scalar compare")
            };
            let pair = (
                ScalarSource::Register(selector as u8),
                ScalarSource::Immediate(0),
            );
            assert_eq!((src0, src1), if second { (pair.1, pair.0) } else { pair });
        }
    }
}

#[test]
fn vector_moves_integer_sources_and_compare_decode_all_four_halves() {
    for (selector, name) in HALVES {
        let source = VectorSource::Scalar(selector as u8);
        assert_eq!(
            one(&[0x7e060200 | selector]),
            Instruction::MoveB32 {
                dst: 3,
                src: source
            }
        );
        assert_eq!(
            one(&[0x7e060200 | selector]).to_string(),
            format!("v_mov_b32_e32 v3, {name}")
        );
        assert!(
            matches!(one(&[0x7d840400 | selector]), Instruction::VectorCompare { src0, src1:2, .. } if src0 == source)
        );
        assert!(
            matches!(one(&[29 <<25 | 3 <<17 | 2 <<9 | selector]), Instruction::VectorXorB32 { dst:3, src0, src1:2 } if src0 == source)
        );
    }
}

#[test]
fn vop3_mask_halves_are_distinct_bus_keys_before_literal_collapse() {
    // V_ADD3_U32 (877) has three active source positions.
    let keys = [8, 106, 107, 126, 127, 255];
    for a in keys {
        for b in keys {
            for c in keys {
                let raw = [a, b, c];
                let distinct = raw
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let code = bytes(&[0xd76d0003, a | b << 9 | c << 18, 106]);
                let result = decode_at(Gfx1030, &code, 0);
                if distinct > 2 {
                    assert!(matches!(
                        result.unwrap_err().kind,
                        DecodeErrorKind::UnsupportedForm {
                            detail: "expected at most two distinct scalar-bus inputs",
                            ..
                        }
                    ));
                } else {
                    let decoded = result.unwrap();
                    assert_eq!(decoded.byte_len, if raw.contains(&255) { 12 } else { 8 });
                    let Instruction::VectorTernary {
                        src0, src1, src2, ..
                    } = decoded.instruction
                    else {
                        panic!("expected ADD3")
                    };
                    let expected = raw.map(|r| {
                        if r == 255 {
                            VectorSource::Immediate(106)
                        } else {
                            VectorSource::Scalar(r as u8)
                        }
                    });
                    assert_eq!([src0, src1, src2], expected);
                }
            }
        }
    }
    for position in 0..3 {
        for (selector, _) in HALVES {
            let mut raw = [256, 257, 258];
            raw[position] = selector;
            assert!(
                decode(
                    Gfx1030,
                    &bytes(&[0xd76d0003, raw[0] | raw[1] << 9 | raw[2] << 18])
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn reserved_selectors_and_lane_register_restrictions_remain() {
    for selector in 108..=125u32 {
        assert!(decode(Gfx1030, &bytes(&[0xbe880300 | selector])).is_err());
        assert!(decode(Gfx1030, &bytes(&[0x7e060200 | selector])).is_err());
        assert!(decode(Gfx1030, &bytes(&[0x7d840400 | selector])).is_err());
    }
    for (selector, _) in HALVES {
        // READLANE index, WRITELANE data/index and PERMLANE select halves remain normal SGPR-only.
        for words in [
            [0xd7600008, 256 | selector << 9 | 128 << 18],
            [0xd7610003, selector | 8 << 9 | 128 << 18],
            [0xd7610003, 8 | selector << 9 | 128 << 18],
            [0xd7770003, 256 | selector << 9 | 8 << 18],
            [0xd7780003, 256 | 8 << 9 | selector << 18],
            [0xd5820000 | selector, 256],
        ] {
            assert!(decode(Gfx1030, &bytes(&words)).is_err());
        }
        // Scalar arithmetic destinations have not been expanded.
        assert!(decode(Gfx1030, &bytes(&[0x80008180 | selector << 16])).is_err());
    }
}

#[test]
fn mask_half_words_do_not_consume_literals_or_following_instructions() {
    for (selector, _) in HALVES {
        for word in [
            0xbe880300 | selector,
            0x7e060200 | selector,
            0x7d840400 | selector,
        ] {
            let code = bytes(&[word, 0xbf810000]);
            let decoded = decode(Gfx1030, &code).unwrap();
            assert_eq!(decoded.len(), 2);
            assert_eq!(decoded[0].byte_len, 4);
            assert_eq!(decoded[1].offset, 4);
            for cut in 1..4 {
                assert!(decode(Gfx1030, &code[..cut]).is_err());
            }
        }
    }
}
