// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{
    Architecture::Gfx1030, CompileOptions, Wave32Launch, compile, compile_with_options,
};
fn options() -> CompileOptions {
    CompileOptions {
        wave32_launch: Wave32Launch::FullWavesRequired,
    }
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn launch_validation_and_guard_precede_input_access() {
    let p = code(&[0xbe89039f, 0xd7600008, 0x02001301, 0x7e040208]);
    assert!(compile(Gfx1030, &p).is_err());
    let s = compile_with_options(Gfx1030, &p, options()).unwrap();
    for n in [0, 1, 31, 33, 4095, 4097, usize::MAX] {
        assert!(s.validate_launch(n).is_err());
    }
    for n in [32, 64, 4096] {
        assert!(s.validate_launch(n).is_ok());
    }
    assert!(s.msl.find("if (grid == 0u").unwrap() < s.msl.find("input[index]").unwrap());
    assert!(s.msl.contains("& 31u)"));
    assert!(!s.msl.contains("simd_"));
    let s = compile(Gfx1030, &code(&[0x7e040301])).unwrap();
    for n in [1, 31, 32, 33, 4096] {
        assert!(s.validate_launch(n).is_ok());
    }
    for n in [0, 4097] {
        assert!(s.validate_launch(n).is_err());
    }
}
#[test]
fn runtime_selector_alias_and_readfirst_masks_compile() {
    let p = code(&[0x7e100501, 0xd7600008, 0x02001101, 0x7e020289, 0x7e040208]);
    assert!(compile(Gfx1030, &p).is_err());
    let s = compile_with_options(Gfx1030, &p, options()).unwrap();
    assert!(s.msl.contains("((index & ~31u) +"));
    for mask in [0, 1, 2, 0x80000000, u32::MAX] {
        assert!(
            compile_with_options(
                Gfx1030,
                &code(&[0xbefe03ff, mask, 0x7e100501, 0xbefe03c1, 0x7e040208]),
                options()
            )
            .is_ok()
        );
    }
    assert!(
        compile_with_options(
            Gfx1030,
            &code(&[0xd7600008, 0x02001301, 0x7e040208]),
            options()
        )
        .is_err()
    );
}
