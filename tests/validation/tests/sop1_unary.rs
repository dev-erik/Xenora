// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent expected values for eleven 32-bit SOP1 unary forms (#215):
//! `s_not_b32`, `s_brev_b32`, `s_bcnt0_i32_b32`, `s_bcnt1_i32_b32`,
//! `s_ff0_i32_b32`, `s_ff1_i32_b32`, `s_flbit_i32_b32`, `s_flbit_i32`,
//! `s_sext_i32_i8`, `s_sext_i32_i16` and `s_abs_i32`.
//!
//! The expectations are written below from the public AMD "RDNA 2" Instruction
//! Set Architecture Reference Guide only: section 12.3 "SOP1 Instructions"
//! (pseudocode and functional examples), Tables 12 and 14 in section 5 (which
//! forms set SCC), section 13.1.3 for the SOP1 word and the scalar operand
//! table for inline integer constants. Nothing here comes from the ISA crate,
//! the semantic registry, the decoder or the shader compiler. Every case runs
//! on the reference interpreter `xenora_ref` and on the second interpreter
//! `independent_ref`; the ignored test also runs the translated MSL on native
//! Metal.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use std::collections::BTreeMap;

use harness::{Case, words};
use xenora_shader::{Architecture, CompileError, compile};

/// The eleven forms with their ENC_SOP1 opcodes (guide 12.3).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    Not = 7,
    Brev = 11,
    Bcnt0 = 13,
    Bcnt1 = 15,
    Ff0 = 17,
    Ff1 = 19,
    FlbitB32 = 21,
    FlbitI32 = 23,
    SextI8 = 25,
    SextI16 = 26,
    Abs = 52,
}

const FORMS: [(Form, &str); 11] = [
    (Form::Not, "s_not_b32"),
    (Form::Brev, "s_brev_b32"),
    (Form::Bcnt0, "s_bcnt0_i32_b32"),
    (Form::Bcnt1, "s_bcnt1_i32_b32"),
    (Form::Ff0, "s_ff0_i32_b32"),
    (Form::Ff1, "s_ff1_i32_b32"),
    (Form::FlbitB32, "s_flbit_i32_b32"),
    (Form::FlbitI32, "s_flbit_i32"),
    (Form::SextI8, "s_sext_i32_i8"),
    (Form::SextI16, "s_sext_i32_i16"),
    (Form::Abs, "s_abs_i32"),
];

fn bit(value: u32, i: u32) -> bool {
    (value >> i) & 1 == 1
}

/// The guide's D for `form` with S0 = `s0`, and the SCC after it: `Some` for
/// the forms that write SCC, `None` for those that leave it unchanged. No form
/// reads SCC.
fn expect(form: Form, s0: u32) -> (u32, Option<bool>) {
    // Tables 12 and 14: NOT, BCNT0, BCNT1 and ABS set SCC ("SCC = (D != 0)",
    // "SCC=result not zero"); BREV, FF0, FF1, FLBIT and SEXT have "n".
    let not_zero = |d: u32| (d, Some(d != 0));
    match form {
        // 7 S_NOT_B32: D = ~S0; SCC = (D != 0).
        Form::Not => not_zero(!s0),
        // 11 S_BREV_B32: D.u[31:0] = S0.u[0:31].
        Form::Brev => (
            (0..32).fold(0, |d, i| d | u32::from(bit(s0, 31 - i)) << i),
            None,
        ),
        // 13 S_BCNT0_I32_B32: D = 0; for i in 0..31: D += (S0[i] == 0 ? 1 : 0);
        // SCC = (D != 0).
        Form::Bcnt0 => not_zero((0..32).filter(|&i| !bit(s0, i)).count() as u32),
        // 15 S_BCNT1_I32_B32: D += (S0[i] == 1 ? 1 : 0); SCC = (D != 0).
        Form::Bcnt1 => not_zero((0..32).filter(|&i| bit(s0, i)).count() as u32),
        // 17 S_FF0_I32_B32: D.i = -1; for i in 0..31 (from the LSB): if
        // S0[i] == 0 then D.i = i; break.
        Form::Ff0 => ((0..32).find(|&i| !bit(s0, i)).unwrap_or(u32::MAX), None),
        // 19 S_FF1_I32_B32: D.i = -1; for i in 0..31: if S0[i] == 1 then
        // D.i = i; break.
        Form::Ff1 => ((0..32).find(|&i| bit(s0, i)).unwrap_or(u32::MAX), None),
        // 21 S_FLBIT_I32_B32: D.i = -1; for i in 0..31: if S0[31 - i] == 1
        // then D.i = i; break. (Zeros before the first one from the MSB.)
        Form::FlbitB32 => ((0..32).find(|&i| bit(s0, 31 - i)).unwrap_or(u32::MAX), None),
        // 23 S_FLBIT_I32: D.i = -1; for i in 1..31: if S0[31 - i] != S0[31]
        // then D.i = i; break. Table 14: -1 if the input is 0 or -1.
        Form::FlbitI32 => (
            (1..32)
                .find(|&i| bit(s0, 31 - i) != bit(s0, 31))
                .unwrap_or(u32::MAX),
            None,
        ),
        // 25 S_SEXT_I32_I8: D.i = signext(S0.i[7:0]), i.e. { 24{S0[7]}, S0[7:0] }.
        Form::SextI8 => (
            if bit(s0, 7) {
                s0 | 0xffff_ff00
            } else {
                s0 & 0xff
            },
            None,
        ),
        // 26 S_SEXT_I32_I16: D.i = signext(S0.i[15:0]), i.e. { 16{S0[15]}, S0[15:0] }.
        Form::SextI16 => (
            if bit(s0, 15) {
                s0 | 0xffff_0000
            } else {
                s0 & 0xffff
            },
            None,
        ),
        // 52 S_ABS_I32: D.i = (S.i < 0 ? -S.i : S.i); SCC = (D.i != 0). The
        // guide's example keeps 0x80000000 ("Note this is negative!").
        Form::Abs => not_zero(if bit(s0, 31) {
            (!s0).wrapping_add(1)
        } else {
            s0
        }),
    }
}

/// The functional examples printed in guide 12.3, with D only; the final five
/// are derived from the pseudocode, as their comment says.
const GUIDE_EXAMPLES: [(Form, u32, u32); 39] = [
    (Form::Bcnt0, 0x0000_0000, 32),
    (Form::Bcnt0, 0xcccc_cccc, 16),
    (Form::Bcnt0, 0xffff_ffff, 0),
    (Form::Bcnt1, 0x0000_0000, 0),
    (Form::Bcnt1, 0xcccc_cccc, 16),
    (Form::Bcnt1, 0xffff_ffff, 32),
    (Form::Ff0, 0xaaaa_aaaa, 0),
    (Form::Ff0, 0x5555_5555, 1),
    (Form::Ff0, 0x0000_0000, 0),
    (Form::Ff0, 0xffff_ffff, 0xffff_ffff),
    (Form::Ff0, 0xfffe_ffff, 16),
    (Form::Ff1, 0xaaaa_aaaa, 1),
    (Form::Ff1, 0x5555_5555, 0),
    (Form::Ff1, 0x0000_0000, 0xffff_ffff),
    (Form::Ff1, 0xffff_ffff, 0),
    (Form::Ff1, 0x0001_0000, 16),
    (Form::FlbitB32, 0x0000_0000, 0xffff_ffff),
    (Form::FlbitB32, 0x0000_cccc, 16),
    (Form::FlbitB32, 0xffff_3333, 0),
    (Form::FlbitB32, 0x7fff_ffff, 1),
    (Form::FlbitB32, 0x8000_0000, 0),
    (Form::FlbitB32, 0xffff_ffff, 0),
    (Form::FlbitI32, 0x0000_0000, 0xffff_ffff),
    (Form::FlbitI32, 0x0000_cccc, 16),
    (Form::FlbitI32, 0xffff_3333, 16),
    (Form::FlbitI32, 0x7fff_ffff, 1),
    (Form::FlbitI32, 0x8000_0000, 1),
    (Form::FlbitI32, 0xffff_ffff, 0xffff_ffff),
    (Form::Abs, 0x0000_0001, 0x0000_0001),
    (Form::Abs, 0x7fff_ffff, 0x7fff_ffff),
    (Form::Abs, 0x8000_0000, 0x8000_0000),
    (Form::Abs, 0x8000_0001, 0x7fff_ffff),
    (Form::Abs, 0x8000_0002, 0x7fff_fffe),
    (Form::Abs, 0xffff_ffff, 0x0000_0001),
    // Not printed as examples; direct readings of the one-line pseudocode.
    (Form::Not, 0x0f0f_0000, 0xf0f0_ffff),
    (Form::Brev, 0x0000_0001, 0x8000_0000),
    (Form::Brev, 0x1234_5678, 0x1e6a_2c48),
    (Form::SextI8, 0x1234_5680, 0xffff_ff80),
    (Form::SextI16, 0x1234_7fff, 0x0000_7fff),
];

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
/// Boundaries, alternating patterns, the guide's example inputs and values
/// whose low 8 or 16 bits have the sign bit set (or just clear).
const VALUES: [u32; 24] = [
    0,
    1,
    2,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0xffff_fffe,
    0xffff_ffff,
    0xaaaa_aaaa,
    0x5555_5555,
    0xcccc_cccc,
    0x0000_cccc,
    0xffff_3333,
    0xfffe_ffff,
    0x0001_0000,
    0x0000_007f,
    0x0000_0080,
    0x0000_00ff,
    0x0000_7fff,
    0x0000_8000,
    0x0000_ffff,
    0xffff_ff7f,
    0x1234_8080,
    0x1234_5678,
];
const INPUT: [u32; 2] = [0, u32::MAX];

const COUNTS: [(&str, usize); 7] = [
    ("sop1 domain", 11 * 24 * 2),
    ("sop1 single bit", 11 * 64 * 2),
    ("sop1 guide example", 39 * 2),
    ("sop1 inline integer", 11 * 81 * 2),
    ("sop1 literal", 11 * 6 * 2),
    ("sop1 aliasing", 11 * 3 * 2 * 2),
    ("sop1 max register", 11 * 3 * 2),
];
/// Per form: 6 destinations and 6 sources outside the supported subset.
const REJECTIONS: usize = 11 * (6 + 6);

/// ENCODING 0b10_1111101 in bits 23-31, SDST 16-22, OP 8-15, SSRC0 0-7
/// (guide 13.1.3).
fn sop1(op: u32, dst: u32, src0: u32) -> u32 {
    0xbe80_0000 | dst << 16 | op << 8 | src0
}

/// The program around `alu`: load v1, `s_mov_b32` each `init` pair, set SCC to
/// `scc` with `s_and_b32 s12, x, x` (x = 1 or 0), run `alu`, then
/// v2 = s<result>, `s_cbranch_scc0` over `v_xor_b32 v2, v1, v2`, and store v2.
/// With the lane input [0, !0] the output is [R, R] when SCC is clear after
/// `alu` and [R, !R] when it is set.
fn program(init: &[(u32, u32)], scc: bool, alu: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    for &(reg, value) in init {
        code.extend([0xbe80_03ff | reg << 16, value]);
    }
    let x = if scc { 129 } else { 128 };
    code.push(0x8000_0000 | 14 << 23 | 12 << 16 | x << 8 | x);
    code.extend(alu);
    code.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

/// The case for `form` with S0 = `s0` and SCC = `scc` before it, reading back
/// `result`, whose value afterwards is `r` (D unless another register is read).
#[allow(clippy::too_many_arguments)]
fn case(
    (form, name): (Form, &'static str),
    family: &'static str,
    label: String,
    code: Vec<u8>,
    s0: u32,
    scc: bool,
    r: Option<u32>,
) -> Case {
    let (d, written) = expect(form, s0);
    let r = r.unwrap_or(d);
    let label = format!("{name} {label}, scc before {scc}");
    let scc = written.unwrap_or(scc);
    Case {
        form: name,
        family,
        label,
        code,
        input: INPUT.to_vec(),
        expected: vec![r, if scc { !r } else { r }],
    }
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        for scc in [false, true] {
            let single = (0..32).flat_map(|k| [1 << k, !(1 << k)]);
            let values = VALUES.iter().map(|&v| (v, "sop1 domain"));
            let examples = GUIDE_EXAMPLES
                .iter()
                .filter(|example| example.0 == form)
                .map(|example| (example.1, "sop1 guide example"));
            let all = values
                .chain(single.map(|v| (v, "sop1 single bit")))
                .chain(examples);
            for (value, family) in all {
                let code = program(&[(8, value)], scc, &[sop1(op, 10, 8)], 10);
                let label = format!("{value:#x}");
                cases.push(case((form, name), family, label, code, value, scc, None));
            }
            for selector in 128..=208 {
                let value = inline_integer(selector);
                let code = program(&[], scc, &[sop1(op, 10, selector)], 10);
                let label = format!("inline {value:#x}");
                cases.push(case(
                    (form, name),
                    "sop1 inline integer",
                    label,
                    code,
                    value,
                    scc,
                    None,
                ));
            }
            for value in [0, 1, 0x8000_0000, u32::MAX, 0x1234_8080, 0x0001_0000] {
                let code = program(&[], scc, &[sop1(op, 10, LITERAL), value], 10);
                let label = format!("literal {value:#x}");
                cases.push(case(
                    (form, name),
                    "sop1 literal",
                    label,
                    code,
                    value,
                    scc,
                    None,
                ));
            }
            // s8 = op(s8), and s10 = op(s8) reading s8 back unchanged.
            for value in [0x8000_0000, 0x0000_80ff, u32::MAX] {
                let code = program(&[(8, value)], scc, &[sop1(op, 8, 8)], 8);
                let label = format!("s8 = op s8 ({value:#x})");
                cases.push(case(
                    (form, name),
                    "sop1 aliasing",
                    label,
                    code,
                    value,
                    scc,
                    None,
                ));
                let code = program(&[(8, value)], scc, &[sop1(op, 10, 8)], 8);
                let label = format!("s8 kept after s10 = op s8 ({value:#x})");
                let kept = Some(value);
                cases.push(case(
                    (form, name),
                    "sop1 aliasing",
                    label,
                    code,
                    value,
                    scc,
                    kept,
                ));
            }
            // The highest SGPR, s105 (0b1101001), as source, destination or both.
            let value = 0x0000_8001;
            for (dst, src, init) in [(105, 105, 105), (10, 105, 105), (105, 8, 8)] {
                let code = program(&[(init, value)], scc, &[sop1(op, dst, src)], dst);
                let label = format!("s{dst} = op s{src} ({value:#x})");
                cases.push(case(
                    (form, name),
                    "sop1 max register",
                    label,
                    code,
                    value,
                    scc,
                    None,
                ));
            }
        }
    }
    cases
}

/// The SOP1 word's byte offset in `program` with one init pair.
const ALU_OFFSET: usize = 12 + 8 + 4;

/// Programs both interpreters must refuse at the SOP1 word.
fn rejections() -> Vec<(String, Vec<u8>)> {
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        // VCC_LO, VCC_HI, M0, NULL, EXEC_LO, EXEC_HI as SDST.
        for dst in [106, 107, 124, 125, 126, 127] {
            let code = program(&[(8, 1)], false, &[sop1(op, dst, 8)], 10);
            all.push((format!("{name} destination {dst}"), code));
        }
        // VCC_LO, VCC_HI, M0, EXEC_LO, EXEC_HI and SCC remain excluded.
        for src in [106, 107, 124, 126, 127, 253] {
            let code = program(&[(8, 1)], false, &[sop1(op, 10, src)], 10);
            all.push((format!("{name} source {src}"), code));
        }
    }
    all
}

fn assert_rejected_at_alu(label: &str, code: &[u8]) {
    match xenora_ref::execute(code, &INPUT) {
        Ok(_) => panic!("reference accepted {label}"),
        Err(error) => assert_eq!(
            error.offset, ALU_OFFSET,
            "reference rejected {label} elsewhere"
        ),
    }
    match independent_ref::execute(code, &INPUT) {
        Ok(_) => panic!("independent accepted {label}"),
        Err(error) => assert!(
            error.starts_with(&format!("{ALU_OFFSET:#x}:")),
            "independent rejected {label} elsewhere: {error}"
        ),
    }
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

#[test]
fn expectations_reproduce_the_guide_examples() {
    for (form, s0, d) in GUIDE_EXAMPLES {
        assert_eq!(expect(form, s0).0, d, "{form:?}({s0:#x})");
    }
}

#[test]
fn sop1_unary_forms_match_the_guide_on_the_reference() {
    let cases = checked_cases();
    let failed = failures(&cases, |code, input| {
        xenora_ref::execute(code, input).map_err(|error| format!("{error:?}"))
    });
    assert!(
        failed.is_empty(),
        "reference disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("sop1_unary: {} reference cases", cases.len());
    for (form, n) in tally(&cases, |case| case.form) {
        eprintln!("  form {form}: {n}");
    }
}

#[test]
fn sop1_unary_forms_match_the_guide_on_the_independent_interpreter() {
    let cases = checked_cases();
    let failed = failures(&cases, independent_ref::execute);
    assert!(
        failed.is_empty(),
        "independent interpreter disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("sop1_unary: {} independent-interpreter cases", cases.len());
}

#[test]
fn unsupported_sop1_operands_are_rejected() {
    let rejections = rejections();
    assert_eq!(rejections.len(), REJECTIONS);
    for (label, code) in &rejections {
        assert_rejected_at_alu(label, code);
    }
    eprintln!("sop1_unary: {} rejections", rejections.len());
}

#[test]
fn other_sop1_opcodes_stay_unsupported() {
    // Exclude MOV, conditional MOV and bit updates, covered by scalar_conditional,
    // as well as the eleven unary forms.
    let supported: Vec<u32> = FORMS
        .iter()
        .map(|&(form, _)| form as u32)
        .chain([3, 5, 27, 29])
        .collect();
    for op in (0..=255).filter(|op| !supported.contains(op)) {
        let code = program(&[(8, 1)], false, &[sop1(op, 10, 8)], 10);
        assert_rejected_at_alu(&format!("opcode {op}"), &code);
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn sop1_unary_forms_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    for (label, code) in rejections() {
        // A decode error at the SOP1 word, so a later failure cannot stand in.
        match compile(Architecture::Gfx1030, &code) {
            Err(CompileError::Decode(error)) => assert_eq!(
                error.offset, ALU_OFFSET,
                "compiler rejected {label} elsewhere: {error:?}"
            ),
            other => panic!("compiler did not reject {label} at decode: {other:?}"),
        }
    }
    let cases = checked_cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "sop1_unary: {} Metal cases in {programs} programs",
        cases.len()
    );
}
