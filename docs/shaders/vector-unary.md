# Integer vector unary instructions

The ordinary gfx1030 VOP1 encodings support `V_NOT_B32`, `V_BFREV_B32`,
`V_FFBH_U32`, `V_FFBL_B32` and `V_FFBH_I32`. Complement and reversal operate
on all 32 bits. Unsigned high/low searches return the leading/trailing zero
count, with `0xffffffff` for zero. Signed high search counts leading sign bits,
including the sign bit; zero and all ones return `0xffffffff`.

Sources use the existing nine-bit vector source domain: VGPRs, normal SGPRs,
inline integer or floating bit patterns, one literal, and the existing compute
VCC-low source. Inline floating selectors are interpreted as raw words.
The decoder consumes four bytes, or eight for a literal. VOP3 aliases, DPP,
SDWA, other special registers and modifiers remain rejected.

Compute writes active EXEC lanes and retains initialized inactive destination
words. Empty EXEC skips operand reads; SCC, VCC and EXEC remain unchanged.
Runtime VCC branch regions do not admit these new operations. Pending-load,
source initialization and reserved destination checks remain in force.

Graphics accepts ordinary scalar uniforms and initialized scalar locals;
VCC and descriptor registers remain excluded. Exact uint values use bitcasts
through the VGPR carrier, including aliases, joins and bounded loops. Integer
results cannot escape to float arithmetic, float exports, floating texture
coordinates or vertex position/parameter outputs. Validated array-layer and
pixel-coordinate metadata cannot be used as integer operands.

Original validation is in `tests/validation/tests/vop1_integer.rs`,
`crates/shader/tests/graphics_vector_unary.rs` and
`crates/metal/tests/native/vector_unary_graphics.rs`. The LLVM oracle verifies
encodings independently. Native readback is evidence about synthetic programs
on the tested Apple GPU, not Xbox hardware or game compatibility.
