//! Typed operands and instruction forms for the bounded gfx1030 compute and graphics subsets.

use std::fmt;

pub mod meta;
pub mod primitive;
mod rdna2;
pub mod semantics;
#[rustfmt::skip]
mod rdna2_meta;
#[rustfmt::skip]
mod semantics_gen;
pub use rdna2::{
    BUFFER_LOAD_DWORD, BUFFER_STORE_DWORD, EXP, IMAGE_SAMPLE_LZ, IMAGE_STORE, S_BRANCH,
    S_CBRANCH_EXECZ, S_CBRANCH_SCC0, S_CBRANCH_VCCZ, S_ENDPGM, S_MOV_B32, S_WAITCNT,
    S_WAITCNT_VSCNT, V_ADD_F32, V_ADD_NC_U32, V_AND_B32, V_ASHRREV_I32, V_CMP_EQ_I32, V_CMP_EQ_U32,
    V_CMP_GE_I32, V_CMP_GE_U32, V_CMP_GT_I32, V_CMP_GT_U32, V_CMP_LE_I32, V_CMP_LE_U32,
    V_CMP_LT_I32, V_CMP_LT_U32, V_CMP_NE_I32, V_CMP_NE_U32, V_INTERP_P1_F32, V_INTERP_P2_F32,
    V_LSHLREV_B32, V_LSHRREV_B32, V_MOV_B32, V_MUL_F32, V_OR_B32, V_XOR_B32,
};
pub use semantics_gen::{ScalarAluOp, ScalarCompareOp, ScalarUnaryOp, VectorAluOp, VectorUnaryOp};

/// SGPR index of `vcc_lo`, the Wave32 vector condition code.
pub const VCC_LO: u8 = 106;

/// Selects the instruction encoding independently of the container and execution ABI.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Architecture {
    /// LLVM's gfx1030 target for AMD RDNA 2.
    Gfx1030,
}

/// A generated first-word opcode pattern and its base length, before literal extensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Opcode {
    mask: u32,
    bits: u32,
    bytes: u8,
}

impl Opcode {
    /// Checks the format and opcode bits. The decoder must still validate operands.
    pub fn matches(self, word: u32) -> bool {
        word & self.mask == self.bits
    }

    /// Returns the base instruction size in bytes.
    pub fn byte_len(self) -> u8 {
        self.bytes
    }
}

/// A 32-bit source for an unmodified vector move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorSource {
    /// VGPR index.
    Register(u8),
    /// Normal SGPR index (s0-s105) or `vcc_lo` (106), read as a wave-uniform value.
    Scalar(u8),
    /// Inline integer or literal word, represented by its bits.
    Immediate(u32),
}

impl fmt::Display for VectorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register(register) => write!(formatter, "v{register}"),
            Self::Scalar(VCC_LO) => formatter.write_str("vcc_lo"),
            Self::Scalar(register) => write!(formatter, "s{register}"),
            Self::Immediate(value) => write!(formatter, "0x{value:x}"),
        }
    }
}

/// Value-only integer binary operation with two full VOP3 source operands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorBinaryOp {
    /// Existing registry arithmetic semantics.
    Alu(VectorAluOp),
    /// `v_and_b32`.
    AndB32,
    /// `v_or_b32`.
    OrB32,
    /// `v_xor_b32`.
    XorB32,
    /// `v_lshlrev_b32`.
    LshlrevB32,
    /// `v_lshrrev_b32`.
    LshrrevB32,
    /// `v_ashrrev_i32`.
    AshrrevI32,
}
impl VectorBinaryOp {
    /// Canonical e64 assembly mnemonic.
    pub const fn assembly_name(self) -> &'static str {
        match self {
            Self::Alu(VectorAluOp::MulI32I24) => "v_mul_i32_i24_e64",
            Self::Alu(VectorAluOp::MulHiI32I24) => "v_mul_hi_i32_i24_e64",
            Self::Alu(VectorAluOp::MulU32U24) => "v_mul_u32_u24_e64",
            Self::Alu(VectorAluOp::MulHiU32U24) => "v_mul_hi_u32_u24_e64",
            Self::Alu(VectorAluOp::MinI32) => "v_min_i32_e64",
            Self::Alu(VectorAluOp::MaxI32) => "v_max_i32_e64",
            Self::Alu(VectorAluOp::MinU32) => "v_min_u32_e64",
            Self::Alu(VectorAluOp::MaxU32) => "v_max_u32_e64",
            Self::Alu(VectorAluOp::XnorB32) => "v_xnor_b32_e64",
            Self::Alu(VectorAluOp::AddNcU32) => "v_add_nc_u32_e64",
            Self::Alu(VectorAluOp::SubNcU32) => "v_sub_nc_u32_e64",
            Self::Alu(VectorAluOp::SubrevNcU32) => "v_subrev_nc_u32_e64",
            Self::AndB32 => "v_and_b32_e64",
            Self::OrB32 => "v_or_b32_e64",
            Self::XorB32 => "v_xor_b32_e64",
            Self::LshlrevB32 => "v_lshlrev_b32_e64",
            Self::LshrrevB32 => "v_lshrrev_b32_e64",
            Self::AshrrevI32 => "v_ashrrev_i32_e64",
        }
    }
}

/// Integer saturation to the signed or unsigned 32-bit destination range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorClampOp {
    /// Saturating `v_mul_i32_i24`.
    MulI24,
    /// Saturating `v_mul_u32_u24`.
    MulU24,
    /// Saturating `v_add_nc_u32`.
    AddU32,
    /// Saturating `v_sub_nc_u32`.
    SubU32,
    /// Saturating `v_subrev_nc_u32`.
    SubrevU32,
}
impl VectorClampOp {
    /// Canonical e64 mnemonic (the clamp suffix belongs to the instruction).
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::MulI24 => "v_mul_i32_i24_e64",
            Self::MulU24 => "v_mul_u32_u24_e64",
            Self::AddU32 => "v_add_nc_u32_e64",
            Self::SubU32 => "v_sub_nc_u32_e64",
            Self::SubrevU32 => "v_subrev_nc_u32_e64",
        }
    }
}

/// Three-source 32-bit multiply-add or sum-of-absolute-differences operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorMadSadOp {
    /// `v_mad_i32_i24`.
    MadI32I24,
    /// `v_mad_u32_u24`.
    MadU32U24,
    /// `v_sad_u8`.
    SadU8,
    /// `v_sad_hi_u8`.
    SadHiU8,
    /// `v_sad_u16`.
    SadU16,
    /// `v_sad_u32`.
    SadU32,
}
impl VectorMadSadOp {
    /// Canonical mnemonic; the instruction carries the optional clamp suffix.
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::MadI32I24 => "v_mad_i32_i24",
            Self::MadU32U24 => "v_mad_u32_u24",
            Self::SadU8 => "v_sad_u8",
            Self::SadHiU8 => "v_sad_hi_u8",
            Self::SadU16 => "v_sad_u16",
            Self::SadU32 => "v_sad_u32",
        }
    }
}

/// Full-width, value-only 32-bit vector product.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorProductOp {
    /// `v_mul_lo_u32`.
    MulLoU32,
    /// `v_mul_hi_u32`.
    MulHiU32,
    /// `v_mul_hi_i32`.
    MulHiI32,
}

impl VectorProductOp {
    /// Returns the unmodified VOP3 mnemonic.
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::MulLoU32 => "v_mul_lo_u32",
            Self::MulHiU32 => "v_mul_hi_u32",
            Self::MulHiI32 => "v_mul_hi_i32",
        }
    }
}

/// Value-only two-source vector bit operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorBitBinaryOp {
    /// `v_bfm_b32`.
    BfmB32,
    /// `v_bcnt_u32_b32`.
    BcntU32B32,
}
impl VectorBitBinaryOp {
    /// Returns the unmodified VOP3 mnemonic.
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::BfmB32 => "v_bfm_b32",
            Self::BcntU32B32 => "v_bcnt_u32_b32",
        }
    }
}

/// Value-only three-source 32-bit vector operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorTernaryOp {
    /// `v_bfi_b32`.
    BfiB32,
    /// `v_add3_u32`.
    Add3U32,
    /// `v_and_or_b32`.
    AndOrB32,
    /// `v_bfe_u32`.
    BfeU32,
    /// `v_alignbit_b32`.
    AlignbitB32,
    /// `v_or3_b32`.
    Or3B32,
    /// `v_xor3_b32`.
    Xor3B32,
    /// `v_bfe_i32`.
    BfeI32,
    /// `v_lshl_add_u32`.
    LshlAddU32,
    /// `v_add_lshl_u32`.
    AddLshlU32,
    /// `v_lshl_or_b32`.
    LshlOrB32,
    /// `v_min3_i32`.
    Min3I32,
    /// `v_min3_u32`.
    Min3U32,
    /// `v_max3_i32`.
    Max3I32,
    /// `v_max3_u32`.
    Max3U32,
    /// `v_med3_i32`.
    Med3I32,
    /// `v_med3_u32`.
    Med3U32,
}

impl VectorTernaryOp {
    /// Returns the unmodified VOP3 mnemonic.
    pub const fn mnemonic(self) -> &'static str {
        match self {
            Self::BfiB32 => "v_bfi_b32",
            Self::Add3U32 => "v_add3_u32",
            Self::AndOrB32 => "v_and_or_b32",
            Self::BfeU32 => "v_bfe_u32",
            Self::AlignbitB32 => "v_alignbit_b32",
            Self::Or3B32 => "v_or3_b32",
            Self::Xor3B32 => "v_xor3_b32",
            Self::BfeI32 => "v_bfe_i32",
            Self::LshlAddU32 => "v_lshl_add_u32",
            Self::AddLshlU32 => "v_add_lshl_u32",
            Self::LshlOrB32 => "v_lshl_or_b32",
            Self::Min3I32 => "v_min3_i32",
            Self::Min3U32 => "v_min3_u32",
            Self::Max3I32 => "v_max3_i32",
            Self::Max3U32 => "v_max3_u32",
            Self::Med3I32 => "v_med3_i32",
            Self::Med3U32 => "v_med3_u32",
        }
    }
}

/// A scalar 32-bit source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarSource {
    /// A normal SGPR index.
    Register(u8),
    /// Inline integer or literal word, represented by its bits.
    Immediate(u32),
}

impl fmt::Display for ScalarSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Register(register) => write!(formatter, "s{register}"),
            Self::Immediate(value) => match value as i32 {
                integer @ -16..=64 => write!(formatter, "{integer}"),
                _ => write!(formatter, "0x{value:x}"),
            },
        }
    }
}

/// Relation tested by a VOPC integer compare; S0 is SRC0 and S1 is VSRC1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompareRelation {
    /// S0 < S1 (opcode offset 1).
    Less,
    /// S0 == S1 (offset 2).
    Equal,
    /// S0 <= S1 (offset 3).
    LessOrEqual,
    /// S0 > S1 (offset 4).
    Greater,
    /// S0 != S1 (offset 5).
    NotEqual,
    /// S0 >= S1 (offset 6).
    GreaterOrEqual,
}

/// A 32-bit VOPC integer compare: one relation over signed or unsigned operands.
///
/// The per-lane test follows the RDNA 2 ISA guide, section 12.9 (Table 59) and
/// the pinned XML descriptions. VCC and EXEC rules are in
/// docs/shaders/vector-compare.md.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VectorCompareOp {
    /// The tested relation.
    pub relation: CompareRelation,
    /// `_i32` (two's complement) when true, `_u32` when false.
    pub signed: bool,
}

impl VectorCompareOp {
    /// Lower-case mnemonic of the 32-bit VOPC encoding.
    pub fn mnemonic(self) -> &'static str {
        const SIGNED: [&str; 6] = [
            "v_cmp_lt_i32_e32",
            "v_cmp_eq_i32_e32",
            "v_cmp_le_i32_e32",
            "v_cmp_gt_i32_e32",
            "v_cmp_ne_i32_e32",
            "v_cmp_ge_i32_e32",
        ];
        const UNSIGNED: [&str; 6] = [
            "v_cmp_lt_u32_e32",
            "v_cmp_eq_u32_e32",
            "v_cmp_le_u32_e32",
            "v_cmp_gt_u32_e32",
            "v_cmp_ne_u32_e32",
            "v_cmp_ge_u32_e32",
        ];
        let index = self.relation as usize;
        if self.signed {
            SIGNED[index]
        } else {
            UNSIGNED[index]
        }
    }

    /// The per-lane test result for operand bits S0 and S1.
    pub fn test(self, src0: u32, src1: u32) -> bool {
        let ordering = if self.signed {
            (src0 as i32).cmp(&(src1 as i32))
        } else {
            src0.cmp(&src1)
        };
        match self.relation {
            CompareRelation::Less => ordering.is_lt(),
            CompareRelation::Equal => ordering.is_eq(),
            CompareRelation::LessOrEqual => ordering.is_le(),
            CompareRelation::Greater => ordering.is_gt(),
            CompareRelation::NotEqual => ordering.is_ne(),
            CompareRelation::GreaterOrEqual => ordering.is_ge(),
        }
    }
}

/// Output targeted by a supported uncompressed four-component export.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportTarget {
    /// Vertex clip position.
    Position0,
    /// Vertex parameter slot zero.
    Parameter0,
    /// Pixel color attachment index (the supported decoder bounds this to 0..=7).
    RenderTarget(u8),
}

/// Phase of a pixel parameter interpolation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterpolationPhase {
    /// Initialize the destination with the base and I-coordinate terms.
    First,
    /// Add the J-coordinate term to the destination.
    Second,
}

/// Instruction forms accepted by the gfx1030 decoder.
///
/// Vector operations use VGPR indices and scalar operations use SGPR indices.
/// The buffer `resource` is the first SGPR of its four-word descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum Instruction {
    /// Indexed 32-bit buffer read into a VGPR.
    BufferLoadDword { data: u8, index: u8, resource: u8 },
    /// Indexed 32-bit buffer write from a VGPR.
    BufferStoreDword { data: u8, index: u8, resource: u8 },
    /// `s_waitcnt vmcnt(0)`.
    WaitLoad,
    /// `s_waitcnt_vscnt null, 0`.
    WaitStore,
    /// VOP1 unmodified 32-bit vector move.
    MoveB32 { dst: u8, src: VectorSource },
    /// VOP3 unsigned 32-bit addition with no carry or input/output modifiers.
    AddNoCarryU32 { dst: u8, src0: u8, src1: u8 },
    /// Unmodified full-width product; preserves SCC, VCC and EXEC.
    VectorProduct {
        op: VectorProductOp,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
    },
    /// Saturating e64 integer operation; preserves SCC, VCC and EXEC.
    VectorClampedE64 {
        op: VectorClampOp,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
    },
    /// Canonical e64 integer binary alias; preserves SCC, VCC and EXEC.
    VectorBinaryE64 {
        op: VectorBinaryOp,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
    },
    /// Canonical e64 integer unary alias; preserves SCC, VCC and EXEC.
    VectorUnaryE64 {
        op: VectorUnaryOp,
        dst: u8,
        src: VectorSource,
    },
    /// Unmodified mask/count operation; preserves SCC, VCC and EXEC.
    VectorBitBinary {
        op: VectorBitBinaryOp,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
    },
    /// MAD24/SAD with optional final destination saturation; preserves flags.
    VectorMadSad {
        op: VectorMadSadOp,
        clamp: bool,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
        src2: VectorSource,
    },
    /// Unmodified three-source integer operation; preserves SCC, VCC and EXEC.
    VectorTernary {
        op: VectorTernaryOp,
        dst: u8,
        src0: VectorSource,
        src1: VectorSource,
        src2: VectorSource,
    },
    /// SOP1 32-bit move to a normal SGPR or an EXEC/VCC half.
    ScalarMoveB32 { dst: u8, src: ScalarSource },
    /// SOP1 read/modify/write of one old-destination bit; preserves SCC.
    ScalarBitSetB32 {
        dst: u8,
        index: ScalarSource,
        value: bool,
    },
    /// SOPC tests a source bit against `value`, replacing SCC only.
    ScalarBitCompareB32 {
        src: ScalarSource,
        index: ScalarSource,
        value: bool,
    },
    /// SOP1 writes a normal SGPR only when incoming SCC is true; preserves SCC.
    ScalarConditionalMoveB32 { dst: u8, src: ScalarSource },
    /// SOPK conditionally writes a sign-extended immediate; preserves SCC.
    ScalarConditionalMoveImmediateI32 { dst: u8, immediate: i16 },
    /// SOP1 32-bit unary integer operation writing a normal SGPR (s0-s105) and,
    /// for some operations, SCC.
    ScalarUnary {
        op: ScalarUnaryOp,
        dst: u8,
        src: ScalarSource,
    },
    /// SOP2 32-bit integer operation writing a normal SGPR (s0-s105) and, for
    /// most operations, SCC; `s_cselect_b32` also reads SCC.
    ScalarAlu {
        op: ScalarAluOp,
        dst: u8,
        src0: ScalarSource,
        src1: ScalarSource,
    },
    /// SOPC 32-bit integer compare writing only SCC.
    ScalarCompare {
        op: ScalarCompareOp,
        src0: ScalarSource,
        src1: ScalarSource,
    },
    /// VOPC 32-bit integer compare writing a Wave32 mask to `vcc_lo`.
    /// `src0` is a VGPR, `s0-s105`, `vcc_lo`, an inline integer, inline float bits
    /// (selectors 240..248), or a literal.
    VectorCompare {
        op: VectorCompareOp,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP1 32-bit per-lane integer operation. Sources use the VOP2 domain.
    /// Inactive lanes keep `dst`; SCC, VCC and EXEC are unchanged.
    VectorUnary {
        op: VectorUnaryOp,
        dst: u8,
        src: VectorSource,
    },
    /// VOP2 32-bit per-lane integer operation from the semantic registry.
    /// `src0` is a VGPR, `s0-s105`, `vcc_lo`, an inline integer, inline float bits
    /// (selectors 240..248), or a literal;
    /// `src1` is a VGPR. Inactive lanes keep `dst`.
    VectorAlu {
        op: VectorAluOp,
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// SOPP jump to a stream byte offset when SCC is clear.
    BranchSccZero { target: usize },
    /// SOPP jump when the Wave32 execution mask is empty.
    BranchExecZero { target: usize },
    /// SOPP jump when the Wave32 vector condition mask is empty.
    BranchVccZero { target: usize },
    /// SOPP unconditional jump to a stream byte offset.
    Branch { target: usize },
    /// VOP2 logical left shift with the count in src0.
    VectorLshlrevB32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 logical right shift with the count in src0.
    VectorLshrrevB32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 arithmetic right shift with the count in src0.
    VectorAshrrevI32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 bitwise AND.
    VectorAndB32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 bitwise OR.
    VectorOrB32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 bitwise XOR.
    VectorXorB32 {
        dst: u8,
        src0: VectorSource,
        src1: u8,
    },
    /// VOP2 single-precision addition. Execution requires an explicit FP mode.
    VectorAddF32 { dst: u8, src0: u8, src1: u8 },
    /// VOP2 single-precision multiplication. Execution requires an explicit FP mode.
    VectorMulF32 { dst: u8, src0: u8, src1: u8 },
    /// Full, uncompressed four-component graphics export.
    Export {
        target: ExportTarget,
        sources: [u8; 4],
        /// Explicit DONE flag. Pixel export sequences terminate only at DONE.
        done: bool,
    },
    /// Parameter interpolation with a source barycentric coordinate and an attribute component.
    Interpolate {
        phase: InterpolationPhase,
        dst: u8,
        coordinate: u8,
        attribute: u8,
        component: u8,
    },
    /// MIMG image_sample_lz 2D RGBA8 texture sample with level of detail zero.
    ImageSampleLz {
        /// Four contiguous destination VGPRs [dst..dst+3] receiving RGBA.
        dst: u8,
        /// Two contiguous coordinate VGPRs [coord..coord+1] containing (u, v).
        coord: u8,
        /// First SGPR of the eight-word resource descriptor s[resource..resource+7].
        resource: u8,
        /// First SGPR of the four-word sampler descriptor s[sampler..sampler+3].
        sampler: u8,
    },
    /// MIMG image_sample_lz 2D-array RGBA texture sample with level of detail zero.
    ImageSampleLzArray {
        /// Four contiguous destination VGPRs [dst..dst+3] receiving RGBA.
        dst: u8,
        /// Three contiguous coordinate VGPRs [coord..coord+2] containing float (u, v, layer).
        coord: u8,
        /// First SGPR of the eight-word resource descriptor s[resource..resource+7].
        resource: u8,
        /// First SGPR of the four-word sampler descriptor s[sampler..sampler+3].
        sampler: u8,
    },
    /// Plain DIM1 image store; coordinates and integer data retain their uint bits.
    ImageStore2D {
        /// First data source VGPR; four words for mask15, one for mask1.
        data: u8,
        /// Two contiguous uint coordinate VGPRs, x then y.
        coord: u8,
        /// Eight-word resource descriptor base SGPR.
        resource: u8,
        /// Supported component masks: 15 (RGBA) or 1 (R).
        dmask: u8,
    },
    /// Terminates the wave and waits for outstanding memory operations.
    EndProgram,
}

/// Formats an SGPR destination with LLVM's names for the EXEC and VCC halves.
struct ScalarDestination(u8);

impl fmt::Display for ScalarDestination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            106 => formatter.write_str("vcc_lo"),
            107 => formatter.write_str("vcc_hi"),
            126 => formatter.write_str("exec_lo"),
            127 => formatter.write_str("exec_hi"),
            register => write!(formatter, "s{register}"),
        }
    }
}

impl Instruction {
    /// Returns the mnemonic for the exact encoding family represented.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::BufferLoadDword { .. } => "buffer_load_dword",
            Self::BufferStoreDword { .. } => "buffer_store_dword",
            Self::WaitLoad => "s_waitcnt",
            Self::WaitStore => "s_waitcnt_vscnt",
            Self::MoveB32 { .. } => "v_mov_b32_e32",
            Self::AddNoCarryU32 { .. } => "v_add_nc_u32_e64",
            Self::VectorProduct { op, .. } => op.mnemonic(),
            Self::VectorClampedE64 { op, .. } => op.mnemonic(),
            Self::VectorBinaryE64 { op, .. } => op.assembly_name(),
            Self::VectorUnaryE64 { op, .. } => match op {
                VectorUnaryOp::NotB32 => "v_not_b32_e64",
                VectorUnaryOp::BfrevB32 => "v_bfrev_b32_e64",
                VectorUnaryOp::FfbhU32 => "v_ffbh_u32_e64",
                VectorUnaryOp::FfblB32 => "v_ffbl_b32_e64",
                VectorUnaryOp::FfbhI32 => "v_ffbh_i32_e64",
            },
            Self::VectorBitBinary { op, .. } => op.mnemonic(),
            Self::VectorMadSad { op, .. } => op.mnemonic(),
            Self::VectorTernary { op, .. } => op.mnemonic(),
            Self::ScalarMoveB32 { .. } => "s_mov_b32",
            Self::ScalarBitSetB32 { value: false, .. } => "s_bitset0_b32",
            Self::ScalarBitSetB32 { value: true, .. } => "s_bitset1_b32",
            Self::ScalarBitCompareB32 { value: false, .. } => "s_bitcmp0_b32",
            Self::ScalarBitCompareB32 { value: true, .. } => "s_bitcmp1_b32",
            Self::ScalarConditionalMoveB32 { .. } => "s_cmov_b32",
            Self::ScalarConditionalMoveImmediateI32 { .. } => "s_cmovk_i32",
            Self::ScalarUnary { op, .. } => op.mnemonic(),
            Self::ScalarAlu { op, .. } => op.mnemonic(),
            Self::ScalarCompare { op, .. } => op.mnemonic(),
            Self::VectorCompare { op, .. } => op.mnemonic(),
            Self::VectorAlu { op, .. } => op.assembly_name(),
            Self::VectorUnary { op, .. } => op.assembly_name(),
            Self::BranchSccZero { .. } => "s_cbranch_scc0",
            Self::BranchExecZero { .. } => "s_cbranch_execz",
            Self::BranchVccZero { .. } => "s_cbranch_vccz",
            Self::Branch { .. } => "s_branch",
            Self::VectorLshlrevB32 { .. } => "v_lshlrev_b32_e32",
            Self::VectorLshrrevB32 { .. } => "v_lshrrev_b32_e32",
            Self::VectorAshrrevI32 { .. } => "v_ashrrev_i32_e32",
            Self::VectorAndB32 { .. } => "v_and_b32_e32",
            Self::VectorOrB32 { .. } => "v_or_b32_e32",
            Self::VectorXorB32 { .. } => "v_xor_b32_e32",
            Self::VectorAddF32 { .. } => "v_add_f32_e32",
            Self::VectorMulF32 { .. } => "v_mul_f32_e32",
            Self::Export { .. } => "exp",
            Self::Interpolate {
                phase: InterpolationPhase::First,
                ..
            } => "v_interp_p1_f32_e32",
            Self::Interpolate {
                phase: InterpolationPhase::Second,
                ..
            } => "v_interp_p2_f32_e32",
            Self::ImageSampleLz { .. } | Self::ImageSampleLzArray { .. } => "image_sample_lz",
            Self::ImageStore2D { .. } => "image_store",
            Self::EndProgram => "s_endpgm",
        }
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.mnemonic())?;
        match *self {
            Self::BufferLoadDword {
                data,
                index,
                resource,
            }
            | Self::BufferStoreDword {
                data,
                index,
                resource,
            } => {
                let last = u16::from(resource) + 3;
                write!(
                    formatter,
                    " v{data}, v{index}, s[{resource}:{last}], 0 idxen"
                )
            }
            Self::WaitLoad => formatter.write_str(" vmcnt(0)"),
            Self::WaitStore => formatter.write_str(" null, 0"),
            Self::MoveB32 { dst, src } => write!(formatter, " v{dst}, {src}"),
            Self::AddNoCarryU32 { dst, src0, src1 }
            | Self::VectorAddF32 { dst, src0, src1 }
            | Self::VectorMulF32 { dst, src0, src1 } => {
                write!(formatter, " v{dst}, v{src0}, v{src1}")
            }
            Self::VectorLshlrevB32 { dst, src0, src1 }
            | Self::VectorLshrrevB32 { dst, src0, src1 }
            | Self::VectorAshrrevI32 { dst, src0, src1 }
            | Self::VectorAndB32 { dst, src0, src1 }
            | Self::VectorOrB32 { dst, src0, src1 }
            | Self::VectorXorB32 { dst, src0, src1 } => {
                write!(formatter, " v{dst}, {src0}, v{src1}")
            }
            Self::ScalarBitSetB32 { dst, index, .. } => {
                write!(formatter, " {}, {index}", ScalarDestination(dst))
            }
            Self::ScalarBitCompareB32 { src, index, .. } => write!(formatter, " {src}, {index}"),
            Self::ScalarConditionalMoveImmediateI32 { dst, immediate } => {
                write!(formatter, " {}, {immediate}", ScalarDestination(dst))
            }
            Self::ScalarMoveB32 { dst, src }
            | Self::ScalarConditionalMoveB32 { dst, src }
            | Self::ScalarUnary { dst, src, .. } => {
                write!(formatter, " {}, {src}", ScalarDestination(dst))
            }
            Self::ScalarAlu {
                dst, src0, src1, ..
            } => write!(formatter, " {}, {src0}, {src1}", ScalarDestination(dst)),
            Self::ScalarCompare { src0, src1, .. } => write!(formatter, " {src0}, {src1}"),
            Self::VectorCompare { src0, src1, .. } => {
                write!(formatter, " vcc_lo, {src0}, v{src1}")
            }
            Self::VectorProduct {
                dst, src0, src1, ..
            }
            | Self::VectorBinaryE64 {
                dst, src0, src1, ..
            }
            | Self::VectorBitBinary {
                dst, src0, src1, ..
            } => write!(formatter, " v{dst}, {src0}, {src1}"),
            Self::VectorTernary {
                dst,
                src0,
                src1,
                src2,
                ..
            } => write!(formatter, " v{dst}, {src0}, {src1}, {src2}"),
            Self::VectorMadSad {
                clamp,
                dst,
                src0,
                src1,
                src2,
                ..
            } => write!(
                formatter,
                " v{dst}, {src0}, {src1}, {src2}{}",
                if clamp { " clamp" } else { "" }
            ),
            Self::VectorClampedE64 {
                dst, src0, src1, ..
            } => write!(formatter, " v{dst}, {src0}, {src1} clamp"),
            Self::VectorUnary { dst, src, .. } | Self::VectorUnaryE64 { dst, src, .. } => {
                write!(formatter, " v{dst}, {src}")
            }
            Self::VectorAlu {
                dst, src0, src1, ..
            } => {
                write!(formatter, " v{dst}, {src0}, v{src1}")
            }
            Self::Branch { target }
            | Self::BranchSccZero { target }
            | Self::BranchExecZero { target }
            | Self::BranchVccZero { target } => {
                write!(formatter, " -> 0x{target:04x}")
            }
            Self::Export {
                target,
                sources: [a, b, c, d],
                done,
            } => {
                match target {
                    ExportTarget::Position0 => write!(formatter, " pos0")?,
                    ExportTarget::Parameter0 => write!(formatter, " param0")?,
                    ExportTarget::RenderTarget(index) => write!(formatter, " mrt{index}")?,
                }
                write!(formatter, ", v{a}, v{b}, v{c}, v{d}")?;
                if done {
                    write!(formatter, " done")?;
                }
                if matches!(target, ExportTarget::RenderTarget(_)) {
                    write!(formatter, " vm")?;
                }
                Ok(())
            }
            Self::Interpolate {
                dst,
                coordinate,
                attribute,
                component,
                ..
            } => {
                let channel = ["x", "y", "z", "w"]
                    .get(usize::from(component))
                    .ok_or(fmt::Error)?;
                write!(
                    formatter,
                    " v{dst}, v{coordinate}, attr{attribute}.{channel}"
                )
            }
            Self::ImageSampleLz {
                dst,
                coord,
                resource,
                sampler,
            } => {
                let last_dst = dst + 3;
                let last_coord = coord + 1;
                let last_res = resource + 7;
                let last_samp = sampler + 3;
                write!(
                    formatter,
                    " v[{dst}:{last_dst}], v[{coord}:{last_coord}], s[{resource}:{last_res}], s[{sampler}:{last_samp}] dmask:0xf dim:SQ_RSRC_IMG_2D"
                )
            }
            Self::ImageSampleLzArray {
                dst,
                coord,
                resource,
                sampler,
            } => {
                let last_dst = dst + 3;
                let last_coord = coord + 2;
                let last_res = resource + 7;
                let last_samp = sampler + 3;
                write!(
                    formatter,
                    " v[{dst}:{last_dst}], v[{coord}:{last_coord}], s[{resource}:{last_res}], s[{sampler}:{last_samp}] dmask:0xf dim:SQ_RSRC_IMG_2D_ARRAY"
                )
            }
            Self::ImageStore2D {
                data,
                coord,
                resource,
                dmask,
            } => {
                let last_data = u16::from(data) + if dmask == 15 { 3 } else { 0 };
                let last_coord = u16::from(coord) + 1;
                let last_resource = u16::from(resource) + 7;
                write!(
                    formatter,
                    " v[{data}:{last_data}], v[{coord}:{last_coord}], s[{resource}:{last_resource}] dmask:0x{dmask:x} dim:SQ_RSRC_IMG_2D unorm"
                )
            }
            Self::EndProgram => Ok(()),
        }
    }
}
