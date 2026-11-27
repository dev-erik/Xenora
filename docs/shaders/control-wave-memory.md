# Control flow, wave operations and memory: opening scope

Reviewed 27 November 2026. These workstreams are open. The inventory is desktop gfx1030; it does not establish Xbox binary compatibility.

The earlier 7/8/12 totals excluded non-arithmetic families. The [generated checklist](shader-checklist.md) now assigns SOPP branches, DS lane operations and memory families explicitly. Reclassification is not implementation progress. State operations and cross-cutting operand modes still need separate closure.

## S3.2 control flow

First increment: SCC1 complements the existing SCC0 condition. Indirect calls/returns, general loops, traps and full divergent control flow remain beyond this increment. Branch-target validation, undefined SCC and countdown proofs must remain intact.

## S3.4 wave and lane

First candidate: MBCNT LO/HI under an explicit Wave32 compute contract. LO counts selected bits below the guest lane and adds the accumulator; HI adds no bits for lanes below 32. This candidate is not implemented by this scope document. Validate against AMD RDNA2 semantics before implementation.

| Family | Form | Decoder present at audit |
| --- | --- | --- |
| DS | `DS_BPERMUTE_B32` | no |
| DS | `DS_PERMUTE_B32` | no |
| DS | `DS_SWIZZLE_B32` | no |
| VOP1 | `V_READFIRSTLANE_B32` | no |
| VOP3 | `V_MBCNT_HI_U32_B32` | no |
| VOP3 | `V_MBCNT_LO_U32_B32` | no |
| VOP3 | `V_PERMLANE16_B32` | no |
| VOP3 | `V_PERMLANEX16_B32` | no |
| VOP3 | `V_READFIRSTLANE_B32` | no |
| VOP3 | `V_READLANE_B32` | no |
| VOP3 | `V_WRITELANE_B32` | no |

The proposed first compute contract is Wave32 only, with guest lane `index & 31`:

- LO: `src1 + popcount(src0 & lower_lane_bits)`, wrapping at 32 bits.
- HI: `src1` unchanged for Wave32.
- EXEC controls destination writes, not which SRC0 bits are counted. Inactive destinations retain their prior values.
- Admit ordinary VGPR/SGPR/inline/literal sources under existing scalar/literal limits; reject LDS-direct, unsupported special sources and modifiers. Graphics remains rejected until a separate lane contract exists.

Independent test requirements: lanes 0/1/31; wrapping accumulation; sparse SRC0; destination/source aliasing; partial EXEC with sentinels; lengths 1/31/32/33/63/65; repeated guest-wave boundaries; and both forms in a recomputed comparison-expression cone. The implementation must avoid shifting a 32-bit value by 32. These are proposed acceptance cases, not tests already run.

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

The [first-increment checkpoint](../testing/stage3-control-memory-opening.md) records SCC1 and VMCNT evidence. S3.4 remains unimplemented.

The [opening plan](../superpowers/plans/2026-11-27-stage3-opening.md) tracks the first increments. Require exact encoding checks, independent CPU expectations, applicable native compute/graphics readback and independent review before acceptance. Checkpoint results must identify the tested source version; this document alone records no passing GPU tests.

Public semantic reference: [AMD RDNA2 ISA, document 70648](https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture). The [source notes](../research/rdna2.md) pin the machine-readable inventory and LLVM version.
