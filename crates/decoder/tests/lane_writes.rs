// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::decode;
use xenora_isa::{Architecture::Gfx1030, Instruction};
fn code(a: u32, b: u32) -> Vec<u8> {
    [a, b].into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn register_domains_and_modifiers_are_exact() {
    for src in 0..512 {
        for lane in [0, 105, 106, 124, 128, 255, 256, 511] {
            let result = decode(Gfx1030, &code(0xd76100ff, src | lane << 9 | 128 << 18));
            assert_eq!(result.is_ok(), src <= 105 && lane <= 105);
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
            src: 105,
            lane: 104
        }
    );
}
