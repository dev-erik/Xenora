# Xenora documentation

## Getting started

Start with the [project README](../README.md) for scope and build commands,
then read the [interface contracts](interfaces.md). Reproduce an
[original guest test program](../experiments/x1/README.md) or run the
[independent validation suite](../tests/validation/README.md) before changing
either path. The [changelog](../CHANGELOG.md) records notable changes.

## Architecture and technical design

- [Original guest graphics calls](graphics-call-v1.md) — Versioned synthetic resource, pipeline, transfer and command descriptors.
- [Interface contracts](interfaces.md) — Translated shader stages, guest services, native command submission, and synthetic textures.
- [Fixture memory model](adr/0003-demo-memory-model.md) and [CPU boundary](adr/0004-cpu-boundary.md) — Buffer bindings, guest address translation, and dependency direction.

## Native Metal graphics

- [Bounded resource dependencies](interfaces.md#bounded-resource-dependency-contract) — Version selection, failure/lifetime rules and [acceptance checklist](superpowers/plans/2026-11-27-resource-graph-acceptance.md).

- [Four-queue resource graphs](interfaces.md#four-queue-graph-increment) — Expanded opt-in queue admission, recovery and [increment checklist](superpowers/plans/2026-11-27-four-queue-graphs.md).

- [Guest resource synchronization](graphics-call-v1.md#explicit-guest-resource-synchronization) — Packed texture/buffer bindings and atomic import/export; [completion plan](superpowers/plans/2026-11-27-guest-synchronization.md).

- [Guest-backed image sources](graphics-call-v1.md#guest-backed-buffer-image-sources) — Checked submission-time buffer snapshots for pitched, mip/array and typed transfers; [implementation plan](superpowers/plans/2026-11-27-guest-image-sources.md).

- [Descriptor tables and runtime indexing](descriptor-tables.md) — Immutable recorded revisions, four-slot bindings, selector rules and validation commands.

- [Selected shader storage](graphics-call-v1.md#bounded-shader-storage-views) — Ordered pixel-address stores, exact prior contents and atomic full-root publication.

- [Writable attachment transactions](graphics-call-v1.md#writable-attachment-views-and-complete-root-transactions) — Selected color/depth rendering, complete-root publication and admission limits.

- [Array-range descriptors](graphics-call-v1.md#array-range-descriptors-and-selected-2d-children) — Nested layer/mip ranges and independently retained 2D children; [implementation plan](superpowers/plans/2026-11-26-array-range-views.md).

- [Typed resource formats and roles](resource-formats.md) — Finite format matrix, native selected-view color Load, representation rules and exclusions.
- [Selected color-view Load plan](superpowers/plans/2026-11-26-selected-color-loads.md) — Immutable render/copy composition and remaining writable-view integration.

- [Multi-node queued graphs](interfaces.md#opt-in-multi-node-queued-graphs) — Bounded transitive dependencies, exact resource versions and legacy backend limits.

- [Translated offscreen stages](interfaces.md#translated-offscreen-stages) — Translated vertex and pixel stages for the bounded gfx1030 subset.
- [Blend and depth passes](interfaces.md) — Native blend and depth graphics passes with per-draw state.
- [Apple GPU evidence](research/apple-gpu.md) and [host lab record](research/host-lab.md) — Observed behavior and historical floating-point results.

## RDNA 2 decoding and shader translation

- [GPU-produced geometry](interfaces.md#gpu-produced-geometry-in-queued-draws) — Exact queued buffer versions, guarded GPU gathering and atomic validation failure.
- [Bounded graphics countdowns](interfaces.md#bounded-countdown-loops-in-original-graphics-stages) — Original unsigned and signed-positive constant/MIN-bounded nested loops and early exits, typed carried values, native evidence and explicit admission limits.
- [Instruction coverage](shaders/instruction-coverage.md) — Desktop gfx1030 instruction forms, operand limits, and CPU and GPU results.
- [Semantic coverage](shaders/semantic-coverage.md) — Generated per-family counts of identified, declared, decoded, lowered and verified gfx1030 forms.
- [SOP2 scalar ALU](shaders/scalar-alu.md) — Supported scalar arithmetic, bitwise and half-word packing forms, their SCC rules and sources.
- [Semantic registry](shaders/semantic-registry.md) — How to add an instruction form: registry fields, validation, generation and the independence rule.
- [Integer vector unary instructions](shaders/vector-unary.md) — Bit operations, scan sentinels, sources and preserved execution state.
- [VOPC integer compares](shaders/vector-compare.md) — Wave32 compare forms, the VCC/EXEC contract, runtime `s_cbranch_vccz` and the Metal mapping.
- [Metal Shading Language generation](shaders/msl.md) and [independent CPU reference](shaders/reference-interpreter.md) — Translation rules and separate CPU execution models.

## Guest CPU and runtime experiments

- [Guest experiment guide](../experiments/x1/README.md) — Bounded x86-64 execution, executable loading, experimental services, native compute, and offscreen rendering.
- [x86-64 instruction registry](cpu/instruction-registry.md) — Shared decoder, generated tables, semantics, native lowering and how to add a verified instruction form.
- [Runtime service registry](runtime/registry.md) and [generated coverage](runtime/services.md) — Import resolution, typed Win64 adapters and how to add a tested service.
- [Guest CPU research](research/cpu-boundary.md) — Public specifications and unresolved console requirements.
- [Synthetic import fixtures](../tests/compat_fixtures/README.md) — Reproducible PE32+ builds and expected service behavior.

## Testing, validation, and benchmarks

- [Engineering progress](progress.md) — Verification commands, metric definitions, and open release gates.
- [Integrated Stage 2 acceptance](testing/stage2-integrated-acceptance.md) — Seven resource contracts, cross-workstream evidence and final acceptance limits.
- [Local graphics checkpoint](testing/local-graphics-checkpoint.md) — Bounded acceptance, reproducible gates, local results and evidence limitations.
- [Selective upstream imports](testing/upstream-imports.md) — Source commits, licensing, adaptations and deferred changes.
- [Test inventory comparison](testing/test-inventory.md) — Detect missing or newly ignored Cargo tests across commits.
- [Persistent resource and translated rendering acceptance](testing/x9-acceptance.md) — Pinned upstream tests, native ARM64 results and unsupported cases.
- [Baseline acceptance](testing/phase-02-baseline.md), [Phase 01 assurance](testing/phase01-assurance.md), and [independent reviews](testing/review-log.md) — Candidate-specific results and release findings.
- [Mutation testing](testing/mutation.md) and [benchmark protocol](testing/benchmarks.md) — Test sensitivity, benchmark inputs, and timing requirements.

## Research and engineering decisions

- [Public sources](research/sources.md), [RDNA 2 notes](research/rdna2.md), and [Xbox GPU evidence](research/xbox-gpu.md) — Specifications, evidence scope, and unknown console behavior.
- [Shader inputs](research/shader-containers.md) and [risk register](risk-register.md) — Input provenance and unresolved engineering risks.
- [Synthetic integration and upstream evidence](research/synthetic-integration.md) — Audited source snapshots and remaining commercial startup prerequisites.

## Security, licensing, and contributions

- [Threat model](threat-model.md) and [security reporting](../SECURITY.md) — Execution limits and disclosure instructions.
- [Legal scope](legal.md), [dependency licensing](licenses.md), and [third-party notices](../THIRD_PARTY_NOTICES.md) — Input policy, attribution, and redistribution findings.
- [Contribution guidelines](../CONTRIBUTING.md) — Original authorship, Developer Certificate of Origin sign-off, and evidence requirements.
- [Agent task queue](coordination/agent-tasks.md) — Areas, claiming, leases, upstream sync, verification and the security model for parallel contributors and coding agents.
- [CPU/runtime expansion campaign ledger](coordination/cpu-campaign-ledger.md) — Progress ledger for the Xenora native CPU and runtime compatibility campaign: starting/current commits, completed batches, new forms, verification evidence and the next implementation target.
- [Infrastructure QA ownership](coordination/infrastructure-qa-ownership.md) — Non-overlapping paths for infrastructure and QA tooling.
- [Agent queue smoke evidence](coordination/agent-queue-smoke.md) — Live Actions results from the disposable smoke repository.
- [Infrastructure QA tooling](testing/infrastructure/README.md) — Advisory orchestration, PR validation, benchmark analysis and release checks.

- [Resource-view completion plan](superpowers/plans/2026-11-26-view-completion.md) — remaining S2.4 integration and acceptance criteria.

Shader-indexed array bindings and their validated layer contract are documented in the
[guest graphics call reference](graphics-call-v1.md#shader-indexed-array-sampling).
