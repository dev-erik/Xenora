# Local graphics checkpoint

This summary records the accumulated graphics checkpoint and the subsequent
nested-view, compute-cache, sampler-handle, guest-copy, R8, mip/slice, array-view and typed-format increments on 25 November 2026. It is not release certification or a
claim of Xbox hardware equivalence. Supporting raw logs, mutation records and
source manifests remain local and are not distributed with the repository.
The array-range descriptor increment below was validated on 26 November 2026.

## One-command validation

From the repository root, run `./scripts/validate-graphics.sh` for the focused
checks, or `./scripts/validate-graphics.sh full` for all nine repository gates.
The script enables Metal validation, runs GPU tests serially, stops on the first
failure, and saves each step's output under `artifacts/validation-*`. A successful
run ends with `PASS`. Quick mode also rejects filters that select no passing tests.
Use `--help` for prerequisites. These tests render offscreen and compare pixels;
they do not open an application window. Avoid other GPU test runs at the same time.

## Native selected-view color Load checkpoint

The native queued renderer accepts an exact GPU-produced, one-mip linear color
view as Load input. It renders into a fresh ordinary surface; an explicit typed-copy
submission can then patch the selected mip/slice into a fresh full backing. This is
not an atomic compound operation, a guest writable-view interface, or S2.4 closure.
See the [native contract](../resource-formats.md#native-selected-view-color-load).

Quick validation passed **172 tests across46 steps**, with zero failures or ignored tests.

Two original trusted-MSL integration tests cover seven color formats, partial writes
and blending where supported, unchanged full-backing bytes, original-root immutability,
zero-draw Load, invalid-copy-back recovery, and rejected multi-mip/sRGB/format/extent/
cohort/generation inputs. Depth and runtime-supported BC views reject as color inputs.
A real GPU gate tests automatic producer ordering and descendant-first collection.
Two private native tests verify complete-root ownership, payload admission/refund
across collection/reset, and rejection of a controlled failed-producer state. The
failure injection follows actual GPU completion and is not a device-fault test.

All nine gates passed: **907 passed, zero failed, 111 ignored** in the workspace;
**409 passed, zero failed, one manual case ignored** in the GPU script. The GPU
script includes overlapping CPU checks, so these totals are not additive unique GPU
tests. Native binaries were verified as ARM64 linked to system Metal on the M4 Max
host, with API and shader validation enabled.

Both counted production mutations compiled and failed decisive assertions: forcing
produced Load to Clear returned zero bytes instead of literal selected contents;
omitting only the selected-view producer dependency triggered the explicit early-GPU
marker assertion. Exact source restoration was verified. An earlier mutation harness
attempt failed compilation on a private-field access and is excluded from the count.
Local logs/manifests use `artifacts/attachment-views-*`. No guest integration or Xbox
hardware equivalence is inferred from these trusted native shader tests.

## Array-range descriptor checkpoint

[Array-range descriptors](../graphics-call-v1.md#array-range-descriptors-and-selected-2d-children)
select and nest layer/mip intervals. Selected 2D children normalize to the root and
remain valid after parent destruction. Ranges retain their backing and share the
existing object budget. They cannot bind directly as shader arrays or attachments;
writable views, shader-selected layers and range-aware write hazards remain open.

Quick validation passed **168 tests across44 steps**, with zero failures or ignored tests.

Six new CPU tests cover canonical offsets, bounds/overflow, one-layer arrays,
ten storage families, parent/root lifetimes, budget recovery and invalid-result-span
atomicity. Two new original guest cases verify literal slice/mip pixels, linear/sRGB
interpretations, updated snapshots across all three adapters, and pending copied
versions with parent/resource retirement in both collection orders.

All nine gates passed: **907 passed, zero failed, 109 ignored** in the workspace,
and **405 passed, zero failed, one manual case ignored** in the GPU script. GPU
script totals include overlapping CPU tests and must not be added as unique GPU
cases. The guest binary was verified as ARM64 with system Metal on the same M4 Max
host; Metal API/shader validation was enabled.

Two deliberate production mutations were detected and restored byte-for-byte:
omitting the relative slice changed the CPU canonical selection from `(3,3,1)` to
`(2,3,1)`; omitting a nested relative mip produced the wrong mip's decoy pixels in
the native guest test. Later formatting and the final gates cover the final source.
These results establish this descriptor increment, not complete resource-view support
or Xbox hardware equivalence. Logs and manifests use `artifacts/array-ranges-*`.

## Typed formats and resource roles checkpoint

The [ten-format matrix](../resource-formats.md) is implemented: exact same-format
mip/slice transfers, typed sampling, seven color storage formats, per-target blending
where supported, and retained Depth32 attachments. BC1/BC3 require runtime capability.
Queued consumers retain exact produced resources and selected views. Completion
validates each attachment's format and extent before publishing any output; only
RGBA8 slot zero publishes the legacy RGBA presentation image. Writable subresource
views, shader-indexed arrays and general format conversion remain outside this matrix.

All nine repository gates passed. Workspace tests reported **901 passed, zero failed,
107 ignored**; the native GPU script reported **403 passed, zero failed, one manual
case ignored**. The GPU script includes CPU checks and overlaps the workspace suite;
these totals must not be added as unique GPU tests. Final quick validation passed
**160 tests across 42 steps**, with zero failures or ignored tests.

Native execution used an Apple M4 Max running macOS 27.2.1. The native and guest
executables were checked as ARM64 and linked to system Metal. Tests cover literal
packed bytes, integer bit preservation through shader joins, normalized/float blend
results, selected array/mip views, retained depth, completion rejection and lifetime
budgets. Original guest cases exercise synchronous, asynchronous and queued adapters.
This establishes local synthetic behavior, not Xbox hardware equivalence.

Six deliberate production mutations triggered decisive assertions: wrong copy
format, wrong buffer offset, numeric conversion of integer sample bits, missing
produced-view wait, clearing a retained Load, and accepting incorrect completion
format metadata. Source restoration was checked after each campaign. The Load
mutation tests preservation, not substitution of one produced version for another.
Subsequent formatting and final gates validate the final source snapshot.

Intermediate failures remain in local logs. The expanded guest test was corrected
to reuse its producer queue within the existing two-queue limit. The blend oracle
was independently re-derived with an input away from a quantization tie; no tolerance
was broadened. Raw evidence is stored locally under `artifacts/formats-roles-*`.

## Selected array-slice view checkpoint

Guest opcode52 creates a read-only 2D view of a selected RGBA8 array slice and mip
range. Existing view bindings and nested mip views accept these handles. Host arrays
are uploaded as complete immutable backings; queued views retain exact GPU-produced
versions. One-layer arrays retain their identity. Ordinary 2D APIs and older backend
installers reject array views explicitly. Writable views, shader-selected array
indexing and independent subresource concurrency remain unsupported.

Quick validation passed **116 tests across 33 steps**, with zero failures or ignored
tests. New coverage includes three loader tests, four native view tests, two native
ownership/failure tests and two original guest tests. Host guest cases exercise all
three adapters, one- and two-layer arrays and changed uploads. Queued cases exercise
both collection orders with poisoned host bytes and exact produced versions.

The native tests compare distinct literal slice/mip pixels, linear and sRGB views,
relative LOD and parent-independent ownership. The budget test retains only a view
after producer collection/reset and checks the complete root charge until final
release. A controlled terminal-state injection after real GPU completion verifies
failure propagation to descendant polling/collection; it does not simulate a native
device fault or establish hardware-fault recovery.

Three deliberate production mutations fail decisive assertions: wrong slice,
wrong native mip level and a missing produced-view dependency. The first two fail
literal pixel comparisons. The third observes a consumer GPU marker before producer
release, rather than relying on a timeout. Restored source hashes match the originals.

All nine repository gates passed: **884 workspace tests passed, zero failed,
100 ignored**; the GPU suite reported **376 passed, zero failed, one manual test
ignored**. GPU-suite totals include CPU checks and overlapping workspace coverage.
Native ARM64 binaries and system Metal linkage were verified on Apple M4 Max with
macOS27.2.1. The first build caught an adapter using a private error constructor;
the fix maps native errors through the existing adapter boundary. Its failure log
is retained separately from the passing run.

Focused commands (use the Metal validation environment below):

```sh
cargo test --locked -p xenora-loader --test array_views
cargo test --locked -p xenora-metal --features gpu-tests --test native_compute array_views:: -- --nocapture --test-threads=1
cargo test --locked -p xenora-metal --features gpu-tests --lib native_array_view_ -- --nocapture --test-threads=1
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_array_views_ -- --nocapture --test-threads=1
```

## Explicit mip and array-slice transfer checkpoint

Guest opcode50 records selected RGBA8 mip/slice transfers; opcode51 creates bounded,
explicitly typed arrays. Texture→texture, buffer→texture and texture→buffer copies
preserve complete destination backings, including untouched levels, slices, borders
and pitched buffer gaps. Arrays remain arrays with one layer. Legacy interfaces retain
single-level restrictions; mip-only backends reject arrays, including slice0.

Produced versions stay on the GPU and retain exact storage kind, extent, mip count
and layer count. Copying an array slice into a 2D mip chain creates a 2D output that
can feed existing sampled views. At that checkpoint, array views/sampling, attachment roles, compressed
or depth transfers, and independent subresource concurrency remain unsupported.
Limits are 256 layers, 4096 subresources and existing complete-backing byte budgets.

The quick runner passed **105 tests across 29 steps**, with no failures or ignored
tests. New coverage consists of three array-layout CPU tests, nine loader tests,
nineteen native transfer tests, two native payload/ownership tests and six original
guest tests. Native tests exercise all directions with independently gated inputs,
full literal readbacks, old-version reuse, one-layer identity and direct sampling
of a 2D GPU copy. Guest tests vary both pending input roles and collection order;
poisoned host bytes distinguish exact pending versions from stale fallback data.

Focused commands (use the Metal validation environment below):

```sh
cargo test --locked -p xenora-texture --test packed_layout array_ -- --nocapture
cargo test --locked -p xenora-loader --test mip_transfers --test array_transfers
cargo test --locked -p xenora-metal --features gpu-tests --test native_compute transfers:: -- --nocapture --test-threads=1
cargo test --locked -p xenora-metal --features gpu-tests --lib native_mip_array_ -- --nocapture --test-threads=1
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics _transfers_ -- --nocapture --test-threads=1
```

Nine intentional production faults were detected: wrong source/destination mips,
wrong untouched-mip cloning, wrong source/destination slices, wrong untouched-slice
cloning, omitted source/prior ownership and omitted GPU waits. The wait fault caused
explicit early GPU markers in all six array direction/input-role cases; watchdog
release was not counted as evidence. Ownership faults failed actual retained-budget
assertions. Byte mutations failed literal readbacks. Sources were restored and their
hashes checked before the complete validation run.

All nine repository gates passed: **881 workspace tests passed, zero failed,
96 ignored**; the GPU suite reported **368 passed, zero failed, one manual test
ignored**. GPU totals include CPU checks and overlapping workspace coverage. The
native transfer, guest and budget binaries are ARM64 linked to system Metal, verified
on Apple M4 Max with macOS27.2.1 and API/shader validation enabled. This is local
synthetic evidence, not Xbox equivalence or game compatibility.

The first loader run exposed a legacy publication guard that rejected mip-chain
completion; the corrected path validates the admitted complete layout. Initial
native test setup incorrectly used the legacy whole-copy API for mip chains and was
corrected. An out-of-range array-test sentinel and the older unknown-opcode50 test
were also corrected; initial failure logs remain available locally. Independent
review strengthened equal-length wrong-layout, full-backing budget and stale-host
oracles before the final run.

## Linear R8 sampled texture checkpoint

R8 textures now keep one-byte texels throughout packed storage, transfers and
native upload. All three production adapters sample actual Metal R8 textures,
including selected and nested read-only mip views. That checkpoint was sampled-only;
R8 attachments, Produced textures and queued texture copies were outside its scope.

Quick validation passed **66 tests across 21 steps**, with zero failures or ignored
tests. Five new CPU checks cover packed odd mip chains, overflow, native upload
preflight, byte budgets, explicit capabilities and unsupported resource roles.
Existing capability/reset and guest service tables now include R8. Three native
checks and two original guest tests cover exact channel values, poisoned padding,
relative LOD, multiple immutable owners, malformed inputs, changed uploads and
retirement. Guest tests exercise synchronous, asynchronous and queued adapters.

Focused commands, with the Metal validation environment below for GPU selections:

```sh
cargo test --locked -p xenora-texture --test packed_layout r8_ -- --nocapture
cargo test --locked -p xenora-metal --lib r8_ -- --nocapture
cargo test --locked -p xenora-loader --test pipeline cpu_r8_ -- --nocapture
cargo test --locked -p xenora-metal --features gpu-tests --test native_compute r8_sampled:: -- --nocapture --test-threads=1
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_r8_ -- --nocapture --test-threads=1
```

The native pixel expectations are literal and independent of production layout
helpers. Red bytes0,17,128,255 must produce `[r,0,0,255]`, following the
[MSL channel-default specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification.pdf).
An intentional upload-pitch mutation introduced padding byte221 into actual pixels
and failed the oracle. The source hash matched after restoration. A review corrected
a guest-test assumption before execution: R8 host creation is allowed without native
support, but drawing on a legacy backend must reject before renderer dispatch.

All nine repository gates passed after restoration: **869 workspace tests passed,
zero failed, 88 ignored**; the GPU suite reported **341 passed, zero failed, one
manual test ignored**. GPU totals include CPU checks and overlapping workspace
coverage. Native binaries are ARM64 linked to system Metal, with API/shader validation
enabled. This completes the linear R8 sampled-texture increment; full Stage2/3 and
R8 render/copy roles remain outside the accepted scope.

## Guest-backed buffer-region copy checkpoint

Opcode43 now accepts guest-backed buffer sources, using the same full-source
submission snapshot as whole copies. The destination remains owned, and image
transfers remain outside this extension. All logical source bytes must be readable,
even outside the selected region; the full padded source staging is budgeted.

The updated quick script passed **56 tests across 16 steps**, zero failures or
ignored tests. Four new CPU tests first failed against the old recording guard,
then passed with the source-policy extension. Two new original guest tests passed
on the native queued adapter. Focused selections are:

```sh
cargo test --locked -p xenora-loader --test queued_copies cpu_guest_region -- --nocapture
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_backed_region_ -- --nocapture --test-threads=1
```

Enable the Metal validation environment below for the guest command, or use the
script. Coverage includes odd and exact-end regions, untouched destination borders,
full-source readability and accounting, retry freshness, alignment restrictions,
post-submit mutation and retirement. A pending produced destination has different
bytes from stale host storage, so the guest oracle distinguishes its prior version.
Both collection orders are tested; this is not a controlled GPU-wait fault test.

An intentional source-offset-zero mutation failed the actual guest readback with
shifted patch bytes. The source hash matched after restoration. CPU completion bytes
are scripted contract checks; native guest tests establish the copy-byte results.

All nine gates passed after restoration: **862 workspace tests passed, zero failed,
86 ignored**; the GPU script reported **334 passed, zero failed, one manual test
ignored**. GPU totals include CPU checks and overlapping workspace coverage.
Architecture/linkage evidence confirms native ARM64 with system Metal; API/shader
validation was enabled. Broader Stage2/3 work remains open.

## Guest-backed whole-buffer copy checkpoint

Queued whole-buffer copies can capture a guest-backed buffer into an owned
destination. The snapshot occurs at submission: earlier guest writes are observed,
and later writes cannot alter pending work. At this checkpoint guest-backed
destinations and region/image copies remained rejected. This does not provide general coherent guest/GPU memory.

Quick validation now runs 14 steps and passed **50 tests, zero failed or ignored**.
The new focused checks account for ten CPU tests and two original guest tests:

```sh
cargo test --locked -p xenora-loader --lib guest_copy_budget_rejection -- --nocapture
cargo test --locked -p xenora-loader --test queued_copies cpu_guest_copy -- --nocapture
cargo test --locked -p xenora-loader --test graphics_service cpu_guest_backed -- --nocapture
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_backed_copy_ -- --nocapture --test-threads=1
```

Use the Metal validation environment below for the guest command, or run the script
to set it automatically. CPU checks cover permission and unreadable-tail faults,
odd logical lengths without padding reads, budget ordering, retry snapshots,
completion rejection and the opcode envelope. Guest tests run the queued adapter,
compare literal bytes after guest writes before/after submission, and exercise
both collection orders, pending handle destruction and rejected-copy preservation.
Their failure case is preparation validation, not an injected GPU/device fault.

A deliberate wrong-source mutation returned zero-filled resource storage instead
of the submitted guest bytes and failed the actual guest readback comparison.
The production source hash matched after restoration. The initial quick run found
a test assertion placed after a separate successful operation; moving it to the
failed-copy observation boundary preserved the intended check and passed on rerun.

All nine repository gates passed after restoration: **858 workspace tests passed,
zero failed, 84 ignored**; the GPU script reported **332 passed, zero failed, one
manual test ignored**. The GPU total includes CPU checks and overlaps workspace
coverage. Native evidence is ARM64 with system Metal and API/shader validation.
Full Stage2/3 and game compatibility remain outside this checkpoint.

## Reusable sampler handles

Sampler handles are immutable descriptors reused across recorded draws. Binding
copies their values, so destroying a handle afterward does not invalidate recorded
or pending work. New binds using destroyed handles reject. The current scope retains
nearest/clamp sampling and existing view-relative LOD validation.

Run the focused CPU selections from the repository root:

```sh
CARGO_BUILD_JOBS=4 cargo test --locked -p xenora-loader --test pipeline cpu_sampler -- --nocapture
CARGO_BUILD_JOBS=4 cargo test --locked -p xenora-loader --test graphics_service cpu_guest_sampler -- --nocapture
```

These selections cover recording snapshots, stale handles, atomic table validation,
LOD checks, guest memory preflight and shared object/command budgets. The focused
run passed all seven (three pipeline, four service). For the original guest pixel
tests, enable the Metal validation environment shown below and run:

```sh
CARGO_BUILD_JOBS=4 cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_sampler_handles -- --nocapture --test-threads=1
```

Both guest tests passed through synchronous, asynchronous and queued adapters.
They check cross-slot sampling, distinct recorded descriptors, handle destruction
before collection, stale-second-handle failure atomicity and view-relative LOD
rejection preserving previous output. A deliberate mutation forcing explicit mip0
failed with incorrect readback pixels; source hashes matched after restoration.
All nine repository gates passed: **848 workspace tests passed, zero failed,82
ignored**; the GPU script reported **330 passed, zero failed, one manual test
ignored**. GPU totals include CPU checks and overlapping coverage. Native guest
execution was ARM64 with system Metal and API/shader validation enabled.
Independent source and literal-oracle reviews found no scoped blocker. Broader
descriptor heaps, filter modes and full Stage2/3 remain open.

## Latest nested-view and compute-cache checkpoint

All nine gates passed after the nested-view extension and selective upstream cache
adaptation: **841 workspace tests passed, zero failed,80 ignored**; the GPU script
reported **328 passed, zero failed, one manual test ignored**. GPU-script totals
include CPU checks and overlap with the workspace run; they are not unique GPU-test
counts. Six focused nested-view CPU tests, two original guest tests across three
adapters, and four native cache tests passed with the applicable Metal validation.

The cache tests first exposed three failures in the uncached implementation. A
separate injected wrong-mip-base fault failed the nested-view assertion (expected
root mip3, got mip1); the original source was restored before the full gates.
Independent source and test reviews found no scoped implementation blocker. Changes
are limited to read-only nested views and bounded compute pipeline reuse; broader
Stage2/3 remain open. No benchmark speedup is claimed.

## Earlier produced-geometry evidence

All nine local gates completed with exit status zero. Independent review checked
the terminal records, summed test-result lines in workspace and GPU logs, and
verified all 16 files in the produced-geometry source manifest against the working
tree. That manifest covers the final increment, not every accumulated change.
The only reported post-gate code change moved rustdoc onto
`queued_submission_capacity`; formatting and diff checks were rerun afterward.

| Suite | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Workspace | 835 | 0 | 78 |
| GPU script | 322 | 0 | 1 |

The GPU-script total includes CPU checks and overlapping coverage. It is not 322
independent GPU tests; do not add these totals as unique coverage. Ignored tests
are not passes. The GPU script's remaining ignored case is manual. Counts describe
this checkpoint and may change as tests are added.

Host records identify ARM64 native test binaries and LLVM fixture tools, with
system Metal and Foundation linkage. Native tests ran with Metal API and shader
validation enabled. This is local Apple Silicon evidence, not verification across
all Apple GPU generations.

For produced geometry, eight native tests and two original PE guest tests passed.
Eight injected fault types were detected after strengthening the poll-before-collect
regression. The initial surviving poll-validation mutation remains in the local
record and is not counted as an initial detection. Faults exercise waits, fetch
ranges and validation, index interpretation, ownership and failure publication.
Mutation sensitivity does not prove absence of defects.

## Run focused graphics checks

Run these commands from the repository root on a Metal-capable Apple Silicon Mac.
They build and execute tests; no application window opens. The rendering tests
create offscreen targets, read back pixels and compare them with independently
specified expected colors or geometry. Negative cases assert rejection and that
previous color/depth results remain intact. An `ok` result means those assertions
passed, not that a person inspected an image on screen.

Enable native validation before running either targeted GPU command:

```sh
export CARGO_BUILD_JOBS=4
export MTL_DEBUG_LAYER=1
export MTL_SHADER_VALIDATION=1
export MTL_SHADER_VALIDATION_REPORT_TO_STDERR=1
export MTL_SHADER_VALIDATION_ABORT_ON_FAULT=1
```

Run the native produced-geometry cases, then the original guest programs through
the production adapter:

```sh
cargo test --locked -p xenora-metal --features gpu-tests --test native_compute produced_geometry:: -- --nocapture --test-threads=1
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_produced_geometry_ -- --nocapture --test-threads=1
```

The reviewed produced-geometry checkpoint reported eight passing native tests and
two passing guest tests for these selections. Expect exit status zero, no failures
and the selected test names marked `ok`; unrelated tests appear as filtered out.
Zero selected tests is not evidence of success: check the names with `-- --list`
if a later revision changes the filters.

CPU-only produced-geometry and nested-view checks can be selected separately:

```sh
cargo test --locked -p xenora-loader --test produced_geometry -- --nocapture
cargo test --locked -p xenora-loader --test pipeline cpu_nested_view -- --nocapture
cargo test --locked -p xenora-loader --test graphics_service cpu_guest_child_view -- --nocapture
```

The nested-view pipeline filter selects checks for relative mip-range
normalization, parent destruction, invalid ranges/formats, superseded references
and sampled/output backing aliases. The separate `cpu_guest_child_view` filter
selects guest ABI preflight, independent lifetime, object-budget and root-format
checks; these names do not contain `nested`. All six focused CPU checks passed
(three per selection). Full-gate results for this increment are recorded above.

Run the original nested-view guest programs with the validation environment above:

```sh
cargo test --locked -p xenora-ffi --features gpu-tests --test guest_graphics original_guest_nested_views -- --nocapture --test-threads=1
```

The local guest log records two passing tests with Metal API and shader validation.
These exercise nonzero mip-base composition, linear/sRGB interpretation, parent
bounds and independent root lifetime across the three adapters. This is targeted
evidence, not full-suite or full-stage acceptance. The nested-loop shader tests
are a different feature and are not substitutes for view tests.

The adapted compute pipeline cache has its own targeted selection (using the GPU
validation environment above):

```sh
cargo test --locked -p xenora-metal --features gpu-tests --test native_compute pipeline_cache:: -- --nocapture --test-threads=1
```

This selects four cache tests. The local targeted run passed all four with Metal
API and shader validation enabled, after a baseline with three failures and one
pass. These results are separate from the earlier checkpoint totals above.
See [selective upstream imports](upstream-imports.md)
for provenance and adaptation scope.

For the full native/guest validation suite, run:

```sh
scripts/test-gpu.sh
```

The script sets validation variables itself. Its aggregate includes CPU checks;
see the checkpoint totals and limitations above. For the complete nine gate
categories, use the commands below.

## Reproducing the gates

Use native ARM64 macOS with Rust, Xcode/Metal tools, LLVM fixture tools and the
script dependencies described in the repository testing guides. These commands
reproduce the gate categories used for the latest checkpoint. Run
GPU work serially, with at most four Cargo build jobs.

```sh
export CARGO_BUILD_JOBS=4
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
scripts/test-gpu.sh
scripts/assemble-fixture.sh --check
python3 scripts/audit/status.py --check
scripts/audit/claims.sh
scripts/audit/dependencies.sh
git diff --check
```

`test-gpu.sh` enables Metal API and shader validation and selects native and guest
checks. Clean formatting and diff checks produce no output. Record command exits
and full test results when reproducing; do not infer success from this summary.

## Acceptance and limits

Independent criterion-level review accepted the original Stage 1 rendering
milestone within its documented contracts: larger indexed draws, per-draw state,
depth, blending and multiple render targets, including integrated synthetic scenes
and actual original guest programs. This reconciles older pending-acceptance
statements for bounded ordered-pass, region-state and lifetime interfaces.

Independent review also accepted the subsequent bounded increments through
GPU-produced geometry. The latter consumes exact pending buffer versions, keeps
geometry payloads on the GPU, checks logical fetch ranges and finite attributes,
retains source and scratch ownership, and rejects invalid passes and dependent
publication. Legacy and host-only paths retain their documented contracts.
See [interfaces](../interfaces.md) and the [guest graphics ABI](../graphics-call-v1.md)
for capability, capacity, layout and validation restrictions.

Full Stage 2 and Stage 3 remain open. General resource/view/format coverage,
broader synchronization, general shader control flow and divergent execution are
not established. Additional geometry formats, instancing and indirect draws are
outside this increment. Original synthetic fixtures do not demonstrate commercial
Xbox game execution, console hardware equivalence or a process-wide memory ceiling.
Payload budgets cover explicitly documented owned allocations rather than total
driver or process memory.
