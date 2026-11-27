// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent expected values for the twelve SOPC 32-bit integer compares of
//! #215 stage D: `s_cmp_{eq,lg,gt,ge,lt,le}_{i32,u32}`.
//!
//! The expectations are written below from the public AMD "RDNA 2" Instruction
//! Set Architecture Reference Guide only: section 12.4 "SOPC Instructions" for
//! the SCC of each form, section 13.1.4 for the SOPC word, section 5.3 (compare
//! operations set SCC = 1 when true) and the scalar operand table for inline
//! integer constants. Nothing here comes from the ISA crate, the semantic
//! registry, the decoder or the shader compiler. Every case runs on the
//! reference interpreter `xenora_ref` and on the second interpreter
//! `independent_ref`; the ignored test also runs the translated MSL on native
//! Metal.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use std::collections::BTreeMap;

use harness::{Case, words};
use xenora_shader::{Architecture, compile};

/// The twelve forms with their ENC_SOPC opcodes (guide 12.4, Table 70).
#[derive(Clone, Copy, Debug)]
enum Form {
    EqI32 = 0,
    LgI32 = 1,
    GtI32 = 2,
    GeI32 = 3,
    LtI32 = 4,
    LeI32 = 5,
    EqU32 = 6,
    LgU32 = 7,
    GtU32 = 8,
    GeU32 = 9,
    LtU32 = 10,
    LeU32 = 11,
}

const FORMS: [(Form, &str); 12] = [
    (Form::EqI32, "s_cmp_eq_i32"),
    (Form::LgI32, "s_cmp_lg_i32"),
    (Form::GtI32, "s_cmp_gt_i32"),
    (Form::GeI32, "s_cmp_ge_i32"),
    (Form::LtI32, "s_cmp_lt_i32"),
    (Form::LeI32, "s_cmp_le_i32"),
    (Form::EqU32, "s_cmp_eq_u32"),
    (Form::LgU32, "s_cmp_lg_u32"),
    (Form::GtU32, "s_cmp_gt_u32"),
    (Form::GeU32, "s_cmp_ge_u32"),
    (Form::LtU32, "s_cmp_lt_u32"),
    (Form::LeU32, "s_cmp_le_u32"),
];

/// The guide's SCC for `form` with S0 = `s0` (SSRC0, the left operand) and
/// S1 = `s1` (SSRC1). The compares write no SGPR and read no SCC.
fn expect(form: Form, s0: u32, s1: u32) -> bool {
    let (i0, i1) = (s0 as i32, s1 as i32);
    match form {
        // 0 S_CMP_EQ_I32 and 6 S_CMP_EQ_U32 ("identical opcodes"): SCC = (S0 == S1).
        Form::EqI32 | Form::EqU32 => s0 == s1,
        // 1 S_CMP_LG_I32 and 7 S_CMP_LG_U32 ("identical opcodes"): SCC = (S0 != S1).
        Form::LgI32 | Form::LgU32 => s0 != s1,
        // 2 S_CMP_GT_I32: SCC = (S0.i > S1.i).
        Form::GtI32 => i0 > i1,
        // 3 S_CMP_GE_I32: SCC = (S0.i >= S1.i).
        Form::GeI32 => i0 >= i1,
        // 4 S_CMP_LT_I32: SCC = (S0.i < S1.i).
        Form::LtI32 => i0 < i1,
        // 5 S_CMP_LE_I32: SCC = (S0.i <= S1.i).
        Form::LeI32 => i0 <= i1,
        // 8 S_CMP_GT_U32: SCC = (S0.u > S1.u).
        Form::GtU32 => s0 > s1,
        // 9 S_CMP_GE_U32: SCC = (S0.u >= S1.u).
        Form::GeU32 => s0 >= s1,
        // 10 S_CMP_LT_U32: SCC = (S0.u < S1.u).
        Form::LtU32 => s0 < s1,
        // 11 S_CMP_LE_U32: SCC = (S0.u <= S1.u).
        Form::LeU32 => s0 <= s1,
    }
}

/// Guide scalar operand table: 128 is 0, 129-192 are 1 to 64 and 193-208 are
/// -1 to -16, as 32-bit words.
fn inline_integer(selector: u32) -> u32 {
    match selector {
        128..=192 => selector - 128,
        193..=208 => (-((selector - 192) as i32)) as u32,
        _ => unreachable!("not an integer inline constant"),
    }
}

const LITERAL: u32 = 255;
/// Signed and unsigned boundaries and their neighbours.
const VALUES: [u32; 8] = [
    0,
    1,
    0x7fff_fffe,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0xffff_fffe,
    0xffff_ffff,
];
/// The value s10 holds throughout; the output shows it unchanged.
const MARKER: u32 = 0x0f0f_1234;
const INPUT: [u32; 2] = [0, u32::MAX];

const COUNTS: [(&str, usize); 5] = [
    ("sopc domain", 12 * 8 * 8 * 2),
    ("sopc inline integer", 12 * 81 * 3 * 2),
    ("sopc literal", 12 * 5 * 3 * 2),
    ("sopc aliasing", 12 * 3),
    ("sopc no sgpr write", 12 * 5),
];
/// Per form: two literals, 5 special registers in
/// each slot.
const REJECTIONS: usize = 12 * (1 + 5 * 2);

/// ENCODING 0b10_1111110 in bits 23-31, OP 16-22, SSRC1 8-15, SSRC0 0-7
/// (guide 13.1.4).
fn sopc(op: u32, src0: u32, src1: u32) -> u32 {
    0xbf00_0000 | op << 16 | src1 << 8 | src0
}

/// The program around `alu`: load v1, `s_mov_b32` each `init` pair and
/// s10 = MARKER, set SCC to `scc` with `s_and_b32 s12, x, x` (x = 1 or 0, a
/// SOP2 form, so no compare is involved), run `alu`, then v2 = s<result>,
/// `s_cbranch_scc0` over `v_xor_b32 v2, v1, v2`, and store v2. With the lane
/// input [0, !0] the output is [R, R] when SCC is clear (branch taken) and
/// [R, !R] when it is set (fall through).
fn program(init: &[(u32, u32)], scc: bool, alu: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    for &(reg, value) in init.iter().chain(&[(10, MARKER)]) {
        code.extend([0xbe80_03ff | reg << 16, value]);
    }
    let x = if scc { 129 } else { 128 };
    code.push(0x8000_0000 | 14 << 23 | 12 << 16 | x << 8 | x);
    code.extend(alu);
    code.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

fn case(
    name: &'static str,
    family: &'static str,
    label: String,
    code: Vec<u8>,
    out: (u32, bool),
) -> Case {
    let (r, scc) = out;
    Case {
        form: name,
        family,
        label: format!("{name} {label}"),
        code,
        input: INPUT.to_vec(),
        expected: vec![r, if scc { !r } else { r }],
    }
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        for a in VALUES {
            for b in VALUES {
                for scc in [false, true] {
                    let code = program(&[(8, a), (9, b)], scc, &[sopc(op, 8, 9)], 10);
                    let label = format!("{a:#x}, {b:#x}, scc before {scc}");
                    let out = (MARKER, expect(form, a, b));
                    cases.push(case(name, "sopc domain", label, code, out));
                }
            }
        }
        // Every integer inline constant (-16..64) in each slot against a
        // register that is negative as i32 and large as u32, then 32. SCC
        // before is the opposite of the result, so a missing write shows.
        for selector in 128..=208 {
            let value = inline_integer(selector);
            for other in [0xffff_fff8, 0x20, value] {
                for slot in 0..2 {
                    let (alu, scc) = if slot == 0 {
                        (sopc(op, selector, 8), expect(form, value, other))
                    } else {
                        (sopc(op, 8, selector), expect(form, other, value))
                    };
                    let code = program(&[(8, other)], !scc, &[alu], 10);
                    let label = format!("inline {value:#x} in src{slot}, s8 {other:#x}");
                    cases.push(case(
                        name,
                        "sopc inline integer",
                        label,
                        code,
                        (MARKER, scc),
                    ));
                }
            }
        }
        // A literal dword after the SOPC word in each slot.
        for value in [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX] {
            for other in [0x7fff_ffff, 0x8000_0000, value] {
                for slot in 0..2 {
                    let (alu, scc) = if slot == 0 {
                        ([sopc(op, LITERAL, 8), value], expect(form, value, other))
                    } else {
                        ([sopc(op, 8, LITERAL), value], expect(form, other, value))
                    };
                    let code = program(&[(8, other)], !scc, &alu, 10);
                    let label = format!("literal {value:#x} in src{slot}, s8 {other:#x}");
                    cases.push(case(name, "sopc literal", label, code, (MARKER, scc)));
                }
            }
        }
        // One register in both slots: S0 == S1.
        for value in [0, 0x8000_0000, u32::MAX] {
            let scc = expect(form, value, value);
            let code = program(&[(8, value)], !scc, &[sopc(op, 8, 8)], 10);
            let label = format!("s8, s8 = {value:#x}");
            cases.push(case(name, "sopc aliasing", label, code, (MARKER, scc)));
        }
        // No SGPR is written: the sources, the marker and s11 (which a SOP2
        // reading of the word would name as SDST for opcode 11) keep their
        // values.
        let (a, b) = (0x8000_0000, 1);
        let scc = expect(form, a, b);
        let init = [(8, a), (9, b), (11, 0x1111_1111)];
        // s12 holds the SCC-setting AND result, 1 when SCC was set before.
        let s12 = u32::from(!scc);
        for (reg, value) in [(8, a), (9, b), (10, MARKER), (11, 0x1111_1111), (12, s12)] {
            let code = program(&init, !scc, &[sopc(op, 8, 9)], reg);
            let label = format!("s{reg} after s8 {a:#x}, s9 {b:#x}");
            cases.push(case(name, "sopc no sgpr write", label, code, (value, scc)));
        }
    }
    cases
}

/// The SOPC word's byte offset in `program` with `init` SGPR moves: three
/// header words, one 8-byte move per init pair and the marker, and the AND.
fn alu_offset(init: usize) -> usize {
    12 + 8 * (init + 1) + 4
}

/// Both interpreters must refuse `code` at the SOPC word, not later.
fn assert_rejected_at_alu(label: &str, code: &[u8], init: usize) {
    let offset = alu_offset(init);
    match xenora_ref::execute(code, &INPUT) {
        Ok(_) => panic!("reference accepted {label}"),
        Err(error) => assert_eq!(error.offset, offset, "reference rejected {label} elsewhere"),
    }
    match independent_ref::execute(code, &INPUT) {
        Ok(_) => panic!("independent accepted {label}"),
        Err(error) => assert!(
            error.starts_with(&format!("{offset:#x}:")),
            "independent rejected {label} elsewhere: {error}"
        ),
    }
}

/// Programs both interpreters must refuse at the SOPC word, with the number
/// of init pairs before it.
fn rejections() -> Vec<(String, Vec<u8>, usize)> {
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        let code = program(&[], false, &[sopc(op, LITERAL, LITERAL), 1], 10);
        all.push((format!("{name} two literals"), code, 0));
        // VCC_LO, VCC_HI, M0, EXEC_LO and EXEC_HI remain excluded.
        for selector in [106, 107, 124, 126, 127] {
            for slot in 0..2 {
                let alu = if slot == 0 {
                    sopc(op, selector, 8)
                } else {
                    sopc(op, 8, selector)
                };
                let code = program(&[(8, 1)], false, &[alu], 10);
                all.push((format!("{name} source {selector} in src{slot}"), code, 1));
            }
        }
    }
    all
}

fn tally(cases: &[Case], key: fn(&Case) -> &'static str) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for case in cases {
        *counts.entry(key(case)).or_default() += 1;
    }
    counts
}

/// Runs `execute` on every case and returns one line per failing family: the
/// number of failures and the first one.
fn failures(
    cases: &[Case],
    execute: impl Fn(&[u8], &[u32]) -> Result<Vec<u32>, String>,
) -> Vec<String> {
    let mut failed: BTreeMap<&str, (usize, String)> = BTreeMap::new();
    for case in cases {
        let output = execute(&case.code, &case.input);
        if output.as_deref().ok() != Some(case.expected.as_slice()) {
            let first = format!("{}: {output:?}, expected {:?}", case.label, case.expected);
            failed.entry(case.family).or_insert((0, first)).0 += 1;
        }
    }
    failed
        .into_iter()
        .map(|(family, (n, first))| format!("  {family}: {n} failed, first: {first}"))
        .collect()
}

fn checked_cases() -> Vec<Case> {
    let cases = cases();
    assert_eq!(tally(&cases, |case| case.family), BTreeMap::from(COUNTS));
    cases
}

fn reference(code: &[u8], input: &[u32]) -> Result<Vec<u32>, String> {
    xenora_ref::execute(code, input).map_err(|error| format!("{error:?}"))
}

#[test]
fn stage_d_compares_match_the_guide_on_the_reference() {
    let cases = checked_cases();
    let failed = failures(&cases, reference);
    assert!(
        failed.is_empty(),
        "reference disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("sopc_stage_d: {} reference cases", cases.len());
    for (form, n) in tally(&cases, |case| case.form) {
        eprintln!("  form {form}: {n}");
    }
}

#[test]
fn stage_d_compares_match_the_guide_on_the_independent_interpreter() {
    let cases = checked_cases();
    let failed = failures(&cases, independent_ref::execute);
    assert!(
        failed.is_empty(),
        "independent interpreter disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!(
        "sopc_stage_d: {} independent-interpreter cases",
        cases.len()
    );
}

#[test]
fn unsupported_compare_operands_are_rejected() {
    let rejections = rejections();
    assert_eq!(rejections.len(), REJECTIONS);
    for (label, code, init) in &rejections {
        assert_rejected_at_alu(label, code, *init);
    }
    eprintln!("sopc_stage_d: {} rejections", rejections.len());
}

#[test]
fn neighbouring_sopc_opcodes_stay_unsupported() {
    // Bit tests12/13 are covered by scalar_conditional. Their B64 neighbors14/15,
    // 64-bit compares18/19 and remaining unimplemented encodings stay rejected.
    for op in 14..=127 {
        let code = program(&[(8, 1), (9, 2)], false, &[sopc(op, 8, 9)], 10);
        assert_rejected_at_alu(&format!("opcode {op}"), &code, 2);
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn stage_d_compares_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    for (label, code, _) in rejections() {
        assert!(
            compile(Architecture::Gfx1030, &code).is_err(),
            "compiler accepted {label}"
        );
    }
    let cases = checked_cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "sopc_stage_d: {} Metal cases in {programs} programs",
        cases.len()
    );
}
