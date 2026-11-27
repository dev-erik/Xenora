# S3.2, S3.4 and S3.5 implementation boundaries

Date: 27 November 2026. Status: implementation starting, no stage accepted.

Goal: extend the original gfx1030-to-native-Metal path across control flow, wave/lane operations and memory/interfaces. This is not an Xbox ABI completion claim.

The old 7/8/12-form counts were provisional arithmetic-family buckets. SOPP branches, DS permutations and memory families were excluded. Stage closure must include those forms and operand/semantic requirements. The generated inventory will be corrected without claiming implementation progress from reclassification.

Architecture: retain the typed decoder, verified compute/graphics lowering and bounded original fixture ABI. Unknown states, unsupported cycles, modes and interfaces remain explicit errors. No fallback no-ops or invented synchronization semantics.

Alternatives: adding only opcode recognition would inflate coverage without execution; a general wave interpreter would require an architectural expansion. Use incremental native lowering with independent expected results and exact admission contracts.

## S3.2 first slice

Add S_CBRANCH_SCC1 as the complementary condition to existing S_CBRANCH_SCC0. Preserve signed relative target arithmetic, full-stream instruction-boundary validation, undefined-SCC rejection, and existing compute/graphics cycle restrictions. Graphics CFG analysis and merging must use the same predicate direction as execution. Test taken and untaken paths, target errors, undefined SCC, and native output. Do not claim indirect calls, returns, general loops or divergent execution.

## S3.4 opening audit

Enumerate READLANE/WRITELANE/READFIRSTLANE, MBCNT, PERMLANE and DS permutations. Establish guest Wave32/Wave64, EXEC participation, partial waves and graphics-stage rules before native lowering. Host SIMD width is not assumed to equal guest wave width. DPP/SDWA operand modes are additional debt, not independent inventory forms.

## S3.5 opening audit

Include buffer, typed buffer, scalar/global/flat/scratch/LDS memory, atomics, images, exports/interpolation, counters/barriers and cache controls. Resource addresses, out-of-bounds behavior, ordering and lifetime require explicit contracts. Existing synthetic resources do not implement arbitrary Xbox descriptors. Select a bounded first slice after audit; do not map waits/barriers to no-ops without a verified synchronous contract.

## Acceptance

Each increment needs independent primary-source expectations, CPU decoder/negative tests, applicable native Metal execution, reviewed status evidence and two independent reviewers. Updates to the graph must preserve accepted-versus-decoded distinctions. Full stage closure additionally requires an audited feature/operand denominator, not just all assigned mnemonic rows.
