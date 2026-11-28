// SPDX-License-Identifier: GPL-3.0-or-later
//! Original bit-copy answers; VOP3 encoding must retain the VOP1 move contract.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
fn program(e64: bool, dst: u32, source: u32, bits: u32, mask: u32) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70, 0xbe8803ff, bits];
    if dst != 1 {
        w.extend([0x7e0002ff | dst << 17, 0x13579bdf]);
    }
    w.extend([0xbefe03ff, mask]);
    if e64 {
        w.extend([0xd5810000 | dst, source | 128 << 9 | 128 << 18]);
    } else {
        w.push(0x7e000200 | dst << 17 | source);
    }
    if source == 255 {
        w.push(bits);
    }
    w.extend([
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    let mut count = 0;
    for bits in [
        0, 1, 0x80000000, 0xffffffff, 0x7f800000, 0xff800000, 0x7fa12345, 0x007fffff,
    ] {
        for source in [257, 255, 8] {
            for dst in [1, 2, 255] {
                for mask in [u32::MAX, 0x80000005, 0] {
                    let code = program(true, dst, source, bits, mask);
                    let want: Vec<_> = (0..35)
                        .map(|lane| {
                            if mask & (1 << (lane % 32)) != 0 || dst == 1 {
                                bits
                            } else {
                                0x13579bdf
                            }
                        })
                        .collect();
                    let input = vec![bits; 35];
                    assert_eq!(xenora_ref::execute(&code, &input).unwrap(), want);
                    assert_eq!(independent_ref::execute(&code, &input).unwrap(), want);
                    let shader = compile(Gfx1030, &code).unwrap();
                    let original =
                        compile(Gfx1030, &program(false, dst, source, bits, mask)).unwrap();
                    assert_eq!(shader.msl, original.msl);
                    if let Some(gpu) = gpu {
                        assert_eq!(gpu.run_compiled_words(&shader, &input).unwrap(), want);
                    }
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 216);
}
#[test]
fn exact_words_sources_aliases_masks_and_existing_lowering() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_move_e64() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn assembled_move_keeps_words() {
    let code = include_bytes!("../../../fixtures/rdna2/vop3-move.bin");
    let input = [0, 0x80000000, 0x7fa12345, 0xffffffff];
    assert_eq!(xenora_ref::execute(code, &input).unwrap(), input);
    assert_eq!(independent_ref::execute(code, &input).unwrap(), input);
    compile(Gfx1030, code).unwrap();
}
