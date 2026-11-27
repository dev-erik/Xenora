// SPDX-License-Identifier: GPL-3.0-or-later
//! Original bounded wait tests derived from AMD RDNA2 ISA section 12.2.
//! Native ARM64 LLVM gfx1030 -show-encoding: null,0 = [00,00,7d,bc].
//! No production decoder or evaluator supplies expected words.
#[path = "../independent_ref.rs"]
mod independent_ref;

const VM: u32 = 0xbc7d0000;
const VS: u32 = 0xbbfd0000;
const END: u32 = 0xbf810000;
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const STORE: [u32; 2] = [0xe0702000, 0x80010200];
const NOT: u32 = 0x7e046f01; // v_not_b32 v2,v1; independently LLVM-checked.
const INPUT: [u32; 8] = [
    0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x7fa12345, 0x01234567, 0xffff0000,
];
const OUTPUT: [u32; 8] = [
    0xffffffff, 0xfffffffe, 0, 0x7fffffff, 0x80000000, 0x805edcba, 0xfedcba98, 0x0000ffff,
];

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn valid(repeated: bool) -> Vec<u8> {
    let mut w = Vec::new();
    if repeated {
        w.extend([VM, VM]);
    } // No outstanding load.
    w.extend(LOAD);
    w.push(VM);
    if repeated {
        w.push(VM);
    }
    w.push(NOT);
    w.extend(STORE);
    // A load wait with a pending store must leave that store outstanding.
    if repeated {
        w.extend([VM, VM]);
    }
    w.extend([VS, END]);
    bytes(&w)
}
fn check(code: &[u8]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, &INPUT).unwrap(), OUTPUT);
    assert_eq!(independent_ref::execute(code, &INPUT).unwrap(), OUTPUT);
}
fn rejects(code: &[u8], offset: usize, decode: bool, intent: &str) {
    let compiler = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    assert!(compiler.to_string().contains(intent), "{compiler}");
    match compiler {
        xenora_shader::CompileError::Decode(e) if decode => assert_eq!(e.offset, offset),
        xenora_shader::CompileError::Verify { offset: at, .. } if !decode => assert_eq!(at, offset),
        other => panic!("wrong rejection category: {other:?}"),
    }
    let primary = xenora_ref::execute(code, &INPUT).unwrap_err();
    assert_eq!(primary.offset, offset, "{primary}");
    let secondary = independent_ref::execute(code, &INPUT).unwrap_err();
    assert!(
        secondary.starts_with(&format!("{offset:#x}:")),
        "{secondary}"
    );
}

#[test]
fn exact_encoding_and_repeated_waits_match_literal_answers() {
    assert_eq!(VM.to_le_bytes(), [0, 0, 0x7d, 0xbc]);
    check(&valid(false));
    check(&valid(true));
}

#[test]
fn missing_or_store_wait_does_not_release_load() {
    for waits in [vec![], vec![VS], vec![VS, VS]] {
        let mut words = LOAD.to_vec();
        words.extend(waits);
        let at = words.len() * 4;
        words.push(NOT); // Sole invalidity: v1 has not become readable.
        words.extend(STORE);
        words.extend([VS, END]);
        rejects(&bytes(&words), at, false, "pending a load wait");
    }
}

#[test]
fn load_wait_does_not_complete_store() {
    for waits in [vec![], vec![VM], vec![VM, VM]] {
        let mut words = LOAD.to_vec();
        words.extend([VM, NOT]);
        words.extend(STORE);
        words.extend(waits);
        let at = words.len() * 4;
        words.push(END); // Sole invalidity: output store still pending.
        rejects(&bytes(&words), at, false, "termination requires");
    }
}

#[test]
fn unsupported_registers_and_thresholds_are_decode_errors() {
    // Restriction belongs to this bounded contract, not architectural legality.
    for reg in (0..=127).filter(|&r| r != 125) {
        let mut words = vec![0xbc000000 | reg << 16];
        words.extend(LOAD);
        words.extend([VM, NOT]);
        words.extend(STORE);
        words.extend([VS, END]);
        rejects(&bytes(&words), 0, true, "");
    }
    for imm in [1, 2, 7, 8, 31, 32, 63, 64, 0x7fff, 0x8000, 0xffff] {
        let mut words = vec![VM | imm];
        words.extend(LOAD);
        words.extend([VM, NOT]);
        words.extend(STORE);
        words.extend([VS, END]);
        rejects(&bytes(&words), 0, true, "");
    }
}

#[test]
fn wait_preserves_initialized_and_unset_scc() {
    for (setup, want) in [(0xbf068080, 1), (0xbf068180, 0)] {
        // CMP_EQ followed by wait, CSELECT s8,1,0, and scalar-to-vector move.
        let mut words = LOAD.to_vec();
        words.extend([setup, VM, 0x85088081, 0x7e040208]);
        words.extend(STORE);
        words.extend([VS, END]);
        let code = bytes(&words);
        xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(xenora_ref::execute(&code, &INPUT).unwrap(), [want; 8]);
        assert_eq!(independent_ref::execute(&code, &INPUT).unwrap(), [want; 8]);
    }
    let mut words = LOAD.to_vec();
    words.extend([VM, 0x85088081, 0x7e040208]);
    words.extend(STORE);
    words.extend([VS, END]);
    rejects(&bytes(&words), 12, false, "uninitialized SCC"); // Wait must not initialize SCC.
}

#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_batched_vmcnt_literal_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for repeated in [false, true] {
        let code = valid(repeated);
        check(&code);
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &INPUT)
                .unwrap(),
            OUTPUT
        );
    }
}

#[test]
fn truncated_wait_is_not_accepted() {
    for len in 1..4 {
        let code = &VM.to_le_bytes()[..len];
        assert!(matches!(
            xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code),
            Err(xenora_shader::CompileError::InvalidLength { bytes }) if bytes == len
        ));
        assert_eq!(xenora_ref::execute(code, &INPUT).unwrap_err().offset, 0);
        let error = independent_ref::execute(code, &INPUT).unwrap_err();
        assert!(error.starts_with("0x0:"), "{error}");
    }
}
