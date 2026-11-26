# S2.5 Descriptor Tables Implementation Plan

**Goal:** Complete the finite S2.5 contract in the linked design with native GPU evidence.
**Architecture:** Immutable recorded table revisions, an additive four-slot compiler
and queued binding representation, and shared native validation/ownership.
**Tech Stack:** Rust, public AMD RDNA2 encodings, native ARM64 Metal, original fixtures.

[Design](../specs/2026-11-27-descriptor-tables-design.md)

- [x] Compiler: additive typed4 API, legacy adapters, fixed and uniform-selected bindings,
  strict provenance/type validation and meaningful compiler/decoder regression tests.
- [x] Native: four-slot normalization/cleanup, exact waits, leases, storage alias rejection;
  independent literal pixels, gated producers and lifetime/budget tests.
- [x] Loader: isolated graphics_descriptors module, immutable revision ledger, atomic guest
  operations70–74, record/pending root retention, capability reset, state/range/budget checks.
- [x] FFI: four-slot metadata validation, exact typed views/uploads, full sampler cross-pairs,
  selector checks and storage feedback validation before admission.
- [x] Integration: independent CPU and original guest tests for four slots, actual indexing,
  revisions/atomicity/nulls/destruction/churn/produced inputs/typed views and legacy compatibility.
- [x] Evidence: decisive fault mutations with exact source restoration; complete nine gates,
  independent reviews, current docs/changelog, signed-off commit and verified fork push.

Owners: compiler Feynman; loader Heisenberg; native Confucius; independent tests Anscombe;
FFI/integration lead; independent global review Leibniz. Authors do not certify their
own component. Existing LOW agents only. Builds coordinated by lead, four jobs total,
one GPU workload at a time. No upstream claim or frozen PR branch modification.

Validation: all nine gates passed in `artifacts/validation-full.b2j9aG` (987/0/137
workspace; 444/0/1 native suite with overlap). Six independent fault mutations
failed meaningful assertions and were restored exactly. See the
[checkpoint](../../testing/local-graphics-checkpoint.md#s25-completion-descriptor-tables-and-runtime-indexing).
