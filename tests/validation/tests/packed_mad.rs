// SPDX-License-Identifier: GPL-3.0-or-later
//! Original fixed packed multiply-add answers; wrapping signed and unsigned bits coincide.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
const ROWS: &[(u32, u32, u32, u32)] = &[
    (0x0001ffff, 0xffff0001, 0x00010001, 0x00000000),
    (0x80007fff, 0x7fff8000, 0xffff8000, 0x7fff0000),
    (0xffffffff, 0xffffffff, 0xffffffff, 0x00000000),
    (0x80008000, 0x80008000, 0x7fff7fff, 0x7fff7fff),
    (0x00020003, 0x00040005, 0x00060007, 0x000e0016),
    (0x00000000, 0xffffffff, 0x12345678, 0x12345678),
    (0x1234abcd, 0xfedc5678, 0x13579bdf, 0x500701f7),
    (0xffff0001, 0x0001ffff, 0x00010001, 0x00000000),
];
fn program(op: u32, b: u32, c: u32, dst: u32, mask: u32) -> Vec<u8> {
    let mut w = vec![
        0xe0302000, 0x80000100, 0xbf8c3f70, 0x7e0402ff, b, 0x7e0602ff, c,
    ];
    if dst == 255 {
        w.extend([0x7ffe02ff, 0x13579bdf]);
    }
    w.extend([
        0xbefe03ff,
        mask,
        0xcc004000 | op << 16 | dst,
        0x18000000 | 257 | 258 << 9 | 259 << 18,
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    for &(a, b, c, want) in ROWS {
        for op in [0, 9] {
            for dst in [1, 2, 3, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let code = program(op, b, c, dst, mask);
                    let old = match dst {
                        1 => a,
                        2 => b,
                        3 => c,
                        _ => 0x13579bdf,
                    };
                    let input = vec![a; 35];
                    let expected: Vec<_> = (0..35)
                        .map(|lane| {
                            if mask & (1 << (lane % 32)) != 0 {
                                want
                            } else {
                                old
                            }
                        })
                        .collect();
                    assert_eq!(xenora_ref::execute(&code, &input).unwrap(), expected);
                    assert_eq!(independent_ref::execute(&code, &input).unwrap(), expected);
                    let shader = compile(Gfx1030, &code).unwrap();
                    if let Some(gpu) = gpu {
                        assert_eq!(gpu.run_compiled_words(&shader, &input).unwrap(), expected);
                    }
                }
            }
        }
    }
}
#[test]
fn packed_mad_answers_aliases_masks() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_packed_mad() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn all_decoders_reject_packed_mad_modifiers() {
    for op in [0, 9] {
        for bit in (8..16).chain(59..64) {
            let mut code = program(op, 2, 3, 1, u32::MAX);
            let at = 36 + if bit < 32 { 0 } else { 4 };
            let word =
                u32::from_le_bytes(code[at..at + 4].try_into().unwrap()) ^ (1u32 << (bit % 32));
            code[at..at + 4].copy_from_slice(&word.to_le_bytes());
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[1]).is_err());
            assert!(independent_ref::execute(&code, &[1]).is_err());
        }
    }
}
