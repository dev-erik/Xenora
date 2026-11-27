# Control flow, wave operations and memory

Reviewed 27 November 2026. These workstreams are open. The inventory is desktop gfx1030; it does not establish Xbox binary compatibility.

The earlier 7/8/12 totals excluded non-arithmetic families. The [generated checklist](shader-checklist.md) now assigns SOPP branches, DS lane operations and memory families explicitly. Reclassification is not implementation progress. State operations and cross-cutting operand modes still need separate closure.

## S3.2 control flow

Implemented increments: SCC1 complements SCC0; Wave32 compute EXECNZ/VCCNZ complement the existing zero-mask branches. Indirect calls/returns, general loops, traps and full divergent control flow remain beyond this increment. Branch-target validation, undefined SCC and countdown proofs must remain intact.

## S3.4 wave and lane

Implemented subsets: MBCNT LO/HI and bounded lane-zero reads under an explicit Wave32 compute contract. LO counts selected bits below the guest lane and adds the accumulator; HI adds no bits for lanes below 32. Typed decoding, compute lowering and an independent CPU reference are implemented. Native validation is recorded in the checkpoint; this does not complete wave/lane support.

| Family | Form | Decoder present at audit |
| --- | --- | --- |
| DS | `DS_BPERMUTE_B32` | no |
| DS | `DS_PERMUTE_B32` | no |
| DS | `DS_SWIZZLE_B32` | no |
| VOP1 | `V_READFIRSTLANE_B32` | yes; lane-zero compute subset |
| VOP3 | `V_MBCNT_HI_U32_B32` | yes; Wave32 compute subset |
| VOP3 | `V_MBCNT_LO_U32_B32` | yes; Wave32 compute subset |
| VOP3 | `V_PERMLANE16_B32` | no |
| VOP3 | `V_PERMLANEX16_B32` | no |
| VOP3 | `V_READFIRSTLANE_B32` | no |
| VOP3 | `V_READLANE_B32` | yes; lane-zero compute subset |
| VOP3 | `V_WRITELANE_B32` | no |

The implemented compute contract is Wave32 only, with guest lane `index & 31`:

- LO: `src1 + popcount(src0 & lower_lane_bits)`, wrapping at 32 bits.
- HI: `src1` unchanged for Wave32.
- EXEC controls destination writes, not which SRC0 bits are counted. Inactive destinations retain their prior values.
- Admit ordinary VGPR/SGPR/inline/literal sources under existing scalar/literal limits; reject LDS-direct, unsupported special sources and modifiers. Graphics remains rejected until a separate lane contract exists.

Independent tests cover: lanes 0/1/31; wrapping accumulation; sparse SRC0; destination/source aliasing; partial EXEC with sentinels; lengths 1/31/32/33/63/65; repeated guest-wave boundaries; and both forms in a recomputed comparison-expression cone. The implementation must avoid shifting a 32-bit value by 32. The native suite also covers constant-source LO comparisons and partial-EXEC HI comparison cones.

Contract basis: [AMD RDNA2 ISA §12.12 and §6.2](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf). Narrow operand admission is a project design choice, not a claim that other hardware forms are invalid.

Acceptance must specify Wave32/Wave64, EXEC and inactive lanes, source/destination aliasing, literal/modifier admission, and partial waves. Graphics needs its own guest-lane contract. DPP/SDWA forms, helper lanes and quad behavior cannot be inferred from a mnemonic count. Never assume Metal SIMD width equals guest wave width.

## S3.5 memory and interfaces

Implemented waits: `S_WAITCNT_VMCNT null,0` under the existing synthetic compute pending-load contract. A load wait must not discharge pending stores. Unknown/nonzero thresholds and other scalar operands remain rejected; graphics admission is separate.

| Inventory area | Required closure beyond decoding |
| --- | --- |
| MUBUF / MTBUF / SMEM | Descriptors, addressing, types, offsets, bounds and pending loads/stores |
| FLAT / GLOBAL / SCRATCH | Address spaces, private backing, bounds, visibility and aliasing |
| DS except permutations | LDS layout, bank-independent semantics, atomics, synchronization and scope |
| MIMG | Resource/sampler descriptors, coordinates, formats, sampling, stores and atomics |
| EXP / VINTRP / VOP interpolation | Stage interfaces, export ordering, interpolation and linkage |
| Waits / barriers / cache controls | Exact counters, scopes, visibility, rendezvous and unsupported-mode rejection |

Waits cannot be blanket no-ops. Resource graphs and local readback tests do not establish all guest ordering semantics. LGKMCNT/EXPCNT and register/nonzero thresholds remain deferred until their pending-state contracts exist.

## Evidence and next gates

The [first-increment checkpoint](../testing/stage3-control-memory-opening.md) records SCC1 and VMCNT evidence. S3.4 has bounded masked-bit-count and lane-zero-read increments; seven forms remain undecoded.

The [opening plan](../superpowers/plans/2026-11-27-stage3-opening.md) tracks the first increments. Require exact encoding checks, independent CPU expectations, applicable native compute/graphics readback and independent review before acceptance. Checkpoint results must identify the tested source version; this document alone records no passing GPU tests.

Public semantic reference: [AMD RDNA2 ISA, document 70648](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture). The [source notes](../research/rdna2.md) pin the machine-readable inventory and LLVM version.

## Remaining closure work

These are dependency groups, not equal-sized tasks or a complete operand matrix.

| Stage | Next bounded work | Architectural work still required |
| --- | --- | --- |
| S3.2 | Additional proven loop shapes | Compute loops and live-out joins; code addresses and calls/returns; exceptions; dynamic EXEC and reconvergence |
| S3.4 | Expand lane-read selection and establish contracts for seven remaining forms | Inactive/absent source lanes; cross-lane data movement; graphics/helper lanes; Wave64; DPP/SDWA mode coverage |
| S3.5 | More buffer transfer forms with explicit addressing and destination spans | Multiple outstanding transfers; descriptors and address spaces; scalar/private/LDS memory; atomics and rendezvous; image/stage-interface matrices |

Each accepted increment needs exact source and destination modes, negative cases,
independent expected outputs, applicable native tests and documented limits.
Wait-counter thresholds cannot be accepted before the corresponding event and
dependency states are modeled. The full-stage acceptance criteria remain open.

### Implemented bounded control and memory contracts

Implemented control increment: `S_CBRANCH_EXECNZ` and `S_CBRANCH_VCCNZ` test the
nonzero low mask under Wave32. Preserve full-stream target validation and
forward-only compute admission. Lane-derived VCC may skip only the existing
validated integer-vector region; joined values must select pre-branch state
when the branch is taken. Unknown VCC, nested runtime regions, memory operations
inside a region and graphics use remain rejected. These complements do not
add dynamic EXEC or general loops.

Implemented memory increment: signed and unsigned byte/halfword buffer loads under
the existing synthetic four-byte input stride. Each invocation reads the low
8 or 16 bits of its own little-endian input word; signed forms extend to 32 bits,
unsigned forms zero-extend. This preserves the current two-buffer ABI and input
bounds, single pending load, full EXEC, immutable index, zero offsets and input
resource restrictions. It does not decode hardware resource descriptors or
permit cross-lane addressing. Graphics and other memory modes remain rejected.
The opcode meanings are specified by AMD RDNA2 §12.14; the stride and restricted
bindings are the project ABI, not an inferred Xbox descriptor layout.

## Wave/lane closure dependencies

The implementation order is S3.4 before S3.2; the [continuation checklist](../superpowers/plans/2026-11-27-stage3-opening.md#execution-order-complete-wavelane-support-before-broader-control-flow)
records the full sequence. Numbering does not require completing all control
flow before implementing lane operations. Dynamic execution masks, graphics
helper lanes and pending DS results still need explicit cross-stage integration.

Primary-source contract audit (AMD document 70648, printed pages 137,
183–187 and 205–208):

| Operation group | Contract that must be preserved |
| --- | --- |
| READFIRSTLANE | Select the lowest set EXEC bit, or lane zero for zero EXEC; the access itself ignores EXEC. |
| READLANE / WRITELANE | Select low five lane bits in Wave32 or six in Wave64; ignore EXEC for the lane access. Scalar and vector register banks differ by operation. |
| DS_BPERMUTE | Address selects a source lane; source EXEC affects the fetched value and destination EXEC affects the write. Pending-result rules need their own contract. |
| DS_PERMUTE | Scatter active source lanes to selected destinations; apply destination EXEC separately. The guide's pseudocode and prose disagree on whether duplicate writers deterministically select the highest lane. Resolve or explicitly bound that case before acceptance. |
| DS_SWIZZLE | Cover basic quad/bitmask modes, rotate and FFT modes separately; validate invalid source-lane behavior. |
| PERMLANE16 / PERMLANEX16 | Distinguish same-row from adjacent-row gathers; OP_SEL bits encode fetch-inactive and bound control rather than ordinary arithmetic selection. |

A partial dispatch does not supply arbitrary absent-lane register contents.
Synthetic input bounds and guest register validity are separate requirements;
no implementation may turn an absent-lane read into an unchecked input access.
This audit records required semantics, not completed implementations or native
test evidence. Special sources and graphics applicability remain open.

### Architecture required for full S3.4 acceptance

The current `compile(Architecture, bytes)` entry point implies Wave32. Wave64
requires explicit options or validated external metadata; changing constants
from 32 to 64 is insufficient because some DS routes remain within 32-lane
halves. The legacy entry point can remain a Wave32 wrapper.

Lane-read results are runtime, wave-uniform scalar values. Their uses require
IR representation, source-version capture and stale-state invalidation across
scalar writes. Pure operand recomputation may support immutable input loads;
it must not replay shared-memory writes, atomics or other side effects.

Graphics acceptance additionally needs guest wave membership, quad topology,
coverage versus execution masks and helper-lane side-effect suppression. The
existing graphics function signatures do not provide that state. These are
architectural prerequisites, not missing opcode-table entries. Both independent
read-only reviews identified these requirements; neither review certifies new
implementation or hardware results.

### Bounded lane-read increment

The first lane-read increment admits VOP1 READFIRSTLANE and VOP3 READLANE
only when the selected logical Wave32 lane is provably zero. This lane exists
in every nonempty partial wave. READFIRSTLANE therefore admits EXEC zero or
EXEC with bit zero set. READLANE takes an ordinary, compile-time-known SGPR
selector whose low five bits are zero; it ignores EXEC. Other selected lanes,
inline selector encodings, LDS-direct access and VOP3 READFIRSTLANE remain
outside this increment.

Sources are initialized ordinary VGPRs; destination SGPRs are 8–105. Resource
slots and special scalar registers are excluded. The result is runtime and
wave-uniform, with immutable source-version capture and vector-move consumption.
Scalar arithmetic and other unsupported consumers reject the value rather than
using a stale constant. The existing scalar-write paths invalidate previous
lane-derived state. Graphics and lane-read instructions inside runtime branch
regions remain rejected. Native evidence is recorded in the
[checkpoint](../testing/stage3-control-memory-opening.md#lane-zero-read-continuation);
this increment does not complete S3.4.
