# Stage 2 integrated resource acceptance

Status: **accepted within the finite S2.1–S2.7 contracts**, 27 November 2026.
Implementation baseline: `472cf98`.

Stage 2 covers the finite resource contracts implemented in S2.1–S2.7. This review
checks their interactions and shared failure/lifetime rules. It does not expand
those contracts or claim commercial game execution, console hardware equivalence,
a complete graphics API, or acceptance of Stage 3 shader work.

## Scope and evidence matrix

The following are executable test sources, not planned demonstrations. Native and
original-guest cases require an actual Metal run; CPU tests separately verify
admission, publication and guest-memory rules. The final run record below determines
which evidence was executed. This matrix samples interactions rather than asserting
an exhaustive Cartesian product of every format, view and queue arrangement.

| Workstream | Accepted finite scope | Cross-workstream evidence |
| --- | --- | --- |
| S2.1 Geometry | GPU-produced float4 attributes, optional colors, R16/R32 indices and guarded gathering | [Produced geometry](../../crates/ffi/tests/guest_graphics.rs): `original_guest_produced_geometry_positions_colors_indices_and_retirement` and `original_guest_produced_geometry_failure_preserves_mrt_depth_and_recovery`; [mixed graph](../../crates/metal/tests/native/resource_graph_acceptance.rs) feeds copied geometry into rendering, storage and sampled outputs. |
| S2.2 Transfers | Explicit mips/slices, typed arrays and complete-backing preservation | [Guest transfers/views](../../crates/ffi/tests/guest_graphics.rs): `original_guest_mip_transfers_*`, `original_guest_array_transfers_*`, `original_guest_array_views_pending_copy_sample_and_retirement`; [mirror transfers](../../crates/ffi/tests/guest_graphics/guest_sync.rs) combine import/export with mips, arrays, uint and depth. |
| S2.3 Formats and roles | The [ten-format matrix](../resource-formats.md), with runtime-gated BC and role-specific restrictions | [Typed transfers](../../crates/metal/tests/native/typed_transfers.rs), including `native_typed_transfers_render_depth_copy_then_load_literal_planes`; [guest roles](../../crates/ffi/tests/guest_graphics.rs) cover sampling, seven color attachments and rendered depth. |
| S2.4 Views | Selected/nested/array views, selected attachment transactions, explicit composition and bounded storage | [Composition](../../crates/ffi/tests/guest_graphics/attachment_composition.rs) checks exact ancestor Load and preserved backing; [selected depth](../../crates/metal/tests/native/depth_attachment_views.rs) checks produced mip/slice Load and copy-back; [storage](../../crates/ffi/tests/guest_graphics/storage_views.rs) and the mixed graph connect writes to later consumers. |
| S2.5 Descriptors | Immutable recorded revisions, four texture/sampler slots and validated runtime indices | [Guest descriptors](../../crates/ffi/tests/guest_graphics/descriptor_tables.rs) combine produced slot3 inputs, both collection orders, selected float/uint arrays and depth sampling; the [guest mixed graph](../../crates/ffi/tests/guest_graphics/resource_graph_acceptance.rs) clears live tables after recording and retires their handles. |
| S2.6 Synchronization | Explicit packed mirror binding and atomic import/export; mapping identities and retained receipt guards | [Guest synchronization](../../crates/ffi/tests/guest_graphics/guest_sync.rs) checks typed partial transfers and 24 reuse cycles; the guest mixed graph blocks export through completion until retained receipts are pruned. [CPU mapping tests](../../crates/loader/tests/guest_sync.rs) separately check remapping, permissions and batch atomicity. |
| S2.7 Graphs | Four queues; sixteen retained logical receipts and sixteen uncollected native stages; unchanged payload ceilings | [Native mixed graph](../../crates/metal/tests/native/resource_graph_acceptance.rs) and [guest mixed graph](../../crates/ffi/tests/guest_graphics/resource_graph_acceptance.rs) connect geometry, attachments, descriptor sampling, storage, copies and mirrors, including capacity rejection and retained versions. [Worker recovery](../../crates/ffi/src/graphics_backend/queued.rs) checks selected-point rejection, generation reset, stale inputs and formerly fifth-queue admission. |

## Integrated acceptance requirements

- [x] Cross-workstream coverage reviewed against the implementation and existing
  contracts; no unresolved defect inside the finite accepted scope.
- [x] Exact content and untouched regions survive pending producer use, ancestor
  collection, recorded descriptor replacement and public-handle retirement.
- [x] Admission failures leave state/points intact; ancestor failure prevents
  descendant publication; retained receipts/backings continue consuming budgets
  until their owning references are released.
- [x] Mirror imports update owned resources only at explicit import boundaries;
  GPU results reach guest mirror memory only through explicit export, including
  repeated reuse. CPU-only permission/remapping evidence is identified separately.
- [x] Fresh full validation passes all nine gates with native GPU tests enabled,
  serial GPU execution and no ignored cases counted as GPU success.
- [x] Native build artifacts and linkage pass the ARM64 audit.
- [x] Two independent reviewers accept the integrated matrix and final evidence;
  authors do not certify their own implementations.

Supporting rejection tests include the loader suites `produced_geometry`,
`typed_transfers`, `typed_roles`, `attachment_composition_boundaries`,
`descriptor_tables`, `guest_sync` and `queued_graphs`. They exercise wrong layouts,
conflicting writers, stale publications, descriptor caps, mapping permissions,
retained ancestors and recovery without replacing native execution evidence.
Earlier focused source-mutation records are summarized in the
[local checkpoint](local-graphics-checkpoint.md); this acceptance does not count
them as fresh mutation runs or retest unchanged mechanisms without a new concern.

## Reproduction and final evidence

Run from the repository root on native Apple Silicon with the documented build
dependencies installed. The tests compare offscreen output; no window opens.

```sh
CARGO_BUILD_JOBS=4 ./scripts/validate-graphics.sh full
CARGO_BUILD_JOBS=4 ./scripts/audit/native.sh
```

27 November 2026, implementation baseline
`472cf98c2e5d2d0b0746ab4567ad2655f716a9b9`; this acceptance changes documentation
only. Executed on Apple M4 Max, native arm64, macOS 27.2.1 (26B131), Rust 1.100.0
(0ef1bc386), LLVM 23.1.6, with four Cargo jobs and serial GPU execution.

Fresh `artifacts/validation-full.ns8hGt` passed all nine gates: format, Clippy,
workspace, GPU, fixtures, status, claims, dependencies and diff. Workspace results:
**1,013 passed, zero failed, 146 ignored**. GPU-script suites: **461 passed, zero
failed, one ignored**. The suites overlap and include CPU checks; these are not
unique GPU-test totals. Ignored tests do not establish execution. Metal API and
shader validation were enabled.

The passing `native_persistent_bc1_odd_pitched_mips_views_and_owner_drop` test
unconditionally asserts BC hardware capability. This establishes capability on
this host, so the capability guards in the typed BC matrix do not skip BC1/BC3.
Other hosts must still check support.

The fresh native audit passed: **116 executables/loadable artifacts** were ARM64,
with no Vulkan/MoltenVK linkage; execution was not translated. Evidence is
`artifacts/s2-integrated-native-audit.log`. Two independent LOW-effort reviewers (R1 and R2) accepted the final matrix and
fresh evidence. Both independently recounted results, checked all 13 evidence
hashes and confirmed no production diff against the baseline. The local manifest
is `artifacts/s2-integrated-evidence.json`; review conclusions are retained in
`artifacts/s2-integrated-review.md`.
Full raw logs remain local
under `artifacts/`; the committed record supplies reproducible commands and scope.
Historical mutations retain their original baselines in the
[checkpoint](local-graphics-checkpoint.md), including S2.5 descriptor, S2.6 mirror
and S2.7 graph faults. They are sensitivity evidence for those revisions, not fresh
mutation results or certification of every later source change.

## Limits retained at acceptance

- Evidence applies to the executed Apple Silicon host. Runtime capability checks
  remain required on other devices; neither all-adapter nor all-format-combination
  validation is inferred. BC skips, if any, must be reported in the run record.
- Geometry formats remain float4/R16/R32. Format roles follow the ten-format matrix;
  depth arrays for shader sampling, compressed attachments and arbitrary format
  conversion remain unsupported.
- Writable views compose immutable full-root versions. Hazards remain conservative
  at whole-root granularity; independent subresource execution is not promised.
- Shader storage keeps its one-triangle, unique pixel-address restrictions.
  Descriptor indices are bounded and validated; unrestricted heaps and divergent
  descriptor indexing are outside Stage 2.
- Guest coherence is explicit. No asynchronous callback writes into guest memory.
  Twenty-four transfer reuse cycles are not rendered frames or a long-running game.
- Graphs wait on previously admitted points, not future signals. Attachment
  transactions can hit the native-stage limit before the logical receipt limit.
  Recovery requires all retained receipts pruned, including successful ones.
- Synthetic GPU geometry validation and controlled worker rejection are not evidence
  of hardware-fault injection. Console execution, redistribution approval, broader
  shaders, performance tuning and application compatibility remain separate work.
