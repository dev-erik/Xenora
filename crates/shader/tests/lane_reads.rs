// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{Architecture::Gfx1030, compile, compile_graphics};
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn program(body: &[u32]) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(body);
    words.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
fn reject(body: &[u32], reason: &str) {
    let e = compile(Gfx1030, &program(body)).unwrap_err();
    assert!(e.to_string().contains(reason), "{e}");
}
#[test]
fn readfirst_replays_original_source_snapshot_at_guest_wave_base() {
    let result = compile(
        Gfx1030,
        &program(&[0x7e100501, 0x7e020289, 0x7e040208, 0x7d840480]),
    )
    .unwrap();
    assert!(
        result.msl.contains("input[(index & ~31u)]"),
        "{}",
        result.msl
    );
    assert_eq!(result.msl.matches("input[").count(), 2);
    assert!(!result.msl.contains("simd_"));
    assert!(result.ir.contains("read.lane0"));
    assert!(result.msl.contains("lane_index >= grid"));
}
#[test]
fn source_cone_includes_narrow_loads_masked_writes_mbcnt_and_prior_lane_reads() {
    let mut code = program(&[0xbefe0380, 0x7e100501, 0xbefe03c1, 0x7e040208]);
    code[..4].copy_from_slice(&0xe0242000u32.to_le_bytes());
    let result = compile(Gfx1030, &code).unwrap();
    assert!(
        result
            .msl
            .contains("((input[(index & ~31u)] & 0xffu) ^ 0x80u) - 0x80u")
    );
    let result = compile(
        Gfx1030,
        &program(&[
            0xbefe0382,
            0x7e020289,
            0xbefe0380,
            0x7e100501,
            0xbefe03c1,
            0x7e040208,
            0xd7650002,
            193 | 258 << 9 | 128 << 18,
            0x7e120502,
            0x7e040209,
        ]),
    )
    .unwrap();
    assert!(result.msl.contains("0x00000002u >> 0u"));
    assert!(result.msl.contains("1u << (0u)"));
}
#[test]
fn lane_selection_and_empty_exec_contract_is_explicit() {
    for mask in [0, 1, 0x80000001, u32::MAX] {
        assert!(
            compile(
                Gfx1030,
                &program(&[0xbefe03ff, mask, 0x7e100501, 0xbefe03c1, 0x7e040208])
            )
            .is_ok()
        );
    }
    for mask in [2, 0x80000000] {
        reject(
            &[0xbefe03ff, mask, 0x7e100501, 0xbefe03c1, 0x7e040208],
            "selected lane zero",
        );
    }
    for index in [0, 32, 64, 0xffffffe0] {
        assert!(
            compile(
                Gfx1030,
                &program(&[
                    0xbe8903ff, index, 0xbefe0380, 0xd7600008, 0x02001301, 0xbefe03c1, 0x7e040208
                ])
            )
            .is_ok()
        );
    }
    reject(
        &[0xbe89039f, 0xd7600008, 0x02001301, 0x7e040208],
        "selected lane zero",
    );
    reject(
        &[0xd7600008, 0x02001301, 0x7e040208],
        "scalar source has no known value",
    );
}
#[test]
fn scalar_overwrites_and_unsupported_consumers_do_not_reuse_old_state() {
    let result = compile(Gfx1030, &program(&[0x7e100501, 0xbe880389, 0x7e040208])).unwrap();
    assert!(result.ir.contains("const.u32 0x00000009"));
    reject(
        &[0xbe880380, 0x7e100501, 0xbe890308, 0x7e040209],
        "scalar source has no known value",
    );
    reject(
        &[0x7e100501, 0xbefe0308, 0x7e040208],
        "scalar source has no known value",
    );
    reject(
        &[0x7e100501, 0xd7600008, 0x02001101, 0x7e040208],
        "scalar source has no known value",
    );
    // Scalar move source/destination alias: a constant lane index is read before SGPR replacement.
    assert!(
        compile(
            Gfx1030,
            &program(&[0xbe880380, 0xd7600008, 0x02001101, 0x7e040208])
        )
        .is_ok()
    );
}
#[test]
fn unready_and_reserved_sources_destinations_reject_even_with_empty_exec() {
    for exec in [0, u32::MAX] {
        for read in [0x7e000501, 0x7e100500, 0x7e100503] {
            assert!(
                compile(
                    Gfx1030,
                    &program(&[0xbefe03ff, exec, read, 0xbefe03c1, 0x7e040208])
                )
                .is_err()
            );
        }
    }
    reject(
        &[0xe0302000, 0x80000100, 0x7e100501, 0xbf8c3f70, 0x7e040208],
        "source register is uninitialized",
    );
}
#[test]
fn runtime_regions_and_whole_graphics_stream_reject_lane_reads() {
    for read in [vec![0x7e100501], vec![0xd7600008, 0x02001301]] {
        for branch in [0xbf860000, 0xbf870000] {
            let mut body = vec![0x7d8a0280, branch | read.len() as u32];
            body.extend(&read);
            body.push(0x7e040281);
            reject(&body, "code skipped by a runtime");
        }
        let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
        for skip in [false, true] {
            for vertex in [false, true] {
                let mut words = if skip {
                    vec![0xbf820000 | read.len() as u32]
                } else {
                    vec![]
                };
                words.extend(&read);
                let mut code = bytes(&words);
                code.extend_from_slice(if vertex { vs.as_slice() } else { ps.as_slice() });
                let result = if vertex {
                    compile_graphics(Gfx1030, &code, ps)
                } else {
                    compile_graphics(Gfx1030, vs, &code)
                };
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("lane reads require")
                );
            }
        }
    }
}

#[test]
fn scalar_conditional_overwrite_clears_runtime_value_only_when_taken() {
    for (cmp, replaced) in [(0xbf068080, true), (0xbf068180, false)] {
        let result = compile(
            Gfx1030,
            &program(&[0x7e100501, cmp, 0xbe880589, 0x7e040208]),
        )
        .unwrap();
        assert_eq!(result.ir.contains("const.u32 0x00000009"), replaced);
    }
}
