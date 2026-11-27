# Full-width vector products and three-source integer operations

The bounded gfx1030 VOP3 subset adds six handwritten instruction identities;
the semantic registry is unchanged. All results are 32-bit words.

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
OP_SEL, CLAMP, OMOD or NEG modifiers. Products have two active operands and
require unused SRC2=128 (inline zero). This is a canonical subset restriction:
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

## Evidence and acceptance

Semantics and fields follow the public [AMD RDNA2 ISA, sections 6.2.1, 12.12
and 13.3.4](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content).
Native ARM64 LLVM 23.1.6 gfx1030 assembly/disassembly probes recorded in
`artifacts/s31-vop3-source-evidence.md` establish encoding and source-limit
observations, not arithmetic execution correctness.

The original fixture is `fixtures/rdna2/vop3-integer.s`. Decoder source/field
checks and the LLVM matrix accompany independent references, compute tests,
`crates/shader/tests/graphics_vop3_integer.rs` and native graphics tests in
`crates/metal/tests/native/vop3_integer_graphics.rs`. Both independent reviewers accepted this bounded increment after CPU, LLVM,
native Metal and full repository validation. See the
[local checkpoint](../testing/local-graphics-checkpoint.md) for counts and limits.
This six-form increment does not complete S3.1 or establish Xbox compatibility.
