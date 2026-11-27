// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal cases from AMD RDNA2 section12.2, not production evaluators.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture, compile};
fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn sopk(op: u32, dst: u32, imm: u16) -> u32 {
    0xb0000000 | op << 23 | dst << 16 | u32::from(imm)
}
fn program(op: u32, initial: Option<u32>, imm: u16, scc: Option<bool>, consume: bool) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    if let Some(v) = initial {
        w.extend([0xbe8803ff, v]);
    }
    if let Some(v) = scc {
        w.push(if v { 0xbf068080 } else { 0xbf068180 });
    }
    w.push(sopk(op, 8, imm));
    if consume {
        w.push(0x85098180);
    } // CSELECT s9,0,1: requires SCC, doesn't change it.
    w.push(0x7e040208);
    if scc.is_some() || op == 15 {
        w.extend([0xbf840001, 0x3a040501]);
    }
    w.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>)> {
    let mut c = Vec::new();
    for (imm, d) in [
        (0, 0),
        (1, 1),
        (0x7fff, 0x7fff),
        (0x8000, 0xffff8000),
        (0xffff, u32::MAX),
        (0x80ff, 0xffff80ff),
    ] {
        for scc in [None, Some(false), Some(true)] {
            c.push((
                program(0, None, imm, scc, false),
                vec![d, if scc == Some(true) { !d } else { d }],
            ));
        }
    }
    for (a, imm, d, flag) in [
        (0x7fffffff, 1, 0x80000000, true),
        (0x80000000, 0xffff, 0x7fffffff, true),
        (u32::MAX, 1, 0, false),
        (5, 0xfffd, 2, false),
        (0x80000000, 0x7fff, 0x80007fff, false),
        (0, 0x8000, 0xffff8000, false),
    ] {
        for incoming in [None, Some(false), Some(true)] {
            c.push((
                program(15, Some(a), imm, incoming, false),
                vec![d, if flag { !d } else { d }],
            ));
        }
    }
    for (a, imm, d) in [
        (u32::MAX, 0x8000, 0x8000),
        (0x80000000, 0xffff, 0x80000000),
        (0x7fffffff, 2, 0xfffffffe),
        (123, 0, 0),
        (3, 0xfffe, 0xfffffffa),
        (0x12345678, 1, 0x12345678),
    ] {
        for scc in [None, Some(false), Some(true)] {
            c.push((
                program(16, Some(a), imm, scc, false),
                vec![d, if scc == Some(true) { !d } else { d }],
            ));
        }
    }
    assert_eq!(c.len(), 54);
    c
}
#[test]
fn scalar_immediates_match_literal_results_and_flags() {
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
fn scalar_immediates_preserve_unset_scc_and_require_old_destinations() {
    for op in [0, 16] {
        let code = program(op, Some(7), 0xffff, None, true);
        assert!(compile(Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
    for op in [15, 16] {
        let code = program(op, None, 1, None, false);
        assert!(compile(Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
    for op in [0, 15, 16] {
        for dst in [0, 7, 106, 124, 126, 127] {
            let code = bytes(&[sopk(op, dst, 1), 0xbf810000]);
            assert!(compile(Architecture::Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[0]).is_err());
            assert!(independent_ref::execute(&code, &[0]).is_err());
        }
    }
}
#[test]
#[ignore = "requires native Metal validation"]
fn scalar_immediates_match_literal_results_on_native_metal() {
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
fn assembled_immediate_chain_matches_literal_result() {
    let code = include_bytes!("../../../fixtures/rdna2/scalar-immediate.bin");
    let want = [0x8001, 0xffff7ffe];
    assert_eq!(xenora_ref::execute(code, &[0, u32::MAX]).unwrap(), want);
    assert_eq!(
        independent_ref::execute(code, &[0, u32::MAX]).unwrap(),
        want
    );
    compile(Architecture::Gfx1030, code).unwrap();
}

#[test]
fn graphics_immediates_preserve_unset_scc() {
    use xenora_shader::compile_graphics_pipeline;
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    for original in [
        include_bytes!("../../../fixtures/rdna2/x9-scalar-movk-ps.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/x9-scalar-mulk-ps.bin").as_slice(),
    ] {
        let mut ps = original.to_vec();
        // Replace incoming SCC comparison with a MOV; the later SCC consumer must reject.
        ps[8..12].copy_from_slice(&0xbea20380u32.to_le_bytes());
        assert!(compile_graphics_pipeline(Architecture::Gfx1030, vs, &ps).is_err());
    }
}
