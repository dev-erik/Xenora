// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed boundary answers for low16 arithmetic and saturating integer packing.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
const OPS: [u32; 10] = [832, 850, 851, 853, 854, 856, 857, 862, 874, 875];
const ROWS: &[(u32, u32, u32, [u32; 10])] = &[
    (
        0x11110002,
        0x22220003,
        0x33330004,
        [
            0x0000000a, 0x00000002, 0x00000002, 0x00000004, 0x00000004, 0x00000003, 0x00000003,
            0x0000000a, 0xffffffff, 0x7fff7fff,
        ],
    ),
    (
        0xaaaaffff,
        0x55550001,
        0x98760000,
        [
            0x0000ffff, 0x0000ffff, 0x00000000, 0x00000001, 0x0000ffff, 0x00000000, 0x00000001,
            0x0000ffff, 0xffffffff, 0x7fff8000,
        ],
    ),
    (
        0x12348000,
        0xabcd7fff,
        0xfeedffff,
        [
            0x00007fff, 0x00008000, 0x00007fff, 0x00007fff, 0x0000ffff, 0x0000ffff, 0x00008000,
            0x00007fff, 0xffffffff, 0x80007fff,
        ],
    ),
    (
        0xffff7fff,
        0x00008000,
        0x00000001,
        [
            0x00008001, 0x00008000, 0x00000001, 0x00007fff, 0x00008000, 0x00000001, 0x00007fff,
            0x00008001, 0x8000ffff, 0x7fff8000,
        ],
    ),
    (
        0x80000000,
        0x7fffffff,
        0xffffffff,
        [
            0x0000ffff, 0x0000ffff, 0x00000000, 0x00000000, 0x0000ffff, 0x0000ffff, 0x0000ffff,
            0x0000ffff, 0xffffffff, 0x7fff8000,
        ],
    ),
    (
        0x00010000,
        0xffff7fff,
        0x80000000,
        [
            0x00000000, 0x00000000, 0x00000000, 0x00007fff, 0x00007fff, 0x00000000, 0x00000000,
            0x00000000, 0xffffffff, 0x80007fff,
        ],
    ),
    (
        0xffffffff,
        0xffffffff,
        0xffffffff,
        [
            0x00000000, 0x0000ffff, 0x0000ffff, 0x0000ffff, 0x0000ffff, 0x0000ffff, 0x0000ffff,
            0x00000000, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        0x00007fff,
        0xffff8000,
        0x00008000,
        [
            0x00000000, 0x00008000, 0x00007fff, 0x00007fff, 0x00008000, 0x00008000, 0x00008000,
            0x00000000, 0xffff7fff, 0x80007fff,
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
        0xd4000000 | op << 16 | dst,
        257 | 258 << 9 | (if op >= 874 { 128 } else { 259 }) << 18,
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
        for (i, op) in OPS.into_iter().enumerate() {
            for dst in [1, 2, 3, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let code = program(op, b, c, dst, mask);
                    let input = vec![a; 35];
                    let old = match dst {
                        1 => a,
                        2 => b,
                        3 => c,
                        _ => 0x13579bdf,
                    };
                    let value = if op >= 874 {
                        wants[i]
                    } else {
                        (old & 0xffff0000) | wants[i]
                    };
                    let expected: Vec<_> = (0..35)
                        .map(|lane| {
                            if mask & (1 << (lane % 32)) != 0 {
                                value
                            } else {
                                old
                            }
                        })
                        .collect();
                    assert_eq!(
                        xenora_ref::execute(&code, &input).unwrap(),
                        expected,
                        "op={op} row={a:x},{b:x},{c:x}"
                    );
                    assert_eq!(independent_ref::execute(&code, &input).unwrap(), expected);
                    let shader = compile(Gfx1030, &code).unwrap();
                    if let Some(gpu) = gpu {
                        assert_eq!(
                            gpu.run_compiled_words(&shader, &input).unwrap(),
                            expected,
                            "op={op} row={a:x},{b:x},{c:x}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn halfword_ternary_known_answers() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_halfword_ternary() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}
#[test]
fn rejects_uninitialized_retained_half() {
    for op in &OPS[..8] {
        let mut code = program(*op, 2, 3, 255, u32::MAX);
        code.drain(28..36);
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[1]).is_err());
        assert!(independent_ref::execute(&code, &[1]).is_err());
    }
}
#[test]
fn rejects_modifiers_in_all_decoders() {
    for op in OPS {
        for bit in (8..16).chain(if op >= 874 { 50..64 } else { 59..64 }) {
            let mut code = program(op, 2, 3, 1, u32::MAX);
            let pos = 36 + if bit < 32 { 0 } else { 4 };
            let w = u32::from_le_bytes(code[pos..pos + 4].try_into().unwrap()) ^ (1 << (bit % 32));
            code[pos..pos + 4].copy_from_slice(&w.to_le_bytes());
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[1]).is_err());
            assert!(independent_ref::execute(&code, &[1]).is_err());
        }
    }
}
