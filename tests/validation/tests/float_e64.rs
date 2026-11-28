// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
fn program(op: u32, dst: u32, wide: bool, value: u32) -> Vec<u8> {
    let mut words = vec![0x7e0202ff, value, 0x7e0402f4];
    if wide {
        words.extend([
            0xd4000000 | (256 + op) << 16 | dst,
            257 | 258 << 9 | 128 << 18,
        ]);
    } else {
        words.push(op << 25 | dst << 17 | 2 << 9 | 257);
    }
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn check(gpu: Option<&xenora_metal::NativeCompute>) {
    let input = [1.0f32, 2.0, 4.0, 8.0].map(f32::to_bits);
    for (op, answers) in [(3, [3.0f32, 4.0, 6.0, 10.0]), (8, [2.0, 4.0, 8.0, 16.0])] {
        for dst in [1, 2, 3, 255] {
            for (value, answer) in input.into_iter().zip(answers) {
                let code = program(op, dst, true, value);
                let expected = [answer.to_bits(); 4];
                assert_eq!(xenora_ref::execute(&code, &input).unwrap(), expected);
                assert_eq!(independent_ref::execute(&code, &input).unwrap(), expected);
                let shader = compile(Gfx1030, &code).unwrap();
                assert_eq!(
                    shader.msl,
                    compile(Gfx1030, &program(op, dst, false, value))
                        .unwrap()
                        .msl
                );
                if let Some(gpu) = gpu {
                    assert_eq!(gpu.run_compiled_words(&shader, &input).unwrap(), expected);
                }
            }
        }
    }
}
#[test]
fn known_answers_and_alias_parity() {
    check(None);
}
#[test]
#[ignore = "requires native Metal validation"]
fn native_float_e64() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    check(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn runtime_compute_keeps_existing_numerical_rejection() {
    for op in [3, 8] {
        for wide in [false, true] {
            let code = program(op, 3, wide, 1.0f32.to_bits());
            let mut runtime: Vec<u8> = [0xe0302000u32, 0x80000100, 0xbf8c3f70]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            runtime.extend_from_slice(&code[8..]);
            assert!(
                compile(Gfx1030, &runtime)
                    .unwrap_err()
                    .to_string()
                    .contains("not proven exact")
            );
        }
    }
}
