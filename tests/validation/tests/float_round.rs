// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
fn encoding(byte: u32, wide: bool, dst: u32, src: u32) -> Vec<u32> {
    if wide {
        vec![0xd4000000 | (384 + byte) << 16 | dst, 0x02010000 | src]
    } else {
        vec![0x7e000000 | dst << 17 | byte << 9 | src]
    }
}
fn program(byte: u32, wide: bool, dst: u32, mask: u32) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    if dst != 1 {
        words.extend([0x7e0002ff | dst << 17, 0x13579bdf]);
    }
    words.extend([0xbefe03ff, mask]);
    words.extend(encoding(byte, wide, dst, 257));
    words.extend([
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[path = "../../../fixtures/rdna2/float-round-cases.rs"]
mod cases;
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    for op in [14, 33, 34, 35, 36] {
        for wide in [false, true] {
            for dst in [1, 3, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let input: Vec<u32> = if op == 14 {
                        (0..64)
                            .map(|i| (0xa5963cc0u32.wrapping_add(i / 16 * 0x1000000)) | (i % 16))
                            .collect()
                    } else {
                        cases::ROUND_ROWS.iter().map(|r| r.0).collect()
                    };
                    let expected: Vec<u32> = input
                        .iter()
                        .enumerate()
                        .map(|(i, &v)| {
                            if mask & (1 << (i % 32)) == 0 {
                                if dst == 1 { v } else { 0x13579bdf }
                            } else if op == 14 {
                                cases::OFFSET_BITS[(v & 15) as usize]
                            } else {
                                cases::ROUND_ROWS[i].1[(op - 33) as usize]
                            }
                        })
                        .collect();
                    let code = program(op, wide, dst, mask);
                    assert_eq!(
                        xenora_ref::execute(&code, &input).unwrap(),
                        expected,
                        "op={op}"
                    );
                    assert_eq!(
                        independent_ref::execute(&code, &input).unwrap(),
                        expected,
                        "op={op}"
                    );
                    let shader = compile(Gfx1030, &code).unwrap();
                    if let Some(gpu) = gpu {
                        assert_eq!(
                            gpu.run_compiled_words(&shader, &input).unwrap(),
                            expected,
                            "op={op} wide={wide} dst={dst}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn float_round_known_answers_aliases_masks() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_float_round() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}
#[test]
fn all_decoders_reject_modifiers() {
    for op in [14, 33, 34, 35, 36] {
        for bit in (8..16).chain(41..64) {
            let mut code = program(op, true, 1, u32::MAX);
            code[20 + bit / 8] ^= 1 << (bit % 8);
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[0]).is_err());
            assert!(independent_ref::execute(&code, &[0]).is_err());
        }
    }
}

fn sweep(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<u32> = (0..256)
        .flat_map(|exponent| {
            [0, 1, 0x3fffff, 0x400000, 0x400001, 0x7ffffe, 0x7fffff]
                .into_iter()
                .flat_map(move |fraction| {
                    [0, 0x80000000].map(|sign| sign | exponent << 23 | fraction)
                })
        })
        .collect();
    for op in [33, 34, 35, 36] {
        for wide in [false, true] {
            let code = program(op, wide, 1, u32::MAX);
            let expected = xenora_ref::execute(&code, &input).unwrap();
            assert_eq!(independent_ref::execute(&code, &input).unwrap(), expected);
            let shader = compile(Gfx1030, &code).unwrap();
            if let Some(gpu) = gpu {
                assert_eq!(
                    gpu.run_compiled_words(&shader, &input).unwrap(),
                    expected,
                    "sweep op={op}"
                );
            }
        }
    }
}
#[test]
fn every_exponent_boundary_sweep() {
    sweep(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_round_exponent_sweep() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    sweep(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn rejects_guest_mode_changes() {
    for instruction in [0xbfa40000u32, 0xbfa50000, 0xbfa5000f] {
        let mut code = instruction.to_le_bytes().to_vec();
        code.extend(program(35, false, 1, u32::MAX));
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[1]).is_err());
        assert!(independent_ref::execute(&code, &[1]).is_err());
    }
}
