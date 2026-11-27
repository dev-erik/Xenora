// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU compiler/admission checks; no native execution claims.
use xenora_shader::{Architecture::Gfx1030, compile, compile_graphics};
const OPS: [u32; 4] = [0xe0202000, 0xe0242000, 0xe0282000, 0xe02c2000];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn finish(mut words: Vec<u32>) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
fn reject(words: Vec<u32>, reason: &str) {
    let error = compile(Gfx1030, &finish(words)).unwrap_err();
    assert!(
        error.to_string().contains(reason),
        "expected {reason}: {error}"
    );
}
#[test]
fn transforms_own_input_and_replayed_lane_input_identically() {
    let expressions = [
        "(input[INDEX] & 0xffu)",
        "((input[INDEX] & 0xffu) ^ 0x80u) - 0x80u",
        "(input[INDEX] & 0xffffu)",
        "((input[INDEX] & 0xffffu) ^ 0x8000u) - 0x8000u",
    ];
    for (op, expression) in OPS.into_iter().zip(expressions) {
        for wait in [0xbf8c3f70, 0xbc7d0000] {
            let result = compile(Gfx1030, &finish(vec![op, 0x80000100, wait, 0x7d840280])).unwrap();
            assert!(
                result.msl.contains(&format!(
                    "uint t0 = {};",
                    expression.replace("INDEX", "index")
                )),
                "{}",
                result.msl
            );
            assert!(
                result.msl.contains(&format!(
                    "uint l0 = {};",
                    expression.replace("INDEX", "lane_index")
                )),
                "{}",
                result.msl
            );
            assert!(result.msl.contains("(index & ~31u) + lane"));
            assert!(result.msl.contains("if (lane_index >= grid) { break; }"));
            assert_eq!(result.msl.matches("input[").count(), 2);
            assert!(!result.msl.contains("simd_"));
        }
    }
}
#[test]
fn full_exec_required_including_empty_exec() {
    for op in OPS {
        for exec in [0, 1, 0x80000000, 0xfffffffe] {
            reject(
                vec![0xbefe03ff, exec, op, 0x80000100, 0xbf8c3f70, 0xbefe03c1],
                "vector memory requires full Wave32 EXEC",
            );
        }
    }
}
#[test]
fn exact_resource_and_index_checks_apply_even_to_skipped_loads() {
    for op in OPS {
        for second in [0x80000101, 0x800001ff, 0x80010100, 0x80190100] {
            reject(
                vec![op, second, 0xbf8c3f70],
                "unsupported resource or buffer index",
            );
            reject(
                vec![0xbf820002, op, second, 0x7e020287],
                "unsupported resource or buffer index",
            );
        }
        reject(
            vec![op, 0x80000000, 0xbf8c3f70],
            "load writes the index register",
        );
    }
}
#[test]
fn all_load_kinds_share_the_single_pending_slot() {
    for first in OPS.into_iter().chain([0xe0302000]) {
        for second in OPS.into_iter().chain([0xe0302000]) {
            reject(
                vec![first, 0x80000100, second, 0x80000200, 0xbf8c3f70],
                "another load is outstanding",
            );
            // Completed loads may reuse the destination; conversions stay associated with each read.
            assert!(
                compile(
                    Gfx1030,
                    &finish(vec![
                        first, 0x80000100, 0xbf8c3f70, second, 0x80000100, 0xbc7d0000
                    ])
                )
                .is_ok()
            );
        }
    }
}
#[test]
fn readiness_pending_destination_and_wait_domains_are_unchanged() {
    for op in OPS {
        for use_before_wait in [vec![], vec![0xbbfd0000], vec![0x7d840280]] {
            let mut body = vec![0x7e020287, op, 0x80000100];
            body.extend(use_before_wait);
            reject(body, "source register is uninitialized");
        }
        reject(
            vec![op, 0x80000100, 0x7e020287, 0xbf8c3f70],
            "replaces a pending load",
        );
        reject(
            vec![
                op,
                0x80000100,
                0xd5250001,
                129 | 129 << 9 | 128 << 18,
                0xbf8c3f70,
            ],
            "replaces a pending load",
        );
        // Waiting for loads does not release the final store.
        let code = bytes(&[
            op, 0x80000100, 0xbf8c3f70, 0xe0702000, 0x80010100, 0xbc7d0000, 0xbf810000,
        ]);
        assert!(
            compile(Gfx1030, &code)
                .unwrap_err()
                .to_string()
                .contains("termination requires")
        );
        // A completed store does not release a later pending load at termination.
        let code = bytes(&[
            0x7e020287, 0xe0702000, 0x80010100, 0xbbfd0000, op, 0x80000200, 0xbf810000,
        ]);
        assert!(
            compile(Gfx1030, &code)
                .unwrap_err()
                .to_string()
                .contains("termination requires")
        );
    }
}
#[test]
fn runtime_regions_still_reject_memory_for_both_polarities() {
    for op in OPS {
        for branch in [0xbf860002, 0xbf870002] {
            for exec in [0, u32::MAX] {
                reject(
                    vec![
                        0x7e020287, 0x7d8a0280, 0xbefe03ff, exec, branch, op, 0x80000200,
                        0xbefe03c1,
                    ],
                    "code skipped by a runtime",
                );
            }
        }
    }
}
#[test]
fn graphics_rejects_narrow_loads_in_every_stream_position() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for op in OPS {
        for skip in [false, true] {
            for vertex in [false, true] {
                let mut code = if skip {
                    bytes(&[0xbf820002])
                } else {
                    Vec::new()
                };
                code.extend(bytes(&[op, 0x80000800]));
                code.extend_from_slice(if vertex { vs.as_slice() } else { ps.as_slice() });
                let error = if vertex {
                    compile_graphics(Gfx1030, &code, ps)
                } else {
                    compile_graphics(Gfx1030, vs, &code)
                }
                .unwrap_err();
                assert!(
                    error.to_string().contains("narrow buffer loads require"),
                    "{error}"
                );
            }
        }
    }
}
