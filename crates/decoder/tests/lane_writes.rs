// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::decode;
use xenora_isa::{Architecture::Gfx1030, Instruction, ScalarSource};
fn code(a: u32, b: u32) -> Vec<u8> {
    [a, b].into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn register_domains_and_modifiers_are_exact() {
    for src in 0..512 {
        for lane in 0..512 {
            let mut raw = code(0xd76100ff, src | lane << 9 | 128 << 18);
            if src == 255 {
                raw.extend(0x12345678u32.to_le_bytes());
            }
            let result = decode(Gfx1030, &raw);
            assert_eq!(
                result.is_ok(),
                matches!(src, 0..=105 | 128..=208 | 240..=248 | 255) && lane <= 105
            );
        }
    }
    let base = 0x02001208;
    for bit in 8..16 {
        assert!(decode(Gfx1030, &code(0xd7610001 | 1 << bit, base)).is_err());
    }
    for bit in 18..32 {
        assert!(decode(Gfx1030, &code(0xd7610001, base ^ (1 << bit))).is_err());
    }
    assert!(decode(Gfx1030, &0xd7610001u32.to_le_bytes()).is_err());
    assert_eq!(
        decode(Gfx1030, &code(0xd76100ff, 0x0200d069)).unwrap()[0].instruction,
        Instruction::WriteLaneB32 {
            dst: 255,
            src: ScalarSource::Register(105),
            lane: 104
        }
    );
}

#[test]
fn constants_preserve_bits_and_literal_length() {
    let floats = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for selector in (128..=208).chain(240..=248).chain([255]) {
        let expected = match selector {
            128..=192 => selector - 128,
            193..=208 => 192u32.wrapping_sub(selector),
            240..=248 => floats[(selector - 240) as usize],
            _ => 0x89abcdef,
        };
        let mut raw = code(0xd7610001, selector | 9 << 9 | 128 << 18);
        if selector == 255 {
            raw.extend(expected.to_le_bytes());
        }
        let decoded = decode(Gfx1030, &raw).unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(
            decoded[0].instruction,
            Instruction::WriteLaneB32 {
                dst: 1,
                src: ScalarSource::Immediate(expected),
                lane: 9,
            }
        );
        for length in 0..raw.len() {
            if length != 0 {
                assert!(decode(Gfx1030, &raw[..length]).is_err());
            }
        }
    }
}

#[test]
fn literal_truncation_reports_whole_instruction_length() {
    let mut raw = code(0xd7610001, 255 | 9 << 9 | 128 << 18);
    raw.extend(0x12345678u32.to_le_bytes());
    for available in 8..12 {
        let error = decode(Gfx1030, &raw[..available]).unwrap_err();
        assert_eq!(error.offset, 0);
        assert_eq!(
            error.kind,
            xenora_decoder::DecodeErrorKind::Truncated {
                needed: 12,
                available
            }
        );
    }
}
