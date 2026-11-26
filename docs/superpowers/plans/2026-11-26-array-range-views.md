# Array-range views implementation plan

> Execution uses the subagent-driven-development workflow with independent tests and reviews.

**Goal:** Add guest-visible immutable array-range descriptors and nested, selected 2D children without weakening ownership or bounds.

**Architecture:** Range descriptors normalize directly to a root resource plus absolute slice/mip intervals. Selecting a slice reuses the accepted 2D view path; no new native texture representation or shader indexing is claimed. A live range retains its root, while descendants own their own root references and survive parent destruction.

**Tech stack:** Rust loader, GraphicsCallV1 synthetic guest ABI, native ARM64 Metal validation.

## Scope and implementation

- [x] Add GraphicsArrayRangeViewDesc and independent range handle storage in crates/loader/src/graphics.rs (or graphics_array_ranges.rs); count range objects in the global cap and reject backing destruction while any live range exists.
- [x] Validate nonzero mip/layer counts with checked addition, exact storage family, explicit array identity and valid root before allocating a handle. Compose child offsets relative to parent and retain only root identity. A range handle cannot be bound as a texture2D view.
- [x] Expose opcodes57 create range,58 nested range,59 select 2D slice,60 destroy range in crates/loader/src/graphics_service.rs. Validate integer narrowing, descriptor tails and result slots before mutation. Unknown opcode becomes61 in tests.
- [x] Independently test bounds, nested offsets, parent/root lifetime, budgets and service rejection in crates/loader/tests/array_ranges.rs.
- [x] Exercise range-derived 2D children using original guest programs across all three adapters, distinct mip/slice literal pixels, sRGB/linear interpretations, changed snapshots and pending copied versions in both collection orders. Keep original slice tests and add separate range entry points.
- [x] Run focused loader and guest tests; perform a wrong-relative-offset mutation and restore exact source. Run all nine gates and quick validation, with one GPU workload and four Cargo jobs maximum.
- [ ] Independent scope/evidence and semantic review; document actual outcomes, commit with DCO sign-off and publish to the authorized fork.

## Remaining S2.4 work

This increment is not S2.4 closure. Shader-indexed array-range bindings, writable color/depth subresource views preserving untouched backing, range-aware read/write conflicts, and writable shader storage with its Stage3 instruction dependency remain open. Later work must validate exact produced-version composition and integrated guest operation, not only metadata.
