# Full-width vector products, bitfields and three-source integer operations

The bounded gfx1030 VOP3 subset contains the six product/ternary forms below
and the six bitfield/count extension forms described later. All twelve use
handwritten instruction identities; the semantic registry is unchanged.
All results are 32-bit words.

| Instruction | OP | Result for inputs a, b, c |
| --- | ---: | --- |
| `V_MUL_LO_U32` | 361 | Low 32 bits of unsigned a × b |
| `V_MUL_HI_U32` | 362 | High 32 bits of unsigned a × b |
| `V_MUL_HI_I32` | 364 | High 32 bits of signed 32 × 32 product |
| `V_BFI_B32` | 330 | (a & b) OR (~a & c), with a as the selection mask |
| `V_ADD3_U32` | 877 | a + b + c modulo 2^32 |
| `V_AND_OR_B32` | 881 | (a & b) OR c |

Products use all input bits, without 24-bit truncation. MSL uses unsigned
multiplication/addition for wrapping results and signed or unsigned `mulhi`
for high products. The signed result is reinterpreted as a uint word.

## Encoding and sources

First word: `0xd4000000 | OP << 16 | VDST`; second word:
`SRC0 | SRC1 << 9 | SRC2 << 18`. VDST is eight bits and each source is nine
bits. First-word bits 8–15 and second-word bits 27–31 must be zero: no ABS,
OP_SEL, CLAMP, OMOD or NEG modifiers. Products, BFM and BCNT have two active
operands and require unused SRC2=128 (inline zero). This is a canonical subset restriction:
LLVM also disassembles other unused values, so rejection does not establish
hardware invalidity. The existing `V_ADD_NC_U32` e64 contract is unchanged.

Every active position admits selectors 0–105 (normal SGPR), 106 (`vcc_lo`),
128–192 (0 through 64), 193–208 (−1 through −16), 240–248 (raw floating-named
inline words), 255 (literal), and 256–511 (VGPR). Other selectors, special
registers, DPP/SDWA and LDS-direct remain excluded. Inline words are bit
patterns, not numeric float conversions.

At most two distinct scalar-bus identities are allowed. Each distinct SGPR
or `vcc_lo` consumes one slot; all literal positions share one literal key.
VGPRs and inline constants consume none. Validation uses raw selectors before
converting immediates: a forced literal equal to an inline constant still
consumes a slot. Repeated registers count once. Three different SGPRs, or two
SGPRs plus a literal, reject. The instruction occupies eight bytes, or twelve
when any active source selects the single shared trailing literal word.

## Execution and graphics limits

Compute captures all sources before writing the destination, writes active
EXEC lanes, and retains initialized inactive destination words. Empty EXEC
skips value reads after full-stream decoding. SCC, VCC and EXEC remain
unchanged, including undefined flags. Initialization, pending-load and
reserved-destination checks still apply. These forms do not extend runtime
VCC branch regions or introduce new divergence.

Graphics reads scalar uniforms s24–s31 and initialized permitted locals
s32–s105; VCC and descriptor operands remain excluded by its ABI. Uniform
reads in every active position contribute to the required mask (s31 is bit
128). Pixel destinations v0/v1 remain reserved. Inputs are captured before
alias writes; uint bits travel through float VGPR carriers by explicit
bitcasts. Branch joins and bounded-loop writes retain integer provenance.
Integer results require compatible typed outputs and cannot feed float
arithmetic, float exports, floating texture coordinates, float storage data,
or vertex POS/PARAM exports. Uniform array-layer carriers and pixel-coordinate
metadata cannot be consumed as integer operands, including SRC2.

## Bitfield and count extension

Six further operations share the source contract above. BFM and BCNT have two
active inputs and canonical unused SRC2=128; the other four have three.

| Instruction | OP | Word result |
| --- | ---: | --- |
| `V_BFE_U32` | 328 | Extract width c&31 bits from a starting at b&31, zero-fill |
| `V_BFM_B32` | 867 | Mask of a&31 low bits, shifted left by b&31 |
| `V_ALIGNBIT_B32` | 334 | Low word of concatenated a:high,b:low shifted right by c&31 |
| `V_BCNT_U32_B32` | 868 | Population count of a plus b, wrapping |
| `V_OR3_B32` | 882 | a OR b OR c |
| `V_XOR3_B32` | 376 | a XOR b XOR c |

A width of 32 masks to zero, and ALIGNBIT count 0 returns b. The lowering avoids
shifts by 32; extraction crossing the high end zero-fills. These operations are
lane-local and preserve flags. Their output provenance and every active source
position have the same graphics restrictions as the preceding six forms.
LLVM accepts an ALIGNBIT clamp modifier; the current no-modifiers contract is
a deliberate subset restriction, and broader modifier coverage remains S3.1 debt.

## Signed extraction and shift arithmetic

Four additional three-source operations retain the same checked source and
integer graphics contract:

| Instruction | OP | Result |
| --- | ---: | --- |
| `V_BFE_I32` | 329 | Signed extraction at b&31 with width c&31 |
| `V_LSHL_ADD_U32` | 838 | (a shifted left by b&31) + c |
| `V_ADD_LSHL_U32` | 839 | (a + b) shifted left by c&31 |
| `V_LSHL_OR_B32` | 879 | (a shifted left by b&31) OR c |

Signed extraction returns zero for zero masked width. Otherwise it extracts
min(width,32-offset) available bits and sign-extends that field. A crossing
field therefore extends the original high bit; a narrower field extends its own
highest selected bit. Generated shifts never use count32. Shift/add operations
use uint arithmetic and wrap to32 bits; no flags are changed.

This interpretation follows the public signed-bitfield compiler transformations
in [LLVM AMDGPU lowering](https://llvm.googlesource.com/llvm-project/+/7aef15d44bf13fad3ae95546fee9b2ac15da8aa3/llvm/lib/Target/AMDGPU/AMDGPUISelLowering.cpp)
alongside the AMD ISA guide. ALIGNBYTE remains unsupported pending resolution
of its large-count behavior; it is not treated as completed work.

## Three-input integer ordering

MIN3_I32/U32, MAX3_I32/U32 and MED3_I32/U32 have opcodes338/339,341/342 and
344/345 respectively. They select the least, greatest or middle input word under
signed two's-complement or unsigned ordering. Median chooses an input word;
it does not average values. Equal inputs retain that repeated word, including
all-equal triples. Signed lowering reinterprets words as int for comparisons and
returns raw uint bits. No subtraction-based ordering or saturation is used.

The same three-source admission, literal, alias, mask, flag and graphics
provenance rules apply. Adjacent floating-point forms remain excluded. Synthetic
native tests cover order permutations, ties, sign boundaries and NaN-shaped raw
words. Both independent reviewers accepted this increment after separate
full and focused validation recorded in the local checkpoint.

## Multiply-add and packed absolute differences

Six three-source forms accept both wrapping and integer-clamp modes (bit 15):

| Instruction | OP | Full result before wrapping or saturation |
| --- | ---: | --- |
| `V_MAD_I32_I24` | 322 | signed24(a) × signed24(b) + signed32(c) |
| `V_MAD_U32_U24` | 323 | unsigned24(a) × unsigned24(b) + unsigned32(c) |
| `V_SAD_U8` | 346 | c + sum of four unsigned byte absolute differences |
| `V_SAD_HI_U8` | 347 | c + (sum of four unsigned byte differences shifted left 16) |
| `V_SAD_U16` | 348 | c + sum of two unsigned halfword absolute differences |
| `V_SAD_U32` | 349 | c + unsigned32 absolute difference |

Wrapping returns the low 32 bits. Clamp saturates the final signed MAD result
into the signed 32-bit range and every other result into the unsigned 32-bit
range. MAD retains the full product before adding the accumulator: a product
outside the destination range can return into range through cancellation.
Only MAD multiplicands discard their high eight input bits. Every accumulator
uses all 32 bits, including the low half for SAD_HI. Saturation is spec-derived
from the public integer clamp rule and each operation's result type.

The three-source admission, scalar-bus, source capture, mask, flag and graphics
provenance rules above apply. These six forms permit bit 15; their other
modifiers remain rejected. They do not extend runtime VCC branch regions.
The fixture is `fixtures/rdna2/vop3-mad-sad.s`. Both independent reviewers accepted
this extension; the local checkpoint records validation counts and limits.

## Evidence and acceptance

Semantics and fields follow the public [AMD RDNA2 ISA, sections 6.2.1, 12.12
and 13.3.4](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content).
Native ARM64 LLVM 23.1.6 gfx1030 assembly/disassembly probes recorded in
`artifacts/s31-vop3-source-evidence.md` establish encoding and source-limit
observations, not arithmetic execution correctness.

The original fixture is `fixtures/rdna2/vop3-integer.s`. Decoder source/field
checks and the LLVM matrix accompany independent references, compute tests,
`crates/shader/tests/graphics_vop3_integer.rs` and native graphics tests in
`crates/metal/tests/native/vop3_integer_graphics.rs`. Both independent reviewers
accepted the original six-form increment after CPU, LLVM, native Metal and
full repository validation. See the
[local checkpoint](../testing/local-graphics-checkpoint.md) for counts and limits.
The extension uses `fixtures/rdna2/vop3-bitfield.s` and the separate encoding
record `artifacts/s31-bitfield-source-evidence.md`. Both independent reviewers accepted that extension with the evidence recorded
in the local checkpoint. Both reviewers also accepted the four arithmetic forms above after separate
validation recorded in that checkpoint. None of these increments completes S3.1 or establishes Xbox compatibility.


## XOR-add, packed byte average and masked byte differences

Three further bounded, unmodified VOP3 forms use three active 32-bit sources:

| Instruction | OP | Result |
| --- | ---: | --- |
| `V_XAD_U32` | 837 | ((a XOR b) + c) modulo 2^32 |
| `V_LERP_U8` | 333 | Four independent unsigned byte averages, rounded per byte by c |
| `V_MSAD_U8` | 369 | c plus unsigned byte absolute differences, excluding bytes whose b component is zero, modulo 2^32 |

For byte position j=0..3, LERP produces
`(byte(a,j) + byte(b,j) + ((c >> (8*j)) & 1)) >> 1` in that byte.
Only c bits0/8/16/24 affect rounding. Other c bits are ignored, and addition
uses enough width to retain a byte sum of511 before shifting. No carry passes
between output bytes. This is a packed integer average, not floating interpolation.

MSAD tests the reference byte in SRC1, not SRC0, for exclusion. Its four
unsigned differences accumulate into the full32-bit SRC2; carries cross byte
boundaries and final overflow wraps. XAD has no carry input or flag output.
All three preserve SCC, VCC and EXEC. No clamp, ABS, NEG, OMOD or OP_SEL mode
is admitted by this increment.

Sources admit normal SGPRs, the checked VCC/EXEC halves, integer inline values,
the nine raw floating-named inline words, a shared literal word and VGPRs.
Mask-half reads still require initialized values under the selected wave contract;
this does not define Wave32 high halves after comparison or at launch.
The existing two-distinct-scalar-identity rule applies before immediate
collapse. Every literal position shares the same trailing word; encodings are
8 or12 bytes. LDS-direct, DPP/SDWA and other special selectors remain excluded.

Compute retains source snapshots, active-lane writes, initialized inactive
values and empty-EXEC read suppression. These forms do not expand runtime VCC
branch regions. Graphics retains the integer provenance, descriptor, uniform,
join/loop and typed-output restrictions described above; it does not admit
mask-register sources or convert raw integer words into floating values.

Semantics follow [AMD RDNA2 ISA §12.12](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture).
The [arithmetic checkpoint](../testing/local-graphics-checkpoint.md#stage-31-increment-xor-add-byte-average-and-masked-differences)
records independent review, CPU/LLVM checks, 473 native compute dispatches and
52 new typed integer graphics renders on Apple M4 Max. This evidence covers the
bounded contract above, not every operand mode or Xbox execution.


## Low-halfword multiply-add

Two bounded VOP3 forms multiply the low16 bits of SRC0 and SRC1, then add the
full32-bit SRC2 and retain the low32 result bits:

| Instruction | OP | Result modulo 2^32 |
| --- | ---: | --- |
| `V_MAD_U32_U16` | 883 | unsigned16(a) × unsigned16(b) + unsigned32(c) |
| `V_MAD_I32_I16` | 885 | signed16(a) × signed16(b) + signed32(c) |

The signed form sign-extends each low halfword before multiplication; the
unsigned form zero-extends it. Upper multiplicand bits do not affect the result.
SRC2 and the output remain full32-bit words. Overflow wraps without saturation,
and SCC, VCC and EXEC are preserved. All modifiers are rejected, including
OP_SEL, CLAMP, ABS, NEG and OMOD; high-half selection is not admitted.

The checked three-source domain uses position-specific inline payloads:

| Selector | SRC0/SRC1 half bits | SRC2 single bits |
| --- | --- | --- |
| 240 | `0x3800` | `0x3f000000` |
| 241 | `0xb800` | `0xbf000000` |
| 242 | `0x3c00` | `0x3f800000` |
| 243 | `0xbc00` | `0xbf800000` |
| 244 | `0x4000` | `0x40000000` |
| 245 | `0xc000` | `0xc0000000` |
| 246 | `0x4400` | `0x40800000` |
| 247 | `0xc400` | `0xc0800000` |
| 248 | `0x3118` | `0x3e22f983` |

AMD Table20 defines the width-dependent inline constants, including the exact
half and single encodings of1/(2*pi). These bits are consumed as integers;
the signed MAD sign-extends the halfword, without any floating arithmetic.
Inline constants do not consume scalar-bus identities. Literal restrictions remain:

- If SRC0 or SRC1 selects the literal, its raw payload must be at most0xffff.
  This is a conservative implementation restriction, not a claim of ISA
  invalidity. A literal used only in SRC2 may contain any32-bit word. If SRC2
  shares that literal with either multiplicand, the0xffff restriction still applies.

There is at most one shared trailing literal word and at most two distinct
scalar-bus identities, checked before immediates are collapsed. Encodings are
8 bytes without a literal and12 with one. Normal SGPRs, checked mask halves,
integer inline values and VGPRs otherwise retain their existing source rules;
LDS-direct, DPP/SDWA and other unsupported special selectors remain excluded.

Compute captures operands before alias writes, preserves initialized inactive
destinations and suppresses value reads under empty EXEC after decoding. Pending
register and initialization checks remain, as do runtime VCC-region exclusions.
Graphics retains its uniform/descriptor restrictions, integer provenance,
branch/loop handling and compatible typed-output requirements; it does not
admit mask-register sources or integer values escaping into floating outputs.

The [MAD16 checkpoint](../testing/local-graphics-checkpoint.md#stage-31-increment-mixed-width-multiply-add)
records independent review, 259 native compute dispatches and 48 typed integer
graphics renders on Apple M4 Max. These results cover only this bounded subset.

The subsequent [inline-width checkpoint](../testing/local-graphics-checkpoint.md#stage-31-increment-width-dependent-mad16-inline-constants)
adds72 native compute and72 typed graphics cases for the position-specific
constants above. Modifier and broad literal support remain separate gaps.
