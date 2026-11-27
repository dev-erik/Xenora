# Guest-backed Image Sources Implementation Plan

**Goal:** Extend submission-time guest-buffer snapshots to image transfers without changing destination publication.
**Architecture:** Preserve the existing owned snapshot and native copy interfaces. Admit guest-backed buffers only on source paths; validate the full logical payload budget before reading guest memory.
**Tech Stack:** Rust, checked GuestMemory, native ARM64 Metal, original synthetic guest fixtures.

## Contract and design

Existing opcode49 buffers become sources for image operations44/50/54. Source
locations are pitched buffers; destinations remain owned texture backings. Recording
validates metadata only. Submission reads the entire logical source, including padding
and uncopied bytes, into an immutable snapshot. Pre-submit writes are visible;
post-submit writes, unmapping and destruction cannot alter admitted work. Read-only
source mappings suffice. Mapping/permission faults and admission failures leave queue
points and owned destination publication unchanged; retry captures fresh bytes.

Use the existing format, pitch, block, mip/slice, budget and capability validators.
Typed transfers keep runtime format gating, exact unsigned/depth bits and BC layout
rules. Destination-prior versions remain exact and retain untouched subresources.
Do not enable guest destinations, guest-backed texture creation or implicit shared
coherence. These remain S2.6 follow-up milestones, including atomic destination
publication with permission/remapping/lifetime rules and integration acceptance.

The alternatives were a new staging-buffer API or implicit coherent backing. Reusing
explicit submission snapshots avoids a new ABI and preserves the established ownership
boundary. The broader alternatives need separate mapping/versioning contracts.

## Steps

- [x] Independent CPU and original guest tests in loader/tests/guest_image_sources.rs and ffi/tests/guest_graphics/guest_image_sources.rs: offsets/pitch, full-source faults, retries, mutation after submit, produced destination and typed/mip/array preservation. The private image-budget regression observed baseline rejection before production changes.
- [x] In graphics.rs allow guest source layouts for image admission and record_image_copy, preserving destination=false and full-budget-before-input ordering.
- [x] In graphics_typed_copy.rs add a source-aware wrapper that delegates guest buffers to copy_layout_with_guest_source(h,true); all other resources use typed_copy_layout. Use it only for source recording/submission. Keep commit/destination validation unchanged.
- [x] Update the old negative image-source regressions to accept sources while retaining destination rejection; reset the guest list before its separate invalid-barrier scenario. Add a private budget-before-read test for the image path.
- [x] Run focused CPU/guest tests with four Cargo jobs and one GPU workload. Independently review the production diff and literal expectations. Exercise decisive snapshot/budget faults with exact restoration.
- [x] Update ABI/interfaces/changelog, register focused validation, run nine gates, signed-off commit and push to fork. Report this increment and remaining S2.6 scope explicitly.

Commands: CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 cargo test --locked -p xenora-loader --test guest_image_sources; the corresponding xenora-ffi --features gpu-tests --test guest_graphics original_guest_image_sources_ runs serially under Metal validation. Full acceptance uses scripts/validate-graphics.sh full.
