# Execution contract: bounded Wave32 VOPC integer compares (issue #186)

Status: v2, after independent review (codex-contract.log); findings 1-7 addressed. Scope is the Xenora compute-fixture ABI
(one input word and one output word per dispatched thread; dispatch index `i`
is wave `i / 32`, lane `i % 32`; every wave starts with EXEC_LO = 0xffffffff;
scalar state is wave-uniform).

## 1. Sources (public only)

- AMD "RDNA 2" Instruction Set Architecture Reference Guide, 30 Nov 2020
  ([public PDF](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf)):
  - S1 §2.1 "Wave32 and Wave64", printed p. 7-8 (PDF p. 15-16): "The upper half
    of EXEC and VCC are ignored for wave32 waves."
  - S2 §3.9 "Vector Compares: VCC and VCCZ", printed p. 19-20 (PDF p. 27-28):
    compares set VCC (1 = pass, 0 = fail); "VCCZ is updated every time VCC is
    updated: vector compares and scalar writes to VCC"; "V_CMP_* =>
    VCC[n] = EXEC[n] & (test passed for thread[n])"; "VCC is fully written;
    there are no partial mask updates"; in wave32 "only the lower 32 bits of
    VCC contribute to the value of VCCZ".
  - S3 §4.5 "Manually Inserted Wait States", printed p. 30 (PDF p. 38):
    "Inserting S_NOP is not required to achieve correct operation."
  - S3b §3.3 "EXECute Mask", printed p. 13 (PDF p. 21-22): EXEC "does not
    affect scalar (ALU or memory) execution or branches"; in wave32 EXECZ
    reflects only the lower 32 bits.
  - S4 §6.2.2 "Instruction Outputs", printed p. 43 (PDF p. 51): a thread
    writes a VGPR result only if its EXEC bit is set; V_CMPX writes EXEC.
  - S4b §6.2 Table 20 (operand selectors), PDF p. 49-50: 106 = VCC_LO.
  - S5 §12.9 "VOPC Instructions", printed p. 150-152 (PDF p. 158-160): field map
    SRC0 / VSRC1 / OP; "producing a 1 bit result per lane into VCC or EXEC";
    Table 59: integer compare offsets LT 1, EQ 2, LE 3, GT 4, LG/NE 5, GE 6.
    Table 61 "VOPC Compare Opcodes", printed p. 155 and 159 (PDF p. 163, 167):
    128-135 V_CMP_{F,LT,EQ,LE,GT,NE,GE,T}_I32 and 192-199 the _U32 forms,
    "D = VCC in VOPC encoding". The XML is authoritative for encodings.
- AMD RDNA 2 machine-readable ISA XML (pinned SHA-256 `d671ecbc...`, used by
  `crates/isa/generate_rdna2.py`; see `docs/research/rdna2.md`): ENC_VOPC opcodes V_CMP_{LT,EQ,LE,GT,NE,GE}_I32
  = 129-134 and V_CMP_{LT,EQ,LE,GT,NE,GE}_U32 = 193-198; descriptions "Set the
  per-lane condition code to 1 iff the first input is <rel> the second input";
  SRC0 and VSRC1 data format FMT_NUM_I32 for the _I32 forms and FMT_NUM_U32
  for the _U32 forms; destination OPR_VCC.
- LLVM 23.1.6 AMDGPU (`llvm-mc`, gfx1030, `+wavefrontsize32`) as an encoding
  oracle only: `compares_fixture_instructions_against_llvm` checks every form
  with every source class against the decoder and the reference.

## 2. Supported forms

The 32-bit VOPC encoding (bits 31:25 = 0b0111110), no SDWA or DPP:

| Form | per-lane test, S0 = SRC0, S1 = VSRC1 |
| --- | --- |
| v_cmp_lt_i32 / v_cmp_lt_u32 | S0 < S1 |
| v_cmp_eq_i32 / v_cmp_eq_u32 | S0 == S1 |
| v_cmp_le_i32 / v_cmp_le_u32 | S0 <= S1 |
| v_cmp_gt_i32 / v_cmp_gt_u32 | S0 > S1 |
| v_cmp_ne_i32 / v_cmp_ne_u32 | S0 != S1 |
| v_cmp_ge_i32 / v_cmp_ge_u32 | S0 >= S1 |

`_i32` compares the 32-bit two's-complement values, `_u32` the unsigned values.
Operand order matters (S0 is SRC0): lt with swapped operands is gt.

SRC0 selectors, exhaustively: 0-105 (normal SGPR), 106 (VCC_LO, see 5),
128-208 (inline integers 0..64 and -1..-16), 240-248 (inline floating bit patterns), 255 (one 32-bit literal, which
must be present in full after the word), 256-511 (VGPR). Every other selector
is rejected at decode, independent of EXEC: TTMPs, M0, NULL, EXEC_LO/HI,
VCC_HI, apertures, wave-ID, reserved values, SDWA 249,
DPP8 233/234, DPP16 250, VCCZ/EXECZ/SCC 251-253, LDS direct 254. VSRC1 is a
VGPR.

Wave size: the compute fixture ABI is defined as Wave32. A raw instruction
stream carries no wave-size declaration (the VOPC word has no wave-size field),
so every caller of the translator and the reference establishes Wave32 by
using this ABI; there is no Wave64 entry point to reject.

## 3. Semantics of one supported compare, Wave32

Let E = EXEC_LO before the instruction and A = the set of lanes that exist in
this wave (all 32 except in a final partial wave, see 7). As an unsigned
32-bit value,

    VCC_LO := OR over 0 <= n < 32 of ( [n in A] & E[n] & test(S0[n], S1[n]) ) << n

- VCC_LO is fully written, every time, including bits of inactive or absent
  lanes, which become 0 (S2). With E = 0 the result is VCC_LO = 0 even if VCC
  held a nonzero value before.
- Inactive lanes (E[n] = 0) read no VGPR source; their bit is 0 regardless of
  their data.
- VCC_HI: S1 says it is ignored in wave32; whether hardware writes it is not
  settled by the sources. After any supported compare, VCC_HI has no known
  value. (No supported instruction can read vcc_hi today; this keeps any
  future reader honest.)
- EXEC, SCC, every VGPR and every other SGPR are unchanged.
- VCCZ, as read by `s_cbranch_vccz`, is (VCC_LO == 0) (S2, wave32 sentence).
- No wait states are needed between the compare and a reader of VCC (S3).

## 4. Rejected (controlled translation/reference error, never a silent no-op)

- V_CMPX_* (writes EXEC: divergent execution; needs its own contract).
- The VOP3 encoding of any compare (arbitrary SGPR destination, modifiers).
- SDWA and DPP forms; F/T, 16-bit, 64-bit and float compares; v_cmp_class.
- SRC0 SGPRs other than s0-s106, TTMPs,
  M0, EXEC_LO/HI, VCC_HI, SCC/VCCZ/EXECZ selectors, LDS direct.
- A normal-SGPR SRC0 without a known value, when EXEC_LO != 0.
- Wave64 in any form.

## 5. VCC_LO state and where a compare result may flow

Both the translator and the reference track VCC_LO in one of three static
states, decided by the instruction path alone, never by lane data:
- *uninitialized* (program start);
- *known*: written by `s_mov_b32 vcc_lo, <known value>`, or by a compare
  executed while EXEC_LO is statically 0 (result: known 0);
- *lane-derived*: written by a compare executed while EXEC_LO != 0. It stays
  lane-derived even when a particular wave's result happens to be 0 or equal
  to a constant; acceptance never depends on that value.

Acceptance rules for one compare:
- EXEC_LO = 0 (EXEC is always a known value in this ABI): the encoding is
  validated, no source is read or checked (not VGPR readiness, SGPR knowledge
  or VCC state), and VCC_LO becomes known 0.
- EXEC_LO != 0: VSRC1 and a VGPR SRC0 must be initialized and not pending a
  load; a normal-SGPR SRC0 must be known; a `vcc_lo` SRC0 must be known or
  lane-derived (uninitialized is rejected). VCC_LO becomes lane-derived.

While VCC_LO is lane-derived it may be read only by:

1. `s_cbranch_vccz` (6);
2. `v_mov_b32_e32 vD, vcc_lo`, which writes the 32-bit mask into vD of active
   lanes (inactive lanes keep vD, as for any partial-EXEC move);
3. SRC0 of a supported compare (`v_cmp_* vcc_lo, vcc_lo, vN`), which reads the
   old VCC_LO in every active lane before the new value is written.
4. SRC0 of a registry VOP2 integer form (for example
   `v_add_nc_u32_e32 vD, vcc_lo, vN`), which reads the 32-bit mask in every
   active lane, as `v_mov_b32_e32` does.

`v_mov_b32_e32 vD, vcc_lo` and the VOP2 forms follow the usual EXEC rules: with EXEC_LO = 0 it
is skipped without reading VCC; otherwise VCC_LO must be known or
lane-derived. Every other read of vcc_lo is rejected in any state: the
supported scalar instructions (`s_mov_b32`, SOP2, `s_cmp_eq_u32`) do not
accept vcc_lo as a source at decode, so `s_mov_b32 exec_lo, vcc_lo` cannot
create divergent EXEC. `s_mov_b32 vcc_lo, <known>` makes VCC_LO known again.

## 6. Branches

- `s_cbranch_scc0`: unaffected (compares do not touch SCC).
- `s_cbranch_execz`: unaffected (compares do not touch EXEC).
- `s_cbranch_vccz` with a known VCC_LO: resolved during translation, as today.
- `s_cbranch_vccz` with a lane-derived VCC_LO (a *runtime branch*): the whole
  wave takes it iff VCC_LO == 0. Control flow stays wave-uniform; no lane
  diverges. Accepted only if the skipped region R (from the end of the branch
  to the target, exclusive) satisfies all of:
  - the target is a forward instruction boundary (existing rule);
  - no vector load is outstanding at the branch;
  - R contains only these integer vector forms: `v_mov_b32_e32` (from a
    VGPR, an immediate (inline integer, any of the nine inline floating bit patterns or a
    literal; the move copies bits), a known normal SGPR, or vcc_lo known or
    lane-derived), `v_add_nc_u32_e64`, `v_lshlrev_b32_e32`,
    `v_lshrrev_b32_e32`, `v_ashrrev_i32_e32`, `v_and_b32_e32`, `v_or_b32_e32`,
    `v_xor_b32_e32`. Nothing else: no FP32 form, scalar instruction, compare,
    branch, wait, memory access or `s_endpgm`;
  - every VGPR written in R was initialized before the branch;
  - every source read in R is ready where it is read (the normal rules, as on
    the fall-through path).
  These rules are checked on R's instructions whether the branch is taken or
  not, and whatever EXEC_LO is, so acceptance does not depend on lane data.
  After R, each VGPR written in R holds its value from R if the branch was not
  taken, else its value from before the branch; scalar state (SGPRs, SCC,
  EXEC, VCC state) is the same on both paths. The translator's known-bit
  facts for a joined VGPR keep only bits known and equal on both inputs, and
  the union of the candidate values when it has at most two (so an exact
  FP32 operation after the join is still accepted when both edges allow it).
  An outstanding store at the branch is allowed: the store already read its
  data.

## 7. Wave formation and absent lanes

Waves are formed from consecutive dispatch indices: wave w is indices
32w..32w+31. A final partial wave (input length not a multiple of 32) has
absent lanes; they never execute and contribute 0 to every compare result.
(The fixture's EXEC_LO still reads 0xffffffff in such a wave; that existing
ABI simplification is unchanged and documented as a limitation.)

## 8. Mapping to Metal (no SIMD-group assumption)

Metal SIMD groups, threadgroups and RDNA 2 waves are not assumed to coincide.
The Metal kernel makes no cross-thread call: no simd_ballot, no threadgroup
memory, no barrier. For each compare, every thread recomputes the compare's
per-lane operand values for each of the 32 lanes of its own RDNA wave
(dispatch indices (index & ~31) + n, bounded by the grid size from
`[[threads_per_grid]]`), from those lanes' input words, and assembles VCC_LO
itself. All values are pure functions of input words and translation-time
constants, so every thread of a wave computes the same VCC_LO. A runtime
branch becomes a per-register select on (VCC_LO != 0) at the join; region
instructions are pure 32-bit ALU and are evaluated unconditionally.

## 9. Verification

`fixtures/rdna2/vector-compare.s` is an original program, assembled by LLVM.
It uses all twelve forms with VGPR, SGPR, inline, literal and `vcc_lo`
sources, a compare and a mask move under partial EXEC, a compare under empty
EXEC that must clear a stale VCC, SCC kept across compares, and a runtime
`s_cbranch_vccz`. Its expected output is written in
`tests/validation/known_answers.rs` from the listing, with an explicit
relation for each row. The test input has a wave whose branch is taken, one
whose branch is not, and a final partial wave of 9 lanes.

The same file runs each form against 10 boundary values (0, 1, 0x7ffffffe,
0x7fffffff, 0x80000000, 0x80000001, 0xfffffff0, 0xfffffffe, 0xffffffff,
0x0f0f0f0f), in both operand orders. That is 240 programs, checked against
exact i128 differences. It also covers partial and empty EXEC with seeded
destinations, both runtime branch edges in one dispatch, and 32 rejections
pinned to their byte offset. The translator, the reference (lockstep waves,
compares on widened i64 values) and the second interpreter (sign-bit flip,
then an unsigned compare) must all agree. The 9 ordered compare slots in the
fixture each change the result when their signedness is flipped. The same
programs, plus 2 mask-chain programs (252 in all), match on native Metal with API and shader validation
(`vector_compare_native_metal_known_answers`, run by `scripts/test-gpu.sh`).

A deliberately broken copy of each implementation fails these tests: the
reference with unsigned-only compares or with the EXEC test removed; the
second interpreter with unsigned-only compares or with no empty-EXEC rule;
the translator with a stale VCC under empty EXEC (CPU), and, on Metal, with
swapped signedness, without the EXEC mask, without the absent-lane bound
(shader validation aborts on the out-of-range read) or with an inverted join.

Evidence class: agreement with a specification-derived reference, run
locally on Apple M5 Pro. It is not agreement with RDNA 2 or Xbox hardware.

## 10. Limitations

- Wave32 only. No `v_cmpx`, VOP3, SDWA, DPP, 16-bit, 64-bit or float compares.
- A lane-derived VCC_LO cannot reach EXEC or any scalar instruction, so there
  is no divergent execution. That needs its own contract.
- The skipped region of a runtime `s_cbranch_vccz` holds only integer vector
  ALU instructions. Loops, nested runtime branches and scalar state that
  differs between the two edges are not supported.
- Each thread recomputes up to 32 lanes per compare. The cost grows with the
  compare's dependency chain. That is fine for fixtures, and no performance is
  claimed.
