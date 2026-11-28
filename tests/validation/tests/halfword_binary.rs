// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixed mathematical halfword answers; never computed using production evaluators.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
const OPS: [u32; 10] = [771, 772, 773, 775, 776, 777, 778, 779, 780, 788];
const ANSWER_INDEX: [usize; 10] = [8, 9, 0, 4, 5, 10, 6, 11, 7, 3];
const ROWS: &[(u32, u32, [u32; 12])] = &[
    (
        0x0001ffff,
        0xffff0001,
        [
            0xffffffff, 0x00000000, 0x0002fffe, 0xfffe8000, 0x7fff0000, 0xffff0000, 0x00010001,
            0xffffffff, 0x00000000, 0x0002fffe, 0xffffffff, 0x00010001,
        ],
    ),
    (
        0x80007fff,
        0x7fff8000,
        [
            0x80008000, 0xffffffff, 0x0001ffff, 0x7fff0000, 0x7fff0001, 0x7fffffff, 0x7fff7fff,
            0x80008000, 0xffffffff, 0x0001ffff, 0x80008000, 0x7fff7fff,
        ],
    ),
    (
        0xffff8000,
        0x0002ffff,
        [
            0xfffe8000, 0x00017fff, 0xfffd8001, 0x0000ffff, 0x0000ffff, 0x0000ffff, 0x0002ffff,
            0xffff8000, 0x00017fff, 0xfffd8001, 0xffffffff, 0x00028000,
        ],
    ),
    (
        0x000f0010,
        0x80018001,
        [
            0x800f0010, 0x80108011, 0x800e800f, 0x80008001, 0x00018001, 0xffff8001, 0x000f0010,
            0x80018001, 0x80108011, 0x800e800f, 0x80018001, 0x000f0010,
        ],
    ),
    (
        0x001f0001,
        0xffff0001,
        [
            0xffe10001, 0x001e0002, 0x00200000, 0x80000002, 0x00010000, 0xffff0000, 0x001f0001,
            0xffff0001, 0x001e0002, 0x00200000, 0xffff0001, 0x001f0001,
        ],
    ),
    (
        0x00000000,
        0xffffffff,
        [
            0x00000000, 0xffffffff, 0x00010001, 0xffffffff, 0xffffffff, 0xffffffff, 0x00000000,
            0xffffffff, 0xffffffff, 0x00010001, 0xffffffff, 0x00000000,
        ],
    ),
    (
        0xffffffff,
        0xffffffff,
        [
            0x00010001, 0xfffefffe, 0x00000000, 0x80008000, 0x00010001, 0xffffffff, 0xffffffff,
            0xffffffff, 0xfffefffe, 0x00000000, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        0x1234abcd,
        0xfedc5678,
        [
            0x3cb06618, 0x11100245, 0x13585555, 0xedc00000, 0x0fed0002, 0xffed0002, 0x12345678,
            0xfedcabcd, 0x11100245, 0x13585555, 0xfedcabcd, 0x12345678,
        ],
    ),
];
fn program(op: u32, b: u32, dst: u32, mask: u32) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70, 0x7e0402ff, b];
    if dst != 1 && dst != 2 {
        w.extend([0x7e0002ff | dst << 17, 0x13579bdf]);
    }
    w.extend([
        0xbefe03ff,
        mask,
        0xd4000000 | op << 16 | dst,
        0x02000000 | 257 | 258 << 9,
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    for &(a, b, wants) in ROWS {
        for (i, op) in OPS.into_iter().enumerate() {
            for dst in [1, 2, 3, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let code = program(op, b, dst, mask);
                    let input = vec![a; 35];
                    let old = if dst == 1 {
                        a
                    } else if dst == 2 {
                        b
                    } else {
                        0x13579bdf
                    };
                    let expected: Vec<_> = (0..35)
                        .map(|lane| {
                            if mask & (1 << (lane % 32)) != 0 {
                                (wants[ANSWER_INDEX[i]] & 65535) | (old & 0xffff0000)
                            } else {
                                old
                            }
                        })
                        .collect();
                    assert_eq!(
                        xenora_ref::execute(&code, &input).unwrap(),
                        expected,
                        "op={op} a={a:x} b={b:x}"
                    );
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
fn halfword_known_answers_aliases_masks() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_halfword_integer() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn rejects_unsupported_modes_in_all_decoders() {
    for op in OPS {
        let original = program(op, 0x8001ffff, 1, u32::MAX);
        for bit in (8..16).chain(50..64) {
            let mut code = original.clone();
            let index = 28 + if bit < 32 { 0 } else { 4 };
            let value =
                u32::from_le_bytes(code[index..index + 4].try_into().unwrap()) ^ (1 << (bit % 32));
            code[index..index + 4].copy_from_slice(&value.to_le_bytes());
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[1]).is_err());
            assert!(independent_ref::execute(&code, &[1]).is_err());
        }
    }
}

#[test]
fn rejects_uninitialized_retained_half() {
    for op in OPS {
        let mut code = program(op, 2, 3, u32::MAX);
        code.drain(20..28);
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[1]).is_err());
        assert!(independent_ref::execute(&code, &[1]).is_err());
    }
}
