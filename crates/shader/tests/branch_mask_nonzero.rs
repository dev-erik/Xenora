// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU compilation/admission checks; arithmetic readback belongs to independent validation.
use xenora_shader::{Architecture::Gfx1030, CompileError, compile, compile_graphics};

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn program(body: &[u32]) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70, 0x7e040287];
    words.extend_from_slice(body);
    words.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
fn mov_scalar(dst: u32, value: u32) -> [u32; 2] {
    [0xbe8003ff | dst << 16, value]
}
fn runtime(opcode: u32, region: &[u32], exec: u32, after: &[u32]) -> Vec<u8> {
    // Compare while EXEC is full so VCC remains lane-derived even after EXEC becomes zero.
    let mut body = vec![0x7d8a0280]; // v_cmp_ne_u32 vcc_lo, 0, v1
    body.extend(mov_scalar(126, exec));
    body.push(opcode | region.len() as u32);
    body.extend_from_slice(region);
    body.push(0xbefe03c1); // restore EXEC
    body.extend_from_slice(after);
    program(&body)
}
fn reject(code: &[u8], expected: &str) {
    let error = compile(Gfx1030, code).unwrap_err();
    assert!(
        error.to_string().contains(expected),
        "expected {expected}: {error}"
    );
}

#[test]
fn known_low_masks_select_complementary_paths_independent_of_high_halves() {
    for (z, nz, register) in [(0xbf880000, 0xbf890000, 126), (0xbf860000, 0xbf870000, 106)] {
        for low in [0, 1, 0x80000000, u32::MAX] {
            for high in [0, u32::MAX] {
                for (opcode, taken) in [(z, low == 0), (nz, low != 0)] {
                    let mut body = mov_scalar(register, low).to_vec();
                    body.extend(mov_scalar(register + 1, high));
                    body.push(opcode | 1);
                    body.push(0xbe880389); // harmless scalar write, also executes with empty EXEC
                    body.push(0xbefe03c1);
                    let compiled = compile(Gfx1030, &program(&body)).unwrap();
                    assert!(
                        compiled.ir.contains(&format!("taken={taken}")),
                        "{}",
                        compiled.ir
                    );
                }
            }
        }
    }
}

#[test]
fn vccnz_still_reads_vcc_with_empty_exec_and_empty_compare_clears_it() {
    reject(
        &program(&[0xbefe0380, 0xbf870000, 0xbefe03c1]),
        "uninitialized VCC_LO",
    );
    let compiled = compile(
        Gfx1030,
        &program(&[
            0xbeea03c1, // VCC_LO = -1
            0xbefe0380, // EXEC_LO = 0
            0x7d8a0280, // compare clears VCC, without source reads
            0xbf870001, 0xbe880389, 0xbefe03c1,
        ]),
    )
    .unwrap();
    assert!(compiled.ir.contains("taken=false"));
    assert!(!compiled.ir.contains("branch.vccnz"));
}

#[test]
fn runtime_join_inverts_only_its_predicate_and_recomputes_in_comparison_cone() {
    for exec in [0, 1, 0x80000002, u32::MAX] {
        let z = compile(
            Gfx1030,
            &runtime(0xbf860000, &[0x7e040301], exec, &[0x7d840480]),
        )
        .unwrap();
        let nz = compile(
            Gfx1030,
            &runtime(0xbf870000, &[0x7e040301], exec, &[0x7d840480]),
        )
        .unwrap();
        assert_eq!(z.ir, nz.ir.replace("vccnz", "vccz"));
        if exec != 0 {
            assert!(nz.ir.contains("join.vccnz"));
            // Both the thread expression and the expression recomputed per lane use NZ polarity.
            assert_eq!(nz.msl.matches(" == 0u ? ").count(), 2, "{}", nz.msl);
            assert_eq!(z.msl, nz.msl.replace(" == 0u ? ", " != 0u ? "));
            if exec != u32::MAX {
                assert!(nz.ir.contains("merge.lanes"));
            }
        } else {
            assert!(!nz.ir.contains("join.vccnz"));
            assert_eq!(z.msl, nz.msl);
        }
        assert!(nz.msl.contains("if (lane_index >= grid) { break; }"));
    }
}

#[test]
fn runtime_region_whitelist_does_not_expand_for_nz() {
    let forbidden: &[&[u32]] = &[
        &[0xbe880389],                             // scalar write
        &[0x7d8a0280],                             // compare
        &[0xbf820000],                             // nested unconditional branch
        &[0xbf870000],                             // nested VCCNZ
        &[0xbf890000],                             // nested EXECNZ
        &[0xbf8c3f70],                             // wait
        &[0xe0302000, 0x80000300],                 // load
        &[0xe0702000, 0x80010200],                 // store
        &[0x06040501],                             // floating add
        &[0xd7650002, 193 | 128 << 9 | 128 << 18], // MBCNT
    ];
    for exec in [0, 1, u32::MAX] {
        for region in forbidden {
            reject(
                &runtime(0xbf870000, region, exec, &[]),
                "code skipped by a runtime s_cbranch_vccnz",
            );
        }
        reject(
            &runtime(0xbf870000, &[0x7e060289], exec, &[]),
            "must be initialized before the branch",
        );
        reject(
            &runtime(0xbf870000, &[0x36040481], exec, &[]),
            "SRC0 must be a VGPR",
        );
    }
    reject(
        &runtime(0xbf870000, &[0x7e040303], u32::MAX, &[]),
        "source register is uninitialized",
    );
}

#[test]
fn runtime_branch_requires_completed_load_but_allows_prior_pending_store() {
    reject(
        &program(&[0x7d8a0280, 0xe0302000, 0x80000300, 0xbf870001, 0x7e040289]),
        "requires no outstanding vector load",
    );
    let code = bytes(&[
        0xe0302000, 0x80000100, 0xbf8c3f70, 0x7e040287, 0x7d8a0280, 0xe0702000, 0x80010200,
        0xbf870001, 0x7e040289, 0xbbfd0000, 0xbf810000,
    ]);
    assert!(compile(Gfx1030, &code).is_ok());
}

#[test]
fn known_vcc_write_restores_static_branch_resolution() {
    let compiled = compile(
        Gfx1030,
        &program(&[0x7d8a0280, 0xbeea0381, 0xbf870001, 0xbe880389]),
    )
    .unwrap();
    assert!(compiled.ir.contains("taken=true"));
    assert!(!compiled.ir.contains("branch.vccnz"));
}

#[test]
fn forward_boundaries_are_checked_even_on_skipped_paths() {
    for opcode in [0xbf870000, 0xbf890000] {
        for displacement in [0xffff, 0xfffe] {
            reject(
                &program(&[0xbeea0380, opcode | displacement]),
                "forward instruction boundary",
            );
            reject(
                &program(&[0xbf820001, opcode | displacement]),
                "forward instruction boundary",
            );
        }
        // Target the literal payload or the second word of an eight-byte store.
        assert!(compile(Gfx1030, &program(&[opcode | 1, 0x7e0402ff, 9])).is_err());
        assert!(compile(Gfx1030, &program(&[opcode | 1])).is_err());
        assert!(compile(Gfx1030, &program(&[opcode | 4])).is_err()); // EOF
        // Zero displacement is the next instruction and is allowed.
        assert!(compile(Gfx1030, &program(&[0xbeea0381, opcode])).is_ok());
    }
}

#[test]
fn both_stages_reject_nz_even_when_unreachable() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for opcode in [0xbf870000, 0xbf890000] {
        for skip in [false, true] {
            for vertex in [false, true] {
                let mut code = if skip {
                    bytes(&[0xbf820001])
                } else {
                    Vec::new()
                };
                code.extend(bytes(&[opcode]));
                code.extend_from_slice(if vertex { vs.as_slice() } else { ps.as_slice() });
                let error = if vertex {
                    compile_graphics(Gfx1030, &code, ps)
                } else {
                    compile_graphics(Gfx1030, vs, &code)
                }
                .unwrap_err();
                assert!(
                    error.to_string().contains("Wave32 compute branch contract"),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn decoding_exposes_typed_targets_without_relaxing_compute_cycles() {
    use xenora_decoder::{Instruction, decode_at};
    for (opcode, expected) in [
        (0xbf870000, Instruction::BranchVccNonZero { target: 4 }),
        (0xbf890000, Instruction::BranchExecNonZero { target: 4 }),
    ] {
        let decoded = decode_at(Gfx1030, &bytes(&[opcode]), 0).unwrap();
        assert_eq!(decoded.instruction, expected);
        assert_eq!(decoded.branch_target(), Some(4));
        assert_eq!(decoded.byte_len, 4);
        let error = compile(Gfx1030, &program(&[opcode | 0xffff])).unwrap_err();
        assert!(matches!(error, CompileError::Verify { .. }));
    }
}
