// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent per-lane expected values for twelve 32-bit VOP2 integer forms
//! (#215): `v_mul_i32_i24`, `v_mul_hi_i32_i24`, `v_mul_u32_u24`,
//! `v_mul_hi_u32_u24`, `v_min_i32`, `v_max_i32`, `v_min_u32`, `v_max_u32`,
//! `v_xnor_b32`, `v_add_nc_u32`, `v_sub_nc_u32` and `v_subrev_nc_u32`, all
//! `_e32`.
//!
//! The expectations are written below from the public AMD "RDNA 2" Instruction
//! Set Architecture Reference Guide only: section 12.7 "VOP2 Instructions"
//! (pseudocode and descriptions), section 13.3.1 for the VOP2 word and the
//! vector operand table for SRC0 (SGPRs, `vcc_lo`, inline integers, the
//! literal, VGPRs). Nothing here comes from the ISA crate, the semantic
//! registry, the decoder or the shader compiler. Every case runs on the
//! reference interpreter `xenora_ref` and on the second interpreter
//! `independent_ref`; the ignored test also runs the translated MSL on native
//! Metal.
//!
//! Readings of the guide (not hardware evidence):
//! - `.i24` / `.u24` read bits 23:0 of a source; `.i24` sign-extends bit 23.
//!   The product is exact (at most 48 bits); MUL_* keep bits 31:0 and MUL_HI_*
//!   bits 63:32 of the 64-bit two's-complement (I24) or unsigned (U24) value,
//!   so `>> 32` is arithmetic for the signed form.
//! - SUBREV is `D = S1 - S0`; the `_NC` forms have "no carry-in or
//!   carry-out", so none of the twelve writes VCC or SCC.
//! - MAX uses `S0 >= S1 ? S0 : S1`; on a tie both choices are the same value.
//! - A lane whose EXEC bit is clear keeps its old VDST (guide section 3.3).

#[path = "operand_domains/harness.rs"]
#[allow(dead_code)]
mod harness;
#[path = "../independent_ref.rs"]
mod independent_ref;

use std::collections::BTreeMap;

use harness::{Case, words};
use xenora_shader::{Architecture, CompileError, compile};

/// The twelve forms with their ENC_VOP2 opcodes (guide 12.7).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Form {
    MulI24 = 9,
    MulHiI24 = 10,
    MulU24 = 11,
    MulHiU24 = 12,
    MinI32 = 17,
    MaxI32 = 18,
    MinU32 = 19,
    MaxU32 = 20,
    Xnor = 30,
    AddNc = 37,
    SubNc = 38,
    SubrevNc = 39,
}

const FORMS: [(Form, &str); 12] = [
    (Form::MulI24, "v_mul_i32_i24"),
    (Form::MulHiI24, "v_mul_hi_i32_i24"),
    (Form::MulU24, "v_mul_u32_u24"),
    (Form::MulHiU24, "v_mul_hi_u32_u24"),
    (Form::MinI32, "v_min_i32"),
    (Form::MaxI32, "v_max_i32"),
    (Form::MinU32, "v_min_u32"),
    (Form::MaxU32, "v_max_u32"),
    (Form::Xnor, "v_xnor_b32"),
    (Form::AddNc, "v_add_nc_u32"),
    (Form::SubNc, "v_sub_nc_u32"),
    (Form::SubrevNc, "v_subrev_nc_u32"),
];

/// `S.i24`: bits 23:0, sign-extended from bit 23.
fn i24(s: u32) -> i64 {
    let field = i64::from(s & 0x00ff_ffff);
    if field >= 1 << 23 {
        field - (1 << 24)
    } else {
        field
    }
}

/// `S.u24`: bits 23:0, zero-extended.
fn u24(s: u32) -> i64 {
    i64::from(s & 0x00ff_ffff)
}

/// The guide's D for `form` with S0 = `s0` and S1 = `s1`. Arithmetic is exact
/// in i64; `as u32` keeps the low 32 bits.
fn expect(form: Form, s0: u32, s1: u32) -> u32 {
    let (i0, i1) = (i64::from(s0 as i32), i64::from(s1 as i32));
    let (u0, u1) = (i64::from(s0), i64::from(s1));
    match form {
        // 9 V_MUL_I32_I24: D.i32 = S0.i24 * S1.i24.
        Form::MulI24 => (i24(s0) * i24(s1)) as u32,
        // 10 V_MUL_HI_I32_I24: D.i32 = (S0.i24 * S1.i24)>>32.
        Form::MulHiI24 => ((i24(s0) * i24(s1)) >> 32) as u32,
        // 11 V_MUL_U32_U24: D.u32 = S0.u24 * S1.u24.
        Form::MulU24 => (u24(s0) * u24(s1)) as u32,
        // 12 V_MUL_HI_U32_U24: D.u32 = (S0.u24 * S1.u24)>>32.
        Form::MulHiU24 => ((u24(s0) * u24(s1)) >> 32) as u32,
        // 17 V_MIN_I32: D.i32 = (S0.i32 < S1.i32 ? S0.i32 : S1.i32).
        Form::MinI32 => {
            if i0 < i1 {
                s0
            } else {
                s1
            }
        }
        // 18 V_MAX_I32: D.i32 = (S0.i32 >= S1.i32 ? S0.i32 : S1.i32).
        Form::MaxI32 => {
            if i0 >= i1 {
                s0
            } else {
                s1
            }
        }
        // 19 V_MIN_U32: D.u32 = (S0.u32 < S1.u32 ? S0.u32 : S1.u32).
        Form::MinU32 => {
            if u0 < u1 {
                s0
            } else {
                s1
            }
        }
        // 20 V_MAX_U32: D.u32 = (S0.u32 >= S1.u32 ? S0.u32 : S1.u32).
        Form::MaxU32 => {
            if u0 >= u1 {
                s0
            } else {
                s1
            }
        }
        // 30 V_XNOR_B32: D.u32 = ~(S0.u32 ^ S1.u32).
        Form::Xnor => !(s0 ^ s1),
        // 37 V_ADD_NC_U32: D.u32 = S0.u32 + S1.u32. No carry-in or carry-out.
        Form::AddNc => (u0 + u1) as u32,
        // 38 V_SUB_NC_U32: D.u32 = S0.u32 - S1.u32. No carry-in or carry-out.
        Form::SubNc => (u0 - u1) as u32,
        // 39 V_SUBREV_NC_U32: D.u32 = S1.u32 - S0.u32. No carry-in or carry-out.
        Form::SubrevNc => (u1 - u0) as u32,
    }
}

/// Values worked by hand from the pseudocode above (S0, S1, D).
const GUIDE_EXAMPLES: [(Form, u32, u32, u32); 20] = [
    // (-2^23)^2 = 2^46: low half 0, high half 2^14.
    (Form::MulI24, 0x0080_0000, 0x0080_0000, 0x0000_0000),
    (Form::MulHiI24, 0x0080_0000, 0x0080_0000, 0x0000_4000),
    // -1 * 2 = -2, sign-extended into the high half.
    (Form::MulI24, 0x00ff_ffff, 0x0000_0002, 0xffff_fffe),
    (Form::MulHiI24, 0x00ff_ffff, 0x0000_0002, 0xffff_ffff),
    // -2^23 * (2^23 - 1) = -2^46 + 2^23.
    (Form::MulI24, 0x0080_0000, 0x007f_ffff, 0x0080_0000),
    (Form::MulHiI24, 0x0080_0000, 0x007f_ffff, 0xffff_c000),
    // Bits 31:24 are ignored: 3 * 5.
    (Form::MulI24, 0xff00_0003, 0x0100_0005, 0x0000_000f),
    (Form::MulU24, 0xff00_0003, 0x0100_0005, 0x0000_000f),
    // (2^24 - 1)^2 = 0xffff_fe00_0001.
    (Form::MulU24, 0x00ff_ffff, 0x00ff_ffff, 0xfe00_0001),
    (Form::MulHiU24, 0x00ff_ffff, 0x00ff_ffff, 0x0000_ffff),
    (Form::MulHiI24, 0x00ff_ffff, 0x00ff_ffff, 0x0000_0000),
    (Form::MinI32, 0x8000_0000, 0x7fff_ffff, 0x8000_0000),
    (Form::MinU32, 0x8000_0000, 0x7fff_ffff, 0x7fff_ffff),
    (Form::MaxI32, 0x8000_0000, 0x7fff_ffff, 0x7fff_ffff),
    (Form::MaxU32, 0x8000_0000, 0x7fff_ffff, 0x8000_0000),
    (Form::Xnor, 0x0f0f_0f0f, 0x00ff_00ff, 0xf00f_f00f),
    (Form::AddNc, 0xffff_ffff, 0x0000_0001, 0x0000_0000),
    (Form::SubNc, 0x0000_0005, 0x0000_0003, 0x0000_0002),
    (Form::SubrevNc, 0x0000_0005, 0x0000_0003, 0xffff_fffe),
    (Form::SubrevNc, 0x0000_0000, 0x8000_0000, 0x8000_0000),
];

/// Boundary operands (also the first 18 lane inputs): 0, 1, 2, around the
/// 24-bit sign bit and field top, values with bits above 23 set, and the
/// 32-bit boundaries.
const VALUES: [u32; 18] = [
    0x0000_0000,
    0x0000_0001,
    0x0000_0002,
    0x007f_ffff,
    0x0080_0000,
    0x0080_0001,
    0x00ff_ffff,
    0x0100_0000,
    0x1234_5678,
    0x5555_5555,
    0x7fff_ffff,
    0x8000_0000,
    0x8000_0001,
    0xaaaa_aaaa,
    0xff00_0001,
    0xff80_0000,
    0xffff_fffe,
    0xffff_ffff,
];
/// The other 14 lane inputs, so all 32 lanes differ.
const MORE_LANES: [u32; 14] = [
    0x007f_fffe,
    0x00ff_fffe,
    0x0100_0001,
    0x00c0_0000,
    0xff7f_ffff,
    0xff80_0001,
    0xffff_0000,
    0x0000_ffff,
    0x4000_0000,
    0xc000_0000,
    0x8765_4321,
    0x0f0f_0f0f,
    0x7fff_fffe,
    0x1fff_ffff,
];
const LITERALS: [u32; 9] = [
    0,
    1,
    0x7fff_ffff,
    0x8000_0000,
    u32::MAX,
    0x00ff_ffff,
    0x0080_0000,
    0xff80_0000,
    0x0100_0001,
];
const SGPR_VALUES: [u32; 3] = [0x8000_0000, 0x00ff_ffff, 0xff80_0001];
/// XORed with v1 to make a second per-lane source that differs from v1.
const PAIR_KEYS: [u32; 5] = [0x00ff_ffff, 0x8000_0000, u32::MAX, 0x0080_0001, 0x5555_5555];
/// The last one is set explicitly, as a full EXEC written by `s_mov_b32`.
const EXEC_MASKS: [u32; 7] = [
    0,
    0x5555_5555,
    0xaaaa_aaaa,
    0x8000_0001,
    0x0000_ffff,
    0x7fff_fffe,
    u32::MAX,
];
/// XORed with v1 to give every lane its own old VDST before a masked write.
const OLD: u32 = 0x0bad_f00d;

const COUNTS: [(&str, usize); 11] = [
    ("vop2 domain", 12 * 18 * 2),
    ("vop2 lane pairs", 12 * 5 * 2),
    ("vop2 aliasing", 12 * 4),
    ("vop2 inline integer", 12 * 81),
    ("vop2 literal", 12 * 9),
    ("vop2 sgpr source", 12 * 2 * 3),
    ("vop2 vcc_lo source", 12 * 3),
    ("vop2 max register", 12 * 3),
    ("vop2 exec", 12 * (7 + 3)),
    ("vop2 partial wave", 12 * 3),
    ("vop2 no vcc or scc write", 12 * 5),
];
/// Per form: a missing literal, 8 special
/// SRC0 selectors, the 24 reserved selectors 209-232, SDWA, DPP16 and two DPP8 forms, and the VOP3 form with
/// clamp; and for each form but ADD_NC (whose plain VOP3 form is supported
/// already) the plain VOP3 form.
const REJECTIONS: usize = 12 * (1 + 8 + 24 + 4 + 1) + 11;

const LITERAL: u32 = 255;
const VCC_LO: u32 = 106;

/// ENCODING 0 in bit 31, OP 25-30, VDST 17-24, VSRC1 9-16, SRC0 0-8 (guide
/// 13.3.1); SRC0 256 + n is VGPR n.
fn vop2(op: u32, dst: u32, src0: u32, vsrc1: u32) -> u32 {
    op << 25 | dst << 17 | vsrc1 << 9 | src0
}

const fn vgpr(n: u32) -> u32 {
    256 + n
}

/// `v_mov_b32 v<reg>, literal` (VOP1 opcode 1).
fn vmov(reg: u32, value: u32) -> [u32; 2] {
    [0x7e00_02ff | reg << 17, value]
}

/// `s_mov_b32 s<reg>, literal` (SOP1 opcode 3).
fn smov(reg: u32, value: u32) -> [u32; 2] {
    [0xbe80_03ff | reg << 16, value]
}

/// `v_xor_b32 v<dst>, v<a>, v<b>` (VOP2 opcode 29, supported before this form set).
fn vxor(dst: u32, a: u32, b: u32) -> u32 {
    vop2(29, dst, vgpr(a), b)
}

/// `s_mov_b32 exec_lo, -1`.
const FULL_EXEC: u32 = 0xbefe_03c1;
/// `s_cmp_eq_u32 0, 0` (SCC = 1) and `s_cmp_eq_u32 0, 1` (SCC = 0).
const SCC_SET: u32 = 0xbf06_8080;
const SCC_CLEAR: u32 = 0xbf06_8180;
/// v6 = vcc_lo; v2 ^= v6; `s_cbranch_scc0` over v2 ^= v1.
const FLAG_TAIL: [u32; 4] = [0x7e0c_026a, 0x3a04_0506, 0xbf84_0001, 0x3a04_0501];

/// Load v1 = lane input, then `setup`, the ALU words, `after`, then
/// v2 = v<result> (unless it is v2) and store v2.
fn program(setup: &[u32], alu: &[u32], after: &[u32], result: u32) -> Vec<u8> {
    let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
    code.extend(setup);
    code.extend(alu);
    code.extend(after);
    if result != 2 {
        code.push(0x7e04_0300 | result);
    }
    code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
    words(&code)
}

/// The ALU word's byte offset in `program` with `setup`.
fn alu_offset(setup: &[u32]) -> usize {
    12 + 4 * setup.len()
}

fn lanes() -> Vec<u32> {
    VALUES.iter().chain(&MORE_LANES).copied().collect()
}

/// A case whose lane `n` (dispatch index) with input `x` expects `lane(n, x)`.
fn case(
    (_, name): (Form, &'static str),
    family: &'static str,
    label: String,
    code: Vec<u8>,
    input: Vec<u32>,
    lane: impl Fn(usize, u32) -> u32,
) -> Case {
    let expected = input.iter().enumerate().map(|(n, &x)| lane(n, x)).collect();
    Case {
        form: name,
        family,
        label: format!("{name} {label}"),
        code,
        input,
        expected,
    }
}

/// D where the lane is active under `exec` (bit `n % 32`), else the old VDST.
fn masked(exec: u32, n: usize, d: u32, old: u32) -> u32 {
    if exec >> (n % 32) & 1 == 1 { d } else { old }
}

fn inline_integer(selector: u32) -> u32 {
    match selector {
        128..=192 => selector - 128,
        193..=208 => (-((selector - 192) as i32)) as u32,
        _ => unreachable!("not an integer inline constant"),
    }
}

fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for entry @ (form, _) in FORMS {
        let op = form as u32;
        let f = move |a, b| expect(form, a, b);
        // Every boundary value against all 32 lane inputs, in each slot.
        for b in VALUES {
            let setup = vmov(3, b);
            let code = program(&setup, &[vop2(op, 2, vgpr(1), 3)], &[], 2);
            let label = format!("v1, {b:#x}");
            cases.push(case(entry, "vop2 domain", label, code, lanes(), |_, x| {
                f(x, b)
            }));
            let code = program(&setup, &[vop2(op, 2, vgpr(3), 1)], &[], 2);
            let label = format!("{b:#x}, v1");
            cases.push(case(entry, "vop2 domain", label, code, lanes(), |_, x| {
                f(b, x)
            }));
        }
        // Both sources differ per lane: v4 = v1 ^ key.
        for key in PAIR_KEYS {
            let mut setup = vmov(3, key).to_vec();
            setup.push(vxor(4, 3, 1));
            let family = "vop2 lane pairs";
            let code = program(&setup, &[vop2(op, 2, vgpr(1), 4)], &[], 2);
            let label = format!("v1, v1 ^ {key:#x}");
            cases.push(case(entry, family, label, code, lanes(), |_, x| {
                f(x, x ^ key)
            }));
            let code = program(&setup, &[vop2(op, 2, vgpr(4), 1)], &[], 2);
            let label = format!("v1 ^ {key:#x}, v1");
            cases.push(case(entry, family, label, code, lanes(), |_, x| {
                f(x ^ key, x)
            }));
        }
        // VDST aliasing a source, with v3 = b.
        let b = 0xff80_0001;
        let setup = vmov(3, b);
        let family = "vop2 aliasing";
        let alu = [vop2(op, 1, vgpr(1), 1)];
        let code = program(&setup, &alu, &[], 1);
        cases.push(case(
            entry,
            family,
            "v1 = v1, v1".into(),
            code,
            lanes(),
            |_, x| f(x, x),
        ));
        let code = program(&setup, &[vop2(op, 1, vgpr(1), 3)], &[], 1);
        cases.push(case(
            entry,
            family,
            "v1 = v1, v3".into(),
            code,
            lanes(),
            |_, x| f(x, b),
        ));
        let code = program(&setup, &[vop2(op, 3, vgpr(1), 3)], &[], 3);
        cases.push(case(
            entry,
            family,
            "v3 = v1, v3".into(),
            code,
            lanes(),
            |_, x| f(x, b),
        ));
        let code = program(&setup, &[vop2(op, 3, vgpr(3), 3)], &[], 3);
        cases.push(case(
            entry,
            family,
            "v3 = v3, v3".into(),
            code,
            lanes(),
            |_, _| f(b, b),
        ));
        // Every integer inline constant (-16..64) as SRC0.
        for selector in 128..=208 {
            let value = inline_integer(selector);
            let code = program(&[], &[vop2(op, 2, selector, 1)], &[], 2);
            let label = format!("inline {selector} ({value:#x}), v1");
            let family = "vop2 inline integer";
            cases.push(case(entry, family, label, code, lanes(), |_, x| {
                f(value, x)
            }));
        }
        // One literal dword after the VOP2 word as SRC0.
        for literal in LITERALS {
            let code = program(&[], &[vop2(op, 2, LITERAL, 1), literal], &[], 2);
            let label = format!("literal {literal:#x}, v1");
            let family = "vop2 literal";
            cases.push(case(entry, family, label, code, lanes(), |_, x| {
                f(literal, x)
            }));
        }
        // SGPR SRC0: s8 and s105, the highest SGPR (guide 5.2), and vcc_lo.
        for (reg, family) in [
            (8, "vop2 sgpr source"),
            (105, "vop2 sgpr source"),
            (VCC_LO, "vop2 vcc_lo source"),
        ] {
            for value in SGPR_VALUES {
                let code = program(&smov(reg, value), &[vop2(op, 2, reg, 1)], &[], 2);
                let label = format!("s{reg} = {value:#x}, v1");
                cases.push(case(entry, family, label, code, lanes(), |_, x| {
                    f(value, x)
                }));
            }
        }
        // v255, the highest VGPR, in each slot.
        let b = 0x8000_0001;
        let setup = vmov(255, b);
        let family = "vop2 max register";
        let alu = [vop2(op, 255, vgpr(1), 1)];
        let code = program(&setup, &alu, &[], 255);
        cases.push(case(
            entry,
            family,
            "v255 = v1, v1".into(),
            code,
            lanes(),
            |_, x| f(x, x),
        ));
        let code = program(&setup, &[vop2(op, 2, vgpr(255), 1)], &[], 2);
        cases.push(case(
            entry,
            family,
            "v255, v1".into(),
            code,
            lanes(),
            |_, x| f(b, x),
        ));
        let code = program(&setup, &[vop2(op, 2, vgpr(1), 255)], &[], 2);
        cases.push(case(
            entry,
            family,
            "v1, v255".into(),
            code,
            lanes(),
            |_, x| f(x, b),
        ));
        // EXEC around the ALU word: v2 = v1 ^ OLD before it, so each inactive
        // lane must keep its own old value; EXEC is -1 again for the store.
        let b = 0x00ff_fff0;
        let mut setup = vmov(3, b).to_vec();
        setup.extend(vmov(5, OLD));
        setup.push(vxor(2, 5, 1));
        let family = "vop2 exec";
        for exec in EXEC_MASKS {
            let mut before = setup.clone();
            before.extend(smov(126, exec));
            let code = program(&before, &[vop2(op, 2, vgpr(1), 3)], &[FULL_EXEC], 2);
            let label = format!("exec {exec:#x}, v1, {b:#x}");
            cases.push(case(entry, family, label, code, lanes(), |n, x| {
                masked(exec, n, f(x, b), x ^ OLD)
            }));
        }
        // Partial EXEC with a scalar SRC0 (s8) and with a literal.
        let exec = 0x0f0f_0f0f;
        let value = 0xff80_0003;
        let mut before = setup.clone();
        before.extend(smov(8, value));
        before.extend(smov(126, exec));
        let code = program(&before, &[vop2(op, 2, 8, 1)], &[FULL_EXEC], 2);
        let label = format!("exec {exec:#x}, s8 = {value:#x}, v1");
        cases.push(case(entry, family, label, code, lanes(), |n, x| {
            masked(exec, n, f(value, x), x ^ OLD)
        }));
        // The literal also under empty EXEC: its dword is still consumed.
        for exec in [exec, 0] {
            let mut before = setup.clone();
            before.extend(smov(126, exec));
            let alu = [vop2(op, 2, LITERAL, 1), value];
            let code = program(&before, &alu, &[FULL_EXEC], 2);
            let label = format!("exec {exec:#x}, literal {value:#x}, v1");
            cases.push(case(entry, family, label, code, lanes(), |n, x| {
                masked(exec, n, f(value, x), x ^ OLD)
            }));
        }
        // A final wave with absent lanes: 7 lanes, and 45 lanes (one full wave
        // and 13 lanes), also with partial EXEC in both waves.
        let family = "vop2 partial wave";
        let long: Vec<u32> = lanes()
            .into_iter()
            .chain(MORE_LANES.into_iter().take(13))
            .collect();
        let code = program(&setup, &[vop2(op, 2, vgpr(1), 3)], &[], 2);
        let label = "7 lanes".into();
        let short = lanes()[..7].to_vec();
        cases.push(case(entry, family, label, code.clone(), short, |_, x| {
            f(x, b)
        }));
        cases.push(case(
            entry,
            family,
            "45 lanes".into(),
            code,
            long.clone(),
            |_, x| f(x, b),
        ));
        let exec = 0x5555_5555;
        let mut before = setup.clone();
        before.extend(smov(126, exec));
        let code = program(&before, &[vop2(op, 2, vgpr(1), 3)], &[FULL_EXEC], 2);
        let label = format!("45 lanes, exec {exec:#x}");
        cases.push(case(entry, family, label, code, long, |n, x| {
            masked(exec, n, f(x, b), x ^ OLD)
        }));
        // VCC_LO seeded to 0 or -1 and SCC to 0 or 1 before the ALU word.
        // Afterwards each lane outputs D ^ VCC_LO, XORed with its input when
        // SCC is set, so any VCC or SCC write changes the output. b = -1 makes
        // ADD carry and SUB borrow in most lanes.
        let b = u32::MAX;
        for vcc in [0, u32::MAX] {
            for scc in [false, true] {
                let mut setup = vmov(3, b).to_vec();
                setup.extend(smov(VCC_LO, vcc));
                setup.push(if scc { SCC_SET } else { SCC_CLEAR });
                let code = program(&setup, &[vop2(op, 2, vgpr(1), 3)], &FLAG_TAIL, 2);
                let label = format!("vcc_lo {vcc:#x}, scc {scc}");
                let family = "vop2 no vcc or scc write";
                cases.push(case(entry, family, label, code, lanes(), |_, x| {
                    f(x, b) ^ vcc ^ if scc { x } else { 0 }
                }));
            }
        }
        // The same under partial EXEC with SCC = 1 and VCC_LO bits both inside
        // and outside EXEC, so a VCC write limited to active lanes also shows.
        let (exec, vcc) = (0x0f0f_0f0f, 0x0000_00f3);
        let mut before = setup.clone();
        before.extend(vmov(3, b));
        before.extend(smov(VCC_LO, vcc));
        before.push(SCC_SET);
        before.extend(smov(126, exec));
        let mut after = vec![FULL_EXEC];
        after.extend(FLAG_TAIL);
        let code = program(&before, &[vop2(op, 2, vgpr(1), 3)], &after, 2);
        let label = format!("exec {exec:#x}, vcc_lo {vcc:#x}, scc true");
        let family = "vop2 no vcc or scc write";
        cases.push(case(entry, family, label, code, lanes(), |n, x| {
            masked(exec, n, f(x, b), x ^ OLD) ^ vcc ^ x
        }));
    }
    cases
}

/// Programs both interpreters must refuse at the ALU word, with that offset.
fn rejections() -> Vec<(String, Vec<u8>, usize)> {
    let setup = vmov(3, 7);
    let offset = alu_offset(&setup);
    let mut all = Vec::new();
    for (form, name) in FORMS {
        let op = form as u32;
        // A literal SRC0 with no literal dword: the program ends at the word.
        let mut code = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
        code.extend(setup);
        code.push(vop2(op, 2, LITERAL, 1));
        all.push((format!("{name} missing literal"), words(&code), offset));
        // Reserved selectors 108-110, M0, NULL, VCCZ, EXECZ and SCC
        // (outside the supported SRC0 subset), and reserved selectors 209-232.
        let special = [108, 109, 110, 124, 125, 251, 252, 253];
        for src0 in special.into_iter().chain(209..=232) {
            let code = program(&setup, &[vop2(op, 2, src0, 1)], &[], 2);
            all.push((format!("{name} src0 {src0}"), code, offset));
        }
        // SDWA (249) with SRC0 v1 and DWORD selects, DPP16 (250) with the
        // identity quad_perm, DPP8 (233) and DPP8FI (234) with the identity
        // lane selects.
        for (src0, extension) in [
            (249, 0x0606_0601),
            (250, 0xff00_e401),
            (233, 0xfac6_8801),
            (234, 0xfac6_8801),
        ] {
            let code = program(&setup, &[vop2(op, 2, src0, 3), extension], &[], 2);
            all.push((format!("{name} src0 {src0} extension"), code, offset));
        }
        // VOP3 (ENCODING 0b110101, OP 256 + the VOP2 opcode): v2 = v1, v3,
        // with the clamp bit, and without it for the forms not yet supported
        // as VOP3.
        let vop3 = 0xd400_0000 | (256 + op) << 16 | 2;
        let sources = vgpr(1) | vgpr(3) << 9;
        let code = program(&setup, &[vop3 | 0x8000, sources], &[], 2);
        all.push((format!("{name} VOP3 clamp"), code, offset));
        if form != Form::AddNc {
            let code = program(&setup, &[vop3, sources], &[], 2);
            all.push((format!("{name} VOP3"), code, offset));
        }
    }
    all
}

fn assert_rejected_at(label: &str, code: &[u8], offset: usize, input: &[u32]) {
    match xenora_ref::execute(code, input) {
        Ok(_) => panic!("reference accepted {label}"),
        Err(error) => assert_eq!(
            error.offset, offset,
            "reference rejected {label} elsewhere: {error:?}"
        ),
    }
    match independent_ref::execute(code, input) {
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
fn expectations_reproduce_the_worked_examples() {
    for (form, s0, s1, d) in GUIDE_EXAMPLES {
        assert_eq!(expect(form, s0, s1), d, "{form:?}({s0:#x}, {s1:#x})");
    }
}

#[test]
fn vop2_integer_forms_match_the_guide_on_the_reference() {
    let cases = checked_cases();
    let failed = failures(&cases, |code, input| {
        xenora_ref::execute(code, input).map_err(|error| format!("{error:?}"))
    });
    assert!(
        failed.is_empty(),
        "reference disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!("vop2_integer: {} reference cases", cases.len());
    for (form, n) in tally(&cases, |case| case.form) {
        eprintln!("  form {form}: {n}");
    }
}

#[test]
fn vop2_integer_forms_match_the_guide_on_the_independent_interpreter() {
    let cases = checked_cases();
    let failed = failures(&cases, independent_ref::execute);
    assert!(
        failed.is_empty(),
        "independent interpreter disagrees with the guide:\n{}",
        failed.join("\n")
    );
    eprintln!(
        "vop2_integer: {} independent-interpreter cases",
        cases.len()
    );
}

#[test]
fn unsupported_vop2_integer_encodings_are_rejected() {
    let rejections = rejections();
    assert_eq!(rejections.len(), REJECTIONS);
    for (label, code, offset) in &rejections {
        assert_rejected_at(label, code, *offset, &lanes());
    }
    eprintln!("vop2_integer: {} rejections", rejections.len());
}

#[test]
fn other_vop2_opcodes_stay_unsupported() {
    // Every VOP2 opcode (0-61; 62 and 63 are the VOPC and VOP1 encodings)
    // except the forms supported before this set (V_ADD_F32 3, V_MUL_F32 8,
    // the shifts 22, 24, 26 and AND, OR, XOR 27-29) and these twelve. VDST is
    // initialized, so a form that also reads it fails only on the opcode.
    let earlier = [3, 8, 22, 24, 26, 27, 28, 29];
    let supported: Vec<u32> = FORMS
        .iter()
        .map(|&(form, _)| form as u32)
        .chain(earlier)
        .collect();
    let mut setup = vmov(3, 7).to_vec();
    setup.extend(vmov(2, 5));
    for op in (0..=61).filter(|op| !supported.contains(op)) {
        let code = program(&setup, &[vop2(op, 2, vgpr(1), 3)], &[], 2);
        assert_rejected_at(&format!("opcode {op}"), &code, alu_offset(&setup), &lanes());
    }
}

#[test]
#[ignore = "requires a native Metal device with API and shader validation"]
fn vop2_integer_forms_match_the_guide_on_native_metal() {
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
        "vop2_integer: {} Metal cases in {programs} programs",
        cases.len()
    );
}
