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

/// Scalar operand spelling for APIs whose register bank is always SGPR.
pub type ScalarSource = Source;

/// Operations decoded independently by the reference interpreter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    DsPermuteB32 {
        dst: u8,
        addr: u8,
        data: u8,
        offset: u16,
    },
    DsBpermuteB32 {
        dst: u8,
        addr: u8,
        data: u8,
        offset: u16,
    },
    DsSwizzleB32 {
        dst: u8,
        src: u8,
        offset: u16,
    },
    Permlane16B32 {
        dst: u8,
        src: u8,
        select_lo: u8,
        select_hi: u8,
        cross_row: bool,
        fetch_inactive: bool,
        bound_control: bool,
    },
    WaitLgkm,
    WriteLane {
        dst: u8,
        src: ScalarSource,
        index: u8,
    },
    /// A VGPR read into an SGPR; None chooses the first EXEC lane (zero when empty).
    ReadLane {
        dst: u8,
        src: u8,
        index: Option<u8>,
    },
    Load {
        data: u8,
        index: u8,
        resource: u8,
    },
    /// Read the low byte or halfword of the lane's four-byte input element.
    LoadNarrow {
        bytes: u8,
        signed: bool,
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
    ScalarBitSet(u8, ScalarSource, bool),
    ScalarBitCompare(ScalarSource, ScalarSource, bool),
    ScalarConditionalMove(u8, ScalarSource),
    ScalarConditionalMoveImmediate(u8, i16),
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
    Vector(VectorAlu, u8, CompareSource, u8),
    MadSad(
        MadSadOp,
        bool,
        u8,
        CompareSource,
        CompareSource,
        CompareSource,
    ),
    Clamped(ClampOp, u8, CompareSource, CompareSource),
    /// VOP2 32-bit integer form: VDST, SRC0 (VGPR, SGPR, `vcc_lo`, inline
    /// integer or literal) and VSRC1. Writes neither VCC nor SCC.
    VectorBinaryAlias(VectorAlias, u8, CompareSource, CompareSource),
    VectorInteger(VectorInteger, u8, CompareSource, u8),
    VectorUnary(VectorUnary, u8, CompareSource),
    VectorProduct(VectorProduct, u8, CompareSource, CompareSource),
    BitBinary(BitBinary, u8, CompareSource, CompareSource),
    VectorTernary(
        VectorTernary,
        u8,
        CompareSource,
        CompareSource,
        CompareSource,
    ),
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorProduct {
    Low,
    HighUnsigned,
    HighSigned,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BitBinary {
    BitFieldMask,
    CountBitsAdd,
    MaskedCountLow,
    MaskedCountHigh,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorTernary {
    MinimumSigned,
    MinimumUnsigned,
    MaximumSigned,
    MaximumUnsigned,
    MedianSigned,
    MedianUnsigned,
    ExtractSigned,
    ShiftAdd,
    AddShift,
    ShiftOr,
    ExtractUnsigned,
    AlignBits,
    OrThree,
    XorThree,
    MadU32U16,
    MadI32I16,
    XorAdd,
    LerpU8,
    MaskedSadU8,
    BitSelect,
    AddThree,
    AndOr,
}

/// Independent VOP1 integer semantics (AMD RDNA 2 section 12.8).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorUnary {
    Not,
    Reverse,
    FirstHighUnsigned,
    FirstLow,
    FirstHighSigned,
}

impl VectorUnary {
    fn apply(self, value: u32) -> u32 {
        match self {
            Self::Not => !value,
            Self::Reverse => value.reverse_bits(),
            Self::FirstLow => {
                if value == 0 {
                    u32::MAX
                } else {
                    value.trailing_zeros()
                }
            }
            Self::FirstHighUnsigned | Self::FirstHighSigned => {
                let search = if self == Self::FirstHighSigned && value & 0x80000000 != 0 {
                    !value
                } else {
                    value
                };
                if search == 0 {
                    u32::MAX
                } else {
                    search.leading_zeros()
                }
            }
        }
    }
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
    /// Take the branch when SCC is set.
    SccOne,
    /// Take the branch when the low half of VCC is clear.
    VccZero,
    /// Take the branch when any bit of the low half of VCC is set.
    VccNonZero,
    /// Take the branch when the low half of EXEC is clear.
    ExecZero,
    /// Take the branch when any bit of the low half of EXEC is set.
    ExecNonZero,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MadSadOp {
    MadI32I24,
    MadU32U24,
    SadU8,
    SadHiU8,
    SadU16,
    SadU32,
}

/// Saturate the mathematical result before narrowing to its 32-bit destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClampOp {
    MulI24,
    MulU24,
    AddU32,
    SubU32,
    SubrevU32,
}

/// Canonical e64 aliases retain two independently decoded generic sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorAlias {
    Legacy(VectorAlu),
    Integer(VectorInteger),
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveLaunch {
    #[default]
    PartialAllowed,
    FullWavesRequired,
}
pub use WaveLaunch as Wave32Launch;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveSize {
    #[default]
    Wave32,
    Wave64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaveConfig {
    pub size: WaveSize,
    pub launch: WaveLaunch,
}
impl WaveSize {
    fn lanes(self) -> usize {
        match self {
            Self::Wave32 => 32,
            Self::Wave64 => 64,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ExecuteOptions {
    pub wave32_launch: Wave32Launch,
}

/// Execute original little-endian gfx1030 bytes with the synthetic binding ABI.
///
/// Each dispatched lane reads the corresponding input word and may write only
/// its own output word. Integer operations are exact modulo 2^32.
pub fn execute(code: &[u8], input: &[u32]) -> Result<Vec<u32>, ReferenceError> {
    execute_with_options(code, input, ExecuteOptions::default())
}

pub fn execute_with_options(
    code: &[u8],
    input: &[u32],
    options: ExecuteOptions,
) -> Result<Vec<u32>, ReferenceError> {
    execute_with_wave_config(
        code,
        input,
        WaveConfig {
            size: WaveSize::Wave32,
            launch: options.wave32_launch,
        },
    )
}

pub fn execute_with_wave_config(
    code: &[u8],
    input: &[u32],
    config: WaveConfig,
) -> Result<Vec<u32>, ReferenceError> {
    let width = config.size.lanes();
    if config.size == WaveSize::Wave64 && config.launch != WaveLaunch::FullWavesRequired {
        return Err(error(0, "Wave64 requires full waves"));
    }
    if config.launch == WaveLaunch::FullWavesRequired && !input.len().is_multiple_of(width) {
        return Err(error(
            0,
            "full wave input must contain a multiple of the wave size",
        ));
    }
    if input.is_empty() || input.len() > MAX_INPUT_WORDS {
        return Err(error(0, "input word count outside reference limits"));
    }
    if code.len() > MAX_CODE_BYTES {
        return Err(error(0, "instruction stream exceeds reference limit"));
    }
    let program = decode(code)?;
    for instruction in &program {
        if config.size == WaveSize::Wave64 {
            match instruction.op {
                Op::ReadLane {
                    dst,
                    index: Some(index),
                    ..
                } if dst == index => {
                    return Err(error(
                        instruction.offset,
                        "Wave64 READLANE scalar source/destination overlap",
                    ));
                }
                Op::Compare(_, CompareSource::Scalar(106 | 107), _) => {
                    return Err(error(
                        instruction.offset,
                        "Wave64 compare scalar source overlaps VCC destination",
                    ));
                }
                _ => {}
            }
        }
        match instruction.op {
            Op::DsSwizzleB32 {
                offset: immediate, ..
            } if (0xc000..0xe000).contains(&immediate) && immediate & 31 != 0 => {
                return Err(error(
                    instruction.offset,
                    "DS_SWIZZLE masked rotate rejected: primary source conflict between AMD examples and pseudocode",
                ));
            }
            Op::DsPermuteB32 { .. }
            | Op::DsBpermuteB32 { .. }
            | Op::DsSwizzleB32 { .. }
            | Op::Permlane16B32 { .. }
                if config.launch != WaveLaunch::FullWavesRequired =>
            {
                return Err(error(
                    instruction.offset,
                    "gathers require full Wave32 launch",
                ));
            }
            Op::End if instruction.offset + instruction.byte_len != code.len() => {
                return Err(error(
                    instruction.offset + instruction.byte_len,
                    "trailing code after s_endpgm",
                ));
            }
            Op::Load {
                index, resource, ..
            }
            | Op::LoadNarrow {
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
    // Waves are consecutive groups of the configured width. Only Wave32
    // permits absent final lanes, which contribute zero to compare masks.
    let mut output = Vec::with_capacity(input.len());
    for wave in input.chunks(width) {
        output.extend(execute_wave(&program, wave, config)?);
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
                lo,
                hi,
                tag @ (0xc8 | 0xcc | 0xd4),
                family @ (0xda | 0xd8),
                tail @ ..,
            ] if matches!((*tag, *family), (0xc8 | 0xcc, 0xda) | (0xd4, 0xd8)) => {
                let [addr, data, unused, dst, ..] = tail else {
                    return Err(error(offset, "truncated DS gather"));
                };
                let immediate = u16::from_le_bytes([*lo, *hi]);
                if *unused != 0 || (*family == 0xd8 && *data != 0) {
                    return Err(error(offset, "unsupported DS gather fields"));
                }
                (
                    if *tag == 0xc8 {
                        Op::DsPermuteB32 {
                            dst: *dst,
                            addr: *addr,
                            data: *data,
                            offset: immediate,
                        }
                    } else if *family == 0xda {
                        Op::DsBpermuteB32 {
                            dst: *dst,
                            addr: *addr,
                            data: *data,
                            offset: immediate,
                        }
                    } else {
                        Op::DsSwizzleB32 {
                            dst: *dst,
                            src: *addr,
                            offset: immediate,
                        }
                    },
                    8,
                )
            }
            [dst, flags, opcode @ (0x77 | 0x78), 0xd7, tail @ ..] => {
                let [a, b, c, d, ..] = tail else {
                    return Err(error(offset, "truncated permlane"));
                };
                let fields = u32::from_le_bytes([*a, *b, *c, *d]);
                let data = fields & 511;
                let lo = (fields >> 9) & 511;
                let hi = (fields >> 18) & 511;
                if flags & !0x18 != 0 || fields >> 27 != 0 || data < 256 || lo > 105 || hi > 105 {
                    return Err(error(offset, "unsupported permlane fields"));
                }
                (
                    Op::Permlane16B32 {
                        dst: *dst,
                        src: data as u8,
                        select_lo: lo as u8,
                        select_hi: hi as u8,
                        cross_row: *opcode == 0x78,
                        fetch_inactive: flags & 8 != 0,
                        bound_control: flags & 16 != 0,
                    },
                    8,
                )
            }
            [0x7f, 0xc0, 0x8c, 0xbf, ..] => (Op::WaitLgkm, 4),
            // Spec-derived VOP3 READFIRST subset: no LLVM encoding oracle exists.
            [dst, flags, 0x82, 0xd5, tail @ ..] => {
                let [src, bank, pad0, pad1, ..] = tail else {
                    return Err(error(offset, "truncated readfirst VOP3"));
                };
                if *dst > 105 || *flags != 0 || *bank != 1 || *pad0 != 0 || *pad1 != 0 {
                    return Err(error(offset, "unsupported readfirst VOP3 fields"));
                }
                (
                    Op::ReadLane {
                        dst: *dst,
                        src: *src,
                        index: None,
                    },
                    8,
                )
            }
            [lo, packed, upper, tag @ 0x7e..=0x7f, ..] if upper & 1 == 0 && packed >> 1 == 2 => {
                let dst = (upper >> 1) | ((tag & 1) << 7);
                if dst > 105 || packed & 1 == 0 {
                    return Err(error(offset, "lane read requires normal SGPR and VGPR"));
                }
                (
                    Op::ReadLane {
                        dst,
                        src: *lo,
                        index: None,
                    },
                    4,
                )
            }
            [dst, flags, 0x61, 0xd7, rest @ ..] => {
                let [a, b, c, d, ..] = rest else {
                    return Err(error(offset, "truncated writelane"));
                };
                let index = (u16::from(*b) >> 1) | (u16::from(*c & 3) << 7);
                // AMD RDNA2 p184 permits constants as data, but this contract
                // keeps the lane selector a normal SGPR and all modifiers clear.
                if *flags != 0
                    || !matches!(*a, 0..=105 | 128..=208 | 240..=248 | 255)
                    || b & 1 != 0
                    || index > 105
                    || c & 0xfc != 0
                    || *d != 2
                {
                    return Err(error(offset, "unsupported writelane operands or modifiers"));
                }
                let (src, source_length) = decode_scalar(*a, &rest[4..], offset)?;
                (
                    Op::WriteLane {
                        dst: *dst,
                        src,
                        index: index as u8,
                    },
                    4 + source_length,
                )
            }
            [dst, flags, 0x60, 0xd7, rest @ ..] => {
                let [a, b, c, d, ..] = rest else {
                    return Err(error(offset, "truncated readlane"));
                };
                let lane = (u16::from(*b) >> 1) | (u16::from(*c & 3) << 7);
                if *flags != 0 || *dst > 105 || b & 1 == 0 || lane > 105 || c & 0xfc != 0 || *d != 2
                {
                    return Err(error(offset, "unsupported readlane operands or modifiers"));
                }
                (
                    Op::ReadLane {
                        dst: *dst,
                        src: *a,
                        index: Some(lane as u8),
                    },
                    8,
                )
            }
            [_, _, _, _, ..]
                if {
                    let first = u32::from_le_bytes(bytes[..4].try_into().unwrap());
                    first & 0xfc000000 == 0xd4000000
                        && matches!(
                            (first >> 16) & 1023,
                            361 | 362
                                | 364
                                | 330
                                | 877
                                | 881
                                | 328
                                | 867
                                | 334
                                | 868
                                | 869
                                | 870
                                | 882
                                | 376
                                | 329
                                | 838
                                | 839
                                | 879
                                | 883
                                | 885
                                | 886
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
                } =>
            {
                decode_vop3_integer(bytes, offset)?
            }
            [
                0,
                0x20,
                opcode @ (0x20 | 0x24 | 0x28 | 0x2c | 0x30 | 0x70),
                0xe0,
                index,
                data,
                group @ 0..=25,
                0x80,
                ..,
            ] => {
                let (data, index, resource) = (*data, *index, *group * 4);
                let op = if *opcode < 0x30 {
                    Op::LoadNarrow {
                        bytes: if *opcode < 0x28 { 1 } else { 2 },
                        signed: *opcode & 4 != 0,
                        data,
                        index,
                        resource,
                    }
                } else if *opcode == 0x30 {
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
            // SOPK s_waitcnt_vmcnt null, 0: only this bounded operand form.
            [0x00, 0x00, 0x7d, 0xbc, ..] => (Op::WaitRead, 4),
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
            [
                source,
                opcode @ (5 | 27 | 29),
                destination @ 0x80..=0xe9,
                0xbe,
                tail @ ..,
            ] => {
                let (source, length) = decode_scalar(*source, tail, offset)?;
                let dst = destination & 0x7f;
                (
                    if *opcode == 5 {
                        Op::ScalarConditionalMove(dst, source)
                    } else {
                        Op::ScalarBitSet(dst, source, *opcode == 29)
                    },
                    length,
                )
            }
            [src, index, opcode @ (12 | 13), 0xbf, tail @ ..] if (*src, *index) != (255, 255) => {
                let (src, a) = decode_scalar(*src, tail, offset)?;
                let (index, b) = decode_scalar(*index, tail, offset)?;
                (Op::ScalarBitCompare(src, index, *opcode == 13), a.max(b))
            }
            [lo, hi, dst @ 0..=105, 0xb1, ..] => (
                Op::ScalarConditionalMoveImmediate(*dst, i16::from_le_bytes([*lo, *hi])),
                4,
            ),
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
            // AMD RDNA 2 ISA guide 12.2, 13.1.2: SOPK has no literal dword.
            // CMPK reads SDST and writes only SCC; unsigned CMPK zero-extends.
            [lo, hi, dst, encoding @ 0xb0..=0xbf, ..]
                if dst & 0x7f <= 105 && matches!((encoding & 0xf) << 1 | dst >> 7, 0 | 3..=16) =>
            {
                let opcode = (encoding & 0xf) << 1 | dst >> 7;
                let dst = dst & 0x7f;
                let immediate = Source::Immediate(if (9..=14).contains(&opcode) {
                    u32::from(u16::from_le_bytes([*lo, *hi]))
                } else {
                    i32::from(i16::from_le_bytes([*lo, *hi])) as u32
                });
                let op = match opcode {
                    0 => Op::ScalarMove(dst, immediate),
                    3 | 9 => Op::CompareEqual(Source::Register(dst), immediate),
                    4..=8 | 10..=14 => Op::ScalarCompare(
                        Comparison {
                            offset: [2, 5, 4, 6, 1, 3][usize::from((opcode - 3) % 6)],
                            signed: opcode < 9,
                        },
                        Source::Register(dst),
                        immediate,
                    ),
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
            [
                lo,
                hi,
                opcode @ (0x82 | 0x84 | 0x85 | 0x86 | 0x87 | 0x88 | 0x89),
                0xbf,
                ..,
            ] => {
                let words = i16::from_le_bytes([*lo, *hi]);
                let words = usize::try_from(words).map_err(|_| error(offset, "backward branch"))?;
                (
                    Op::Branch {
                        target: offset + 4 + words * 4,
                        condition: match opcode {
                            0x84 => BranchCondition::SccZero,
                            0x85 => BranchCondition::SccOne,
                            0x86 => BranchCondition::VccZero,
                            0x87 => BranchCondition::VccNonZero,
                            0x88 => BranchCondition::ExecZero,
                            0x89 => BranchCondition::ExecNonZero,
                            _ => BranchCondition::Always,
                        },
                    },
                    4,
                )
            }
            [first, second, third, opcode, tail @ ..]
                if matches!(
                    opcode & 0xfe,
                    0x06 | 0x10 | 0x2c | 0x30 | 0x34 | 0x36 | 0x38 | 0x3a
                ) =>
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
                let (src0, length) =
                    if matches!(kind, VectorAlu::FloatAdd | VectorAlu::FloatMultiply) {
                        if second & 1 == 0 {
                            return Err(error(offset, "floating VOP2 requires VGPR SRC0"));
                        }
                        (CompareSource::Vector(*first), 4)
                    } else {
                        decode_vector_source(
                            *first,
                            *second,
                            tail,
                            offset,
                            "unsupported legacy VOP2 source",
                        )?
                    };
                (Op::Vector(kind, dst, src0, src1), length)
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
                if upper & 1 == 0 && (55..=59).contains(&(packed >> 1)) =>
            {
                let kind = match packed >> 1 {
                    55 => VectorUnary::Not,
                    56 => VectorUnary::Reverse,
                    57 => VectorUnary::FirstHighUnsigned,
                    58 => VectorUnary::FirstLow,
                    _ => VectorUnary::FirstHighSigned,
                };
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let (src, length) = decode_vector_source(
                    *lo,
                    *packed,
                    tail,
                    offset,
                    "unsupported VOP1 integer source",
                )?;
                (Op::VectorUnary(kind, dst, src), length)
            }
            [lo, packed, upper, tag @ 0x7e..=0x7f, tail @ ..]
                if packed & 0xfe == 2 && upper & 1 == 0 =>
            {
                let dst = (upper >> 1) | ((tag & 1) << 7);
                let selector = u16::from(*lo) | (u16::from(packed & 1) << 8);
                if matches!(selector, 0..=107 | 126..=127) {
                    // Normal SGPRs and independently tracked VCC/EXEC halves.
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
                    240 => (Source::Immediate(0x3f000000), 4),
                    241 => (Source::Immediate(0xbf000000), 4),
                    242 => (Source::Immediate(0x3f800000), 4),
                    243 => (Source::Immediate(0xbf800000), 4),
                    244 => (Source::Immediate(0x40000000), 4),
                    245 => (Source::Immediate(0xc0000000), 4),
                    246 => (Source::Immediate(0x40800000), 4),
                    247 => (Source::Immediate(0xc0800000), 4),
                    248 => (Source::Immediate(0x3e22f983), 4),
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
        0..=107 | 126..=127 => Ok((Source::Register(selector), 4)),
        128..=192 => Ok((Source::Immediate(u32::from(selector - 128)), 4)),
        193..=208 => Ok((
            Source::Immediate(0u32.wrapping_sub(u32::from(selector - 192))),
            4,
        )),
        240 => Ok((Source::Immediate(0x3f000000), 4)),
        241 => Ok((Source::Immediate(0xbf000000), 4)),
        242 => Ok((Source::Immediate(0x3f800000), 4)),
        243 => Ok((Source::Immediate(0xbf800000), 4)),
        244 => Ok((Source::Immediate(0x40000000), 4)),
        245 => Ok((Source::Immediate(0xc0000000), 4)),
        246 => Ok((Source::Immediate(0x40800000), 4)),
        247 => Ok((Source::Immediate(0xc0800000), 4)),
        248 => Ok((Source::Immediate(0x3e22f983), 4)),
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
fn decode_vop3_integer(bytes: &[u8], offset: usize) -> Result<(Op, usize), ReferenceError> {
    let first = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let second = u32::from_le_bytes(
        bytes
            .get(4..8)
            .ok_or_else(|| error(offset, "truncated VOP3 instruction"))?
            .try_into()
            .unwrap(),
    );
    let clamp = first & 0x8000 != 0;
    let opcode = (first >> 16) & 1023;
    if first & 0x7f00 != 0
        || second >> 27 != 0
        || (clamp
            && !matches!(
                opcode,
                265 | 267 | 293 | 294 | 295 | 322 | 323 | 346 | 347 | 348 | 349
            ))
    {
        return Err(error(offset, "unsupported VOP3 modifiers"));
    }
    let binary = matches!(
        opcode,
        886 | 361
            | 362
            | 364
            | 867
            | 868
            | 869
            | 870
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
    );
    let unary = matches!(opcode, 439..=443);
    let raw = [second & 511, (second >> 9) & 511, (second >> 18) & 511];
    if unary && (raw[1] != 128 || raw[2] != 128) {
        return Err(error(offset, "noncanonical VOP3 unary SRC1/SRC2"));
    }
    if binary && raw[2] != 128 {
        return Err(error(offset, "noncanonical VOP3 binary SRC2"));
    }
    let active = &raw[..if unary {
        1
    } else if binary {
        2
    } else {
        3
    }];
    let mut keys = std::collections::BTreeSet::new();
    for &src in active {
        if !matches!(src, 0..=107 | 126..=208 | 240..=248 | 255..=511) {
            return Err(error(offset, "unsupported VOP3 source"));
        }
        if src <= 107 || matches!(src, 126 | 127 | 255) {
            keys.insert(src);
        }
    }
    if keys.len() > 2 {
        return Err(error(
            offset,
            "VOP3 scalar bus exceeds two distinct identities",
        ));
    }
    let literal = active.contains(&255);
    if literal && bytes.len() < 12 {
        return Err(error(offset, "truncated VOP3 literal"));
    }
    // High literal halves remain a deferred subset, not architecturally invalid.
    if matches!(opcode, 883 | 885)
        && raw[..2].contains(&255)
        && u32::from_le_bytes(bytes[8..12].try_into().unwrap()) > 0xffff
    {
        return Err(error(offset, "unsupported MAD16 high literal half"));
    }
    let mut args = [CompareSource::Immediate(0); 3];
    for (i, &src) in active.iter().enumerate() {
        args[i] = decode_vector_source(
            src as u8,
            (src >> 8) as u8,
            &bytes[8..],
            offset,
            "unsupported VOP3 source",
        )?
        .0;
    }
    // AMD RDNA2 Table 20: named floats use the source operand width.
    // MAD16 multiplicands are HALF raw bits; the addend remains full32.
    if matches!(opcode, 883 | 885) {
        for i in 0..2 {
            if (240..=248).contains(&raw[i]) {
                args[i] = CompareSource::Immediate(
                    [
                        0x3800, 0xb800, 0x3c00, 0xbc00, 0x4000, 0xc000, 0x4400, 0xc400, 0x3118,
                    ][(raw[i] - 240) as usize],
                );
            }
        }
    }
    let dst = first as u8;
    let mad_sad = match opcode {
        322 => Some(MadSadOp::MadI32I24),
        323 => Some(MadSadOp::MadU32U24),
        346 => Some(MadSadOp::SadU8),
        347 => Some(MadSadOp::SadHiU8),
        348 => Some(MadSadOp::SadU16),
        349 => Some(MadSadOp::SadU32),
        _ => None,
    };
    if let Some(kind) = mad_sad {
        return Ok((
            Op::MadSad(kind, clamp, dst, args[0], args[1], args[2]),
            if literal { 12 } else { 8 },
        ));
    }
    if clamp {
        let kind = match opcode {
            265 => ClampOp::MulI24,
            267 => ClampOp::MulU24,
            293 => ClampOp::AddU32,
            294 => ClampOp::SubU32,
            295 => ClampOp::SubrevU32,
            _ => unreachable!(),
        };
        return Ok((
            Op::Clamped(kind, dst, args[0], args[1]),
            if literal { 12 } else { 8 },
        ));
    }
    let op = match opcode {
        293 if matches!(
            (args[0], args[1]),
            (CompareSource::Vector(_), CompareSource::Vector(_))
        ) =>
        {
            Op::Add(dst, raw[0] as u8, raw[1] as u8)
        }
        265 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(9).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        266 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(10).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        267 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(11).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        268 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(12).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        273 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(17).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        274 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(18).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        275 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(19).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        276 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(20).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        286 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(30).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        293 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(37).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        294 | 886 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(38).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        295 => Op::VectorBinaryAlias(
            VectorAlias::Integer(VectorInteger::from_opcode(39).unwrap()),
            dst,
            args[0],
            args[1],
        ),
        282 => Op::VectorBinaryAlias(
            VectorAlias::Legacy(VectorAlu::ShiftLeft),
            dst,
            args[0],
            args[1],
        ),
        278 => Op::VectorBinaryAlias(
            VectorAlias::Legacy(VectorAlu::ShiftRight),
            dst,
            args[0],
            args[1],
        ),
        280 => Op::VectorBinaryAlias(
            VectorAlias::Legacy(VectorAlu::ArithmeticShiftRight),
            dst,
            args[0],
            args[1],
        ),
        283 => Op::VectorBinaryAlias(VectorAlias::Legacy(VectorAlu::And), dst, args[0], args[1]),
        284 => Op::VectorBinaryAlias(VectorAlias::Legacy(VectorAlu::Or), dst, args[0], args[1]),
        285 => Op::VectorBinaryAlias(VectorAlias::Legacy(VectorAlu::Xor), dst, args[0], args[1]),
        439 => Op::VectorUnary(VectorUnary::Not, dst, args[0]),
        440 => Op::VectorUnary(VectorUnary::Reverse, dst, args[0]),
        441 => Op::VectorUnary(VectorUnary::FirstHighUnsigned, dst, args[0]),
        442 => Op::VectorUnary(VectorUnary::FirstLow, dst, args[0]),
        443 => Op::VectorUnary(VectorUnary::FirstHighSigned, dst, args[0]),

        361 => Op::VectorProduct(VectorProduct::Low, dst, args[0], args[1]),
        362 => Op::VectorProduct(VectorProduct::HighUnsigned, dst, args[0], args[1]),
        364 => Op::VectorProduct(VectorProduct::HighSigned, dst, args[0], args[1]),
        338 => Op::VectorTernary(VectorTernary::MinimumSigned, dst, args[0], args[1], args[2]),
        339 => Op::VectorTernary(
            VectorTernary::MinimumUnsigned,
            dst,
            args[0],
            args[1],
            args[2],
        ),
        341 => Op::VectorTernary(VectorTernary::MaximumSigned, dst, args[0], args[1], args[2]),
        342 => Op::VectorTernary(
            VectorTernary::MaximumUnsigned,
            dst,
            args[0],
            args[1],
            args[2],
        ),
        344 => Op::VectorTernary(VectorTernary::MedianSigned, dst, args[0], args[1], args[2]),
        345 => Op::VectorTernary(
            VectorTernary::MedianUnsigned,
            dst,
            args[0],
            args[1],
            args[2],
        ),
        329 => Op::VectorTernary(VectorTernary::ExtractSigned, dst, args[0], args[1], args[2]),
        883 => Op::VectorTernary(VectorTernary::MadU32U16, dst, args[0], args[1], args[2]),
        885 => Op::VectorTernary(VectorTernary::MadI32I16, dst, args[0], args[1], args[2]),
        837 => Op::VectorTernary(VectorTernary::XorAdd, dst, args[0], args[1], args[2]),
        333 => Op::VectorTernary(VectorTernary::LerpU8, dst, args[0], args[1], args[2]),
        369 => Op::VectorTernary(VectorTernary::MaskedSadU8, dst, args[0], args[1], args[2]),
        838 => Op::VectorTernary(VectorTernary::ShiftAdd, dst, args[0], args[1], args[2]),
        839 => Op::VectorTernary(VectorTernary::AddShift, dst, args[0], args[1], args[2]),
        879 => Op::VectorTernary(VectorTernary::ShiftOr, dst, args[0], args[1], args[2]),
        330 => Op::VectorTernary(VectorTernary::BitSelect, dst, args[0], args[1], args[2]),
        877 => Op::VectorTernary(VectorTernary::AddThree, dst, args[0], args[1], args[2]),
        881 => Op::VectorTernary(VectorTernary::AndOr, dst, args[0], args[1], args[2]),
        867 => Op::BitBinary(BitBinary::BitFieldMask, dst, args[0], args[1]),
        868 => Op::BitBinary(BitBinary::CountBitsAdd, dst, args[0], args[1]),
        869 => Op::BitBinary(BitBinary::MaskedCountLow, dst, args[0], args[1]),
        870 => Op::BitBinary(BitBinary::MaskedCountHigh, dst, args[0], args[1]),
        328 => Op::VectorTernary(
            VectorTernary::ExtractUnsigned,
            dst,
            args[0],
            args[1],
            args[2],
        ),
        334 => Op::VectorTernary(VectorTernary::AlignBits, dst, args[0], args[1], args[2]),
        882 => Op::VectorTernary(VectorTernary::OrThree, dst, args[0], args[1], args[2]),
        376 => Op::VectorTernary(VectorTernary::XorThree, dst, args[0], args[1], args[2]),
        _ => return Err(error(offset, "unsupported VOP3 integer opcode")),
    };
    Ok((op, if literal { 12 } else { 8 }))
}

fn decode_vector_source(
    lo: u8,
    packed: u8,
    tail: &[u8],
    offset: usize,
    unsupported: &'static str,
) -> Result<(CompareSource, usize), ReferenceError> {
    let selector = u16::from(lo) | (u16::from(packed & 1) << 8);
    Ok(match selector {
        0..=107 | 126..=127 => (CompareSource::Scalar(lo), 4),
        128..=192 => (CompareSource::Immediate(u32::from(selector - 128)), 4),
        193..=208 => (
            CompareSource::Immediate(0_u32.wrapping_sub(u32::from(selector - 192))),
            4,
        ),
        240 => (CompareSource::Immediate(0x3f000000), 4),
        241 => (CompareSource::Immediate(0xbf000000), 4),
        242 => (CompareSource::Immediate(0x3f800000), 4),
        243 => (CompareSource::Immediate(0xbf800000), 4),
        244 => (CompareSource::Immediate(0x40000000), 4),
        245 => (CompareSource::Immediate(0xc0000000), 4),
        246 => (CompareSource::Immediate(0x40800000), 4),
        247 => (CompareSource::Immediate(0xc0800000), 4),
        248 => (CompareSource::Immediate(0x3e22f983), 4),
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
fn execute_wave(
    program: &[DecodedInstruction],
    input: &[u32],
    config: WaveConfig,
) -> Result<Vec<u32>, ReferenceError> {
    let launch = config.launch;
    let width = config.size.lanes();
    let full_exec = if width == 64 {
        u64::MAX
    } else {
        u64::from(u32::MAX)
    };
    let lanes = input.len();
    let mut registers = vec![[None::<u32>; 256]; lanes];
    let mut scalars = [None; 128];
    let mut lane_scalars = [None::<u32>; 106];
    let mut exec = full_exec;
    scalars[EXEC_LO] = Some(exec as u32);
    // Wave32 does not establish an initial high-half value.
    scalars[127] = if width == 64 { Some(u32::MAX) } else { None };
    let mut scc = None;
    // Per-half provenance survives writes to the other half.
    let mut lane_derived_vcc = [false; 2];
    let mut target = 0;
    let mut pending_read = None;
    let mut pending_ds: Option<(u8, Vec<u32>)> = None;
    let mut pending_format = None;
    let mut pending_write = vec![None; lanes];
    let mut output = vec![None; lanes];
    for (position, &DecodedInstruction { offset, op, .. }) in program.iter().enumerate() {
        if offset < target {
            continue;
        }
        let scalar_write = match op {
            Op::ScalarMove(dst, _)
            | Op::ScalarUnary(_, dst, _)
            | Op::Scalar(_, dst, _, _)
            | Op::ScalarBitSet(dst, _, _) => Some(dst),
            Op::ScalarConditionalMove(dst, _) | Op::ScalarConditionalMoveImmediate(dst, _)
                if scc == Some(true) =>
            {
                Some(dst)
            }
            _ => None,
        };
        if let Some(dst) = scalar_write.and_then(|dst| lane_scalars.get_mut(usize::from(dst))) {
            *dst = None;
        }
        let read_scalar = |source| match source {
            Source::Register(reg @ (106 | 107)) if lane_derived_vcc[usize::from(reg - 106)] => {
                Err(error(offset, "runtime mask scalar ALU source unsupported"))
            }
            Source::Register(reg) => scalars[usize::from(reg)]
                .ok_or_else(|| error(offset, "read from uninitialized SGPR")),
            Source::Immediate(value) => Ok(value),
        };
        let read_vector_scalar = |source| match source {
            Source::Register(reg) => scalars[usize::from(reg)]
                .ok_or_else(|| error(offset, "read from uninitialized SGPR")),
            Source::Immediate(value) => Ok(value),
        };
        let read = |lane: usize, register: u8| {
            if register == 0 {
                return Err(error(offset, "v0 is reserved for the resource index"));
            }
            if pending_read == Some(register)
                || pending_ds.as_ref().is_some_and(|(dst, _)| *dst == register)
            {
                return Err(error(offset, "VGPR read before load wait"));
            }
            registers[lane][usize::from(register)]
                .ok_or_else(|| error(offset, "read from uninitialized VGPR"))
        };
        // Lanes that execute a vector ALU instruction under the current EXEC.
        let executes = |lane: usize| exec & (1 << lane) != 0;
        if exec != full_exec {
            match op {
                Op::Move(dst, _)
                | Op::MoveScalar(dst, _)
                | Op::Add(dst, ..)
                | Op::Vector(_, dst, ..)
                | Op::BitBinary(_, dst, ..)
                | Op::VectorProduct(_, dst, ..)
                | Op::VectorTernary(_, dst, ..)
                | Op::MadSad(_, _, dst, ..)
                | Op::Clamped(_, dst, ..)
                | Op::VectorBinaryAlias(_, dst, ..)
                | Op::VectorInteger(_, dst, ..)
                | Op::VectorUnary(_, dst, _) => {
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
                        && scalars[usize::from(src)].is_none()
                        && (src > 105 || lane_scalars[usize::from(src)].is_none())
                    {
                        return Err(error(offset, "read from uninitialized SGPR"));
                    }
                }
                Op::Load { .. } | Op::LoadNarrow { .. } | Op::Store { .. } => {
                    return Err(error(offset, "vector memory requires full Wave32 EXEC"));
                }
                _ => {}
            }
        }
        // Per-lane results of a vector ALU instruction, applied after all lanes read.
        let mut results: Vec<Option<(u8, u32)>> = vec![None; lanes];
        match op {
            Op::WaitLgkm => {
                if let Some((dst, values)) = pending_ds.take() {
                    for (lane, value) in values.into_iter().enumerate() {
                        results[lane] = Some((dst, value));
                    }
                }
            }
            Op::DsPermuteB32 {
                dst,
                addr,
                data,
                offset: immediate,
            } => {
                if dst == 0 || pending_read == Some(dst) || pending_ds.is_some() {
                    return Err(error(
                        offset,
                        "invalid scatter destination or outstanding DS",
                    ));
                }
                let mut scattered = vec![0; lanes];
                // Ascending active writers choose the highest numbered contender,
                // one of the outcomes permitted by the primary ISA prose.
                for writer in 0..lanes {
                    let address = read(writer, addr)?;
                    let value = read(writer, data)?;
                    if exec & (1 << writer) != 0 {
                        let destination = (writer / 32) * 32
                            + (address.wrapping_add(u32::from(immediate)) % 128 / 4) as usize;
                        scattered[destination] = value;
                    }
                }
                for (lane, value) in scattered.iter_mut().enumerate() {
                    if exec & (1 << lane) == 0 {
                        *value = read(lane, dst)?;
                    }
                }
                pending_ds = Some((dst, scattered));
            }
            Op::DsBpermuteB32 { dst, .. }
            | Op::DsSwizzleB32 { dst, .. }
            | Op::Permlane16B32 { dst, .. } => {
                let is_ds = !matches!(op, Op::Permlane16B32 { .. });
                if dst == 0
                    || pending_read == Some(dst)
                    || pending_ds.as_ref().is_some_and(|(r, _)| *r == dst)
                    || (is_ds && pending_ds.is_some())
                {
                    return Err(error(
                        offset,
                        "invalid gather destination or outstanding DS",
                    ));
                }
                if !matches!(op, Op::DsBpermuteB32 { .. }) && exec != full_exec {
                    return Err(error(offset, "swizzle/permlane requires full EXEC"));
                }
                let selectors = if let Op::Permlane16B32 {
                    select_lo,
                    select_hi,
                    ..
                } = op
                {
                    (
                        read_scalar(Source::Register(select_lo))?,
                        read_scalar(Source::Register(select_hi))?,
                    )
                } else {
                    (0, 0)
                };
                let mut values = Vec::with_capacity(lanes);
                for lane in 0..lanes {
                    let (source, selected) = match op {
                        Op::DsBpermuteB32 {
                            addr,
                            data,
                            offset: immediate,
                            ..
                        } => {
                            let address = read(lane, addr)?;
                            // Independently model byte addressing modulo the 128-byte Wave32 bank.
                            (
                                data,
                                (lane / 32) * 32
                                    + (address.wrapping_add(u32::from(immediate)) % 128 / 4)
                                        as usize,
                            )
                        }
                        Op::DsSwizzleB32 {
                            src,
                            offset: immediate,
                            ..
                        } => {
                            let o = usize::from(immediate);
                            let mask = o % 32;
                            let target = match o {
                                0xe000.. => {
                                    let reversed = (0..5)
                                        .fold(0, |bits, b| bits | (((lane >> b) & 1) << (4 - b)));
                                    (reversed >> mask.count_ones()) | (lane & mask)
                                }
                                0xc000.. => {
                                    let amount = (o / 32) % 32;
                                    let rotated = if o & 1024 == 0 {
                                        lane + amount
                                    } else {
                                        lane + 32 - amount
                                    };
                                    (lane & mask) | (rotated & !mask)
                                }
                                0x8000.. => (lane / 4) * 4 + ((o >> (2 * (lane % 4))) & 3),
                                _ => ((lane & mask) | ((o / 32) % 32)) ^ ((o / 1024) % 32),
                            };
                            (src, (lane / 32) * 32 + target % 32)
                        }
                        Op::Permlane16B32 { src, cross_row, .. } => {
                            let within = lane % 16;
                            let packed = if within < 8 { selectors.0 } else { selectors.1 };
                            let selected = ((packed >> (4 * (within % 8))) & 15) as usize;
                            (src, ((lane / 16) ^ usize::from(cross_row)) * 16 + selected)
                        }
                        _ => unreachable!(),
                    };
                    // Validate the initialized source snapshot even when EXEC disables the fetch.
                    let source_value = read(selected, source)?;
                    let value = if exec & (1 << lane) == 0 {
                        read(lane, dst)?
                    } else if exec & (1 << selected) == 0 {
                        0
                    } else {
                        source_value
                    };
                    values.push(value);
                }
                if is_ds {
                    pending_ds = Some((dst, values));
                } else {
                    for (lane, value) in values.into_iter().enumerate() {
                        results[lane] = Some((dst, value));
                    }
                }
            }
            Op::WriteLane { dst, src, index } => {
                // Resolve both scalar snapshots before inspecting or changing any lane.
                // Runtime selectors are admitted only under the explicit full-wave contract.
                let selector = if launch == Wave32Launch::FullWavesRequired {
                    scalars[usize::from(index)]
                        .or(lane_scalars[usize::from(index)])
                        .ok_or_else(|| error(offset, "read from uninitialized SGPR"))?
                } else {
                    read_scalar(Source::Register(index))?
                };
                let selected = selector & (width as u32 - 1);
                let value = match src {
                    Source::Immediate(bits) => bits,
                    Source::Register(register) => scalars[usize::from(register)]
                        .or(lane_scalars[usize::from(register)])
                        .ok_or_else(|| error(offset, "read from uninitialized SGPR"))?,
                };
                for (lane, result) in results.iter_mut().enumerate() {
                    let old = read(lane, dst)?;
                    *result = Some((dst, if lane as u32 == selected { value } else { old }));
                }
            }
            Op::ReadLane { dst, src, index } => {
                if dst < 8 {
                    return Err(error(offset, "lane read writes resource descriptor"));
                }
                let lane = if let Some(index) = index {
                    let value = if launch == Wave32Launch::FullWavesRequired {
                        scalars[usize::from(index)]
                            .or(lane_scalars[usize::from(index)])
                            .ok_or_else(|| error(offset, "read from uninitialized SGPR"))?
                    } else {
                        read_scalar(Source::Register(index))?
                    };
                    (value & (width as u32 - 1)) as usize
                } else {
                    (0..width).find(|bit| exec & (1 << bit) != 0).unwrap_or(0)
                };
                if lane != 0 && launch == Wave32Launch::PartialAllowed {
                    return Err(error(
                        offset,
                        "lane read requires selected lane zero for partial-wave safety",
                    ));
                }
                let value = read(lane, src)?;
                scalars[usize::from(dst)] = None;
                lane_scalars[usize::from(dst)] = Some(value);
            }
            Op::Load { data: dst, .. } | Op::LoadNarrow { data: dst, .. } => {
                if dst == 0
                    || pending_read.is_some()
                    || pending_ds.as_ref().is_some_and(|(r, _)| *r == dst)
                {
                    return Err(error(
                        offset,
                        "invalid load destination or overlapping load",
                    ));
                }
                pending_read = Some(dst);
                pending_format = match op {
                    Op::LoadNarrow { bytes, signed, .. } => Some((bytes, signed)),
                    _ => None,
                };
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
                        let value = match pending_format {
                            Some((1, false)) => u32::from(word.to_le_bytes()[0]),
                            Some((1, true)) => i32::from(word.to_le_bytes()[0] as i8) as u32,
                            Some((2, signed)) => {
                                let b = word.to_le_bytes();
                                let half = u16::from_le_bytes([b[0], b[1]]);
                                if signed {
                                    i32::from(half as i16) as u32
                                } else {
                                    u32::from(half)
                                }
                            }
                            None => word,
                            _ => unreachable!("decoder admits only byte or halfword loads"),
                        };
                        results[lane] = Some((dst, value));
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
                    let value = scalars[usize::from(src)]
                        .or_else(|| lane_scalars.get(usize::from(src)).copied().flatten())
                        .ok_or_else(|| error(offset, "read from uninitialized SGPR"))?;
                    results[lane] = Some((dst, value));
                }
            }
            Op::Vector(operator, dst, lhs, rhs) => {
                // The expanded integer domain has wave-uniform readiness checks,
                // including present inactive lanes. Preserve the old FP path.
                if exec != 0 && !matches!(operator, VectorAlu::FloatAdd | VectorAlu::FloatMultiply)
                {
                    for lane in 0..lanes {
                        if let CompareSource::Vector(reg) = lhs {
                            read(lane, reg)?;
                        }
                        read(lane, rhs)?;
                    }
                    if let CompareSource::Scalar(reg) = lhs {
                        read_vector_scalar(Source::Register(reg))?;
                    }
                }
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let left = match lhs {
                        CompareSource::Vector(reg) => read(lane, reg)?,
                        CompareSource::Scalar(reg) => read_vector_scalar(Source::Register(reg))?,
                        CompareSource::Immediate(bits) => bits,
                    };
                    let value = vector_alu(operator, left, read(lane, rhs)?, offset)?;
                    results[lane] = Some((dst, value));
                }
            }
            Op::VectorUnary(kind, dst, src) => {
                if exec != 0 {
                    match src {
                        CompareSource::Vector(reg) => {
                            for lane in 0..lanes {
                                read(lane, reg)?;
                            }
                        }
                        CompareSource::Scalar(reg) => {
                            read_vector_scalar(Source::Register(reg))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                }
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let value = match src {
                        CompareSource::Vector(reg) => read(lane, reg)?,
                        CompareSource::Scalar(reg) => read_vector_scalar(Source::Register(reg))?,
                        CompareSource::Immediate(bits) => bits,
                    };
                    results[lane] = Some((dst, kind.apply(value)));
                }
            }
            Op::MadSad(_, _, dst, a, b, _)
            | Op::Clamped(_, dst, a, b)
            | Op::VectorBinaryAlias(_, dst, a, b)
            | Op::VectorProduct(_, dst, a, b)
            | Op::BitBinary(_, dst, a, b)
            | Op::VectorTernary(_, dst, a, b, _) => {
                let c = match op {
                    Op::MadSad(_, _, _, _, _, c) | Op::VectorTernary(_, _, _, _, c) => c,
                    _ => CompareSource::Immediate(0),
                };
                let source = |lane, src| match src {
                    CompareSource::Vector(reg) => read(lane, reg),
                    CompareSource::Scalar(reg) => read_vector_scalar(Source::Register(reg)),
                    CompareSource::Immediate(bits) => Ok(bits),
                };
                if exec != 0 {
                    for lane in 0..lanes {
                        for src in [a, b, c] {
                            source(lane, src)?;
                        }
                    }
                }
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let (a, b, c) = (source(lane, a)?, source(lane, b)?, source(lane, c)?);
                    let value = match op {
                        Op::MadSad(kind, clamp, ..) => {
                            let signed24 = |x: u32| i64::from(((x << 8) as i32) >> 8);
                            let result = match kind {
                                MadSadOp::MadI32I24 => {
                                    signed24(a) * signed24(b) + i64::from(c as i32)
                                }
                                MadSadOp::MadU32U24 => {
                                    i64::from(a & 0xffffff) * i64::from(b & 0xffffff) + i64::from(c)
                                }
                                _ => {
                                    let width = match kind {
                                        MadSadOp::SadU8 | MadSadOp::SadHiU8 => 8,
                                        MadSadOp::SadU16 => 16,
                                        _ => 32,
                                    };
                                    let mask = u32::MAX >> (32 - width);
                                    let sum: i64 = (0..32 / width)
                                        .map(|i| {
                                            i64::from(
                                                ((a >> (i * width)) & mask)
                                                    .abs_diff((b >> (i * width)) & mask),
                                            )
                                        })
                                        .sum();
                                    (if kind == MadSadOp::SadHiU8 {
                                        sum << 16
                                    } else {
                                        sum
                                    }) + i64::from(c)
                                }
                            };
                            if !clamp {
                                result as u32
                            } else if kind == MadSadOp::MadI32I24 {
                                result.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as u32
                            } else {
                                result.clamp(0, i64::from(u32::MAX)) as u32
                            }
                        }
                        Op::Clamped(kind, ..) => {
                            let signed24 = |x: u32| i64::from(((x << 8) as i32) >> 8);
                            let result = match kind {
                                ClampOp::MulI24 => signed24(a) * signed24(b),
                                ClampOp::MulU24 => {
                                    i64::from(a & 0xffffff) * i64::from(b & 0xffffff)
                                }
                                ClampOp::AddU32 => i64::from(a) + i64::from(b),
                                ClampOp::SubU32 => i64::from(a) - i64::from(b),
                                ClampOp::SubrevU32 => i64::from(b) - i64::from(a),
                            };
                            let (low, high) = if kind == ClampOp::MulI24 {
                                (i64::from(i32::MIN), i64::from(i32::MAX))
                            } else {
                                (0, i64::from(u32::MAX))
                            };
                            result.clamp(low, high) as u32
                        }
                        Op::VectorBinaryAlias(VectorAlias::Integer(kind), ..) => kind.apply(a, b),
                        Op::VectorBinaryAlias(VectorAlias::Legacy(kind), ..) => {
                            vector_alu(kind, a, b, offset)?
                        }
                        Op::VectorProduct(VectorProduct::Low, ..) => {
                            (u64::from(a) * u64::from(b)) as u32
                        }
                        Op::VectorProduct(VectorProduct::HighUnsigned, ..) => {
                            ((u64::from(a) * u64::from(b)) >> 32) as u32
                        }
                        Op::VectorProduct(VectorProduct::HighSigned, ..) => {
                            ((i64::from(a as i32) * i64::from(b as i32)) >> 32) as u32
                        }
                        // AMD RDNA2 pp170,178,181: independent byte components,
                        // reference-byte (SRC1) masking, and unsigned wrap respectively.
                        // AMD RDNA2 p185: 16-bit multiplicands, full 32-bit addend.
                        Op::VectorTernary(VectorTernary::MadU32U16, ..) => u32::from(a as u16)
                            .wrapping_mul(u32::from(b as u16))
                            .wrapping_add(c),
                        Op::VectorTernary(VectorTernary::MadI32I16, ..) => {
                            (i32::from(a as i16) * i32::from(b as i16)).wrapping_add(c as i32)
                                as u32
                        }
                        Op::VectorTernary(VectorTernary::XorAdd, ..) => (a ^ b).wrapping_add(c),
                        Op::VectorTernary(VectorTernary::LerpU8, ..) => {
                            let aa = a.to_le_bytes();
                            let bb = b.to_le_bytes();
                            let rounding = c.to_le_bytes();
                            let bytes = std::array::from_fn(|i| {
                                ((u16::from(aa[i]) + u16::from(bb[i]) + u16::from(rounding[i] & 1))
                                    / 2) as u8
                            });
                            u32::from_le_bytes(bytes)
                        }
                        Op::VectorTernary(VectorTernary::MaskedSadU8, ..) => a
                            .to_le_bytes()
                            .into_iter()
                            .zip(b.to_le_bytes())
                            .fold(c, |sum, (sample, reference)| {
                                if reference == 0 {
                                    sum
                                } else {
                                    sum.wrapping_add(u32::from(sample.abs_diff(reference)))
                                }
                            }),
                        Op::VectorTernary(VectorTernary::BitSelect, ..) => (a & b) | (!a & c),
                        Op::VectorTernary(VectorTernary::AddThree, ..) => {
                            a.wrapping_add(b).wrapping_add(c)
                        }
                        Op::VectorTernary(VectorTernary::AndOr, ..) => (a & b) | c,
                        Op::BitBinary(BitBinary::BitFieldMask, ..) => {
                            ((1u32 << (a & 31)) - 1) << (b & 31)
                        }
                        Op::BitBinary(BitBinary::CountBitsAdd, ..) => {
                            a.count_ones().wrapping_add(b)
                        }
                        Op::BitBinary(BitBinary::MaskedCountLow, ..) => {
                            // AMD RDNA2 §12.12: low-word positions below the logical lane.
                            // Count positions directly, independently of the MSL mask expression.
                            let count = (0..lane.min(32)).filter(|bit| (a >> bit) & 1 != 0).count();
                            b.wrapping_add(count as u32)
                        }
                        Op::BitBinary(BitBinary::MaskedCountHigh, ..) => {
                            let count = (0..lane.saturating_sub(32))
                                .filter(|bit| (a >> bit) & 1 != 0)
                                .count();
                            b.wrapping_add(count as u32)
                        }
                        Op::VectorTernary(VectorTernary::MinimumSigned, ..)
                        | Op::VectorTernary(VectorTernary::MaximumSigned, ..)
                        | Op::VectorTernary(VectorTernary::MedianSigned, ..) => {
                            let mut ordered = [a as i32, b as i32, c as i32];
                            ordered.sort_unstable();
                            let rank = match op {
                                Op::VectorTernary(VectorTernary::MinimumSigned, ..) => 0,
                                Op::VectorTernary(VectorTernary::MaximumSigned, ..) => 2,
                                _ => 1,
                            };
                            ordered[rank] as u32
                        }
                        Op::VectorTernary(VectorTernary::MinimumUnsigned, ..)
                        | Op::VectorTernary(VectorTernary::MaximumUnsigned, ..)
                        | Op::VectorTernary(VectorTernary::MedianUnsigned, ..) => {
                            let mut ordered = [a, b, c];
                            ordered.sort_unstable();
                            let rank = match op {
                                Op::VectorTernary(VectorTernary::MinimumUnsigned, ..) => 0,
                                Op::VectorTernary(VectorTernary::MaximumUnsigned, ..) => 2,
                                _ => 1,
                            };
                            ordered[rank]
                        }
                        Op::VectorTernary(VectorTernary::ExtractSigned, ..) => {
                            let width = (c & 31).min(32 - (b & 31));
                            if width == 0 {
                                0
                            } else {
                                let field = (u64::from(a) >> (b & 31)) & ((1u64 << width) - 1);
                                let signed = if field & (1u64 << (width - 1)) != 0 {
                                    field as i64 - (1i64 << width)
                                } else {
                                    field as i64
                                };
                                signed as u32
                            }
                        }
                        Op::VectorTernary(VectorTernary::ShiftAdd, ..) => {
                            (a << (b & 31)).wrapping_add(c)
                        }
                        Op::VectorTernary(VectorTernary::AddShift, ..) => {
                            a.wrapping_add(b) << (c & 31)
                        }
                        Op::VectorTernary(VectorTernary::ShiftOr, ..) => (a << (b & 31)) | c,
                        Op::VectorTernary(VectorTernary::ExtractUnsigned, ..) => {
                            (a >> (b & 31)) & ((1u32 << (c & 31)) - 1)
                        }
                        Op::VectorTernary(VectorTernary::AlignBits, ..) => {
                            (((u64::from(a) << 32) | u64::from(b)) >> (c & 31)) as u32
                        }
                        Op::VectorTernary(VectorTernary::OrThree, ..) => a | b | c,
                        Op::VectorTernary(VectorTernary::XorThree, ..) => a ^ b ^ c,
                        _ => unreachable!(),
                    };
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
                            read_vector_scalar(Source::Register(register))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                    check(src1)?;
                }
                for lane in (0..lanes).filter(|&lane| executes(lane)) {
                    let s0 = match src0 {
                        CompareSource::Vector(register) => read(lane, register)?,
                        CompareSource::Scalar(register) => {
                            read_vector_scalar(Source::Register(register))?
                        }
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
                    exec = (exec & !u64::from(u32::MAX)) | u64::from(value);
                }
                if dst == 127 && width == 64 {
                    exec = (exec & u64::from(u32::MAX)) | (u64::from(value) << 32);
                }
                if matches!(dst, 106 | 107) {
                    lane_derived_vcc[usize::from(dst - 106)] = false;
                }
                scalars[usize::from(dst)] = Some(value);
            }
            Op::ScalarBitSet(dst, index, value) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                let old = read_scalar(Source::Register(dst))?;
                let mask = 1u32 << (read_scalar(index)? & 31);
                scalars[usize::from(dst)] = Some(if value { old | mask } else { old & !mask });
            }
            Op::ScalarBitCompare(src, index, value) => {
                scc = Some(((read_scalar(src)? >> (read_scalar(index)? & 31)) & 1 != 0) == value);
            }
            Op::ScalarConditionalMove(dst, src) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                if scc.ok_or_else(|| error(offset, "conditional move requires initialized SCC"))? {
                    scalars[usize::from(dst)] = Some(read_scalar(src)?);
                }
            }
            Op::ScalarConditionalMoveImmediate(dst, immediate) => {
                if dst < 8 {
                    return Err(error(offset, "write to a reserved resource SGPR"));
                }
                if scc.ok_or_else(|| error(offset, "conditional move requires initialized SCC"))? {
                    scalars[usize::from(dst)] = Some(i32::from(immediate) as u32);
                }
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
                let mut mask = 0u64;
                if exec != 0 {
                    // Sources are checked once for the wave, even if no present lane is active.
                    let check = |register: u8| {
                        (0..lanes).try_for_each(|lane| read(lane, register).map(|_| ()))
                    };
                    match src0 {
                        CompareSource::Vector(register) => check(register)?,
                        CompareSource::Scalar(register) => {
                            read_vector_scalar(Source::Register(register))?;
                        }
                        CompareSource::Immediate(_) => {}
                    }
                    check(src1)?;
                    for lane in (0..lanes).filter(|&lane| executes(lane)) {
                        let s0 = match src0 {
                            CompareSource::Vector(register) => read(lane, register)?,
                            CompareSource::Scalar(register) => {
                                read_vector_scalar(Source::Register(register))?
                            }
                            CompareSource::Immediate(bits) => bits,
                        };
                        if comparison.holds(s0, read(lane, src1)?) {
                            mask |= 1 << lane;
                        }
                    }
                }
                // Publish both halves only after every lane has evaluated its inputs.
                scalars[VCC_LO] = Some(mask as u32);
                scalars[VCC_HI] = if width == 64 {
                    Some((mask >> 32) as u32)
                } else {
                    None
                };
                lane_derived_vcc = [exec != 0, width == 64 && exec != 0];
            }
            Op::Branch {
                target: destination,
                condition,
            } => {
                if matches!(
                    condition,
                    BranchCondition::VccZero | BranchCondition::VccNonZero
                ) && (scalars[VCC_LO].is_none() || (width == 64 && scalars[VCC_HI].is_none()))
                {
                    return Err(error(offset, "branch with uninitialized VCC_LO"));
                }
                if matches!(
                    condition,
                    BranchCondition::VccZero | BranchCondition::VccNonZero
                ) && (lane_derived_vcc[0] || (width == 64 && lane_derived_vcc[1]))
                {
                    let mut region_scalars = scalars;
                    for (slot, value) in lane_scalars.iter().copied().enumerate() {
                        if region_scalars[slot].is_none() {
                            region_scalars[slot] = value;
                        }
                    }
                    check_runtime_region(
                        &program[position + 1..],
                        destination,
                        &registers,
                        &region_scalars,
                        exec,
                        pending_read.is_some() || pending_ds.is_some(),
                        offset,
                    )?;
                }
                let take_branch = match condition {
                    BranchCondition::Always => true,
                    BranchCondition::SccZero => {
                        !scc.ok_or_else(|| error(offset, "branch with uninitialized SCC"))?
                    }
                    BranchCondition::SccOne => {
                        scc.ok_or_else(|| error(offset, "branch with uninitialized SCC"))?
                    }
                    BranchCondition::VccZero => {
                        scalars[VCC_LO] == Some(0) && (width == 32 || scalars[VCC_HI] == Some(0))
                    }
                    BranchCondition::VccNonZero => {
                        scalars[VCC_LO] != Some(0) || (width == 64 && scalars[VCC_HI] != Some(0))
                    }
                    BranchCondition::ExecZero => exec == 0,
                    BranchCondition::ExecNonZero => exec != 0,
                };
                if take_branch {
                    target = destination;
                }
            }
            Op::End => {
                if pending_read.is_some()
                    || pending_ds.is_some()
                    || pending_write.iter().any(Option::is_some)
                {
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
                if dst == 0
                    || pending_read == Some(dst)
                    || pending_ds.as_ref().is_some_and(|(r, _)| *r == dst)
                {
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
    exec: u64,
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
                let CompareSource::Vector(lhs) = lhs else {
                    return Err(error(
                        offset,
                        "runtime VCC region requires VGPR SRC0 for legacy VOP2",
                    ));
                };
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
            for (dst, a, b) in [(106, 8, 9), (8, 108, 9), (8, 8, 108), (8, 255, 255)] {
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
    fn cmpk_eq_normalizes_like_sopc_with_correct_immediate_extension() {
        for (opcode, imm, expected) in [
            (3, 0x7fff, 0x0000_7fff),
            (3, 0x8000, 0xffff_8000),
            (3, 0xffff, 0xffff_ffff),
            (9, 0x7fff, 0x0000_7fff),
            (9, 0x8000, 0x0000_8000),
            (9, 0xffff, 0x0000_ffff),
        ] {
            let instruction = 0xb008_0000 | opcode << 23 | imm;
            let program = decode(&bytes(&[instruction, 0xbf81_0000])).unwrap();
            assert_eq!(
                program[0].op,
                Op::CompareEqual(Source::Register(8), Source::Immediate(expected))
            );
            assert_eq!(program[1].offset, 4);
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
