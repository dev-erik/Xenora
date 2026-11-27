// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent literal SCC1 programs, AMD RDNA2 section 12.5.
//! https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture
//! LLVM SOPInstructions.td: gfx10 SOPP SCC0=4, SCC1=5 (not gfx11's opcode22).
//! Target = instruction byte offset + 4 + sign_extend(SIMM16)*4.
//! Backedges are architectural; rejection below is the bounded compute contract.
#[path = "../independent_ref.rs"]
mod independent_ref;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn program(body: &[u32]) -> Vec<u8> {
    let mut words = body.to_vec();
    // Restore EXEC, expose scalar s8, store under the existing synthetic ABI.
    words.extend([
        0xbefe03c1, 0x7e020208, 0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000,
    ]);
    bytes(&words)
}

fn check(code: &[u8], expected: u32) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, &[0; 32]).unwrap(), [expected; 32]);
    assert_eq!(
        independent_ref::execute(code, &[0; 32]).unwrap(),
        [expected; 32]
    );
}

fn reject_target(code: &[u8], at: usize) {
    match xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err() {
        xenora_shader::CompileError::Verify { offset, .. } => assert_eq!(offset, at),
        xenora_shader::CompileError::Decode(error) => assert_eq!(error.offset, at),
        other => panic!("wrong target rejection: {other:?}"),
    }
    assert_eq!(xenora_ref::execute(code, &[0]).unwrap_err().offset, at);
    let error = independent_ref::execute(code, &[0]).unwrap_err();
    assert!(error.starts_with(&format!("{at:#x}:")), "{error}");
}

fn compare(flag: bool) -> u32 {
    // s_cmp_eq_u32 0,0 or 1,0; fixed raw words, no production encoder.
    if flag { 0xbf068080 } else { 0xbf068081 }
}

fn positives() -> Vec<(Vec<u8>, u32)> {
    let mut cases = Vec::new();
    for flag in [false, true] {
        for exec in [0x80, 0x81, 0xc1] {
            for (branch, taken) in [(0xbf850001, flag), (0xbf840001, !flag)] {
                cases.push((
                    program(&[
                        0xbe880387,
                        compare(flag),
                        0xbefe0300 | exec,
                        branch,
                        0xbe880389,
                    ]),
                    if taken { 7 } else { 9 },
                ));
            }
        }
        // +2 skips the complete eight-byte scalar literal move, not two instructions.
        cases.push((
            program(&[
                0xbe880387,
                compare(flag),
                0xbf850002,
                0xbe8803ff,
                0x12345678,
            ]),
            if flag { 7 } else { 0x12345678 },
        ));
        // Zero displacement means the next instruction, not a self-loop.
        cases.push((
            program(&[0xbe880387, compare(flag), 0xbf850000, 0xbe880389]),
            9,
        ));
        // Consecutive complementary branches observe the same unchanged SCC.
        cases.push((
            program(&[
                0xbe880387,
                compare(flag),
                0xbf850001,
                0xbe880389,
                0xbf840001,
                0xbe88038b,
            ]),
            if flag { 11 } else { 9 },
        ));
    }
    cases
}

#[test]
fn complementary_conditions_exec_independence_literal_skip_and_preserved_scc() {
    for (code, expected) in positives() {
        check(&code, expected);
    }
}

#[test]
fn existing_bitwise_fixture_complements_scc0_without_changing_its_target() {
    let mut code = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin").to_vec();
    assert_eq!(&code[20..24], &0xbf840002u32.to_le_bytes());
    code[22] = 0x85;
    let inputs = [0, 1, 0xffffffff, 0x80000000];
    for (comparison, expected) in [
        (6u8, [0; 4]),
        (7u8, [0x12345678, 0x12345679, 0x12cb5687, 0x12345678]),
    ] {
        code[17] = 128 + comparison;
        xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(xenora_ref::execute(&code, &inputs).unwrap(), expected);
        assert_eq!(independent_ref::execute(&code, &inputs).unwrap(), expected);
    }
}

#[test]
fn unknown_scc_is_not_invented_even_with_exec_zero_or_zero_displacement() {
    for displacement in [0, 1] {
        let code = program(&[
            0xbe880387,
            0xbefe0380,
            0xbf850000 | displacement,
            0xbe880389,
        ]);
        match xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap_err() {
            xenora_shader::CompileError::Verify { offset, reason } => {
                assert_eq!(offset, 8);
                assert!(reason.contains("SCC"), "{reason}");
            }
            other => panic!("expected undefined SCC verification, got {other:?}"),
        }
        let error = xenora_ref::execute(&code, &[0]).unwrap_err();
        assert_eq!(error.offset, 8);
        assert!(error.detail.to_ascii_lowercase().contains("scc"));
        let error = independent_ref::execute(&code, &[0]).unwrap_err();
        assert!(error.starts_with("0x8:"), "{error}");
        assert!(error.to_ascii_lowercase().contains("scc"), "{error}");
    }
}

#[test]
fn malformed_targets_reject_even_when_scc1_is_not_taken() {
    for flag in [false, true] {
        // Branch at8: +1 targets the literal at16; +3 targets the store's
        // second word at24. +7fff is out of range; ffff is self, fffe backward.
        for displacement in [1, 3, 0x7fff, 0xffff, 0xfffe, 0x8000] {
            let code = bytes(&[
                0xbe880387,
                compare(flag),
                0xbf850000 | displacement,
                0xbe8803ff,
                0x12345678,
                0xe0702000,
                0x80010100,
                0xbbfd0000,
                0xbf810000,
            ]);
            reject_target(&code, 8);
        }
        // Target equals byte length: there is no instruction at EOF.
        let code = bytes(&[compare(flag), 0xbf850001, 0xbf810000]);
        reject_target(&code, 4);
    }
}

#[test]
#[ignore = "requires coordinated native Metal execution with validation"]
fn native_scc1_literal_results() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, expected) in positives() {
        check(&code, expected);
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[0; 32])
                .unwrap(),
            [expected; 32]
        );
    }
}

#[test]
fn reference_decodes_exact_scc1_identity_and_next_pc_relative_target() {
    use xenora_ref::{BranchCondition, Op};
    let code = bytes(&[0xbe880387, 0xbf850001, 0xbe880389, 0xbf810000]);
    let decoded = xenora_ref::decode(&code).unwrap();
    assert_eq!(
        decoded[1].op,
        Op::Branch {
            target: 12,
            condition: BranchCondition::SccOne
        }
    );
}

#[test]
fn both_reference_consumers_execute_independent_scc1_answers() {
    // Can run before production lowering lands; neither reference calls it.
    for (code, expected) in positives() {
        assert_eq!(
            xenora_ref::execute(&code, &[0; 32]).unwrap(),
            [expected; 32]
        );
        assert_eq!(
            independent_ref::execute(&code, &[0; 32]).unwrap(),
            [expected; 32]
        );
    }
}
