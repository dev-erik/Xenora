// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent expected values for sixteen 32-bit SOP2 integer forms (#215):
//! `s_min_i32`, `s_min_u32`, `s_max_i32`, `s_max_u32`, `s_cselect_b32`,
//! `s_bfm_b32`, `s_mul_i32`, `s_bfe_u32`, `s_bfe_i32`, `s_absdiff_i32`,
//! `s_lshl1_add_u32` to `s_lshl4_add_u32`, `s_mul_hi_u32` and `s_mul_hi_i32`.
//!
//! The expectations are written below from the public AMD "RDNA 2" Instruction
//! Set Architecture Reference Guide only: section 12.1 "SOP2 Instructions"
//! (pseudocode and the ABSDIFF functional examples), Tables 11, 12 and 14 in
//! section 5 (which forms set SCC, and that CSELECT reads it), section 13.1.1
//! for the SOP2 word and the scalar operand table for inline integer
//! constants. Nothing here comes from the ISA crate, the semantic registry, the
//! decoder or the shader compiler. Every case runs on the reference interpreter
//! `xenora_ref` and on the second interpreter `independent_ref`; the ignored
//! test also runs the translated MSL on native Metal.
//!
//! Reading of the pseudocode where the guide leaves C-like operators open:
//! arithmetic is exact (unbounded integers, as the guide's own
//! `>= 0x100000000ULL` carry tests require) and D keeps the low 32 bits. So a
//! BFE width of 32 or more gives an all-ones mask, and LSHLn_ADD's SCC is the
//! carry out of the whole `(S0 << N) + S1`, including bits shifted out of S0.
//! These are guide readings, not hardware evidence.

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use std::collections::BTreeMap;

use harness::{Case, words};
use xenora_shader::{Architecture, CompileError, compile};

/// The sixteen forms with their ENC_SOP2 opcodes (guide 12.1).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    MinI32 = 6,
    MinU32 = 7,
    MaxI32 = 8,
    MaxU32 = 9,
    Cselect = 10,
    Bfm = 36,
    Mul = 38,
    BfeU32 = 39,
    BfeI32 = 40,
    Absdiff = 44,
    Lshl1Add = 46,
    Lshl2Add = 47,
    Lshl3Add = 48,
    Lshl4Add = 49,
    MulHiU32 = 53,
    MulHiI32 = 54,
}

const FORMS: [(Form, &str); 16] = [
    (Form::MinI32, "s_min_i32"),
    (Form::MinU32, "s_min_u32"),
    (Form::MaxI32, "s_max_i32"),
    (Form::MaxU32, "s_max_u32"),
    (Form::Cselect, "s_cselect_b32"),
    (Form::Bfm, "s_bfm_b32"),
    (Form::Mul, "s_mul_i32"),
    (Form::BfeU32, "s_bfe_u32"),
    (Form::BfeI32, "s_bfe_i32"),
    (Form::Absdiff, "s_absdiff_i32"),
    (Form::Lshl1Add, "s_lshl1_add_u32"),
    (Form::Lshl2Add, "s_lshl2_add_u32"),
    (Form::Lshl3Add, "s_lshl3_add_u32"),
    (Form::Lshl4Add, "s_lshl4_add_u32"),
    (Form::MulHiU32, "s_mul_hi_u32"),
    (Form::MulHiI32, "s_mul_hi_i32"),
];

/// BFE's S1 fields (guide 12.1): "S1[4:0] is field offset, S1[22:16] is field
/// width", so the width is seven bits (0-127) and bits 5-15 and 23-31 are
/// ignored.
fn field(s1: u32) -> (u32, u32) {
    (s1 & 0x1f, (s1 >> 16) & 0x7f)
}

/// `(1 << width) - 1` in exact arithmetic.
fn mask(width: u32) -> i128 {
    ((1u128 << width) - 1) as i128
}

/// The guide's D for `form` with S0 = `s0`, S1 = `s1` and SCC = `scc` before
/// it, and the SCC after it: `Some` for the forms that write SCC, `None` for
/// those that leave it unchanged. Only CSELECT reads `scc`.
fn expect(form: Form, s0: u32, s1: u32, scc: bool) -> (u32, Option<bool>) {
    let (i0, i1) = (i128::from(s0 as i32), i128::from(s1 as i32));
    let (u0, u1) = (i128::from(s0), i128::from(s1));
    // D keeps the low 32 bits of the exact result.
    let low = |exact: i128| exact as u32;
    let not_zero = |d: u32| (d, Some(d != 0));
    match form {
        // 6 S_MIN_I32: D.i = (S0.i < S1.i) ? S0.i : S1.i; SCC = (S0.i < S1.i).
        // Equal operands give S1 (the same value) and SCC = 0; Table 11's
        // "SCC = 1 if S0 was min" agrees only if a tie does not count as S0.
        Form::MinI32 => (if i0 < i1 { s0 } else { s1 }, Some(i0 < i1)),
        // 7 S_MIN_U32: D.u = (S0.u < S1.u) ? S0.u : S1.u; SCC = (S0.u < S1.u).
        Form::MinU32 => (if u0 < u1 { s0 } else { s1 }, Some(u0 < u1)),
        // 8 S_MAX_I32: D.i = (S0.i > S1.i) ? S0.i : S1.i; SCC = (S0.i > S1.i).
        Form::MaxI32 => (if i0 > i1 { s0 } else { s1 }, Some(i0 > i1)),
        // 9 S_MAX_U32: D.u = (S0.u > S1.u) ? S0.u : S1.u; SCC = (S0.u > S1.u).
        Form::MaxU32 => (if u0 > u1 { s0 } else { s1 }, Some(u0 > u1)),
        // 10 S_CSELECT_B32: D.u = SCC ? S0.u : S1.u. Table 12: "Sets SCC? n".
        Form::Cselect => (if scc { s0 } else { s1 }, None),
        // 36 S_BFM_B32: D.u = ((1 << S0.u[4:0]) - 1) << S1.u[4:0]. Table 14:
        // "Sets SCC? n".
        Form::Bfm => (low(mask(s0 & 0x1f) << (s1 & 0x1f)), None),
        // 38 S_MUL_I32: D.i = S0.i * S1.i; Table 11: "Low 32 bits of result",
        // "Sets SCC? n".
        Form::Mul => (low(i0 * i1), None),
        // 39 S_BFE_U32: D.u = (S0.u >> S1.u[4:0]) & ((1 << S1.u[22:16]) - 1);
        // SCC = (D.u != 0). Width 0 gives 0; bits past bit 31 of S0 are 0.
        Form::BfeU32 => {
            let (offset, width) = field(s1);
            not_zero(low((u0 >> offset) & mask(width)))
        }
        // 40 S_BFE_I32: D.i = signext((S0.i >> S1.u[4:0]) & ((1 << S1.u[22:16])
        // - 1)); SCC = (D.i != 0). Section 5.7: "Bit Field Extract, then
        // sign-extend result", i.e. from the field's top bit, width - 1. S0.i
        // shifts arithmetically, so a field past bit 31 is filled with S0[31].
        // Width 0 gives 0.
        Form::BfeI32 => {
            let (offset, width) = field(s1);
            let bits = (i0 >> offset) & mask(width);
            let d = if width > 0 && (bits >> (width - 1)) & 1 == 1 {
                bits.wrapping_sub(1i128.wrapping_shl(width))
            } else {
                bits
            };
            not_zero(low(d))
        }
        // 44 S_ABSDIFF_I32: D.i = S0.i - S1.i; if (D.i < 0) D.i = -D.i;
        // SCC = (D.i != 0). D.i is 32-bit: the functional examples give
        // (0x80000000, 1) => 0x7fffffff and keep 0x80000000 for
        // (0x80000000, 0), so the difference wraps before the negation.
        Form::Absdiff => {
            let d = (s0 as i32).wrapping_sub(s1 as i32);
            not_zero(if d < 0 { d.wrapping_neg() } else { d } as u32)
        }
        // 46-49 S_LSHLn_ADD_U32: D.u = (S0.u << N) + S1.u; SCC = (((S0.u << N)
        // + S1.u) >= 0x100000000ULL ? 1 : 0), N = 1, 2, 3, 4.
        Form::Lshl1Add | Form::Lshl2Add | Form::Lshl3Add | Form::Lshl4Add => {
            let n = form as u32 - 45;
            let exact = (u0 << n) + u1;
            (low(exact), Some(exact >= 1 << 32))
        }
        // 53 S_MUL_HI_U32: D.u = (S0.u * S1.u) >> 32. No SCC line.
        Form::MulHiU32 => (low((u0 * u1) >> 32), None),
        // 54 S_MUL_HI_I32: D.i = (S0.i * S1.i) >> 32 (signed). No SCC line.
        Form::MulHiI32 => (low((i0 * i1) >> 32), None),
    }
}

/// The ABSDIFF functional examples printed in guide 12.1, with D only, and
/// direct readings of other one-line pseudocode.
const GUIDE_EXAMPLES: [(Form, u32, u32, u32); 18] = [
    (Form::Absdiff, 0x0000_0002, 0x0000_0005, 0x0000_0003),
    (Form::Absdiff, 0xffff_ffff, 0x0000_0000, 0x0000_0001),
    (Form::Absdiff, 0x8000_0000, 0x0000_0000, 0x8000_0000),
    (Form::Absdiff, 0x8000_0000, 0x0000_0001, 0x7fff_ffff),
    (Form::Absdiff, 0x8000_0000, 0xffff_ffff, 0x7fff_ffff),
    (Form::Absdiff, 0x8000_0000, 0xffff_fffe, 0x7fff_fffe),
    // Not printed as examples.
    (Form::Bfm, 4, 8, 0x0000_0f00),
    (Form::Bfm, 31, 1, 0xffff_fffe),
    (Form::Bfm, 32, 0, 0),
    (Form::BfeU32, 0x1234_5678, 0x0008_0004, 0x0000_0067),
    (Form::BfeU32, 0x8000_0000, 0x0004_001f, 0x0000_0001),
    (Form::BfeI32, 0x0000_0080, 0x0008_0000, 0xffff_ff80),
    (Form::BfeI32, 0x8000_0000, 0x0004_001f, 0xffff_ffff),
    (Form::BfeI32, 0x1234_5678, 0x0040_0000, 0x1234_5678),
    (Form::MulHiU32, 0xffff_ffff, 0xffff_ffff, 0xffff_fffe),
    (Form::MulHiI32, 0xffff_ffff, 0xffff_ffff, 0x0000_0000),
    (Form::MulHiI32, 0x8000_0000, 0x8000_0000, 0x4000_0000),
    (Form::Lshl4Add, 0x1000_0000, 0x0000_0001, 0x0000_0001),
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
/// Boundaries, their neighbours and alternating patterns; all pairs include
/// the equal pairs that are MIN/MAX ties.
const VALUES: [u32; 13] = [
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
    0x1234_5678,
    0x0000_ffff,
    0xffff_0000,
];
/// BFE widths around 0, 32 and the seven-bit field, and offsets around 0, 31
/// and the five-bit field.
const WIDTHS: [u32; 10] = [0, 1, 2, 8, 16, 31, 32, 33, 64, 127];
const OFFSETS: [u32; 8] = [0, 1, 4, 16, 28, 31, 32, 63];
/// S1 words with bit 23 (just above the width) or every ignored bit set.
const BFE_EXTRA: [u32; 2] = [0x0084_0004, 0xff80_ffe4];
/// BFM sizes and offsets around the five-bit fields.
const BFM_FIELDS: [u32; 9] = [0, 1, 4, 16, 31, 32, 33, 63, 0xffff_ffe1];
const INPUT: [u32; 2] = [0, u32::MAX];

const COUNTS: [(&str, usize); 7] = [
    ("sop2 arith domain", 16 * 13 * 13 * 2),
    ("sop2 arith bit field", (2 * 13 * (10 * 8 + 2) + 9 * 9) * 2),
    ("sop2 arith guide example", 18 * 2),
    ("sop2 arith aliasing", 16 * 2 * 5 * 2),
    ("sop2 arith inline integer", 16 * 81 * 2 * 2 * 2),
    ("sop2 arith literal", 16 * 6 * 2 * 2),
    ("sop2 arith max register", 16 * 4 * 2),
];
/// Per form: two literals, 6 destinations and 15 sources in each slot outside
/// the supported subset; and CSELECT with SCC never written.
const REJECTIONS: usize = 16 * (1 + 6 + 2 * 15) + 1 + 4;

/// ENCODING 0b10 in bits 30-31, OP 23-29, SDST 16-22, SSRC1 8-15, SSRC0 0-7
/// (guide 13.1.1).
fn sop2(op: u32, dst: u32, src0: u32, src1: u32) -> u32 {
    0x8000_0000 | op << 23 | dst << 16 | src1 << 8 | src0
}

/// The program around `alu`: load v1, `s_mov_b32` each `init` pair, set SCC to
/// `scc` with `s_cmp_eq_u32` (or leave it unwritten for `None`), run `alu`, then
/// v2 = s<result>, XORed with v1 when SCC is set afterwards, and store v2. With
/// the lane input [0, !0] the output is [R, R] when SCC is clear after `alu`
/// and [R, !R] when it is set.
fn program(init: &[(u32, u32)], scc: Option<bool>, alu: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    for &(reg, value) in init {
        code.extend([0xbe80_03ff | reg << 16, value]);
    }
    match scc {
        Some(true) => code.push(0xbf06_8080),
        Some(false) => code.push(0xbf06_8180),
        None => {}
    }
    code.extend(alu);
    code.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

/// The case for `form` on `s0`, `s1` with SCC = `scc` before it, reading back
/// `result`, whose value afterwards is `r` (D unless another register is read).
#[allow(clippy::too_many_arguments)]
fn case(
    (form, name): (Form, &'static str),
    family: &'static str,
    label: String,
    code: Vec<u8>,
    (s0, s1): (u32, u32),
    scc: bool,
    r: Option<u32>,
) -> Case {
    let (d, written) = expect(form, s0, s1, scc);
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

fn pairs(form: Form) -> Vec<(u32, u32, &'static str)> {
    let mut all: Vec<_> = VALUES
        .iter()
        .flat_map(|&a| VALUES.map(|b| (a, b, "sop2 arith domain")))
        .collect();
    let family = "sop2 arith bit field";
    if matches!(form, Form::BfeU32 | Form::BfeI32) {
        let fields = WIDTHS
            .iter()
            .flat_map(|&w| OFFSETS.map(|o| w << 16 | o))
            .chain(BFE_EXTRA);
        let fields: Vec<_> = fields.collect();
        all.extend(
            VALUES
                .iter()
                .flat_map(|&a| fields.iter().map(move |&b| (a, b, family))),
        );
    }
    if form == Form::Bfm {
        all.extend(
            BFM_FIELDS
                .iter()
                .flat_map(|&a| BFM_FIELDS.map(|b| (a, b, family))),
        );
    }
    let examples = GUIDE_EXAMPLES.iter().filter(|example| example.0 == form);
    all.extend(examples.map(|example| (example.1, example.2, "sop2 arith guide example")));
    all
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        for scc in [false, true] {
            let alu = [sop2(op, 10, 8, 9)];
            for (a, b, family) in pairs(form) {
                let code = program(&[(8, a), (9, b)], Some(scc), &alu, 10);
                let label = format!("{a:#x}, {b:#x}");
                cases.push(case((form, name), family, label, code, (a, b), scc, None));
            }
            // dst/src aliasing with s8 = a, s9 = b.
            for (a, b) in [(0xffff_fff0, 0x0008_0004), (0x8000_0001, 0x8000_0001)] {
                for (dst, src0, src1) in [(8, 8, 9), (9, 8, 9), (8, 8, 8), (9, 9, 9), (8, 9, 8)] {
                    let pick = |reg| if reg == 8 { a } else { b };
                    let alu = [sop2(op, dst, src0, src1)];
                    let code = program(&[(8, a), (9, b)], Some(scc), &alu, dst);
                    let label = format!("s{dst} = s{src0}, s{src1} ({a:#x}, {b:#x})");
                    let operands = (pick(src0), pick(src1));
                    let family = "sop2 arith aliasing";
                    cases.push(case((form, name), family, label, code, operands, scc, None));
                }
            }
            // Every integer inline constant (-16..64) in each source slot. The
            // register operand s8 takes two words chosen so a wrong inline value
            // changes D for at least one of them; as a BFE S1 they are fields
            // (width 8, offset 0) and (width 32, offset 1).
            let bfe = matches!(form, Form::BfeU32 | Form::BfeI32);
            for selector in 128..=208 {
                let value = inline_integer(selector);
                for slot in 0..2 {
                    let others = if slot == 0 && bfe {
                        [0x0008_0000, 0x0020_0001]
                    } else {
                        [0x8000_0f0f, 0x7fff_f0f0]
                    };
                    for other in others {
                        let (alu, operands) = if slot == 0 {
                            (sop2(op, 10, selector, 8), (value, other))
                        } else {
                            (sop2(op, 10, 8, selector), (other, value))
                        };
                        let code = program(&[(8, other)], Some(scc), &[alu], 10);
                        let label = format!("inline {value:#x} in src{slot}, s8 {other:#x}");
                        let family = "sop2 arith inline integer";
                        cases.push(case((form, name), family, label, code, operands, scc, None));
                    }
                }
            }
            // The highest SGPR, s105 (0b1101001), as destination, either
            // source or all three; s8 and s105 differ in sign and as BFE fields.
            let (a, b, top) = (0x0004_0001, 0x0003_0002, 0x8000_0006);
            for (dst, src0, src1) in [(105, 8, 9), (10, 105, 8), (10, 8, 105), (105, 105, 105)] {
                let pick = |reg| match reg {
                    8 => a,
                    9 => b,
                    _ => top,
                };
                let alu = [sop2(op, dst, src0, src1)];
                let code = program(&[(8, a), (9, b), (105, top)], Some(scc), &alu, dst);
                let label = format!("s{dst} = s{src0}, s{src1}");
                let operands = (pick(src0), pick(src1));
                let family = "sop2 arith max register";
                cases.push(case((form, name), family, label, code, operands, scc, None));
            }
            // A 32-bit literal dword after the SOP2 word in each source slot.
            for value in [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX, 0x0021_0003] {
                for slot in 0..2 {
                    let other = 0xc005_0003;
                    let (alu, operands) = if slot == 0 {
                        ([sop2(op, 10, LITERAL, 8), value], (value, other))
                    } else {
                        ([sop2(op, 10, 8, LITERAL), value], (other, value))
                    };
                    let code = program(&[(8, other)], Some(scc), &alu, 10);
                    let label = format!("literal {value:#x} in src{slot}");
                    let family = "sop2 arith literal";
                    cases.push(case((form, name), family, label, code, operands, scc, None));
                }
            }
        }
    }
    cases
}

/// The SOP2 word's byte offset in `program` with two init pairs and the SCC
/// preset.
const ALU_OFFSET: usize = 12 + 16 + 4;

/// Programs both interpreters must refuse at the SOP2 word, with that offset.
fn rejections() -> Vec<(String, Vec<u8>, usize)> {
    let init = [(8, 1), (9, 2)];
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        // Guide 12.1 (SOP2 note): at most one literal; two are refused.
        let alu = [sop2(op, 10, LITERAL, LITERAL), 1];
        let code = program(&init, Some(false), &alu, 10);
        all.push((format!("{name} two literals"), code, ALU_OFFSET));
        // VCC_LO, VCC_HI, M0, NULL, EXEC_LO, EXEC_HI as SDST.
        for dst in [106, 107, 124, 125, 126, 127] {
            let code = program(&init, Some(false), &[sop2(op, dst, 8, 9)], 10);
            all.push((format!("{name} destination {dst}"), code, ALU_OFFSET));
        }
        // VCC_LO, VCC_HI, M0, EXEC_LO, EXEC_HI, SCC, and the float inline
        // constants 240-248 in either source slot.
        for src in [106, 107, 124, 126, 127, 253].into_iter().chain(240..=248) {
            for (src0, src1, slot) in [(src, 9, 0), (8, src, 1)] {
                let code = program(&init, Some(false), &[sop2(op, 10, src0, src1)], 10);
                all.push((format!("{name} src{slot} {src}"), code, ALU_OFFSET));
            }
        }
    }
    // CSELECT reads SCC; here nothing has written it.
    let code = program(&init, None, &[sop2(Form::Cselect as u32, 10, 8, 9)], 10);
    all.push(("s_cselect_b32 with SCC unset".into(), code, ALU_OFFSET - 4));
    // The forms that leave SCC unchanged must keep it unset, so a CSELECT after
    // them is still refused (at the CSELECT, the second ALU word).
    for form in [Form::Bfm, Form::Mul, Form::MulHiU32, Form::MulHiI32] {
        let alu = [
            sop2(form as u32, 10, 8, 9),
            sop2(Form::Cselect as u32, 11, 8, 9),
        ];
        let code = program(&init, None, &alu, 11);
        all.push((
            format!("{form:?} then s_cselect_b32 with SCC unset"),
            code,
            ALU_OFFSET,
        ));
    }
    all
}

fn assert_rejected_at(label: &str, code: &[u8], offset: usize) {
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
    for (form, s0, s1, d) in GUIDE_EXAMPLES {
        assert_eq!(
            expect(form, s0, s1, false).0,
            d,
            "{form:?}({s0:#x}, {s1:#x})"
        );
    }
}

#[test]
fn sop2_arith_forms_match_the_guide_on_the_reference() {
    let cases = checked_cases();
    let failed = failures(&cases, |code, input| {
        xenora_ref::execute(code, input).map_err(|error| format!("{error:?}"))
    });
    assert!(
        failed.is_empty(),
        "reference disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("sop2_arith: {} reference cases", cases.len());
    for (form, n) in tally(&cases, |case| case.form) {
        eprintln!("  form {form}: {n}");
    }
}

#[test]
fn sop2_arith_forms_match_the_guide_on_the_independent_interpreter() {
    let cases = checked_cases();
    let failed = failures(&cases, independent_ref::execute);
    assert!(
        failed.is_empty(),
        "independent interpreter disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("sop2_arith: {} independent-interpreter cases", cases.len());
}

#[test]
fn unsupported_sop2_arith_operands_are_rejected() {
    let rejections = rejections();
    assert_eq!(rejections.len(), REJECTIONS);
    for (label, code, offset) in &rejections {
        assert_rejected_at(label, code, *offset);
    }
    eprintln!("sop2_arith: {} rejections", rejections.len());
}

#[test]
fn other_sop2_opcodes_stay_unsupported() {
    // Every SOP2 opcode (0-95; 96 and up are other encodings) except the
    // earlier stages' forms (0-5 and the even opcodes 14-34) and these
    // sixteen. Sources and destination are aligned pairs, all initialized, so
    // a 64-bit form would read and write only set registers.
    let earlier = (0..=5).chain((14..=34).step_by(2));
    let supported: Vec<u32> = FORMS
        .iter()
        .map(|&(form, _)| form as u32)
        .chain(earlier)
        .chain(50..=52) // Scalar packing has separate independent coverage.
        .collect();
    let init = [(8, 1), (9, 2), (12, 3), (13, 4)];
    for op in (0..=95).filter(|op| !supported.contains(op)) {
        let code = program(&init, Some(false), &[sop2(op, 10, 8, 12)], 10);
        assert_rejected_at(&format!("opcode {op}"), &code, 12 + 32 + 4);
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn sop2_arith_forms_match_the_guide_on_native_metal() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    for (label, code, offset) in rejections() {
        // Refused at the same word as by the interpreters, so a later failure
        // cannot stand in.
        let at = match compile(Architecture::Gfx1030, &code) {
            Err(CompileError::Decode(error)) => error.offset,
            Err(CompileError::Verify { offset, .. }) => offset,
            other => panic!("compiler did not reject {label}: {other:?}"),
        };
        assert_eq!(at, offset, "compiler rejected {label} elsewhere");
    }
    let cases = checked_cases();
    let (failures, programs) = harness::check_metal(&cases);
    failures.assert_none("native Metal disagrees with the guide");
    eprintln!(
        "sop2_arith: {} Metal cases in {programs} programs",
        cases.len()
    );
}
