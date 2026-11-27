// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent CPU execution of the synthetic gfx1030 fixture subset, reference sampling, and host-side references for depth-tested rendering, indexed draws and blending.

pub mod blend;
pub mod depth;
pub mod indexed;
pub mod texture_sampling;

use std::fmt;

const MAX_CODE_BYTES: usize = 1024;
const MAX_INPUT_WORDS: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub struct ReferenceError {
    pub offset: usize,
    pub detail: &'static str,
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "reference byte {:#x}: {}", self.offset, self.detail)
    }
}

impl std::error::Error for ReferenceError {}

/// Register index or immediate bits. The opcode determines the register bank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    Register(u8),
    Immediate(u32),
}

/// Operations decoded independently by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    Load {
        data: u8,
        index: u8,
        resource: u8,
    },
    Store {
        data: u8,
        index: u8,
        resource: u8,
    },
    WaitRead,
    WaitWrite,
    Move(u8, Source),
    Add(u8, u8, u8),
    ScalarMove(u8, Source),
    /// SOP1 unary operation writing an SGPR (s0-s105), and SCC for some.
    ScalarUnary(ScalarUnary, u8, Source),
    /// VOP1 move of a normal SGPR or `vcc_lo` (106) into a VGPR.
    MoveScalar(u8, u8),
    /// VOPC 32-bit integer compare writing the Wave32 mask to `vcc_lo`.
    Compare(Comparison, CompareSource, u8),
    /// SOP2 operation writing an SGPR, and SCC for some forms.
    Scalar(ScalarAlu, u8, Source, Source),
    /// SOPC `s_cmp_eq_{i32,u32}` (opcodes 0 and 6, identical per guide 12.4).
    CompareEqual(Source, Source),
    /// The other SOPC 32-bit integer compares (opcodes 1-5 and 7-11), writing
    /// only SCC: LG, GT, GE, LT, LE, signed for 1-5, unsigned for 7-11.
    ScalarCompare(Comparison, Source, Source),
    Branch {
        target: usize,
        condition: BranchCondition,
    },
    Vector(VectorAlu, u8, u8, u8),
    /// VOP2 32-bit integer form: VDST, SRC0 (VGPR, SGPR, `vcc_lo`, inline
    /// integer or literal) and VSRC1. Writes neither VCC nor SCC.
    VectorInteger(VectorInteger, u8, CompareSource, u8),
    End,
}

/// Relation and operand interpretation of a VOPC integer compare.
///
/// AMD RDNA 2 ISA guide section 12.9, Tables 59 and 61: opcode 128 + offset
/// for signed and 192 + offset for unsigned 32-bit operands, with offsets
/// LT 1, EQ 2, LE 3, GT 4, NE 5, GE 6.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Comparison {
    /// Opcode offset 1-6 within the 32-bit integer group.
    pub offset: u8,
    /// Whether the operands are two's-complement integers.
    pub signed: bool,
}

impl Comparison {
    /// The per-lane result, from an exact comparison of the widened operands.
    fn holds(self, s0: u32, s1: u32) -> bool {
        let widen = |bits: u32| {
            if self.signed {
                i64::from(bits as i32)
            } else {
                i64::from(bits)
            }
        };
        let (s0, s1) = (widen(s0), widen(s1));
        match self.offset {
            1 => s0 < s1,
            2 => s0 == s1,
            3 => s0 <= s1,
            4 => s0 > s1,
            5 => s0 != s1,
            _ => s0 >= s1,
        }
    }
}

/// First source of a VOPC compare or a VOP2 integer form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareSource {
    /// VGPR index.
    Vector(u8),
    /// SGPR index s0-s105, or 106 for `vcc_lo`.
    Scalar(u8),
    /// Inline integer or literal bits.
    Immediate(u32),
}

/// Scalar condition tested by a forward branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchCondition {
    /// Take the branch unconditionally.
    Always,
    /// Take the branch when SCC is clear.
    SccZero,
    /// Take the branch when the low half of VCC is clear.
    VccZero,
    /// Take the branch when the low half of EXEC is clear.
    ExecZero,
}

/// SOP2 operations decoded by the reference interpreter.
///
/// AMD RDNA 2 XML descriptions: additions store the carry-out in SCC,
/// subtractions the borrow; ADDC/SUBB add or subtract SCC; bitwise
/// operations and shifts set SCC iff the result is nonzero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarAlu {
    Add,
    Subtract,
    AddWithCarry,
    SubtractWithBorrow,
    /// SCC = signed overflow (RDNA 2 ISA guide 12.1).
    AddSigned,
    SubtractSigned,
    And,
    Or,
    Xor,
    /// RDNA 2 ISA guide 12.1: D = S0 & ~S1, SCC = (D != 0).
    AndNot,
    /// D = S0 | ~S1, SCC = (D != 0).
    OrNot,
    /// D = ~(S0 & S1), SCC = (D != 0).
    Nand,
    /// D = ~(S0 | S1), SCC = (D != 0).
    Nor,
    /// D = ~(S0 ^ S1), SCC = (D != 0).
    Xnor,
    /// D = S0 << S1[4:0], SCC = (D != 0).
    ShiftLeft,
    /// D = S0 >> S1[4:0] (logical), SCC = (D != 0).
    ShiftRight,
    /// D = signext(S0) >> S1[4:0], SCC = (D != 0).
    ArithmeticShiftRight,
    /// Guide 12.1: D = (S0 < S1) ? S0 : S1, SCC = (S0 < S1); signed or not.
    Minimum {
        signed: bool,
    },
    /// D = (S0 > S1) ? S0 : S1, SCC = (S0 > S1).
    Maximum {
        signed: bool,
    },
    /// D = SCC ? S0 : S1; reads SCC and leaves it unchanged.
    Select,
    /// D = ((1 << S0[4:0]) - 1) << S1[4:0]; SCC unchanged.
    BitfieldMask,
    /// D = low 32 bits of S0 * S1; SCC unchanged.
    Multiply,
    /// D = (S0 >> S1[4:0]) & ((1 << S1[22:16]) - 1), sign-extended from the
    /// field's top bit when signed (S0 then shifts arithmetically);
    /// SCC = (D != 0).
    BitfieldExtract {
        signed: bool,
    },
    /// D = |S0 - S1| with the 32-bit difference (0x80000000 stays),
    /// SCC = (D != 0).
    AbsoluteDifference,
    /// D = (S0 << N) + S1, SCC = ((S0 << N) + S1 >= 2^32) with N = 1..4.
    ShiftLeftAdd(u8),
    /// Guide 12.1: pack S0's selected half into D[15:0] and S1's into
    /// D[31:16]. LL, LH and HH leave SCC unchanged, including unset.
    Pack {
        high0: bool,
        high1: bool,
    },
    /// D = high 32 bits of S0 * S1; SCC unchanged.
    MultiplyHigh {
        signed: bool,
    },
}

/// SOP1 unary operations decoded by the reference interpreter (RDNA 2 ISA
/// guide 12.3; SCC per Tables 12 and 14).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarUnary {
    /// D = ~S0, SCC = (D != 0).
    Not,
    /// D[31:0] = S0[0:31].
    ReverseBits,
    /// D = number of zero bits, SCC = (D != 0).
    CountZeros,
    /// D = number of one bits, SCC = (D != 0).
    CountOnes,
    /// D = index of the lowest zero bit, or -1.
    FindFirstZero,
    /// D = index of the lowest one bit, or -1.
    FindFirstOne,
    /// D = number of zeros above the highest one bit, or -1 for 0.
    FindLastBit,
    /// D = number of leading bits equal to the sign bit, or -1 for 0 and -1.
    FindLastSignBit,
    /// D = signext(S0[7:0]).
    SignExtendByte,
    /// D = signext(S0[15:0]).
    SignExtendShort,
    /// D = |S0| (0x80000000 stays), SCC = (D != 0).
    Absolute,
}

/// VOP2 operations decoded by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorAlu {
    ShiftLeft,
    ShiftRight,
    ArithmeticShiftRight,
    And,
    Or,
    Xor,
    FloatAdd,
    FloatMultiply,
}

/// VOP2 32-bit integer forms with a general SRC0 (AMD RDNA 2 ISA guide
/// section 12.7). None has a carry-in or carry-out.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorInteger {
    /// 9 / 11: D = low 32 bits of S0[23:0] * S1[23:0], sign-extended from
    /// bit 23 when `signed`.
    Multiply24 { signed: bool },
    /// 10 / 12: D = bits 63:32 of the same 48-bit product.
    MultiplyHigh24 { signed: bool },
    /// 17 / 19: D = S0 < S1 ? S0 : S1.
    Minimum { signed: bool },
    /// 18 / 20: D = S0 >= S1 ? S0 : S1.
    Maximum { signed: bool },
    /// 30: D = ~(S0 ^ S1).
    Xnor,
    /// 37: D = S0 + S1.
    Add,
    /// 38: D = S0 - S1.
    Subtract,
    /// 39: D = S1 - S0.
    SubtractReverse,
}

impl VectorInteger {
    /// Opcode bits 30-25 of the VOP2 word.
    fn from_opcode(opcode: u8) -> Option<Self> {
        Some(match opcode {
            9 => Self::Multiply24 { signed: true },
            10 => Self::MultiplyHigh24 { signed: true },
            11 => Self::Multiply24 { signed: false },
            12 => Self::MultiplyHigh24 { signed: false },
            17 => Self::Minimum { signed: true },
            18 => Self::Maximum { signed: true },
            19 => Self::Minimum { signed: false },
            20 => Self::Maximum { signed: false },
            30 => Self::Xnor,
            37 => Self::Add,
            38 => Self::Subtract,
            39 => Self::SubtractReverse,
            _ => return None,
        })
    }

    /// D for S0 = `a`, S1 = `b`, from exact i64 arithmetic reduced modulo 2^32.
    fn apply(self, a: u32, b: u32) -> u32 {
        let field = |bits: u32, signed: bool| {
            if signed {
                i64::from(((bits << 8) as i32) >> 8)
            } else {
                i64::from(bits & 0x00ff_ffff)
            }
        };
        let widen = |bits: u32, signed: bool| {
            if signed {
                i64::from(bits as i32)
            } else {
                i64::from(bits)
            }
        };
        match self {
            Self::Multiply24 { signed } => (field(a, signed) * field(b, signed)) as u32,
            Self::MultiplyHigh24 { signed } => ((field(a, signed) * field(b, signed)) >> 32) as u32,
            Self::Minimum { signed } => {
                if widen(a, signed) < widen(b, signed) {
                    a
                } else {
                    b
                }
            }
            Self::Maximum { signed } => {
                if widen(a, signed) >= widen(b, signed) {
                    a
                } else {
                    b
                }
            }
            Self::Xnor => !(a ^ b),
            Self::Add => a.wrapping_add(b),
            Self::Subtract => a.wrapping_sub(b),
            Self::SubtractReverse => b.wrapping_sub(a),
        }
    }
}

/// Execute original little-endian gfx1030 bytes with the synthetic binding ABI.
///
/// Each dispatched lane reads the corresponding input word and may write only
/// its own output word. Integer operations are exact modulo 2^32.
pub fn execute(code: &[u8], input: &[u32]) -> Result<Vec<u32>, ReferenceError> {
    if input.is_empty() || input.len() > MAX_INPUT_WORDS {
        return Err(error(0, "input word count outside reference limits"));
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(error(0, "instruction stream exceeds reference limit"));
    }
    let program = decode(code)?;
    for instruction in &program {
        match instruction.op {
            Op::End if instruction.offset + instruction.byte_len != code.len() => {
                return Err(error(
                    instruction.offset + instruction.byte_len,
                    "trailing code after s_endpgm",
                ));
            }
            Op::Load {
                index, resource, ..
            } if (index, resource) != (0, 0) => {
                return Err(error(
                    instruction.offset,
                    "load outside synthetic binding ABI",
                ));
            }
            Op::Store {
                index, resource, ..
            } if (index, resource) != (0, 4) => {
                return Err(error(
                    instruction.offset,
                    "store outside synthetic binding ABI",
                ));
            }
            _ => {}
        }
    }
    if !program
        .last()
        .is_some_and(|instruction| instruction.op == Op::End)
    {
        return Err(error(code.len(), "missing s_endpgm"));
    }
    // Waves are consecutive groups of 32 dispatch indices; a final partial
    // wave has absent lanes, which contribute 0 to compare masks.
    let mut output = Vec::with_capacity(input.len());
    for wave in input.chunks(32) {
        output.extend(execute_wave(&program, wave)?);
    }
    Ok(output)
}

fn error(offset: usize, detail: &'static str) -> ReferenceError {
    ReferenceError { offset, detail }
}

/// An operation and its span in the original byte stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedInstruction {
    pub offset: usize,
    pub byte_len: usize,
    pub op: Op,
}

/// Decode the supported gfx1030 subset without applying the execution ABI.
///
/// AMD RDNA 2 ISA guide, Instruction Encoding. Identify the forms by bytes
/// and independently extract their variable fields.
///
/// # Errors
///
/// Returns the failing byte offset for an unsupported, truncated
/// or invalid instruction stream.
pub fn decode(code: &[u8]) -> Result<Vec<DecodedInstruction>, ReferenceError> {
    let mut program = Vec::new();
    let mut offset = 0;
    while let Some(bytes) = code.get(offset..).filter(|bytes| !bytes.is_empty()) {
        let (op, length) = match bytes {
            [
                0,
                0x20,
                opcode @ (0x30 | 0x70),
                0xe0,
                index,
                data,
                group @ 0..=25,
                0x80,
                ..,
            ] => {
                let (data, index, resource) = (*data, *index, *group * 4);
                let op = if *opcode == 0x30 {
                    Op::Load {
                        data,
                        index,
                        resource,
                    }
                } else {
                    Op::Store {
                        data,
                        index,
                        resource,
                    }
                };
                (op, 8)
            }
            [0x70, 0x3f, 0x8c, 0xbf, ..] => (Op::WaitRead, 4),
            [0x00, 0x00, 0xfd, 0xbb, ..] => (Op::WaitWrite, 4),
            [0x00, 0x00, 0x81, 0xbf, ..] => (Op::End, 4),
            [
                source,
                3,
                destination @ (0x80..=0xeb | 0xfe..=0xff),
                0xbe,
                tail @ ..,
            ] => {
                let (source, length) = decode_scalar(*source, tail, offset)?;
                (Op::ScalarMove(destination & 0x7f, source), length)
            }
            // SOP1: ENCODING 0b10_1111101 in bits 23-31, SDST in 16-22 (s0-s105
            // only), OP in 8-15.
            [
                source,
                op @ (7 | 11 | 13 | 15 | 17 | 19 | 21 | 23 | 25 | 26 | 52),
                destination @ 0x80..=0xe9,
                0xbe,
                tail @ ..,
            ] => {
                let kind = match op {
                    7 => ScalarUnary::Not,
                    11 => ScalarUnary::ReverseBits,
                    13 => ScalarUnary::CountZeros,
                    15 => ScalarUnary::CountOnes,
                    17 => ScalarUnary::FindFirstZero,
                    19 => ScalarUnary::FindFirstOne,
                    21 => ScalarUnary::FindLastBit,
                    23 => ScalarUnary::FindLastSignBit,
                    25 => ScalarUnary::SignExtendByte,
                    26 => ScalarUnary::SignExtendShort,
                    _ => ScalarUnary::Absolute,
                };
                let (source, length) = decode_scalar(*source, tail, offset)?;
                (Op::ScalarUnary(kind, destination & 0x7f, source), length)
            }
            // SOPC: ENCODING 0b10_1111110 in bits 23-31, OP in 16-22 (guide
            // 13.1.4); 0-5 signed and 6-11 unsigned EQ, LG, GT, GE, LT, LE.
            [left, right, op @ 0..=11, 0xbf, tail @ ..] if (*left, *right) != (255, 255) => {
                let (left, left_length) = decode_scalar(*left, tail, offset)?;
                let (right, right_length) = decode_scalar(*right, tail, offset)?;
                // VOPC relation offsets: LT 1, EQ 2, LE 3, GT 4, NE 5, GE 6.
                let relation = [2, 5, 4, 6, 1, 3][usize::from(op % 6)];
                let comparison = Comparison {
                    offset: relation,
                    signed: *op < 6,
                };
                let op = if relation == 2 {
                    Op::CompareEqual(left, right)
                } else {
                    Op::ScalarCompare(comparison, left, right)
                };
                (op, left_length.max(right_length))
            }
            // AMD RDNA 2 ISA guide 12.2, 13.1.2: SOPK uses a signed
            // 16-bit immediate, no literal dword. ADDK/MULK read old SDST.
            [lo, hi, dst, encoding @ 0xb0..=0xbf, ..]
                if dst & 0x7f <= 105 && matches!((encoding & 0xf) << 1 | dst >> 7, 0 | 15 | 16) =>
            {
                let opcode = (encoding & 0xf) << 1 | dst >> 7;
                let dst = dst & 0x7f;
                let immediate = Source::Immediate(i32::from(i16::from_le_bytes([*lo, *hi])) as u32);
                let op = match opcode {
                    0 => Op::ScalarMove(dst, immediate),
                    15 => Op::Scalar(ScalarAlu::AddSigned, dst, Source::Register(dst), immediate),
                    _ => Op::Scalar(ScalarAlu::Multiply, dst, Source::Register(dst), immediate),
                };
                (op, 4)
            }
            // SOP2: OP in bits 23-29, SDST in bits 16-22, SSRC1, SSRC0.
            [left, right, low, high @ 0x80..=0xbf, tail @ ..]
                if (*left, *right) != (255, 255)
                    && low & 0x7f <= 105
                    && matches!(
                        (high & 0x3f) << 1 | low >> 7,
                        0..=10 | 14 | 16 | 18 | 20 | 22 | 24 | 26 | 28 | 30 | 32 | 34
                            | 36 | 38..=40 | 44 | 46..=54
                    ) =>
            {
                let kind = match (high & 0x3f) << 1 | low >> 7 {
                    0 => ScalarAlu::Add,
                    1 => ScalarAlu::Subtract,
                    2 => ScalarAlu::AddSigned,
                    3 => ScalarAlu::SubtractSigned,
                    4 => ScalarAlu::AddWithCarry,
                    5 => ScalarAlu::SubtractWithBorrow,
                    6 => ScalarAlu::Minimum { signed: true },
                    7 => ScalarAlu::Minimum { signed: false },
                    8 => ScalarAlu::Maximum { signed: true },
                    9 => ScalarAlu::Maximum { signed: false },
                    10 => ScalarAlu::Select,
                    14 => ScalarAlu::And,
                    16 => ScalarAlu::Or,
                    18 => ScalarAlu::Xor,
                    20 => ScalarAlu::AndNot,
                    22 => ScalarAlu::OrNot,
                    24 => ScalarAlu::Nand,
                    26 => ScalarAlu::Nor,
                    28 => ScalarAlu::Xnor,
                    30 => ScalarAlu::ShiftLeft,
                    32 => ScalarAlu::ShiftRight,
                    34 => ScalarAlu::ArithmeticShiftRight,
                    36 => ScalarAlu::BitfieldMask,
                    38 => ScalarAlu::Multiply,
                    39 => ScalarAlu::BitfieldExtract { signed: false },
                    40 => ScalarAlu::BitfieldExtract { signed: true },
                    44 => ScalarAlu::AbsoluteDifference,
                    op @ 46..=49 => ScalarAlu::ShiftLeftAdd(op - 45),
                    op @ 50..=52 => ScalarAlu::Pack {
                        high0: op == 52,
                        high1: op != 50,
                    },
                    53 => ScalarAlu::MultiplyHigh { signed: false },
                    _ => ScalarAlu::MultiplyHigh { signed: true },
                };
                let (left, left_length) = decode_scalar(*left, tail, offset)?;
                let (right, right_length) = decode_scalar(*right, tail, offset)?;
                (
                    Op::Scalar(kind, low & 0x7f, left, right),
                    left_length.max(right_length),
                )
            }
            [lo, hi, opcode @ (0x82 | 0x84 | 0x86 | 0x88), 0xbf, ..] => {
                let words = i16::from_le_bytes([*lo, *hi]);
                let words = usize::try_from(words).map_err(|_| error(offset, "backward branch"))?;
                (
                    Op::Branch {
                        target: offset + 4 + words * 4,
                        condition: match opcode {
                            0x84 => BranchCondition::SccZero,
                            0x86 => BranchCondition::VccZero,
                            0x88 => BranchCondition::ExecZero,
                            _ => BranchCondition::Always,
                        },
                    },
                    4,
                )
            }
            [first, second, third, opcode, ..]
                if matches!(
                    opcode & 0xfe,
                    0x06 | 0x10 | 0x2c | 0x30 | 0x34 | 0x36 | 0x38 | 0x3a
                ) && second & 1 != 0 =>
            {
                let kind = match opcode & 0xfe {
                    0x06 => VectorAlu::FloatAdd,
                    0x10 => VectorAlu::FloatMultiply,
                    0x34 => VectorAlu::ShiftLeft,
                    0x2c => VectorAlu::ShiftRight,
                    0x30 => VectorAlu::ArithmeticShiftRight,
                    0x36 => VectorAlu::And,
                    0x38 => VectorAlu::Or,
                    _ => VectorAlu::Xor,
                };
                let dst = (third >> 1) | ((opcode & 1) << 7);
                let src1 = (second >> 1) | ((third & 1) << 7);
                (Op::Vector(kind, dst, *first, src1), 4)
            }
            [dst, 0, 0x25, 0xd5, lo, middle, hi @ 2..=3, 0x02, ..] if middle & 1 == 1 => {
                let src1 = (middle >> 1) | ((hi & 1) << 7);
                (Op::Add(*dst, *lo, src1), 8)
            }
            // VOPC (bits 31-25 = 0b0111110): SRC0 bits 0-8, VSRC1 bits 9-16, OP bits 17-24.
            [lo, packed, upper, tag @ 0x7c..=0x7d, tail @ ..]
                if matches!(
                    (u16::from(*tag & 1) << 7) | u16::from(*upper >> 1),
                    129..=134 | 193..=198
                ) =>
            {
                let opcode = (u16::from(*tag & 1) << 7) | u16::from(*upper >> 1);
                let comparison = Comparison {
                    offset: (opcode & 0x3f) as u8,
                    signed: opcode < 192,
                };
                let vsrc1 = (packed >> 1) | ((upper & 1) << 7);
                let (src0, length) =
                    decode_vector_source(*lo, *packed, tail, offset, "unsupported VOPC source")?;
                (Op::Compare(comparison, src0, vsrc1), length)
            }
            // VOP2 integer forms (bit 31 = 0): OP bits 25-30, VDST 17-24,
            // VSRC1 9-16, SRC0 0-8.
            [lo, packed, upper, tag @ 0x00..=0x7b, tail @ ..]
                if VectorInteger::from_opcode(tag >> 1).is_some() =>
            {
                let kind = VectorInteger::from_opcode(tag >> 1).unwrap();
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let vsrc1 = (packed >> 1) | ((upper & 1) << 7);
                let (src0, length) =
                    decode_vector_source(*lo, *packed, tail, offset, "unsupported VOP2 source")?;
                (Op::VectorInteger(kind, dst, src0, vsrc1), length)
            }
            [lo, packed, upper, tag @ 0x7e..=0x7f, tail @ ..]
                if packed & 0xfe == 2 && upper & 1 == 0 =>
            {
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let selector = u16::from(*lo) | (u16::from(packed & 1) << 8);
                if selector <= 106 {
                    // VOP1 SRC0 0-105 selects a normal SGPR, 106 vcc_lo.
                    program.push(DecodedInstruction {
                        offset,
                        byte_len: 4,
                        op: Op::MoveScalar(dst, *lo),
                    });
                    offset += 4;
                    continue;
                }
                let (src, length) = match selector {
                    128..=192 => (Source::Immediate(u32::from(selector - 128)), 4),
                    193..=208 => (
                        Source::Immediate(0_u32.wrapping_sub(u32::from(selector - 192))),
                        4,
                    ),
                    242 => (Source::Immediate(1.0f32.to_bits()), 4),
                    255 => {
                        let literal = tail
                            .first_chunk::<4>()
                            .ok_or_else(|| error(offset, "truncated literal"))?;
                        (Source::Immediate(u32::from_le_bytes(*literal)), 8)
                    }
                    256..=511 => (Source::Register(*lo), 4),
                    _ => return Err(error(offset, "unsupported VOP1 source")),
                };
                (Op::Move(dst, src), length)
            }
            _ => {
                return Err(error(
                    offset,
                    "unsupported or truncated gfx1030 instruction",
                ));
            }
        };
        program.push(DecodedInstruction {
            offset,
            byte_len: length,
            op,
        });
        offset += length;
    }
    for instruction in &program {
        if let Op::Branch { target, .. } = instruction.op
            && program
                .binary_search_by_key(&target, |item| item.offset)
                .is_err()
        {
            return Err(error(
                instruction.offset,
                "branch target is not an instruction boundary",
            ));
        }
    }
    Ok(program)
}

fn decode_scalar(
    selector: u8,
    tail: &[u8],
    offset: usize,
) -> Result<(Source, usize), ReferenceError> {
    match selector {
        0..=105 => Ok((Source::Register(selector), 4)),
        128..=192 => Ok((Source::Immediate(u32::from(selector - 128)), 4)),
        193..=208 => Ok((
            Source::Immediate(0u32.wrapping_sub(u32::from(selector - 192))),
            4,
        )),
        255 => {
            let literal = tail
                .first_chunk::<4>()
                .ok_or_else(|| error(offset, "truncated scalar literal"))?;
            Ok((Source::Immediate(u32::from_le_bytes(*literal)), 8))
        }
        _ => Err(error(offset, "unsupported scalar source")),
    }
}

/// A vector SRC0 selector (bits 0-8: `lo` and bit 0 of `packed`): s0-s105 or
/// `vcc_lo` (106), an inline integer, a literal dword from `tail`, or a VGPR.
fn decode_vector_source(
    lo: u8,
    packed: u8,
    tail: &[u8],
    offset: usize,
    unsupported: &'static str,
) -> Result<(CompareSource, usize), ReferenceError> {
    let selector = u16::from(lo) | (u16::from(packed & 1) << 8);
    Ok(match selector {
        0..=106 => (CompareSource::Scalar(lo), 4),
        128..=192 => (CompareSource::Immediate(u32::from(selector - 128)), 4),
        193..=208 => (
            CompareSource::Immediate(0_u32.wrapping_sub(u32::from(selector - 192))),
            4,
        ),
        255 => {
            let literal = tail
                .first_chunk::<4>()
                .ok_or_else(|| error(offset, "truncated literal"))?;
            (CompareSource::Immediate(u32::from_le_bytes(*literal)), 8)
        }
        256..=511 => (CompareSource::Vector(lo), 4),
        _ => return Err(error(offset, unsupported)),
    })
}

fn vector_alu(op: VectorAlu, a: u32, b: u32, offset: usize) -> Result<u32, ReferenceError> {
    let error = || {
        error(
            offset,
            "float operation outside positive, normal, exact binary32 domain",
        )
    };
    match op {
        VectorAlu::ShiftLeft => return Ok(b << (a & 31)),
        VectorAlu::ShiftRight => return Ok(b >> (a & 31)),
        VectorAlu::ArithmeticShiftRight => return Ok(((b as i32) >> (a & 31)) as u32),
        VectorAlu::And => return Ok(a & b),
        VectorAlu::Or => return Ok(a | b),
        VectorAlu::Xor => return Ok(a ^ b),
        _ => {}
    }
    let (a, b) = (f32::from_bits(a), f32::from_bits(b));
    if !a.is_normal() || !b.is_normal() || a.is_sign_negative() || b.is_sign_negative() {
        return Err(error());
    }
    let (a, b) = (f64::from(a), f64::from(b));
    let exact = match op {
        VectorAlu::FloatAdd => {
            let sum = a + b;
            // Recover the smaller input to detect even a binary64 addition rounding.
            if sum - a.max(b) != a.min(b) {
                return Err(error());
            }
            sum
        }
        VectorAlu::FloatMultiply => a * b, // Two 24-bit significands fit in binary64.
        VectorAlu::ShiftLeft
        | VectorAlu::ShiftRight
        | VectorAlu::ArithmeticShiftRight
        | VectorAlu::And
        | VectorAlu::Or
        | VectorAlu::Xor => unreachable!(),
    };
    let result = exact as f32;
    if !result.is_normal() || f64::from(result) != exact {
        return Err(error());
    }
    Ok(result.to_bits())
}

const VCC_LO: usize = 106;
const VCC_HI: usize = 107;
const EXEC_LO: usize = 126;

/// Runs one Wave32 wave in lockstep. Scalar state is shared by the lanes;
/// VGPRs, transfers and outputs are per lane.
fn execute_wave(program: &[DecodedInstruction], input: &[u32]) -> Result<Vec<u32>, ReferenceError> {
    let lanes = input.len();
    let mut registers = vec![[None::<u32>; 256]; lanes];
    let mut scalars = [None; 128];
    let mut exec = u32::MAX;
    scalars[EXEC_LO] = Some(exec);
    let mut scc = None;
    // Whether VCC_LO was last written by a compare with nonzero EXEC.
    let mut lane_derived_vcc = false;
    let mut target = 0;
    let mut pending_read = None;
    let mut pending_write = vec![None; lanes];
    let mut output = vec![None; lanes];
    for (position, &DecodedInstruction { offset, op, .. }) in program.iter().enumerate() {
        if offset < target {
            continue;
        }
        let read_scalar = |source| match source {
            Source::Register(reg) => scalars[usize::from(reg)]
                .ok_or_else(|| error(offset, "read from uninitialized SGPR")),
            Source::Immediate(value) => Ok(value),
        };
        let read = |lane: usize, register: u8| {
            if register == 0 {
                return Err(error(offset, "v0 is reserved for the resource index"));
            }
            if pending_read == Some(register) {
                return Err(error(offset, "VGPR read before load wait"));
            }
            registers[lane][usize::from(register)]
                .ok_or_else(|| error(offset, "read from uninitialized VGPR"))
        };
        // Lanes that execute a vector ALU instruction under the current EXEC.
        let executes = |lane: usize| exec & (1 << lane) != 0;
        if exec != u32::MAX {
            match op {
                Op::Move(dst, _)
                | Op::MoveScalar(dst, _)
                | Op::Add(dst, ..)
                | Op::Vector(_, dst, ..)
                | Op::VectorInteger(_, dst, ..) => {
                    if exec != 0
                        && registers
                            .iter()
                            .any(|lane| lane[usize::from(dst)].is_none())
                    {
                        return Err(error(offset, "partial EXEC requires an initialized VGPR"));
                    }
                    // As in translation, an SGPR source must be known even for an inactive lane.
                    if let Op::MoveScalar(_, src) = op
                        && exec != 0
                    {
                        read_scalar(Source::Register(src))?;
                    }
                }
                Op::Load { .. } | Op::Store { .. } => {
                    return Err(error(offset, "vector memory requires full Wave32 EXEC"));
                }
                _ => {}
            }
        }
        // Per-lane results of a vector ALU instruction, applied after all lanes read.
        let mut results: Vec<Option<(u8, u32)>> = vec![None; lanes];
        match op {
            Op::Load { data: dst, .. } => {
                if dst == 0 || pending_read.is_some() {
                    return Err(error(
                        offset,
                        "invalid load destination or overlapping load",
                    ));
                }
                pending_read = Some(dst);
            }
            Op::Store { data: src, .. } => {
                for lane in 0..lanes {
                    let value = read(lane, src)?;
                    if pending_write[lane].is_some() || output[lane].is_some() {
                        return Err(error(offset, "multiple writes to the output"));
                    }
                    pending_write[lane] = Some(value);
                }
            }
            Op::WaitRead => {
                if let Some(dst) = pending_read.take() {
                    for (lane, &word) in input.iter().enumerate() {
                        results[lane] = Some((dst, word));
                    }
                }
            }
            Op::WaitWrite => {
                for lane in 0..lanes {
                    if let Some(value) = pending_write[lane].take() {
                        output[lane] = Some(value);
                    }
                }
            }
            Op::Move(dst, source) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = match source {
                        Source::Register(src) => read(lane, src)?,
                        Source::Immediate(value) => value,
                    };
                    results[lane] = Some((dst, value));
                }
            }
            Op::Add(dst, lhs, rhs) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = read(lane, lhs)?.wrapping_add(read(lane, rhs)?);
                    results[lane] = Some((dst, value));
                }
            }
            Op::MoveScalar(dst, src) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    results[lane] = Some((dst, read_scalar(Source::Register(src))?));
                }
            }
            Op::Vector(operator, dst, lhs, rhs) => {
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = vector_alu(operator, read(lane, lhs)?, read(lane, rhs)?, offset)?;
                    results[lane] = Some((dst, value));
                }
            }
            Op::VectorInteger(kind, dst, src0, src1) => {
                if exec != 0 {
                    // As for a compare, sources are checked once for the wave,
                    // whichever present lanes are active.
                    let check = |register: u8| {
                        (0..lanes).try_for_each(|lane| read(lane, register).map(|_| ()))
                    };
                    match src0 {
                        CompareSource::Vector(register) => check(register)?,
                        CompareSource::Scalar(register) => {
                            read_scalar(Source::Register(register))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                    check(src1)?;
                }
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let s0 = match src0 {
                        CompareSource::Vector(register) => read(lane, register)?,
                        CompareSource::Scalar(register) => read_scalar(Source::Register(register))?,
                        CompareSource::Immediate(bits) => bits,
                    };
                    results[lane] = Some((dst, kind.apply(s0, read(lane, src1)?)));
                }
            }
            Op::ScalarMove(dst, src) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let value = read_scalar(src)?;
                if usize::from(dst) == EXEC_LO {
                    exec = value;
                }
                if usize::from(dst) == VCC_LO {
                    lane_derived_vcc = false;
                }
                scalars[usize::from(dst)] = Some(value);
            }
            Op::ScalarUnary(kind, dst, src) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let value = read_scalar(src)?;
                // -1 (all ones) when no bit is found.
                let found = |index: u32| if index == 32 { u32::MAX } else { index };
                let result = match kind {
                    ScalarUnary::Not => !value,
                    ScalarUnary::ReverseBits => value.reverse_bits(),
                    ScalarUnary::CountZeros => value.count_zeros(),
                    ScalarUnary::CountOnes => value.count_ones(),
                    ScalarUnary::FindFirstZero => found(value.trailing_ones()),
                    ScalarUnary::FindFirstOne => found(value.trailing_zeros()),
                    ScalarUnary::FindLastBit => found(value.leading_zeros()),
                    ScalarUnary::FindLastSignBit => {
                        let same = if value >> 31 == 1 {
                            value.leading_ones()
                        } else {
                            value.leading_zeros()
                        };
                        found(same)
                    }
                    ScalarUnary::SignExtendByte => value as u8 as i8 as i32 as u32,
                    ScalarUnary::SignExtendShort => value as u16 as i16 as i32 as u32,
                    ScalarUnary::Absolute => (value as i32).wrapping_abs() as u32,
                };
                scalars[usize::from(dst)] = Some(result);
                if matches!(
                    kind,
                    ScalarUnary::Not
                        | ScalarUnary::CountZeros
                        | ScalarUnary::CountOnes
                        | ScalarUnary::Absolute
                ) {
                    scc = Some(result != 0);
                }
            }
            Op::CompareEqual(lhs, rhs) => {
                scc = Some(read_scalar(lhs)? == read_scalar(rhs)?);
            }
            // Guide 12.4: SCC = (S0 <relation> S1); no SGPR is written.
            Op::ScalarCompare(comparison, lhs, rhs) => {
                scc = Some(comparison.holds(read_scalar(lhs)?, read_scalar(rhs)?));
            }
            Op::Scalar(kind, dst, lhs, rhs) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let (lhs, rhs) = (i64::from(read_scalar(lhs)?), i64::from(read_scalar(rhs)?));
                let carry = match kind {
                    ScalarAlu::AddWithCarry | ScalarAlu::SubtractWithBorrow => i64::from(
                        scc.ok_or_else(|| error(offset, "carry-in from uninitialized SCC"))?,
                    ),
                    ScalarAlu::Select => {
                        i64::from(scc.ok_or_else(|| error(offset, "select on uninitialized SCC"))?)
                    }
                    _ => 0,
                };
                let (signed_lhs, signed_rhs) =
                    (i64::from(lhs as u32 as i32), i64::from(rhs as u32 as i32));
                // Exact integer arithmetic, then reduction modulo 2^32.
                let (exact, flag) = match kind {
                    ScalarAlu::Add | ScalarAlu::AddWithCarry => {
                        let sum = lhs + rhs + carry;
                        (sum, sum >= 1 << 32)
                    }
                    ScalarAlu::Subtract | ScalarAlu::SubtractWithBorrow => {
                        let difference = lhs - rhs - carry;
                        (difference, difference < 0)
                    }
                    // The exact signed result must fit in i32, else SCC = overflow.
                    ScalarAlu::AddSigned | ScalarAlu::SubtractSigned => {
                        let signed = |value: i64| i64::from(value as u32 as i32);
                        let exact = if kind == ScalarAlu::AddSigned {
                            signed(lhs) + signed(rhs)
                        } else {
                            signed(lhs) - signed(rhs)
                        };
                        (exact, i32::try_from(exact).is_err())
                    }
                    ScalarAlu::And | ScalarAlu::Or | ScalarAlu::Xor => {
                        let bits = match kind {
                            ScalarAlu::And => lhs & rhs,
                            ScalarAlu::Or => lhs | rhs,
                            _ => lhs ^ rhs,
                        };
                        (bits, bits != 0)
                    }
                    // Guide 12.1: strict comparisons, so equal operands give
                    // S1 (the same value) and SCC = 0.
                    ScalarAlu::Minimum { signed } | ScalarAlu::Maximum { signed } => {
                        let (a, b) = if signed {
                            (signed_lhs, signed_rhs)
                        } else {
                            (lhs, rhs)
                        };
                        let first = if matches!(kind, ScalarAlu::Minimum { .. }) {
                            a < b
                        } else {
                            a > b
                        };
                        (if first { lhs } else { rhs }, first)
                    }
                    // The SCC-less forms; `flag` is unused for them below.
                    ScalarAlu::Select => (if carry == 1 { lhs } else { rhs }, false),
                    ScalarAlu::BitfieldMask => (((1 << (lhs & 0x1f)) - 1) << (rhs & 0x1f), false),
                    ScalarAlu::Multiply => (signed_lhs * signed_rhs, false),
                    ScalarAlu::Pack { high0, high1 } => {
                        let low = (lhs >> if high0 { 16 } else { 0 }) & 0xffff;
                        let high = (rhs >> if high1 { 16 } else { 0 }) & 0xffff;
                        (low + high * 65536, false)
                    }
                    ScalarAlu::MultiplyHigh { signed } => {
                        // The unsigned product fits only in u64.
                        let high = if signed {
                            (signed_lhs * signed_rhs) >> 32
                        } else {
                            ((lhs as u64 * rhs as u64) >> 32) as i64
                        };
                        (high, false)
                    }
                    // Offset S1[4:0], width S1[22:16] (0-127). A width of 32 or
                    // more keeps every bit of the shifted word.
                    ScalarAlu::BitfieldExtract { signed } => {
                        let (shift, width) = (rhs & 0x1f, (rhs >> 16) & 0x7f);
                        let data = if signed { signed_lhs } else { lhs };
                        let shifted = data >> shift;
                        let value = if width == 0 {
                            0
                        } else if width >= 32 {
                            shifted
                        } else {
                            let field = shifted & ((1 << width) - 1);
                            if signed && field >> (width - 1) == 1 {
                                field - (1 << width)
                            } else {
                                field
                            }
                        };
                        (value, value.rem_euclid(1 << 32) != 0)
                    }
                    // The 32-bit difference first (guide examples), then |D|.
                    ScalarAlu::AbsoluteDifference => {
                        let difference = (signed_lhs - signed_rhs) as u32 as i32;
                        let value = difference.unsigned_abs();
                        (i64::from(value), value != 0)
                    }
                    // The carry out of the exact (S0 << N) + S1.
                    ScalarAlu::ShiftLeftAdd(n) => {
                        let sum = (lhs << n) + rhs;
                        (sum, sum >= 1 << 32)
                    }
                    // Guide 12.1: shifts use only S1[4:0]; all set SCC = (D != 0).
                    _ => {
                        let (a, b) = (lhs as u32, rhs as u32);
                        let bits = match kind {
                            ScalarAlu::AndNot => a & !b,
                            ScalarAlu::OrNot => a | !b,
                            ScalarAlu::Nand => !(a & b),
                            ScalarAlu::Nor => !(a | b),
                            ScalarAlu::Xnor => !(a ^ b),
                            ScalarAlu::ShiftLeft => a << (b & 0x1f),
                            ScalarAlu::ShiftRight => a >> (b & 0x1f),
                            _ => ((a as i32) >> (b & 0x1f)) as u32,
                        };
                        (i64::from(bits), bits != 0)
                    }
                };
                let value = exact.rem_euclid(1 << 32) as u32;
                scalars[usize::from(dst)] = Some(value);
                // Guide 12.1 and Tables 11, 12, 14: these forms leave SCC unchanged.
                if !matches!(
                    kind,
                    ScalarAlu::Select
                        | ScalarAlu::BitfieldMask
                        | ScalarAlu::Multiply
                        | ScalarAlu::MultiplyHigh { .. }
                        | ScalarAlu::Pack { .. }
                ) {
                    scc = Some(flag);
                }
            }
            // RDNA 2 ISA section 3.9: VCC[n] = EXEC[n] & test(n), fully written.
            Op::Compare(comparison, src0, src1) => {
                let mut mask = 0u32;
                if exec != 0 {
                    // Sources are checked once for the wave, even if no present lane is active.
                    let check = |register: u8| {
                        (0..lanes).try_for_each(|lane| read(lane, register).map(|_| ()))
                    };
                    match src0 {
                        CompareSource::Vector(register) => check(register)?,
                        CompareSource::Scalar(register) => {
                            read_scalar(Source::Register(register))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                    check(src1)?;
                    for lane in (0..lanes).filter(|&lane| executes(lane)) {
                        let s0 = match src0 {
                            CompareSource::Vector(register) => read(lane, register)?,
                            CompareSource::Scalar(register) => {
                                read_scalar(Source::Register(register))?
                            }
                            CompareSource::Immediate(bits) => bits,
                        };
                        if comparison.holds(s0, read(lane, src1)?) {
                            mask |= 1 << lane;
                        }
                    }
                }
                scalars[VCC_LO] = Some(mask);
                scalars[VCC_HI] = None;
                lane_derived_vcc = exec != 0;
            }
            Op::Branch {
                target: destination,
                condition,
            } => {
                if condition == BranchCondition::VccZero && scalars[VCC_LO].is_none() {
                    return Err(error(offset, "branch with uninitialized VCC_LO"));
                }
                if condition == BranchCondition::VccZero && lane_derived_vcc {
                    check_runtime_region(
                        &program[position + 1..],
                        destination,
                        &registers,
                        &scalars,
                        exec,
                        pending_read.is_some(),
                        offset,
                    )?;
                }
                let take_branch = match condition {
                    BranchCondition::Always => true,
                    BranchCondition::SccZero => {
                        !scc.ok_or_else(|| error(offset, "branch with uninitialized SCC"))?
                    }
                    BranchCondition::VccZero => scalars[VCC_LO] == Some(0),
                    BranchCondition::ExecZero => exec == 0,
                };
                if take_branch {
                    target = destination;
                }
            }
            Op::End => {
                if pending_read.is_some() || pending_write.iter().any(Option::is_some) {
                    return Err(error(offset, "outstanding transfer at termination"));
                }
                return output
                    .iter()
                    .map(|value| {
                        value.ok_or_else(|| error(offset, "lane did not write its output"))
                    })
                    .collect();
            }
        }
        for (lane, result) in results.into_iter().enumerate() {
            if let Some((dst, value)) = result {
                if dst == 0 || pending_read == Some(dst) {
                    return Err(error(offset, "write to immutable or pending VGPR"));
                }
                registers[lane][usize::from(dst)] = Some(value);
            }
        }
    }
    Err(error(0, "missing decoded termination"))
}

/// docs/shaders/vector-compare.md section 6: code skipped by `s_cbranch_vccz`
/// on a lane-derived VCC_LO may hold only integer vector ALU instructions
/// whose destinations and sources are ready at the branch. Checked on the
/// code itself, whether or not the branch is taken.
fn check_runtime_region(
    following: &[DecodedInstruction],
    end: usize,
    registers: &[[Option<u32>; 256]],
    scalars: &[Option<u32>; 128],
    exec: u32,
    load_pending: bool,
    branch: usize,
) -> Result<(), ReferenceError> {
    if load_pending {
        return Err(error(branch, "runtime VCC branch with an outstanding load"));
    }
    let ready = |register: u8| {
        register != 0
            && registers
                .iter()
                .all(|lane| lane[usize::from(register)].is_some())
    };
    for instruction in following.iter().take_while(|item| item.offset < end) {
        let offset = instruction.offset;
        let (dst, vector_sources, scalar_source) = match instruction.op {
            Op::Move(dst, Source::Register(src)) => (dst, vec![src], None),
            Op::Move(dst, Source::Immediate(_)) => (dst, vec![], None),
            Op::MoveScalar(dst, src) => (dst, vec![], Some(src)),
            Op::Add(dst, lhs, rhs) => (dst, vec![lhs, rhs], None),
            Op::Vector(kind, dst, lhs, rhs)
                if !matches!(kind, VectorAlu::FloatAdd | VectorAlu::FloatMultiply) =>
            {
                (dst, vec![lhs, rhs], None)
            }
            _ => {
                return Err(error(
                    offset,
                    "runtime VCC branch skips a non-integer-vector instruction",
                ));
            }
        };
        if !ready(dst) {
            return Err(error(
                offset,
                "runtime VCC branch skips a write to an uninitialized VGPR",
            ));
        }
        if exec == 0 {
            continue;
        }
        if vector_sources.into_iter().any(|register| !ready(register)) {
            return Err(error(
                offset,
                "runtime VCC branch skips an unready VGPR read",
            ));
        }
        if let Some(register) = scalar_source
            && scalars[usize::from(register)].is_none()
        {
            return Err(error(
                offset,
                "runtime VCC branch skips an unknown SGPR read",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");

    #[test]
    fn decodes_sopk_signed_immediates_and_normal_destinations() {
        for opcode in [0, 15, 16] {
            for dst in [0, 105, 106, 127] {
                for (imm, bits) in [(0x7fff, 0x7fff), (0x8000, 0xffff_8000), (0xffff, u32::MAX)] {
                    let word = 0xb000_0000u32 | opcode << 23 | dst << 16 | imm;
                    let mut code = word.to_le_bytes().to_vec();
                    code.extend(0xbf81_0000u32.to_le_bytes());
                    let result = decode(&code);
                    if dst > 105 {
                        assert!(result.is_err());
                        continue;
                    }
                    let program = result.unwrap();
                    let dst = dst as u8;
                    let immediate = Source::Immediate(bits);
                    let expected = match opcode {
                        0 => Op::ScalarMove(dst, immediate),
                        15 => {
                            Op::Scalar(ScalarAlu::AddSigned, dst, Source::Register(dst), immediate)
                        }
                        _ => Op::Scalar(ScalarAlu::Multiply, dst, Source::Register(dst), immediate),
                    };
                    assert_eq!(program[0].op, expected);
                    assert_eq!(program[1].offset, 4);
                }
            }
        }
    }

    #[test]
    fn executes_scalar_pack_and_preserves_scc_including_unset() {
        for (opcode, expected) in [(50, 0xdef0_5678), (51, 0x9abc_5678), (52, 0x9abc_1234)] {
            for initial_scc in [None, Some(false), Some(true)] {
                for dst in [8, 9, 10] {
                    let mut words = vec![0xbe88_03ff, 0x1234_5678, 0xbe89_03ff, 0x9abc_def0];
                    if let Some(flag) = initial_scc {
                        words.push(if flag { 0xbf06_8080 } else { 0xbf06_8180 });
                    }
                    words.push(0x8000_0000 | opcode << 23 | dst << 16 | 9 << 8 | 8);
                    let finish = |prefix: &[u32], output: u32| {
                        let mut words = prefix.to_vec();
                        words.extend([
                            0x7e02_0200 | output,
                            0xe070_2000,
                            0x8001_0100,
                            0xbbfd_0000,
                            0xbf81_0000,
                        ]);
                        words
                            .iter()
                            .flat_map(|w| w.to_le_bytes())
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(execute(&finish(&words, dst), &[0]), Ok(vec![expected]));
                    // CSELECT reads SCC: unset must still fail, known flags select
                    // distinct constants rather than depending on the packed value.
                    words.push(0x850a_8281);
                    let result = execute(&finish(&words, 10), &[0]);
                    match initial_scc {
                        None => {
                            assert_eq!(result.unwrap_err().detail, "select on uninitialized SCC")
                        }
                        Some(flag) => assert_eq!(result, Ok(vec![if flag { 1 } else { 2 }])),
                    }
                }
            }
        }
    }

    #[test]
    fn decodes_scalar_pack_forms_and_keeps_operand_restrictions() {
        for (opcode, high0, high1) in [(50, false, false), (51, false, true), (52, true, true)] {
            let word = |dst: u32, a: u32, b: u32| {
                0x8000_0000u32 | (opcode << 23) | (dst << 16) | (b << 8) | a
            };
            let bytes = |words: &[u32]| {
                words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<_>>()
            };
            let code = bytes(&[word(105, 8, 193), 0xbf81_0000]);
            assert_eq!(
                decode(&code).unwrap()[0].op,
                Op::Scalar(
                    ScalarAlu::Pack { high0, high1 },
                    105,
                    Source::Register(8),
                    Source::Immediate(u32::MAX),
                )
            );
            for (a, b) in [(255, 8), (8, 255)] {
                let code = bytes(&[word(8, a, b), 0xabcd_1234, 0xbf81_0000]);
                assert_eq!(decode(&code).unwrap()[0].byte_len, 8);
                assert!(decode(&bytes(&[word(8, a, b)])).is_err());
            }
            for (dst, a, b) in [(106, 8, 9), (8, 106, 9), (8, 8, 106), (8, 255, 255)] {
                assert!(decode(&bytes(&[word(dst, a, b), 0xbf81_0000])).is_err());
            }
        }
    }

    #[test]
    fn computes_the_independent_known_answers() {
        let input = [0, 1, u32::MAX, 0u32.wrapping_sub(0x1234_5678)];
        let expected = [0x1234_5678, 0x1234_5679, 0x1234_5677, 0];
        assert_eq!(execute(FIXTURE, &input), Ok(expected.to_vec()));
    }

    #[test]
    fn executes_both_scalar_branch_outcomes() {
        let fixture = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
        let inputs = [0, u32::MAX, 0x1234_5678];
        assert_eq!(
            execute(fixture, &inputs),
            Ok(vec![0x1234_5678, 0x12cb_5687, 0x1200_5600])
        );
        let mut equal = *fixture;
        equal[17] = 0x87; // Compare s8 with 7, so the SCC-zero edge is not taken.
        assert_eq!(execute(&equal, &inputs), Ok(vec![0; 3]));
    }

    #[test]
    fn rejects_invalid_branch_even_when_not_taken() {
        let fixture = include_bytes!("../../../fixtures/rdna2/bitwise-branch.bin");
        let mut invalid = *fixture;
        invalid[17] = 0x87;
        invalid[20] = 3; // Target byte 36, the VOP1 literal.
        assert_eq!(
            execute(&invalid, &[0]).unwrap_err().detail,
            "branch target is not an instruction boundary"
        );
        invalid[20..22].copy_from_slice(&(-1i16).to_le_bytes());
        assert_eq!(
            execute(&invalid, &[0]).unwrap_err().detail,
            "backward branch"
        );
    }

    #[test]
    fn executes_exact_float_fixture_and_rejects_subnormal_operand() {
        let fixture = include_bytes!("../../../fixtures/rdna2/float-exact.bin");
        assert_eq!(
            execute(fixture, &[0, u32::MAX]),
            Ok(vec![0x410c_0000, 0x4120_0000])
        );
        let mut broken = *fixture;
        broken[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            execute(&broken, &[0]).unwrap_err().detail,
            "float operation outside positive, normal, exact binary32 domain"
        );
    }

    #[test]
    fn rejects_truncated_programs_and_trailing_bytes() {
        for length in 0..FIXTURE.len() {
            assert!(
                execute(&FIXTURE[..length], &[0]).is_err(),
                "length {length}"
            );
        }
        let mut trailing = FIXTURE.to_vec();
        trailing.push(0);
        assert_eq!(execute(&trailing, &[0]).unwrap_err().offset, FIXTURE.len());
    }

    #[test]
    fn rejects_access_before_wait() {
        let mut no_read_wait = FIXTURE.to_vec();
        no_read_wait.drain(8..12);
        assert_eq!(
            execute(&no_read_wait, &[1]).unwrap_err().detail,
            "VGPR read before load wait"
        );
        let mut no_write_wait = FIXTURE.to_vec();
        no_write_wait.drain(36..40);
        assert_eq!(
            execute(&no_write_wait, &[1]).unwrap_err().detail,
            "outstanding transfer at termination"
        );
    }

    #[test]
    fn decodes_high_register_indices() {
        // V_MOV_B32 v255, v254 and V_ADD_NC_U32 v255, v128, v255.
        let code = [
            0xfe, 0x03, 0xfe, 0x7f, 0xff, 0, 0x25, 0xd5, 0x80, 0xff, 0x03, 0x02, 0, 0, 0x81, 0xbf,
        ];
        assert_eq!(
            decode(&code),
            Ok(vec![
                DecodedInstruction {
                    offset: 0,
                    byte_len: 4,
                    op: Op::Move(255, Source::Register(254))
                },
                DecodedInstruction {
                    offset: 4,
                    byte_len: 8,
                    op: Op::Add(255, 128, 255)
                },
                DecodedInstruction {
                    offset: 12,
                    byte_len: 4,
                    op: Op::End
                },
            ])
        );
    }
}
