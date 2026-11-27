//! Decoder for Xenora's bounded little-endian gfx1030 instruction stream.

use std::fmt;

pub use xenora_isa::{
    Architecture, CompareRelation, ExportTarget, Instruction, InterpolationPhase, ScalarAluOp,
    ScalarCompareOp, ScalarSource, ScalarUnaryOp, VectorAluOp, VectorCompareOp, VectorSource,
};
use xenora_isa::{
    BUFFER_LOAD_DWORD, BUFFER_STORE_DWORD, EXP, IMAGE_SAMPLE_LZ, IMAGE_STORE, S_BRANCH,
    S_CBRANCH_EXECZ, S_CBRANCH_SCC0, S_CBRANCH_VCCZ, S_ENDPGM, S_MOV_B32, S_WAITCNT,
    S_WAITCNT_VSCNT, V_ADD_F32, V_ADD_NC_U32, V_AND_B32, V_ASHRREV_I32, V_CMP_EQ_I32, V_CMP_EQ_U32,
    V_CMP_GE_I32, V_CMP_GE_U32, V_CMP_GT_I32, V_CMP_GT_U32, V_CMP_LE_I32, V_CMP_LE_U32,
    V_CMP_LT_I32, V_CMP_LT_U32, V_CMP_NE_I32, V_CMP_NE_U32, V_INTERP_P1_F32, V_INTERP_P2_F32,
    V_LSHLREV_B32, V_LSHRREV_B32, V_MOV_B32, V_MUL_F32, V_OR_B32, V_XOR_B32, VCC_LO,
};

use xenora_isa::meta::EncodingFamily;
use xenora_isa::semantics::FORM_IDS;
pub use xenora_isa::semantics::VectorBinary;

/// Metadata forms that [`decode`] turns into an [`Instruction`] for at least one
/// operand combination: the hand-written forms below followed by every form of
/// the semantic registry ([`xenora_isa::semantics::FORM_IDS`]). Operand
/// restrictions are listed in docs/status.toml; every other form in
/// [`xenora_isa::meta::FORMS`] is rejected as unsupported.
pub const SEMANTIC_FORMS: [(&str, EncodingFamily); HANDWRITTEN_FORMS.len() + FORM_IDS.len()] =
    concat_forms();

const fn concat_forms<const N: usize>() -> [(&'static str, EncodingFamily); N] {
    let mut forms = [("", EncodingFamily::Sop2); N];
    let mut index = 0;
    while index < N {
        forms[index] = if index < HANDWRITTEN_FORMS.len() {
            HANDWRITTEN_FORMS[index]
        } else {
            FORM_IDS[index - HANDWRITTEN_FORMS.len()]
        };
        index += 1;
    }
    forms
}

/// Decoded forms not yet described by the semantic registry.
const HANDWRITTEN_FORMS: [(&str, EncodingFamily); 52] = [
    ("BUFFER_LOAD_DWORD", EncodingFamily::Mubuf),
    ("BUFFER_STORE_DWORD", EncodingFamily::Mubuf),
    ("EXP", EncodingFamily::Exp),
    ("IMAGE_SAMPLE_LZ", EncodingFamily::Mimg),
    ("IMAGE_STORE", EncodingFamily::Mimg),
    ("S_ADDK_I32", EncodingFamily::Sopk),
    ("S_CMPK_EQ_I32", EncodingFamily::Sopk),
    ("S_CMPK_LG_I32", EncodingFamily::Sopk),
    ("S_CMPK_GT_I32", EncodingFamily::Sopk),
    ("S_CMPK_GE_I32", EncodingFamily::Sopk),
    ("S_CMPK_LT_I32", EncodingFamily::Sopk),
    ("S_CMPK_LE_I32", EncodingFamily::Sopk),
    ("S_CMPK_EQ_U32", EncodingFamily::Sopk),
    ("S_CMPK_LG_U32", EncodingFamily::Sopk),
    ("S_CMPK_GT_U32", EncodingFamily::Sopk),
    ("S_CMPK_GE_U32", EncodingFamily::Sopk),
    ("S_CMPK_LT_U32", EncodingFamily::Sopk),
    ("S_CMPK_LE_U32", EncodingFamily::Sopk),
    ("S_MOVK_I32", EncodingFamily::Sopk),
    ("S_MULK_I32", EncodingFamily::Sopk),
    ("S_BRANCH", EncodingFamily::Sopp),
    ("S_CBRANCH_EXECZ", EncodingFamily::Sopp),
    ("S_CBRANCH_SCC0", EncodingFamily::Sopp),
    ("S_CBRANCH_VCCZ", EncodingFamily::Sopp),
    ("S_ENDPGM", EncodingFamily::Sopp),
    ("S_MOV_B32", EncodingFamily::Sop1),
    ("S_WAITCNT", EncodingFamily::Sopp),
    ("S_WAITCNT_VSCNT", EncodingFamily::Sopk),
    ("V_ADD_F32", EncodingFamily::Vop2),
    ("V_ADD_NC_U32", EncodingFamily::Vop3),
    ("V_AND_B32", EncodingFamily::Vop2),
    ("V_ASHRREV_I32", EncodingFamily::Vop2),
    ("V_CMP_EQ_I32", EncodingFamily::Vopc),
    ("V_CMP_EQ_U32", EncodingFamily::Vopc),
    ("V_CMP_GE_I32", EncodingFamily::Vopc),
    ("V_CMP_GE_U32", EncodingFamily::Vopc),
    ("V_CMP_GT_I32", EncodingFamily::Vopc),
    ("V_CMP_GT_U32", EncodingFamily::Vopc),
    ("V_CMP_LE_I32", EncodingFamily::Vopc),
    ("V_CMP_LE_U32", EncodingFamily::Vopc),
    ("V_CMP_LT_I32", EncodingFamily::Vopc),
    ("V_CMP_LT_U32", EncodingFamily::Vopc),
    ("V_CMP_NE_I32", EncodingFamily::Vopc),
    ("V_CMP_NE_U32", EncodingFamily::Vopc),
    ("V_INTERP_P1_F32", EncodingFamily::Vintrp),
    ("V_INTERP_P2_F32", EncodingFamily::Vintrp),
    ("V_LSHLREV_B32", EncodingFamily::Vop2),
    ("V_LSHRREV_B32", EncodingFamily::Vop2),
    ("V_MOV_B32", EncodingFamily::Vop1),
    ("V_MUL_F32", EncodingFamily::Vop2),
    ("V_OR_B32", EncodingFamily::Vop2),
    ("V_XOR_B32", EncodingFamily::Vop2),
];

/// A successfully decoded instruction and its location in the input stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedInstruction {
    /// Byte offset from the start of the stream.
    pub offset: usize,
    /// Total size in bytes, including any literal word.
    pub byte_len: u8,
    /// The decoded instruction and operand fields.
    pub instruction: Instruction,
}

impl DecodedInstruction {
    /// Destination byte offset of a branch, from the start of the stream.
    #[must_use]
    pub fn branch_target(&self) -> Option<usize> {
        match self.instruction {
            Instruction::Branch { target }
            | Instruction::BranchSccZero { target }
            | Instruction::BranchExecZero { target }
            | Instruction::BranchVccZero { target } => Some(target),
            _ => None,
        }
    }
}

impl fmt::Display for DecodedInstruction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04x}: {}", self.offset, self.instruction)
    }
}

/// Decoder failures classified for callers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeErrorKind {
    /// An instruction, second word or literal extends past the available input.
    Truncated {
        /// Number of bytes required at the instruction start.
        needed: u8,
        /// Number of bytes available at the instruction start.
        available: usize,
    },
    /// No supported opcode pattern matches the first word.
    UnsupportedOpcode {
        /// First instruction word in host integer order.
        word: u32,
    },
    /// A supported opcode uses an operand or modifier outside the subset.
    UnsupportedForm {
        /// Opcode mnemonic.
        mnemonic: &'static str,
        /// Details of the unsupported or reserved fields.
        detail: &'static str,
    },
    /// A forward branch leaves the stream or lands inside an instruction.
    InvalidBranchTarget {
        /// Destination byte offset from the start of the stream.
        target: usize,
    },
}

/// A decoder error and the offset of the offending instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeError {
    /// Byte offset from the start of the stream.
    pub offset: usize,
    /// Decoder-specific failure.
    pub kind: DecodeErrorKind,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "gfx1030 decode at byte 0x{:x}: ", self.offset)?;
        match self.kind {
            DecodeErrorKind::Truncated { needed, available } => {
                write!(
                    formatter,
                    "instruction requires {needed} bytes, found {available}"
                )
            }
            DecodeErrorKind::UnsupportedOpcode { word } => {
                write!(formatter, "unsupported opcode in 0x{word:08x}")
            }
            DecodeErrorKind::UnsupportedForm { mnemonic, detail } => {
                write!(formatter, "unsupported {mnemonic} form: {detail}")
            }
            DecodeErrorKind::InvalidBranchTarget { target } => {
                write!(
                    formatter,
                    "branch target 0x{target:x} is outside instruction boundaries"
                )
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes a stream of instructions. Program termination, memory hazards and
/// register initialization are checked by later stages.
///
/// # Errors
///
/// Returns the failing byte offset for a truncated instruction, unsupported
/// opcode, unsupported form or invalid branch target. An empty stream
/// decodes to an empty list.
pub fn decode(
    architecture: Architecture,
    bytes: &[u8],
) -> Result<Vec<DecodedInstruction>, DecodeError> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let decoded = decode_at(architecture, bytes, offset)?;
        offset += usize::from(decoded.byte_len);
        result.push(decoded);
    }
    for decoded in &result {
        if let Some(target) = decoded.branch_target()
            && result
                .binary_search_by_key(&target, |item| item.offset)
                .is_err()
        {
            return Err(DecodeError {
                offset: decoded.offset,
                kind: DecodeErrorKind::InvalidBranchTarget { target },
            });
        }
    }
    Ok(result)
}

/// Decodes the one instruction that starts at `offset` in `bytes`. Branch
/// targets are stream offsets, but whether they land on an instruction
/// boundary is left to the caller, which knows the rest of the stream.
///
/// # Errors
///
/// Returns `offset` with a truncated instruction, unsupported opcode or
/// unsupported form. An `offset` at or past the end is truncated.
pub fn decode_at(
    architecture: Architecture,
    bytes: &[u8],
    offset: usize,
) -> Result<DecodedInstruction, DecodeError> {
    let decode_instruction = match architecture {
        Architecture::Gfx1030 => decode_gfx1030,
    };
    let rest = bytes.get(offset..).unwrap_or_default();
    let (instruction, byte_len) =
        decode_instruction(rest, offset).map_err(|kind| DecodeError { offset, kind })?;
    Ok(DecodedInstruction {
        offset,
        byte_len,
        instruction,
    })
}

fn word(bytes: &[u8], end: u8) -> Result<u32, DecodeErrorKind> {
    let end_index = usize::from(end);
    let Some(word) = bytes
        .get(end_index - 4..end_index)
        .and_then(|word| <[u8; 4]>::try_from(word).ok())
    else {
        return Err(DecodeErrorKind::Truncated {
            needed: end,
            available: bytes.len(),
        });
    };
    Ok(u32::from_le_bytes(word))
}

fn unsupported(mnemonic: &'static str, detail: &'static str) -> DecodeErrorKind {
    DecodeErrorKind::UnsupportedForm { mnemonic, detail }
}

fn decode_gfx1030(bytes: &[u8], offset: usize) -> Result<(Instruction, u8), DecodeErrorKind> {
    let first = word(bytes, 4)?;

    if BUFFER_LOAD_DWORD.matches(first) {
        let len = BUFFER_LOAD_DWORD.byte_len();
        let second = word(bytes, len)?;
        let (data, index, resource) = buffer_operands(first, second, "buffer_load_dword")?;
        return Ok((
            Instruction::BufferLoadDword {
                data,
                index,
                resource,
            },
            len,
        ));
    }
    if BUFFER_STORE_DWORD.matches(first) {
        let len = BUFFER_STORE_DWORD.byte_len();
        let second = word(bytes, len)?;
        let (data, index, resource) = buffer_operands(first, second, "buffer_store_dword")?;
        return Ok((
            Instruction::BufferStoreDword {
                data,
                index,
                resource,
            },
            len,
        ));
    }
    if EXP.matches(first) {
        let len = EXP.byte_len();
        let sources = word(bytes, len)?.to_le_bytes();
        // RDNA 2 ISA section 13.9 (EXP). Match operand and reserved bits, including
        // the four enable bits, compression, DONE and valid-mask flags.
        let target = match first & 0x03ff_ffff {
            0x08cf => ExportTarget::Position0,
            0x020f => ExportTarget::Parameter0,
            operands if operands & !0x0870 == 0x100f => {
                ExportTarget::RenderTarget(((operands >> 4) & 7) as u8)
            }
            _ => {
                return Err(unsupported(
                    "exp",
                    "expected full uncompressed POS0 done, PARAM0, or MRT0..7 vm with explicit DONE",
                ));
            }
        };
        return Ok((
            Instruction::Export {
                target,
                sources,
                done: first & (1 << 11) != 0,
            },
            len,
        ));
    }
    if IMAGE_STORE.matches(first) {
        let len = IMAGE_STORE.byte_len();
        let second = word(bytes, len)?;
        // AMD RDNA2 Tables 41/42 and 100/101: DIM1, UNRM1, contiguous
        // uint x/y. No sampler, NSA, A16/D16, coherency or fault extensions.
        if !matches!(first & 0x0203_fffe, 0x0000_1f08 | 0x0000_1108) || second & 0xffe0_0000 != 0 {
            return Err(unsupported(
                "image_store",
                "expected DIM1 UNRM1 DMASK15/1 with zero sampler and modifier bits",
            ));
        }
        let coord = second as u8;
        let data = (second >> 8) as u8;
        let resource = (((second >> 16) & 31) * 4) as u8;
        let dmask = ((first >> 8) & 15) as u8;
        if coord > 254 || (dmask == 15 && data > 252) || resource > 98 {
            return Err(unsupported(
                "image_store",
                "store source spans exceed normal registers",
            ));
        }
        return Ok((
            Instruction::ImageStore2D {
                data,
                coord,
                resource,
                dmask,
            },
            len,
        ));
    }
    if IMAGE_SAMPLE_LZ.matches(first) {
        let len = IMAGE_SAMPLE_LZ.byte_len();
        let second = word(bytes, len)?;
        let (dst, coord, resource, sampler) =
            image_sample_operands(first, second, "image_sample_lz")?;
        return Ok((
            if (first >> 3) & 7 == 5 {
                Instruction::ImageSampleLzArray {
                    dst,
                    coord,
                    resource,
                    sampler,
                }
            } else {
                Instruction::ImageSampleLz {
                    dst,
                    coord,
                    resource,
                    sampler,
                }
            },
            len,
        ));
    }
    for (opcode, phase) in [
        (V_INTERP_P1_F32, InterpolationPhase::First),
        (V_INTERP_P2_F32, InterpolationPhase::Second),
    ] {
        if opcode.matches(first) {
            return Ok((
                Instruction::Interpolate {
                    phase,
                    dst: (first >> 18) as u8,
                    coordinate: first as u8,
                    attribute: ((first >> 10) & 0x3f) as u8,
                    component: ((first >> 8) & 3) as u8,
                },
                opcode.byte_len(),
            ));
        }
    }
    if V_MOV_B32.matches(first) {
        return move_b32(first, bytes);
    }
    {
        use CompareRelation::{Equal, Greater, GreaterOrEqual, Less, LessOrEqual, NotEqual};
        for (opcode, relation, signed) in [
            (V_CMP_LT_I32, Less, true),
            (V_CMP_EQ_I32, Equal, true),
            (V_CMP_LE_I32, LessOrEqual, true),
            (V_CMP_GT_I32, Greater, true),
            (V_CMP_NE_I32, NotEqual, true),
            (V_CMP_GE_I32, GreaterOrEqual, true),
            (V_CMP_LT_U32, Less, false),
            (V_CMP_EQ_U32, Equal, false),
            (V_CMP_LE_U32, LessOrEqual, false),
            (V_CMP_GT_U32, Greater, false),
            (V_CMP_NE_U32, NotEqual, false),
            (V_CMP_GE_U32, GreaterOrEqual, false),
        ] {
            if opcode.matches(first) {
                return vector_compare(VectorCompareOp { relation, signed }, first, bytes);
            }
        }
    }
    if V_ADD_NC_U32.matches(first) {
        let len = V_ADD_NC_U32.byte_len();
        let second = word(bytes, len)?;
        return Ok((add_u32(first, second)?, len));
    }
    // Pinned AMD metadata: SOPK ENCODING[31:28] and OP[27:23] form
    // mask 0xff800000. Match complete opcodes so other scalar families and
    // S_WAITCNT_VSCNT retain their own decoding and operand checks.
    let cmpk_opcode = (first >> 23) & 31;
    if first >> 28 == 0xb && (3..=14).contains(&cmpk_opcode) {
        use ScalarCompareOp::*;
        let op = [
            CmpEqI32, CmpLgI32, CmpGtI32, CmpGeI32, CmpLtI32, CmpLeI32, CmpEqU32, CmpLgU32,
            CmpGtU32, CmpGeU32, CmpLtU32, CmpLeU32,
        ][(cmpk_opcode - 3) as usize];
        // SDST is a read-only source for CMPK, not a destination.
        let reg = ((first >> 16) & 127) as u8;
        if reg > 105 {
            return Err(unsupported(
                op.mnemonic(),
                "expected a normal SGPR comparison source",
            ));
        }
        let immediate = if cmpk_opcode < 9 {
            first as i16 as i32 as u32
        } else {
            first & 0xffff
        };
        return Ok((
            Instruction::ScalarCompare {
                op,
                src0: ScalarSource::Register(reg),
                src1: ScalarSource::Immediate(immediate),
            },
            4,
        ));
    }
    let sopk = first & 0xff80_0000;
    if matches!(sopk, 0xb000_0000 | 0xb780_0000 | 0xb800_0000) {
        let mnemonic = match sopk {
            0xb000_0000 => "s_movk_i32",
            0xb780_0000 => "s_addk_i32",
            _ => "s_mulk_i32",
        };
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 105 {
            return Err(unsupported(mnemonic, "expected a normal SGPR destination"));
        }
        let src = ScalarSource::Immediate(first as i16 as i32 as u32);
        // RDNA2 section 12.2: ADDK/MULK read the old destination. Existing
        // AddI32 sets signed-overflow SCC; move and MulI32 preserve SCC.
        let instruction = if sopk == 0xb000_0000 {
            Instruction::ScalarMoveB32 { dst, src }
        } else {
            Instruction::ScalarAlu {
                op: if sopk == 0xb780_0000 {
                    ScalarAluOp::AddI32
                } else {
                    ScalarAluOp::MulI32
                },
                dst,
                src0: ScalarSource::Register(dst),
                src1: src,
            }
        };
        return Ok((instruction, 4));
    }
    if S_MOV_B32.matches(first) {
        let dst = ((first >> 16) & 0x7f) as u8;
        if !matches!(dst, 0..=107 | 126..=127) {
            return Err(unsupported(
                "s_mov_b32",
                "expected a normal SGPR or EXEC/VCC destination",
            ));
        }
        let (src, len) = scalar_source(first as u8, bytes, "s_mov_b32")?;
        return Ok((Instruction::ScalarMoveB32 { dst, src }, len));
    }
    if let Some(op) = ScalarUnaryOp::from_word(first) {
        // SOP1: SSRC0 in bits 0-7, SDST in bits 16-22.
        let mnemonic = op.mnemonic();
        let dst = ((first >> 16) & 0x7f) as u8;
        if dst > 105 {
            return Err(unsupported(mnemonic, "expected a normal SGPR destination"));
        }
        let (src, len) = scalar_source(first as u8, bytes, mnemonic)?;
        return Ok((Instruction::ScalarUnary { op, dst, src }, len));
    }
    if let Some(op) = ScalarCompareOp::from_word(first) {
        let mnemonic = op.mnemonic();
        let src0_bits = first as u8;
        let src1_bits = (first >> 8) as u8;
        if src0_bits == 255 && src1_bits == 255 {
            return Err(unsupported(
                mnemonic,
                "only one source may select a literal",
            ));
        }
        let (src0, len0) = scalar_source(src0_bits, bytes, mnemonic)?;
        let (src1, len1) = scalar_source(src1_bits, bytes, mnemonic)?;
        return Ok((
            Instruction::ScalarCompare { op, src0, src1 },
            len0.max(len1),
        ));
    }
    if S_BRANCH.matches(first) {
        let target = branch_target(first, offset, "s_branch")?;
        return Ok((Instruction::Branch { target }, S_BRANCH.byte_len()));
    }
    if let Some(op) = ScalarAluOp::from_word(first) {
        return scalar_alu(op, first, bytes);
    }
    if S_CBRANCH_SCC0.matches(first) {
        let target = branch_target(first, offset, "s_cbranch_scc0")?;
        return Ok((
            Instruction::BranchSccZero { target },
            S_CBRANCH_SCC0.byte_len(),
        ));
    }
    if S_CBRANCH_EXECZ.matches(first) {
        let target = branch_target(first, offset, "s_cbranch_execz")?;
        return Ok((
            Instruction::BranchExecZero { target },
            S_CBRANCH_EXECZ.byte_len(),
        ));
    }
    if S_CBRANCH_VCCZ.matches(first) {
        let target = branch_target(first, offset, "s_cbranch_vccz")?;
        return Ok((
            Instruction::BranchVccZero { target },
            S_CBRANCH_VCCZ.byte_len(),
        ));
    }
    if let Some(op) = VectorAluOp::from_word(first) {
        // VOP2: SRC0 in bits 0-8, VSRC1 in bits 9-16, VDST in bits 17-24.
        let dst = ((first >> 17) & 0xff) as u8;
        let src1 = ((first >> 9) & 0xff) as u8;
        let (src0, len) = vector_src0(first, bytes, op.assembly_name())?;
        return Ok((
            Instruction::VectorAlu {
                op,
                dst,
                src0,
                src1,
            },
            len,
        ));
    }
    if V_LSHLREV_B32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_lshlrev_b32_e32")?;
        return Ok((
            Instruction::VectorLshlrevB32 { dst, src0, src1 },
            V_LSHLREV_B32.byte_len(),
        ));
    }
    if V_ASHRREV_I32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_ashrrev_i32_e32")?;
        return Ok((
            Instruction::VectorAshrrevI32 { dst, src0, src1 },
            V_ASHRREV_I32.byte_len(),
        ));
    }
    if V_LSHRREV_B32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_lshrrev_b32_e32")?;
        return Ok((
            Instruction::VectorLshrrevB32 { dst, src0, src1 },
            V_LSHRREV_B32.byte_len(),
        ));
    }
    if V_AND_B32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_and_b32_e32")?;
        return Ok((
            Instruction::VectorAndB32 { dst, src0, src1 },
            V_AND_B32.byte_len(),
        ));
    }
    if V_OR_B32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_or_b32_e32")?;
        return Ok((
            Instruction::VectorOrB32 { dst, src0, src1 },
            V_OR_B32.byte_len(),
        ));
    }
    if V_XOR_B32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_xor_b32_e32")?;
        return Ok((
            Instruction::VectorXorB32 { dst, src0, src1 },
            V_XOR_B32.byte_len(),
        ));
    }
    if V_ADD_F32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_add_f32_e32")?;
        return Ok((
            Instruction::VectorAddF32 { dst, src0, src1 },
            V_ADD_F32.byte_len(),
        ));
    }
    if V_MUL_F32.matches(first) {
        let (dst, src0, src1) = vop2_registers(first, "v_mul_f32_e32")?;
        return Ok((
            Instruction::VectorMulF32 { dst, src0, src1 },
            V_MUL_F32.byte_len(),
        ));
    }
    if S_WAITCNT.matches(first) {
        return scalar_form(
            first,
            S_WAITCNT.byte_len(),
            0xbf8c_3f70,
            Instruction::WaitLoad,
            "expected vmcnt(0) alone",
        );
    }
    if S_WAITCNT_VSCNT.matches(first) {
        return scalar_form(
            first,
            S_WAITCNT_VSCNT.byte_len(),
            0xbbfd_0000,
            Instruction::WaitStore,
            "expected null, 0",
        );
    }
    if S_ENDPGM.matches(first) {
        return scalar_form(
            first,
            S_ENDPGM.byte_len(),
            0xbf81_0000,
            Instruction::EndProgram,
            "expected zero immediate",
        );
    }

    Err(DecodeErrorKind::UnsupportedOpcode { word: first })
}

/// SOP2: SSRC0 in bits 0-7, SSRC1 in bits 8-15, SDST in bits 16-22.
/// Other SOP2 opcodes stay unsupported.
fn scalar_alu(
    op: ScalarAluOp,
    first: u32,
    bytes: &[u8],
) -> Result<(Instruction, u8), DecodeErrorKind> {
    let mnemonic = op.mnemonic();
    let dst = ((first >> 16) & 0x7f) as u8;
    if dst > 105 {
        return Err(unsupported(mnemonic, "expected a normal SGPR destination"));
    }
    let src0_bits = first as u8;
    let src1_bits = (first >> 8) as u8;
    if src0_bits == 255 && src1_bits == 255 {
        return Err(unsupported(
            mnemonic,
            "only one source may select a literal",
        ));
    }
    let (src0, len0) = scalar_source(src0_bits, bytes, mnemonic)?;
    let (src1, len1) = scalar_source(src1_bits, bytes, mnemonic)?;
    Ok((
        Instruction::ScalarAlu {
            op,
            dst,
            src0,
            src1,
        },
        len0.max(len1),
    ))
}

fn scalar_source(
    bits: u8,
    bytes: &[u8],
    mnemonic: &'static str,
) -> Result<(ScalarSource, u8), DecodeErrorKind> {
    let value = u32::from(bits);
    match bits {
        0..=105 => Ok((ScalarSource::Register(bits), 4)),
        128..=192 => Ok((ScalarSource::Immediate(value - 128), 4)),
        193..=208 => Ok((ScalarSource::Immediate(192u32.wrapping_sub(value)), 4)),
        255 => Ok((ScalarSource::Immediate(word(bytes, 8)?), 8)),
        _ => Err(unsupported(
            mnemonic,
            "expected a normal SGPR, inline integer or literal",
        )),
    }
}

fn branch_target(
    first: u32,
    offset: usize,
    mnemonic: &'static str,
) -> Result<usize, DecodeErrorKind> {
    let displacement = isize::from(first as u16 as i16);
    // SOPP displacement is signed and relative to the next four-byte instruction.
    // Keep the final offset checked in both directions; stream-boundary validation
    // is performed separately after all instruction lengths are known.
    offset
        .checked_add_signed(4 + displacement * 4)
        .ok_or_else(|| unsupported(mnemonic, "target byte offset overflowed"))
}

fn vop2_registers(first: u32, mnemonic: &'static str) -> Result<(u8, u8, u8), DecodeErrorKind> {
    if first & 0x100 == 0 {
        return Err(unsupported(
            mnemonic,
            "expected two unmodified VGPR sources",
        ));
    }
    Ok(((first >> 17) as u8, first as u8, (first >> 9) as u8))
}

fn image_sample_operands(
    first: u32,
    second: u32,
    mnemonic: &'static str,
) -> Result<(u8, u8, u8, u8), DecodeErrorKind> {
    // In word 0:
    // Bits 1..2: NSA (must be 0 for contiguous VADDR)
    // Bits 3..5: DIM (1 = 2D, 5 = 2D-array; RDNA2 Table 41)
    // Bit 7: DLC (0)
    // Bits 8..11: DMASK (0xf for RGBA 4-component return)
    // Bit 12: UNORM (0)
    // Bit 13: GLC (0)
    // Bit 14: DA (0)
    // Bit 15: R128 (0)
    // Bit 16: TFE (0)
    // Bit 25: SLC (0)
    // Bit 0 & Bits 18..24: OP (0x27 / 39)
    // Bits 26..31: ENCODING (0x3c)
    // Mask off OP and ENCODING (bits 0, 18..24, 26..31).
    // The expected variable bits for 2D RGBA8 sampling:
    // dim = 1 (bits 3..5 = 1 << 3 = 0x8), dmask = 0xf (bits 8..11 = 0xf << 8 = 0xf00).
    // All other modifier flags (NSA, DLC, UNORM, GLC, DA, R128, TFE, SLC) must be zero.
    const MODIFIER_MASK: u32 = 0x0203_fffe;
    const EXPECTED_2D_RGBA: u32 = 0x0000_0f08;

    const EXPECTED_ARRAY_RGBA: u32 = 0x0000_0f28;
    if !matches!(
        first & MODIFIER_MASK,
        EXPECTED_2D_RGBA | EXPECTED_ARRAY_RGBA
    ) {
        return Err(unsupported(
            mnemonic,
            "expected DIM 1 or 5 and dmask:0xf with zero modifier flags",
        ));
    }

    // In word 1:
    // Bits 0..7: VADDR (u8)
    // Bits 8..15: VDATA (u8)
    // Bits 16..20: SRSRC >> 2 (5 bits)
    // Bits 21..25: SSAMP >> 2 (5 bits)
    // Bits 26..29: reserved (must be 0)
    // Bit 30: A16 (0)
    // Bit 31: D16 (0)
    if second & 0xfc00_0000 != 0 {
        return Err(unsupported(
            mnemonic,
            "expected 32-bit float coordinates and data without A16/D16",
        ));
    }

    let coord = (second & 0xff) as u8;
    let dst = ((second >> 8) & 0xff) as u8;
    let resource = (((second >> 16) & 0x1f) * 4) as u8;
    let sampler = (((second >> 21) & 0x1f) * 4) as u8;

    if (first >> 3) & 7 == 5 && (coord > 253 || dst > 252) {
        return Err(unsupported(
            mnemonic,
            "array coordinate/data spans exceed v255",
        ));
    }

    // RDNA 2 resource descriptor (T#) requires 8 SGPRs; sampler (S#) requires 4 SGPRs.
    // SGPR range is 0..=105 for normal allocatable registers.
    if resource > 98 {
        return Err(unsupported(
            mnemonic,
            "resource descriptor must fit in normal SGPRs s[0:105]",
        ));
    }
    if sampler > 102 {
        return Err(unsupported(
            mnemonic,
            "sampler descriptor must fit in normal SGPRs s[0:105]",
        ));
    }

    Ok((dst, coord, resource, sampler))
}

fn buffer_operands(
    first: u32,
    second: u32,
    mnemonic: &'static str,
) -> Result<(u8, u8, u8), DecodeErrorKind> {
    // OFFSET, OFFEN, IDXEN, GLC, DLC, LDS and the reserved bit before OP.
    const ADDRESS_MODE_MASK: u32 = 0x0003_ffff;
    const IDXEN: u32 = 1 << 13;

    // SOFFSET is inline zero. TFE, SLC and the reserved high operand bit are zero.
    if first & ADDRESS_MODE_MASK != IDXEN || second & 0xffe0_0000 != 0x8000_0000 {
        return Err(unsupported(
            mnemonic,
            "expected idxen alone with zero offsets",
        ));
    }

    let index = second as u8;
    let data = (second >> 8) as u8;
    let resource = ((second >> 16) & 0x1f) as u8 * 4;
    // AMD RDNA 2 ISA §3.6.2. A resource descriptor requires four normal SGPRs.
    if resource > 102 {
        return Err(unsupported(
            mnemonic,
            "resource descriptor must fit in normal SGPRs",
        ));
    }
    Ok((data, index, resource))
}

/// VOPC: SRC0 in bits 0-8, VSRC1 in bits 9-16 (RDNA 2 ISA section 12.9). The
/// destination is implicitly `vcc_lo`. SRC0 excludes inline float constants,
/// `vcc_hi`, EXEC, M0, TTMPs and the condition selectors (SDWA 249, DPP 250/233).
fn vector_compare(
    op: VectorCompareOp,
    first: u32,
    bytes: &[u8],
) -> Result<(Instruction, u8), DecodeErrorKind> {
    let src1 = ((first >> 9) & 0xff) as u8;
    let (src0, len) = vector_src0(first, bytes, op.mnemonic())?;
    Ok((Instruction::VectorCompare { op, src0, src1 }, len))
}

/// The 9-bit SRC0 of a VOPC or VOP2 word: a VGPR, `s0-s105`, `vcc_lo`, an
/// inline integer -16..64 or a literal word; everything else is unsupported.
fn vector_src0(
    first: u32,
    bytes: &[u8],
    mnemonic: &'static str,
) -> Result<(VectorSource, u8), DecodeErrorKind> {
    let source = first & 0x1ff;
    let base_len = 4;
    Ok(match source {
        0..=106 => (VectorSource::Scalar(source as u8), base_len),
        128..=192 => (VectorSource::Immediate(source - 128), base_len),
        193..=208 => (
            VectorSource::Immediate(192u32.wrapping_sub(source)),
            base_len,
        ),
        255 => {
            let len = base_len + 4;
            (VectorSource::Immediate(word(bytes, len)?), len)
        }
        256..=511 => (VectorSource::Register(source as u8), base_len),
        _ => {
            return Err(unsupported(
                mnemonic,
                "expected a VGPR, normal SGPR, vcc_lo, inline integer or literal",
            ));
        }
    })
}

fn move_b32(first: u32, bytes: &[u8]) -> Result<(Instruction, u8), DecodeErrorKind> {
    let dst = (first >> 17) as u8;
    let source = first & 0x1ff;
    let base_len = V_MOV_B32.byte_len();
    let (src, len) = match source {
        0..=105 => (VectorSource::Scalar(source as u8), base_len),
        // vcc_lo, which may hold a lane-derived compare mask (docs/shaders/vector-compare.md).
        106 => (VectorSource::Scalar(VCC_LO), base_len),
        128..=192 => (VectorSource::Immediate(source - 128), base_len),
        193..=208 => (
            VectorSource::Immediate(192u32.wrapping_sub(source)),
            base_len,
        ),
        // AMD RDNA 2 ISA Table 77: VOP1 source selector 242 supplies +1.0f.
        242 => (VectorSource::Immediate(0x3f80_0000), base_len),
        255 => {
            let len = base_len + 4;
            (VectorSource::Immediate(word(bytes, len)?), len)
        }
        256..=511 => (VectorSource::Register(source as u8), base_len),
        _ => {
            return Err(unsupported(
                "v_mov_b32_e32",
                "expected a VGPR, normal SGPR, vcc_lo, inline integer, inline +1.0f or literal",
            ));
        }
    };
    Ok((Instruction::MoveB32 { dst, src }, len))
}

fn add_u32(first: u32, second: u32) -> Result<Instruction, DecodeErrorKind> {
    let src0 = second & 0x1ff;
    let src1 = (second >> 9) & 0x1ff;
    // All output and input modifiers are clear. The unused third source uses
    // the gfx1030 assembler's inline-zero operand encoding.
    if first & 0xff00 != 0 || second >> 18 != 128 || src0 < 256 || src1 < 256 {
        return Err(unsupported(
            "v_add_nc_u32_e64",
            "expected two VGPR sources and no modifiers",
        ));
    }
    Ok(Instruction::AddNoCarryU32 {
        dst: first as u8,
        src0: src0 as u8,
        src1: src1 as u8,
    })
}

fn scalar_form(
    word: u32,
    byte_len: u8,
    expected: u32,
    instruction: Instruction,
    detail: &'static str,
) -> Result<(Instruction, u8), DecodeErrorKind> {
    if word == expected {
        Ok((instruction, byte_len))
    } else {
        Err(unsupported(instruction.mnemonic(), detail))
    }
}
