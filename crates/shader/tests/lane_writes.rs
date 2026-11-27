// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{Architecture::Gfx1030, compile, compile_graphics};
fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
#[test]
fn explicit_mask_ignores_exec_and_replays_in_lane_read_cone() {
    for selector in [0, 31, 32, 63] {
        let result = compile(
            Gfx1030,
            &code(&[
                0xbe880387, 0xbe8903ff, selector, 0xbefe0380, 0xd7610001, 0x02001208, 0x7e140501,
                0xbefe03c1,
            ]),
        )
        .unwrap();
        assert!(
            result
                .msl
                .contains(&format!("0x{:08x}u >>", 1u32 << (selector & 31)))
        );
        assert!(result.msl.contains("input[(index & ~31u)]"));
        assert!(!result.msl.contains("simd_"));
    }
}
#[test]
fn runtime_scalar_snapshot_is_supported_but_runtime_index_is_not() {
    assert!(
        compile(
            Gfx1030,
            &code(&[
                0x7e100501, 0xbe890381, 0xbefe0380, 0xd7610001, 0x02001208, 0xbe880389, 0xbefe03c1
            ])
        )
        .is_ok()
    );
    assert!(
        compile(
            Gfx1030,
            &code(&[0x7e120501, 0xbe880387, 0xd7610001, 0x02001208])
        )
        .is_err()
    );
    // Data and index may alias when known; descriptors are not implicit constants.
    assert!(compile(Gfx1030, &code(&[0xbe880380, 0xd7610001, 0x02001008])).is_ok());
    assert!(compile(Gfx1030, &code(&[0xbe890380, 0xd7610001, 0x02001200])).is_err());
}
#[test]
fn unready_destinations_reject_even_exec_zero() {
    for dst in [0, 2] {
        assert!(
            compile(
                Gfx1030,
                &code(&[
                    0xbe880387,
                    0xbe890380,
                    0xbefe0380,
                    0xd7610000 | dst,
                    0x02001208,
                    0xbefe03c1
                ])
            )
            .is_err()
        );
    }
    assert!(
        compile(
            Gfx1030,
            &code(&[
                0xbe880387, 0xbe890380, 0xe0302000, 0x80000100, 0xbefe0380, 0xd7610001, 0x02001208,
                0xbf8c3f70, 0xbefe03c1
            ])
        )
        .is_err()
    );
}
#[test]
fn runtime_regions_and_skipped_graphics_reject() {
    for branch in [0xbf860002, 0xbf870002] {
        assert!(
            compile(
                Gfx1030,
                &code(&[
                    0xbe880387, 0xbe890380, 0x7d8a0280, branch, 0xd7610001, 0x02001208
                ])
            )
            .is_err()
        );
    }
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for skip in [false, true] {
        let mut w = vec![];
        if skip {
            w.push(0xbf820002);
        }
        w.extend([0xd7610001, 0x02001208]);
        for vertex in [false, true] {
            let mut stream = bytes(&w);
            stream.extend_from_slice(if vertex { vs } else { ps });
            let err = if vertex {
                compile_graphics(Gfx1030, &stream, ps)
            } else {
                compile_graphics(Gfx1030, vs, &stream)
            }
            .unwrap_err();
            assert!(err.to_string().contains("lane writes require"));
        }
    }
}
