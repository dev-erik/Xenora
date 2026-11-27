# Local graphics checkpoint

This summary records the accumulated graphics checkpoint and the subsequent
nested-view, compute-cache, sampler-handle, guest-copy, R8, mip/slice, array-view and typed-format increments on 25 November 2026. It is not release certification or a
claim of Xbox hardware equivalence. Supporting raw logs, mutation records and
source manifests remain local and are not distributed with the repository.
The array-range descriptor increment below was validated on 26 November 2026.

## Stage 3.1 increment: scalar half-word packing

27 November 2026. Three SOP2 forms now pack selected 16-bit halves: LL, LH and HH.
They preserve SCC (including unset) and keep the current normal-SGPR, inline-integer
and single-literal restrictions. Compute resolves these scalar operands during
translation; graphics supports both constant and runtime uniform/local operands.
The source contract is [scalar ALU](../shaders/scalar-alu.md#scalar-half-word-packing).

The initial regression failed at the unsupported opcode before implementation
(`artifacts/s31-pack-red.log`). Independent literal cases check both destination
alias directions, equal sources, distinct halves, zero/all-ones/high bits, literals
and incoming SCC states. A later SCC consumer rejects when packing followed an
unset SCC. Both reference interpreters implement these forms independently of the
registry and generated production code.

The focused LLVM oracle passed 887 instruction comparisons across 103 forms.
Generator validation passed 111 tests. Native graphics checks cover all three
forms, both alias directions, constant and runtime operands, both SCC values and
matching/nonmatching result probes (48 draws). Native compute uses 63 case vectors
plus one original assembled chained fixture. Tests run under Metal API and shader
validation. These checks establish synthetic gfx1030 behavior on Apple M4 Max,
not AMD/Xbox hardware equivalence.

Three isolated faults failed runtime assertions: initializing compute SCC,
initializing graphics SCC, and selecting the wrong high half in graphics. Each
source was restored byte-for-byte; restored native graphics passed. Records are
in `artifacts/s31-pack/mutations.json`. They are scoped sensitivity checks, not
an exhaustive mutation score.

All nine gates passed in `artifacts/validation-full.m4o7Ly`: **1,026 workspace
checks passed, zero failed, 147 ignored**; the GPU-script suites reported **463
passed, zero failed, one ignored**. These aggregates overlap and include CPU tests;
ignored cases are not GPU evidence. The native packing compute test executed under
the GPU script despite being ignored by default in the CPU-only workspace run.
The native artifact audit passed for **117 ARM64 executables/loadable artifacts**,
with no Vulkan/MoltenVK linkage. Two independent LOW-effort reviewers accepted
the three-form increment after inspecting source, expectations, mutations and
final evidence. Both verified all 20 log hashes and 24 source/test/fixture/script
hashes in `artifacts/s31-pack/evidence.json`; no findings remain. This increment
adds three forms; S3.1's broader instruction/operand coverage and S3.2–S3.6 remain
open. Decoded-form and declared-form totals do not measure game compatibility.

## Stage 1 revalidation before shader expansion

27 November 2026, implementation baseline `3368468`. The original finite rendering
scope was reviewed again before Stage 3: larger indexed draws, per-draw state and
programs, depth, blending/masks, and one through eight render targets. Fresh tests
passed **154 checks, zero failed, zero ignored** with native Metal API and shader
validation enabled. This is a focused revalidation, not a new full-nine-gate run.

| Evidence | Passing checks | Purpose |
| --- | ---: | --- |
| Native `translated_render::` | 41 | Ordered passes, state reset, MRT, limits, depth and rollback |
| Native `depth::` and `blend::` | 4 | Known-answer color/depth and blend cases |
| CPU `depth_reference` and `blend_reference` | 2 | Independent reference fixture comparisons |
| Loader `render_limits` | 10 | Admission boundaries and rejection before dispatch |
| Loader `graphics_d3d12`, GPU enabled | 8 | Indexed input rejection and actual Metal guest rendering |
| FFI `large_scene`, `integrated_scene`, `pipeline`, `guest_graphics` | 89 | Both index widths, repeated tiles, mixed programs, actual PE guests and integrated recovery |

The log directory is `artifacts/s1-revalidation`; `commands.json` records the six
serial command groups. `loader-rejection.log` records the additional GPU-enabled
`graphics_d3d12` run. Original mutation records remain historical sensitivity
evidence, not new mutation runs. The previous criterion-level review is retained
in `artifacts/stage1-evidence.md` and `artifacts/stage1-integration-local-reviews.md`.
Two independent LOW-effort reviewers accepted the finite S1 scope after inspecting
the sources and fresh logs. R1 checked the initial 146 passing tests; R2 also checked
the eight additional loader rejection/native cases. Neither authored or reran tests
for this review. No unresolved original S1 requirement was found.

The accepted scope does not include instancing, indirect draws, stencil, MSAA or
independent per-attachment blending. These were not original S1 acceptance criteria.
Stage 3 remains a separate shader expansion milestone.

## Integrated Stage 2 acceptance

The [integrated acceptance record](stage2-integrated-acceptance.md) records accepted
S2.1–S2.7 integration on 27 November 2026: all nine gates passed and 116 native
artifacts passed the architecture/linkage audit. Two independent reviewers accepted
the finite scope and evidence. Increment sections below preserve historical
results and limitations; their open-work statements refer to that increment.

## One-command validation

From the repository root, run `./scripts/validate-graphics.sh` for the focused
checks, or `./scripts/validate-graphics.sh full` for all nine repository gates.
The script enables Metal validation, runs GPU tests serially, stops on the first
failure, and saves each step's output under `artifacts/validation-*`. A successful
run ends with `PASS`. Quick mode also rejects filters that select no passing tests.
Use `--help` for prerequisites. These tests render offscreen and compare pixels;
they do not open an application window. Avoid other GPU test runs at the same time.

## S2.7 acceptance: bounded mixed-resource graphs

27 November 2026. The [dependency contract](../interfaces.md#bounded-resource-dependency-contract)
combines the existing four-queue graph with produced geometry, descriptor revisions,
attachment Load, storage writes, transfers and explicit guest mirrors. Logical
receipts and native stages each have a sixteen-entry ceiling; attachment transactions
can consume several native stages. No queue or payload limit was raised.

New focused coverage comprises two native tests, one original guest test, one
worker test and one CPU-only test. The native mixed graph fills sixteen slots,
uses literal full-texture oracles and separately gates geometry and attachment
fan-in producers. It preserves untouched storage pixels and retained exact outputs
across collection. Invalid GPU-produced indices prevent descendant publication;
this is guarded geometry validation, not an injected hardware fault.

The guest program connects sixteen retained receipts across all four queues,
including produced attributes, storage, descriptor sampling, attachment Load and
copies. Native transaction stages are collected at explicit boundaries while
logical receipts remain retained. Clearing descriptor tables after recording must
not change captured revisions. Capacity rejection, released ancestors, resource
retirement, unchanged mirror guards and export only after pruning are checked in
both collection orders. Publication remains monotonic: the program's four render
submissions produce three presentation updates because node6 is collected before
node5; the older result cannot replace the newer presentation.

The worker test calls the same selected-point rejection handler as message dispatch,
after native descendants complete but before their results are delivered. Rejection
of either branch fails all unpublished tickets; an unknown point does nothing.
After draining and releasing entries, a previously rejected fifth queue becomes
the first mapping of a fresh generation. Stale inputs reject and four fresh mappings
produce literal buffers. This exercises worker state directly, not channel dispatch
or a Metal command fault. The CPU test checks malformed ancestor output, failed
publication, retained budget and generation recovery independently of the GPU.

Four isolated source faults compiled and failed runtime checks: lost loader generation
invalidation, retained worker queue mappings, omitted geometry validation and omitted
attachment Load dependencies. The last fault failed a pixel oracle, not the early
queue-marker assertion. Exact source restoration is recorded in
`artifacts/s27-acceptance-mutations`. The first loader-fault attempt also panicked
during cleanup; supplying the controlled result before assertions yielded a clean
assertion failure on rerun. Test expectations were not weakened.

Focused native/guest/worker runs passed with Metal API and shader validation; logs
are `artifacts/s27-*-acceptance.log`. All nine gates passed on restored sources in
`artifacts/validation-full.tN7aM4`: **1,013 workspace tests passed, zero failed,
146 ignored**; native suites reported **461 passed, zero failed, one ignored**.
The suites overlap; ignored tests are not GPU evidence. Executed native/guest test
binaries were verified as ARM64; native dependencies use Apple's Metal framework.

Two independent reviewers accepted the bounded implementation and evidence, with
their full-gate condition satisfied by that run. They independently checked literal
outputs, presentation ordering, gate placement, recovery, mutation failures and
restoration hashes. **S2.7 is complete within the documented finite contract.**
At this increment, final integrated Stage2 acceptance remained separate; it is
now recorded in the integrated acceptance section above.

## S2.7 increment: four-queue resource graphs

27 November 2026. Opt-in graph cohorts now admit four logical/native queues with
independent event timelines and pending tails. Legacy cohorts retain two-queue
admission; the shared sixteen-node and payload ceilings are unchanged. All five
worker mapping paths, including attachment transactions, use the four-queue map.
Queue mappings survive successful collection and reset on generation recovery.

Five native tests and one original guest test passed. Literal buffer results cover
four-queue chains, gated fan-out/fan-in in both wait orders, descendant-first
collection, source destruction, sixteen-node capacity, legacy rejection and
abort/drain/reinitialization with stale-point/output rejection. The guest test
checks fifth-queue rejection before and after drainage, then retries on an existing
mapped queue. It does not establish the rejected queue's eventual first point
across generation recovery. New native coverage exercises buffer/copy paths,
not four-queue attachment rendering.

All nine gates passed in `artifacts/validation-full.kfYuBx`: **1,012 workspace tests
passed, zero failed, 145 ignored**; native suites reported **457 passed, zero
failed, one ignored**. These suites overlap; ignored tests are not GPU evidence.
Three isolated mutations reached assertion failures: removing legacy queue
rejection, raising the node ceiling to17, and rejecting the fourth worker queue.
An independent reviewer checked failures and exact restoration hashes in
`artifacts/s27-four-queue-mutations`. Two independent reviews accepted this
increment with their full-gate condition satisfied by the recorded run.

The [increment checklist](../superpowers/plans/2026-11-27-four-queue-graphs.md)
records scope and validation. Legacy cohorts currently allocate all four physical
queues/events while admitting only the original two. At this increment, S2.7 was
still open for mixed attachment/copy/geometry/descriptor/storage/synchronization graph acceptance,
wider failure and lifetime pressure, and a final dependency contract. Final
integrated Stage2 acceptance remains separate.

## S2.6 completion: explicit guest resource synchronization

27 November 2026. The [synchronization contract](../graphics-call-v1.md#explicit-guest-resource-synchronization)
completes S2.6 through explicit import/export boundaries. Existing buffer and typed
texture creation plus binding creates a packed guest-backed mirror, including mips,
array slices and packed depth. Atomic batches synchronize up to16 disjoint roots.
Imports stage all bytes and publication epochs; exports validate every destination
before writing. Guest mapping replacement requires explicit rebind. Current
permissions and retained submissions are checked before any transfer.

Synchronization guards retain original handles across successful or failed terminal
completion until release/pruning, independently of native input retirement. No
asynchronous callback writes guest memory. A native regression also exposed and
fixed missing depth-barrier publication in legacy buffer-copy completion.

The focused acceptance set is **22 tests**: twelve CPU integration, four mapping
identity, two private publication/remapping and four original native guest tests.
CPU coverage includes all ten packed formats, adjacent regions, clone/remap
invalidation, read-only/write-only permissions, batch atomicity, alias/count/state
rejection and nonqueued async success/failure. Native queued coverage includes
literal partial copies of RGBA8 mips/arrays, uint, depth and odd buffers, guarded
writeback, explicit rebind/destruction, retained receipts and **24 repeated transfer
cycles**. These are copy/synchronization cycles, not rendered game frames. Mapping
fault/remap evidence is CPU-side; native compressed-format synchronization and
all-adapter coverage are not claimed by these new tests.

All nine gates passed in `artifacts/validation-full.6jRepZ`: **1,012 workspace tests
passed, zero failed, 144 ignored**; the native suite reported **451 passed, zero
failed, one ignored**. Suites overlap; ignored tests are not GPU evidence. Logs
remain local. The earlier full run stopped on a fixed-size decoding lint, corrected
before the successful final run.

Four isolated mutations produced assertion failures: accepting stale mappings,
publishing an import before validating later roots, omitting export permission
preflight, and dropping terminal synchronization guards. Exact source restoration
was verified against each manifest's hash. Evidence remains under
`artifacts/s26-sync-mutations`. Independent reviewers accepted production behavior,
memory identities, native literals, CPU expectations and mutation evidence without
authors certifying their own components. Their final-gate condition is satisfied.

The [completed checklist](../superpowers/plans/2026-11-27-guest-synchronization.md)
defines the finite milestone. Opaque native depth requires conversion into a packed
transfer root before export. Automatic shared-memory coherence, concurrent external
writers, unrestricted aliases and a console memory model are not established.
**S2.7 and final integrated Stage2 acceptance remain open.** Historical S2.6
increment sections below describe the limits at their earlier checkpoints.

## S2.6 increment: guest-buffer image sources

27 November 2026. Guest-backed buffers now supply pitched, mip/array and typed
image copies (opcodes44/50/54). Submission snapshots the entire logical source,
including padding and uncopied tails, after full-budget admission. Destinations
remain owned; partial copies retain exact produced destination contents.

Seven focused CPU tests (six integration and one private budget regression) and
three original guest tests cover snapshots, readable ranges, retry/refund,
capability denial, source retirement and preservation of untouched contents.
Mapping/remapping and read-only permission evidence is CPU-side. Native evidence
uses the queued adapter; it does not establish three-adapter coverage. Compressed
format checks cover a single-block mip tail, not multirow compressed pitch.

All nine gates passed in `artifacts/validation-full.TACM0r`: **994 workspace tests
passed, zero failed, 140 ignored**; the native suite reported **447 passed, zero
failed, one ignored**. Suites overlap and ignored tests are not GPU evidence.
An older source-rejection expectation was updated for the new contract; its list
is reset before the separate invalid-barrier atomicity check so that failure
remains independently exercised.

Four isolated mutations failed meaningful assertions: undercharging the logical
source, omitting an unreadable tail, reading the owned shadow instead of guest
bytes, and dropping the produced destination prior. The last caused incorrect
literal native readback. Each mutation was restored exactly; an independent
reviewer checked failures and restoration hashes. Logs remain in
`artifacts/s26-image-mutations`. Two independent reviews accepted the bounded
increment, with their final-gate condition satisfied by the run above.

The [implementation checklist](../superpowers/plans/2026-11-27-guest-image-sources.md)
records this increment. **S2.6 remains open** for guest-backed texture creation,
guest destination publication (permissions, remapping, lifetime and atomicity),
and broader coherence/integrated acceptance. S2.7 is separate. No game execution
or Xbox hardware equivalence is established.

## S2.5 completion: descriptor tables and runtime indexing

27 November 2026. The [descriptor contract](../descriptor-tables.md) adds immutable
recorded revisions, atomic typed updates, four texture/sampler slots and bounded GPU
selection by integer uniforms. Resource contents resolve at submission; exact
produced versions, complete-root ownership and dependency-retained revisions survive
public-handle destruction, command reset and completion until final retirement.

The focused acceptance set has **41 tests**: 16 compiler, 11 loader, seven original
guest, three native binding, two native lifetime and two worker preflight tests.
Literal GPU checks cover independent selectors, one-sided/diagonal selection,
branches, float/uint array layers, depth, A/B/A revisions and pending slot3 inputs.
Array tests cover both layers across all textures, not every texture/mip/layer
combination. Invalid producer identities, layouts and missing uniforms reject before
attachment imports; retries retain the original producer and queue usability.

All nine repository gates passed in `artifacts/validation-full.b2j9aG`:
**987 workspace tests passed, zero failed, 137 ignored**; the native suite reported
**444 passed, zero failed, one ignored**. These suites overlap; totals must not be
added as unique tests. Ignored tests are not GPU evidence. The full GPU script now
explicitly includes descriptor worker preflight. Two earlier runs found stale
negative-test boundaries (unknown opcode70 and five binding pairs), corrected to
opcode75 and seventeen pairs without weakening rejection/depth assertions.

Six isolated faults failed meaningful assertions: missing slot3 wait (consumer GPU
marker ran early), missing full-root lease (272 versus392 bytes after pruning),
upper-slot collapse (wrong literal pixels), retroactive revision replacement (valid
submission rejected), unchecked selector4 (invalid submission accepted), and dropped
receipt revisions (budget refunded before release). Every mutation was restored
exactly; an independent reviewer checked the source hashes and failure reasons.
Logs remain under ignored `artifacts/s25-mutations`.

Independent component and global reviewers accepted the finite contract and evidence;
authors did not certify their own components.

The [completed checklist](../superpowers/plans/2026-11-27-descriptor-tables.md)
defines the bounded milestone. General descriptor heaps, arbitrary/divergent
indexing, broader guest coherence and general queue graphs are not established.
S2.6, S2.7 and final Stage2 integrated acceptance remain open.

## S2.4 completion: selected shader storage

S2.4 is complete within the documented synthetic resource-view contracts. Earlier
checkpoint sections below describe their historical limits; the completed checklist
is in the [view plan](../superpowers/plans/2026-11-26-view-completion.md).

Selected RGBA8/R32Uint storage now uses ordered guest IMAGE_STORE effects, exact
GPU prior contents and atomic complete-root publication. Runtime branches and
bounded loops preserve skipped writes, while one triangle and proven pixel X/Y
addresses give each invocation a distinct pixel. See the [storage contract](../graphics-call-v1.md#bounded-shader-storage-views)
for format, geometry, feedback and instruction restrictions.

All nine gates passed in `artifacts/validation-full.D3FtXT`: workspace **960 passed,
zero failed, 128 ignored**; GPU script **430 passed, zero failed, one manual case
ignored**. Totals overlap and are not unique GPU counts. Focused evidence includes
**3 decoder**, **12 compiler**, **6 loader**, **3 native**, **5 FFI boundary/transaction**
and **3 original guest tests**. Native/guest runs enabled Metal API and GPU validation.
Independent reviewers accepted the compiler/loader and native/FFI components.

The tests distinguish false/true branches, zero/repeated loop writes, integer bits,
non-square pixel coordinates, pending producers, partial coverage and complete-root
preservation. A role-separation test checks semantic prior A against newer base B,
including B's disjoint byte. Both color and storage copy-back failure points drain
without success and recover in a fresh generation. Native ownership tests retain a
72-byte full-root differential and refund all charges; loader tests enforce a
160 MiB + 256-byte reservation and reject a second receipt before dispatch.

Six production mutations failed decisive assertions: omitted store, numeric uint
conversion, wrong copy-back slice, omitted producer wait, omitted prior copy and
omitted root lease. Current source hashes match exact restoration manifests under
`artifacts/storage-*`; restored tests and the final full run passed. Initial logs
retain the fixed FP-pragma placement failure and the validation wrapper error from
an in-flight help-text edit; the unchanged final wrapper completed successfully.

No checklist items remain for this bounded S2.4 milestone. Broader synchronization,
formats/storage instructions and shader semantics remain later work. Synthetic
validation does not establish Xbox hardware equivalence or commercial game support.

## Shader-indexed array sampling checkpoint

Shader arrays now retain their type and complete backing through decoding, shader
translation, guest descriptors and native Metal views. Float and unsigned integer
sampling support constant or direct-uniform integral layers relative to a checked
slice/mip range, including one-layer arrays and either texture/sampler slot.
All reachable branch requirements are validated before dispatch; depth arrays,
computed/interpolated layers and invalid float layer values are rejected.

All nine repository gates passed: workspace **939 passed, zero failed, 122 ignored**;
GPU script **424 passed, zero failed, one manual case ignored**. Totals overlap and
are not unique GPU counts. Focused tests passed: decoder3, compiler11, loader4,
native2, private native2 and original guest3. Native tests use an explicit producer
gate and a complete-root lease differential; guest fixtures compare literal float
pixels and exact integer bits, pending versions and nested selections.

Three production mutations failed their intended assertions: forced layer zero,
omitted producer wait and omitted full-root lease. Exact SHA-256 restoration and
restored compiler11/native2/private2/guest3 passes were recorded. Two independent
reviews accepted the bounded increment. Evidence is under
`artifacts/validation-full.PB3NoD`, `artifacts/shader-arrays-*` and
`artifacts/shader-array-mutation-*`. Shader storage-write views remain open in S2.4.
These are original synthetic tests on native Metal, not console execution evidence.

## Explicit range-composition checkpoint

Opcode65 separates semantic inputs from the destination used to preserve newer,
disjoint writes. Load and sampling use the requested ancestor; copy-back starts
from an explicitly authorized descendant. Checks cover every intervening write,
selected slice/mip overlap, whole-root barriers and exact backing layout.
Published identities survive receipt release, so losing a newer capability cannot
silently authorize replacing its contents with an older root. Retired public roots
remain in failure ordering without blocking unrelated successor preparation.

All nine repository gates passed: workspace **921 passed, zero failed, 117 ignored**;
GPU script **417 passed, zero failed, one manual case ignored**. Totals overlap and
are not unique GPU counts. Focused evidence includes **8 CPU tests**, **5 private
native tests** (four existing transaction tests plus role separation), and **1 original
guest composition test** exercising both collection orders. The guest preserves B's
slice0 while reading sampled data from A and loading A's slice1. CPU tests cover
transitive overlap, distinct mips, released/retained publication, unrelated ancestry,
failed ancestry with a cached successful result and uploaded-root retirement.

The full run caught a legacy second-frame Load regression in generation handling;
it was fixed without changing the legacy test. The final full run and focused legacy
case passed. Two production mutations compiled and failed assertions: Load from the
composition base, and omitted published-base admission validation. SHA-256 hashes
confirmed exact restoration; restored CPU8 and private native5 tests passed.
Logs: `artifacts/validation-full.mWnUdr`, `artifacts/range-composition-*`, and
`artifacts/range-mutation-*`. CPU failure injection is not GPU fault evidence, and
naturally ordered descendant waits do not establish independent producer gating.

At this checkpoint, shader-indexed array sampling and storage-write views remained open;
the newer array checkpoint above records subsequent acceptance.
Composition is serialized; independent subresource concurrency remains S2.7 work.

## Guest selected attachment transaction checkpoint

Writable guest color/depth selections now compose into complete immutable texture
versions, with all roots validated before atomic publication. Original guest tests
check same-root MRT slices, pending Load, both collection orders, lifetime changes,
fresh depth Clear and selected read-only/write-enabled depth comparisons. Four
private native tests cover failure after each copy-back, partial admission drainage,
internal-node capacity/refund and opaque depth shape validation.

All nine repository gates passed before the final budget-test addition: workspace
**912 passed, zero failed, 116 ignored**; GPU script **416 passed, zero failed,
one manual case ignored**. These suite totals overlap and are not unique GPU counts.
The final focused loader suite passed **6 tests**, including a real 140 MiB + 64 byte
reservation that rejects a second transaction before dispatch, retains the charge
after completion and refunds it on receipt release. This is CPU mock accounting
evidence. The separate private native suite passed **4 tests**, and the original
guest suite passed **3**, with Metal API and shader validation on the local M4 Max.
Final Clippy and formatting checks passed after the test addition.

Two production mutations compiled and failed full-root byte assertions: composing
from a stale root lost an earlier slice update, and forcing copy-back slice zero
wrote the wrong location. The restored original guest suite passed all three tests.
Raw logs are under `artifacts/attachment-transaction-*`, `artifacts/attachment-mutation-*`
and `artifacts/validation-full.f7wnlw`. Receipt publication is atomic; native admission
is incremental and failures drain the cohort rather than roll back its timeline.

S2.4 remains open for range-aware composition, shader-indexed array views and
shader storage-write views. Cross-submission hazards remain whole-root.

## Native selected-view depth Load checkpoint

The native queue now loads one selected Depth32Float mip/slice into a fresh depth
surface. Read-only comparisons, writes and explicit full-root copy-back preserve
exact produced versions. At this native checkpoint, guest writable-view
transactions were still unimplemented (see the subsequent checkpoint above);
S2.4 is open. See the [contract](../resource-formats.md#native-selected-depth-view-load)
and [completion checklist](../superpowers/plans/2026-11-26-view-completion.md).

Four added tests cover literal .25/.75 depth values, .5 writes and .625 comparisons,
selected zero-draw Load, full-root immutability, copy-back preservation, producer
ordering, invalid inputs, root ownership and controlled terminal-failure propagation.
The ownership check compares equivalent output/scratch charges and requires the
additional complete80-byte root. This measures logical accounting, not driver RAM.

All nine gates passed: workspace **907 passed, zero failed, 113 ignored**; GPU script
**413 passed, zero failed, one manual case ignored**. After strengthening the budget
test and adding the direct selected zero-draw case, quick validation passed **178
check executions across48 steps**, zero failed/ignored, and formatting/lint passed.
Two native depth checks also match the older broad attachment-view filter, so this
quick total includes repeat executions. GPU-script totals also include CPU overlap.
The native test executable is ARM64 and links system Metal; API/shader validation
were enabled for the quick suite.

Three deliberate faults compiled and failed decisive assertions: forcing retained
depth Load to Clear produced wrong literal pixels; omitting the selected producer
wait released a GPU marker too early; omitting the root lease lost exactly80 bytes
from the differential ownership check. The first lease mutation survived the earlier
absolute threshold because scratch allocations masked the missing charge. That
result is retained separately; the strengthened test catches it. Every mutation's
source restoration was checked by SHA256. Logs use `artifacts/depth-attachment-*`.
These are original trusted native shaders, not translated guest storage operations
or GPU hardware-fault recovery evidence.

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

At the historical checkpoint above, full Stage 2 and Stage 3 remained open.
The finite Stage 2 resource contracts are now accepted in the integrated record
linked above. Stage 3 general shader control flow and divergent execution remain
open. Additional geometry formats, instancing and indirect draws are
outside this increment. Original synthetic fixtures do not demonstrate commercial
Xbox game execution, console hardware equivalence or a process-wide memory ceiling.
Payload budgets cover explicitly documented owned allocations rather than total
driver or process memory.
