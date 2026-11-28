// SPDX-License-Identifier: GPL-3.0-or-later
//! Original fixed packed integer dot-product answers; signed and unsigned lane widths differ.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
const ROWS: &[(u32, u32, u32, [u32; 6])] = &[
    (
        0x00000001,
        0x00000001,
        0x00000000,
        [
            0x00000001, 0x00000001, 0x00000001, 0x00000001, 0x00000001, 0x00000001,
        ],
    ),
    (
        0xffffffff,
        0xffffffff,
        0xffffffff,
        [
            0x00000001, 0xfffc0001, 0x00000003, 0x0003f803, 0x00000007, 0x00000707,
        ],
    ),
    (
        0x80008000,
        0x80008000,
        0x00000000,
        [
            0x80000000, 0x80000000, 0x00008000, 0x00008000, 0x00000080, 0x00000080,
        ],
    ),
    (
        0x80007fff,
        0x7fff8000,
        0x80000000,
        [
            0x00010000, 0xffff0000, 0x7fff8100, 0x80007f00, 0x7fffff90, 0x80000070,
        ],
    ),
    (
        0x80808080,
        0x7f7f7f7f,
        0x7fffffff,
        [
            0x01007eff, 0xfffe7eff, 0x7fff01ff, 0x8000fdff, 0x7fffff1f, 0x800000df,
        ],
    ),
    (
        0x88888888,
        0x77777777,
        0xffffffff,
        [
            0x907f5e6f, 0x7f6d5e6f, 0xffff20df, 0x0000fcdf, 0xfffffe3f, 0x000001bf,
        ],
    ),
    (
        0x1234abcd,
        0xfedc5678,
        0x13579bdf,
        [
            0xf6d23ea7, 0x5f7e3ea7, 0x13575ff5, 0x135873f5, 0x13579b81, 0x13579d91,
        ],
    ),
    (
        0x01020304,
        0x05060708,
        0x00000009,
        [
            0x001a4435, 0x001a4435, 0x0000004f, 0x0000004f, 0x0000000f, 0x0000004f,
        ],
    ),
    (
        0xffff0001,
        0x0001ffff,
        0x00000000,
        [
            0xfffffffe, 0x0001fffe, 0xfffffffe, 0x000001fe, 0xfffffffe, 0x0000001e,
        ],
    ),
    (
        0x00000000,
        0xffffffff,
        0x12345678,
        [
            0x12345678, 0x12345678, 0x12345678, 0x12345678, 0x12345678, 0x12345678,
        ],
    ),
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
    for &(a, b, c, wants) in ROWS {
        for (index, op) in (20..=25).enumerate() {
            let want = wants[index];
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
fn packed_dot_answers_aliases_masks() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_packed_dot() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn all_decoders_reject_packed_dot_modifiers() {
    for op in 20..=25 {
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
