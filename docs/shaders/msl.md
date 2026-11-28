# Synthetic compute lowering

The compiler accepts explicitly tagged raw gfx1030 code up to 1,024 bytes and requires a whole number of 32-bit words. The verifier checks source initialization, exact resource slots and address operands, immutability of VGPR 0, memory ordering, one completed output store and program termination. VGPR 0 is reserved for resource indexing. Keeping it out of arithmetic means the host's grid position is never exposed as a guest register value.

Verification maps each initialized register to its latest typed temporary. Reads after a load require a load wait. Because generated memory accesses are synchronous expressions within a thread and buffers are disjoint, waits can be discharged during verification. Metal completion is still required before host readback. Store and load aliases between lanes are excluded by the [fixture ABI](../adr/0003-demo-memory-model.md). The output buffer uses Metal index 1 and the read-only input uses index 0.

Apple documents `uint`, device address space and the scalar compute grid-position attribute used here. These MSL language features are class B (CLAIM-MSL-COMPUTE-TYPES). [Metal Shading Language Specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification.pdf)

The integer emitter uses unsigned addition, bitwise logic, and shifts. Scalar moves and compares are evaluated at translation time using known SGPR initializers. Forward branches must target instruction boundaries. Conditional branches require known scalar condition code (SCC), execution mask (EXEC), or vector condition code (VCC) state. Wave32 zero checks read the low halves of EXEC and VCC. Known partial EXEC masks merge each vector result with the initialized destination according to the logical lane number, computed from the grid index modulo 32. Empty EXEC suppresses vector arithmetic and moves. Reachable vector memory operations require a full mask. The IR records the statically selected path. Scalar values can originate only from constants and initialized SGPRs, and SGPR 0–7 remain reserved for synthetic descriptors. The generated kernel contains only the selected scalar branch path. The partial-mask merge uses a per-invocation selection from two vector values.

Twelve VOP2 integer forms from the semantic registry lower through one typed template per declared operation (`VectorBinary`), not per opcode: wrapping `uint` add, subtract and XNOR; `min`/`max` on `uint`, or on `as_type<int>` for the signed forms; and the 24-bit multiplies, which first sign-extend bit 23 with `as_type<int>(x << 8u) >> 8` (or mask to 24 bits for the unsigned forms), then take the low word as a wrapping `uint` product or the high word with MSL `mulhi`. `v_subrev_nc_u32` declares reversed sources, so the translator swaps them. A wave-uniform SRC0 (an SGPR, inline integer or literal) becomes a constant operand, while the lane's VGPRs stay runtime values; `vcc_lo` as SRC0 uses the lane-derived compare mask when a VOPC produced it. These forms are refused inside code skipped by a runtime `s_cbranch_vccz`, as both interpreters refuse them.

The float addition/multiplication subset accepts operations only when every possible operand is a positive normal binary32 value and the exact result is itself representable as a normal binary32 value. A known-bit analysis propagates the effect of masks and XOR. It enumerates up to two candidate bit patterns per operand, proves the result for each pair, and retains at most two unique results for an ensuing operation. No rounding mode, signed-zero behavior, subnormal mode, NaN handling or infinity policy is required for this restricted domain. Unknown or inexact cases fail verification.

The generated MSL uses unsigned words for register bits and `as_type` for reinterpretation at individual float operations. It disables contraction and reassociation for these kernels. The compiler tests compare all three fixture outputs against tracked IR and MSL goldens. The [native demonstration](../../scripts/demo.sh) checks executed results.

## Fixed-mode FP32 rounding and signed-nibble conversion

Both VOP1/e32 and VOP3/e64 encodings of `V_TRUNC_F32`, `V_CEIL_F32`,
`V_RNDNE_F32`, `V_FLOOR_F32`, and `V_CVT_OFF_F32_I4` accept a VGPR input
and write raw IEEE bits through compute and graphics integer carriers. The
rounding operations use unsigned bit arithmetic in MSL, avoiding host rounding,
fast-math and flush-to-zero dependence. `V_CVT_OFF_F32_I4` interprets the low
four bits as signed -8..7 and scales by 1/16, exactly representable in FP32.
Higher source bits are ignored. Aliased operands are captured before writing;
empty and partial EXEC retain inactive destination words in compute.

This synthetic ABI fixes the rounding subset to preserved input subnormals,
IEEE NaN propagation and no output clamp: infinities pass through, signed zero
is retained, and NaNs retain sign and payload with the quiet bit set. The four
rounding instructions choose truncation, ceiling, nearest-even or floor
explicitly. This is a bounded numerical contract, **not support for arbitrary
console MODE state**. Mode-register writes, `S_ROUND_MODE`, `S_DENORM_MODE`,
non-VGPR inputs, DPP/SDWA, ABS/NEG, CLAMP and output modifiers remain rejected.
Floating-point exception flags/traps are not modeled; these tests make no claim
about them. The existing restricted FP32 add/multiply contract is unchanged.

Public semantic basis: AMD's [RDNA2 ISA guide](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture),
§3.5 (MODE fields), §6.4 (denormal/rounding state), and §12.8 (VOP1
opcodes 14, 33–36). The NaN policy is the stated IEEE-propagation contract;
no Xbox hardware validation or exhaustive mode compatibility is asserted.
The [local checkpoint](../testing/local-graphics-checkpoint.md#stage-31-batch-fp32-rounding-and-signed-nibble-conversion)
contains exact readback evidence and reproduction commands.
