// SPDX-License-Identifier: GPL-3.0-or-later

//! Verified compilation for Xenora's synthetic compute and graphics ABIs.

use std::fmt::{self, Write};

mod graphics;
pub use graphics::{
    CompiledGraphics, CompiledGraphics4, GraphicsArrayLayer, GraphicsExportType,
    GraphicsPipelineTypes, GraphicsPipelineTypes4, GraphicsSampleType, GraphicsStorageType,
    PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics, compile_graphics_pipeline,
    compile_graphics_pipeline_typed, compile_graphics_pipeline_typed4,
};

pub use xenora_decoder::Architecture;
use xenora_decoder::{
    BufferLoadKind, DecodeError, DecodedInstruction, Instruction, ScalarSource, VectorBinary,
    VectorBinaryOp, VectorBitBinaryOp, VectorClampOp, VectorCompareOp, VectorMadSadOp,
    VectorProductOp, VectorSource, VectorTernaryOp, VectorUnaryOp,
};

/// SGPR index of `vcc_lo`.
const VCC_LO: usize = 106;
/// SGPR index of `vcc_hi`.
const VCC_HI: usize = 107;

/// Entry point of all kernels produced by this compiler.
pub const ENTRY_POINT: &str = "xenora_compute";
const MAX_CODE_BYTES: usize = 1024;

/// Dispatch population available to guest wave operations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveLaunch {
    #[default]
    PartialAllowed,
    FullWavesRequired,
}

/// Compatibility re-export, including variant imports.
pub use WaveLaunch as Wave32Launch;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveSize {
    #[default]
    Wave32,
    Wave64,
}
impl WaveSize {
    fn lanes(self) -> u32 {
        if self == Self::Wave64 { 64 } else { 32 }
    }
    fn full_mask(self) -> u64 {
        if self == Self::Wave64 {
            u64::MAX
        } else {
            u32::MAX as u64
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WaveConfig {
    pub size: WaveSize,
    pub launch: WaveLaunch,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CompileOptions {
    pub wave32_launch: Wave32Launch,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LaunchError(pub &'static str);
impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for LaunchError {}

#[derive(Debug)]
pub struct CompiledShader {
    config: WaveConfig,
    pub disassembly: String,
    pub ir: String,
    pub msl: String,
}

impl CompiledShader {
    pub fn validate_launch(&self, word_count: usize) -> Result<(), LaunchError> {
        if !(1..=4096).contains(&word_count) {
            return Err(LaunchError("input word count must be in 1..=4096"));
        }
        if self.config.launch == WaveLaunch::FullWavesRequired
            && !word_count.is_multiple_of(self.config.size.lanes() as usize)
        {
            return Err(LaunchError(if self.config.size == WaveSize::Wave64 {
                "full Wave64 launch requires a multiple of 64 words"
            } else {
                "full Wave32 launch requires a multiple of 32 words"
            }));
        }
        Ok(())
    }
    pub fn wave_config(&self) -> WaveConfig {
        self.config
    }
    pub fn launch_policy(&self) -> WaveLaunch {
        self.config.launch
    }
    pub fn wave32_launch(&self) -> Wave32Launch {
        self.config.launch
    }
}

#[derive(Debug)]
pub enum CompileError {
    InvalidLength {
        bytes: usize,
    },
    Decode(DecodeError),
    GraphicsStage {
        stage: &'static str,
        error: Box<CompileError>,
    },
    GraphicsLink {
        reason: &'static str,
    },
    Verify {
        offset: usize,
        reason: &'static str,
    },
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { bytes } => write!(
                formatter,
                "fixture length must be a nonzero multiple of four up to {MAX_CODE_BYTES} bytes, got {bytes}"
            ),
            Self::Decode(error) => error.fmt(formatter),
            Self::GraphicsStage { stage, error } => write!(formatter, "{stage} shader: {error}"),
            Self::GraphicsLink { reason } => write!(formatter, "graphics linking: {reason}"),
            Self::Verify { offset, reason } => {
                write!(
                    formatter,
                    "shader verification at byte 0x{offset:x}: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for CompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(error) => Some(error),
            Self::GraphicsStage { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Value(usize);

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "t{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MaskCondition {
    Zero,
    NonZero,
}

impl MaskCondition {
    fn suffix(self) -> &'static str {
        match self {
            Self::Zero => "vccz",
            Self::NonZero => "vccnz",
        }
    }

    fn fallthrough_test(self) -> &'static str {
        match self {
            Self::Zero => "!=",
            Self::NonZero => "==",
        }
    }
}

#[derive(Clone, Copy)]
// hi=None means the Wave32 zero test ignores HI, never an unknown Wave64 half.
struct MaskValue {
    lo: Value,
    hi: Option<Value>,
}
impl MaskValue {
    fn expression(self, name: &dyn Fn(Value) -> String) -> String {
        match self.hi {
            Some(hi) => format!("({} | {})", name(self.lo), name(hi)),
            None => name(self.lo),
        }
    }
}
impl fmt::Display for MaskValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.expression(&|v| v.to_string()))
    }
}
fn mask_test(mask: u64, lane: &str, wave: WaveSize) -> String {
    if wave == WaveSize::Wave32 {
        format!("((0x{mask:08x}u >> {lane}) & 1u) != 0u")
    } else {
        format!(
            "(((({lane}) < 32u ? 0x{:08x}u : 0x{:08x}u) >> (({lane}) & 31u)) & 1u) != 0u",
            mask as u32,
            (mask >> 32) as u32
        )
    }
}

#[derive(Clone, Copy)]
enum GatherSelection {
    Address { value: Value, offset: u16 },
    Table([u8; 32]),
}

fn swizzle_table(offset: u16) -> [u8; 32] {
    std::array::from_fn(|i| {
        let i = i as u32;
        let o = u32::from(offset);
        let mask = o & 31;
        let selected = if o >= 0xe000 {
            ((i.reverse_bits() >> 27) >> mask.count_ones()) | (i & mask)
        } else if o >= 0xc000 {
            let rotate = (o >> 5) & 31;
            let rotated = if o & 0x400 == 0 {
                i.wrapping_add(rotate)
            } else {
                i.wrapping_sub(rotate)
            };
            (i & mask) | (rotated & !mask)
        } else if o & 0x8000 != 0 {
            (i & !3) | ((o >> (2 * (i & 3))) & 3)
        } else {
            ((i & mask) | ((o >> 5) & 31)) ^ ((o >> 10) & 31)
        };
        (selected & 31) as u8
    })
}

#[derive(Clone, Copy)]
enum Op {
    Scatter {
        address: Value,
        data: Value,
        offset: u16,
        exec: u64,
    },
    Gather {
        source: Value,
        selection: GatherSelection,
        exec: u64,
    },
    Load(Option<BufferLoadKind>),
    ReadLane {
        source: Value,
        selector: Value,
    },
    /// Captured scalar selector/data; only the selected guest lane changes.
    WriteLane {
        selector: Value,
        data: Value,
        old: Value,
    },
    Constant(u32),
    Binary(Binary, Value, Value),
    Unary(VectorUnaryOp, Value),
    Product(VectorProductOp, Value, Value),
    BitBinary(VectorBitBinaryOp, Value, Value),
    Ternary(VectorTernaryOp, Value, Value, Value),
    MadSad(VectorMadSadOp, bool, Value, Value, Value),
    MergeLanes {
        mask: u64,
        active: Value,
        inactive: Value,
    },
    Store(Value),
    ScalarBranch {
        target: usize,
        taken: bool,
    },
    /// One 32-bit projection of VOPC: bit n tests guest lane half+n.
    /// The value is wave-uniform; every thread computes it for its own wave.
    LaneMask {
        op: VectorCompareOp,
        src0: Value,
        src1: Value,
        exec: u64,
        half: u32,
    },
    /// Conditional branch on the configured-width VCC pair (IR annotation only).
    RuntimeBranch {
        condition: MaskCondition,
        mask: MaskValue,
        target: usize,
    },
    /// Select fall-through or pre-branch state according to the branch polarity.
    JoinVcc {
        condition: MaskCondition,
        mask: MaskValue,
        fallthrough: Value,
        taken: Value,
    },
}

impl Op {
    fn operands(self) -> Vec<Value> {
        match self {
            Self::Scatter { address, data, .. } => vec![address, data],
            Self::Gather {
                source, selection, ..
            } => match selection {
                GatherSelection::Address { value, .. } => vec![source, value],
                GatherSelection::Table(_) => vec![source],
            },
            Self::Load(_) | Self::Constant(_) | Self::ScalarBranch { .. } => Vec::new(),
            Self::Binary(_, left, right) => vec![left, right],
            Self::ReadLane { source, selector } => vec![source, selector],
            Self::WriteLane {
                selector,
                data,
                old,
            } => vec![selector, data, old],
            Self::Unary(_, source) => vec![source],
            Self::Product(_, a, b) => vec![a, b],
            Self::BitBinary(_, a, b) => vec![a, b],
            Self::Ternary(_, a, b, c) => vec![a, b, c],
            Self::MadSad(_, _, a, b, c) => vec![a, b, c],
            Self::MergeLanes {
                active, inactive, ..
            } => vec![active, inactive],
            Self::Store(value) => vec![value],
            Self::LaneMask { src0, src1, .. } => vec![src0, src1],
            Self::RuntimeBranch { mask, .. } => std::iter::once(mask.lo).chain(mask.hi).collect(),
            Self::JoinVcc {
                mask,
                fallthrough,
                taken,
                ..
            } => [fallthrough, taken, mask.lo]
                .into_iter()
                .chain(mask.hi)
                .collect(),
        }
    }

    /// The MSL expression of a per-thread value, with operand names, the
    /// dispatch index and the lane given by the caller.
    fn expression(
        self,
        name: &dyn Fn(Value) -> String,
        index: &str,
        lane: &str,
        wave: WaveSize,
    ) -> String {
        match self {
            Self::Load(kind) => {
                let word = format!("input[{index}]");
                // The synthetic ABI keeps a four-byte stride for every load kind.
                // Unsigned subtraction sign-extends without signed overflow or narrowing casts.
                match kind {
                    None => word,
                    Some(BufferLoadKind::Ubyte) => format!("({word} & 0xffu)"),
                    Some(BufferLoadKind::Sbyte) => format!("(({word} & 0xffu) ^ 0x80u) - 0x80u"),
                    Some(BufferLoadKind::Ushort) => format!("({word} & 0xffffu)"),
                    Some(BufferLoadKind::Sshort) => {
                        format!("(({word} & 0xffffu) ^ 0x8000u) - 0x8000u")
                    }
                }
            }
            Self::Constant(value) => format!("0x{value:08x}u"),
            Self::Unary(op, source) => vector_unary_expression(op, &name(source)),
            Self::Product(op, a, b) => vector_product_expression(op, &name(a), &name(b)),
            Self::BitBinary(op, a, b) => {
                let (a, b) = (name(a), name(b));
                if wave == WaveSize::Wave64 {
                    match op {
                        VectorBitBinaryOp::MbcntLoU32B32 => {
                            return format!(
                                "popcount(({a}) & (({lane}) >= 32u ? 0xffffffffu : ((1u << (({lane}) & 31u)) - 1u))) + ({b})"
                            );
                        }
                        VectorBitBinaryOp::MbcntHiU32B32 => {
                            return format!(
                                "popcount(({a}) & (({lane}) <= 32u ? 0u : ((1u << (({lane}) & 31u)) - 1u))) + ({b})"
                            );
                        }
                        _ => {}
                    }
                }
                vector_bit_binary_expression(op, &a, &b, lane)
            }
            Self::Ternary(op, a, b, c) => {
                vector_ternary_expression(op, &name(a), &name(b), &name(c))
            }
            Self::MadSad(op, clamp, a, b, c) => {
                mad_sad_expression(op, clamp, &name(a), &name(b), &name(c))
            }
            Self::Binary(operator, left, right) => {
                let (left, right) = (name(left), name(right));
                let (_, symbol) = operator.operator();
                match operator {
                    Binary::FloatAdd | Binary::FloatMul => format!(
                        "as_type<uint>(as_type<float>({left}) {symbol} as_type<float>({right}))"
                    ),
                    Binary::ArithmeticShiftRight => {
                        format!("as_type<uint>(as_type<int>({left}) {symbol} ({right} & 31u))")
                    }
                    Binary::ShiftLeft | Binary::ShiftRight => {
                        format!("{left} {symbol} ({right} & 31u)")
                    }
                    Binary::Vector(operation) => vector_expression(operation, &left, &right),
                    Binary::Clamped(op) => clamped_expression(op, &left, &right),
                    _ => format!("{left} {symbol} {right}"),
                }
            }
            Self::WriteLane {
                selector,
                data,
                old,
            } => format!(
                "({lane}) == ({} & {}u) ? {} : {}",
                name(selector),
                wave.lanes() - 1,
                name(data),
                name(old)
            ),
            Self::MergeLanes {
                mask,
                active,
                inactive,
            } => format!(
                "{} ? {} : {}",
                mask_test(mask, lane, wave),
                name(active),
                name(inactive)
            ),
            Self::JoinVcc {
                condition,
                mask,
                fallthrough,
                taken,
            } => format!(
                "{} {} 0u ? {} : {}",
                mask.expression(name),
                condition.fallthrough_test(),
                name(fallthrough),
                name(taken)
            ),
            Self::Scatter { .. }
            | Self::Gather { .. }
            | Self::ReadLane { .. }
            | Self::Store(_)
            | Self::ScalarBranch { .. }
            | Self::LaneMask { .. }
            | Self::RuntimeBranch { .. } => unreachable!("not a per-thread expression"),
        }
    }
}

fn compare_expression(op: VectorCompareOp, left: &str, right: &str) -> String {
    use xenora_decoder::CompareRelation;
    let symbol = match op.relation {
        CompareRelation::Less => "<",
        CompareRelation::Equal => "==",
        CompareRelation::LessOrEqual => "<=",
        CompareRelation::Greater => ">",
        CompareRelation::NotEqual => "!=",
        CompareRelation::GreaterOrEqual => ">=",
    };
    if op.signed {
        format!("as_type<int>({left}) {symbol} as_type<int>({right})")
    } else {
        format!("{left} {symbol} {right}")
    }
}

#[derive(Clone, Copy)]
enum Binary {
    Add,
    ShiftLeft,
    ShiftRight,
    ArithmeticShiftRight,
    And,
    Or,
    Xor,
    FloatAdd,
    FloatMul,
    /// A registry VOP2 integer operation, lowered by its typed template.
    Vector(VectorBinary),
    Clamped(VectorClampOp),
}

impl Binary {
    fn operator(self) -> (&'static str, &'static str) {
        match self {
            Self::Add => ("add.u32", "+"),
            Self::ShiftLeft => ("lshl.b32", "<<"),
            Self::ShiftRight => ("lshr.b32", ">>"),
            Self::ArithmeticShiftRight => ("ashr.i32", ">>"),
            Self::And => ("and.b32", "&"),
            Self::Or => ("or.b32", "|"),
            Self::Xor => ("xor.b32", "^"),
            Self::FloatAdd => ("add.f32", "+"),
            Self::FloatMul => ("mul.f32", "*"),
            Self::Clamped(op) => (op.mnemonic(), ""),
            Self::Vector(operation) => (
                match operation {
                    VectorBinary::Add => "add.u32",
                    VectorBinary::Sub => "sub.u32",
                    VectorBinary::Xnor => "xnor.b32",
                    VectorBinary::MinSigned => "min.i32",
                    VectorBinary::MinUnsigned => "min.u32",
                    VectorBinary::MaxSigned => "max.i32",
                    VectorBinary::MaxUnsigned => "max.u32",
                    VectorBinary::Mul24Signed => "mul24.i32",
                    VectorBinary::Mul24Unsigned => "mul24.u32",
                    VectorBinary::MulHi24Signed => "mulhi24.i32",
                    VectorBinary::MulHi24Unsigned => "mulhi24.u32",
                },
                "",
            ),
        }
    }
}

/// The MSL template of each per-lane registry operation. Integer arithmetic is
/// on `uint` (wrapping); signed forms reinterpret with `as_type`, and the
/// 24-bit forms sign-extend bit 23 with a shift pair or mask to 24 bits first.
fn vector_expression(operation: VectorBinary, left: &str, right: &str) -> String {
    let int = |value: &str| format!("as_type<int>({value})");
    let i24 = |value: &str| format!("(as_type<int>({value} << 8u) >> 8)");
    let u24 = |value: &str| format!("({value} & 0x00ffffffu)");
    match operation {
        VectorBinary::Add => format!("{left} + {right}"),
        VectorBinary::Sub => format!("{left} - {right}"),
        VectorBinary::Xnor => format!("~({left} ^ {right})"),
        VectorBinary::MinSigned => format!("as_type<uint>(min({}, {}))", int(left), int(right)),
        VectorBinary::MinUnsigned => format!("min({left}, {right})"),
        VectorBinary::MaxSigned => format!("as_type<uint>(max({}, {}))", int(left), int(right)),
        VectorBinary::MaxUnsigned => format!("max({left}, {right})"),
        // The low 32 bits of a product do not depend on signedness, so the
        // sign-extended factors multiply as wrapping uint.
        VectorBinary::Mul24Signed => format!(
            "as_type<uint>({}) * as_type<uint>({})",
            i24(left),
            i24(right)
        ),
        VectorBinary::Mul24Unsigned => format!("{} * {}", u24(left), u24(right)),
        VectorBinary::MulHi24Signed => {
            format!("as_type<uint>(mulhi({}, {}))", i24(left), i24(right))
        }
        VectorBinary::MulHi24Unsigned => format!("mulhi({}, {})", u24(left), u24(right)),
    }
}

// Integer saturation is applied to the mathematical result before narrowing.
// Products use low/high32 words instead of requiring Metal64-bit arithmetic.
fn clamped_expression(op: VectorClampOp, a: &str, b: &str) -> String {
    match op {
        VectorClampOp::AddU32 => {
            format!("(({a}) > (0xffffffffu - ({b})) ? 0xffffffffu : (({a}) + ({b})))")
        }
        VectorClampOp::SubU32 => format!("(({a}) < ({b}) ? 0u : (({a}) - ({b})))"),
        VectorClampOp::SubrevU32 => format!("(({b}) < ({a}) ? 0u : (({b}) - ({a})))"),
        VectorClampOp::MulU24 => {
            let low = vector_expression(VectorBinary::Mul24Unsigned, a, b);
            let high = vector_expression(VectorBinary::MulHi24Unsigned, a, b);
            format!("(({high}) != 0u ? 0xffffffffu : ({low}))")
        }
        VectorClampOp::MulI24 => {
            let low = vector_expression(VectorBinary::Mul24Signed, a, b);
            let high = vector_expression(VectorBinary::MulHi24Signed, a, b);
            let limit =
                format!("(((({a}) ^ ({b})) & 0x00800000u) != 0u ? 0x80000000u : 0x7fffffffu)");
            format!("(({high}) == (0u - (({low}) >> 31u)) ? ({low}) : {limit})")
        }
    }
}

// AMD RDNA2 section 12.8: integer unary results preserve all condition/mask state.
// Signed FFBH counts leading sign bits including the sign bit itself.
fn vector_unary_expression(op: VectorUnaryOp, source: &str) -> String {
    use VectorUnaryOp::*;
    match op {
        NotB32 => format!("~({source})"),
        BfrevB32 => format!("reverse_bits({source})"),
        FfbhU32 => format!("(({source}) == 0u) ? 0xffffffffu : clz({source})"),
        FfblB32 => format!("(({source}) == 0u) ? 0xffffffffu : ctz({source})"),
        FfbhI32 => {
            let differing = format!("(({source}) ^ (0u - (({source}) >> 31u)))");
            format!("({differing} == 0u) ? 0xffffffffu : clz({differing})")
        }
    }
}

// Return the existing arithmetic primitive and whether assembly sources reverse.
fn alias_binary(op: VectorBinaryOp) -> (Binary, bool) {
    match op {
        VectorBinaryOp::Alu(op) => (Binary::Vector(op.lowering()), op.swaps_sources()),
        VectorBinaryOp::AndB32 => (Binary::And, false),
        VectorBinaryOp::OrB32 => (Binary::Or, false),
        VectorBinaryOp::XorB32 => (Binary::Xor, false),
        VectorBinaryOp::LshlrevB32 => (Binary::ShiftLeft, true),
        VectorBinaryOp::LshrrevB32 => (Binary::ShiftRight, true),
        VectorBinaryOp::AshrrevI32 => (Binary::ArithmeticShiftRight, true),
    }
}

fn vector_product_expression(op: VectorProductOp, a: &str, b: &str) -> String {
    match op {
        VectorProductOp::MulLoU32 => format!("({a}) * ({b})"),
        VectorProductOp::MulHiU32 => format!("mulhi({a}, {b})"),
        VectorProductOp::MulHiI32 => {
            format!("as_type<uint>(mulhi(as_type<int>({a}), as_type<int>({b})))")
        }
    }
}
fn vector_bit_binary_expression(op: VectorBitBinaryOp, a: &str, b: &str, lane: &str) -> String {
    match op {
        VectorBitBinaryOp::BfmB32 => format!("((1u << (({a}) & 31u)) - 1u) << (({b}) & 31u)"),
        VectorBitBinaryOp::BcntU32B32 => format!("popcount({a}) + ({b})"),
        // AMD RDNA2 ISA §12.12: EXEC gates writes, not the counted SRC0 bits.
        // Callers supply a logical Wave32 lane, so this never shifts by 32.
        VectorBitBinaryOp::MbcntLoU32B32 => {
            format!("popcount(({a}) & ((1u << ({lane})) - 1u)) + ({b})")
        }
        VectorBitBinaryOp::MbcntHiU32B32 => format!("({b})"),
    }
}
fn vector_ternary_expression(op: VectorTernaryOp, a: &str, b: &str, c: &str) -> String {
    match op {
        VectorTernaryOp::MadU32U16 => format!("((({a}) & 65535u) * (({b}) & 65535u)) + ({c})"),
        VectorTernaryOp::MadI32I16 => {
            // Sign-extend low16 into unsigned words. Both product and addition
            // then wrap in uint, without signed-overflow assumptions.
            let signed_word = |v: &str| format!("(((({v}) & 65535u) ^ 32768u) - 32768u)");
            format!("({} * {}) + ({c})", signed_word(a), signed_word(b))
        }
        VectorTernaryOp::XadU32 => format!("(({a}) ^ ({b})) + ({c})"),
        // AMD RDNA2 §12.12: SRC2 contributes only bits 0,8,16,24.
        // Each byte sum is at most511, so averaging cannot carry across bytes.
        VectorTernaryOp::LerpU8 => (0..4)
            .map(|byte| {
                let shift = byte * 8;
                format!("(((((({a}) >> {shift}u) & 255u) + ((({b}) >> {shift}u) & 255u) + ((({c}) >> {shift}u) & 1u)) >> 1u) << {shift}u)")
            })
            .collect::<Vec<_>>()
            .join(" | "),
        // Mask by the reference byte (SRC1), not SRC0. Unsigned addition
        // deliberately preserves wraparound of the complete accumulator.
        VectorTernaryOp::MsadU8 => {
            let mut terms = vec![format!("({c})")];
            for shift in [0, 8, 16, 24] {
                let x = format!("((({a}) >> {shift}u) & 255u)");
                let y = format!("((({b}) >> {shift}u) & 255u)");
                terms.push(format!("({y} == 0u ? 0u : (max({x}, {y}) - min({x}, {y})))"));
            }
            terms.join(" + ")
        }
        VectorTernaryOp::BfiB32 => format!("(({a}) & ({b})) | (~({a}) & ({c}))"),
        VectorTernaryOp::Add3U32 => format!("(({a}) + ({b})) + ({c})"),
        VectorTernaryOp::AndOrB32 => format!("(({a}) & ({b})) | ({c})"),
        VectorTernaryOp::BfeU32 => {
            format!("(({a}) >> (({b}) & 31u)) & ((1u << (({c}) & 31u)) - 1u)")
        }
        // Select the low word for count zero; every evaluated shift is at most31.
        VectorTernaryOp::AlignbitB32 => format!(
            "((({c}) & 31u) == 0u ? ({b}) : ((({b}) >> (({c}) & 31u)) | (({a}) << ((32u - (({c}) & 31u)) & 31u))))"
        ),
        VectorTernaryOp::Or3B32 => format!("({a}) | ({b}) | ({c})"),
        VectorTernaryOp::Xor3B32 => format!("({a}) ^ ({b}) ^ ({c})"),
        VectorTernaryOp::BfeI32 => {
            let offset = format!("(({b}) & 31u)");
            let width = format!("(({c}) & 31u)");
            // A zero-width field is zero. Crossing the word boundary sign-extends
            // bit31; narrower fields sign-extend their own highest selected bit.
            format!(
                "({width} == 0u ? 0u : ({offset} + {width} >= 32u ? as_type<uint>(as_type<int>({a}) >> {offset}) : as_type<uint>(as_type<int>(({a}) << ((32u - {offset} - {width}) & 31u)) >> ((32u - {width}) & 31u))))"
            )
        }
        VectorTernaryOp::LshlAddU32 => format!("(({a}) << (({b}) & 31u)) + ({c})"),
        VectorTernaryOp::AddLshlU32 => format!("(({a}) + ({b})) << (({c}) & 31u)"),
        VectorTernaryOp::LshlOrB32 => format!("(({a}) << (({b}) & 31u)) | ({c})"),
        VectorTernaryOp::Min3U32 => format!("min(min({a}, {b}), {c})"),
        VectorTernaryOp::Max3U32 => format!("max(max({a}, {b}), {c})"),
        VectorTernaryOp::Med3U32 => format!("max(min({a}, {b}), min(max({a}, {b}), {c}))"),
        VectorTernaryOp::Min3I32 => format!(
            "as_type<uint>(min(min(as_type<int>({a}), as_type<int>({b})), as_type<int>({c})))"
        ),
        VectorTernaryOp::Max3I32 => format!(
            "as_type<uint>(max(max(as_type<int>({a}), as_type<int>({b})), as_type<int>({c})))"
        ),
        VectorTernaryOp::Med3I32 => format!(
            "as_type<uint>(max(min(as_type<int>({a}), as_type<int>({b})), min(max(as_type<int>({a}), as_type<int>({b})), as_type<int>({c}))))"
        ),
    }
}

// Preserve the complete product through accumulator addition before saturation.
fn mad_sad_expression(op: VectorMadSadOp, clamp: bool, a: &str, b: &str, c: &str) -> String {
    use VectorMadSadOp::*;
    if matches!(op, MadI32I24 | MadU32U24) {
        let signed = matches!(op, MadI32I24);
        let low = vector_expression(
            if signed {
                VectorBinary::Mul24Signed
            } else {
                VectorBinary::Mul24Unsigned
            },
            a,
            b,
        );
        let sum = format!("(({low}) + ({c}))");
        if !clamp {
            return sum;
        }
        let high = vector_expression(
            if signed {
                VectorBinary::MulHi24Signed
            } else {
                VectorBinary::MulHi24Unsigned
            },
            a,
            b,
        );
        let carry = format!("uint(({sum}) < ({low}))");
        if signed {
            let high_sum = format!("(({high}) + (0u - (({c}) >> 31u)) + {carry})");
            format!(
                "(({high_sum}) == (0u - (({sum}) >> 31u)) ? ({sum}) : ((({high_sum}) & 0x80000000u) != 0u ? 0x80000000u : 0x7fffffffu))"
            )
        } else {
            format!("((({high}) + {carry}) != 0u ? 0xffffffffu : ({sum}))")
        }
    } else {
        let (bits, count, shift) = match op {
            SadU8 => (8, 4, 0),
            SadHiU8 => (8, 4, 16),
            SadU16 => (16, 2, 0),
            SadU32 => (32, 1, 0),
            _ => unreachable!("products handled above"),
        };
        let terms: Vec<_> = (0..count)
            .map(|lane| {
                let part = |value: &str| {
                    if bits == 32 {
                        format!("({value})")
                    } else {
                        format!(
                            "((({value}) >> {}u) & 0x{:x}u)",
                            lane * bits,
                            (1u32 << bits) - 1
                        )
                    }
                };
                let (left, right) = (part(a), part(b));
                format!("(max({left}, {right}) - min({left}, {right}))")
            })
            .collect();
        let sum = format!("(({}) << {shift}u)", terms.join(" + "));
        if clamp {
            clamped_expression(VectorClampOp::AddU32, &sum, c)
        } else {
            format!("({sum}) + ({c})")
        }
    }
}

fn scalar_bit_set(old: u32, index: u32, value: bool) -> u32 {
    let mask = 1u32 << (index & 31);
    if value { old | mask } else { old & !mask }
}
fn scalar_bit_test(word: u32, index: u32, value: bool) -> bool {
    ((word >> (index & 31)) & 1 != 0) == value
}

// Each bit is either known zero, known one or unconstrained.
#[derive(Clone, Copy)]
struct KnownBits {
    zero: u32,
    one: u32,
    values: Option<[u32; 2]>,
}

impl KnownBits {
    const UNKNOWN: Self = Self {
        zero: 0,
        one: 0,
        values: None,
    };

    fn constant(value: u32) -> Self {
        Self {
            zero: !value,
            one: value,
            values: Some([value; 2]),
        }
    }

    fn and(self, other: Self) -> Self {
        Self {
            zero: self.zero | other.zero,
            one: self.one & other.one,
            values: None,
        }
    }

    fn or(self, other: Self) -> Self {
        Self {
            zero: self.zero & other.zero,
            one: self.one | other.one,
            values: None,
        }
    }

    fn xor(self, other: Self) -> Self {
        Self {
            zero: (self.zero & other.zero) | (self.one & other.one),
            one: (self.zero & other.one) | (self.one & other.zero),
            values: None,
        }
    }

    fn positive_normal_candidates(self) -> Option<[f32; 2]> {
        let values = match self.values {
            Some(values) => values,
            None => {
                let unknown = !(self.zero | self.one);
                if unknown.count_ones() > 1 {
                    return None;
                }
                [self.one, self.one | unknown]
            }
        };
        let candidates = values.map(f32::from_bits);
        candidates
            .iter()
            .all(|n| n.is_normal() && n.is_sign_positive())
            .then_some(candidates)
    }
}

fn exact_float_result(operator: Binary, left: KnownBits, right: KnownBits) -> Option<KnownBits> {
    let left = left.positive_normal_candidates()?;
    let right = right.positive_normal_candidates()?;
    let mut zero = u32::MAX;
    let mut one = u32::MAX;
    let mut results = Vec::with_capacity(4);
    for a in left.map(f64::from) {
        for b in right.map(f64::from) {
            let exact = match operator {
                Binary::FloatAdd => a + b,
                Binary::FloatMul => a * b,
                _ => return None,
            };
            let value = exact as f32;
            if !value.is_normal() || f64::from(value) != exact {
                return None;
            }
            // Binary64 can round away the smaller operand of a widely separated
            // binary32 sum. Require that subtraction recovers the smaller input.
            if matches!(operator, Binary::FloatAdd) && exact - a.max(b) != a.min(b) {
                return None;
            }
            let bits = value.to_bits();
            zero &= !bits;
            one &= bits;
            if !results.contains(&bits) {
                results.push(bits);
            }
        }
    }
    let values = match results.as_slice() {
        [only] => Some([*only; 2]),
        [first, second] => Some([*first, *second]),
        _ => None,
    };
    Some(KnownBits { zero, one, values })
}

/// Compiles explicitly tagged raw machine code to a synthetic Metal compute kernel.
///
/// Bind equal-length disjoint input and output word buffers and dispatch one
/// invocation per word in a one-dimensional grid.
pub fn compile(architecture: Architecture, bytes: &[u8]) -> Result<CompiledShader, CompileError> {
    compile_with_options(architecture, bytes, CompileOptions::default())
}

pub fn compile_with_options(
    architecture: Architecture,
    bytes: &[u8],
    options: CompileOptions,
) -> Result<CompiledShader, CompileError> {
    compile_with_wave_config(
        architecture,
        bytes,
        WaveConfig {
            size: WaveSize::Wave32,
            launch: options.wave32_launch,
        },
    )
}

pub fn compile_with_wave_config(
    architecture: Architecture,
    bytes: &[u8],
    config: WaveConfig,
) -> Result<CompiledShader, CompileError> {
    if config.size == WaveSize::Wave64 && config.launch != WaveLaunch::FullWavesRequired {
        return Err(replay_error("Wave64 requires full waves"));
    }
    let decoded = decode_fixture(architecture, bytes)?;
    let operations = lift(&decoded, bytes.len(), config)?;
    let lane = format!("(index & {}u)", config.size.lanes() - 1);

    let disassembly = decoded
        .iter()
        .map(|line| {
            let text = format!("{line}\n");
            if config.size == WaveSize::Wave64
                && matches!(line.instruction, Instruction::VectorCompare { .. })
            {
                // Only the comparison destination; source half names remain explicit.
                text.replacen(" vcc_lo,", " vcc,", 1)
            } else {
                text
            }
        })
        .collect();
    let mut ir = String::new();
    let has_mask = operations
        .iter()
        .any(|(_, operation)| matches!(operation, Op::LaneMask { .. }));
    let grid = if has_mask || config.launch == WaveLaunch::FullWavesRequired {
        ",\n    uint grid [[threads_per_grid]]"
    } else {
        ""
    };
    let mut msl = format!(
        "#include <metal_stdlib>\nusing namespace metal;\nkernel void {ENTRY_POINT}(\n    device const uint* input [[buffer(0)]],\n    device uint* output [[buffer(1)]],\n    uint index [[thread_position_in_grid]]{grid}) {{\n"
    );
    if config.launch == WaveLaunch::FullWavesRequired {
        append_msl(
            &mut msl,
            &format!(
                "    if (grid == 0u || grid > 4096u || (grid & {}u) != 0u) {{ return; }}\n",
                config.size.lanes() - 1
            ),
        )?;
    }
    if operations.iter().any(|(_, operation)| {
        matches!(
            operation,
            Op::Binary(Binary::FloatAdd | Binary::FloatMul, _, _)
        )
    }) {
        append_msl(
            &mut msl,
            "    #pragma clang fp contract(off)\n    #pragma clang fp reassociate(off)\n",
        )?;
    }
    // Wave-uniform values: identical in every lane of an RDNA wave.
    let mut replay_nodes = ReplayBudget {
        wave: config.size,
        enforce_work: operations
            .iter()
            .any(|(_, op)| matches!(op, Op::Scatter { .. })),
        ..ReplayBudget::default()
    };
    for (i, &(_, op)) in operations.iter().enumerate() {
        if let Op::Gather {
            selection: GatherSelection::Table(table),
            ..
        } = op
        {
            // Count per-invocation table initialization, not GPU instructions or time.
            replay_nodes.charge(32, 1)?;
            let entries = table
                .iter()
                .map(|v| format!("{v}u"))
                .collect::<Vec<_>>()
                .join(", ");
            append_msl(
                &mut msl,
                &format!("    const uint gather_{i}[32] = {{{entries}}};\n"),
            )?;
        }
    }
    let mut uniform = Vec::with_capacity(operations.len());
    for (index, &(offset, operation)) in operations.iter().enumerate() {
        replay_nodes.charge(1, 1)?;
        let dest = Value(index);
        uniform.push(match operation {
            Op::Constant(_) | Op::LaneMask { .. } | Op::ReadLane { .. } => true,
            Op::Load(_)
            | Op::Gather { .. }
            | Op::Scatter { .. }
            | Op::MergeLanes { .. }
            | Op::WriteLane { .. }
            | Op::BitBinary(
                VectorBitBinaryOp::MbcntLoU32B32 | VectorBitBinaryOp::MbcntHiU32B32,
                _,
                _,
            ) => false,
            _ => operation.operands().iter().all(|value| uniform[value.0]),
        });
        let thread = |value: Value| value.to_string();
        let (step, statement) = match operation {
            Op::Gather { .. } | Op::Scatter { .. } => {
                let mut replay = Replay::new(
                    &mut msl,
                    &operations,
                    &uniform,
                    index,
                    &mut replay_nodes,
                    lane.clone(),
                    "index".into(),
                    true,
                );
                let value = replay.value(Value(index), 0, 0)?;
                append_msl(replay.msl, &format!("    uint {dest} = {value};\n"))?;
                let _ = writeln!(ir, "{offset:04x}: {dest} = gather.snapshot");
                continue;
            }
            Op::ReadLane { source, selector } => {
                let label = if matches!(operations[selector.0].1, Op::Constant(0)) {
                    "read.lane0"
                } else {
                    "read.lane"
                };
                let _ = writeln!(ir, "{offset:04x}: {dest} = {label} {source}, {selector}");
                emit_selected_lane(
                    &mut msl,
                    &operations,
                    &uniform,
                    index,
                    source,
                    selector,
                    &mut replay_nodes,
                )?;
                continue;
            }
            Op::Load(kind) => (
                match kind {
                    None => format!("{dest} = load.u32 input[index]"),
                    Some(kind) => format!("{dest} = {} input[index]", kind.mnemonic()),
                },
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "index", "", config.size)
                ),
            ),
            Op::Constant(value) => (
                format!("{dest} = const.u32 0x{value:08x}"),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::Binary(operator, left, right) => (
                format!("{dest} = {} {left}, {right}", operator.operator().0),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::Unary(op, source) => (
                format!("{dest} = {} {source}", op.mnemonic()),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::Product(op, a, b) => (
                format!("{dest} = {} {a}, {b}", op.mnemonic()),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::BitBinary(op, a, b) => (
                format!("{dest} = {} {a}, {b}", op.mnemonic()),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", &lane, config.size)
                ),
            ),
            Op::Ternary(op, a, b, c) => (
                format!("{dest} = {} {a}, {b}, {c}", op.mnemonic()),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::MadSad(op, clamp, a, b, c) => (
                format!("{dest} = {} clamp={clamp} {a}, {b}, {c}", op.mnemonic()),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::ScalarBranch { target, taken } => {
                let _ = writeln!(
                    ir,
                    "{offset:04x}: branch.uniform 0x{target:04x} taken={taken}"
                );
                continue;
            }
            Op::RuntimeBranch {
                condition,
                mask,
                target,
            } => {
                let suffix = condition.suffix();
                let _ = writeln!(ir, "{offset:04x}: branch.{suffix} {mask} -> 0x{target:04x}");
                continue;
            }
            Op::WriteLane {
                selector,
                data,
                old,
            } => (
                format!("{dest} = write.lane {selector}, {data}, {old}"),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", &lane, config.size)
                ),
            ),
            Op::MergeLanes {
                mask,
                active,
                inactive,
            } => (
                format!("{dest} = merge.lanes mask=0x{mask:08x}, {active}, {inactive}"),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", &lane, config.size)
                ),
            ),
            Op::JoinVcc {
                condition,
                mask,
                fallthrough,
                taken,
            } => (
                format!(
                    "{dest} = join.{} {mask}, {fallthrough}, {taken}",
                    condition.suffix()
                ),
                format!(
                    "uint {dest} = {};",
                    operation.expression(&thread, "", "", config.size)
                ),
            ),
            Op::LaneMask {
                op,
                src0,
                src1,
                exec,
                half,
            } => {
                let _ = writeln!(
                    ir,
                    "{offset:04x}: {dest} = wave.mask {} {src0}, {src1} exec=0x{exec:08x}",
                    op.mnemonic()
                );
                emit_lane_mask(
                    &mut msl,
                    &operations,
                    &uniform,
                    index,
                    op,
                    src0,
                    src1,
                    exec,
                    half,
                    &mut replay_nodes,
                )?;
                continue;
            }
            Op::Store(value) => (
                format!("store.u32 output[index], {value}"),
                format!("output[index] = {value};"),
            ),
        };
        // Writing to a String is infallible.
        let _ = writeln!(ir, "{offset:04x}: {step}");
        append_msl(&mut msl, &format!("    {statement}\n"))?;
    }
    append_msl(&mut msl, "}\n")?;
    Ok(CompiledShader {
        config,
        disassembly,
        ir,
        msl,
    })
}

const MAX_MSL_BYTES: usize = 64 * 1024;
const MAX_REPLAY_NODES: usize = 8192;
const MAX_REPLAY_DEPTH: usize = 64;
const MAX_REPLAY_WORK: usize = 8192;
struct ReplayBudget {
    nodes: usize,
    work: usize,
    enforce_work: bool,
    wave: WaveSize,
}
impl Default for ReplayBudget {
    fn default() -> Self {
        Self {
            nodes: 0,
            work: 0,
            enforce_work: true,
            wave: WaveSize::Wave32,
        }
    }
}
impl ReplayBudget {
    fn charge(&mut self, operations: usize, multiplier: usize) -> Result<(), CompileError> {
        let total = operations
            .checked_mul(multiplier)
            .and_then(|cost| self.work.checked_add(cost))
            .filter(|&total| !self.enforce_work || total <= MAX_REPLAY_WORK)
            .ok_or_else(|| replay_error("replay weighted work exceeds 8192"))?;
        self.work = total;
        Ok(())
    }
}
fn replay_error(reason: &'static str) -> CompileError {
    CompileError::Verify { offset: 0, reason }
}
fn append_msl(msl: &mut String, text: &str) -> Result<(), CompileError> {
    if text.len() > MAX_MSL_BYTES.saturating_sub(msl.len()) {
        return Err(replay_error("generated MSL exceeds 64 KiB"));
    }
    msl.push_str(text);
    Ok(())
}

// One instance per lexical scope. Context IDs are interned; memo entries never
// escape a compare loop. Earlier uniform snapshots are reused, not re-evaluated.
struct Replay<'a> {
    msl: &'a mut String,
    operations: &'a [(usize, Op)],
    uniform: &'a [bool],
    root: usize,
    scope: String,
    budget: &'a mut ReplayBudget,
    multiplier: usize,
    scatter_depth: usize,
    contexts: Vec<(String, String)>,
    context_keys: std::collections::BTreeMap<String, usize>,
    memo: std::collections::BTreeMap<(usize, usize), String>,
    thread_context: bool,
}
impl<'a> Replay<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        msl: &'a mut String,
        operations: &'a [(usize, Op)],
        uniform: &'a [bool],
        root: usize,
        nodes: &'a mut ReplayBudget,
        lane: String,
        index: String,
        thread_context: bool,
    ) -> Self {
        Self {
            msl,
            operations,
            uniform,
            root,
            scope: root.to_string(),
            budget: nodes,
            multiplier: 1,
            scatter_depth: 0,
            contexts: vec![(lane, index)],
            context_keys: Default::default(),
            memo: Default::default(),
            thread_context,
        }
    }
    fn value(
        &mut self,
        value: Value,
        context: usize,
        depth: usize,
    ) -> Result<String, CompileError> {
        if value.0 < self.root && (self.uniform[value.0] || (context == 0 && self.thread_context)) {
            return Ok(value.to_string());
        }
        if let Some(name) = self.memo.get(&(value.0, context)) {
            return Ok(name.clone());
        }
        if depth > MAX_REPLAY_DEPTH {
            return Err(replay_error("replay depth exceeds 64"));
        }
        if self.budget.nodes >= MAX_REPLAY_NODES {
            return Err(replay_error("replay nodes exceed 8192"));
        }
        self.budget.charge(1, self.multiplier)?;
        self.budget.nodes += 1;
        let (lane, index) = self.contexts[context].clone();
        let operation = self.operations[value.0].1;
        let wave = self.budget.wave;
        let base_mask = wave.lanes() - 1;
        if depth >= MAX_REPLAY_DEPTH && matches!(operation, Op::Gather { .. } | Op::Scatter { .. })
        {
            return Err(replay_error("replay depth exceeds 64"));
        }
        if let Op::Scatter {
            address,
            data,
            offset,
            exec,
        } = operation
        {
            if self.scatter_depth != 0 {
                return Err(replay_error("nested scatter replay is unsupported"));
            }
            let multiplier = self
                .multiplier
                .checked_mul(32)
                .ok_or_else(|| replay_error("replay weighted work exceeds 8192"))?;
            // Fixed loop control, EXEC test, target calculation, comparison and assignment.
            self.budget.charge(6, multiplier)?;
            let name = format!("s{}_{}_{}", self.scope, value.0, context);
            let writer = format!("w{}_{}_{}", self.scope, value.0, context);
            append_msl(
                self.msl,
                &format!(
                    "    uint {name} = 0u;\n    for (uint {writer} = 0u; {writer} < 32u; ++{writer}) {{\n"
                ),
            )?;
            // A new lexical scope owns the writer's memoized expressions. No loop-local
            // temporary can escape, and both address/data paths carry the scatter depth.
            let writer_lane = if wave == WaveSize::Wave64 {
                format!("(({lane} & 32u) | {writer})")
            } else {
                writer.clone()
            };
            let mut body = Replay::new(
                self.msl,
                self.operations,
                self.uniform,
                self.root,
                self.budget,
                writer_lane.clone(),
                format!("((index & ~{base_mask}u) + {writer_lane})"),
                false,
            );
            body.scope = format!("{}_s{}_{}", self.scope, value.0, context);
            body.multiplier = multiplier;
            body.scatter_depth = self.scatter_depth + 1;
            let address = body.value(address, 0, depth + 1)?;
            let data = body.value(data, 0, depth + 1)?;
            let active = mask_test(exec, &writer_lane, wave);
            let destination = if wave == WaveSize::Wave64 {
                format!("({lane} & 31u)")
            } else {
                lane.clone()
            };
            append_msl(
                body.msl,
                &format!(
                    "        if ({active} && ((({address} + {offset}u) >> 2u) & 31u) == ({destination})) {{ {name} = {data}; }}\n    }}\n"
                ),
            )?;
            self.memo.insert((value.0, context), name.clone());
            return Ok(name);
        }
        let expression = if let Op::Gather {
            source,
            selection,
            exec,
        } = operation
        {
            let selected = match selection {
                GatherSelection::Address {
                    value: address,
                    offset,
                } => {
                    let address = self.value(address, context, depth)?;
                    format!("(({address} + {offset}u) >> 2u) & 31u")
                }
                GatherSelection::Table(_) => {
                    if wave == WaveSize::Wave64 {
                        format!("gather_{}[({lane}) & 31u]", value.0)
                    } else {
                        format!("gather_{}[{lane}]", value.0)
                    }
                }
            };
            let selected = if wave == WaveSize::Wave64 {
                format!("(({lane} & 32u) | ({selected}))")
            } else {
                selected
            };
            self.budget.charge(4, self.multiplier)?;
            let child = if let Some(&i) = self.context_keys.get(&selected) {
                i
            } else {
                let id = self.contexts.len();
                let name = format!("c{}_{}", self.scope, id);
                append_msl(self.msl, &format!("    uint {name} = {selected};\n"))?;
                self.context_keys.insert(selected, id);
                self.contexts
                    .push((name.clone(), format!("((index & ~{base_mask}u) + {name})")));
                id
            };
            let fetched = self.value(source, child, depth + 1)?;
            let selected_lane = &self.contexts[child].0;
            format!("{} ? {fetched} : 0u", mask_test(exec, selected_lane, wave))
        } else {
            let mut names = std::collections::BTreeMap::new();
            for operand in operation.operands() {
                names.insert(operand.0, self.value(operand, context, depth)?);
            }
            operation.expression(&|v| names[&v.0].clone(), &index, &lane, self.budget.wave)
        };
        let name = if context == 0 && lane == "lane" {
            format!("l{}", value.0)
        } else {
            format!("r{}_{}_{}", self.scope, value.0, context)
        };
        let indent = if self.contexts[0].0 == "lane" {
            "        "
        } else {
            "    "
        };
        append_msl(self.msl, &format!("{indent}uint {name} = {expression};\n"))?;
        self.memo.insert((value.0, context), name.clone());
        Ok(name)
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_selected_lane(
    msl: &mut String,
    operations: &[(usize, Op)],
    uniform: &[bool],
    index: usize,
    source: Value,
    selector: Value,
    nodes: &mut ReplayBudget,
) -> Result<(), CompileError> {
    let mask = nodes.wave.lanes() - 1;
    let zero = matches!(operations[selector.0].1, Op::Constant(0));
    let lane = if zero {
        "0u".into()
    } else {
        format!("({selector} & {mask}u)")
    };
    let source_index = if zero {
        format!("(index & ~{mask}u)")
    } else {
        format!("((index & ~{mask}u) + {lane})")
    };
    let mut replay = Replay::new(
        msl,
        operations,
        uniform,
        index,
        nodes,
        lane,
        source_index,
        false,
    );
    let source = replay.value(source, 0, 0)?;
    append_msl(
        replay.msl,
        &format!("    uint {} = {source};\n", Value(index)),
    )
}

#[allow(clippy::too_many_arguments)]
fn emit_lane_mask(
    msl: &mut String,
    operations: &[(usize, Op)],
    uniform: &[bool],
    index: usize,
    op: VectorCompareOp,
    src0: Value,
    src1: Value,
    exec: u64,
    half: u32,
    nodes: &mut ReplayBudget,
) -> Result<(), CompileError> {
    let wave = nodes.wave;
    let base_mask = wave.lanes() - 1;
    let end = half + 32;
    nodes.charge(4, 32)?;
    let dest = Value(index);
    append_msl(
        msl,
        &format!(
            "    uint {dest} = 0u;\n    for (uint lane = {half}u; lane < {end}u; ++lane) {{\n        uint lane_index = (index & ~{base_mask}u) + lane;\n        if (lane_index >= grid) {{ break; }}\n"
        ),
    )?;
    let mut replay = Replay::new(
        msl,
        operations,
        uniform,
        index,
        nodes,
        "lane".into(),
        "lane_index".into(),
        false,
    );
    replay.multiplier = 32;
    let a = replay.value(src0, 0, 0)?;
    let b = replay.value(src1, 0, 0)?;
    let accumulation = if wave == WaveSize::Wave32 {
        format!("((0x{exec:08x}u >> lane) & pass) << lane")
    } else {
        format!(
            "((0x{:08x}u >> (lane & 31u)) & pass) << (lane & 31u)",
            (exec >> half) as u32
        )
    };
    append_msl(
        replay.msl,
        &format!(
            "        uint pass = ({}) ? 1u : 0u;\n        {dest} |= {accumulation};\n    }}\n",
            compare_expression(op, &a, &b)
        ),
    )
}

fn decode_fixture(
    architecture: Architecture,
    bytes: &[u8],
) -> Result<Vec<DecodedInstruction>, CompileError> {
    if bytes.is_empty() || bytes.len() > MAX_CODE_BYTES || !bytes.len().is_multiple_of(4) {
        return Err(CompileError::InvalidLength { bytes: bytes.len() });
    }
    xenora_decoder::decode(architecture, bytes).map_err(CompileError::Decode)
}

fn lift(
    decoded: &[DecodedInstruction],
    code_len: usize,
    config: WaveConfig,
) -> Result<Vec<(usize, Op)>, CompileError> {
    let launch = config.launch;
    let wave = config.size;
    let lane_mask = wave.lanes() - 1;
    let mut operations = Vec::new();
    let mut registers = [None::<Value>; 256];
    let mut facts = Vec::<KnownBits>::new();
    let mut pending_load = None;
    let mut pending_ds: Option<(usize, Value)> = None;
    let mut stored = false;
    let mut store_pending = false;
    let mut scalars = [None; 128];
    // Each half retains independent provenance: known in scalars, runtime here,
    // or unknown in both. A half write invalidates only that runtime slot.
    let mut lane_scalars = [None::<Value>; 128];
    scalars[126] = Some(u32::MAX);
    // Wave32 high EXEC has no established initial numeric value in this ABI.
    if wave == WaveSize::Wave64 {
        scalars[127] = Some(u32::MAX);
    }
    let mut scc = None;
    let mut skip_before = 0;
    // Open runtime VCC region: end, mask, polarity and pre-branch VGPRs.
    let mut region: Option<(usize, MaskValue, MaskCondition, [Option<Value>; 256])> = None;

    // Every decoded instruction must satisfy the fixture layout and binding
    // contract, even when a known branch skips it.
    for (index, instruction) in decoded.iter().enumerate() {
        let offset = instruction.offset;
        let reason = match instruction.instruction {
            Instruction::ReadLaneB32 { dst, lane, .. }
                if wave == WaveSize::Wave64 && dst == lane =>
            {
                "Wave64 READLANE scalar source/destination overlap"
            }
            Instruction::VectorCompare {
                src0: VectorSource::Scalar(106 | 107),
                ..
            } if wave == WaveSize::Wave64 => {
                "Wave64 comparison explicit VCC source/destination overlap"
            }
            Instruction::DsSwizzleB32 {
                offset: immediate, ..
            } if (0xc000..0xe000).contains(&immediate) && immediate & 31 != 0 => {
                "DS_SWIZZLE masked rotate rejected: primary source conflict between AMD examples and pseudocode"
            }
            Instruction::Branch { target }
            | Instruction::BranchSccZero { target }
            | Instruction::BranchSccOne { target }
            | Instruction::BranchExecNonZero { target }
            | Instruction::BranchExecZero { target }
            | Instruction::BranchVccNonZero { target }
            | Instruction::BranchVccZero { target }
                if target <= offset
                    || decoded
                        .binary_search_by_key(&target, |instruction| instruction.offset)
                        .is_err() =>
            {
                "branch target must be a forward instruction boundary"
            }
            Instruction::BufferLoadDword {
                index, resource, ..
            }
            | Instruction::BufferLoadNarrow {
                index, resource, ..
            } if (index, resource) != (0, 0) => "unsupported resource or buffer index",
            Instruction::BufferStoreDword {
                index, resource, ..
            } if (index, resource) != (0, 4) => "unsupported resource or buffer index",
            Instruction::DsPermuteB32 { .. }
            | Instruction::DsBpermuteB32 { .. }
            | Instruction::DsSwizzleB32 { .. }
            | Instruction::Permlane16B32 { .. }
                if launch != Wave32Launch::FullWavesRequired =>
            {
                "gathers require full Wave32 launch"
            }
            Instruction::Export { .. }
            | Instruction::Interpolate { .. }
            | Instruction::ImageSampleLz { .. }
            | Instruction::ImageSampleLzArray { .. }
            | Instruction::ImageStore2D { .. } => {
                "graphics instruction is unsupported in a compute fixture"
            }
            Instruction::EndProgram if index + 1 != decoded.len() => {
                return Err(CompileError::Verify {
                    offset: decoded[index + 1].offset,
                    reason: "instruction follows program termination",
                });
            }
            _ => continue,
        };
        return Err(CompileError::Verify { offset, reason });
    }

    for instruction in decoded {
        let offset = instruction.offset;
        let error = |reason| CompileError::Verify { offset, reason };
        if let Some((end, mask, condition, before)) = region
            && offset >= end
        {
            region = None;
            for slot in 0..registers.len() {
                if let (Some(fallthrough), Some(taken)) = (registers[slot], before[slot])
                    && fallthrough.0 != taken.0
                {
                    let next = Value(operations.len());
                    operations.push((
                        end,
                        Op::JoinVcc {
                            condition,
                            mask,
                            fallthrough,
                            taken,
                        },
                    ));
                    let (left, right) = (facts[fallthrough.0], facts[taken.0]);
                    // Keep up to two candidate values when both edges have them.
                    let values = match (left.values, right.values) {
                        (Some([a, b]), Some([c, d])) => {
                            let mut union = vec![a, b, c, d];
                            union.sort_unstable();
                            union.dedup();
                            match union.as_slice() {
                                [only] => Some([*only; 2]),
                                [first, second] => Some([*first, *second]),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    facts.push(KnownBits {
                        zero: left.zero & right.zero,
                        one: left.one & right.one,
                        values,
                    });
                    registers[slot] = Some(next);
                }
            }
        }
        // docs/shaders/vector-compare.md section 6: checked whatever EXEC is.
        if let Some((_, _, condition, before)) = &region {
            match instruction.instruction {
                Instruction::VectorLshlrevB32 { src0, .. }
                | Instruction::VectorLshrrevB32 { src0, .. }
                | Instruction::VectorAshrrevI32 { src0, .. }
                | Instruction::VectorAndB32 { src0, .. }
                | Instruction::VectorOrB32 { src0, .. }
                | Instruction::VectorXorB32 { src0, .. }
                    if !matches!(src0, VectorSource::Register(_)) =>
                {
                    return Err(error(if *condition == MaskCondition::Zero {
                        "runtime s_cbranch_vccz integer SRC0 must be a VGPR"
                    } else {
                        "runtime s_cbranch_vccnz integer SRC0 must be a VGPR"
                    }));
                }
                _ => {}
            }
            let dst = match instruction.instruction {
                Instruction::MoveB32 { dst, .. }
                | Instruction::AddNoCarryU32 { dst, .. }
                | Instruction::VectorLshlrevB32 { dst, .. }
                | Instruction::VectorLshrrevB32 { dst, .. }
                | Instruction::VectorAshrrevI32 { dst, .. }
                | Instruction::VectorAndB32 { dst, .. }
                | Instruction::VectorOrB32 { dst, .. }
                | Instruction::VectorXorB32 { dst, .. } => dst,
                _ => {
                    return Err(error(if *condition == MaskCondition::Zero {
                        "code skipped by a runtime s_cbranch_vccz may contain only v_mov_b32, v_add_nc_u32_e64 and the hand-written integer VOP2 forms"
                    } else {
                        "code skipped by a runtime s_cbranch_vccnz may contain only v_mov_b32, v_add_nc_u32_e64 and the hand-written integer VOP2 forms"
                    }));
                }
            };
            if before[usize::from(dst)].is_none() {
                return Err(error(if *condition == MaskCondition::Zero {
                    "a VGPR written under a runtime s_cbranch_vccz must be initialized before the branch"
                } else {
                    "a VGPR written under a runtime s_cbranch_vccnz must be initialized before the branch"
                }));
            }
        }
        let read = |register: u8| {
            registers[usize::from(register)].ok_or_else(|| {
                error("source register is uninitialized, pending a load wait, or reserved for indexing")
            })
        };
        if offset < skip_before {
            continue;
        }
        let mut exec = u64::from(scalars[126].expect("static EXEC low"));
        if wave == WaveSize::Wave64 {
            exec |= u64::from(scalars[127].expect("static EXEC high")) << 32;
        }
        // Constant scalar writes invalidate any older lane-derived value. Untaken
        // conditional writes preserve both pieces of scalar state.
        let scalar_write = match instruction.instruction {
            Instruction::ScalarMoveB32 { dst, .. }
            | Instruction::ScalarUnary { dst, .. }
            | Instruction::ScalarAlu { dst, .. }
            | Instruction::ScalarBitSetB32 { dst, .. } => Some(dst),
            Instruction::ScalarConditionalMoveB32 { dst, .. }
            | Instruction::ScalarConditionalMoveImmediateI32 { dst, .. }
                if scc == Some(true) =>
            {
                Some(dst)
            }
            _ => None,
        };
        let scalar_sources = lane_scalars;
        if let Some(dst) = scalar_write.and_then(|dst| lane_scalars.get_mut(usize::from(dst))) {
            *dst = None;
        }
        let read_scalar = |source| match source {
            ScalarSource::Register(register) if scalar_sources[usize::from(register)].is_some() => {
                Err(error(
                    "scalar source has no known value; runtime scalar ALU unsupported",
                ))
            }
            ScalarSource::Immediate(value) => Ok(value),
            ScalarSource::Register(register) => scalars
                .get(usize::from(register))
                .copied()
                .flatten()
                .ok_or_else(|| error("scalar source has no known value")),
        };
        // Scalar instructions and waits still execute when all lanes are inactive.
        if exec == 0 {
            match instruction.instruction {
                Instruction::MoveB32 { .. }
                | Instruction::AddNoCarryU32 { .. }
                | Instruction::VectorLshlrevB32 { .. }
                | Instruction::VectorLshrrevB32 { .. }
                | Instruction::VectorAshrrevI32 { .. }
                | Instruction::VectorAndB32 { .. }
                | Instruction::VectorOrB32 { .. }
                | Instruction::VectorXorB32 { .. }
                | Instruction::VectorAddF32 { .. }
                | Instruction::VectorMulF32 { .. }
                | Instruction::VectorAlu { .. }
                | Instruction::VectorUnary { .. }
                | Instruction::VectorUnaryE64 { .. }
                | Instruction::VectorBinaryE64 { .. }
                | Instruction::VectorClampedE64 { .. }
                | Instruction::VectorProduct { .. }
                | Instruction::VectorBitBinary { .. }
                | Instruction::VectorMadSad { .. }
                | Instruction::VectorTernary { .. } => continue,
                _ => {}
            }
        }
        if exec != wave.full_mask()
            && matches!(
                instruction.instruction,
                Instruction::BufferLoadDword { .. }
                    | Instruction::BufferLoadNarrow { .. }
                    | Instruction::BufferStoreDword { .. }
            )
        {
            return Err(error(if wave == WaveSize::Wave64 {
                "vector memory requires full Wave64 EXEC"
            } else {
                "vector memory requires full Wave32 EXEC"
            }));
        }
        let next = Value(operations.len());
        let (dest, operation) = match instruction.instruction {
            Instruction::DsPermuteB32 {
                dst,
                addr,
                data,
                offset: immediate,
            } => {
                let slot = usize::from(dst);
                if dst == 0 || pending_ds.is_some() || pending_load.is_some_and(|(r, _)| r == slot)
                {
                    return Err(error(
                        "scatter writes reserved/pending destination or overlaps DS",
                    ));
                }
                let address = read(addr)?;
                let data = read(data)?;
                let old = registers[slot];
                operations.push((
                    offset,
                    Op::Scatter {
                        address,
                        data,
                        offset: immediate,
                        exec,
                    },
                ));
                facts.push(KnownBits::UNKNOWN);
                let value =
                    merge_lanes(&mut operations, &mut facts, offset, exec, next, old, wave)?;
                registers[slot] = None;
                pending_ds = Some((slot, value));
                continue;
            }
            Instruction::DsBpermuteB32 { dst, .. }
            | Instruction::DsSwizzleB32 { dst, .. }
            | Instruction::Permlane16B32 { dst, .. } => {
                let slot = usize::from(dst);
                let is_ds = !matches!(instruction.instruction, Instruction::Permlane16B32 { .. });
                if dst == 0
                    || pending_load.is_some_and(|(r, _)| r == slot)
                    || pending_ds.is_some_and(|(r, _)| r == slot)
                    || (is_ds && pending_ds.is_some())
                {
                    return Err(error(
                        "gather writes reserved/pending destination or overlaps DS",
                    ));
                }
                let (source, selection) = match instruction.instruction {
                    Instruction::DsBpermuteB32 {
                        addr, data, offset, ..
                    } => (
                        read(data)?,
                        GatherSelection::Address {
                            value: read(addr)?,
                            offset,
                        },
                    ),
                    Instruction::DsSwizzleB32 { src, offset, .. } => {
                        if exec != wave.full_mask() {
                            return Err(error("swizzle requires full EXEC"));
                        }
                        (read(src)?, GatherSelection::Table(swizzle_table(offset)))
                    }
                    Instruction::Permlane16B32 {
                        src,
                        select_lo,
                        select_hi,
                        cross_row,
                        ..
                    } => {
                        if exec != wave.full_mask() {
                            return Err(error("permlane requires full EXEC"));
                        }
                        let lo = read_scalar(ScalarSource::Register(select_lo))?;
                        let hi = read_scalar(ScalarSource::Register(select_hi))?;
                        let table = std::array::from_fn(|i| {
                            let nibble = (if i & 15 < 8 { lo } else { hi } >> ((i & 7) * 4)) & 15;
                            (((i as u32 & 16) ^ if cross_row { 16 } else { 0 }) | nibble) as u8
                        });
                        (read(src)?, GatherSelection::Table(table))
                    }
                    _ => unreachable!(),
                };
                let old = registers[slot];
                operations.push((
                    offset,
                    Op::Gather {
                        source,
                        selection,
                        exec,
                    },
                ));
                facts.push(KnownBits::UNKNOWN);
                let value =
                    merge_lanes(&mut operations, &mut facts, offset, exec, next, old, wave)?;
                if is_ds {
                    registers[slot] = None;
                    pending_ds = Some((slot, value));
                } else {
                    registers[slot] = Some(value);
                }
                continue;
            }
            Instruction::WaitLgkm => {
                if let Some((slot, value)) = pending_ds.take() {
                    registers[slot] = Some(value);
                }
                continue;
            }
            Instruction::WriteLaneB32 { dst, src, lane } => {
                let selected = scalars[usize::from(lane)].map(|bits| bits & lane_mask);
                let runtime_selector = if selected.is_none() {
                    if launch != WaveLaunch::FullWavesRequired {
                        return Err(error("runtime lane write selector requires full waves"));
                    }
                    Some(
                        lane_scalars[usize::from(lane)]
                            .ok_or_else(|| error("lane write selector has no captured value"))?,
                    )
                } else {
                    None
                };
                let old = read(dst)?;
                let value = match src {
                    ScalarSource::Immediate(bits) => {
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                    ScalarSource::Register(src @ 0..=105) => {
                        if let Some(bits) = scalars[usize::from(src)] {
                            constant(&mut operations, &mut facts, offset, bits)
                        } else {
                            lane_scalars[usize::from(src)]
                                .ok_or_else(|| error("scalar source has no known value"))?
                        }
                    }
                    ScalarSource::Register(_) => {
                        return Err(error(
                            "lane write data requires an ordinary SGPR or constant",
                        ));
                    }
                };
                let written = if let Some(selector) = runtime_selector {
                    let next = Value(operations.len());
                    operations.push((
                        offset,
                        Op::WriteLane {
                            selector,
                            data: value,
                            old,
                        },
                    ));
                    facts.push(KnownBits {
                        zero: facts[value.0].zero & facts[old.0].zero,
                        one: facts[value.0].one & facts[old.0].one,
                        values: None,
                    });
                    next
                } else {
                    merge_lanes(
                        &mut operations,
                        &mut facts,
                        offset,
                        1u64 << selected.unwrap(),
                        value,
                        Some(old),
                        wave,
                    )?
                };
                registers[usize::from(dst)] = Some(written);
                continue;
            }
            Instruction::ReadFirstLaneB32 { dst, src }
            | Instruction::ReadLaneB32 { dst, src, .. } => {
                if dst < 8 {
                    return Err(error("lane read would modify a resource descriptor"));
                }
                let source = read(src)?;
                let selector = match instruction.instruction {
                    Instruction::ReadFirstLaneB32 { .. } => {
                        let lane = if exec == 0 { 0 } else { exec.trailing_zeros() };
                        if lane != 0 && launch == Wave32Launch::PartialAllowed {
                            return Err(error(
                                "lane read requires selected lane zero for partial-wave safety",
                            ));
                        }
                        constant(&mut operations, &mut facts, offset, lane)
                    }
                    Instruction::ReadLaneB32 { lane, .. } => {
                        if let Some(bits) = scalars[usize::from(lane)] {
                            if bits & lane_mask != 0 && launch == Wave32Launch::PartialAllowed {
                                return Err(error(
                                    "lane read requires selected lane zero for partial-wave safety",
                                ));
                            }
                            constant(&mut operations, &mut facts, offset, bits & lane_mask)
                        } else if launch == Wave32Launch::FullWavesRequired {
                            lane_scalars[usize::from(lane)]
                                .ok_or_else(|| error("scalar source has no known value"))?
                        } else {
                            return Err(error("scalar source has no known value"));
                        }
                    }
                    _ => unreachable!(),
                };
                let next = Value(operations.len());
                operations.push((offset, Op::ReadLane { source, selector }));
                facts.push(KnownBits::UNKNOWN);
                scalars[usize::from(dst)] = None;
                lane_scalars[usize::from(dst)] = Some(next);
                continue;
            }
            Instruction::BufferLoadDword { data, .. }
            | Instruction::BufferLoadNarrow { data, .. } => {
                let slot = usize::from(data);
                if data == 0
                    || pending_load.is_some()
                    || pending_ds.is_some_and(|(dst, _)| dst == slot)
                {
                    return Err(error(
                        "load writes the index register or another load is outstanding",
                    ));
                }
                registers[slot] = None;
                pending_load = Some((slot, next));
                let kind = match instruction.instruction {
                    Instruction::BufferLoadNarrow { kind, .. } => Some(kind),
                    _ => None,
                };
                (None, Op::Load(kind))
            }
            Instruction::BufferStoreDword { data, .. } => {
                if stored {
                    return Err(error("only one output store is supported"));
                }
                stored = true;
                store_pending = true;
                (None, Op::Store(read(data)?))
            }
            Instruction::WaitLoad | Instruction::WaitVmcnt => {
                if let Some((register, value)) = pending_load.take() {
                    registers[register] = Some(value);
                }
                continue;
            }
            Instruction::WaitStore => {
                store_pending = false;
                continue;
            }
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Register(src),
            } => {
                let value = read(src)?;
                let slot = usize::from(dst);
                if dst == 0
                    || (pending_load.is_some_and(|(register, _)| register == slot)
                        || pending_ds.is_some_and(|(register, _)| register == slot))
                {
                    return Err(error(
                        "move writes the index register or replaces a pending load",
                    ));
                }
                registers[slot] = Some(merge_lanes(
                    &mut operations,
                    &mut facts,
                    offset,
                    exec,
                    value,
                    registers[slot],
                    wave,
                )?);
                continue;
            }
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Immediate(value),
            } => (Some(dst), Op::Constant(value)),
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Scalar(register),
            } if matches!(register, 106 | 107)
                && let Some(mask) = lane_scalars[usize::from(register)] =>
            {
                let slot = usize::from(dst);
                if dst == 0
                    || (pending_load.is_some_and(|(register, _)| register == slot)
                        || pending_ds.is_some_and(|(register, _)| register == slot))
                {
                    return Err(error(
                        "move writes the index register or replaces a pending load",
                    ));
                }
                registers[slot] = Some(merge_lanes(
                    &mut operations,
                    &mut facts,
                    offset,
                    exec,
                    mask,
                    registers[slot],
                    wave,
                )?);
                continue;
            }
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Scalar(register),
            } if register <= 105
                && scalars[usize::from(register)].is_none()
                && let Some(value) = lane_scalars[usize::from(register)] =>
            {
                let slot = usize::from(dst);
                if dst == 0
                    || (pending_load.is_some_and(|(pending, _)| pending == slot)
                        || pending_ds.is_some_and(|(pending, _)| pending == slot))
                {
                    return Err(error(
                        "move writes the index register or replaces a pending load",
                    ));
                }
                registers[slot] = Some(merge_lanes(
                    &mut operations,
                    &mut facts,
                    offset,
                    exec,
                    value,
                    registers[slot],
                    wave,
                )?);
                continue;
            }
            // Other SGPR values must be known during translation.
            Instruction::MoveB32 {
                dst,
                src: VectorSource::Scalar(register),
            } => (
                Some(dst),
                Op::Constant(read_scalar(ScalarSource::Register(register))?),
            ),
            Instruction::AddNoCarryU32 { dst, src0, src1 } => {
                (Some(dst), Op::Binary(Binary::Add, read(src0)?, read(src1)?))
            }
            Instruction::VectorLshlrevB32 { dst, src0, src1 }
            | Instruction::VectorLshrrevB32 { dst, src0, src1 }
            | Instruction::VectorAshrrevI32 { dst, src0, src1 }
            | Instruction::VectorAndB32 { dst, src0, src1 }
            | Instruction::VectorOrB32 { dst, src0, src1 }
            | Instruction::VectorXorB32 { dst, src0, src1 } => {
                let left = match src0 {
                    VectorSource::Register(register) => read(register)?,
                    VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                        match (
                            lane_scalars[usize::from(register)],
                            scalars[usize::from(register)],
                        ) {
                            (Some(mask), _) => mask,
                            (None, Some(bits)) => {
                                constant(&mut operations, &mut facts, offset, bits)
                            }
                            (None, None) => {
                                return Err(error("vector integer reads uninitialized VCC_LO"));
                            }
                        }
                    }
                    VectorSource::Scalar(register) => {
                        let bits = read_scalar(ScalarSource::Register(register))?;
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                    VectorSource::Immediate(bits) => {
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                };
                let right = read(src1)?;
                let (operator, left, right) = match instruction.instruction {
                    Instruction::VectorLshlrevB32 { .. } => (Binary::ShiftLeft, right, left),
                    Instruction::VectorLshrrevB32 { .. } => (Binary::ShiftRight, right, left),
                    Instruction::VectorAshrrevI32 { .. } => {
                        (Binary::ArithmeticShiftRight, right, left)
                    }
                    Instruction::VectorAndB32 { .. } => (Binary::And, left, right),
                    Instruction::VectorOrB32 { .. } => (Binary::Or, left, right),
                    _ => (Binary::Xor, left, right),
                };
                (Some(dst), Op::Binary(operator, left, right))
            }
            Instruction::ScalarMoveB32 { dst, src } => {
                let value = read_scalar(src)?;
                if dst < 8 {
                    return Err(error("scalar move would modify a resource descriptor"));
                }
                let slot = scalars
                    .get_mut(usize::from(dst))
                    .ok_or_else(|| error("scalar destination exceeds the fixture register file"))?;
                *slot = Some(value);
                continue;
            }
            Instruction::ScalarBitSetB32 { dst, index, value } => {
                if dst < 8 {
                    return Err(error(
                        "scalar bit update would modify a resource descriptor",
                    ));
                }
                let old = read_scalar(ScalarSource::Register(dst))?;
                let index = read_scalar(index)?;
                scalars[usize::from(dst)] = Some(scalar_bit_set(old, index, value));
                continue;
            }
            Instruction::ScalarBitCompareB32 { src, index, value } => {
                scc = Some(scalar_bit_test(
                    read_scalar(src)?,
                    read_scalar(index)?,
                    value,
                ));
                continue;
            }
            Instruction::ScalarConditionalMoveB32 { dst, .. }
            | Instruction::ScalarConditionalMoveImmediateI32 { dst, .. } => {
                if dst < 8 {
                    return Err(error(
                        "scalar conditional move would modify a resource descriptor",
                    ));
                }
                let take =
                    scc.ok_or_else(|| error("scalar conditional move reads uninitialized SCC"))?;
                if take {
                    let value = match instruction.instruction {
                        Instruction::ScalarConditionalMoveB32 { src, .. } => read_scalar(src)?,
                        Instruction::ScalarConditionalMoveImmediateI32 { immediate, .. } => {
                            immediate as i32 as u32
                        }
                        _ => unreachable!(),
                    };
                    scalars[usize::from(dst)] = Some(value);
                }
                continue;
            }
            Instruction::ScalarUnary { op, dst, src } => {
                let src = read_scalar(src)?;
                if dst < 8 {
                    return Err(error("scalar unary would modify a resource descriptor"));
                }
                let (value, flag) = op.evaluate(src);
                let slot = scalars
                    .get_mut(usize::from(dst))
                    .ok_or_else(|| error("scalar destination exceeds the fixture register file"))?;
                *slot = Some(value);
                if let Some(flag) = flag {
                    scc = Some(flag);
                }
                continue;
            }
            Instruction::ScalarAlu {
                op,
                dst,
                src0,
                src1,
            } => {
                let (src0, src1) = (read_scalar(src0)?, read_scalar(src1)?);
                let scc_in = if op.reads_scc() {
                    scc.ok_or_else(|| error("SOP2 SCC input reads uninitialized SCC"))?
                } else {
                    false
                };
                if dst < 8 {
                    return Err(error("scalar ALU would modify a resource descriptor"));
                }
                let (value, flag) = op.evaluate(src0, src1, scc_in);
                let slot = scalars
                    .get_mut(usize::from(dst))
                    .ok_or_else(|| error("scalar destination exceeds the fixture register file"))?;
                *slot = Some(value);
                if let Some(flag) = flag {
                    scc = Some(flag);
                }
                continue;
            }
            Instruction::ScalarCompare { op, src0, src1 } => {
                scc = Some(op.test(read_scalar(src0)?, read_scalar(src1)?));
                continue;
            }
            // docs/shaders/vector-compare.md sections 3 and 5.
            Instruction::VectorCompare { op, src0, src1 } => {
                if exec == 0 {
                    scalars[VCC_HI] = None;
                    scalars[VCC_LO] = Some(0);
                    lane_scalars[VCC_LO] = None;
                    lane_scalars[VCC_HI] = None;
                    if wave == WaveSize::Wave64 {
                        scalars[VCC_HI] = Some(0);
                    }
                    continue;
                }
                let constant =
                    |operations: &mut Vec<(usize, Op)>, facts: &mut Vec<KnownBits>, bits: u32| {
                        let value = Value(operations.len());
                        operations.push((offset, Op::Constant(bits)));
                        facts.push(KnownBits::constant(bits));
                        value
                    };
                let src0 = match src0 {
                    VectorSource::Register(register) => read(register)?,
                    VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                        match (
                            lane_scalars[usize::from(register)],
                            scalars[usize::from(register)],
                        ) {
                            (Some(mask), _) => mask,
                            (None, Some(bits)) => constant(&mut operations, &mut facts, bits),
                            (None, None) => {
                                return Err(error("compare reads uninitialized VCC_LO"));
                            }
                        }
                    }
                    VectorSource::Scalar(register) => {
                        let bits = read_scalar(ScalarSource::Register(register))?;
                        constant(&mut operations, &mut facts, bits)
                    }
                    VectorSource::Immediate(bits) => constant(&mut operations, &mut facts, bits),
                };
                let src1 = read(src1)?;
                let mask = Value(operations.len());
                operations.push((
                    offset,
                    Op::LaneMask {
                        op,
                        src0,
                        src1,
                        exec,
                        half: 0,
                    },
                ));
                facts.push(KnownBits::UNKNOWN);
                let high = if wave == WaveSize::Wave64 {
                    let high = Value(operations.len());
                    operations.push((
                        offset,
                        Op::LaneMask {
                            op,
                            src0,
                            src1,
                            exec,
                            half: 32,
                        },
                    ));
                    facts.push(KnownBits::UNKNOWN);
                    Some(high)
                } else {
                    None
                };
                scalars[VCC_LO] = None;
                scalars[VCC_HI] = None;
                lane_scalars[VCC_LO] = Some(mask);
                lane_scalars[VCC_HI] = high;
                continue;
            }
            Instruction::BranchVccZero { target } | Instruction::BranchVccNonZero { target }
                if lane_scalars[VCC_LO].is_some()
                    || (wave == WaveSize::Wave64 && lane_scalars[VCC_HI].is_some()) =>
            {
                let mut projection = |slot: usize| -> Result<Value, CompileError> {
                    if let Some(value) = lane_scalars[slot] {
                        Ok(value)
                    } else {
                        Ok(constant(
                            &mut operations,
                            &mut facts,
                            offset,
                            scalars[slot].ok_or_else(|| {
                                error("conditional branch reads uninitialized VCC half")
                            })?,
                        ))
                    }
                };
                let mask = MaskValue {
                    lo: projection(VCC_LO)?,
                    hi: if wave == WaveSize::Wave64 {
                        Some(projection(VCC_HI)?)
                    } else {
                        None
                    },
                };
                let condition =
                    if matches!(instruction.instruction, Instruction::BranchVccZero { .. }) {
                        MaskCondition::Zero
                    } else {
                        MaskCondition::NonZero
                    };
                if pending_load.is_some() || pending_ds.is_some() {
                    return Err(error(if condition == MaskCondition::Zero {
                        "a runtime s_cbranch_vccz requires no outstanding vector load"
                    } else {
                        "a runtime s_cbranch_vccnz requires no outstanding vector load"
                    }));
                }
                region = Some((target, mask, condition, registers));
                (
                    None,
                    Op::RuntimeBranch {
                        condition,
                        mask,
                        target,
                    },
                )
            }
            Instruction::BranchSccZero { target }
            | Instruction::BranchSccOne { target }
            | Instruction::BranchExecNonZero { target }
            | Instruction::BranchExecZero { target }
            | Instruction::BranchVccNonZero { target }
            | Instruction::BranchVccZero { target } => {
                let taken = match instruction.instruction {
                    Instruction::BranchSccZero { .. } => {
                        !scc.ok_or_else(|| error("conditional branch reads uninitialized SCC"))?
                    }
                    Instruction::BranchSccOne { .. } => {
                        scc.ok_or_else(|| error("conditional branch reads uninitialized SCC"))?
                    }
                    Instruction::BranchVccZero { .. } | Instruction::BranchVccNonZero { .. } => {
                        let lo = scalars[VCC_LO].ok_or_else(|| {
                            error("conditional branch reads uninitialized VCC_LO")
                        })?;
                        let hi = if wave == WaveSize::Wave64 {
                            scalars[VCC_HI].ok_or_else(|| {
                                error("conditional branch reads uninitialized VCC_HI")
                            })?
                        } else {
                            0
                        };
                        let zero = (lo | hi) == 0;
                        if matches!(instruction.instruction, Instruction::BranchVccZero { .. }) {
                            zero
                        } else {
                            !zero
                        }
                    }
                    Instruction::BranchExecNonZero { .. } => exec != 0,
                    _ => exec == 0,
                };
                if taken {
                    skip_before = target;
                }
                (None, Op::ScalarBranch { target, taken })
            }
            Instruction::Branch { target } => {
                skip_before = target;
                (
                    None,
                    Op::ScalarBranch {
                        target,
                        taken: true,
                    },
                )
            }
            Instruction::VectorProduct {
                op,
                dst,
                src0,
                src1,
            } => {
                let mut values = [Value(0); 2];
                for (slot, src) in values.iter_mut().zip([src0, src1]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                (Some(dst), Op::Product(op, values[0], values[1]))
            }
            Instruction::VectorClampedE64 {
                op,
                dst,
                src0,
                src1,
            } => {
                let mut values = [Value(0); 2];
                for (slot, src) in values.iter_mut().zip([src0, src1]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                (
                    Some(dst),
                    Op::Binary(Binary::Clamped(op), values[0], values[1]),
                )
            }
            Instruction::VectorBinaryE64 {
                op,
                dst,
                src0,
                src1,
            } => {
                let mut values = [Value(0); 2];
                for (slot, src) in values.iter_mut().zip([src0, src1]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                let (operator, reverse) = alias_binary(op);
                let [left, right] = if reverse {
                    [values[1], values[0]]
                } else {
                    values
                };
                (Some(dst), Op::Binary(operator, left, right))
            }
            Instruction::VectorBitBinary {
                op,
                dst,
                src0,
                src1,
            } => {
                let mut values = [Value(0); 2];
                for (slot, src) in values.iter_mut().zip([src0, src1]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                (Some(dst), Op::BitBinary(op, values[0], values[1]))
            }
            Instruction::VectorTernary {
                op,
                dst,
                src0,
                src1,
                src2,
            } => {
                let mut values = [Value(0); 3];
                for (slot, src) in values.iter_mut().zip([src0, src1, src2]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                (Some(dst), Op::Ternary(op, values[0], values[1], values[2]))
            }
            Instruction::VectorMadSad {
                op,
                clamp,
                dst,
                src0,
                src1,
                src2,
            } => {
                let mut values = [Value(0); 3];
                for (slot, src) in values.iter_mut().zip([src0, src1, src2]) {
                    *slot = match src {
                        VectorSource::Register(register) => read(register)?,
                        VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                            match (
                                lane_scalars[usize::from(register)],
                                scalars[usize::from(register)],
                            ) {
                                (Some(mask), _) => mask,
                                (None, Some(bits)) => {
                                    constant(&mut operations, &mut facts, offset, bits)
                                }
                                (None, None) => {
                                    return Err(error("VOP3 integer reads uninitialized VCC_LO"));
                                }
                            }
                        }
                        VectorSource::Scalar(register) => {
                            let bits = read_scalar(ScalarSource::Register(register))?;
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                        VectorSource::Immediate(bits) => {
                            constant(&mut operations, &mut facts, offset, bits)
                        }
                    };
                }
                (
                    Some(dst),
                    Op::MadSad(op, clamp, values[0], values[1], values[2]),
                )
            }
            Instruction::VectorUnary { op, dst, src }
            | Instruction::VectorUnaryE64 { op, dst, src } => {
                let source = match src {
                    VectorSource::Register(register) => read(register)?,
                    VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                        match (
                            lane_scalars[usize::from(register)],
                            scalars[usize::from(register)],
                        ) {
                            (Some(mask), _) => mask,
                            (None, Some(bits)) => {
                                constant(&mut operations, &mut facts, offset, bits)
                            }
                            (None, None) => {
                                return Err(error("vector unary reads uninitialized VCC_LO"));
                            }
                        }
                    }
                    VectorSource::Scalar(register) => {
                        let bits = read_scalar(ScalarSource::Register(register))?;
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                    VectorSource::Immediate(bits) => {
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                };
                (Some(dst), Op::Unary(op, source))
            }
            Instruction::VectorAlu {
                op,
                dst,
                src0,
                src1,
            } => {
                let src0 = match src0 {
                    VectorSource::Register(register) => read(register)?,
                    VectorSource::Scalar(register) if matches!(register, 106 | 107) => {
                        match (
                            lane_scalars[usize::from(register)],
                            scalars[usize::from(register)],
                        ) {
                            (Some(mask), _) => mask,
                            (None, Some(bits)) => {
                                constant(&mut operations, &mut facts, offset, bits)
                            }
                            (None, None) => {
                                return Err(error("vector ALU reads uninitialized VCC_LO"));
                            }
                        }
                    }
                    VectorSource::Scalar(register) => {
                        let bits = read_scalar(ScalarSource::Register(register))?;
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                    VectorSource::Immediate(bits) => {
                        constant(&mut operations, &mut facts, offset, bits)
                    }
                };
                let src1 = read(src1)?;
                let (left, right) = if op.swaps_sources() {
                    (src1, src0)
                } else {
                    (src0, src1)
                };
                (
                    Some(dst),
                    Op::Binary(Binary::Vector(op.lowering()), left, right),
                )
            }
            Instruction::VectorAddF32 { dst, src0, src1 } => (
                Some(dst),
                Op::Binary(Binary::FloatAdd, read(src0)?, read(src1)?),
            ),
            Instruction::VectorMulF32 { dst, src0, src1 } => (
                Some(dst),
                Op::Binary(Binary::FloatMul, read(src0)?, read(src1)?),
            ),
            Instruction::Export { .. }
            | Instruction::Interpolate { .. }
            | Instruction::ImageSampleLz { .. }
            | Instruction::ImageSampleLzArray { .. }
            | Instruction::ImageStore2D { .. } => {
                return Err(error(
                    "graphics instruction is unsupported in a compute fixture",
                ));
            }
            Instruction::EndProgram => {
                if !stored || store_pending || pending_load.is_some() || pending_ds.is_some() {
                    return Err(error(
                        "termination requires one completed store and no pending loads",
                    ));
                }
                return Ok(operations);
            }
            #[allow(unreachable_patterns)]
            _ => {
                return Err(error(
                    "instruction requires an unsupported compute execution contract",
                ));
            }
        };
        let value_bits = match operation {
            Op::Constant(bits) => KnownBits::constant(bits),
            Op::Binary(Binary::And, left, right) => facts[left.0].and(facts[right.0]),
            Op::Binary(Binary::Or, left, right) => facts[left.0].or(facts[right.0]),
            Op::Binary(Binary::Xor, left, right) => facts[left.0].xor(facts[right.0]),
            Op::Binary(operator @ (Binary::FloatAdd | Binary::FloatMul), left, right) => {
                exact_float_result(operator, facts[left.0], facts[right.0])
                    .ok_or_else(|| error("FP32 operation is not proven exact for positive normal operands and results"))?
            }
            _ => KnownBits::UNKNOWN,
        };
        // Arms may push operand constants first, so the result is the next slot
        // now rather than `next`.
        let produced = Value(operations.len());
        facts.push(value_bits);
        operations.push((offset, operation));
        if let Some(dest) = dest {
            let slot = usize::from(dest);
            if dest == 0
                || (pending_load.is_some_and(|(register, _)| register == slot)
                    || pending_ds.is_some_and(|(register, _)| register == slot))
            {
                return Err(error(
                    "instruction writes the index register or replaces a pending load",
                ));
            }
            registers[slot] = Some(merge_lanes(
                &mut operations,
                &mut facts,
                offset,
                exec,
                produced,
                registers[slot],
                wave,
            )?);
        }
    }
    Err(CompileError::Verify {
        offset: code_len,
        reason: "missing program termination",
    })
}

/// A wave-uniform constant operand at `offset`.
fn constant(
    operations: &mut Vec<(usize, Op)>,
    facts: &mut Vec<KnownBits>,
    offset: usize,
    bits: u32,
) -> Value {
    let value = Value(operations.len());
    operations.push((offset, Op::Constant(bits)));
    facts.push(KnownBits::constant(bits));
    value
}

// Keeping a value for inactive lanes requires a previously initialized destination.
fn merge_lanes(
    operations: &mut Vec<(usize, Op)>,
    facts: &mut Vec<KnownBits>,
    offset: usize,
    mask: u64,
    active: Value,
    inactive: Option<Value>,
    wave: WaveSize,
) -> Result<Value, CompileError> {
    if mask == wave.full_mask() {
        return Ok(active);
    }
    let inactive = inactive.ok_or(CompileError::Verify {
        offset,
        reason: "partial EXEC requires an initialized vector destination",
    })?;
    let next = Value(operations.len());
    operations.push((
        offset,
        Op::MergeLanes {
            mask,
            active,
            inactive,
        },
    ));
    facts.push(KnownBits {
        zero: facts[active.0].zero & facts[inactive.0].zero,
        one: facts[active.0].one & facts[inactive.0].one,
        values: None,
    });
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::{Binary, KnownBits, exact_float_result};

    #[test]
    fn propagates_known_bits_through_integer_operators() {
        let left = KnownBits::constant(0b1100);
        let right = KnownBits::constant(0b1010);
        let and = left.and(right);
        assert_eq!((and.zero, and.one), (!0b1000, 0b1000));
        let xor = left.xor(right);
        assert_eq!((xor.zero, xor.one), (!0b0110, 0b0110));
    }

    #[test]
    fn rejects_unbounded_or_invalid_float_operands() {
        let one = KnownBits::constant(1.0f32.to_bits());
        let varying = KnownBits::UNKNOWN
            .and(KnownBits::constant(0x0040_0000))
            .xor(one);
        assert_eq!(varying.positive_normal_candidates(), Some([1.0, 1.5]));
        assert_eq!(one.and(one).positive_normal_candidates(), Some([1.0; 2]));
        assert!(KnownBits::UNKNOWN.positive_normal_candidates().is_none());
        for invalid in [0x8000_0000, 0xbf80_0000, 1, 0x7f80_0000] {
            assert!(
                KnownBits::constant(invalid)
                    .positive_normal_candidates()
                    .is_none()
            );
        }
    }

    #[test]
    fn keeps_only_bits_shared_by_every_exact_result() {
        let operand = KnownBits::UNKNOWN
            .and(KnownBits::constant(0x0040_0000))
            .xor(KnownBits::constant(1.0f32.to_bits()));
        let sum = exact_float_result(Binary::FloatAdd, operand, operand).unwrap();
        // The sums 2, 2.5 and 3 differ only in the two highest fraction bits.
        assert_eq!(sum.zero, 0xbf9f_ffff);
        assert_eq!(sum.one, 0x4000_0000);
        assert!(sum.values.is_none());
    }
}

#[cfg(test)]
#[path = "gather_replay_tests.rs"]
mod gather_replay_tests;
