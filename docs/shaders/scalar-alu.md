# SOP2 scalar ALU subset

Xenora decodes and executes thirty-six gfx1030 SOP2 forms. Each takes 32-bit
operands and writes a normal SGPR (s0-s105). The seventeen below set SCC; the
sixteen in [Arithmetic, select and bit-field forms](#arithmetic-select-and-bit-field-forms)
set it or leave it as the guide says.

| Form | D | SCC |
| --- | --- | --- |
| `s_add_u32` | S0 + S1 | carry-out of bit 31 |
| `s_sub_u32` | S0 - S1 | borrow (S1 > S0) |
| `s_add_i32` | S0 + S1 | signed overflow: S0 and S1 have the same sign and D's sign differs |
| `s_sub_i32` | S0 - S1 | signed overflow: S0 and S1 have opposite signs and D's sign differs from S0's |
| `s_addc_u32` | S0 + S1 + SCC | carry-out of bit 31 |
| `s_subb_u32` | S0 - S1 - SCC | borrow (S1 + SCC > S0) |
| `s_and_b32` | S0 & S1 | D != 0 |
| `s_or_b32` | S0 \| S1 | D != 0 |
| `s_xor_b32` | S0 ^ S1 | D != 0 |
| `s_andn2_b32` | S0 & ~S1 | D != 0 |
| `s_orn2_b32` | S0 \| ~S1 | D != 0 |
| `s_nand_b32` | ~(S0 & S1) | D != 0 |
| `s_nor_b32` | ~(S0 \| S1) | D != 0 |
| `s_xnor_b32` | ~(S0 ^ S1) | D != 0 |
| `s_lshl_b32` | S0 << S1[4:0] | D != 0 |
| `s_lshr_b32` | S0 >> S1[4:0], zero fill | D != 0 |
| `s_ashr_i32` | S0 >> S1[4:0], sign fill | D != 0 |

The last eight were added through the semantic registry alone: one TOML entry
each, with no hand-written Rust per form. Shifts use only the low five bits of
S1 as the count, as the guide's pseudocode states (section 12.1); higher bits are
ignored.

Sources are normal SGPRs, inline integers (-16 to 64), inline floating bit patterns (selectors 240-248), or one literal word.

These forms are declared as data in the semantic registry,
`crates/isa/semantics/rdna2.toml`: opcode, operation, signedness, carry-in,
SCC effect, operand rule and guide references. `crates/isa/generate_semantics.py`
checks each declaration against the pinned AMD XML snapshot and rejects
contradictions (for example a carry-in on a bitwise form, or a signed form whose
SCC is a carry). It then generates `crates/isa/src/semantics_gen.rs`, which holds
the decoder's opcode dispatch and `ScalarAluOp`'s mnemonic, SCC read and
evaluation through the shared primitives in `crates/isa/src/primitive.rs`. Run
`python3 crates/isa/generate_semantics.py validate`, `generate` or `--check`
(Python 3.11 or newer). The reference and independent interpreters do not use
the registry.
`v_mov_b32_e32` may read a normal SGPR, so the results can reach vector code.

## Inline floating source words

The existing 32-bit scalar source readers, vector moves and registry-backed
vector integer/comparison readers accept selectors 240 through 248. They supply
exact bits, even when an integer instruction consumes them:

| Selectors | Values | Raw words |
| --- | --- | --- |
| 240 / 241 | +0.5 / -0.5 | `3f000000` / `bf000000` |
| 242 / 243 | +1.0 / -1.0 | `3f800000` / `bf800000` |
| 244 / 245 | +2.0 / -2.0 | `40000000` / `c0000000` |
| 246 / 247 | +4.0 / -4.0 | `40800000` / `c0800000` |
| 248 | 1/(2*pi) | `3e22f983` |

These consume no trailing literal word. Selector 255 still requires one; two
literal selectors remain outside the supported subset. Nine-bit vector source
selectors 496-504 still name v240-v248. This extends operand decoding, not the
set of arithmetic opcodes, precision modes or 16/64-bit operands. Register-only
legacy vector instruction readers retain their restrictions.

## Graphics scalar execution

Graphics stages support all 36 decoded 32-bit SOP2 forms and all 11 decoded
SOP1 unary forms described here. Normal local registers s32-s105 hold constants
or runtime values; pixel uniforms s24-s31 are read-only inputs. Existing resource
descriptor, reserved-register and uninitialized-source checks remain in force.
Both sources are captured before an aliased destination changes. Flagless forms
preserve SCC, including its uninitialized state; flag-producing forms establish it.

Runtime lowering uses unsigned bit arithmetic, masked shift counts and explicit
bit reinterpretation for signed comparisons, arithmetic shifts and high products.
Field extraction handles zero widths and widths at least 32 without an oversized
native shift. Shift-add carry includes bits discarded by the shift. Unary bit
searches return the documented all-ones sentinel when no bit matches.
Constant evaluation uses the same production semantics as compute; independent
literal graphics tests exercise both constant and runtime paths. This closes the
graphics gap for these decoded scalar forms, not full instruction or operand
coverage. The [local checkpoint](../testing/local-graphics-checkpoint.md) records
validation and acceptance separately.

## Sources

- The AMD "RDNA 2" Instruction Set Architecture reference guide (GPUOpen,
  `rdna2-shader-instruction-set-architecture.pdf`, 291 pages) gives the
  pseudocode for every form in section 12.1, "SOP2 Instructions" (pages
  101-102). Section 5.3, "Scalar Condition Code" (page 33), defines SCC as
  signed overflow for signed add and subtract and as carry-out for the
  unsigned forms. The unsigned rules below were derived before the guide was
  available and agree with it.

- Opcodes, operand fields and which forms read and write SCC come from the
  pinned AMD RDNA 2 XML (`crates/isa/generate_rdna2.py`). Its descriptions say
  that additions "store the carry-out bit into SCC", ADDC adds "a carry-in bit
  from SCC", SUBB subtracts "the carry-in bit", and the bitwise forms "set SCC
  iff the result is nonzero".
- For subtraction, the XML's "carry-out" does not say whether SCC is the borrow
  or its complement. LLVM's AMDGPU backend settles it. It lowers unsigned
  subtract-with-overflow (`S_USUBO_PSEUDO`) to `s_sub_u32` and reads SCC as the
  overflow. It also lowers 64-bit subtraction to `s_sub_u32` followed by
  `s_subb_u32` (`SIISelLowering.cpp`), which is only correct if SCC is the
  borrow and SUBB subtracts it. LLVM 23.1.6 also checks every encoding here
  against the decoder and the reference (`compares_fixture_instructions_against_llvm`).

## Scalar half-word packing

The three scalar packing forms select 16-bit halves without reading or changing
SCC. `s_pack_ll_b32_b16` places S0's low half below S1's low half;
`s_pack_lh_b32_b16` places S0's low half below S1's high half;
`s_pack_hh_b32_b16` places S0's high half below S1's high half.
These follow the [AMD RDNA 2 guide, section 12.1](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture).
Normal SGPR destinations/sources, inline integers, inline floating bit patterns and at most one literal retain
the existing operand restrictions. Aliased sources are read before the destination
changes. An unset SCC remains unset.

Compute translation resolves scalar values at translation time. Graphics stages
also accept runtime uniform/local operands and emit unsigned integer expressions;
they preserve SCC across packing rather than deriving it from the result. Runtime
graphics values are checked against literal packed words and SCC-dependent pixels.
This extends three forms, not the whole Stage 3 instruction milestone.

Original LLVM fixtures are `scalar-pack` and `x9-scalar-pack-{ll,lh,hh}-ps`.
Independent cases are in `tests/validation/tests/scalar_pack.rs`; the reference
interpreters do not share production semantic code. Native compute tests are
opt-in and require Metal validation. Current execution evidence is recorded in
the local graphics checkpoint after validation.

## Not supported

- Every other SOP2 form, including the 64-bit bitwise forms and `s_cselect_b64`.
- Destinations other than s0-s105 (VCC, EXEC, TTMP and M0).
- Sources other than normal SGPRs, inline integers, inline floating bit patterns and one literal.
- Both sources selecting a literal.
- ADDC, SUBB or CSELECT before SCC is set.
- Writes to the fixture's resource descriptors s0-s7.

## How it runs

Scalar state in a compute fixture is wave-uniform and known while the code is
translated. As with `s_mov_b32` and the SOPC compares, the translator evaluates
SOP2 forms itself. It resolves SCC-dependent branches and emits SGPR values
read by `v_mov_b32_e32` as constants (`fixtures/golden/scalar-alu.ir`). So the
IR and MSL contain the results, not scalar instructions.

## Verification

`signed_add_and_subtract_set_scc_on_overflow_not_carry` checks eight signed
cases worked by hand, including -1 + 1 and 0 - 1, which carry or borrow but do
not overflow. The reference checks that the exact signed result fits in i32.
The independent interpreter applies the guide's sign-bit rule.

`fixtures/rdna2/scalar-alu.s` is an original program, assembled by LLVM. It
chains a carry through `s_add_u32`/`s_addc_u32` and a borrow through
`s_sub_u32`/`s_subb_u32`. It also covers aliased destinations, inline and
literal operands, bitwise results that set and clear SCC, and an
`s_cbranch_scc0` that depends on them. It also includes two signed overflows.
Its expected output, input ^ 0x0000000e,
was derived by hand.

`tests/validation/known_answers.rs` runs the first nine forms over 10 boundary
operands squared, with SCC clear and set. That is 1,800 programs. The same file
covers aliasing, inline and literal operands, and rejected forms. Expected
values come from exact integer definitions in the test. Three implementations
must agree with them:
- the translator (resolved branch and emitted constant);
- the reference interpreter (exact integers, reduced modulo 2^32);
- the independent interpreter (a bit-serial ripple-carry adder).

The eight logical and shift forms added through the registry have their own
suite, `tests/validation/tests/sop2_stage_c.rs`: 6,896 cases whose expected
values were derived from the guide separately from the registry, checked on
both CPU interpreters and, through the translator, on native Metal. The
LLVM-assembled `fixtures/rdna2/scalar-logic.s` (expected input ^ 0x887f00f8)
runs on all three and on native Metal.

`scalar_alu_native_metal_known_answers` runs the fixture, the 1,800 programs
and the aliasing and immediate cases on native Metal with API and shader
validation. `scripts/test-gpu.sh` runs it. This is local evidence from one
Apple M5 Pro; independent acceptance is pending.

## Arithmetic, select and bit-field forms

Sixteen more 32-bit SOP2 forms are declared in the registry with the same
operand rule (`sop2_sgpr`), each one TOML entry over a shared primitive:

| Form | D | SCC |
| --- | --- | --- |
| `s_min_i32`, `s_min_u32` | S0 if S0 < S1 (signed or unsigned), else S1 | S0 < S1 |
| `s_max_i32`, `s_max_u32` | S0 if S0 > S1, else S1 | S0 > S1 |
| `s_cselect_b32` | SCC ? S0 : S1 | read, not written |
| `s_bfm_b32` | ((1 << S0[4:0]) - 1) << S1[4:0] | unchanged |
| `s_mul_i32` | low 32 bits of S0 * S1 | unchanged |
| `s_mul_hi_u32`, `s_mul_hi_i32` | high 32 bits of the unsigned or signed 64-bit product | unchanged |
| `s_bfe_u32` | (S0 >> S1[4:0]) masked to S1[22:16] bits | D != 0 |
| `s_bfe_i32` | (S0 >> S1[4:0], sign fill) masked to S1[22:16] bits, sign-extended from the field's top bit | D != 0 |
| `s_absdiff_i32` | \|S0 - S1\| of the wrapped 32-bit difference (0x80000000 stays) | D != 0 |
| `s_lshl1_add_u32` to `s_lshl4_add_u32` | (S0 << N) + S1, N = 1 to 4 | carry out of the exact sum, counting bits shifted out of S0 |

These follow the guide's 12.1 pseudocode and Tables 11, 12 and 14 (pages
34-36). Min and max compare strictly, so equal operands give S1 and SCC = 0.
The guide leaves two C-like operators open; Xenora reads them as exact
arithmetic: a BFE width of 32 to 127 keeps every shifted bit, and a width of 0
gives 0. These are guide readings, not hardware evidence. CSELECT is refused
when SCC has not been set, as ADDC and SUBB are.

The registry gained the vocabulary for them: the operations above, a
`selected_src0` SCC effect for min and max, `carry_in = "scc"` for the select
condition, and a declared `shift_amount` (1 to 4) for the shift-and-add forms.
The generated `ScalarAluOp::evaluate` now returns `(u32, Option<bool>)`, with
`None` for forms that leave SCC unchanged.

`tests/validation/tests/sop2_arith.rs` checks 21,070 cases, with expected values
derived from the guide (including its ABSDIFF examples) separately from the
registry, on both CPU interpreters and, through the translator, on native
Metal; its 597 rejected programs are checked on the CPU interpreters and the
compiler. `fixtures/rdna2/scalar-arith.s` (LLVM-assembled, expected input ^
0x81157dfe) chains all sixteen, including a min tie and CSELECT on both SCC
values, and shows SCC surviving the forms that do not write it.

## SOPC compares

The twelve 32-bit SOPC integer compares `s_cmp_{eq,lg,gt,ge,lt,le}_{i32,u32}`
are declared in the same registry (operand rule `sopc_scalar`). Each sets only
SCC, to whether the relation holds with SSRC0 on the left; LG means not equal.
EQ and LG give the same result for `_i32` and `_u32`; GT, GE, LT and LE compare
two's-complement values for `_i32` and unsigned values for `_u32` (guide section
12.4). Sources follow the SOP2 rule: normal SGPRs, inline integers, inline floating bit patterns or one
literal. Both sources selecting the literal encoding (255) is rejected for SOPC
as for SOP2; whether the guide allows one shared literal there was not settled.

`tests/validation/tests/sopc_stage_d.rs` checks 7,824 cases, with expected
values derived from the guide separately from the registry, on both CPU
interpreters and, through the translator, on native Metal (compiled per case
and read back from nine distinct generated kernels). Its 348 rejected programs
are checked on both CPU interpreters and against the compiler, which rejects
them before any Metal dispatch. `fixtures/rdna2/scalar-compare.s` (LLVM-assembled, expected input ^
0x00000d5b) records every compare's SCC through an `s_cbranch_scc0` per form.

## SOP1 unary forms

Eleven 32-bit SOP1 forms are declared in the registry (operand rule
`sop1_sgpr`): `s_not_b32`, `s_brev_b32`, `s_bcnt0_i32_b32`, `s_bcnt1_i32_b32`,
`s_ff0_i32_b32`, `s_ff1_i32_b32`, `s_flbit_i32_b32`, `s_flbit_i32`,
`s_sext_i32_i8`, `s_sext_i32_i16` and `s_abs_i32`. Following the guide (section
12.3): a find-first or find-last search that finds nothing returns -1
(0xffffffff); `s_flbit_i32` returns -1 for both 0 and -1; sign extension reads
S0[7:0] or S0[15:0]; `s_abs_i32` of 0x80000000 is 0x80000000. Only NOT, BCNT0,
BCNT1 and ABS write SCC, as D != 0; the other seven leave it unchanged.
`s_mov_b32` stays hand-written, because it also accepts EXEC and VCC halves as
destinations. As for SOP2, destinations s0-s7 are refused when a fixture runs
(they hold its buffer descriptors). Inline floating constants carry exact integer bits;
they do not numerically convert a floating value to an integer.

`tests/validation/tests/sop1_unary.rs` checks 4,126 cases, with expected values
derived from the guide (including its worked examples) separately from the
registry, on both CPU interpreters and, through the translator, on native
Metal; its 231 rejected programs are checked on the CPU interpreters and the
compiler. `fixtures/rdna2/scalar-unary.s` (LLVM-assembled, expected input ^
0x0f0080f5) chains all eleven and shows SCC surviving the forms that do not
write it.

## Signed immediate scalar instructions

The gfx1030 SOPK subset accepts `s_movk_i32` (0), `s_addk_i32` (15), and
`s_mulk_i32` (16), each four bytes with a signed 16-bit immediate and normal
SGPR destination (s0–s105 at decoding; consumers retain protected-register checks).
ADDK and MULK read the old destination before writing it. MOVK requires no old
value. ADDK sets SCC on signed overflow; MOVK and MULK preserve SCC, including
its uninitialized state. Multiplication retains the low 32 bits.

Production decoding normalizes these to existing move/add/multiply operations;
metadata retains the original SOPK identity. Compute resolves scalar constants;
graphics supports constants and runtime scalar uniforms/locals. The shared
graphics multiply operation also accepts existing SOP2 `s_mul_i32`. Unsupported
special registers and neighboring opcodes still reject explicitly.

The contract comes from the public AMD RDNA2 ISA, section 12.2, and pinned
`rdna2.jsonl` metadata. `scalar_immediate.rs` uses literal expected words and
independently implemented references. `compares_sopk_fixture_against_llvm`
checks instruction bytes, offsets and normalized semantics while explicitly
checking all three original SOPK identities. Native tests are synthetic Metal
checks, not evidence of AMD or Xbox hardware equivalence.

## Scalar immediate comparisons

All twelve gfx1030 SOPK `s_cmpk_{eq,lg,gt,ge,lt,le}_{i32,u32}` forms are
implemented for s0–s105 sources. The SDST field supplies a read-only scalar
source. Signed operations sign-extend the 16-bit immediate; unsigned operations
zero-extend it. Every comparison replaces SCC with its predicate and preserves
the source register. Special registers remain outside this subset; an
uninitialized source rejects in compute and graphics consumers.

The four-byte encoding normalizes to existing scalar comparison operations.
The LLVM oracle separately checks the original twelve SOPK identities. Compute
uses statically resolved scalars; graphics compares constants or runtime
uniform/local values. Literal relation masks and original shaders test the
[public AMD RDNA2 guide](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content),
section 12.2, independently of the production evaluator. This does not establish
AMD or Xbox hardware equivalence or complete S3.1.
