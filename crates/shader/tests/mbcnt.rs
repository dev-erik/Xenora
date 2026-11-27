// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU compiler contract checks; these do not execute Metal arithmetic.
use xenora_shader::{Architecture::Gfx1030, compile, compile_graphics};

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn mbcnt(op: u32, dst: u32, a: u32, b: u32) -> [u32; 2] {
    [0xd4000000 | op << 16 | dst, a | b << 9 | 128 << 18]
}
fn program(body: &[u32]) -> Vec<u8> {
    let mut words = body.to_vec();
    // Store v1, wait for stores, end.
    words.extend([0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
#[test]
fn wave32_lowering_uses_guest_index_and_unsigned_accumulation() {
    for op in [869, 870] {
        let result = compile(Gfx1030, &program(&mbcnt(op, 1, 193, 193))).unwrap();
        assert!(result.msl.contains("0xffffffffu"));
        if op == 869 {
            assert!(
                result
                    .msl
                    .contains("popcount((t0) & ((1u << ((index & 31u))) - 1u)) + (t1)"),
                "{}",
                result.msl
            );
        } else {
            assert!(!result.msl.contains("popcount"));
            assert!(result.msl.contains("uint t2 = (t1);"), "{}", result.msl);
        }
        assert!(!result.msl.contains("simd_"));
    }
}
#[test]
fn aliases_and_partial_exec_preserve_the_previous_destination() {
    for op in [869, 870] {
        for (a, b) in [(257, 129), (193, 257), (257, 257)] {
            let mut body = vec![0x7e0202ff, 0xdeadbeef];
            // s_mov_b32 exec_lo, sparse literal; v1 already has a sentinel.
            body.extend([0xbefe03ff, 0x80000002]);
            body.extend(mbcnt(op, 1, a, b));
            body.push(0xbefe03c1); // restore full EXEC
            let result = compile(Gfx1030, &program(&body)).unwrap();
            assert!(result.ir.contains("merge.lanes mask=0x80000002"));
            assert!(result.msl.contains("0xdeadbeefu"));
            assert!(result.msl.contains("0x80000002u >> (index & 31u)"));
        }
        let mut body = vec![0xbefe0381];
        body.extend(mbcnt(op, 1, 193, 128));
        body.push(0xbefe03c1);
        assert!(compile(Gfx1030, &program(&body)).is_err());
    }
}
#[test]
fn comparison_cone_recomputes_both_forms_at_each_guest_lane() {
    let mut body = mbcnt(869, 1, 193, 128).to_vec();
    body.extend(mbcnt(870, 1, 193, 257));
    // v_cmp_eq_u32 vcc_lo, 1, v1
    body.push(0x7d840281);
    let result = compile(Gfx1030, &program(&body)).unwrap();
    assert!(
        result.msl.contains("(index & ~31u) + lane"),
        "{}",
        result.msl
    );
    assert!(result.msl.contains("if (lane_index >= grid) { break; }"));
    let cone = result.msl.split("for (uint lane").nth(1).unwrap();
    assert!(cone.contains("1u << (lane)"), "{cone}");
    assert!(!cone.contains("1u << ((index & 31u))"));
    assert!(cone.contains(" = (l"), "{cone}");
}
#[test]
fn high_form_still_requires_initialized_sources() {
    for sources in [(258, 128), (193, 258), (8, 128), (193, 8)] {
        assert!(compile(Gfx1030, &program(&mbcnt(870, 1, sources.0, sources.1))).is_err());
    }
}
#[test]
fn both_graphics_stages_reject_both_forms() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for op in [869, 870] {
        for skip in [false, true] {
            let mut prefix = if skip {
                bytes(&[0xbf820002])
            } else {
                Vec::new()
            };
            prefix.extend(bytes(&mbcnt(op, 8, 193, 128)));
            for vertex in [false, true] {
                let mut modified = prefix.clone();
                modified.extend_from_slice(if vertex { vs.as_slice() } else { ps.as_slice() });
                let error = if vertex {
                    compile_graphics(Gfx1030, &modified, ps)
                } else {
                    compile_graphics(Gfx1030, vs, &modified)
                }
                .unwrap_err();
                assert!(
                    error.to_string().contains("Wave32 compute lane contract"),
                    "{error}"
                );
            }
        }
    }
}
