// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal relation masks derived from AMD RDNA2 §12.2: EQ, LG, GT, GE, LT, LE.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture, compile};
// Bits 0..5 are comparison outcomes, not values produced by an ISA evaluator.
const ROWS: [(u32, u16, u8, u8); 12] = [
    (0, 0, 0x29, 0x29),
    (0xffffffff, 0xffff, 0x29, 0x0e),
    (0xffff8000, 0x8000, 0x29, 0x0e),
    (0x8000, 0x8000, 0x0e, 0x29),
    (0x7fffffff, 0xffff, 0x0e, 0x0e),
    (0x80000000, 0, 0x32, 0x0e),
    (0, 0xffff, 0x0e, 0x32),
    (0x7ffe, 0x7fff, 0x32, 0x32),
    (0x10000, 0xffff, 0x0e, 0x0e),
    (0xffffffff, 0, 0x32, 0x0e),
    (0xffff7fff, 0x8000, 0x32, 0x0e),
    (0xff, 0xff, 0x29, 0x29),
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn program(op: u32, a: Option<u32>, imm: u16, incoming: Option<bool>) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    if let Some(a) = a {
        w.extend([0xbe8803ff, a]);
    }
    if let Some(flag) = incoming {
        w.push(if flag { 0xbf068080 } else { 0xbf068180 });
    }
    w.push(0xb0000000 | op << 23 | 8 << 16 | u32::from(imm));
    // Comparison must not modify s8. Consume SCC through a branch and s8 as output.
    w.extend([
        0x7e040208, 0xbf840001, 0x3a040501, 0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000,
    ]);
    bytes(&w)
}
fn cases() -> Vec<(Vec<u8>, [u32; 2])> {
    let mut result = vec![];
    for (a, imm, signed, unsigned) in ROWS {
        for op in 3..=14 {
            for incoming in [None, Some(false), Some(true)] {
                let mask = if op < 9 { signed } else { unsigned };
                let flag = mask & (1 << ((op - 3) % 6)) != 0;
                result.push((
                    program(op, Some(a), imm, incoming),
                    [a, if flag { !a } else { a }],
                ));
            }
        }
    }
    assert_eq!(result.len(), 432);
    result
}
#[test]
fn immediate_comparisons_match_literal_relations() {
    for (code, want) in cases() {
        compile(Architecture::Gfx1030, &code).unwrap();
        assert_eq!(xenora_ref::execute(&code, &[0, u32::MAX]).unwrap(), want);
        assert_eq!(
            independent_ref::execute(&code, &[0, u32::MAX]).unwrap(),
            want
        );
    }
}
#[test]
fn immediate_comparisons_reject_uninitialized_or_special_sources() {
    for op in 3..=14 {
        let code = program(op, None, 0, None);
        assert!(compile(Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
        for reg in 106..=127 {
            let code = bytes(&[0xb0000000 | op << 23 | reg << 16, 0xbf810000]);
            assert!(compile(Architecture::Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[0]).is_err());
            assert!(independent_ref::execute(&code, &[0]).is_err());
        }
    }
}
#[test]
#[ignore = "requires native Metal validation"]
fn immediate_comparisons_match_literal_relations_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, want) in cases() {
        let shader = compile(Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&shader.msl, xenora_shader::ENTRY_POINT, &[0, u32::MAX])
                .unwrap(),
            want
        );
    }
}

#[test]
fn assembled_comparison_chain_matches_literal_result() {
    let code = include_bytes!("../../../fixtures/rdna2/scalar-compare-immediate.bin");
    compile(Architecture::Gfx1030, code).unwrap();
    assert_eq!(
        xenora_ref::execute(code, &[0, u32::MAX]).unwrap(),
        [u32::MAX, u32::MAX]
    );
    assert_eq!(
        independent_ref::execute(code, &[0, u32::MAX]).unwrap(),
        [u32::MAX, u32::MAX]
    );
}
#[test]
fn graphics_immediate_comparisons_require_initialized_sources() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    for op in 3..=14 {
        let mut ps = include_bytes!("../../../fixtures/rdna2/x9-scalar-cmpk-ps.bin").to_vec();
        ps[12..16].copy_from_slice(&(0xb0000000u32 | op << 23 | 34 << 16 | 0xffff).to_le_bytes());
        assert!(xenora_shader::compile_graphics_pipeline(Architecture::Gfx1030, vs, &ps).is_err());
    }
}
