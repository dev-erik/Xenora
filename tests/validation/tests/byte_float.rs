// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
fn encoding(byte: u32, wide: bool, dst: u32, src: u32) -> Vec<u32> {
    if wide {
        vec![0xd4000000 | (401 + byte) << 16 | dst, 0x02010000 | src]
    } else {
        vec![0x7e000000 | dst << 17 | (17 + byte) << 9 | src]
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
// Construct exact IEEE normal bits independently of host float conversion.
fn expected(n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    let mut exponent = 0;
    while (1u32 << (exponent + 1)) <= n {
        exponent += 1;
    }
    ((127 + exponent) << 23) | ((n - (1 << exponent)) << (23 - exponent))
}
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    for byte in 0..4 {
        for wide in [false, true] {
            for dst in [1, 3, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let code = program(byte, wide, dst, mask);
                    let input: Vec<u32> = (0..256)
                        .map(|n| (0xa5963cc3 & !(255 << (byte * 8))) | (n << (byte * 8)))
                        .collect();
                    let wants: Vec<u32> = (0..256)
                        .map(|n| {
                            if mask & (1 << (n % 32)) != 0 {
                                expected(n)
                            } else if dst == 1 {
                                input[n as usize]
                            } else {
                                0x13579bdf
                            }
                        })
                        .collect();
                    assert_eq!(xenora_ref::execute(&code, &input).unwrap(), wants);
                    assert_eq!(independent_ref::execute(&code, &input).unwrap(), wants);
                    let shader = compile(Gfx1030, &code).unwrap();
                    if let Some(gpu) = gpu {
                        assert_eq!(gpu.run_compiled_words(&shader, &input).unwrap(), wants);
                    }
                }
            }
        }
    }
}
#[test]
fn every_byte_value_alias_and_exec() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_byte_float() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn rejects_modifiers_in_all_decoders() {
    for byte in 0..4 {
        for bit in (8..16).chain(41..64) {
            let mut code = program(byte, true, 1, u32::MAX);
            code[20 + bit / 8] ^= 1 << (bit % 8);
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[1]).is_err());
            assert!(independent_ref::execute(&code, &[1]).is_err());
        }
    }
    for byte in 0..4 {
        let mut code = program(byte, false, 1, u32::MAX);
        code[22] ^= 1;
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[1]).is_err());
        assert!(independent_ref::execute(&code, &[1]).is_err());
    }
}
