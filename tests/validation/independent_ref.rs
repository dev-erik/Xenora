// SPDX-License-Identifier: GPL-3.0-or-later

//! A second, structurally independent interpreter for the demo instruction
//! subset, used only by the semantic-review tests.
//!
//! This module deliberately imports neither `xenora-decoder`, `xenora-isa` nor
//! `xenora-ref`. It re-decodes the gfx1030 encodings from the AMD RDNA 2 ISA
//! field layout and executes them with an explicit program-counter machine, so
//! a defect in the production decoder or reference cannot cancel out here.
//!
//! Scope matches the demo fixtures: scalar move, eleven SOP1 unary forms
//! (NOT, BREV, BCNT0/1, FF0/1, FLBIT, FLBIT_I32, SEXT_I8/I16, ABS), the twelve SOPC 32-bit integer
//! compares, branch, SOP2 unsigned
//! and signed add and subtract, add-with-carry, subtract-with-borrow, AND, OR
//! and XOR, the stage C forms ANDN2, ORN2, NAND, NOR, XNOR, LSHL, LSHR and
//! ASHR (32-bit), the 32-bit integer forms MIN/MAX (I32, U32), CSELECT, BFM,
//! MUL_I32, BFE (U32, I32), ABSDIFF, LSHL1-4_ADD, PACK (LL, LH, HH) and MUL_HI (U32, I32), VOP1 move
//! (including from an SGPR),
//! VOP2 bitwise, shift, arithmetic-shift and float, the VOP2 integer forms
//! MUL_I32_I24, MUL_HI_I32_I24, MUL_U32_U24, MUL_HI_U32_U24, MIN/MAX (I32,
//! U32), XNOR and ADD/SUB/SUBREV_NC_U32 (any SRC0 a VOPC compare takes; a
//! shift-and-add multiplier and floor division for the high half, the ripple
//! adder for add and subtract), VOP3 add-no-carry, MUBUF
//! load/store and the two waits plus `s_endpgm`. Anything else is a controlled
//! error. The arithmetic shift is evaluated as a floor division by a power of
//! two rather than with a host signed shift, so the oracle does not share the
//! production operator. SOP2 arithmetic runs through a bit-serial ripple
//! adder (subtraction as addition of the complement), not host integer ops.
//!
//! Wave32 VOPC integer compares (`v_cmp_{lt,eq,le,gt,ne,ge}_{i32,u32}_e32`,
//! contract `docs/shaders/vector-compare.md`) need all lanes of a wave, so execution
//! runs lockstep waves of 32 consecutive dispatch indices with wave-uniform
//! scalar state and per-lane vector state. A lane-local failure stops only that
//! lane and the lowest-index error is reported, so programs without compares
//! fail exactly as independent per-lane runs did. Signed compares flip the sign
//! bit of both operands and then compare unsigned. VCC_LO is uninitialized,
//! known or lane-derived; a lane-derived VCC_LO feeds `s_cbranch_vccz`/`s_cbranch_vccnz` only
//! after a static check of the skipped region.

const MAX_CODE_BYTES: usize = 1024;
const MAX_INPUT_WORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Src {
    Reg(u8),
    Imm(u32),
}

/// A VOPC SRC0 operand (contract 2 allowlist).
#[derive(Clone, Copy, Debug)]
enum CmpSrc {
    Sgpr(u8),
    Vcc,
    Imm(u32),
    Vgpr(u8),
}

// Independent bit-search formulation, without production tables or evaluators.
#[derive(Clone, Copy, Debug)]
enum Unary {
    Complement,
    Mirror,
    High,
    Low,
    SignedHigh,
}

impl Unary {
    fn result(self, input: u32) -> u32 {
        match self {
            Self::Complement => input ^ 0xffff_ffff,
            Self::Mirror => {
                let mut output = 0;
                for bit in 0..32 {
                    output |= ((input >> bit) & 1) << (31 - bit);
                }
                output
            }
            Self::Low => (0..32)
                .find(|&bit| input & (1 << bit) != 0)
                .unwrap_or(u32::MAX),
            Self::High => (0..32)
                .find(|&n| input & (1 << (31 - n)) != 0)
                .unwrap_or(u32::MAX),
            Self::SignedHigh => {
                let sign = input >> 31;
                (1..32)
                    .find(|&n| ((input >> (31 - n)) & 1) != sign)
                    .unwrap_or(u32::MAX)
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Insn {
    SMov(u8, Src),
    /// SOP1 unary opcode 7, 11, 13, 15, 17, 19, 21, 23, 25, 26 or 52.
    SUnary(u32, u8, Src),
    SBitWrite(u8, Src, bool),
    SBitTest(Src, Src, bool),
    SConditional(u8, Src),
    /// SOP2 opcode 0-10, 14, 16, 18, an even opcode 20-36, 38-40, 44, 46-54.
    SAlu(u32, u8, Src, Src),
    /// SOPC opcode 0-11: `s_cmp_{eq,lg,gt,ge,lt,le}_i32` then `_u32`.
    SCmp(u32, Src, Src),
    BranchScc0(usize),
    BranchScc1(usize),
    BranchMask {
        target: usize,
        register: usize,
        nonzero: bool,
    },
    Branch(usize),
    VMov(u8, Src),
    VMovFromSgpr(u8, u8),
    /// `v_mov_b32_e32 vD, vcc_lo`.
    VMovFromVcc(u8),
    /// VOPC compare into VCC_LO: relation 1 lt, 2 eq, 3 le, 4 gt, 5 ne, 6 ge.
    VCmp {
        relation: u8,
        signed: bool,
        src0: CmpSrc,
        src1: u8,
    },
    VMadSad(u32, bool, u8, CmpSrc, CmpSrc, CmpSrc),
    VSaturate(u32, u8, CmpSrc, CmpSrc),
    VAlias(u32, u8, CmpSrc, CmpSrc),
    VAddNc(u8, u8, u8),
    VMulLow(u8, CmpSrc, CmpSrc),
    VMulHighUnsigned(u8, CmpSrc, CmpSrc),
    VMulHighSigned(u8, CmpSrc, CmpSrc),
    VBitSelect(u8, CmpSrc, CmpSrc, CmpSrc),
    VAddThree(u8, CmpSrc, CmpSrc, CmpSrc),
    VAndOr(u8, CmpSrc, CmpSrc, CmpSrc),
    VMad16(bool, u8, CmpSrc, CmpSrc, CmpSrc),
    VXad(u8, CmpSrc, CmpSrc, CmpSrc),
    VLerp(u8, CmpSrc, CmpSrc, CmpSrc),
    VMsad(u8, CmpSrc, CmpSrc, CmpSrc),
    VLeastSigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VLeastUnsigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VGreatestSigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VGreatestUnsigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VMiddleSigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VMiddleUnsigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VSignedField(u8, CmpSrc, CmpSrc, CmpSrc),
    VShiftThenSum(u8, CmpSrc, CmpSrc, CmpSrc),
    VSumThenShift(u8, CmpSrc, CmpSrc, CmpSrc),
    VShiftUnion(u8, CmpSrc, CmpSrc, CmpSrc),
    VExtractUnsigned(u8, CmpSrc, CmpSrc, CmpSrc),
    VMask(u8, CmpSrc, CmpSrc),
    VAlignBits(u8, CmpSrc, CmpSrc, CmpSrc),
    VCountAdd(u8, CmpSrc, CmpSrc),
    VOrThree(u8, CmpSrc, CmpSrc, CmpSrc),
    VXorThree(u8, CmpSrc, CmpSrc, CmpSrc),
    VAnd(u8, CmpSrc, u8),
    VOr(u8, CmpSrc, u8),
    VShiftLeft(u8, CmpSrc, u8),
    VShiftRight(u8, CmpSrc, u8),
    VShiftRightArithmetic(u8, CmpSrc, u8),
    VXor(u8, CmpSrc, u8),
    /// VOP2 integer opcode 9-12, 17-20, 30 or 37-39: VDST, SRC0, VSRC1.
    VInt(u32, u8, CmpSrc, u8),
    VUnary(Unary, u8, CmpSrc),
    VAddF(u8, u8, u8),
    VMulF(u8, u8, u8),
    Load(u8, u8, bool),
    Store(u8),
    WaitRead,
    WaitWrite,
    End,
}

/// Execute the synthetic fixture ABI and return one output word per input word.
pub fn execute(code: &[u8], input: &[u32]) -> Result<Vec<u32>, String> {
    if input.is_empty() || input.len() > MAX_INPUT_WORDS {
        return Err("input word count outside limits".to_string());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("code exceeds limit".to_string());
    }
    let program = decode(code)?;
    let mut output = Vec::with_capacity(input.len());
    for wave in input.chunks(32) {
        output.extend(run_wave(&program, wave)?);
    }
    Ok(output)
}

/// Per-lane vector and memory state of one wave lane.
struct Lane {
    input: u32,
    vgpr: [Option<u32>; 256],
    pending_read: Option<u8>,
    pending_value: u32,
    pending_write: Option<u32>,
    written: Option<u32>,
    /// A lane-local failure; the lane stops executing.
    error: Option<String>,
}

/// The lowest-index failed lane's error, in dispatch order.
fn first_error(lanes: &[Lane]) -> Option<String> {
    lanes.iter().find_map(|lane| lane.error.clone())
}

/// Fail every still-running lane with a wave-uniform error and report the
/// lowest-index failure, exactly as independent per-lane runs would.
fn wave_error(lanes: &mut [Lane], error: String) -> String {
    for lane in lanes.iter_mut() {
        lane.error.get_or_insert_with(|| error.clone());
    }
    first_error(lanes).unwrap_or(error)
}

fn run_wave(program: &[(usize, Insn)], wave: &[u32]) -> Result<Vec<u32>, String> {
    let mut lanes: Vec<Lane> = wave
        .iter()
        .map(|&input| Lane {
            input,
            vgpr: [None; 256],
            pending_read: None,
            pending_value: 0,
            pending_write: None,
            written: None,
            error: None,
        })
        .collect();
    // Scalar state is wave-uniform and shared by all lanes.
    let mut sgpr = [None::<u32>; 128];
    sgpr[126] = Some(u32::MAX);
    let mut scc: Option<bool> = None;
    // sgpr[106] holds VCC_LO's value; this flag marks it lane-derived.
    let mut vcc_lane_derived = false;
    let mut pc = 0usize;

    while pc < program.len() {
        let (offset, insn) = program[pc];
        if lanes.iter().all(|lane| lane.error.is_some()) {
            break;
        }
        let read_s = |sgpr: &[Option<u32>; 128], src: Src| -> Result<u32, String> {
            match src {
                Src::Imm(value) => Ok(value),
                Src::Reg(reg) => {
                    sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
                }
            }
        };
        let execution_mask = match read_s(&sgpr, Src::Reg(126)) {
            Ok(mask) => mask,
            Err(error) => return Err(wave_error(&mut lanes, error)),
        };

        let scalar: Result<Option<usize>, String> = match insn {
            Insn::End => {
                for lane in lanes.iter_mut().filter(|lane| lane.error.is_none()) {
                    if lane.pending_read.is_some() || lane.pending_write.is_some() {
                        lane.error = Some(format!("{offset:#x}: outstanding transfer at end"));
                    } else if lane.written.is_none() {
                        lane.error = Some(format!("{offset:#x}: missing output store"));
                    }
                }
                if let Some(error) = first_error(&lanes) {
                    return Err(error);
                }
                return Ok(lanes.iter().map(|lane| lane.written.unwrap()).collect());
            }
            Insn::VCmp {
                relation,
                signed,
                src0,
                src1,
            } => {
                // A compare couples lanes; a failed lane has no defined
                // contribution, so the wave stops with its error.
                if let Some(error) = first_error(&lanes) {
                    return Err(error);
                }
                compare(
                    &lanes,
                    &sgpr,
                    execution_mask,
                    (relation, signed, src0, src1),
                    offset,
                )
                .map(|mask| {
                    sgpr[106] = Some(mask);
                    // VCC_HI has no known value after a compare (contract 3).
                    sgpr[107] = None;
                    vcc_lane_derived = execution_mask != 0;
                    None
                })
            }
            Insn::SBitWrite(dst, index, set) => {
                if dst < 8 {
                    Err(format!("{offset:#x}: write to a resource SGPR"))
                } else {
                    read_s(&sgpr, Src::Reg(dst)).and_then(|old| {
                        read_s(&sgpr, index).map(|index| {
                            let bit = 2u32.pow(index % 32);
                            let present = !(old / bit).is_multiple_of(2);
                            sgpr[dst as usize] = Some(match (present, set) {
                                (false, true) => old + bit,
                                (true, false) => old - bit,
                                _ => old,
                            });
                            None
                        })
                    })
                }
            }
            Insn::SBitTest(src, index, set) => read_s(&sgpr, src).and_then(|word| {
                read_s(&sgpr, index).map(|index| {
                    scc = Some((word / 2u32.pow(index % 32)) % 2 == u32::from(set));
                    None
                })
            }),
            Insn::SConditional(dst, src) => {
                if dst < 8 {
                    Err(format!("{offset:#x}: write to a resource SGPR"))
                } else {
                    scc.ok_or_else(|| {
                        format!("{offset:#x}: conditional move requires initialized SCC")
                    })
                    .and_then(|take| {
                        if take {
                            sgpr[dst as usize] = Some(read_s(&sgpr, src)?);
                        }
                        Ok(None)
                    })
                }
            }
            Insn::SUnary(opcode, dst, src) => {
                if dst < 8 {
                    Err(format!("{offset:#x}: write to a resource SGPR"))
                } else {
                    read_s(&sgpr, src).map(|value| {
                        let (result, flag) = scalar_unary(opcode, value);
                        sgpr[dst as usize] = Some(result);
                        if let Some(flag) = flag {
                            scc = Some(flag);
                        }
                        None
                    })
                }
            }
            Insn::SMov(dst, src) => {
                if dst < 8 {
                    Err(format!("{offset:#x}: write to a resource SGPR"))
                } else {
                    read_s(&sgpr, src).map(|value| {
                        sgpr[dst as usize] = Some(value);
                        if dst == 106 {
                            vcc_lane_derived = false;
                        }
                        None
                    })
                }
            }
            Insn::SCmp(opcode, a, b) => read_s(&sgpr, a).and_then(|a| {
                read_s(&sgpr, b).map(|b| {
                    scc = Some(scalar_compare(opcode, a, b));
                    None
                })
            }),
            Insn::SAlu(opcode, dst, a, b) => {
                scalar_alu(&mut sgpr, &mut scc, (opcode, dst, a, b), offset).map(|()| None)
            }
            Insn::BranchScc0(target) => scc
                .ok_or_else(|| format!("{offset:#x}: uninitialized SCC"))
                .and_then(|condition| {
                    if condition {
                        Ok(None)
                    } else {
                        index_of(program, target, offset).map(Some)
                    }
                }),
            Insn::BranchScc1(target) => scc
                .ok_or_else(|| format!("{offset:#x}: uninitialized SCC"))
                .and_then(|condition| {
                    if condition {
                        index_of(program, target, offset).map(Some)
                    } else {
                        Ok(None)
                    }
                }),
            Insn::BranchMask {
                target,
                register,
                nonzero,
            } => {
                if register == 106 && vcc_lane_derived {
                    let live = lanes.iter().find(|lane| lane.error.is_none()).unwrap();
                    check_runtime_region(program, pc, target, live, &sgpr, execution_mask, offset)
                        .and_then(|()| {
                            if (sgpr[106] != Some(0)) == nonzero {
                                index_of(program, target, offset).map(Some)
                            } else {
                                Ok(None)
                            }
                        })
                } else {
                    sgpr[register]
                        .ok_or_else(|| format!("{offset:#x}: uninitialized mask"))
                        .and_then(|mask| {
                            if (mask != 0) == nonzero {
                                index_of(program, target, offset).map(Some)
                            } else {
                                Ok(None)
                            }
                        })
                }
            }
            Insn::Branch(target) => index_of(program, target, offset).map(Some),
            _ => {
                for (lane_index, lane) in lanes.iter_mut().enumerate() {
                    if lane.error.is_none()
                        && let Err(error) =
                            lane_step(lane, &sgpr, execution_mask, insn, offset, lane_index)
                    {
                        lane.error = Some(error);
                    }
                }
                Ok(None)
            }
        };
        match scalar {
            Ok(Some(next)) => pc = next,
            Ok(None) => pc += 1,
            Err(error) => return Err(wave_error(&mut lanes, error)),
        }
    }
    Err(first_error(&lanes).unwrap_or_else(|| "stream ended without s_endpgm".to_string()))
}

fn scalar_alu(
    sgpr: &mut [Option<u32>; 128],
    scc: &mut Option<bool>,
    (opcode, dst, a, b): (u32, u8, Src, Src),
    offset: usize,
) -> Result<(), String> {
    if dst < 8 {
        return Err(format!("{offset:#x}: write to a resource SGPR"));
    }
    let read = |src: Src| match src {
        Src::Imm(value) => Ok(value),
        Src::Reg(reg) => {
            sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
        }
    };
    let (a, b) = (read(a)?, read(b)?);
    let carry_in = || scc.ok_or_else(|| format!("{offset:#x}: uninitialized SCC"));
    let bit = |word: u32, i: u32| (word >> i) & 1 == 1;
    let (value, flag) = match opcode {
        0 => ripple(a, b, false),
        // RDNA 2 ISA guide 12.1: MIN/MAX pick S0 iff S0 < S1 (> for MAX), the
        // order a borrow out of S0 - S1 (or S1 - S0) shows; signed forms flip
        // both sign bits first. SCC = whether S0 was picked (0 on a tie).
        6..=9 => {
            let flip = if opcode % 2 == 0 { 0x8000_0000 } else { 0 };
            let (x, y) = (a ^ flip, b ^ flip);
            let (p, q) = if opcode < 8 { (x, y) } else { (y, x) };
            let first = !ripple(p, !q, true).1;
            (if first { a } else { b }, first)
        }
        // CSELECT: D = SCC ? S0 : S1.
        10 => {
            let pick_first = carry_in()?;
            (if pick_first { a } else { b }, false)
        }
        // BFM: bit i of D is set iff S1[4:0] <= i < S1[4:0] + S0[4:0].
        36 => {
            let (size, start) = (a % 32, b % 32);
            let value = (0..32)
                .filter(|&i| i >= start && i < start + size)
                .fold(0, |d, i| d | 1 << i);
            (value, false)
        }
        // Guide 12.1, opcodes 50-52: copy the selected source bits into
        // the low (S0) and high (S1) halves, with no SCC write.
        50..=52 => {
            let start0 = if opcode == 52 { 16 } else { 0 };
            let start1 = if opcode == 50 { 0 } else { 16 };
            let mut value = 0;
            for i in 0..16 {
                value |= u32::from(bit(a, start0 + i)) << i;
                value |= u32::from(bit(b, start1 + i)) << (16 + i);
            }
            (value, false)
        }
        // MUL_I32 and MUL_HI: the 64-bit unsigned product by shift and add;
        // the signed high word subtracts S1 when S0 < 0 and S0 when S1 < 0
        // (two's complement correction).
        38 | 53 | 54 => {
            let product = (0..32)
                .filter(|&i| bit(b, i))
                .fold(0_u64, |sum, i| sum + (u64::from(a) << i));
            let (low, mut high) = (product as u32, (product >> 32) as u32);
            if opcode == 54 {
                if bit(a, 31) {
                    high = ripple(high, !b, true).0;
                }
                if bit(b, 31) {
                    high = ripple(high, !a, true).0;
                }
            }
            (if opcode == 38 { low } else { high }, false)
        }
        // BFE: D[i] = S0[offset + i] for i < width (offset S1[4:0], width
        // S1[22:16]); positions past bit 31 read 0 (U32) or S0[31] (I32), and
        // I32 copies D[width - 1] into the bits at and above width. SCC = D != 0.
        39 | 40 => {
            let (start, width) = (b % 32, (b >> 16) % 128);
            let source = |i: u32| match start + i {
                at @ 0..=31 => bit(a, at),
                _ => opcode == 40 && bit(a, 31),
            };
            let value = (0..32).fold(0, |d, i| {
                let set = if i < width {
                    source(i)
                } else {
                    opcode == 40 && width > 0 && source(width - 1)
                };
                d | u32::from(set) << i
            });
            (value, value != 0)
        }
        // ABSDIFF: the 32-bit S0 - S1, negated when bit 31 is set
        // (0x80000000 stays). SCC = D != 0.
        44 => {
            let difference = ripple(a, !b, true).0;
            let value = if bit(difference, 31) {
                ripple(!difference, 0, true).0
            } else {
                difference
            };
            (value, value != 0)
        }
        // LSHLn_ADD, n = 1-4: D = (S0 << n) + S1; SCC = 1 iff the exact sum
        // reaches 2^32, i.e. bits shifted out of S0 or a carry out of the add.
        46..=49 => {
            let n = opcode - 45;
            let (value, carry) = ripple(a << n, b, false);
            (value, carry || a >> (32 - n) != 0)
        }
        // RDNA 2 ISA guide 12.1: signed overflow from the sign bits,
        // add: S0[31] == S1[31] != D[31]; subtract: S0[31] != S1[31] != D[31].
        2 | 3 => {
            let value = if opcode == 2 {
                ripple(a, b, false).0
            } else {
                ripple(a, !b, true).0
            };
            let sign = |word: u32| word & 0x8000_0000 != 0;
            let same = sign(a) == sign(b);
            let overflow = (if opcode == 2 { same } else { !same }) && sign(a) != sign(value);
            (value, overflow)
        }
        4 => ripple(a, b, carry_in()?),
        // a - b - borrow = a + !b + (1 - borrow); borrow-out = !carry-out.
        1 | 5 => {
            let borrow = opcode == 5 && carry_in()?;
            let (value, carry) = ripple(a, !b, !borrow);
            (value, !carry)
        }
        // RDNA 2 ISA guide 12.1: the shifts use only S1[4:0], taken here as
        // the remainder modulo 32, and scale by a power of two in exact
        // arithmetic; ASHR is floor division of the signed S0. SCC = D != 0.
        30 | 32 | 34 => {
            let scale = 1_i64 << (b % 32);
            let value = match opcode {
                30 => (i64::from(a) * scale).rem_euclid(1 << 32),
                32 => i64::from(a) / scale,
                _ => i64::from(a as i32).div_euclid(scale).rem_euclid(1 << 32),
            } as u32;
            (value, value != 0)
        }
        _ => {
            let mut value = 0;
            for bit in 0..32 {
                let (x, y) = ((a >> bit) & 1 == 1, (b >> bit) & 1 == 1);
                let set = match opcode {
                    14 => x && y,
                    16 => x || y,
                    18 => x != y,
                    20 => x && !y,
                    22 => x || !y,
                    24 => !(x && y),
                    26 => !(x || y),
                    _ => x == y,
                };
                value |= u32::from(set) << bit;
            }
            (value, value != 0)
        }
    };
    sgpr[dst as usize] = Some(value);
    // CSELECT, BFM, PACK, MUL_I32 and MUL_HI have no SCC line in guide 12.1 ("n" in
    // Tables 11, 12 and 14): SCC stays as it was, possibly unset.
    if !matches!(opcode, 10 | 36 | 38 | 50..=54) {
        *scc = Some(flag);
    }
    Ok(())
}

/// RDNA 2 ISA guide 12.4: SCC = (S0 <relation> S1). Signed operands (opcodes
/// 0-5) are ordered by flipping both sign bits and comparing unsigned.
fn scalar_compare(opcode: u32, a: u32, b: u32) -> bool {
    let (a, b) = if opcode < 6 {
        (a ^ 0x8000_0000, b ^ 0x8000_0000)
    } else {
        (a, b)
    };
    match opcode % 6 {
        0 => a == b,
        1 => a != b,
        2 => a > b,
        3 => a >= b,
        4 => a < b,
        _ => a <= b,
    }
}

/// RDNA 2 ISA guide 12.3, evaluated with arithmetic on the value (no host bit
/// intrinsics): D and, for NOT, BCNT0, BCNT1 and ABS, SCC = (D != 0).
fn scalar_unary(opcode: u32, value: u32) -> (u32, Option<bool>) {
    // The bits from the LSB, as repeated halving produces them.
    let mut bits = [false; 32];
    let mut rest = u64::from(value);
    for slot in &mut bits {
        *slot = rest % 2 == 1;
        rest /= 2;
    }
    let word = |bits: &dyn Fn(usize) -> bool| {
        (0..32).rev().fold(0u64, |w, i| w * 2 + u64::from(bits(i))) as u32
    };
    let position = |found: Option<usize>| found.map_or(u32::MAX, |i| i as u32);
    let ones = bits.iter().filter(|&&b| b).count() as u32;
    let result = match opcode {
        7 => 0xffff_ffff - value,
        11 => word(&|i| bits[31 - i]),
        13 => 32 - ones,
        15 => ones,
        17 => position(bits.iter().position(|&b| !b)),
        19 => position(bits.iter().position(|&b| b)),
        21 => position(bits.iter().rev().position(|&b| b)),
        23 => position(
            bits.iter()
                .rev()
                .skip(1)
                .position(|&b| b != bits[31])
                .map(|i| i + 1),
        ),
        // signext of the low 8 or 16 bits: subtract 2^n when the value mod 2^n
        // is at least 2^(n-1).
        25 | 26 => {
            let span = if opcode == 25 { 256 } else { 65_536 };
            let low = i64::from(value) % span;
            let signed = if low >= span / 2 { low - span } else { low };
            signed.rem_euclid(1 << 32) as u32
        }
        // |S0| modulo 2^32, so INT_MIN stays 0x80000000.
        _ => i64::from(value as i32).abs().rem_euclid(1 << 32) as u32,
    };
    let flag = matches!(opcode, 7 | 13 | 15 | 52).then_some(result != 0);
    (result, flag)
}

fn read_vgpr(lane: &Lane, reg: u8, offset: usize) -> Result<u32, String> {
    if reg == 0 {
        return Err(format!("{offset:#x}: v0 is the resource index"));
    }
    if lane.pending_read == Some(reg) {
        return Err(format!("{offset:#x}: read before load wait"));
    }
    lane.vgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized v{reg}"))
}

/// Evaluate one VOPC compare for the whole wave and return the new VCC_LO.
fn compare(
    lanes: &[Lane],
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    (relation, signed, src0, src1): (u8, bool, CmpSrc, u8),
    offset: usize,
) -> Result<u32, String> {
    if execution_mask == 0 {
        // Encoding already validated; nothing is read (contract 5).
        return Ok(0);
    }
    let scalar_src0 = match src0 {
        CmpSrc::Sgpr(reg) => {
            Some(sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))?)
        }
        CmpSrc::Vcc => Some(sgpr[106].ok_or_else(|| format!("{offset:#x}: uninitialized vcc_lo"))?),
        CmpSrc::Imm(value) => Some(value),
        CmpSrc::Vgpr(_) => None,
    };
    // VGPR readiness is wave-uniform; validate it on every existing lane so
    // acceptance does not depend on which lanes are active.
    for lane in lanes {
        if let CmpSrc::Vgpr(reg) = src0 {
            read_vgpr(lane, reg, offset)?;
        }
        read_vgpr(lane, src1, offset)?;
    }
    let mut mask = 0u32;
    for (n, lane) in lanes.iter().enumerate() {
        if (execution_mask >> n) & 1 == 0 {
            continue;
        }
        let a = match (scalar_src0, src0) {
            (Some(value), _) => value,
            (None, CmpSrc::Vgpr(reg)) => read_vgpr(lane, reg, offset)?,
            (None, _) => unreachable!(),
        };
        let b = read_vgpr(lane, src1, offset)?;
        // Signed order equals unsigned order after flipping the sign bit.
        let (a, b) = if signed {
            (a ^ 0x8000_0000, b ^ 0x8000_0000)
        } else {
            (a, b)
        };
        let pass = match relation {
            1 => a < b,
            2 => a == b,
            3 => a <= b,
            4 => a > b,
            5 => a != b,
            _ => a >= b,
        };
        mask |= u32::from(pass) << n;
    }
    Ok(mask)
}

/// Static acceptance of the region skipped by a runtime VCC zero/nonzero branch
/// (contract 6), independent of lane data, EXEC and the branch outcome.
fn check_runtime_region(
    program: &[(usize, Insn)],
    pc: usize,
    target: usize,
    lane: &Lane,
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    offset: usize,
) -> Result<(), String> {
    // Contract 6 forbids an outstanding load; a pending store is waited for later.
    if lane.pending_read.is_some() {
        return Err(format!("{offset:#x}: load outstanding at a runtime branch"));
    }
    let end = index_of(program, target, offset)?;
    for &(at, insn) in &program[pc + 1..end] {
        // (destination, VGPR sources, SGPR source)
        let (dst, vgprs, scalar): (u8, Vec<u8>, Option<u8>) = match insn {
            Insn::VMov(dst, Src::Reg(reg)) => (dst, vec![reg], None),
            Insn::VMov(dst, Src::Imm(_)) => (dst, vec![], None),
            Insn::VMovFromSgpr(dst, src) => (dst, vec![], Some(src)),
            Insn::VMovFromVcc(dst) => (dst, vec![], Some(106)),
            Insn::VAddNc(dst, a, b) => (dst, vec![a, b], None),
            Insn::VAnd(dst, a, b)
            | Insn::VOr(dst, a, b)
            | Insn::VXor(dst, a, b)
            | Insn::VShiftLeft(dst, a, b)
            | Insn::VShiftRight(dst, a, b)
            | Insn::VShiftRightArithmetic(dst, a, b) => {
                let CmpSrc::Vgpr(a) = a else {
                    return Err(format!(
                        "{at:#x}: runtime VCC region requires VGPR SRC0 for legacy VOP2"
                    ));
                };
                (dst, vec![a, b], None)
            }
            _ => {
                return Err(format!(
                    "{at:#x}: instruction not allowed in a runtime branch region"
                ));
            }
        };
        if dst == 0 || lane.vgpr[dst as usize].is_none() {
            return Err(format!(
                "{at:#x}: runtime branch region writes an uninitialized VGPR"
            ));
        }
        // Sources are read only when some lane executes (contract 6).
        if execution_mask == 0 {
            continue;
        }
        for reg in vgprs {
            read_vgpr(lane, reg, at)?;
        }
        if let Some(reg) = scalar {
            sgpr[reg as usize].ok_or_else(|| format!("{at:#x}: uninitialized s{reg}"))?;
        }
    }
    Ok(())
}

/// One per-lane vector, memory or wait instruction; behaviour and errors match
/// the former independent per-lane interpreter.
fn lane_step(
    lane: &mut Lane,
    sgpr: &[Option<u32>; 128],
    execution_mask: u32,
    insn: Insn,
    offset: usize,
    lane_index: usize,
) -> Result<(), String> {
    let read_s = |reg: u8| -> Result<u32, String> {
        sgpr[reg as usize].ok_or_else(|| format!("{offset:#x}: uninitialized s{reg}"))
    };
    let read_vcc = || sgpr[106].ok_or_else(|| format!("{offset:#x}: uninitialized vcc_lo"));
    // New VOP3 forms capture every source before committing, including SRC2.
    let vop3 = match insn {
        Insn::VSaturate(_, d, a, b)
        | Insn::VAlias(_, d, a, b)
        | Insn::VMulLow(d, a, b)
        | Insn::VMulHighUnsigned(d, a, b)
        | Insn::VMask(d, a, b)
        | Insn::VCountAdd(d, a, b)
        | Insn::VMulHighSigned(d, a, b) => Some((d, [a, b, CmpSrc::Imm(0)])),
        Insn::VMadSad(_, _, d, a, b, c)
        | Insn::VLeastSigned(d, a, b, c)
        | Insn::VLeastUnsigned(d, a, b, c)
        | Insn::VGreatestSigned(d, a, b, c)
        | Insn::VGreatestUnsigned(d, a, b, c)
        | Insn::VMiddleSigned(d, a, b, c)
        | Insn::VMiddleUnsigned(d, a, b, c)
        | Insn::VSignedField(d, a, b, c)
        | Insn::VShiftThenSum(d, a, b, c)
        | Insn::VSumThenShift(d, a, b, c)
        | Insn::VShiftUnion(d, a, b, c)
        | Insn::VExtractUnsigned(d, a, b, c)
        | Insn::VAlignBits(d, a, b, c)
        | Insn::VOrThree(d, a, b, c)
        | Insn::VXorThree(d, a, b, c)
        | Insn::VBitSelect(d, a, b, c)
        | Insn::VAddThree(d, a, b, c)
        | Insn::VMad16(_, d, a, b, c)
        | Insn::VXad(d, a, b, c)
        | Insn::VLerp(d, a, b, c)
        | Insn::VMsad(d, a, b, c)
        | Insn::VAndOr(d, a, b, c) => Some((d, [a, b, c])),
        _ => None,
    };
    if let Some((dst, sources)) = vop3 {
        if execution_mask == 0 {
            return Ok(());
        }
        if execution_mask != u32::MAX && lane.vgpr[dst as usize].is_none() {
            return Err(format!("{offset:#x}: partial EXEC on uninitialized VGPR"));
        }
        let mut values = [0u32; 3];
        for (i, src) in sources.into_iter().enumerate() {
            values[i] = match src {
                CmpSrc::Sgpr(r) => read_s(r)?,
                CmpSrc::Vcc => read_vcc()?,
                CmpSrc::Vgpr(r) => read_vgpr(lane, r, offset)?,
                CmpSrc::Imm(v) => v,
            };
        }
        if execution_mask & (1 << lane_index) == 0 {
            return Ok(());
        }
        check_destination(dst, lane.pending_read, offset)?;
        let [a, b, c] = values;
        let result = match insn {
            Insn::VMulLow(..) | Insn::VMulHighUnsigned(..) | Insn::VMulHighSigned(..) => {
                let (a0, a1, b0, b1) = (a % 65536, a / 65536, b % 65536, b / 65536);
                let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
                let middle = p00 / 65536 + p01 % 65536 + p10 % 65536;
                let high = p11 + p01 / 65536 + p10 / 65536 + middle / 65536;
                match insn {
                    Insn::VMulLow(..) => p00 % 65536 + 65536 * (middle % 65536),
                    Insn::VMulHighUnsigned(..) => high,
                    _ => high
                        .wrapping_sub(if a >= 0x80000000 { b } else { 0 })
                        .wrapping_sub(if b >= 0x80000000 { a } else { 0 }),
                }
            }
            Insn::VMadSad(kind, clamp, ..) => {
                let total = if kind == 322 || kind == 323 {
                    let product = multiply_field(a, b, 24, kind == 322);
                    if kind == 322 && c >= 0x80000000 {
                        product - (0x100000000i128 - i128::from(c))
                    } else {
                        product + i128::from(c)
                    }
                } else {
                    let component_bits = match kind {
                        346 | 347 => 8,
                        348 => 16,
                        _ => 32,
                    };
                    let radix = 2u64.pow(component_bits);
                    let mut left = u64::from(a);
                    let mut right = u64::from(b);
                    let mut sum = 0i128;
                    for _ in 0..32 / component_bits {
                        let x = (left % radix) as u32;
                        let y = (right % radix) as u32;
                        let difference = if x >= y {
                            ripple(x, !y, true).0
                        } else {
                            ripple(y, !x, true).0
                        };
                        sum += i128::from(difference);
                        left /= radix;
                        right /= radix;
                    }
                    sum * (if kind == 347 { 65536 } else { 1 }) + i128::from(c)
                };
                if !clamp {
                    total.rem_euclid(0x100000000) as u32
                } else if kind == 322 {
                    if total < -2147483648 {
                        0x80000000
                    } else if total > 2147483647 {
                        0x7fffffff
                    } else {
                        total as u32
                    }
                } else if total > 4294967295 {
                    u32::MAX
                } else {
                    total as u32
                }
            }
            Insn::VSaturate(kind, ..) => match kind {
                265 | 267 => {
                    let (fa, fb) = (a % 0x1000000, b % 0x1000000);
                    let na = kind == 265 && fa >= 0x800000;
                    let nb = kind == 265 && fb >= 0x800000;
                    let ma = if na { 0x1000000 - fa } else { fa };
                    let mb = if nb { 0x1000000 - fb } else { fb };
                    let negative = na != nb;
                    let limit = if kind == 267 {
                        u32::MAX
                    } else if negative {
                        0x80000000
                    } else {
                        0x7fffffff
                    };
                    // Check magnitude against the quotient before multiplying.
                    let magnitude = if mb != 0 && ma > limit / mb {
                        limit
                    } else {
                        ma * mb
                    };
                    if negative {
                        ripple(!magnitude, 0, true).0
                    } else {
                        magnitude
                    }
                }
                293 => {
                    let (sum, carry) = ripple(a, b, false);
                    if carry { u32::MAX } else { sum }
                }
                294 => {
                    if a < b {
                        0
                    } else {
                        ripple(a, !b, true).0
                    }
                }
                295 => {
                    if b < a {
                        0
                    } else {
                        ripple(b, !a, true).0
                    }
                }
                _ => unreachable!(),
            },
            Insn::VAlias(kind, ..) => match kind {
                22 => b / 2u32.pow(a % 32),
                24 => i64::from(b as i32).div_euclid(2i64.pow(a % 32)) as u32,
                26 => (u64::from(b) * 2u64.pow(a % 32)) as u32,
                27 => a & b,
                28 => a | b,
                29 => a ^ b,
                _ => vector_integer(kind, a, b),
            },
            Insn::VBitSelect(..) => {
                let mut out = 0;
                for bit in 0..32 {
                    let weight = 2u32.pow(bit);
                    let selected = if (a / weight) % 2 != 0 { b } else { c };
                    out += ((selected / weight) % 2) * weight;
                }
                out
            }
            Insn::VAddThree(..) => {
                let low = a % 65536 + b % 65536 + c % 65536;
                let high = a / 65536 + b / 65536 + c / 65536 + low / 65536;
                low % 65536 + (high % 65536) * 65536
            }
            Insn::VLeastSigned(..)
            | Insn::VLeastUnsigned(..)
            | Insn::VGreatestSigned(..)
            | Insn::VGreatestUnsigned(..)
            | Insn::VMiddleSigned(..)
            | Insn::VMiddleUnsigned(..) => {
                // Bias only comparison keys. Select original words and use
                // inclusive ranks so equal inputs retain their repeated value.
                let bias = if matches!(
                    insn,
                    Insn::VLeastSigned(..) | Insn::VGreatestSigned(..) | Insn::VMiddleSigned(..)
                ) {
                    0x80000000
                } else {
                    0
                };
                let (ka, kb, kc) = (a ^ bias, b ^ bias, c ^ bias);
                match insn {
                    Insn::VLeastSigned(..) | Insn::VLeastUnsigned(..) => {
                        if ka <= kb && ka <= kc {
                            a
                        } else if kb <= kc {
                            b
                        } else {
                            c
                        }
                    }
                    Insn::VGreatestSigned(..) | Insn::VGreatestUnsigned(..) => {
                        if ka >= kb && ka >= kc {
                            a
                        } else if kb >= kc {
                            b
                        } else {
                            c
                        }
                    }
                    _ => {
                        if (kb <= ka && ka <= kc) || (kc <= ka && ka <= kb) {
                            a
                        } else if (ka <= kb && kb <= kc) || (kc <= kb && kb <= ka) {
                            b
                        } else {
                            c
                        }
                    }
                }
            }
            Insn::VSignedField(..) => {
                let start = b % 32;
                let available = (c % 32).min(32 - start);
                let mut out = 0;
                if available != 0 {
                    for bit in 0..32 {
                        let source_bit = start + bit.min(available - 1);
                        if !(a / 2u32.pow(source_bit)).is_multiple_of(2) {
                            out |= 2u32.pow(bit);
                        }
                    }
                }
                out
            }
            Insn::VShiftThenSum(..) => {
                let shifted = (u64::from(a) * 2u64.pow(b % 32)) as u32;
                ripple(shifted, c, false).0
            }
            Insn::VSumThenShift(..) => (u64::from(ripple(a, b, false).0) * 2u64.pow(c % 32)) as u32,
            Insn::VShiftUnion(..) => {
                let mut out = c;
                for bit in b % 32..32 {
                    if !(a / 2u32.pow(bit - b % 32)).is_multiple_of(2) {
                        out |= 2u32.pow(bit);
                    }
                }
                out
            }
            // Independent arithmetic decomposition of AMD packed-byte definitions.
            Insn::VMad16(signed, ..) => {
                let half = |word: u32| {
                    let low = i64::from(word % 65536);
                    if signed && low >= 32768 {
                        low - 65536
                    } else {
                        low
                    }
                };
                (half(a) * half(b) + i64::from(c)).rem_euclid(4294967296) as u32
            }
            Insn::VXad(..) => ripple(a ^ b, c, false).0,
            Insn::VLerp(..) => {
                let mut packed = 0u64;
                for component in 0..4 {
                    let scale = 256u64.pow(component);
                    let x = (u64::from(a) / scale) % 256;
                    let y = (u64::from(b) / scale) % 256;
                    let round = (u64::from(c) / scale) % 2;
                    packed += ((x + y + round) / 2) * scale;
                }
                packed as u32
            }
            Insn::VMsad(..) => {
                let mut total = i64::from(c);
                for component in 0..4 {
                    let scale = 256u32.pow(component);
                    let reference = (b / scale) % 256;
                    if reference != 0 {
                        total += (i64::from((a / scale) % 256) - i64::from(reference)).abs();
                    }
                }
                (total % 4294967296) as u32
            }
            Insn::VAndOr(..) => !((!a | !b) & !c),
            Insn::VCountAdd(..) => {
                let mut rest = a;
                let mut count = 0;
                while rest != 0 {
                    rest &= rest - 1;
                    count += 1;
                }
                ripple(b, count, false).0
            }
            Insn::VExtractUnsigned(..)
            | Insn::VMask(..)
            | Insn::VAlignBits(..)
            | Insn::VOrThree(..)
            | Insn::VXorThree(..) => {
                let mut out = 0;
                for j in 0..32 {
                    let weight = 2u32.pow(j);
                    let bit = match insn {
                        Insn::VExtractUnsigned(..) => {
                            j < c % 32
                                && j + b % 32 < 32
                                && !(a / 2u32.pow(j + b % 32)).is_multiple_of(2)
                        }
                        Insn::VMask(..) => j >= b % 32 && j - b % 32 < a % 32,
                        Insn::VAlignBits(..) => {
                            let k = j + c % 32;
                            if k < 32 {
                                (b / 2u32.pow(k)) % 2 != 0
                            } else {
                                !(a / 2u32.pow(k - 32)).is_multiple_of(2)
                            }
                        }
                        Insn::VOrThree(..) => {
                            (a / weight) % 2 + (b / weight) % 2 + (c / weight) % 2 != 0
                        }
                        _ => ((a / weight) % 2 + (b / weight) % 2 + (c / weight) % 2) % 2 != 0,
                    };
                    if bit {
                        out += weight;
                    }
                }
                out
            }
            _ => unreachable!(),
        };
        lane.vgpr[dst as usize] = Some(result);
        return Ok(());
    }
    if execution_mask.count_ones() < 32 {
        match insn {
            Insn::Load(..) | Insn::Store(_) => {
                return Err(format!(
                    "{offset:#x}: incomplete EXEC on a buffer instruction"
                ));
            }
            Insn::VMov(dst, _)
            | Insn::VMovFromSgpr(dst, _)
            | Insn::VMovFromVcc(dst)
            | Insn::VAddNc(dst, ..)
            | Insn::VAnd(dst, ..)
            | Insn::VOr(dst, ..)
            | Insn::VXor(dst, ..)
            | Insn::VShiftLeft(dst, ..)
            | Insn::VShiftRight(dst, ..)
            | Insn::VShiftRightArithmetic(dst, ..)
            | Insn::VInt(_, dst, ..)
            | Insn::VUnary(_, dst, _)
            | Insn::VAddF(dst, ..)
            | Insn::VMulF(dst, ..) => {
                if execution_mask != 0 && lane.vgpr[usize::from(dst)].is_none() {
                    return Err(format!("{offset:#x}: partial EXEC on uninitialized VGPR"));
                }
                if execution_mask != 0 {
                    match insn {
                        Insn::VMovFromSgpr(_, src) => {
                            read_s(src)?;
                        }
                        Insn::VMovFromVcc(_) => {
                            read_vcc()?;
                        }
                        Insn::VUnary(_, _, src) => match src {
                            CmpSrc::Sgpr(reg) => {
                                read_s(reg)?;
                            }
                            CmpSrc::Vcc => {
                                read_vcc()?;
                            }
                            CmpSrc::Vgpr(reg) => {
                                read_vgpr(lane, reg, offset)?;
                            }
                            CmpSrc::Imm(_) => {}
                        },
                        // Sources must be readable even in an inactive lane.
                        Insn::VInt(_, _, src0, src1)
                        | Insn::VAnd(_, src0, src1)
                        | Insn::VOr(_, src0, src1)
                        | Insn::VXor(_, src0, src1)
                        | Insn::VShiftLeft(_, src0, src1)
                        | Insn::VShiftRight(_, src0, src1)
                        | Insn::VShiftRightArithmetic(_, src0, src1) => {
                            match src0 {
                                CmpSrc::Sgpr(reg) => {
                                    read_s(reg)?;
                                }
                                CmpSrc::Vcc => {
                                    read_vcc()?;
                                }
                                CmpSrc::Vgpr(reg) => {
                                    read_vgpr(lane, reg, offset)?;
                                }
                                CmpSrc::Imm(_) => {}
                            }
                            read_vgpr(lane, src1, offset)?;
                        }
                        _ => {}
                    }
                }
                if (execution_mask.rotate_right(lane_index as u32) & 1) == 0 {
                    return Ok(());
                }
            }
            _ => {}
        }
    }

    let read_v = |reg: u8| read_vgpr(lane, reg, offset);
    let read_legacy_source = |source| match source {
        CmpSrc::Sgpr(reg) => read_s(reg),
        CmpSrc::Vcc => read_vcc(),
        CmpSrc::Vgpr(reg) => read_v(reg),
        CmpSrc::Imm(value) => Ok(value),
    };
    let pending_read = lane.pending_read;
    let value = match insn {
        Insn::Load(data, width, signed) => {
            if data == 0 || pending_read.is_some() {
                return Err(format!("{offset:#x}: invalid or overlapping load"));
            }
            // Reconstruct the loaded low bits individually; replicate the sign
            // bit only above the loaded width. Original fixed four-byte stride.
            let sign = signed && lane.input & (1 << (width - 1)) != 0;
            lane.pending_value = (0..32).fold(0, |value, bit| {
                let set = if bit < width {
                    lane.input & (1 << bit) != 0
                } else {
                    sign
                };
                value | (u32::from(set) << bit)
            });
            lane.pending_read = Some(data);
            return Ok(());
        }
        Insn::Store(data) => {
            let value = read_v(data)?;
            if lane.pending_write.is_some() || lane.written.is_some() {
                return Err(format!("{offset:#x}: multiple output writes"));
            }
            lane.pending_write = Some(value);
            return Ok(());
        }
        Insn::WaitRead => {
            if let Some(dst) = lane.pending_read.take() {
                lane.vgpr[dst as usize] = Some(lane.pending_value);
            }
            return Ok(());
        }
        Insn::WaitWrite => {
            if let Some(value) = lane.pending_write.take() {
                lane.written = Some(value);
            }
            return Ok(());
        }
        Insn::VMov(dst, src) => {
            let value = match src {
                Src::Imm(value) => value,
                Src::Reg(reg) => read_v(reg)?,
            };
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VMovFromSgpr(dst, src) => {
            let value = read_s(src)?;
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VMovFromVcc(dst) => {
            let value = read_vcc()?;
            check_destination(dst, pending_read, offset)?;
            (dst, value)
        }
        Insn::VAddNc(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            let sum = u64::from(read_v(a)?) + u64::from(read_v(b)?);
            (dst, sum as u32)
        }
        Insn::VAnd(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_legacy_source(a)? & read_v(b)?)
        }
        Insn::VOr(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_legacy_source(a)? | read_v(b)?)
        }
        Insn::VShiftLeft(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            let factor = 2_u64.pow(read_legacy_source(count)? % 32);
            let product = u64::from(read_v(value)?) * factor;
            (dst, (product % (1_u64 << 32)) as u32)
        }
        Insn::VShiftRight(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            let divisor = 2_u32.pow(read_legacy_source(count)? % 32);
            (dst, read_v(value)? / divisor)
        }
        Insn::VShiftRightArithmetic(dst, count, value) => {
            check_destination(dst, pending_read, offset)?;
            // Floor division of the sign-extended value, reduced modulo
            // 2^32. This is arithmetic shift without using `>>`.
            let shifted = i64::from(read_v(value)? as i32)
                .div_euclid(1_i64 << (read_legacy_source(count)? % 32));
            (dst, shifted.rem_euclid(1_i64 << 32) as u32)
        }
        Insn::VXor(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, read_legacy_source(a)? ^ read_v(b)?)
        }
        Insn::VUnary(operation, dst, src) => {
            check_destination(dst, pending_read, offset)?;
            let input = match src {
                CmpSrc::Sgpr(reg) => read_s(reg)?,
                CmpSrc::Vcc => read_vcc()?,
                CmpSrc::Vgpr(reg) => read_v(reg)?,
                CmpSrc::Imm(bits) => bits,
            };
            (dst, operation.result(input))
        }
        Insn::VInt(opcode, dst, src0, src1) => {
            check_destination(dst, pending_read, offset)?;
            let a = match src0 {
                CmpSrc::Sgpr(reg) => read_s(reg)?,
                CmpSrc::Vcc => read_vcc()?,
                CmpSrc::Imm(value) => value,
                CmpSrc::Vgpr(reg) => read_v(reg)?,
            };
            (dst, vector_integer(opcode, a, read_v(src1)?))
        }
        Insn::VAddF(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, exact_float(true, read_v(a)?, read_v(b)?, offset)?)
        }
        Insn::VMulF(dst, a, b) => {
            check_destination(dst, pending_read, offset)?;
            (dst, exact_float(false, read_v(a)?, read_v(b)?, offset)?)
        }
        _ => unreachable!("scalar instruction in lane_step"),
    };
    lane.vgpr[value.0 as usize] = Some(value.1);
    Ok(())
}

/// D of VOP2 integer opcode `opcode` (guide 12.7) for S0 = `a`, S1 = `b`.
fn vector_integer(opcode: u32, a: u32, b: u32) -> u32 {
    // Signed order equals unsigned order after flipping the sign bit.
    let below = |signed: bool| {
        let flip = if signed { 0x8000_0000 } else { 0 };
        (a ^ flip) < (b ^ flip)
    };
    match opcode {
        9 => product_word(multiply_field(a, b, 24, true), false),
        10 => product_word(multiply_field(a, b, 24, true), true),
        11 => product_word(multiply_field(a, b, 24, false), false),
        12 => product_word(multiply_field(a, b, 24, false), true),
        17 | 19 => {
            if below(opcode == 17) {
                a
            } else {
                b
            }
        }
        18 | 20 => {
            if below(opcode == 18) {
                b
            } else {
                a
            }
        }
        30 => (a & b) | (!a & !b),
        37 => ripple(a, b, false).0,
        38 => ripple(a, !b, true).0,
        _ => ripple(b, !a, true).0,
    }
}

/// The exact product of the low `width` bits of `a` and `b`, read as two's
/// complement when `signed`, by shift-and-add on the magnitudes.
fn multiply_field(a: u32, b: u32, width: u32, signed: bool) -> i128 {
    let read = |value: u32| {
        let low = i128::from(value) % (1_i128 << width);
        if signed && low >= 1_i128 << (width - 1) {
            low - (1_i128 << width)
        } else {
            low
        }
    };
    let (x, y) = (read(a), read(b));
    let (mx, my) = (x.unsigned_abs(), y.unsigned_abs());
    let mut magnitude = 0_u128;
    for bit in 0..width {
        if (my >> bit) & 1 == 1 {
            magnitude += mx << bit;
        }
    }
    let magnitude = magnitude as i128;
    if (x < 0) != (y < 0) {
        -magnitude
    } else {
        magnitude
    }
}

/// Bits 31:0 (or 63:32 when `high`) of `product` in 64-bit two's complement,
/// by Euclidean division rather than a shift.
fn product_word(product: i128, high: bool) -> u32 {
    let word = 1_i128 << 32;
    let part = if high {
        product.div_euclid(word)
    } else {
        product
    };
    part.rem_euclid(word) as u32
}

/// Bit-serial addition with carry-in; returns the sum and the carry-out.
fn ripple(a: u32, b: u32, carry_in: bool) -> (u32, bool) {
    let mut carry = carry_in;
    let mut sum = 0;
    for bit in 0..32 {
        let (x, y) = ((a >> bit) & 1 == 1, (b >> bit) & 1 == 1);
        sum |= u32::from(x ^ y ^ carry) << bit;
        carry = (x && y) || (carry && (x ^ y));
    }
    (sum, carry)
}

fn check_destination(dst: u8, load: Option<u8>, offset: usize) -> Result<(), String> {
    if dst == 0 || load == Some(dst) {
        return Err(format!("{offset:#x}: write to the index or pending VGPR"));
    }
    Ok(())
}

fn index_of(program: &[(usize, Insn)], target: usize, offset: usize) -> Result<usize, String> {
    program
        .iter()
        .position(|(at, _)| *at == target)
        .ok_or_else(|| format!("{offset:#x}: branch target {target:#x} is not a boundary"))
}

fn exact_float(add: bool, a_bits: u32, b_bits: u32, offset: usize) -> Result<u32, String> {
    fn unpack(bits: u32) -> Option<(u64, i32)> {
        let biased = ((bits >> 23) & 0xff) as i32;
        if bits >> 31 != 0 || !(1..=254).contains(&biased) {
            return None;
        }
        let significand = (1 << 23) | (bits & 0x007f_ffff);
        Some((u64::from(significand), biased - 127 - 23))
    }

    let operand_error = || format!("{offset:#x}: float outside positive normal domain");
    let (a, scale_a) = unpack(a_bits).ok_or_else(operand_error)?;
    let (b, scale_b) = unpack(b_bits).ok_or_else(operand_error)?;
    let result_error = || format!("{offset:#x}: float result not exact normal binary32");
    let (integer, scale) = if add {
        let scale = scale_a.min(scale_b);
        // A nonzero operand at least 24 exponents lower is strictly
        // less than one ULP of the larger operand and cannot sum exactly.
        if scale_a.abs_diff(scale_b) >= 24 {
            return Err(result_error());
        }
        ((a << (scale_a - scale)) + (b << (scale_b - scale)), scale)
    } else {
        // Two 24-bit significands multiply exactly in an unsigned 64-bit word.
        (a * b, scale_a + scale_b)
    };

    let leading = integer.ilog2();
    let trailing = integer.trailing_zeros();
    let exponent = scale + leading as i32;
    let significant_width = leading - trailing + 1;
    if significant_width > 24 || !(-126..=127).contains(&exponent) {
        return Err(result_error());
    }
    let normalized = ((integer >> trailing) << (24 - significant_width)) as u32;
    Ok((((exponent + 127) as u32) << 23) | (normalized & 0x007f_ffff))
}

fn decode(code: &[u8]) -> Result<Vec<(usize, Insn)>, String> {
    let mut program = Vec::new();
    let mut offset = 0;
    while offset < code.len() {
        let (insn, len) = decode_one(code, offset)?;
        if matches!(insn, Insn::End) && offset + len != code.len() {
            return Err(format!("{offset:#x}: code after s_endpgm"));
        }
        program.push((offset, insn));
        offset += len;
    }
    for &(at, insn) in &program {
        if let Insn::Branch(target)
        | Insn::BranchScc0(target)
        | Insn::BranchScc1(target)
        | Insn::BranchMask { target, .. } = insn
            && !program.iter().any(|(boundary, _)| *boundary == target)
        {
            return Err(format!(
                "{at:#x}: branch target {target:#x} is not a boundary"
            ));
        }
    }
    Ok(program)
}

fn decode_one(code: &[u8], offset: usize) -> Result<(Insn, usize), String> {
    let first = word(code, offset)?;
    let fail = |detail: &str| format!("{offset:#x}: {detail}");
    let opcode = (first / 65536) % 1024;
    if first >> 26 == 53
        && matches!(
            opcode,
            361 | 362
                | 364
                | 330
                | 877
                | 881
                | 328
                | 867
                | 334
                | 868
                | 882
                | 376
                | 329
                | 838
                | 839
                | 879
                | 883
                | 885
                | 837
                | 333
                | 369
                | 338
                | 339
                | 341
                | 342
                | 344
                | 345
                | 265
                | 266
                | 267
                | 268
                | 273
                | 274
                | 275
                | 276
                | 286
                | 293
                | 294
                | 295
                | 282
                | 278
                | 280
                | 283
                | 284
                | 285
                | 439
                | 440
                | 441
                | 442
                | 443
                | 322
                | 323
                | 346
                | 347
                | 348
                | 349
        )
    {
        let next = word(code, offset + 4).map_err(|_| fail("truncated VOP3 instruction"))?;
        let clamp = (first / 32768) % 2 != 0;
        if (first / 256) % 128 != 0
            || next / (1 << 27) != 0
            || (clamp
                && !matches!(
                    opcode,
                    265 | 267 | 293 | 294 | 295 | 322 | 323 | 346 | 347 | 348 | 349
                ))
        {
            return Err(fail("unsupported VOP3 modifiers"));
        }
        let raw = [next % 512, (next / 512) % 512, (next / 262144) % 512];
        let count = if matches!(opcode, 439..=443) {
            1
        } else if matches!(
            opcode,
            361 | 362
                | 364
                | 867
                | 868
                | 265
                | 266
                | 267
                | 268
                | 273
                | 274
                | 275
                | 276
                | 286
                | 293
                | 294
                | 295
                | 282
                | 278
                | 280
                | 283
                | 284
                | 285
        ) {
            2
        } else {
            3
        };
        if count == 1 && (raw[1] != 128 || raw[2] != 128) {
            return Err(fail("noncanonical VOP3 unary padding"));
        }
        if count == 2 && raw[2] != 128 {
            return Err(fail("noncanonical VOP3 binary SRC2"));
        }
        let mut identities = 0;
        for i in 0..count {
            let selector = raw[i];
            if !matches!(selector,0..=106|128..=208|240..=248|255..=511) {
                return Err(fail("bad VOP3 source"));
            }
            if (selector <= 106 || selector == 255) && !raw[..i].contains(&selector) {
                identities += 1;
            }
        }
        if identities > 2 {
            return Err(fail("VOP3 scalar bus exceeds two distinct identities"));
        }
        let has_literal = raw[..count].contains(&255);
        let literal = if has_literal {
            word(code, offset + 8).map_err(|_| fail("truncated VOP3 literal"))?
        } else {
            0
        };
        // Deliberate subset restrictions; no claim these encodings are invalid AMD forms.
        if matches!(opcode, 883 | 885)
            && (raw[..2].iter().any(|x| (240..=248).contains(x))
                || (raw[..2].contains(&255) && literal >= 65536))
        {
            return Err(fail("unsupported MAD16 multiplicand constant"));
        }
        let mut args = [CmpSrc::Imm(0); 3];
        for i in 0..count {
            args[i] = match raw[i] {
                0..=105 => CmpSrc::Sgpr(raw[i] as u8),
                106 => CmpSrc::Vcc,
                255 => CmpSrc::Imm(literal),
                other => match vector_source(other).unwrap() {
                    Src::Reg(v) => CmpSrc::Vgpr(v),
                    Src::Imm(bits) => CmpSrc::Imm(bits),
                },
            };
        }
        let dst = first as u8;
        if matches!(opcode, 322 | 323 | 346 | 347 | 348 | 349) {
            return Ok((
                Insn::VMadSad(opcode, clamp, dst, args[0], args[1], args[2]),
                if has_literal { 12 } else { 8 },
            ));
        }
        if clamp {
            return Ok((
                Insn::VSaturate(opcode, dst, args[0], args[1]),
                if has_literal { 12 } else { 8 },
            ));
        }
        let insn = match opcode {
            293 if raw[0] >= 256 && raw[1] >= 256 => Insn::VAddNc(dst, raw[0] as u8, raw[1] as u8),
            265 => Insn::VAlias(9, dst, args[0], args[1]),
            266 => Insn::VAlias(10, dst, args[0], args[1]),
            267 => Insn::VAlias(11, dst, args[0], args[1]),
            268 => Insn::VAlias(12, dst, args[0], args[1]),
            273 => Insn::VAlias(17, dst, args[0], args[1]),
            274 => Insn::VAlias(18, dst, args[0], args[1]),
            275 => Insn::VAlias(19, dst, args[0], args[1]),
            276 => Insn::VAlias(20, dst, args[0], args[1]),
            286 => Insn::VAlias(30, dst, args[0], args[1]),
            293 => Insn::VAlias(37, dst, args[0], args[1]),
            294 => Insn::VAlias(38, dst, args[0], args[1]),
            295 => Insn::VAlias(39, dst, args[0], args[1]),
            282 => Insn::VAlias(26, dst, args[0], args[1]),
            278 => Insn::VAlias(22, dst, args[0], args[1]),
            280 => Insn::VAlias(24, dst, args[0], args[1]),
            283 => Insn::VAlias(27, dst, args[0], args[1]),
            284 => Insn::VAlias(28, dst, args[0], args[1]),
            285 => Insn::VAlias(29, dst, args[0], args[1]),

            439 => Insn::VUnary(Unary::Complement, dst, args[0]),
            440 => Insn::VUnary(Unary::Mirror, dst, args[0]),
            441 => Insn::VUnary(Unary::High, dst, args[0]),
            442 => Insn::VUnary(Unary::Low, dst, args[0]),
            443 => Insn::VUnary(Unary::SignedHigh, dst, args[0]),

            361 => Insn::VMulLow(dst, args[0], args[1]),
            362 => Insn::VMulHighUnsigned(dst, args[0], args[1]),
            364 => Insn::VMulHighSigned(dst, args[0], args[1]),
            330 => Insn::VBitSelect(dst, args[0], args[1], args[2]),
            877 => Insn::VAddThree(dst, args[0], args[1], args[2]),
            881 => Insn::VAndOr(dst, args[0], args[1], args[2]),
            338 => Insn::VLeastSigned(dst, args[0], args[1], args[2]),
            339 => Insn::VLeastUnsigned(dst, args[0], args[1], args[2]),
            341 => Insn::VGreatestSigned(dst, args[0], args[1], args[2]),
            342 => Insn::VGreatestUnsigned(dst, args[0], args[1], args[2]),
            344 => Insn::VMiddleSigned(dst, args[0], args[1], args[2]),
            345 => Insn::VMiddleUnsigned(dst, args[0], args[1], args[2]),
            329 => Insn::VSignedField(dst, args[0], args[1], args[2]),
            883 | 885 => Insn::VMad16(opcode == 885, dst, args[0], args[1], args[2]),
            837 => Insn::VXad(dst, args[0], args[1], args[2]),
            333 => Insn::VLerp(dst, args[0], args[1], args[2]),
            369 => Insn::VMsad(dst, args[0], args[1], args[2]),
            838 => Insn::VShiftThenSum(dst, args[0], args[1], args[2]),
            839 => Insn::VSumThenShift(dst, args[0], args[1], args[2]),
            879 => Insn::VShiftUnion(dst, args[0], args[1], args[2]),
            328 => Insn::VExtractUnsigned(dst, args[0], args[1], args[2]),
            867 => Insn::VMask(dst, args[0], args[1]),
            334 => Insn::VAlignBits(dst, args[0], args[1], args[2]),
            868 => Insn::VCountAdd(dst, args[0], args[1]),
            882 => Insn::VOrThree(dst, args[0], args[1], args[2]),
            376 => Insn::VXorThree(dst, args[0], args[1], args[2]),
            _ => return Err(fail("unsupported VOP3 integer opcode")),
        };
        return Ok((insn, if has_literal { 12 } else { 8 }));
    }

    // The synthetic ABI permits only idxen with zero offsets. Only the
    // data VGPR varies across each complete eight-byte encoding.
    if matches!(
        first,
        0xe020_2000 | 0xe024_2000 | 0xe028_2000 | 0xe02c_2000 | 0xe030_2000 | 0xe070_2000
    ) {
        let load = first != 0xe070_2000;
        let expected = if load { 0x8000_0000 } else { 0x8001_0000 };
        let second = word(code, offset + 4)?;
        if second & 0xffff_00ff != expected {
            return Err(fail("buffer operand outside synthetic ABI"));
        }
        let data = (second >> 8) as u8;
        return Ok((
            if load {
                let (width, signed) = match first {
                    0xe020_2000 => (8, false),
                    0xe024_2000 => (8, true),
                    0xe028_2000 => (16, false),
                    0xe02c_2000 => (16, true),
                    _ => (32, false),
                };
                Insn::Load(data, width, signed)
            } else {
                Insn::Store(data)
            },
            8,
        ));
    }
    if first & 0xfe000000 == 0x7e000000 && (55..=59).contains(&((first >> 9) & 255)) {
        let operation = match (first >> 9) & 255 {
            55 => Unary::Complement,
            56 => Unary::Mirror,
            57 => Unary::High,
            58 => Unary::Low,
            _ => Unary::SignedHigh,
        };
        let (src, len) = vector_src0(code, offset, first, "VOP1 integer")?;
        return Ok((Insn::VUnary(operation, (first >> 17) as u8, src), len));
    }
    if first & 0xfe01_fe00 == 0x7e00_0200 {
        let dst = (first >> 17) as u8;
        let selector = first & 0x1ff;
        if selector <= 105 {
            return Ok((Insn::VMovFromSgpr(dst, selector as u8), 4));
        }
        if selector == 106 {
            return Ok((Insn::VMovFromVcc(dst), 4));
        }
        let (src, len) = if selector == 255 {
            (Src::Imm(word(code, offset + 4)?), 8)
        } else {
            (
                vector_source(selector).ok_or_else(|| fail("bad VOP1 source"))?,
                4,
            )
        };
        return Ok((Insn::VMov(dst, src), len));
    }
    // VOPC: bits 31:25 = 0b0111110, OP 24:17, VSRC1 16:9, SRC0 8:0.
    if first >> 25 == 0b011_1110 {
        let opcode = (first >> 17) & 0xff;
        let (signed, relation) = match opcode {
            129..=134 => (true, opcode - 128),
            193..=198 => (false, opcode - 192),
            _ => return Err(fail("unsupported VOPC opcode")),
        };
        let (src0, len) = vector_src0(code, offset, first, "VOPC")?;
        let insn = Insn::VCmp {
            relation: relation as u8,
            signed,
            src0,
            src1: (first >> 9) as u8,
        };
        return Ok((insn, len));
    }
    // VOP2 integer forms: bit 31 = 0, OP 30:25, VDST 24:17, VSRC1 16:9, SRC0 8:0.
    let vop2_op = (first >> 25) & 0x3f;
    if first >> 31 == 0 && matches!(vop2_op, 9..=12 | 17..=20 | 30 | 37..=39) {
        let (src0, len) = vector_src0(code, offset, first, "VOP2")?;
        let insn = Insn::VInt(vop2_op, (first >> 17) as u8, src0, (first >> 9) as u8);
        return Ok((insn, len));
    }
    if first & 0xffff_0000 == 0xd525_0000 {
        let second = word(code, offset + 4)?;
        let src0 = second & 0x1ff;
        let src1 = (second >> 9) & 0x1ff;
        if first & 0xff00 != 0 || second >> 18 != 128 || src0 < 256 || src1 < 256 {
            return Err(fail("bad VOP3 add form"));
        }
        return Ok((Insn::VAddNc(first as u8, src0 as u8, src1 as u8), 8));
    }
    let bit_opcode = (first >> 8) & 255;
    if first >> 23 == 0b1_0111_1101 && matches!(bit_opcode, 5 | 27 | 29) {
        let dst = ((first >> 16) & 127) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOP1 destination"));
        }
        let (src, len) = scalar_source(code, offset, first as u8)?;
        return Ok((
            if bit_opcode == 5 {
                Insn::SConditional(dst, src)
            } else {
                Insn::SBitWrite(dst, src, bit_opcode == 29)
            },
            len,
        ));
    }
    if first >> 23 == 0b1_0111_1110 && matches!((first >> 16) & 127, 12 | 13) {
        let (a, b) = (first as u8, (first >> 8) as u8);
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (a, la) = scalar_source(code, offset, a)?;
        let (b, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SBitTest(a, b, (first >> 16) & 127 == 13), la.max(lb)));
    }
    if first >> 23 == 0x162 {
        let dst = ((first >> 16) & 127) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOPK register"));
        }
        let low = first & 65535;
        let bits = if low >= 32768 { low + 0xffff0000 } else { low };
        return Ok((Insn::SConditional(dst, Src::Imm(bits)), 4));
    }
    // SOP1: ENCODING 0b10_1111101 in bits 23-31, SDST 16-22, OP 8-15.
    let sop1_op = (first >> 8) & 0xff;
    if first >> 23 == 0b1_0111_1101
        && matches!(sop1_op, 7 | 11 | 13 | 15 | 17 | 19 | 21 | 23 | 25 | 26 | 52)
    {
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOP1 destination"));
        }
        let (src, len) = scalar_source(code, offset, first as u8)?;
        return Ok((Insn::SUnary(sop1_op, dst, src), len));
    }
    if first & 0xff80_ff00 == 0xbe80_0300 {
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 107 && dst < 126 {
            return Err(fail("unsupported SOP1 destination"));
        }
        let (src, len) = scalar_source(code, offset, first as u8)?;
        return Ok((Insn::SMov(dst, src), len));
    }
    // AMD RDNA 2 ISA guide 12.2 / 13.1.2: CMPK reads SDST, writes only
    // SCC, and uses an unsigned immediate for opcodes 9-14. Normalize to
    // this interpreter's SOPC operations, independently of production decoding.
    let sopk_op = (first >> 23) & 31;
    if first >> 28 == 0xb && matches!(sopk_op, 0 | 3..=16) {
        let dst = ((first >> 16) & 127) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOPK register"));
        }
        let low = first & 0xffff;
        let immediate = Src::Imm(if !(9..=14).contains(&sopk_op) && low & 0x8000 != 0 {
            low | 0xffff_0000
        } else {
            low
        });
        let insn = match sopk_op {
            0 => Insn::SMov(dst, immediate),
            3..=14 => Insn::SCmp(sopk_op - 3, Src::Reg(dst), immediate),
            15 => Insn::SAlu(2, dst, Src::Reg(dst), immediate),
            _ => Insn::SAlu(38, dst, Src::Reg(dst), immediate),
        };
        return Ok((insn, 4));
    }
    // SOP2: ENCODING 0b10 in bits 30-31, OP in bits 23-29, SDST in 16-22.
    if first >> 30 == 0b10
        && matches!(
            (first >> 23) & 0x7f,
            0..=10
                | 14
                | 16
                | 18
                | 20
                | 22
                | 24
                | 26
                | 28
                | 30
                | 32
                | 34
                | 36
                | 38..=40
                | 44
                | 46..=54
        )
    {
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 105 {
            return Err(fail("unsupported SOP2 destination"));
        }
        let (a, b) = (first as u8, (first >> 8) as u8);
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (sa, la) = scalar_source(code, offset, a)?;
        let (sb, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SAlu((first >> 23) & 0x7f, dst, sa, sb), la.max(lb)));
    }
    // SOPC: ENCODING 0b10_1111110 in bits 23-31, OP in 16-22.
    if first >> 23 == 0b1_0111_1110 && (first >> 16) & 0x7f <= 11 {
        let a = first as u8;
        let b = (first >> 8) as u8;
        if a == 255 && b == 255 {
            return Err(fail("two scalar literals"));
        }
        let (sa, la) = scalar_source(code, offset, a)?;
        let (sb, lb) = scalar_source(code, offset, b)?;
        return Ok((Insn::SCmp((first >> 16) & 0x7f, sa, sb), la.max(lb)));
    }
    if first & 0xffff_0000 == 0xbf82_0000 {
        return Ok((Insn::Branch(branch_target(first, offset)?), 4));
    }
    if first & 0xffff_0000 == 0xbf84_0000 {
        return Ok((Insn::BranchScc0(branch_target(first, offset)?), 4));
    }
    if first & 0xffff_0000 == 0xbf85_0000 {
        return Ok((Insn::BranchScc1(branch_target(first, offset)?), 4));
    }
    let tested_register = match first >> 16 {
        0xbf86 | 0xbf87 => Some(106),
        0xbf88 | 0xbf89 => Some(126),
        _ => None,
    };
    if let Some(register) = tested_register {
        return Ok((
            Insn::BranchMask {
                target: branch_target(first, offset)?,
                register,
                nonzero: first & 0x00010000 != 0,
            },
            4,
        ));
    }
    let legacy_opcode = first >> 25;
    if matches!(legacy_opcode, 22 | 24 | 26..=29) {
        let dst = (first >> 17) as u8;
        let b = (first >> 9) as u8;
        let (a, length) = vector_src0(code, offset, first, "legacy VOP2")?;
        let instruction = match legacy_opcode {
            22 => Insn::VShiftRight(dst, a, b),
            24 => Insn::VShiftRightArithmetic(dst, a, b),
            26 => Insn::VShiftLeft(dst, a, b),
            27 => Insn::VAnd(dst, a, b),
            28 => Insn::VOr(dst, a, b),
            _ => Insn::VXor(dst, a, b),
        };
        return Ok((instruction, length));
    }
    if matches!(first >> 25, 3 | 8) {
        if first & 0x100 == 0 {
            return Err(fail("floating VOP2 requires VGPR SRC0"));
        }
        let (dst, a, b) = ((first >> 17) as u8, first as u8, (first >> 9) as u8);
        let instruction = if first >> 25 == 3 {
            Insn::VAddF(dst, a, b)
        } else {
            Insn::VMulF(dst, a, b)
        };
        return Ok((instruction, 4));
    }
    // Exact SOPK null,0 form; other registers and thresholds remain unsupported.
    if first == 0xbc7d_0000 {
        return Ok((Insn::WaitRead, 4));
    }
    if first & 0xffff_0000 == 0xbf8c_0000 {
        if first != 0xbf8c_3f70 {
            return Err(fail("waitcnt not vmcnt(0)"));
        }
        return Ok((Insn::WaitRead, 4));
    }
    if first & 0xff80_0000 == 0xbb80_0000 {
        if first != 0xbbfd_0000 {
            return Err(fail("waitcnt_vscnt not null,0"));
        }
        return Ok((Insn::WaitWrite, 4));
    }
    if first & 0xffff_0000 == 0xbf81_0000 {
        if first != 0xbf81_0000 {
            return Err(fail("endpgm immediate"));
        }
        return Ok((Insn::End, 4));
    }
    Err(fail("unsupported opcode"))
}

/// SRC0 (bits 8:0 of `first`) of a VOPC or VOP2 integer word: an SGPR,
/// `vcc_lo`, an inline integer, a literal dword or a VGPR.
fn vector_src0(
    code: &[u8],
    offset: usize,
    first: u32,
    what: &str,
) -> Result<(CmpSrc, usize), String> {
    let selector = first & 0x1ff;
    Ok(match selector {
        0..=105 => (CmpSrc::Sgpr(selector as u8), 4),
        106 => (CmpSrc::Vcc, 4),
        128..=192 => (CmpSrc::Imm(selector - 128), 4),
        193..=208 => (CmpSrc::Imm(192u32.wrapping_sub(selector)), 4),
        240..=248 => (
            CmpSrc::Imm(
                [
                    0x3f00_0000,
                    0xbf00_0000,
                    0x3f80_0000,
                    0xbf80_0000,
                    0x4000_0000,
                    0xc000_0000,
                    0x4080_0000,
                    0xc080_0000,
                    0x3e22_f983,
                ][(selector - 240) as usize],
            ),
            4,
        ),
        255 => (
            CmpSrc::Imm(
                word(code, offset + 4)
                    .map_err(|_| format!("{offset:#x}: truncated {what} literal"))?,
            ),
            8,
        ),
        256..=511 => (CmpSrc::Vgpr(selector as u8), 4),
        _ => return Err(format!("{offset:#x}: bad {what} source")),
    })
}

fn vector_source(selector: u32) -> Option<Src> {
    match selector {
        128..=192 => Some(Src::Imm(selector - 128)),
        193..=208 => Some(Src::Imm(192u32.wrapping_sub(selector))),
        240..=248 => Some(Src::Imm(
            [
                0x3f00_0000,
                0xbf00_0000,
                0x3f80_0000,
                0xbf80_0000,
                0x4000_0000,
                0xc000_0000,
                0x4080_0000,
                0xc080_0000,
                0x3e22_f983,
            ][(selector - 240) as usize],
        )),
        256..=511 => Some(Src::Reg(selector as u8)),
        _ => None,
    }
}

fn scalar_source(code: &[u8], offset: usize, selector: u8) -> Result<(Src, usize), String> {
    match selector {
        0..=105 => Ok((Src::Reg(selector), 4)),
        128..=192 => Ok((Src::Imm(u32::from(selector - 128)), 4)),
        193..=208 => Ok((Src::Imm(192u32.wrapping_sub(u32::from(selector))), 4)),
        240..=248 => Ok((
            Src::Imm(
                [
                    0x3f00_0000,
                    0xbf00_0000,
                    0x3f80_0000,
                    0xbf80_0000,
                    0x4000_0000,
                    0xc000_0000,
                    0x4080_0000,
                    0xc080_0000,
                    0x3e22_f983,
                ][(selector - 240) as usize],
            ),
            4,
        )),
        // Attribute an incomplete extension to its owning instruction, as in
        // vector_src0; word() itself reports the address it was asked to read.
        255 => Ok((
            Src::Imm(
                word(code, offset + 4)
                    .map_err(|_| format!("{offset:#x}: truncated scalar literal"))?,
            ),
            8,
        )),
        _ => Err(format!("{offset:#x}: bad scalar source")),
    }
}

fn branch_target(first: u32, offset: usize) -> Result<usize, String> {
    let displacement = first as u16 as i16;
    let words =
        usize::try_from(displacement).map_err(|_| format!("{offset:#x}: backward branch"))?;
    Ok(offset + 4 + words * 4)
}

fn word(code: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = code
        .get(offset..offset + 4)
        .ok_or_else(|| format!("{offset:#x}: truncated instruction"))?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

#[cfg(test)]
mod scalar_pack_tests {
    use super::*;

    #[test]
    fn packs_selected_halves_with_aliasing_and_preserves_every_scc_state() {
        // Four distinct halfwords make source reversal and wrong-half choices visible.
        for (opcode, expected) in [(50, 0xdef0_5678), (51, 0x9abc_5678), (52, 0x9abc_1234)] {
            for initial_scc in [None, Some(false), Some(true)] {
                for dst in [8, 9, 10] {
                    let mut sgpr = [None; 128];
                    sgpr[8] = Some(0x1234_5678);
                    sgpr[9] = Some(0x9abc_def0);
                    let mut scc = initial_scc;
                    scalar_alu(
                        &mut sgpr,
                        &mut scc,
                        (opcode, dst, Src::Reg(8), Src::Reg(9)),
                        0,
                    )
                    .unwrap();
                    assert_eq!(sgpr[dst as usize], Some(expected));
                    assert_eq!(scc, initial_scc);
                    // A zero result must not clear a previously true SCC either.
                    scalar_alu(
                        &mut sgpr,
                        &mut scc,
                        (opcode, dst, Src::Imm(0), Src::Imm(0)),
                        0,
                    )
                    .unwrap();
                    assert_eq!(sgpr[dst as usize], Some(0));
                    assert_eq!(scc, initial_scc);
                }
            }
        }
    }
}

#[cfg(test)]
mod sopk_tests {
    use super::*;

    #[test]
    fn normalizes_sopk_and_preserves_multiply_scc() {
        for (opcode, old, imm, expected, overflow) in [
            (15, 0x7fff_ffff, 1, 0x8000_0000, true),
            (15, 0x8000_0000, 0xffff, 0x7fff_ffff, true),
            (15, 1, 0xffff, 0, false),
            (16, 3, 0x8000, 0xfffe_8000, false),
            (16, 0x8000_0000, 2, 0, false),
        ] {
            let word = 0xb008_0000u32 | opcode << 23 | imm;
            let (insn, length) = decode_one(&word.to_le_bytes(), 0).unwrap();
            assert_eq!(length, 4);
            let Insn::SAlu(op, dst, a, b) = insn else {
                panic!("SOPK arithmetic did not normalize to scalar ALU");
            };
            for initial in [None, Some(false), Some(true)] {
                let mut sgpr = [None; 128];
                sgpr[8] = Some(old);
                let mut scc = initial;
                scalar_alu(&mut sgpr, &mut scc, (op, dst, a, b), 0).unwrap();
                assert_eq!(sgpr[8], Some(expected));
                assert_eq!(
                    scc,
                    if opcode == 15 {
                        Some(overflow)
                    } else {
                        initial
                    }
                );
            }
        }
        for dst in [0, 105, 106, 127] {
            for opcode in [0, 15, 16] {
                let word = 0xb000_ffffu32 | opcode << 23 | dst << 16;
                let result = decode_one(&word.to_le_bytes(), 0);
                if dst > 105 {
                    assert!(result.is_err());
                } else if opcode == 0 {
                    assert!(
                        matches!(result.unwrap(), (Insn::SMov(d, Src::Imm(u32::MAX)), 4) if d == dst as u8)
                    );
                } else {
                    assert!(result.is_ok());
                }
            }
        }
    }
}

#[cfg(test)]
mod cmpk_tests {
    use super::*;

    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    #[test]
    fn cmpk_relations_extension_scc_and_source_preservation() {
        // EQ, LG, GT, GE, LT, LE; immediate sign boundaries and full-width SGPRs.
        let equal = [true, false, false, true, false, true];
        let less = [false, true, false, false, true, true];
        let greater = [false, true, true, true, false, false];
        for (signed, value, imm, expected) in [
            (true, 0xffff_8000, 0x8000, equal),
            (true, 0xffff_7fff, 0x8000, less),
            (true, 0x0000_8000, 0x8000, greater),
            (true, 0xffff_ffff, 0xffff, equal),
            (true, 0x8000_0000, 0xffff, less),
            (true, 0x7fff_ffff, 0x7fff, greater),
            (false, 0x0000_8000, 0x8000, equal),
            (false, 0x0000_7fff, 0x8000, less),
            (false, 0xffff_8000, 0x8000, greater),
            (false, 0x0000_ffff, 0xffff, equal),
            (false, 0, 0xffff, less),
            (false, 0xffff_ffff, 0xffff, greater),
            (true, 0, 0, equal),
            (false, 0, 0, equal),
        ] {
            for (relation, flag) in expected.into_iter().enumerate() {
                let opcode = if signed { 3 } else { 9 } + relation as u32;
                for register in [8, 105] {
                    for previous in [None, Some(false), Some(true)] {
                        let mut words = vec![0xbe80_03ff | register << 16, value];
                        if let Some(previous) = previous {
                            words.push(if previous { 0xbf06_8080 } else { 0xbf06_8180 });
                        }
                        words.push(0xb000_0000 | opcode << 23 | register << 16 | imm);
                        let finish = |prefix: &[u32], source| {
                            let mut code = prefix.to_vec();
                            code.extend([
                                0x7e02_0200 | source,
                                0xe070_2000,
                                0x8001_0100,
                                0xbbfd_0000,
                                0xbf81_0000,
                            ]);
                            bytes(&code)
                        };
                        assert_eq!(
                            execute(&finish(&words, register), &[0]).unwrap(),
                            vec![value]
                        );
                        words.push(0x850a_8281); // CSELECT s10, 1, 2 observes SCC.
                        assert_eq!(
                            execute(&finish(&words, 10), &[0]).unwrap(),
                            vec![if flag { 1 } else { 2 }]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn cmpk_register_bounds_length_and_uninitialized_read() {
        for opcode in 3..=14 {
            for register in 0..=127 {
                let instruction = 0xb000_ffff | opcode << 23 | register << 16;
                let code = bytes(&[instruction, 0xbf81_0000]);
                assert_eq!(decode(&code).is_ok(), register <= 105);
            }
            let instruction = 0xb069_ffff | opcode << 23;
            let code = bytes(&[instruction, 0xbf81_0000]);
            // s105 is a read, so CMPK cannot initialize it.
            assert!(execute(&code, &[0]).is_err());
            for length in 1..4 {
                assert!(decode(&code[..length]).is_err());
            }
        }
    }
}
