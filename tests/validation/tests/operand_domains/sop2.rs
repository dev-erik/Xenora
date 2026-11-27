// SPDX-License-Identifier: GPL-3.0-or-later

//! SOP2 programs: the operands start in SGPRs (or as inline or literal
//! sources), SCC is set or cleared first, and the result reaches the output as
//! v2 = D, XORed with the lane input only when SCC is set afterwards. With the
//! lane input [0, 0xffffffff] the output is [D, D] or [D, !D].

use super::harness::{Case, Rejection, words};
use super::oracle::{self, LITERAL, SOP2_FORMS, Sop2, sop2 as expect};

/// Boundary operands: zero, one, signed max/min and min + 1, unsigned max and
/// max - 1, and an ordinary pattern.
const VALUES: [u32; 8] = [
    0,
    1,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0xffff_fffe,
    u32::MAX,
    0x1234_5678,
];
const LITERALS: [u32; 6] = [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX, 0x1234_5678];
/// 64-bit operands for add/addc and sub/subb chains.
const WIDE: [u64; 6] = [
    0,
    1,
    0xffff_ffff,
    0x1_0000_0000,
    0x8000_0000_0000_0000,
    u64::MAX,
];
const INPUT: [u32; 2] = [0, u32::MAX];

pub const COUNTS: &[(&str, usize)] = &[
    ("sop2 domain", 9 * 8 * 8 * 2 - 8),
    ("sop2 subb full-width borrow", 8),
    ("sop2 carry chain", 2 * 6 * 6 * 2 * 2),
    ("sop2 aliasing", 9 * 5 * 2),
    ("sop2 inline integer", 9 * 81 * 2),
    ("sop2 literal", 9 * 6 * 2 * 2),
    ("sop2 max register", 9 * 4),
];
/// Two literals, 21 special destinations,
/// and a missing literal word in two slots, for each of the 9 forms.
pub const REJECTIONS: usize = 9 * (1 + 21 + 2);
/// The SOP2 word's offset: load 8, wait 4, two literal moves 8 each, s_cmp 4.
const ALU_OFFSET: usize = 0x20;

/// ENCODING 0b10 in bits 30-31, OP 23-29, SDST 16-22, SSRC1 8-15, SSRC0 0-7
/// (guide 13.1.1).
pub fn sop2(op: Sop2, dst: u32, src0: u32, src1: u32) -> u32 {
    0x8000_0000 | (op as u32) << 23 | dst << 16 | src1 << 8 | src0
}

/// `s_mov_b32 s<reg>, literal` (SOP1 opcode 3).
fn mov(reg: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | reg << 16, value]
}

/// The program around `alu`: `init` sets SGPRs, SCC = `scc`, then `alu`, then
/// v2 = s<result>, XORed with v1 when SCC is set.
fn program(init: &[(u32, u32)], scc: bool, alu: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    for &(reg, value) in init {
        code.extend(mov(reg, value));
    }
    // s_cmp_eq_u32 0, 0 sets SCC; s_cmp_eq_u32 0, 1 clears it.
    code.push(if scc { 0xbf06_8080 } else { 0xbf06_8180 });
    code.extend(alu);
    code.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

fn lanes((d, scc): (u32, bool)) -> Vec<u32> {
    vec![d, if scc { !d } else { d }]
}

fn case(
    form: &'static str,
    family: &'static str,
    label: String,
    code: Vec<u8>,
    out: (u32, bool),
) -> Case {
    Case {
        form,
        family,
        label: format!("{form} {label}"),
        code,
        input: INPUT.to_vec(),
        expected: lanes(out),
    }
}

pub fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for (op, name) in SOP2_FORMS {
        let alu = [sop2(op, 10, 8, 9)];
        for a in VALUES {
            for b in VALUES {
                for scc in [false, true] {
                    let family = if op == Sop2::SubbU32 && b == u32::MAX && scc {
                        "sop2 subb full-width borrow"
                    } else {
                        "sop2 domain"
                    };
                    let code = program(&[(8, a), (9, b)], scc, &alu, 10);
                    cases.push(case(
                        name,
                        family,
                        format!("{a:#x}, {b:#x}, scc {scc}"),
                        code,
                        expect(op, a, b, scc),
                    ));
                }
            }
        }
        // dst/src aliasing, SCC set: (dst, src0, src1) with s8 = a, s9 = b.
        for (a, b) in [(0xffff_fff0, 0x11), (0x8000_0000, 0x8000_0000)] {
            for (dst, src0, src1) in [(8, 8, 9), (9, 8, 9), (8, 8, 8), (9, 9, 9), (8, 9, 8)] {
                let pick = |reg| if reg == 8 { a } else { b };
                let code = program(&[(8, a), (9, b)], true, &[sop2(op, dst, src0, src1)], dst);
                let out = expect(op, pick(src0), pick(src1), true);
                cases.push(case(
                    name,
                    "sop2 aliasing",
                    format!("s{dst} = s{src0}, s{src1} ({a:#x}, {b:#x})"),
                    code,
                    out,
                ));
            }
        }
        // Every integer inline constant in each source slot, s8 = 0xfffffff0, SCC set.
        let a = 0xffff_fff0;
        for code_point in (128..=208).filter_map(|c| oracle::inline_integer(c).map(|v| (c, v))) {
            let (selector, value) = code_point;
            for slot in 0..2 {
                let (alu, out) = if slot == 0 {
                    (sop2(op, 10, selector, 8), expect(op, value, a, true))
                } else {
                    (sop2(op, 10, 8, selector), expect(op, a, value, true))
                };
                let code = program(&[(8, a), (9, 0)], true, &[alu], 10);
                cases.push(case(
                    name,
                    "sop2 inline integer",
                    format!("inline {selector} ({value:#x}) slot {slot}"),
                    code,
                    out,
                ));
            }
        }
        // A literal word in each source slot, with SCC clear.
        for a in [1, 0x8000_0000] {
            for literal in LITERALS {
                for slot in 0..2 {
                    let (alu, out) = if slot == 0 {
                        (sop2(op, 10, LITERAL, 8), expect(op, literal, a, false))
                    } else {
                        (sop2(op, 10, 8, LITERAL), expect(op, a, literal, false))
                    };
                    let code = program(&[(8, a), (9, 0)], false, &[alu, literal], 10);
                    cases.push(case(
                        name,
                        "sop2 literal",
                        format!("literal {literal:#x} slot {slot}, s8 {a:#x}"),
                        code,
                        out,
                    ));
                }
            }
        }
        // s105, the highest SGPR a SOP2 operand may name (guide 5.2), as
        // destination and source; s105 = a, s8 = s9 = b, SCC set.
        let (a, b) = (0x8000_0001, u32::MAX);
        for (dst, src0, src1) in [(105, 105, 9), (105, 8, 105), (105, 105, 105), (8, 105, 9)] {
            let pick = |reg| if reg == 105 { a } else { b };
            let code = program(
                &[(105, a), (8, b), (9, b)],
                true,
                &[sop2(op, dst, src0, src1)],
                dst,
            );
            let out = expect(op, pick(src0), pick(src1), true);
            cases.push(case(
                name,
                "sop2 max register",
                format!("s{dst} = s{src0}, s{src1}"),
                code,
                out,
            ));
        }
    }
    cases.extend(chains());
    cases
}

/// 64-bit add and subtract built from the u32 forms: low word with
/// s_add_u32/s_sub_u32, high word with s_addc_u32/s_subb_u32 consuming its
/// SCC. SCC is preset both ways first to show the low form overwrites it. Each
/// chain runs twice, outputting the low word and then the high word, both with
/// the final SCC. The expected words and SCC follow the guide; they are also
/// checked against 64-bit arithmetic here.
fn chains() -> Vec<Case> {
    let mut cases = Vec::new();
    for (low, high, name) in [
        (Sop2::AddU32, Sop2::AddcU32, "s_addc_u32"),
        (Sop2::SubU32, Sop2::SubbU32, "s_subb_u32"),
    ] {
        for x in WIDE {
            for y in WIDE {
                for preset in [false, true] {
                    let (xl, xh, yl, yh) = (x as u32, (x >> 32) as u32, y as u32, (y >> 32) as u32);
                    let (d_low, carry) = expect(low, xl, yl, preset);
                    let (d_high, scc) = expect(high, xh, yh, carry);
                    let exact = if low == Sop2::AddU32 {
                        (x.wrapping_add(y), x.checked_add(y).is_none())
                    } else {
                        (x.wrapping_sub(y), y > x)
                    };
                    let joined = u64::from(d_high) << 32 | u64::from(d_low);
                    assert_eq!((joined, scc), exact, "{x:#x} {y:#x}");
                    let alu = [sop2(low, 12, 8, 9), sop2(high, 13, 10, 11)];
                    let init = [(8, xl), (9, yl), (10, xh), (11, yh)];
                    for (word, d) in [("low", d_low), ("high", d_high)] {
                        let result = if word == "low" { 12 } else { 13 };
                        cases.push(case(
                            name,
                            "sop2 carry chain",
                            format!("{x:#x}, {y:#x}, preset {preset}, {word} word"),
                            program(&init, preset, &alu, result),
                            (d, scc),
                        ));
                    }
                }
            }
        }
    }
    cases
}

pub fn rejections() -> Vec<Rejection> {
    let mut rejections = Vec::new();
    let reject = |family, label: String, alu: &[u32]| Rejection {
        family,
        truncated: false,
        label,
        code: program(&[(8, 1), (9, 2)], false, alu, 10),
        offset: ALU_OFFSET,
    };
    for (op, name) in SOP2_FORMS {
        // Both sources select the literal. The guide allows at most one literal
        // constant per instruction; whether two selectors may share one word is
        // not stated, and Xenora's subset rejects it.
        rejections.push(reject(
            "sop2 two literals",
            format!("{name} two literals"),
            &[sop2(op, 10, LITERAL, LITERAL), 3, 3],
        ));
        // VCC_LO, VCC_HI, TTMP0-15, M0, EXEC_LO, EXEC_HI as destination. The
        // guide lists these as scalar destinations; Xenora's SOP2 subset writes
        // only s0-s105, so these are rejections of the subset, not of the guide.
        for dst in (106..=123).chain([124, 126, 127]) {
            rejections.push(reject(
                "sop2 special destination",
                format!("{name} dst {dst}"),
                &[sop2(op, dst, 8, 9)],
            ));
        }
        // The stream ends where the literal word should be.
        for (slot, alu) in [(0, sop2(op, 10, LITERAL, 8)), (1, sop2(op, 10, 8, LITERAL))] {
            let mut code = program(&[(8, 1), (9, 2)], false, &[], 10);
            code.truncate(ALU_OFFSET);
            code.extend(alu.to_le_bytes());
            rejections.push(Rejection {
                family: "sop2 missing literal",
                truncated: true,
                label: format!("{name} missing literal slot {slot}"),
                code,
                offset: ALU_OFFSET,
            });
        }
    }
    rejections
}
