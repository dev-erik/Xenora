# Control flow, wave operations and memory: opening scope

Reviewed 27 November 2026. These workstreams are open. The inventory is desktop gfx1030; it does not establish Xbox binary compatibility.

The earlier 7/8/12 totals excluded non-arithmetic families. The [generated checklist](shader-checklist.md) now assigns SOPP branches, DS lane operations and memory families explicitly. Reclassification is not implementation progress. State operations and cross-cutting operand modes still need separate closure.

## S3.2 control flow

First increment: SCC1 complements the existing SCC0 condition. Indirect calls/returns, general loops, traps and full divergent control flow remain beyond this increment. Branch-target validation, undefined SCC and countdown proofs must remain intact.

## S3.4 wave and lane

Current increment: MBCNT LO/HI under an explicit Wave32 compute contract. LO counts selected bits below the guest lane and adds the accumulator; HI adds no bits for lanes below 32. Typed decoding, compute lowering and an independent CPU reference are implemented. Native validation is recorded in the checkpoint; this does not complete wave/lane support.

| Family | Form | Decoder present at audit |
| --- | --- | --- |
| DS | `DS_BPERMUTE_B32` | no |
| DS | `DS_PERMUTE_B32` | no |
| DS | `DS_SWIZZLE_B32` | no |
| VOP1 | `V_READFIRSTLANE_B32` | no |
| VOP3 | `V_MBCNT_HI_U32_B32` | yes; Wave32 compute subset |
| VOP3 | `V_MBCNT_LO_U32_B32` | yes; Wave32 compute subset |
| VOP3 | `V_PERMLANE16_B32` | no |
| VOP3 | `V_PERMLANEX16_B32` | no |
| VOP3 | `V_READFIRSTLANE_B32` | no |
| VOP3 | `V_READLANE_B32` | no |
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

First increment: `S_WAITCNT_VMCNT null,0` under the existing synthetic compute pending-load contract. A load wait must not discharge pending stores. Unknown/nonzero thresholds and other scalar operands remain rejected; graphics admission is separate.

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

The [first-increment checkpoint](../testing/stage3-control-memory-opening.md) records SCC1 and VMCNT evidence. S3.4 has the bounded masked-bit-count increment; the other nine forms remain undecoded.

The [opening plan](../superpowers/plans/2026-11-27-stage3-opening.md) tracks the first increments. Require exact encoding checks, independent CPU expectations, applicable native compute/graphics readback and independent review before acceptance. Checkpoint results must identify the tested source version; this document alone records no passing GPU tests.

Public semantic reference: [AMD RDNA2 ISA, document 70648](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture). The [source notes](../research/rdna2.md) pin the machine-readable inventory and LLVM version.

## Remaining closure work

These are dependency groups, not equal-sized tasks or a complete operand matrix.

| Stage | Next bounded work | Architectural work still required |
| --- | --- | --- |
| S3.2 | Complementary EXECNZ/VCCNZ branches; additional proven loop shapes | Compute loops and live-out joins; code addresses and calls/returns; exceptions; dynamic EXEC and reconvergence |
| S3.4 | Explicit contracts for the remaining nine lane forms | Inactive/absent source lanes; cross-lane data movement; graphics/helper lanes; Wave64; DPP/SDWA mode coverage |
| S3.5 | More buffer transfer forms with explicit addressing and destination spans | Multiple outstanding transfers; descriptors and address spaces; scalar/private/LDS memory; atomics and rendezvous; image/stage-interface matrices |

Each accepted increment needs exact source and destination modes, negative cases,
independent expected outputs, applicable native tests and documented limits.
Wait-counter thresholds cannot be accepted before the corresponding event and
dependency states are modeled. The full-stage acceptance criteria remain open.

### Next bounded control and memory contracts

Planned control increment: `S_CBRANCH_EXECNZ` and `S_CBRANCH_VCCNZ` test the
nonzero low mask under Wave32. Preserve full-stream target validation and
forward-only compute admission. Lane-derived VCC may skip only the existing
validated integer-vector region; joined values must select pre-branch state
when the branch is taken. Unknown VCC, nested runtime regions, memory operations
inside a region and graphics use remain rejected. These complements do not
add dynamic EXEC or general loops.

Planned memory increment: signed and unsigned byte/halfword buffer loads under
the existing synthetic four-byte input stride. Each invocation reads the low
8 or 16 bits of its own little-endian input word; signed forms extend to 32 bits,
unsigned forms zero-extend. This preserves the current two-buffer ABI and input
bounds, single pending load, full EXEC, immutable index, zero offsets and input
resource restrictions. It does not decode hardware resource descriptors or
permit cross-lane addressing. Graphics and other memory modes remain rejected.
The opcode meanings are specified by AMD RDNA2 §12.14; the stride and restricted
bindings are the project ABI, not an inferred Xbox descriptor layout.
