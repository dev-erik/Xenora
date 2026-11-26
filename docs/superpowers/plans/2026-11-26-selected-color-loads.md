# Selected color-view Load implementation plan

**Goal:** Render from the exact GPU-produced contents of a selected mip/slice and compose a full-backing update without reading pending data through the CPU.

**Architecture:** Add a queued typed color Load from an immutable one-mip view. Normalize its selected native texture into the existing color-source path while retaining the original producer point and complete backing lease. Render a fresh selected surface; an explicit existing typed-copy submission patches that surface into a fresh full-backing version. Each point keeps existing admission, failure and collection rules. This avoids an unvalidated compound multi-submission transaction.

**Tech stack:** Rust, native ARM64 Metal, existing RenderCohort typed render/copy APIs.

## Design decisions

Direct rendering into a copied full backing would require generalizing render outputs/readbacks and the loader publication model together. A compound wrapper around multiple submissions would need atomic admission and rollback not currently provided. The selected Load operation composes the two already-defined immutable operations explicitly; callers retain and collect each point. It is a prerequisite for guest writable views, not their completion.

Only a one-mip, linear view whose native format and extent match a supported color attachment is accepted. Depth/compressed color roles, sRGB conversion, foreign cohorts, stale generations, incorrect extents and multi-mip ranges reject. There is no public mutable/raw texture access. Inputs stay immutable, including after failure or replacement.

## Implementation and validation

- [x] Extend QueuedColorLoad in crates/metal/src/lib.rs with ProducedTypedView(RenderTypedTextureView); first return unsupported from render_cohort.rs to capture a meaningful red test.
- [x] In render_cohort.rs validate point, selected mip count, exact native/storage format, device and extent before allocations. Retain the view's root point/lease in existing color_sources; only internal selected texture metadata becomes ordinary one-mip2D.
- [x] Add only the crate-private native texture accessor needed in texture_pipeline.rs. Preserve public encapsulation and upload rejection.
- [x] Independently test seven color formats with literal bytes, partial writes, immutable roots and complete-backing copy-back. Include gated producer/automatic dependency and descendant-first collection, rejection/recovery and retained budget evidence.
- [x] Deliberately omit selected Load or producer dependency; require assertion detection and restore exact source.
- [ ] Run native focused tests, all nine gates and the updated quick script. Obtain two independent reviews before signed-off commit/fork publication.

## Remaining Stage2.4 scope

Guest writable color/depth attachment views, shader-indexed array ranges, range-aware read/write conflicts and the writable shader-storage dependency remain open. This native primitive does not close Stage2.4; Stage2.5 begins only after the agreed Stage2.4 scope is accepted.
