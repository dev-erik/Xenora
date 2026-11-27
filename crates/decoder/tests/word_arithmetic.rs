// SPDX-License-Identifier: GPL-3.0-or-later
//! Operand contracts for signed add and byte permutation, with literal encodings.
use xenora_decoder::{Architecture::Gfx1030, decode};
fn code(op: u32, sources: [u32; 3], literal: bool) -> Vec<u8> {
    let mut words = vec![
        0xd4000000 | op << 16 | 255,
        sources[0] | sources[1] << 9 | sources[2] << 18,
    ];
    if literal {
        words.push(0x01234567);
    }
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn source_domains_lengths_and_constant_bus() {
    for (op, arity) in [(895, 2), (836, 3)] {
        for position in 0..arity {
            for source in 0..512 {
                let mut sources = [257, 258, if arity == 2 { 128 } else { 259 }];
                sources[position] = source;
                let admitted = matches!(source, 0..=107 | 126..=208 | 240..=248 | 255..=511);
                let result = decode(Gfx1030, &code(op, sources, source == 255));
                assert_eq!(
                    result.is_ok(),
                    admitted,
                    "opcode={op} pos={position} src={source}"
                );
                if let Ok(decoded) = result {
                    assert_eq!(decoded.len(), 1);
                    assert_eq!(decoded[0].byte_len, if source == 255 { 12 } else { 8 });
                    assert_eq!(
                        decoded[0].instruction.mnemonic(),
                        if op == 895 {
                            "v_add_nc_i32"
                        } else {
                            "v_perm_b32"
                        }
                    );
                }
                if source == 255 {
                    assert!(decode(Gfx1030, &code(op, sources, false)).is_err());
                }
            }
        }
    }
    assert!(decode(Gfx1030, &code(836, [8, 9, 10], false)).is_err());
    assert!(decode(Gfx1030, &code(836, [8, 8, 9], false)).is_ok());
    assert!(decode(Gfx1030, &code(836, [255; 3], true)).is_ok());
    assert!(decode(Gfx1030, &code(895, [257, 258, 0], false)).is_err());
}
