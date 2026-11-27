// SPDX-License-Identifier: GPL-3.0-or-later
//! AMD RDNA2 p181: original fixed selector answers, independent of evaluators.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};

fn program(selector: u32, dst: u32, mask: u32) -> Vec<u8> {
    let mut words = vec![
        0xe0302000, 0x80000100, 0xbf8c3f70, 0x7e0402ff, 0x807f81ff, 0x7e0602ff, selector,
    ];
    if dst == 4 {
        words.extend([0x7e0802ff, 0x13579bdf]);
    }
    words.extend([
        0xbefe03ff,
        mask,
        0xd7440000 | dst,
        257 | 258 << 9 | 259 << 18,
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
// S1 supplies selectors 0..3, S0 supplies 4..7; 8..11 sign-expand odd bytes.
const ANSWERS: [u32; 13] = [
    0xff, 0x81, 0x7f, 0x80, 0x01, 0x7e, 0xfe, 0x00, 0xff, 0xff, 0, 0, 0,
];
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    for selector in 0..256u32 {
        let want = ANSWERS.get(selector as usize).copied().unwrap_or(255) * 0x01010101;
        let control = selector * 0x01010101;
        for dst in [1, 2, 3, 4] {
            for mask in [u32::MAX, 0x80000005, 0] {
                let code = program(control, dst, mask);
                let old = [0, 0x00fe7e01, 0x807f81ff, control, 0x13579bdf][dst as usize];
                let expected: Vec<_> = (0..35)
                    .map(|lane| {
                        if mask & (1 << (lane % 32)) != 0 {
                            want
                        } else {
                            old
                        }
                    })
                    .collect();
                let input = vec![0x00fe7e01; 35];
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
#[test]
fn all_selectors_aliases_and_masks() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_perm_b32_compute() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn rejects_modifiers() {
    let original = program(0x07040100, 4, u32::MAX);
    // Destination initializer adds two words: V_PERM starts at word 11.
    for bit in 8..16 {
        let mut code = original.clone();
        let first = 0xd7440004 | (1 << bit);
        code[44..48].copy_from_slice(&u32::to_le_bytes(first));
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
    for bit in 27..32 {
        let mut code = original.clone();
        let second = 257 | 258 << 9 | 259 << 18 | 1 << bit;
        code[48..52].copy_from_slice(&u32::to_le_bytes(second));
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
}
