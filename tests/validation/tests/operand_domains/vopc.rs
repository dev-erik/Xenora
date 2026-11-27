// SPDX-License-Identifier: GPL-3.0-or-later

//! VOPC programs on one Wave32 wave: v1 = lane input, v3 (or v255) = a literal
//! `b`, an optional EXEC mask around the compare, then every lane outputs
//! VCC_LO. The 32 lane inputs are distinct, so a mask bit landing in the wrong
//! lane changes the output.
//!
//! Not covered: whether a compare also writes EXEC. The supported subset has
//! no instruction that reads EXEC back, so the EXEC cases restore it before
//! the output move.

use super::harness::{Case, Rejection, words};
use super::oracle::{COMPARES, Compare, LITERAL, compare, inline_integer, vcc};

/// Boundary operands, also the first ten lane inputs: zero, one, signed max - 1
/// and max, signed min and min + 1, -16, unsigned max - 1 and max, and a
/// pattern.
const VALUES: [u32; 10] = [
    0,
    1,
    0x7fff_fffe,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0xffff_fff0,
    0xffff_fffe,
    u32::MAX,
    0x0f0f_0f0f,
];
/// The other 22 lane inputs: more values near the boundaries and patterns.
const MORE_LANES: [u32; 22] = [
    2,
    3,
    15,
    16,
    63,
    64,
    65,
    0x7fff_fffd,
    0x8000_0002,
    0xffff_fffd,
    0xffff_ffef,
    0xffff_fff1,
    0x4000_0000,
    0xc000_0000,
    0x0000_ffff,
    0xffff_0000,
    0x1234_5678,
    0x8765_4321,
    0x00ff_00ff,
    0xff00_ff00,
    0x5555_5555,
    0xaaaa_aaaa,
];
const LITERALS: [u32; 5] = [0, 1, 0x7fff_ffff, 0x8000_0000, u32::MAX];
const EXEC_MASKS: [u32; 5] = [0, 0x5555_5555, 0xaaaa_aaaa, 0x8000_0001, 0x0000_ffff];

pub const COUNTS: &[(&str, usize)] = &[
    ("vopc domain", 12 * 10 * 2),
    ("vopc aliasing", 12),
    ("vopc inline integer", 12 * 81),
    ("vopc literal", 12 * 5),
    ("vopc sgpr source", 12 * 2 * 2),
    ("vopc max register", 12 * 2),
    ("vopc exec mask", 12 * 5),
    ("vopc vccz", 12 * 11),
];
/// A missing literal word for each of the 12 forms.
pub const REJECTIONS: usize = 12;
/// The compare's offset: load 8, wait 4, v_mov with literal 8.
const COMPARE_OFFSET: usize = 0x14;

/// ENCODING 0b0111110 in bits 25-31, OP 17-24, VSRC1 9-16, SRC0 0-8
/// (guide 13.3.3); SRC0 256 + n is VGPR n.
fn vopc(opcode: u32, src0: u32, vsrc1: u32) -> u32 {
    0x7c00_0000 | opcode << 17 | vsrc1 << 9 | src0
}

const fn vgpr(n: u32) -> u32 {
    256 + n
}

/// `v_mov_b32 v<reg>, literal` (VOP1 opcode 1).
fn vmov(reg: u32, value: u32) -> [u32; 2] {
    [0x7e00_02ff | reg << 17, value]
}

/// `s_mov_b32 vcc_lo, 0xffffffff`, so a compare that leaves bits untouched
/// shows up.
const SEED_VCC: [u32; 2] = [0xbeea_03ff, u32::MAX];
/// `s_mov_b32 exec_lo, literal` and `s_mov_b32 exec_lo, -1`.
const SET_EXEC: u32 = 0xbefe_03ff;
const FULL_EXEC: u32 = 0xbefe_03c1;
/// After v2 = VCC_LO: `s_cbranch_vccz` over `v_xor_b32 v2, v1, v2`.
const VCCZ_TAIL: [u32; 2] = [0xbf86_0001, 0x3a04_0501];

/// Load v1, v<breg> = b, then `before`, the compare words, `after`, and every
/// lane outputs VCC_LO.
fn program(breg: u32, b: u32, before: &[u32], compare: &[u32], after: &[u32]) -> Vec<u8> {
    program_with_tail(breg, b, before, compare, after, &[])
}

/// As `program`, with `tail` between the output move and the store.
fn program_with_tail(
    breg: u32,
    b: u32,
    before: &[u32],
    compare: &[u32],
    after: &[u32],
    tail: &[u32],
) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    code.extend(vmov(breg, b));
    code.extend(before);
    code.extend(compare);
    code.extend(after);
    code.push(0x7e04_026a);
    code.extend(tail);
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

fn lanes() -> Vec<u32> {
    VALUES.iter().chain(&MORE_LANES).copied().collect()
}

fn case(form: Compare, family: &'static str, label: String, code: Vec<u8>, mask: u32) -> Case {
    Case {
        form: form.name,
        family,
        label: format!("{} {label}", form.name),
        code,
        input: lanes(),
        expected: vec![mask; 32],
    }
}

pub fn cases() -> Vec<Case> {
    let input = lanes();
    let all = u32::MAX;
    let mut cases = Vec::new();
    for form in COMPARES {
        let op = form.opcode;
        for b in VALUES {
            let code = program(3, b, &[], &[vopc(op, vgpr(1), 3)], &[]);
            let mask = vcc(all, |n| compare(form, input[n], b));
            cases.push(case(form, "vopc domain", format!("v1, {b:#x}"), code, mask));
            let code = program(3, b, &[], &[vopc(op, vgpr(3), 1)], &[]);
            let mask = vcc(all, |n| compare(form, b, input[n]));
            cases.push(case(form, "vopc domain", format!("{b:#x}, v1"), code, mask));
        }
        let code = program(3, 0, &[], &[vopc(op, vgpr(1), 1)], &[]);
        let mask = vcc(all, |n| compare(form, input[n], input[n]));
        cases.push(case(form, "vopc aliasing", "v1, v1".into(), code, mask));
        for selector in 128..=208 {
            let value = inline_integer(selector).unwrap();
            let code = program(3, 0, &[], &[vopc(op, selector, 1)], &[]);
            let mask = vcc(all, |n| compare(form, value, input[n]));
            cases.push(case(
                form,
                "vopc inline integer",
                format!("inline {selector} ({value:#x})"),
                code,
                mask,
            ));
        }
        for literal in LITERALS {
            let code = program(3, 0, &[], &[vopc(op, LITERAL, 1), literal], &[]);
            let mask = vcc(all, |n| compare(form, literal, input[n]));
            cases.push(case(
                form,
                "vopc literal",
                format!("literal {literal:#x}"),
                code,
                mask,
            ));
        }
        // SGPR source s8 and s105, the highest SGPR (guide 5.2).
        for reg in [8, 105] {
            for value in [0x8000_0000, 0xffff_fff0] {
                let set = [0xbe80_03ff | reg << 16, value];
                let code = program(3, 0, &set, &[vopc(op, reg, 1)], &[]);
                let mask = vcc(all, |n| compare(form, value, input[n]));
                cases.push(case(
                    form,
                    "vopc sgpr source",
                    format!("s{reg} = {value:#x}"),
                    code,
                    mask,
                ));
            }
        }
        // v255, the highest VGPR, in each slot.
        let b = 0x8000_0001;
        let code = program(255, b, &[], &[vopc(op, vgpr(1), 255)], &[]);
        let mask = vcc(all, |n| compare(form, input[n], b));
        cases.push(case(
            form,
            "vopc max register",
            "v1, v255".into(),
            code,
            mask,
        ));
        let code = program(255, b, &[], &[vopc(op, vgpr(255), 1)], &[]);
        let mask = vcc(all, |n| compare(form, b, input[n]));
        cases.push(case(
            form,
            "vopc max register",
            "v255, v1".into(),
            code,
            mask,
        ));
        // Partial and empty EXEC around the compare, with VCC_LO = -1 before
        // it, so inactive lanes must be cleared ("VCC is fully written");
        // EXEC_LO = -1 again before the output move so every lane stores VCC_LO.
        let b = 0x8000_0000;
        for exec in EXEC_MASKS {
            let before = [SEED_VCC[0], SEED_VCC[1], SET_EXEC, exec];
            let code = program(3, b, &before, &[vopc(op, vgpr(1), 3)], &[FULL_EXEC]);
            let mask = vcc(exec, |n| compare(form, input[n], b));
            cases.push(case(
                form,
                "vopc exec mask",
                format!("exec {exec:#x}"),
                code,
                mask,
            ));
        }
        // VCCZ after the compare (guide 3.9: VCCZ is VCC == 0), read with
        // s_cbranch_vccz: lanes output VCC_LO when it is zero, else input ^ VCC_LO.
        let vccz = |mask: u32| -> Vec<u32> {
            if mask == 0 {
                vec![0; 32]
            } else {
                input.iter().map(|lane| lane ^ mask).collect()
            }
        };
        for b in VALUES {
            let code = program_with_tail(3, b, &[], &[vopc(op, vgpr(1), 3)], &[], &VCCZ_TAIL);
            let mask = vcc(all, |n| compare(form, input[n], b));
            let mut case = case(form, "vopc vccz", format!("v1, {b:#x}"), code, mask);
            case.expected = vccz(mask);
            cases.push(case);
        }
        let before = [SEED_VCC[0], SEED_VCC[1], SET_EXEC, 0];
        let compare_words = [vopc(op, vgpr(1), 3)];
        let code = program_with_tail(3, 0, &before, &compare_words, &[FULL_EXEC], &VCCZ_TAIL);
        let mut case = case(form, "vopc vccz", "exec 0".into(), code, 0);
        case.expected = vccz(0);
        cases.push(case);
    }
    cases
}

pub fn rejections() -> Vec<Rejection> {
    let mut rejections = Vec::new();
    for form in COMPARES {
        let mut code = program(3, 0, &[], &[], &[]);
        code.truncate(COMPARE_OFFSET);
        code.extend(vopc(form.opcode, LITERAL, 1).to_le_bytes());
        rejections.push(Rejection {
            family: "vopc missing literal",
            truncated: true,
            label: format!("{} missing literal", form.name),
            code,
            offset: COMPARE_OFFSET,
        });
    }
    rejections
}
