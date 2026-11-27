// SPDX-License-Identifier: GPL-3.0-or-later
//! Original known answers from AMD RDNA2 guide 12.1, scalar packing.
//! Literal expectations do not call the production semantic primitives.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture, compile};

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn pack(op: u32, dst: u32, a: u32, b: u32) -> u32 {
    0x8000_0000 | op << 23 | dst << 16 | b << 8 | a
}
fn program(op: u32, dst: u32, a: u32, b: u32, literal: Option<u32>, scc: Option<bool>) -> Vec<u8> {
    let mut w = vec![
        0xe030_2000,
        0x8000_0100,
        0xbf8c_3f70,
        0xbe88_03ff,
        0x1234_abcd,
        0xbe89_03ff,
        0x5678_ef01,
    ];
    if let Some(flag) = scc {
        w.push(if flag { 0xbf06_8080 } else { 0xbf06_8180 });
    }
    w.push(pack(op, dst, a, b));
    if let Some(v) = literal {
        w.push(v);
    }
    w.push(0x7e04_0200 | dst);
    if scc.is_some() {
        w.extend([0xbf84_0001, 0x3a04_0501]);
    }
    w.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    bytes(&w)
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>)> {
    let mut out = Vec::new();
    for (op, expected) in [(50, 0xef01_abcd), (51, 0x5678_abcd), (52, 0x5678_1234)] {
        for scc in [None, Some(false), Some(true)] {
            for dst in [8, 9, 10] {
                out.push((
                    program(op, dst, 8, 9, None, scc),
                    vec![
                        expected,
                        if scc == Some(true) {
                            !expected
                        } else {
                            expected
                        },
                    ],
                ));
            }
        }
    }
    // Distinct halves, inline negative/positive, literal in either source,
    // equal sources and all-zero/all-one results. Both SCC states must survive.
    for (op, a, b, lit, d) in [
        (50, 255, 9, Some(0x8000_0001), 0xef01_0001),
        (51, 255, 9, Some(0x8000_0001), 0x5678_0001),
        (52, 255, 9, Some(0x8000_0001), 0x5678_8000),
        (50, 8, 255, Some(0x8000_0001), 0x0001_abcd),
        (51, 8, 255, Some(0x8000_0001), 0x8000_abcd),
        (52, 8, 255, Some(0x8000_0001), 0x8000_1234),
        (50, 193, 192, None, 0x0040_ffff),
        (51, 193, 192, None, 0x0000_ffff),
        (52, 193, 192, None, 0x0000_ffff),
        (50, 8, 8, None, 0xabcd_abcd),
        (51, 8, 8, None, 0x1234_abcd),
        (52, 8, 8, None, 0x1234_1234),
        (50, 128, 128, None, 0),
        (51, 128, 128, None, 0),
        (52, 128, 128, None, 0),
        (50, 193, 193, None, u32::MAX),
        (51, 193, 193, None, u32::MAX),
        (52, 193, 193, None, u32::MAX),
    ] {
        for scc in [false, true] {
            out.push((
                program(op, 10, a, b, lit, Some(scc)),
                vec![d, if scc { !d } else { d }],
            ));
        }
    }
    assert_eq!(out.len(), 63);
    out.push((
        include_bytes!("../../../fixtures/rdna2/scalar-pack.bin").to_vec(),
        vec![0xef01_1234, 0x10fe_edcb],
    ));
    out
}
#[test]
fn scalar_pack_literal_answers_and_scc_match_both_interpreters() {
    for (code, expected) in cases() {
        compile(Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            xenora_ref::execute(&code, &[0, u32::MAX]).unwrap(),
            expected
        );
        assert_eq!(
            independent_ref::execute(&code, &[0, u32::MAX]).unwrap(),
            expected
        );
    }
}
#[test]
fn scalar_pack_invalid_operands_and_missing_literal_reject() {
    for op in 50..=52 {
        for (dst, a, b, lit) in [
            (106, 8, 9, None),
            (10, 106, 9, None),
            (10, 8, 106, None),
            (10, 255, 255, Some(1)),
            (10, 240, 9, None),
        ] {
            let code = program(op, dst, a, b, lit, None);
            assert!(compile(Architecture::Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[0]).is_err());
            assert!(independent_ref::execute(&code, &[0]).is_err());
        }
        let code = bytes(&[pack(op, 10, 255, 8)]);
        assert!(compile(Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
}
#[test]
#[ignore = "requires native Metal with validation enabled"]
fn scalar_pack_literal_answers_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, expected) in cases() {
        let shader = compile(Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&shader.msl, xenora_shader::ENTRY_POINT, &[0, u32::MAX])
                .unwrap(),
            expected
        );
    }
}

#[test]
fn scalar_pack_does_not_initialize_scc_for_later_select() {
    for op in 50..=52 {
        let mut code = program(op, 10, 8, 9, None, None);
        // PACK is at byte28; CSELECT immediately afterwards must reject unset SCC.
        code.splice(32..32, pack(10, 10, 128, 129).to_le_bytes());
        assert!(compile(Architecture::Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    for ps in [
        include_bytes!("../../../fixtures/rdna2/x9-scalar-pack-ll-ps.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/x9-scalar-pack-lh-ps.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/x9-scalar-pack-hh-ps.bin").as_slice(),
    ] {
        let mut code = ps.to_vec();
        code.drain(8..12); // Remove the only preceding SCC initialization.
        code.splice(12..12, pack(10, 32, 128, 129).to_le_bytes());
        let error =
            xenora_shader::compile_graphics_pipeline(Architecture::Gfx1030, vs, &code).unwrap_err();
        assert!(
            error.to_string().contains("SCC is uninitialized"),
            "{error}"
        );
    }
}
