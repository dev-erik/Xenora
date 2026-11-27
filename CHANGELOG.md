# Changelog

Notable changes are grouped using [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Experimental interfaces may change. Authorized releases should identify their
exact revision and known limitations.

## [Unreleased]

Guest execution uses original, purpose-built test programs. Commercial Xbox game execution
remains unsupported.

### Changed
- Memoized graphics loop-bound inference, including unknown results and remaining
  analysis depth, to bound analysis work on shared MIN expression graphs.
- Decode signed SOPP branch targets relative to the following instruction, while
  preserving full-stream instruction-boundary checks. Compute and graphics consumers
  reject unsupported cycles explicitly; bounded graphics countdowns are described below.
- Extended the read-only RDNA2 planning CLI (`python3 -m qa rdna2`) with family/status filters, compact summaries, Codex-ready manifests under `artifacts/`, active-claim exclusion (family claims exclude those families; path-only claims block overlapping permitted files), coverage reconcile against `semantic_coverage` without inflating counts, and inventory diffs. Covered by `tests/qa/test_rdna2_plan.py`; planning only, no GPU production edits.

### Added
- Focused Stage 1 rendering revalidation before shader expansion, covering indexed
  scenes, per-draw state, depth, blending and multiple targets; recorded in the
  local graphics checkpoint with current native and guest test evidence.
- Integrated S2 resource acceptance matrix linking the seven finite contracts,
  cross-workstream tests and final validation evidence; see
  `docs/testing/stage2-integrated-acceptance.md`.

The resource entries below record successive implementation increments. Their
then-current exclusions and open-work statements are historical; the integrated
acceptance matrix and linked contracts describe the current supported scope.

- Added connected four-queue resource graph acceptance tests combining produced
  geometry, attachment loads, descriptor sampling, storage writes and copies.
  Coverage checks bounded admission, retained versions, explicit guest mirrors
  and generation recovery; see the local graphics checkpoint for executed evidence.
- Opt-in resource graphs now admit four logical/native queues while retaining the
  sixteen-node and existing payload limits. Legacy cohorts still admit only their
  original two queues; recovery recreates all four independent event timelines.
- Explicit guest-memory bindings for packed buffer, texture and depth roots through
  opcodes75–76. Atomic batch import/export checks states, retained submissions,
  mapping identities and permissions; remapping requires explicit rebind. GPU
  results are written back only at an explicit idle synchronization boundary.
- Guest-backed buffers can supply pitched image transfers through the existing
  image, mip/array and typed-copy operations. Submission snapshots the complete
  checked logical buffer after budget validation; pending work remains independent
  of subsequent guest writes or unmapping. Destinations remain owned resources.
- Immutable recorded descriptor tables through guest opcodes70–74, with four
  texture and sampler slots, atomic updates and bounded retained revisions.
  An additive shader ABI supports GPU selection by checked integer uniforms;
  legacy two-slot APIs retain their original masks. Exact produced versions,
  full-root ownership and storage feedback checks cover all four slots.
  This is a bounded synthetic interface, not a console descriptor heap format.
- Added bounded shader storage views for selected RGBA8/R32Uint mip/slices, with
  ordered IMAGE_STORE effects, exact prior contents and atomic full-root publication.
  The synthetic contract requires one triangle and unique fragment-pixel addresses;
  unsupported storage forms and feedback aliases reject before execution.
- Shader-indexed float and unsigned-integer array views through opcode66 and
  explicit pipeline dimension flags. DIM5 IMAGE_SAMPLE_LZ accepts constant or
  validated uniform layers; unsupported layer expressions reject. Nested ranges,
  exact pending versions and complete-root ownership extend through native Metal.
- Explicit selected-root composition through opcode65 separates exact shader/Load
  ancestry from the newer backing used for copy-back. Slice/mip access checks reject
  overlap and unknown lineage. Published-version tracking survives receipt release,
  preventing stale full-root replacement; barriers remain whole-resource operations.
- Guest writable color/depth selections through opcodes61–64. A queued transaction
  composes selected rendering into complete immutable texture versions and validates
  every root before atomic guest publication. Partial native admission failure drains
  the cohort; internal timeline values are not rolled back. Cross-submission hazards
  remain whole-root while range scheduling and shader array/storage views are developed.
- Native queued depth Load from a selected one-mip Depth32Float view. Depth
  comparisons and writes operate on a fresh selected surface; explicit typed
  copy-back preserves the rest of the backing. Exact producer ownership and
  full-root budget charges survive collection. This is a native prerequisite;
  guest writable views and S2.4 completion remain open.
- Native queued color Load from a selected one-mip, linear GPU-produced texture
  view. The source remains immutable and retains its exact producer and full-backing
  budget. The rendered surface can be copied into a fresh full-backing version using
  the existing typed-copy API. Guest writable-view bindings and range-aware write
  hazards remain separate work; sRGB conversion and multi-mip attachment loads reject.
- Immutable array-range descriptors and nested selections through guest opcodes57–60.
  Selected 2D children compose relative slice/mip offsets, retain the root backing
  independently of parent lifetime, and reuse exact queued-version sampling.
  Bounds, storage families and the shared object cap are validated before mutation.
  These descriptor ranges do not yet provide shader-indexed arrays or writable views.
- Explicit typed texture transfers, sampling and attachment roles through guest
  opcodes53–56. The finite matrix covers R8, RG8, RGBA8, BGRA8, RGBA16Float,
  R32Float, R32Uint, Depth32Float and runtime-supported BC1/BC3. Copies preserve
  packed bits, complete mip/array backings and exact queued versions. Typed
  pipelines distinguish float, integer and raw-depth sampling; integer targets
  preserve unsigned bits and reject blending. Selected read-only views retain
  complete ownership, and completion validates layouts before atomic publication.
  Writable subresource views, broader bindings and general guest coherence remain
  separate workstreams. See `docs/resource-formats.md` for the matrix and exclusions.
- Read-only 2D views of selected RGBA8 array slices through guest opcode52,
  with nested mip ranges and exact queued GPU versions. Existing view bindings
  accept these handles; full backing ownership and whole-resource hazards remain
  enforced. Shader array indexing and writable views remain unsupported.
  Quick validation passes 116 checks across 33 steps, all nine repository gates
  pass, and wrong-slice, wrong-mip and missing-wait mutations fail assertions.
- Bounded RGBA8 array storage through guest opcode51 and explicit slice/mip copies
  through opcode50. Transfers preserve every untouched subresource and retain exact
  GPU versions; copying a slice into a 2D texture enables existing sampled views.
  Array sampling/views, attachments and independent subresource concurrency remain
  unsupported. Limits apply to complete arrays, including one-layer array identity.
  Quick validation passes 105 checks across 29 steps; all nine repository gates
  pass. Nine deliberate mip/slice, ownership and ordering faults fail observable
  assertions, with restored sources verified afterward.
- Explicit queued RGBA8 mip transfers through guest opcode50, preserving untouched
  mip levels and borders in complete immutable destination versions. Produced mip
  chains retain native GPU storage for read-only sampling. Legacy image-copy APIs
  keep their single-level contract; multi-mip attachment Load remains unsupported.
- Linear R8 sampled textures with one-byte mip storage, explicit R8 capabilities,
  immutable native Metal uploads and read-only root/nested views. Guest storage
  code3 and view code6 preserve existing codes; uploads retain exact R8 bytes.
  R8 attachments, produced outputs and queued texture copies remain unsupported.
  Five new CPU checks, three native tests, two original guest tests across all
  three adapters and all nine gates pass. A wrong-stride mutation fails literal
  native pixels; restored sources pass the full suite.
- Extended submission-time guest-buffer snapshots to byte-region copies through
  opcode43. Copies preserve untouched destination bytes and retain existing
  dependency and alignment checks. The full logical source must be readable;
  guest-backed destinations and image transfers remain unsupported. Four CPU
  regressions, two original guest tests and all nine gates pass; a wrong-offset
  mutation fails the literal native readback comparison.
- Guest-backed buffer creation through opcode49 and submission-time source snapshots
  for queued whole-buffer copies into owned destinations. Checked guest reads capture
  pre-submit writes while isolating pending work from later changes. Guest-backed
  destinations, region/image copies and general shared-memory coherence remain unsupported.
  Ten CPU checks, two original guest tests and all nine repository gates pass;
  a deliberate wrong-source mutation fails the literal guest byte comparison.
- A one-command graphics validation script with focused and full-suite modes,
  native Metal validation, per-step logs and failure/empty-selection detection.
- Reusable immutable sampler handles through guest opcodes46–48. Binding copies
  descriptors into recorded commands, allowing handles to be destroyed without
  changing recorded or pending draws. Stale handles and invalid binding tables
  reject atomically; existing sampled-view and LOD restrictions remain in force.
  Seven CPU tests, two original guest tests across three adapters and all nine
  repository gates pass; a wrong-mip mutation fails literal pixel comparisons.
- Nested read-only texture views through guest opcode45. Child mip ranges are
  relative to the parent and normalize to the root backing; children remain valid
  after parent destruction. Six CPU checks and two original guest tests across
  three adapters pass. Array and writable views remain unsupported.
- Reused bounded Metal compute pipelines, adapted from upstream commit6f1ac2a.
  Cache keys include source, entry and validation mode; contexts remain isolated.
  Compilation failures are not cached. Four native cache tests pass, along with
  all nine repository gates. No local performance speedup is claimed.
- Added an opt-in queued path for GPU-produced positions, colors and R16/R32
  indices, with guarded byte gathering and validation status carried through
  submission dependencies. Host-only and legacy paths retain their contracts.
  Local CPU, eight native and two original guest tests pass, with eight injected
  fault types detected. Final independent acceptance is recorded locally.
- Added bounded signed-positive graphics countdowns with signed MIN proofs and
  exact negative zero-trip live-outs. Signed and unsigned value facts remain
  separate across loop joins; unsafe bounds and other induction forms reject.
  Local CPU regressions, five native tests, an original guest across three adapters
  and all nine gates pass. Six injected faults trigger pixel, admission or fact
  assertions. Two independent reviews accepted this bounded increment.
- Added conditional and unconditional exits to the innermost bounded graphics
  countdown's continuation. Typed live-out values and SCC follow the selected edge;
  wholly unreachable loop intervals need no entry bound. Ten CPU regressions, five
  native tests and an original guest across three adapters passed, along with all
  nine local gates and two independent scoped reviews. Eight faults trigger pixel
  or admission assertions. General loops and divergent-wave execution remain open.
- Added opt-in queued graphs with up to 16 retained nodes and multiple pending
  submissions per queue. Inputs resolve exact ancestor versions; conflicting
  unordered writers reject, and failures propagate before descendant publication.
  Legacy backend limits remain intact. Original native and guest copy/render tests,
  all nine local gates and two independent scoped reviews passed. Eight injected
  faults detect ancestry, identity, failure-ordering and ownership regressions.
- Added arbitrary-byte queued buffer-region copies through the native worker and
  guest opcode43. GPU patches preserve the full destination and exact input versions;
  older backend registrations retain their aligned-only contracts. Native evidence
  covers 153 literal byte cases, pending inputs, multiple threadgroups and payload
  boundaries. All nine local gates and two independent scoped reviews passed;
  ten injected faults trigger byte, ordering, ownership or validation failures.
- Extended the graphics countdown compiler to strictly nested loop forests with
  distinct counters and multiplicative execution budgets, including final child
  header checks. Independently bounded counters retain their bound across loop
  joins only when all writes are validated nonwrapping decrements. Nine new native
  tests and an original guest test pass, including three loop levels, retained
  counters, swaps, texture accumulation and geometry. All nine local gates and
  two independent scoped reviews passed; six faults trigger admission or pixel
  assertions. General loop shapes and wave divergence remain unsupported.
- Added initial compilation of contiguous, nonnested unsigned graphics countdowns,
  with bounds derived from original shader constants or unsigned MIN operations.
  Generated Metal uses real iteration and typed carried values; unsafe loop shapes
  and missing entry definitions reject explicitly. CPU regression and admission-boundary
  tests, eight native tests and an original guest test pass locally. All nine gates
  and two independent scoped reviews passed; six shader faults and three decoder
  faults trigger regression checks. General loops and divergent execution remain unsupported.
- Added native RGBA8 texture rectangles and pitched buffer/texture transfers that
  preserve the full destination and retain both exact input versions. Native tests
  pass with Metal API/GPU validation, including non-256 row pitches and exact final-row
  bounds. Guest opcode 44 records validated copies, and original guest programs
  exercise every direction and pending-input role. All nine local gates and two
  independent scoped reviews passed; 18 fault-injection cases fail observable checks.
  Additional mips/slices, compressed/depth copies and broader graphs remain open.
- Added native aligned buffer-region copies that preserve the complete prior
  destination, including odd logical tails, and retain exact source/destination
  versions. Guest opcode 43 records validated regions; original guest tests cover
  both pending-input roles and both collection orders. All nine local gates and two
  independent scoped reviews passed; six production faults fail observable tests.
  Unaligned copies remain open; image transfers are described above.
- Added native queued whole-buffer and single-mip RGBA8 copies with immutable
  produced outputs, GPU dependencies, exact logical buffer lengths and retained
  payload accounting. Original native tests cover ordering, direct texture use,
  resource lifetime and budget refunds. Guest opcode 42 records copies submitted through the
  typed loader/worker path; four original guest tests pass for buffer chains, render-to-copy,
  copy-to-sampling and copy-to-Load. All nine local checks and two independent scoped
  reviews passed; six production mutations fail observable assertions. Partial and
  subresource transfers, additional formats and broader queue graphs remain open.
- Added typed graphics scalar locals, MOV and eleven arithmetic/selection forms,
  with exact integer joins, carry/borrow and signed overflow, protected inputs and
  definite-assignment checks. Original native fixtures cover result bits and flags,
  arithmetic chains, aliases and high-bit joins; an original guest test exercises
  all three native backend modes. All nine local gates and two independent scoped
  reviews passed; seven production mutations cause observable assertion failures.
  Loops and divergent execution remain open.
- Added typed immutable queued depth versions, GPU copies into fresh working depth,
  read-only preservation, later updates and clear-only initialization. Native, worker
  and original guest tests cover ownership, failure recovery and atomic publication.
  All nine local gates and two independent scoped reviews passed; five production
  mutations fail observable assertions. Depth sampling/views remain open; whole-resource
  copies are described above.
- Added queued color initialization that distinguishes host bytes from an exact
  produced GPU version, with native copying into a fresh attachment before Load.
  Original native, worker and guest tests validate partial overlays, immutable
  versions, failure handling and retained payload reservations. All nine local gates
  and two independent reviews passed; five production faults fail observable tests.
  Whole-resource copies and typed depth versions are described above.
- Added per-draw scalar uniforms (s24..s31), guest opcode 41, and forward runtime
  graphics branches with definite-assignment checks and SCC/VGPR joins. Native and
  sync/async/queued loader adapters retain immutable uniform data through completion.
  Original tests cover all twelve integer comparisons, nested joins, two targets
  and geometry/color changes without recompilation. All nine local gates and two independent scoped reviews passed, with four
  observable production mutation failures. Divergent EXEC/VCC execution and loops
  remain open.
- Added checked canonical RGBA8/BC1/BC3 mip storage layouts for persistent texture
  resources, including complete compressed edge blocks and overflow-safe offsets.
  Original CPU tests cover odd dimensions, mip tails and invalid layouts.
- Added immutable persistent native BC1/BC3 sampled textures, shared-storage
  linear/sRGB mip views, typed loader resources and guest opcode 40. All three FFI
  adapters propagate actual-device BC capabilities, preserve compressed bytes and
  retain typed parent storage; unsupported devices reject without decompression.
  Render outputs remain RGBA8. Targeted guest sync/async, queued and four native
  cases passed. Interpolated alpha and color use independently derived reference
  precision envelopes; wrong-mip and lost-sRGB production mutations remain detected.
  Full local validation gates passed; no Xbox equivalence is claimed.
- Added actual shared-storage sampled mip-range views with RGBA8 linear/sRGB
  interpretation, view-relative dimensions and LOD validation, and guest view
  creation/destruction/binding. Immutable native and produced-view handles preserve
  parent ownership and queued dependency provenance without upload/raw access.
  FFI caches include backing identity, metadata and complete view descriptors.
  Local native, original guest and queued FFI tests cover mip/spatial selection,
  sRGB alpha preservation, retained ownership and failure handling. Arrays, writable
  or arbitrary-format views, wider guest formats and full Stage 2/3 support remain open.
- Added an opt-in two-queue graphics backend with two retained loader receipts,
  dependency-aware planned states and opaque native produced-texture versions.
  `RenderCohort` samples producer RGBA8 outputs directly, stages owned depth uploads
  without waiting in submission, and drains failures in producer-first order before
  host-unblocking consumers. Publication failure rejects dependent results; recovery
  uses a fresh generation after ticket release. Existing async mode remains limited
  to one receipt. Tests cover gated native/FFI handoff and controlled host rejection;
  scripted missing-signal tests share production drain logic but do not demonstrate
  actual GPU-fault recovery. Overlapping depth and broader
  Stage 2 synchronization and Stage 3 control flow remain unsupported.
- Added two independent texture slots and two sampler slots, including all four
  cross-pairs, through translated shaders, loader snapshots, native submission and
  original guest opcode 34. Required binding masks and per-pair LOD compatibility
  are checked; opcode 21 remains slot-zero-only. Native submission rejects conflicting
  slot objects and incompatible devices, clears absent slots per draw and retains
  inputs through async completion. Local native and sync/async guest tests cover
  cross-pairs and sampler rebinding; general descriptors/views, wider synchronization
  and divergent graphics control flow remain open.
- Added original PE guest regressions for both index widths, padded attributes,
  signed base vertices, failed-frame recovery, fractional-alpha blending and channel
  masks through synchronous and asynchronous native submission. A combined indexed,
  sampled, two-target scene checks preserved depth/color and recovery after rejection.
- Extended graphics-stage translation with immediate/literal signed and unsigned
  scalar comparisons and uniform forward SCC0 branches. Uninitialized conditions,
  SGPR operands and invalid targets reject; runtime divergence remains unsupported.
- Added shader-pipeline changes within a single translated render pass, including
  sampled and unsampled draws. Native pipeline caching includes program identity;
  all programs validate before encoding and async tickets retain every draw input.
- Expanded translated native scenes to 65,535 vertices, 256 draws and one million
  pixels per target, with checked 4,096-axis bounds. Software and older fixture limits
  remain separate. Original indexed scenes cover both index widths, padded attributes,
  changing uploads, retained depth/color and atomic rejection of a bad final index.
- Added owned native rendering tickets with nonblocking status polling and explicit
  completion collection, plus a native worker and original guest submit/poll/wait/release
  commands. In-flight resources survive list reset and logical destruction; retired
  storage remains charged until completion. Outputs publish only after validation.
  The original async mode supports one outstanding receipt; the separate bounded
  queued backend described above adds two-queue dependencies.
- Added explicit color attachment Load for preserved RGBA8 targets across submissions,
  including the original guest descriptor interface. Rendering stages prior bytes in
  fresh native targets and publishes colors only after validated completion; existing
  target binding APIs continue to clear. Original guest tests exercise depth-tested
  preservation followed by a clipped overlay from a distinct shader pipeline.
- Graphics stages now follow validated forward unconditional branches while preserving
  reached-path register, interpolation and export checks. Original assembled shaders
  cover skipped geometry/color writes on native Metal. Runtime conditional branches,
  loops and divergent execution remain unsupported; uniform SCC0 support is listed above.
- Added the original `GraphicsCallV1` guest import with versioned fixed-size
  descriptors for resource/pipeline creation, transfers, command recording and
  submission. Writable result spans are preflighted before mutation; descriptors,
  shader reads and object/command counts are bounded. Legacy guest imports share
  the object limit. Vertex-binding arithmetic is checked and overflowing guest
  readback addresses retain memory-fault semantics. Legacy output spans are preflighted
  and failed temporary draw setup releases its objects. This is a synthetic ABI.
- Added compiled graphics pipeline metadata, loader pipeline/color/sampled-resource
  binding and a reusable native Metal adapter. The application fixture now uses
  this adapter; selected original programs exercise changing texture uploads,
  per-draw sampler/UV bindings, retained depth and eight color outputs.
- Loader RGBA8 textures now support bounded packed mip chains. The adapter uploads
  each distinct resource snapshot once per submission and binds independent samplers
  per draw. Explicit mip selection now pins native LOD clamps instead of being
  ignored. Invalid legacy binding use and sampled/output aliases fail before dispatch.
- Added loader depth resources, explicit clear/load binding, depth read/write
  barriers and staged backend snapshots committed with validated color outputs.
  Native adapters transfer immutable owned depth bytes across the loader boundary;
  no unsafe thread-trait declarations are needed. Invalid color readbacks preserve
  prior depth, barrier state and presentation. Clear-only depth passes are supported.
- Added retained native Depth32Float surfaces with explicit clear/load operations
  for translated multi-target passes. Successful synchronous submissions preserve
  depth across passes; staged copies keep earlier contents intact on failure.
  Native coverage includes occlusion, updates, reversed depth, read-only depth
  draws and serialized use across queues. General guest depth-resource services remain open.
- Bind one through eight distinct loader RGBA8 targets with independent clears,
  matching extents, ordered native output callbacks and atomic readback commits.
  All outputs remain available while presentation uses slot zero. Native tests
  render translated MRT exports across repeated frames and copy every resource
  back to guest memory; malformed later outputs preserve all resources and state.
- Added 1–8 native RGBA8 color attachments with separate clears and readbacks,
  sharing extent and per-draw depth/blend/mask state. Existing single-target APIs
  delegate to the same path. Native tests verify all slots and overlap/clipping.
- Preserve MRT0–MRT7 and explicit DONE in decoded pixel exports; translate dense
  output sets without renumbering. Full uncompressed VM exports are supported;
  duplicate targets, holes, missing final DONE and post-DONE exports are rejected.
  An original LLVM-assembled eight-output fixture reaches all eight Metal targets.
  General guest pipeline integration remains open.
- Added configurable depth clears through loader commands and the application
  fixture adapter, including reversed-depth native coverage and reset validation.
- Lower pixel-stage register add/multiply using separate precise FP32 operations.
  Original assembled fixtures exercise interpolated and sampled arithmetic on
  native Metal; reserved registers and arithmetic after exports remain rejected.
- Added retained texture/sampler bindings per draw to stateful translated passes.
  Native tests combine changing uploads, sampling/arithmetic, depth, blending,
  clipping and retained resource ownership over repeated frames. The binding ABI
  remains one slot-zero RGBA8 texture and sampler per draw.
- Added explicit loader resource destruction after all recorded command-list
  references are reset. Live references block destruction even after completed
  or failed submissions; stale handles are rejected. Guest memory and completed
  presentation pixels remain owned independently. This covers synchronous loader
  resources, not asynchronous GPU retirement or a guest destruction ABI.
- Added synchronous guest uploads and readbacks for host-backed loader resources,
  requiring COPY_DEST/COPY_SOURCE states and checked ranges. Failed memory access
  leaves destinations unchanged. Synthetic tests cover upload/render/readback;
  queued whole-resource transfers and their guest ABI are described above.
- Added checked integer viewport origins and per-draw scissor rectangles through
  the loader, native translated passes and application fixture adapter. Scissor
  reset restores full-target clipping; empty rectangles suppress rasterization.
  Native regressions cover clipping, preserved geometry, buffer advancement after
  empty draws, reset/recovery and invalid or overflowing regions. Coordinates
  remain bounded to the target; this does not add a general guest viewport ABI.
- Route the application's trusted-frame runner and persistent/interactive fixture
  loops through a shared loader-to-Metal adapter. Frame loops reuse a context,
  command list and bounded vertex buffer; the C API and fixture output remain
  unchanged. Native tests cover ordered state, invalid-input recovery and the
  existing frame-loop, lifecycle, input-replay and memory checks. The textured
  resource path remains separate; this is original-fixture integration only.
- Added translated Metal passes combining per-draw depth compare/write, blending
  and color write masks. Loader commands retain bounded draw state for ordered
  adapters; legacy adapters reject unsupported state. Native tests cover depth
  write changes, alpha blending, additive blending, masks and command-list reset.
- Stage loader resource-barrier metadata until successful readback so validation
  and backend failures leave resource states unchanged and allow correct retry.
  Local synthetic validation is not independent acceptance or game compatibility.
- Connected ordered loader pass callbacks to native Metal tests, retaining each
  draw's viewport within an extent fixed by the first draw. Legacy adapters remain
  compatible; unsupported target and clear changes are still rejected.
- Reject undersized render targets and malformed readback lengths before presenting
  a frame. Bound completed submission history to 256 handles so repeated frames do
  not accumulate unlimited diagnostic entries. Added translated vertex-arithmetic
  coverage through the ordered pass; no new shader opcode support is claimed.
- Added a native ordered triangle-pass API with per-draw position/color inputs
  and viewports over one cleared RGBA8 target. Existing single-draw APIs retain
  their signatures. Synthetic tests cover ordering, viewport changes, invalid
  later draws and the 256-draw limit;
  local tests do not constitute independent acceptance.
- Extended loader indexed draws to complete triangle lists of up to 768 indices,
  retaining 16-/32-bit index formats, offsets, signed base vertices and checked
  buffer reads. The combined command-list limit remains 256 triangles. Tests
  cover both formats, the maximum count, and invalid final indices.
- Added bounded non-indexed triangle-list draws through the loader and native Metal:
  up to 256 triangles in one command, using one cleared target and one GPU draw.
  The loader backend callback now takes a vertex slice; single-triangle Metal APIs
  remain compatible. Native tests cover both halves of a square, per-triangle colors,
  the maximum vertex count, and malformed input rejection. Instancing remains unsupported.
- Fixed consecutive loader draws erasing earlier geometry: compatible draws now share
  one cleared target, with a combined limit of 256 triangles. Vertex reads complete
  before rendering; viewport, target and clear changes after drawing starts are
  rejected. This is bounded synthetic-ABI batching, not general render-pass support.
  Local validation does not constitute independent acceptance.
- Added loader graphics regression tests that reuse one command list and guest vertex buffer
  across 48 changing frames, replay the sequence in a fresh context, preserve owned readbacks,
  and verify that invalid submissions leave the last presentation intact before recovery.
  A separate injected-backend-error test checks propagation and retry; it is not GPU evidence.
  The native test is included in `scripts/test-gpu.sh`. Local Metal API and shader validation
  passed on Apple M4 Max (96 native frame submissions); independent acceptance remains open.
  These are host-driven graphics API tests with synthetic shaders, not commercial-game execution.
- Added authoritative read-only x86-64 CPU inventory CLI (`python3 -m qa cpu-inventory`) over `crates/loader/x86/instructions.toml` via `generate_x86.load`, with opcode-map/prefix/operand summaries, native vs shared-semantic operation partitions from `dbt.rs`, fixture mnemonic evidence and a separate unsupported catalog. Does not count candidates as implemented. Covered by `tests/qa/test_cpu_inventory.py`.
- Added typed runtime inventory CLI (`python3 -m qa runtime-inventory`) over `crates/loader/runtime/services.txt`, reporting implemented/partial/stub services separately from API-set aliases and leaving independently-verified empty unless recorded. Covered by `tests/qa/test_runtime_inventory.py`.

- Added a typed runtime service registry (`crates/loader/runtime/services.txt`) and `xenora-runtime-gen`, which generates Win64 argument and result adapters, static metadata and a coverage report. It covers 20 services: 16 existing kernel32 services moved to typed handlers, and `sqrt`, `sqrtf`, `fabs` and `fabsf` with arguments and results in XMM registers. Conflicting keys, unlisted API sets, unsupported types and behaviour without an existing test are rejected. See `docs/runtime/registry.md`.
- Added SSE and SSE2 floating point in scalar and packed, binary32 and binary64 forms: add, subtract, multiply, divide, minimum, maximum, square root, compares, COMI/UCOMI and the conversions between formats and integers (parts of #127 and #128; awaiting independent review). `crates/loader/src/softfloat.rs` computes every result exactly and rounds once, with MXCSR rounding modes, sticky flags, denormals-are-zero, flush-to-zero, tininess after rounding and x86 NaN rules; host floating point is not used because ARM64 differs. An unmasked exception exits with `SimdException` and leaves everything but MXCSR unchanged. RCP, RSQRT and MMX conversions are rejected, and no floating-point form is translated to NEON yet.
- Added 6,824 floating-point vectors computed with exact fractions by `tests/x86/float_reference.py`, run through both engines from registers and memory and through the packed forms, and more than two million round-to-nearest results compared with the host's IEEE 754 hardware.
- Added XMM0-XMM15 and MXCSR to the guest CPU state, and the SSE and SSE2 moves and packed integer forms: aligned, unaligned, scalar, half and non-temporal moves, MOVD/MOVQ, sign-mask extraction, bitwise operations, wrapping and saturating arithmetic, compares, minimum, maximum, average, multiplies, PSADBW, packs, unpacks, shifts, shuffles, PINSRW, PEXTRW, LDMXCSR and STMXCSR (parts of #124, #125, #126 and #128; awaiting independent review). Operations with identical NEON lane semantics are translated to ARM64; the rest call the shared semantics. Misaligned 16-byte operands and reserved MXCSR bits raise a distinct general-protection exit. MXCSR is state only: floating-point arithmetic is not implemented.
- Added 374 LLVM-assembled vector instructions with a byte-array reference, compared with both engines over 23,936 states, plus alignment, fault, width and mixed-sequence tests.
- Added a declarative x86-64 instruction registry (`crates/loader/x86/instructions.toml`) and its generator, `crates/loader/generate_x86.py`, with `validate`, `generate` and `--check` modes. The static probe, the interpreter and the native DBT now share one decoder driven by its tables, so instruction boundaries, rejected forms and reported support agree. Overlapping encodings, such as a sign extension claiming the MOV-immediate opcodes, fail validation. See `docs/cpu/instruction-registry.md`.
- Added integer instructions for 8-, 16-, 32- and 64-bit register and checked memory operands: the eight ALU operations, TEST, INC, DEC, NEG, NOT, XCHG, ROL, ROR, SHL, SHR, SAR, MUL, IMUL, DIV, IDIV, CMOVcc, SETcc, BT, BTS, BTR, BTC, BSF, BSR, POPCNT, TZCNT, LZCNT, CBW/CWD families, PUSH, POP, LEAVE, RET imm16 and flag instructions, with parity and auxiliary-carry flags and a distinct divide-error exit (parts of #65, #66, #68, #70, #71, #73, #118 to #123, #194 to #196; awaiting independent review). Forms the DBT does not lower to ARM64 call the same Rust semantics and are counted.
- Added 985 LLVM-assembled integer instructions with a bit-level reference that reads only their assembly text, compared with the interpreter and the DBT over 47,280 states, plus 4,000 random instruction sequences compared between the engines.
- Added sixteen gfx1030 SOP2 integer forms through the semantic registry: `s_min_i32`, `s_min_u32`, `s_max_i32`, `s_max_u32`, `s_cselect_b32`, `s_bfm_b32`, `s_mul_i32`, `s_bfe_u32`, `s_bfe_i32`, `s_absdiff_i32`, `s_lshl1_add_u32` to `s_lshl4_add_u32`, `s_mul_hi_u32` and `s_mul_hi_i32` (AMD guide section 12.1). Min and max set SCC from the strict compare (a tie gives S1 and SCC 0), `s_cselect_b32` reads SCC and is refused when SCC is unset, and BFM, MUL, MUL_HI and CSELECT leave SCC unchanged. The registry gained these primitives, a `selected_src0` SCC effect, the select's SCC input and a declared `shift_amount` for the shift-and-add forms; `ScalarAluOp::evaluate` now returns an optional SCC. Both reference interpreters, written separately from the guide, run the same sixteen forms and are checked against 21,070 guide-derived cases on both CPU interpreters and on native Metal; 597 rejected programs are refused by both interpreters and by the compiler. A new LLVM-assembled fixture covers all sixteen. Part of #215.
- Added MOVZX, MOVSX (0F B6/B7/BE/BF) and MOVSXD (63) for register and checked memory operands in the interpreter and the native DBT, with exact lengths in the static probe (parts of #65, #66, #67, #68 and #117; awaiting independent review). Covered: 16-, 32- and 64-bit destinations, AH/CH/DH/BH without REX, FS/GS bases and RIP-relative operands. Other 0x66 forms are rejected instead of running at the wrong width. Known-answer encodings come from the LLVM assembler; MOV immediates (B8-BF) have regressions proving they are never read as sign extensions.
- Added Phase II infrastructure CLIs under `scripts/qa` and `scripts/release` (#277): read-only RDNA2 expansion planning over live inventories, unified verification profiles with stale-evidence detection, CPU/runtime coverage scans, benchmark regression comparisons that reject incompatible identities, git rename/delete scope collision reports, and release-readiness reports with SBOM/checksum/bundle checks. Covered by `tests/qa` and `tests/release`; advisory until reviewed.
- Added advisory infrastructure QA helpers under `scripts/qa/` and `scripts/release/` (issue #277): exit-status-preserving orchestration with JSON reports, PR scope/DCO/artifact checks, benchmark percentile analysis over existing samples, environment and repository health diagnostics, ARM64 Mach-O/bundle/codesign helpers, and checksum manifests. Covered by `tests/qa/` and `tests/release/`; checks stay advisory until reviewed.
- Recorded agent-queue live smoke evidence for issue #180 against the disposable private repository `ranveersoni98/xenora-agent-queue-smoke`, including claim grant, overlap refusal, release, not-ready denial, progress renew and a successful audit dispatch after fixing comment-only `apply_actions` and audit permissions/argparse issues.
- Added twelve gfx1030 VOP2 vector integer forms through the semantic registry: `v_mul_i32_i24`, `v_mul_hi_i32_i24`, `v_mul_u32_u24`, `v_mul_hi_u32_u24`, `v_min_i32`, `v_max_i32`, `v_min_u32`, `v_max_u32`, `v_xnor_b32`, `v_add_nc_u32`, `v_sub_nc_u32` and `v_subrev_nc_u32` (AMD guide section 12.7), with SRC0 a VGPR, an SGPR, `vcc_lo`, an inline integer or a literal. The registry gained a `vop2_vgpr` operand rule, the vector operations `mul24` and `mul_hi24`, a declared `source_order` for SUBREV, and a per-lane `VectorBinary` that selects one reusable MSL template per operation. Both reference interpreters, written separately from the guide, run the same twelve forms and are checked against 2,040 guide-derived cases, each asserting every lane of a 7-, 32- or 45-lane dispatch (partial, empty and full EXEC), on both CPU interpreters and on native Metal; 575 rejected programs are refused by both interpreters and by the compiler at the same word. A new LLVM-assembled fixture computes every lane from its own input. Part of #215.
- Added sixteen gfx1030 SOP2 integer forms through the semantic registry: `s_min_i32`, `s_min_u32`, `s_max_i32`, `s_max_u32`, `s_cselect_b32`, `s_bfm_b32`, `s_mul_i32`, `s_bfe_u32`, `s_bfe_i32`, `s_absdiff_i32`, `s_lshl1_add_u32` to `s_lshl4_add_u32`, `s_mul_hi_u32` and `s_mul_hi_i32` (AMD guide section 12.1). Min and max set SCC from the strict compare (a tie gives S1 and SCC 0), `s_cselect_b32` reads SCC and is refused when SCC is unset, and BFM, MUL, MUL_HI and CSELECT leave SCC unchanged. The registry gained these primitives, a `selected_src0` SCC effect, the select's SCC input and a declared `shift_amount` for the shift-and-add forms; `ScalarAluOp::evaluate` now returns an optional SCC. Both reference interpreters, written separately from the guide, run the same sixteen forms and are checked against 21,070 guide-derived cases on both CPU interpreters and on native Metal; 597 rejected programs are refused by both interpreters and by the compiler. A new LLVM-assembled fixture covers all sixteen. Part of #215.
- Added audio PCM streaming and sandboxed virtual filesystem (VFS) integration to the native application runtime (Gate 6, #284). The C interface gains audio management functions (`xenora_session_audio_init`, `xenora_session_audio_submit`, `xenora_session_audio_read`, `xenora_session_audio_status`, `xenora_session_audio_set_volume`) and virtual filesystem functions (`xenora_session_vfs_set_root`, `xenora_session_vfs_open`, `xenora_session_vfs_read`, `xenora_session_vfs_seek`, `xenora_session_vfs_close`, `xenora_session_run_audio_vfs_fixture`). The audio subsystem implements a bounded ring buffer supporting configurable sample rates, channel configurations, PCM formats (S16LE, F32LE), volume attenuation, mute, and buffer underrun tracking. The virtual filesystem operates within an explicitly configured, sandboxed directory root (`fixtures/assets`), strictly rejecting path traversal attempts (`..`, absolute paths, null bytes) and invalid or stale handles, while supporting sequential and seekable file reads (`XENORA_SEEK_SET`, `XENORA_SEEK_CUR`, `XENORA_SEEK_END`). An original PE fixture (`fixtures/pe/audio_vfs.exe`, generated by `fixtures/pe/audio_vfs.py` using only the supported ARM64 DBT instruction subset) opens `asset.bin` (generated by `fixtures/assets/generate.py`), loads a 2x2 RGBA texture and 256 PCM audio samples via VFS, initializes audio, submits the PCM audio queue, and renders the loaded texture on Metal. Verified with unit tests, four Metal acceptance tests in `crates/ffi/tests/audio_vfs.rs`, UI controls and a `--smoke-audio-vfs` CLI in `Xenora.app`.
- Added persistent GPU resources, dynamic texture updates, and resource retirement to the native application (Gate 5, #279). The C interface gains `xenora_resource`, resource types (`XENORA_RESOURCE_TYPE_*`), `xenora_session_create_texture`, `xenora_session_update_texture`, `xenora_session_destroy_resource`, `xenora_session_resource_info`, `xenora_session_render_textured`, and `xenora_session_run_textured_fixture`. Persistent GPU resources (textures) are explicitly allocated and owned by the execution session, surviving across execution slices without re-allocation. Dynamic texture subregion updates modify GPU texture contents in-place on native Metal. Resource retirement is completion-aware: destroying a resource held by in-flight GPU execution marks it retired and safely defers destruction until in-flight work completes, while rejecting subsequent caller queries with typed error codes. An original PE fixture (`fixtures/pe/textured.exe`, generated by `fixtures/pe/textured.py` using only the supported ARM64 DBT instruction subset) creates a persistent 2x2 texture via `xenora.dll!texture_create`, renders frame 0, updates pixels via `xenora.dll!texture_update`, renders frame 1, and retires the resource via `xenora.dll!texture_destroy`. Xenora.app gains "Run textured fixture" controls and `--smoke-resources`. Six GPU acceptance tests with Metal validation verify persistent texture lifecycle across multiple frames, dynamic 1x1 and 2x1 subregion updates on native Metal, completion-aware deferred retirement, rejection of invalid/stale/double-destroyed handles, slot reuse across 200 cycles without leaking, and full guest fixture execution. Wired acceptance tests into `scripts/test-gpu.sh`.
- Added interactive guest execution with dynamic input handling and rendering to the native application (Gate 4, #276). The C interface gains `xenora_session_start_interactive_loop`, `xenora_session_send_input`, `xenora_session_queue_input`, and `xenora_session_get_recorded_frame`, along with input masks (`XENORA_INPUT_*`) and `xenora_input_state`. The original interactive guest fixture (`fixtures/pe/interactive.exe`, generated by `fixtures/pe/interactive.py` using only the supported ARM64 DBT instruction subset) polls host input via `xenora.dll!input_poll` and steers its rendered column position (0..3) on native Metal. Xenora.app gains interactive frame-loop controls ("Interactive (300 frames)", "◀ Left", "▶ Right", "Release") and `--smoke-interactive`. Four GPU acceptance tests with Metal validation verify that input events drive guest state and pixel geometry, deterministic input replay reproduces identical frame sequences across multiple sessions, key release and focus loss cleanly hold position without stuck keys, and destroying an interactive session joins promptly. Wired the interactive tests into `scripts/test-gpu.sh`.
- Added persistent guest rendering with pause, resume and stop to the native application (Gate 3, #266). The C interface gains `xenora_session_start_frame_loop`, `xenora_session_pause_frame_loop`, `xenora_session_resume_frame_loop`, `xenora_session_stop_frame_loop`, and `xenora_session_frame_loop_status`, plus new states (`XENORA_LOOP_*`). The guest fixture (`fixtures/pe/frame-loop.exe`, generated by `fixtures/pe/frame-loop.py` without external toolchains) runs on a session-owned thread and repeatedly calls `xenora.dll!frame_submit` until told to halt; each frame is rendered on native Metal, checked per pixel against geometric expectations, and returned via read-back RGBA8 buffers. Destroying a running or paused session safely stops its loop outside the global session lock. Xenora.app gains persistent frame-loop controls ("Start 300 frames", "Pause", "Resume", "Stop") and displays live frames with their frame index. Five GPU acceptance tests with Metal API and GPU validation verify frame ordering over 300 frames, live sampling across 2,000 frames without backwards progression, pause/resume/stop lifecycle, prompt join on session destruction, and stable memory footprint across repeated sessions. Added `--smoke-loop` to Xenora.app and wired the acceptance tests into `scripts/test-gpu.sh`.
- Added read-only executable inspection to the native app (#267). Xenora.app can open a PE file with Open Executable…, by dropping it on the window, or from Finder (it registers as an alternate viewer for Windows executables). The new C function `xenora_inspect_executable` returns a text report built from the existing static probe and loader validation: file size and SHA-256, header facts, whether `ParsedPe::parse` accepts the file, sections, imports not provided by Xenora's standard runtime services, the bounded static disassembly's count of instructions the ARM64 DBT does not support with the first one's address, and relocation and TLS counts. Nothing in the file is executed, and executing opened files remains unavailable pending #257. Inputs over 256 MiB are refused before reading. A panic inside the probe is reported as a Xenora defect instead of crashing the app; the new truncation test found one in `probe_pe_bytes` (truncations of `trusted-runner.exe` to 1,556–1,665 bytes slice past the end of the file while reading an import name), reported as #270 and not fixed here. Checked with the x1 executables in `experiments/x1/fixtures`, which the app does not embed, with malformed inputs, and in the window through the Finder open path and the Open panel; drag and drop was not exercised.
- Added eleven gfx1030 SOP1 unary forms through the semantic registry: `s_not_b32`, `s_brev_b32`, `s_bcnt0_i32_b32`, `s_bcnt1_i32_b32`, `s_ff0_i32_b32`, `s_ff1_i32_b32`, `s_flbit_i32_b32`, `s_flbit_i32`, `s_sext_i32_i8`, `s_sext_i32_i16` and `s_abs_i32` (AMD guide section 12.3). Searches that find nothing return -1, `s_abs_i32` of 0x80000000 stays 0x80000000, and only NOT, BCNT0, BCNT1 and ABS write SCC. The registry gained these primitives, a `sop1_sgpr` operand rule, and a declared `source_bits` for sign extension that is checked against the XML's 8- and 16-bit source operands. The reference and independent semantic-review interpreters, written separately from the guide, evaluate the same eleven forms and are checked against 4,126 guide-derived cases on both CPU interpreters and on native Metal with validation; 231 rejected programs are refused by both interpreters and by the compiler. A new LLVM-assembled fixture covers all eleven. Part of #215.
- Showed the first guest-driven frame in the native app (Gate 2, #263). The C interface gains `xenora_session_run_trusted_fixture`, which runs only the embedded trusted PE fixture: its x86-64 code is translated and executed, its `xenora.dll!fixture_draw` call compiles the x9 RDNA2 shader fixtures to MSL and renders on the native Metal device, and the 4×3 RGBA8 read-back is returned only if the guest halts with the CLI runner's expected results and every pixel equals the pixel shader's export, (1, 0, 0, 1) as opaque red. Xenora.app's "Run trusted fixture" button shows that frame, and `--smoke-frame` prints it. A `gpu-tests` test, run by `scripts/test-gpu.sh`, executes the fixture three times through the interface with Metal validation enabled. The frame is a read-back image, not drawable presentation, and no caller-supplied executable can run. Verified on one Apple Silicon Mac; the window was checked by screenshot.
- Added a minimal native macOS application shell and its Rust interface (Gate 1 of the native application work). `crates/ffi` builds a static library with a versioned C interface (`crates/ffi/include/xenora.h`, version 1): opaque, generation-checked 64-bit session handles (never pointers), at most 64 live sessions, stable status codes, and panics caught at the boundary. Version 1 can only create and destroy sessions and parse the bundled trusted PE fixture; it executes no guest code. `app/` is a SwiftUI app built with Swift Package Manager that shows the interface version and the fixture's section count and entry point. `scripts/build-app.sh` builds an ad-hoc-signed, arm64-only `build/Xenora.app` with no entitlements and runs its `--smoke` check. Six Rust tests cover invalid, stale and double-destroyed handles, version mismatch, null outputs, 4,000 concurrent create and destroy cycles, and header/constant agreement. The CLI and headless workflows are unchanged. Verified locally on one Apple Silicon Mac (macOS 27.2.1, Swift 6.4, Rust 1.100.0); not yet run in CI.
- Added the gfx1030 SOPC compares `s_cmp_{eq,lg,gt,ge,lt,le}_{i32,u32}` through the semantic registry, which replaces the hand-written `s_cmp_eq_u32` path. Each sets only SCC to whether the relation holds with SSRC0 on the left (AMD guide section 12.4); the registry gained a `compare` operation with a declared `relation` and a `sopc_scalar` operand rule. `Instruction::ScalarCompareEqU32` is now `Instruction::ScalarCompare { op: ScalarCompareOp, .. }`; `s_cmp_eq_u32` behaves as before. The reference and independent semantic-review interpreters, written separately from the guide, evaluate all twelve, checked against 7,824 guide-derived cases on both CPU interpreters and on native Metal with validation; 348 rejected programs are refused by both interpreters and by the compiler. A new LLVM-assembled fixture covers all twelve. Two literal selectors stay rejected. Part of #215, stage D.
- Added eight gfx1030 SOP2 forms through the semantic registry alone: `s_andn2_b32`, `s_orn2_b32`, `s_nand_b32`, `s_nor_b32`, `s_xnor_b32`, `s_lshl_b32`, `s_lshr_b32` and `s_ashr_i32`, each with SCC = result != 0. Each is one registry entry with no per-form Rust; the registry gained these primitives and a declared shift-count width (`shift_count_bits = 5`, so shifts use S1[4:0] as in the AMD guide, section 12.1), which the generator applies as a mask and checks against the XML's unsigned count operand. The reference interpreter and the independent semantic-review interpreter, written separately from the guide, now evaluate the same eight forms and are checked against 6,896 guide-derived cases; a new LLVM-assembled fixture (`fixtures/rdna2/scalar-logic.s`) runs on CPU and native Metal with validation. A NULL destination (s125) stays rejected. Part of #215, stage C.
- Added an independent operand-domain verification harness for the 9 supported SOP2 scalar ALU forms and 12 VOPC integer compares (`tests/validation/tests/operand_domains.rs`, part of #215 stage E): 4,788 generated cases (boundary values, 64-bit carry and borrow chains, aliased registers, every integer inline constant, literals, s105 and v255, partial and zero EXEC, VCCZ) and 498 rejections (missing literal words, and encodings outside Xenora's supported subset) run on `xenora_ref` and, as an ignored test picked up by `scripts/test-gpu.sh`, on native Metal. Expected values come from the public AMD RDNA 2 guide's pseudocode, written out in the test without Xenora code; this is agreement with a spec-derived reference, not with Xbox hardware. Local verification only: 32 deliberate code breaks were each caught; the guide leaves the width of `S1 + SCC` in `s_subb_u32` open, and those 8 cases are kept in their own family.
- Added a declarative semantic registry for gfx1030 instruction forms (`crates/isa/semantics/rdna2.toml`, schema 1) and its generator, `crates/isa/generate_semantics.py`, with `validate`, `generate` and `--check` modes. The generator checks every declaration against a closed vocabulary and the pinned AMD XML snapshot (opcode, family, SCC input and output, operand widths and signed formats), rejects contradictory, incomplete or duplicate definitions, and emits static Rust tables marked as generated. The nine SOP2 forms moved onto it with no change in behaviour: the decoder's SOP2 dispatch and `ScalarAluOp`'s mnemonic, SCC read and evaluation are now generated, and evaluation goes through shared primitives instead of per-opcode code. All existing SOP2 tests, including the native Metal known answers and the LLVM comparison, pass unchanged. Decoding speed is unchanged (10.2 ns before, 10.1 ns after, per SOP2 instruction, release build on an Apple M5 Pro). Also added `scripts/audit/semantic_coverage.py`, which generates `docs/shaders/semantic-coverage.md`: per-family counts of identified, declared, decoded, lowered, reference-verified and Metal-verified forms, taking evidence only from `docs/status.toml`. Part of #215, stages A and B.
- Added a native Metal blend pass, `NativeCompute::render_blend`: up to 16 flat-coloured triangles drawn in order into an RGBA8 target, each with its own colour write mask (any subset of R, G, B, A) and either blending disabled or separate RGB and alpha blend factors (Zero, One, source and destination colour and alpha, and their one-minus forms) and operations (Add, Subtract, ReverseSubtract, and Min and Max, which take the unweighted values). Checked on the GPU with Metal validation in 39 passes over two original overlapping triangles: a clear-only pass; blending disabled; each source factor with the destination term zeroed and each destination factor with the source term zeroed; every operation; additive clamping; separate alpha factors; a separate alpha operation; eight write masks (each channel alone, RGB, none, RGBA, and RG with blending); and a pass whose first draw is itself blended and masked. Colours are compared per channel against images derived independently (in exact fractions) from the public Metal blend equations, which a new CPU reference in `xenora-ref` also reproduces. The tolerance was fixed before measuring: channels whose exact result is a whole 8-bit value must match exactly, and channels that need rounding may differ by 1. On an Apple M5 Pro every rounded channel matched round-to-nearest exactly. Synthetic fixtures only; no console blend state is involved.
- Added indexed triangle draws. Native Metal: `NativeCompute::render_indexed` issues one real `drawIndexedPrimitives` call per draw (up to 16) on a 16- or 32-bit index buffer with a start index and a signed base vertex, into an RGBA8 target; a draw can sample an original BC1 texture uploaded natively. Before encoding anything it rejects the whole pass for a bad index count, an index range past the buffer, any `index + base vertex` outside the vertex buffer, and, as this API's policy, a raw all-ones index (the primitive-restart value, which Metal does not define for triangle lists). Loader: a synthetic, CPU-side index buffer binding (resource handle and offset, the view running to the end of the resource) with `cmd_set_index_buffer` and `cmd_draw_indexed_instanced`, which fetch one triangle, drawn once, through the index buffer, start index and base vertex and pass it to the existing one-triangle backend; each drawn triangle replaces the previous output. Recording rejects formats other than DXGI `R16_UINT` and `R32_UINT`, offsets not aligned to the index size or past the resource, and unknown resources. Execution checks each draw in turn, so earlier draws have already rendered when a later one fails: index counts other than 0 or 3, more than one instance or a nonzero start instance, a missing index buffer, indices past the buffer and an `index + base vertex` outside the vertex buffer are typed errors; a valid zero index or instance count draws nothing. All-ones indices are ordinary indices there. Checked on the GPU with Metal validation in 16 passes (both index sizes, 32-bit indices above 16 bits, odd 16-bit start, positive and negative base vertex, reversed winding, flat and textured two-triangle draws, draw order, BC1-textured draws), compared per pixel against images derived independently from the geometry, the public D3D12 and Metal index semantics and the public BC1 rules, which a new CPU reference in `xenora-ref` also reproduces; the loader's fetch is checked against literal positions with synthetic guest-backed buffers and through the native Metal renderer. Synthetic fixtures only; no console index-buffer views, guest programs or console draw state are involved.
- Added a `cpu_decode` cargo-fuzz target for the static x86-64 decoder used by the read-only PE probe. It places up to 4,096 fuzz bytes as the code section of a fixed, original PE32+ header and runs `probe_pe_bytes` twice with a bounded `ProbeConfig` (4,096 bytes, 2,048 instructions, 1,024 worklist entries); nothing is executed. It checks that the two reports are identical and that every decoded instruction is 1 to 15 bytes long, lies inside the code and matches the input bytes. A startup self-check aborts the run if the synthetic header stops reaching the decoder. A 600-second local campaign (nightly cargo-fuzz, one fork-mode worker, seeded from instruction bytes in the loader's probe tests; about 2.38 million executions, coverage 1,350 edges) found one decoder defect, reported to the CPU owner on #74 and not fixed here: the probe accepts unlimited FS/GS segment prefixes and reports instructions longer than 15 bytes (minimized input: 16 bytes of `0x65`). Run it with `rustup run nightly cargo fuzz run cpu_decode`; `scripts/fuzz.sh` does not list it yet.
- Added a `pe_parse` cargo-fuzz target that feeds malformed bytes to the PE32+ parser (`ParsedPe::parse`, `rva_to_file_offset`) and, only when the summed section mapping size and `SizeOfImage` are both at most 64 MiB, to the checked guest-memory mapping step (`load_into_memory`); nothing is executed. A 600-second local campaign (nightly cargo-fuzz, fork mode, seeded from the repository's synthetic PE fixtures, about 120,000 executions, coverage 267 edges) found two loader panics, reported to the CPU owner and not fixed here: truncated optional headers panic on out-of-range slices in `parse`, and `rva_to_file_offset` overflows `u32` when a section's address plus size exceeds 4 GiB. Run it with `rustup run nightly cargo fuzz run pe_parse`; `scripts/fuzz.sh` does not list it yet.
- Added `NativeCompute::run_words_with_group_width` and `max_group_width` to run a trusted kernel at an explicit threadgroup width, rejecting zero and widths above the device or pipeline limit. Native tests check 91 legal width and grid-size pairs (widths 1 to 1,024, including non-divisible grids) against independently computed outputs, and confirm the dispatched group sizes, including trimmed last groups. On an Apple M5 Pro, in three release runs of a 4,096-thread integer-mixing kernel (30 samples per width), widths 64, 128 and 256 were within 0.3 % of each other in two runs, at a median of 58.8 µs of GPU time. Width 512 was 5 to 6 % slower and width 1,024 about twice as slow in all three runs. The first run was about 4.7 times slower overall for reasons not determined, and in it 128 and 256 were 3 to 4 % faster than 64. The default width of 64 is unchanged. This was measured on one device and one workload.
- Added a native Metal depth pass, `NativeCompute::render_depth`: up to 16 triangles drawn in order into an RGBA8 target with an optional `Depth32Float` attachment, a chosen clear depth, a per-draw depth compare function (all eight) and depth write enable. A pass without an attachment accepts only `DepthState::DISABLED`. Colour and the stored depth are both read back; the depth values are returned as the texture read gives them, without numeric conversion. A draw can sample an original BC1 texture whose blocks are uploaded to a native `BC1_RGBA` texture without CPU decoding, gated on `supportsBCTextureCompression`. Checked on the GPU with Metal validation in 30 passes of original overlapping triangles: clear, every compare function at three depths, write enabled and disabled, depth test disabled (`Always` without writes) with an attachment and no attachment at all, and a BC1-textured draw. Colour and depth are compared per pixel against expectations derived independently from the geometry, the Metal compare semantics and the public BC1 rules, which a new CPU reference in `xenora-ref` also reproduces. These are synthetic fixtures only; no console depth formats or guest render state are involved.
- Added a CPU BC7 block decoder in `xenora-texture` for all eight modes, with explicit handling of the invalid all-zero mode byte. It is checked by hand-derived tests and 430 known-answer vectors from a separate clean-room reference written from the Khronos Data Format Specification (merged in PR #224 without a changelog entry).
- Added independent negative tests for gfx1030 extension words: truncated literal, SDWA, DPP16, DPP8 and MIMG NSA trailers (62 prefixes of 18 LLVM-assembled instructions), disallowed extensions, and length recognition kept apart from semantic validity. Tests only, with no decoder change (merged in PR #231 without a changelog entry).
- Added the gfx1030 Wave32 VOPC compares `v_cmp_{lt,eq,le,gt,ne,ge}_{i32,u32}_e32`. VCC_LO is EXEC_LO & the per-lane test, always fully written, so empty EXEC gives 0. A compare result can feed `s_cbranch_vccz`, `v_mov_b32_e32 vN, vcc_lo` and another compare's source; nothing else, so EXEC never becomes data-dependent. A data-dependent `s_cbranch_vccz` is a wave-uniform branch over integer vector ALU code, lowered to a select at the join. The Metal kernel builds each mask by recomputing the 32 lanes of its own RDNA wave, with no SIMD-group, threadgroup or barrier assumption. The reference now runs lockstep Wave32 waves. Verified on an LLVM-assembled fixture plus 251 original programs (boundary operands, partial and empty EXEC, both branch edges, a partial final wave) against exact integer definitions: translator, reference and second interpreter agree on CPU, and all 252 match on native Metal with validation. LLVM agrees on every encoding. VOP3, SDWA, DPP, `v_cmpx` and Wave64 stay rejected. See `docs/shaders/vector-compare.md`.
- Added the gfx1030 SOP2 forms `s_add_i32` and `s_sub_i32`, with SCC as signed overflow as defined in the AMD RDNA 2 ISA guide (sections 5.3 and 12.1). They are checked on CPU and on native Metal with validation on 400 boundary programs, four immediate-operand overflows and the scalar ALU fixture, plus eight hand-worked overflow and no-overflow cases on CPU.
- Added the gfx1030 SOP2 scalar forms `s_add_u32`, `s_sub_u32`, `s_addc_u32`, `s_subb_u32`, `s_and_b32`, `s_or_b32` and `s_xor_b32`. They are decoded, executed by the reference and independent interpreters, and resolved by the translator, with SCC as carry, borrow or nonzero result. `v_mov_b32_e32` can now read a normal SGPR. Verified on 1,400 original operand vectors plus an LLVM-assembled fixture, on CPU and on native Metal with validation. The other SOP2 forms stay unsupported. See `docs/shaders/scalar-alu.md`.
- Added `scripts/audit/test_inventory.py`, which lists the Cargo test inventory of two commits in temporary detached worktrees and reports removed, added, newly ignored and newly un-ignored tests, failing on any loss not explained in an `--allow` file. Run against the old PR #166 head 5450612 and main 951189c, it reports the 12 BC texture tests that head was missing (exit 1); the same commit against itself reports no change. Default features only; see `docs/testing/test-inventory.md`.
- Added complete gfx1030 instruction metadata generated from AMD's public RDNA 2 XML: 1,465 base forms across 21 encoding families with fields, operands, flags and allowed extension words. A committed, normalised snapshot lets `crates/isa/generate_rdna2_meta.py --check` verify the generated table offline; CI runs it.
- Added `xenora_isa::meta::identify`, which finds any gfx1030 instruction's form and exact length, including literal, SDWA, DPP and NSA words, and rejects truncated, unassigned and disallowed-extension encodings. It agrees with every LLVM-assembled fixture and with LLVM 23.1.6 on a probe set covering each family and extension word. Identification does not make an instruction executable: `SEMANTIC_FORMS` lists the forms with semantic decoding (46 as of the VOPC compares).
- Added `xenora shaders coverage [--json]` and `xenora shaders inspect <file>` for per-family coverage and stream listings that separate decoding failures from unsupported semantics. Each instruction is decoded at its stream offset and branch targets are checked against the whole stream's boundaries, so supported branches are no longer misreported as unsupported operands. `xenora_decoder::decode_at` decodes a single instruction for such callers.
- Added an agent task queue for the cpu, gpu, runtime, shared and qa areas: an *Agent task* issue form with priority and interface fields, a workflow that decides `/claim`, `/release`, `/reassign`, `/progress`, `/blocked`, `/unblock` and `/approve-acceptance` comments one at a time (dependency, cycle and file-overlap checks, 24-hour leases renewed by progress), a pull request check (linked claimed task, file ownership, overlap with other pull requests, independent `/verified <head sha>`), `scripts/agent-queue.sh` and `scripts/agent-sync.sh` (upstream fetch every 12 minutes, task branches in separate worktrees) and a machine-readable queue summary. The rules are standard-library Python in `scripts/agent_queue/` with unit tests in `tests/agent_queue/`; issue text is never executed and acceptance commands need maintainer approval. Only comments that start with a command start a job, so ordinary pull request comments never re-run verification, and only an accepted `/verified <full head sha>` evaluates the check again; and each claim run decides every unanswered command in order, so commands whose pending runs GitHub cancelled are still answered. See `docs/coordination/agent-tasks.md`. Unit tests pass locally; the workflows have not run on GitHub yet, and the pull request check stays advisory (it never blocks a merge) until a live smoke test and the migration of existing issues are done.
- Added explicit claim-before-work rules for contributors and coding agents to `AGENTS.md`, `CONTRIBUTING.md` and the pull request template: check for existing claims, claim with `/claim`, keep a 24-hour lease alive with `/progress`, `/release` when stopping, and link the claimed issue. Without the claim workflow, the earliest valid claim comment wins.
- Added a CPU decoder for a bounded subset of BC6H (signed and unsigned HDR) in
  `xenora-texture`: the four single-subset modes (Khronos modes 3, 7, 11 and 15, Microsoft
  modes 11 to 14), listed in `BC6H_SUPPORTED_MODES`, plus the reserved modes, which decode to
  zero as specified. The ten two-subset modes are rejected with `Bc6hError::UnsupportedMode`
  and are not implemented. Tests check hand-derived values and 128 known-answer vectors from a
  separate clean-room reference decoder.
- Added native Metal upload and readback of BC4 and BC5 textures (UNORM and SNORM):
  `xenora-texture` gains a pitched `BcChannelUpload`, and `NativeCompute::read_bc_channel_upload`
  sends the original blocks to `BC4_R*` or `BC5_RG*` textures when the device reports BC
  support, failing with `supportsBCTextureCompression` otherwise. GPU tests cover six- and
  eight-value interpolation, equal and extreme endpoints (signed -128 reads as -1.0), red-green
  channel order, padded row and slice pitches, two layers and two mips against the CPU decoders
  and hand-derived anchors. On an Apple M5 Pro interpolated codes were within 1.71 codes of the
  exact values and up to two codes from the truncating CPU reference.
  Edge levels of 5×3 and 1×1 and invalid payloads (zero extents, short, oversized or
  block-count-mismatched buffers) are covered.
- Added native Metal upload and readback of BC2 and BC3 textures: `xenora-texture` gains a
  pitched `BcAlphaUpload` for 16-byte blocks, and `NativeCompute::read_bc_alpha_upload` sends
  the original blocks to `BC2_RGBA` or `BC3_RGBA` textures when the device reports BC support.
  GPU tests cover explicit and interpolated (six- and eight-value) alpha, both colour endpoint
  orders, padded row and slice pitches, two layers and two mips against the CPU decoders and
  hand-derived anchors. On an Apple M5 Pro the native decoder's interpolated values were within
  8/7 (about 1.14) codes of the exact values, so up to two codes from the truncating CPU reference.
  Edge levels of 5×3 (one block row) and 1×1 are covered, invalid payloads (zero extents,
  short, oversized or block-count-mismatched buffers, bad pitches) are rejected before upload,
  and `MetalError::is_capability_unavailable` separates a device without BC support from an
  implementation failure.
- Added CPU block decoders for BC2, BC3, BC4 and BC5 (UNORM and SNORM) in `xenora-texture`, with tests whose expected texels were derived by hand from the Microsoft and Khronos block-compression specifications. Interpolated values truncate toward zero, as the existing BC1 decoder does; GPU upload and sampling of these formats are not implemented yet.
- Added `dbt::tests::sib_store_executes_through_the_native_dbt`, an independent DBT execution
  test that runs the SIB store form `mov [rsp + 0x20], rbx` through the native ARM64 DBT and
  verifies the value lands in guest memory.
- Added 8/16-bit MOV memory-operand support (opcodes 0x88/0x8A, register-to-memory and
  memory-to-register) to the native ARM64 DBT.
- Added GS/FS legacy segment prefix support to the native DBT and the bounded x86-64
  interpreter, including `CpuState.gs_base`/`CpuState.fs_base` offsets added to the effective
  address computation.
- Added Win64 TEB allocation and `gs_base` initialization to synthetic guest startup.

### Changed
- Registering an import key twice is now an error instead of silently replacing the first service, ordinals are distinct from names, and API sets in the import generator match exactly instead of by prefix.
- `GetSystemTimeAsFileTime` now stops the guest on a null pointer, as on any other invalid pointer, instead of setting the last error.

- Simplified the claim rules in `AGENTS.md`, `CONTRIBUTING.md` and the pull request template at the maintainer's request: claim an issue by replying on it in plain words, never claim over an existing claim, and treat a claim as stale only after about one week without a pull request. The task-queue lease (`lease_hours` in `.github/agent-queue.json`) is now 168 hours to match, and `docs/coordination/agent-tasks.md` marks the queue commands as optional.
- The static PE probe now marks x86-64 SIB memory addressing as DBT-supported, matching the
  native DBT.

### Fixed
- Legacy queued copy completion now publishes depth-resource barriers alongside
  buffer/color barriers, so subsequent depth readback and synchronization observe
  the validated state transition.
- Validate COMMON/PRESENT barrier starting states exactly like other states.
  Stale transitions and list replays now fail without committing resource state,
  presentation or history. Regression coverage reproduces the prior wildcard.
- Revalidate public native RGBA8 texture descriptors at allocation and mip-extent
  boundaries, including per-axis dimensions and mip counts. Malformed packed-row
  descriptors no longer overflow during construction; uploads reject them before
  changing native texture contents.
- Bound RGBA8 textures now determine loader target dimensions, preserving full
  target clears and row layout when per-draw viewports are smaller. Legacy
  callbacks reject layouts they cannot preserve instead of packing incorrect rows.
- Reject unsupported loader resource layouts, zero dimensions and overflowing
  storage/backing ranges before allocation. Buffer and single-level RGBA8 2D
  resources have explicit 64 MiB per-resource and 256 MiB per-context storage
  limits; allocation errors are recoverable and destruction reclaims the budget.
  These are bounded loader limits, not Metal hardware capabilities.
- Non-indexed loader draws now honor the starting vertex and vertex stride, validate
  each position read against the vertex resource (including checked address arithmetic),
  and skip zero-instance draws. Unsupported vertex counts and instancing fail explicitly
  instead of silently drawing one triangle. Regression tests reproduce the prior wrong
  geometry, missing zero-instance handling and unchecked vertex selection; padded layouts
  are also exercised by the native Metal regression target.
- Made the static probe and the binder share one bounded import parser, after the truncated-name fix for #270. Truncated, unterminated, empty or non-ASCII names, reserved bits and endless tables are reported by the probe and refused by the binder.
- Fixed import binding writing some slots before finding an unresolved import, and service calls taking effect before an unreadable return address was noticed. A service that changes a nonvolatile register, including XMM6 to XMM15, now stops the guest (part of #201).
- Fixed older import-generator templates that reported success for unimplemented wrappers, used a nonexistent exit reason, and read floating-point arguments from integer registers.
- Fixed guest flags in the native DBT (#64, #70). They lived only in host NZCV, so they were lost across helper calls and block boundaries and register ADD, SUB and XOR set none. JB, JAE, JBE and JA used ARM carry polarity, which is inverted for subtraction. Flags are now stored in the guest state.
- Fixed the static probe reporting support for forms an engine rejects, including NOP, TEST, most conditional branches and the FF group in the interpreter.
- Fixed the native DBT's `SXTW` encoding, which set bit 3 of the source register and was right only for registers that already had it set; the previous test used such a register by chance.
- Fixed 32-bit guest stores and loads in the native DBT (MOV 89/8B and C7 /0), which accessed eight bytes: stores overwrote the next four guest bytes and accesses at the end of a mapping faulted. RIP-relative operands of byte moves and extensions, which were read as RBP-relative, and LEA with a segment prefix, which added the segment base, are fixed too.
- Fixed an x86 prefix-length test that read an untracked fuzz artifact and so failed on every fresh checkout; its input is now inline.
- Fixed agent-queue `apply_actions` crashing on comment-only decisions (claim refusals) by skipping no-op `gh issue edit` calls; accept `summary --now` after the subcommand; grant the audit job `pull-requests: read` so live audits can list open PRs (issue #180).

- Fixed `xenora_isa::meta::identify` reading six VOP3 interpolation forms (`v_interp_p1_f32_e64`, `v_interp_p2_f32_e64`, `v_interp_mov_f32_e64`, `v_interp_p1ll_f16`, `v_interp_p1lv_f16`, `v_interp_p2_f16`) as 12 bytes when SRC0 is 255. In these forms SRC0 holds attribute bit-fields, not an operand selector (AMD RDNA 2 ISA guide section 10.4.2; XML operand type `OPR_ATTR`), so SRC0 = 255 no longer adds a literal word. The six corpus probes are now 8 bytes, as LLVM 23.1.6 also decodes them. The six pinned `mismatch` rows in the LLVM corpus check are gone: one now agrees fully, and five are `operand_flagged` (same instruction and length; LLVM flags an operand).
- Updated `test_probe_static_analysis_does_not_execute_code`: the SIB store `mov [rsp + 0x20],
  rbx` at RVA 0x1004 is now genuinely supported by the native DBT (verified by the independent
  execution test above), so the probe reports no blocker for this program.
- Added `test_probe_static_analysis_detects_genuinely_unsupported_instruction` with a new
  synthetic fixture containing an INT3 at RVA 0x1004, preserving blocker-detection coverage
  for genuinely unsupported DBT instructions.

## [X18] - Automated Import Compatibility Generation & Metadata Ingestion

Guest execution uses original, purpose-built test programs. Commercial Xbox game execution
remains unsupported.

### Added

- Added `xenora-import-gen` deterministic code generation crate and `xenora imports generate <--pe <exe> | --csv <file>> [--out <dir>]` CLI command. Automatically extracts imported symbols from PE executables or CSV manifests, classifies them against a built-in offline database of documented public Win32/CRT and ISO C specifications without consuming AI tokens or network resources, maps compatible CRT functions to native macOS libc ABI adapters with guest memory bounds checking, generates logged Win32 service stubs, generates deterministic conformance test suites, and produces coverage reports in both Markdown and CSV format.
- Added automated metadata ingestion (`ingest_standard_system_headers`) and `xenora imports metadata sync` command capable of parsing real POSIX/C system headers into typed `MetadataCatalog` definitions (188 entries).
- Added `xenora imports verify` automated behavioral verification runner checking CRT math, string, and memory bounds invariants across 12 independent behavioral test cases.
- Added explicit failure semantics for unsupported stubs returning `ExitReason::IllegalInstruction(0xFE)` rather than fabricated success.
- Added public registration helper `ImportRegistry::register_with_status` and runtime integration test verifying generated bindings and stub failure semantics.
- Added 8-bit and 16-bit guest memory access helpers (`read_u8`, `write_u8`, `read_u16`, `write_u16`) to `xenora-loader::GuestMemory`.

- Added guest-facing D3D12-style graphics runtime interfaces (`GraphicsContext`, `GraphicsDevice`, `GraphicsCommandQueue`, `GraphicsCommandAllocator`, `GraphicsCommandList`, `GraphicsResource`) with explicit backend selection (`GraphicsBackendId::MetalNative` vs `GraphicsBackendId::SoftwareReference`). Original PE32+ integration tests verify device creation, generation-checked handle ownership, command recording, barriers, and presentation.
- Connected guest execution via ARM64 DBT and IAT dispatch to native Apple Silicon Metal rendering (`test_guest_dbt_graphics_pipeline_metal_render_dual_inputs`), proving that different guest vertex positions and clear colors produce distinct, verified GPU-rendered framebuffers with active Metal validation on Apple M5.
- Added explicit test graphics service exports (`xenora_fixture.dll!CreateGraphicsDevice`, `xenora_fixture.dll!SubmitGuestDraw`) cleanly separated from unsupported production Xbox ABI claims.
- Added control-flow-aware x86-64 static PE probe using worklist CFG traversal, eliminating false-positive startup blocker reports on unreachable INT3 padding.
- Added `xenora imports sync` command with `--check` verification mode for automated, idempotent import compatibility backlog generation.
- Added current-process handle and ID queries and a readable priority boost policy for the synthetic guest process. Original PE fixtures verify Win64 arguments, state changes and errors through imported calls.
- Static import reports identify canonical bindings, API-set aliases, implemented and partial handlers, existing stubs, and missing services.

- Added a single startup path for original PE32+ fixtures that maps and rebases the image, binds imports before running thread-local storage callbacks, then enters the guest.
- The static PE probe compares ordinary imports with the standard synthetic registry and exports unresolved DLL, symbol or ordinal, and import slot addresses to `missing_imports.csv`.

- Added synthetic PE32+ runtime support for 64-bit base relocations, thread-local storage setup and callbacks, Windows x64 calling convention arguments, and import binding to a limited service registry. The Xbox-style allocation service maps guest memory. Its free entry point remains a stub.
- Extended native ARM64 dynamic binary translation with selected memory addressing, immediate stores, register increments and decrements, and indirect control flow through guest import slots.
- Added runtime fixtures for rebasing, ordered callbacks, deterministic timing, guest memory allocation, and mock import dispatch. The GPU gate checks every pixel from a native offscreen draw through a synthetic import.

- Added a static PE compatibility probe that exports section, relocation, thread-local storage, import and bounded x86-64 entry-point analysis. Synthetic tests cover inspection and report export.
- Added native Apple Silicon ARM64 Dynamic Binary Translation (DBT) engine (`DbtEngine`) with basic block decoding, native ARM64 code lowering, and W^X-compliant JIT code cache (`JitMemory`) utilizing macOS `MAP_JIT` and `pthread_jit_write_protect_np`.
- Added an ARM64 machine instruction assembler (`Arm64Assembler`) supporting register/immediate arithmetic, logical operations, memory loads/stores, branch/call/ret control flow, and calling convention preservation.
- Added synthetic guest graphics service dispatch connecting guest x86-64 execution through native ARM64 translated blocks directly to Xenora's Metal graphics pipeline (`NativeCompute::render_rgba8`), producing and independently checking the first guest-rendered frame.
- Added Scale-Index-Base (SIB) memory addressing mode support to `xenora-loader`'s x86-64 interpreter (`[base + index*scale + disp]`) for general-purpose memory operand resolution.
- Added synthetic test fixture `test_sib_addressing_mode` verifying SIB stack-relative memory loads and stores in software interpretation.
- Added `xenora-loader` crate providing standalone PE32+ (64-bit Portable Executable) parsing, isolated guest virtual memory with strict page permissions (Read/Write/Execute), bounded software x86-64 instruction interpreter (`CpuState`, `Interpreter`, `RFlags`), and read-only diagnostic triage (`triage_file`, `triage_bytes`).
- Added CLI inspect command (`xenora inspect <path>`) for read-only PE32+ header and section structure examination.
- Added comprehensive integration tests verifying synthetic PE32+ image mapping, register arithmetic, stack push/pop, subroutine calls/returns, deterministic unsupported opcode rejection, memory fault on unmapped access, and execution budget enforcement.
- Added native Metal sampler abstraction (`MetalSampler`) supporting nearest-neighbor minification/magnification filtering, clamp-to-edge addressing, normalized coordinates, validated LOD bounds, and explicit `SamplerDesc` mapping.
- Added native Metal RGBA8 2D texture allocation, mip upload validation (`MetalRgba8Texture`, `Rgba8MipLevel`, `Rgba8TextureDesc`), and texture/sampler pipeline binding definitions (`BoundSampledTexture`, `BindingSlot`) for future translated shader stages.
- Added an independent pure-Rust CPU texture sampling reference for deterministic RGBA8 2D textures with nearest-neighbor filtering, normalized UV coordinates, clamp-to-edge addressing, explicit mip selection, and bounds validation.
- Added typed texture sampler descriptors and texture metadata validation supporting nearest filtering and clamp-to-edge modes for future pipeline integration.
- Added a persistent resource service for original fixed-base PE32+ fixtures. Generation-checked handles support buffer updates, checked submission views, and native Metal readback. Queued work captures its input across updates and slot reuse.
- Added translated gfx1030 vertex and pixel stages for a bounded graphics subset. Original shaders exercise position export, a vertex transform, constant color and perspective interpolation in offscreen Metal pixel comparisons on Apple M5.
- Added immutable guest texture handles for validated pitched BC1 uploads and native Metal readback. Tests cover mip levels, layers, partial edge blocks and transparency.
- Added an acceptance matrix for persistent resources and translated graphics, with a CPU test for explicit cross-lane instruction rejection.

- Added a synthetic PE32+ guest compute import. The guest submits a checked buffer and one-shot descriptor, then reports the result of an original gfx1030 XOR fixture translated to Metal. The Apple M5 gate checks repeatable readback, guest exit status, malformed requests and stale-handle rejection.
- Added native Metal upload and texel readback for validated synthetic BC1 subresources. The Apple M5 fixture checks 61 RGBA8 texels across mip levels and array layers using authored Metal Shading Language.
- Added statically known partial Wave32 execution masks for supported vector arithmetic and moves, with inactive-lane value preservation. An independent CPU decoder and an Apple M5 readback check exercise 67 values across wave boundaries.
- Added a CPU-only synthetic tiled texture reader with checked mip and array-layer addressing and BC1 block decoding.
- Added validated input buffer views for synthetic guest compute. An original translated gfx1030 shader read only the selected range and returned the expected bytes in a local Apple M5 test.
- Added forward Wave32 branches on statically known execution and vector condition masks. A local Apple M5 test checked that inactive vector operations preserve the input to a later arithmetic right shift.
- Added bounded named import resolution for an independently compiled synthetic PE32+ guest. The fixture checks guest memory transformation and guest exit status. An unresolved import fails before guest entry.
- Added synthetic guest fixtures for service dispatch and nested function calls. The X6 fixture checks both writable guest memory and instruction retirement after loading its fixed-base PE32+ image.

- Added a public documentation index for architecture, validation, and contributor references.

- Added a bounded x86-64 guest interpreter on native ARM64 macOS (X1), plus a loader for original fixed-base 64-bit Portable Executable (PE32+) fixtures and an experimental add-one service (X2).
- Added guest-to-GPU command submission for a validated synthetic compute workload (X3) and guest-driven Metal rendering to an offscreen 8×8 RGBA8 texture (X4). Independent review confirmed the compute output and every pixel for both rendering inputs on Apple M5.
- Added a typed shader intermediate representation and Metal Shading Language generation for a bounded subset of 32-lane AMD RDNA 2 compute on the desktop gfx1030 target, with fixture validation on Apple M5. Expanded the verified vector-register-only forms to include bitwise OR and logical shifts. Arithmetic right shift passed local CPU and Metal checks, with independent acceptance still open.
- Added independent CPU known-answer fixtures, LLVM decoder comparisons, and source and native semantic mutation checks.
- Added an arithmetic-right-shift (ASHR) entry to the synthetic fixture provenance manifest and extended the independent CPU oracle and known-answer suite with explicit checkpoints, a positive-value control, aliasing and malformed-stream coverage for `v_ashrrev_i32_e32`.
- Added a correctness-gated benchmark harness that records stage timings, power source, and thermal state.
- Repository contribution guidelines, issue forms, pull request checklist, community expectations and security reporting policy.

### Changed

- Added native ARM64 CI and repository audits for dependency direction, claims, and generated status. Expanded CI to check the standalone guest experiment, synthetic PE fixture bytes, and progress asset consistency.

- Made reading and updating the changelog mandatory for engineering work.

### Fixed

- Stop ignoring `fixtures/pe/trusted-runner.exe`. The runner embeds it with `include_bytes!`, so ignoring the file made a fresh checkout fail to compile.
- Corrected the ARM64 STP pre-index encoder, which had a hardcoded -32-byte immediate and therefore adjusted the host stack by the wrong amount once the translator frame grew to 48 bytes. The wrong frame silently corrupted the caller's stack and caused SIGSEGV in every native translation test. Emitter encodings are now checked against LLVM's assembler, and a test asserts that each translated block opens and closes the same 48-byte frame.
- Translated guest memory faults now stop the block before it commits further architectural effects. A faulting push, call or store leaves guest RSP and registers unchanged, the reported instruction count covers only completed work, and guest RIP identifies the faulting instruction. Previously the block continued after the helper recorded a fault, so later instructions and writes still took effect.
- Compute the static PE probe SHA-256 from the full input, including trailing bytes.
- Report TLS callbacks that halt or exhaust their execution budget as failures. Reject unsupported 32-bit increment and decrement forms in the native translator.

- Reject unknown guest experiment arguments with a nonzero exit status.

- Validate branch targets, buffer bindings, and program termination throughout a shader fixture, including instructions skipped by a resolved branch.
- Include arithmetic right shift in fixture reassembly checks and let bootstrap continue when optional LLVM discovery fails.
- Corrected the gfx1030 three-way comparison and rejection counts in the instruction-coverage documentation to match the current LLVM test output.

### Removed

- Removed the superseded Phase 01 continuation prompt and redundant planning instructions. Candidate verification records remain in the assurance documentation.

- Removed the speculative seven-phase roadmap from the README.
