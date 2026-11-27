# Interfaces

## gfx1030 compute fixtures

The first input is little-endian gfx1030 machine code for a compute fixture with optional statically resolved forward scalar branches. The synthetic ABI starts with full Wave32 EXEC and reserves vector general-purpose register (VGPR) 0 for the grid index used by buffer accesses. Resource operands select documented synthetic bindings. The parser must require explicit target identification and validate all lengths before allocating. Console containers and AMDHSA initialization require separate contracts.

The production decoder takes borrowed code bytes. Its output records typed instructions, operand classes, original offsets and byte lengths. Unknown encodings and unsupported forms fail with a byte offset. The reference interpreter takes the original bytes and input words and performs its own decoding. It shares no production instruction definitions.

The compiler lifts decoded instructions to typed operations and verifies register initialization, supported resource offsets and termination before generating MSL. The initial subset permits only lane-indexed 32-bit buffer accesses against two disjoint buffers with equal lengths. The input is read-only. Each invocation may write only its own output word. The compiler and reference check instruction boundaries, termination placement and buffer bindings throughout the stream. Register initialization and transfer state are checked on the path selected by compile-time known forward scalar branches. Dynamic branches, workgroup barriers and cross-lane operations remain unsupported.

The Wave32 execution mask (EXEC) starts full. A supported scalar move can
assign a known 32-bit value to its low half. Each bit selects the corresponding
logical lane, numbered by the one-dimensional grid index modulo 32. Partial
masks limit vector arithmetic and moves to selected lanes. These instructions
must write an initialized destination so inactive lanes can retain their values.
Empty EXEC suppresses vector arithmetic and moves. Reachable vector memory
operations require full EXEC. Forward zero/nonzero branches on EXEC or the vector condition
code (VCC) test only their low halves. VCC must be initialized before use.
EXECNZ/VCCNZ are compute-only; graphics rejects even skipped occurrences.
Lane-derived VCCNZ uses the same bounded integer-vector region restrictions
as VCCZ, with complementary branch and register-join polarity.

The Wave32 compute contract also supports `V_MBCNT_LO_U32_B32` and
`V_MBCNT_HI_U32_B32` with the existing checked VOP3 source modes. LO counts
SRC0 bits below the logical lane and adds SRC1 with 32-bit wrapping; HI copies
SRC1 in Wave32. EXEC gates destination writes, independently of the counted
bits. Comparison expressions recompute the operation for each logical lane.
Graphics rejects both forms, including unreachable occurrences. This adds no
cross-lane data movement or Wave64 support. See the
[wave-operation contract](shaders/control-wave-memory.md#s34-wave-and-lane).

The native backend exposes `NativeCompute::new()` and `run_words(source, entry, input)` for trusted MSL. It binds input at index 0 and output at index 1, dispatches one invocation per word and returns owned readback after completion. `run_words_profiled` returns the same output plus measured library, pipeline, GPU and host durations for the benchmark harness; those timings are single submissions, not distributions. The caller runs generated MSL in a bounded child process. Raw MSL is a trusted developer input. Errors report the operation and backend detail. Across other stages, diagnostics carry a stage, error kind, optional byte offset and human-readable context.

## Synthetic guest experiment

The separate [X1–X4 driver](../experiments/x1/README.md) loads fixed-base
PE32+ fixtures, interprets a bounded x86-64 subset and dispatches synthetic
services. X3 and X4 validate fixed-size command records from read-only
guest bytes before issuing compute or draw work. Their command formats and
Metal shaders are authored for the experiment.

The multi-function guest fixture (X6) maps PE32+ sections with separate
read, write and execute permissions. Its interpreter tracks the expected
return slot and continuation for each call. A return from the authenticated
root frame ends execution.

The named-import fixture (X7) resolves imports for one synthetic module
before guest entry. An indirect call may invoke only a resolved slot in the
guest import address table. The handler validates the guest buffer and
operates on words in checked guest memory. The command prints the guest
exit value, while its host exit code reports whether loading and execution
succeeded.

With the Metal feature, the guest compute fixture (X8) adds the
`xenora_compute` import to the same loader. RCX points to a four-byte-aligned
readable and writable three-word buffer. RDX points to a readable aligned
16-byte record containing four little-endian 32-bit fields: command 1,
handle 0, input slot 0, and word count 3. Handle 0 names the buffer supplied
on this call and is consumed after a successful dispatch. A second submission
within the same guest run fails with a stale-resource fault.
The CPU validates both spans before creating a Metal context.
It selects the original translated XOR program, waits for completion,
writes the three output words back to guest memory, and returns 3.
The guest can read the result through its existing report import.

A general guest command stream must submit validated resource handles and ranges through an explicit address translation boundary. The graphics side owns mappings and the lifetime of in-flight views. Completion and fence signaling flow back across the command boundary. Guest addresses are never host pointers.

`NativeCompute::run_guest_words` takes a host-provided immutable resource
table and a synthetic guest view. The view selects an input slot, table index,
byte offset and byte length. The backend validates slot zero, little-endian
32-bit words, alignment, bounds and dispatch size before copying the selected
range into a Metal buffer. Slot one holds a separate output buffer. The call
returns owned words after command completion or a stage-specific error.

## PE32+ fixture startup and static import reports

`xenora_loader::startup::run_synthetic_startup` takes an original, trusted
fixture, guest memory addresses, a service registry and an entry instruction
budget. It maps and rebases the image, binds imports, initializes thread-local
storage, invokes its callbacks and runs the entry point. The fixture entry point
uses a Win64-aligned stack and terminates with HLT. The result includes the loaded
guest memory and the entry execution outcome.

`xenora probe --mode static` inventories ordinary PE imports without guest
execution. `missing_imports.csv` lists unresolved keys with their import address
table relative virtual addresses. The full inventory records whether each key is registered, its implementation
status, and the canonical module behind API-set aliases. Runtime-loaded modules
and services require separate analysis. The coverage matrix groups unresolved
imports by shared startup prerequisites. Static imports do not reveal call
order. The JSON report counts import slots, DLL and symbol keys, distinct
names with DLL-scoped ordinals, and canonical registered handlers separately.
The unresolved total includes explicitly tracked unsupported APIs and other
missing handlers. Existing stubs remain visible in the full inventory.

The synthetic process registry supports the current-process pseudo handle and
its process ID. The priority boost setter and getter record and query a guest
policy. They are partial until a guest scheduler applies the policy. The process
ID lookup is limited to the current-process pseudo handle. An unsupported
handle reports `ERROR_INVALID_HANDLE` through the guest error slot. The getter
rejects invalid guest output addresses with `ERROR_NOACCESS`.

Win32 references include Microsoft's documentation for
[GetCurrentProcess](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getcurrentprocess),
[GetProcessId](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocessid),
[SetProcessPriorityBoost](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-setprocesspriorityboost),
and [GetProcessPriorityBoost](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesspriorityboost).

### Thread handles and virtual memory

The thread object model exposes one guest thread with a stable synthetic ID.
`OpenThread` issues rights-checked handles to that thread, and `CloseHandle`
invalidates real handles. A thread pseudo handle refers to the current synthetic
thread. The priority APIs store and query the thread's base-priority setting.
Thread creation, inheritance, debugger exceptions, and scheduling remain outside
this model.

`VirtualAlloc` supports combined reserve and commit for data-only pages.
`VirtualFree` supports `MEM_RELEASE` with the exact allocation base and a
zero size. The guest memory map tracks allocation ownership and bounds live
virtual allocations to 64 MiB. Decommit, executable pages, and console-specific
allocation lifecycle remain unsupported.

References include Microsoft's
[thread handle](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-openthread),
[thread priority](https://learn.microsoft.com/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadpriority),
[handle close](https://learn.microsoft.com/windows/win32/api/handleapi/nf-handleapi-closehandle),
[virtual allocation](https://learn.microsoft.com/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc),
and [virtual release](https://learn.microsoft.com/windows/win32/api/memoryapi/nf-memoryapi-virtualfree) contracts.

## Canonical sampled texture storage

`xenora_texture::PackedTextureLayout` computes a single tightly packed 2D mip chain
for `TextureFormat::{Rgba8Unorm, Bc1RgbaUnorm, Bc3RgbaUnorm}`. Each immutable layout
exposes per-mip logical dimensions, byte offset, row bytes, stored row count and
byte length. RGBA8 stores texel rows; BC1/BC3 store complete 4×4 blocks, including
partial edges and sub-block mip tails, using 8/16 bytes per block respectively.
See the [block-format specification](https://learn.microsoft.com/en-us/windows/win32/direct3d11/texture-block-compression-in-direct3d-11).

Mip dimensions halve and clamp to one. Zero dimensions, empty or excessive mip
chains and row/level/chain arithmetic overflow reject before metadata allocation.
Metadata reservation is fallible; this helper does not allocate texel storage.
Device capability, dimension limits, resource budgets and upload byte lengths
remain caller responsibilities. This canonical storage differs from fixture tile
padding and the common row/slice pitch of the older upload descriptors below.
Native and guest typed compressed-resource integration and local validation are
described below.

## Synthetic tiled textures

The texture crate reads original fixtures in an array-layer-major layout. Each
layer stores a consecutive mip chain. Within each mip, row-major 2×2 tiles store
four row-major Block Compression 1 (BC1) blocks, including slots beyond the
logical edge. The reader validates the full byte length, rejects padding
coordinates and can decode each logical block to sixteen RGBA8 texels.
The decoder uses bit replication to expand 5:6:5 endpoints and rounds integer
palette interpolation down. This layout is specific to Xenora fixtures. The block encoding follows the
[Direct3D BC1 description](https://learn.microsoft.com/en-us/windows/win32/direct3d10/d3d10-graphics-programming-guide-resources-block-compression).

`TiledBc1::linearize` strips the tile padding from one selected subresource
while retaining complete compressed blocks at logical image edges.
`NativeCompute::read_tiled_bc1` checks the pixel budget and the Metal device's
BC texture capability, then uploads the selected blocks as a single
BC1 RGBA texture. A fixed Metal compute kernel reads each texel and returns
row-major RGBA8 words. The texture path uses authored MSL and takes its
input from the host fixture.

## Persistent guest graphics resources (X9)

A separate fixed-base PE32+ test profile exposes `xenora_resource`. RCX
points to a readable, four-byte-aligned request of five little-endian
32-bit words (operation, handle, offset, length and kind). RDX carries a
guest data address for uploads and readback. Other operations require it
to be zero. Handle values encode a slot and generation. Every guest run
owns a separate eight-slot table. Slots can be reused only with a new
generation. Guest addresses and handles never expose host pointers.

| Operation | Request data |
| --- | --- |
| 1, create buffer | Length in bytes and kind 1 (compute words) or 2 (draw vertices). RDX points to initial data. |
| 2, update buffer | Handle, byte offset and length. RDX points to source bytes. |
| 3, destroy resource | Handle. A queued snapshot retains its bytes after destruction. |
| 4, queue compute | Handle, byte offset and length. The selected view binds the original translated XOR compute fixture. |
| 5, complete and read back | Exact result length and a writable RDX destination. |
| 6, queue draw | Handle, byte offset, length 96 and program ID 1 (constant color) or 2 (interpolated color). The input contains three float4 clip positions followed by three float4 colors. Readback is 4×3 row-major RGBA8. |
| 7, create texture | Length of compressed data in bytes. RDX points to eight descriptor words followed immediately by the compressed data. |
| 8, queue texture read | Handle, array layer in the offset field and mip index in the length field. Completion returns row-major RGBA8. |

The texture descriptor contains width, height, mip count, layer count,
format (1 for BC1), usage (1 for sampling), row pitch and slice pitch.
The row pitch is aligned to the eight-byte BC1 block size. Each mip
occupies one slice and uses the same pitches. Slices appear in mip order
within each layer. The whole descriptor and payload must occupy one
readable guest span. Textures are immutable. This is an original linear
upload contract for the test profile. Opcode 8 uploads the selected
subresource and uses an authored Metal texel reader.

Buffers and compressed texture payloads are limited to 4096 bytes.
Queueing captures one immutable view. Completion validates the guest
destination before submitting GPU work and waits for Metal before writing
the result. An update or destruction after queueing cannot change the
captured bytes. Guest exit drops resources and any queued view. Data persists
across guest calls in host memory and is uploaded to Metal for execution.
Persistent device allocations and GPU work in flight across guest calls
are outside this interface.

### Translated offscreen stages

`compile_graphics` decodes two original gfx1030 byte streams and links
one vertex position export, an optional four-component parameter export,
and one pixel color export. Buffer 0 supplies positions to vertex vector
registers 0–3 and buffer 1 supplies colors to registers 4–7. The pixel
stage can consume parameter zero through matching, adjacent interpolation
phase pairs. This maps to center-perspective interpolation in Metal.
The vertex stage supports a bounded set of 32-bit floating-point add and
multiply forms. The graphics ABI also supports register and finite
literal moves. Unsupported stage instructions or export combinations
fail explicitly. The Xbox graphics ABI, hardware-specific floating-point
modes, image instructions, and additional varyings remain unsupported.

## Graphics draw and blend contracts

`NativeCompute::render_indexed` issues one real `drawIndexedPrimitives` call
per draw (up to 16) on a 16- or 32-bit index buffer with a start index and a
signed base vertex, into an offscreen RGBA8 target. A draw can sample an
original BC1 texture uploaded natively. Before encoding anything it rejects
malformed draws; execution checks each draw in turn.

`NativeCompute::render_blend` draws up to 16 flat-coloured triangles in order
into an offscreen RGBA8 target. Each draw has its own colour write mask and
either blending disabled or separate RGB and alpha blend factors and
operations. Colours are compared per channel against expectations derived
independently from the public Metal blend equations.

`NativeCompute::render_depth` draws up to 16 triangles in order with an
optional `Depth32Float` attachment, a chosen clear depth and a per-draw depth
compare function. Colour and the stored depth are both read back.

These passes are checked on the GPU with Metal validation against expectations
derived independently from the public semantics; synthetic fixtures only.

### Bounded triangle-list draws

`NativeCompute::render_triangles_rgba8` accepts 3..=65,535 positions and the same
number of colors, in complete groups of three. It clears one target once and
issues one native Metal triangle-list draw. Trusted translated shaders access
one position/color record per vertex. Existing single-triangle render entry
points retain their signatures and use the same implementation.

The loader graphics backend callback now receives a slice of float4 positions.
Indexed and non-indexed native draws support up to 65,535 vertices with checked starting vertex,
stride and resource bounds. Zero-count draws do not fetch or render. Instancing
and incomplete triangles are refused. Indexed draws support 16- and 32-bit
indices, a checked buffer offset/start index, and a signed base vertex. Consecutive indexed and non-indexed draws are collected in order into
one backend invocation, bounded to 65,535 native vertices and 256 nonempty draws across the command list.
The software reference retains its 768-vertex bound.
All vertex reads finish before rendering. Once a nonempty draw is recorded,
changes to viewport, target or clear color are rejected during execution; repeated
identical settings are allowed. No-op draws do not start a pass. This batching
preserves geometry under the current position-only ABI; it is not a general
render-pass API with per-draw shaders, vertex IDs, depth or blend state.
Failed submissions preserve the last presentation and do not commit staged
resource-barrier metadata.

### Native ordered triangle passes

`NativeCompute::render_triangle_pass_rgba8` accepts 1..=256 `TriangleDraw`
records, bounded to 65,535 vertices in total. Each draw supplies positions, matching
colors and a nonzero integer viewport anchored at (0, 0) within the fixed target.
The pass clears once, issues each draw in order, and reads back once. Vertex IDs
restart at zero for each draw's buffer views. All inputs are checked before shader
compilation or encoding. Existing triangle render entry points delegate to a
single-draw pass and retain their signatures, sampling and profiling behavior.

This native boundary uses one pipeline and one RGBA8 target. It does not add
per-draw shaders or textures. Depth, blending, scissor and viewport offsets use
the stateful translated pass described below. The legacy loader vertex-slice callback continues to reject viewport changes.
The ordered loader callback described below retains per-draw viewports. Synthetic Metal tests
check viewport changes, draw order, untouched clear pixels, invalid later draws,
and the final draw at the 256-draw limit. The bounded rendering milestone received
independent local acceptance; see the [checkpoint summary](testing/local-graphics-checkpoint.md).

### Loader ordered-pass adapter and submission validation

`GraphicsContext::set_metal_pass_backend` installs a callback receiving ordered
`GraphicsDraw` records, a fixed target extent and one clear color. Installing it
replaces the legacy callback, and vice versa. A bound texture fixes the extent; otherwise the first nonempty draw fixes it
from its viewport origin plus size. Later draw viewports must fit inside it. Indexed and non-indexed draws
retain their boundaries, with a combined native limit of 65,535 vertices and 256 nonempty draws. Target
and clear-color changes after drawing starts remain unsupported. The software
reference and legacy adapter continue to reject viewport changes.

Before rendering, the loader checks the fixed target size (at most 1,048,576 native pixels or 4,096 software pixels)
and bound resource capacity. After rendering, readbacks must contain exactly
width * height * 4 bytes. Failures preserve the previous presentation. Clear-only
submissions also update a bound target's host data. Guest backing synchronization
remains separate work. Barrier metadata is staged until successful readback.

Queues retain only the most recent 256 successfully completed submission handles,
oldest first. Failed attempts do not enter that diagnostic history.

### Translated per-draw depth and blending

`TrianglePass` supplies one `TriangleState` per draw: a depth compare/write state,
optional separate RGB/alpha blend equations, and a color write mask. A finite
clear depth in 0..=1 creates a shared Depth32Float attachment for the pass; depth
state without that attachment is rejected. Pipeline variants are reused within
the pass by blend/mask settings. All fallible setup precedes encoding. Depth is
transient; this API returns color only and does not persist or expose depth data.

The loader `cmd_set_draw_state` records eight depth comparisons, an optional
write enable, Replace/Alpha/Additive blend presets, and an RGBA channel mask.
Alpha uses source alpha and one-minus-source-alpha for RGB and source-over alpha;
Additive adds both inputs on all channels. Ordered native adapters must create a
depth attachment cleared to the configured value (default 1.0) when any draw enables depth, and apply each draw's
state. The application fixture adapter and loader regressions demonstrate this
mapping; no new guest service ABI is implied. Legacy/software
adapters reject nondefault state. Reset restores default draw state. Full guest
pipeline descriptors and persistent depth remain open. Configurable clears are
available through the explicit depth-aware adapter described below.

Loader barrier metadata changes are staged, validated against prior staged state,
and committed only after valid successful rendering/readback. Failures preserve
resource states, target bytes, diagnostic history and presentation. This does not
provide asynchronous GPU fences or rollback of arbitrary backend side effects.

### Application fixture adapter

`xenora_ffi::graphics_backend::FixtureRenderer` owns a loader graphics context,
command queue/list/allocator, a bounded guest-memory region and one reusable vertex
buffer. It compiles only the original constant-red fixture shader and installs a
native ordered-pass adapter that maps loader depth comparisons, blend presets and
write masks to Metal. It accepts up to 256 ordered draws and 768 total vertices.

The C trusted-frame runner and persistent/interactive frame loops use this renderer.
A frame loop retains it on the guest thread and releases it when execution ends.
The C interface and expected fixture images are unchanged. The existing textured
resource path is separate; this does not install a general guest pipeline/shader
ABI or add commercial-game support. Native tests cover invalid-input recovery,
stateful ordered draws, 300 sequential frames, lifecycle controls, input replay and
repeated-session memory behavior through the actual application interface.

### Bounded viewport origins and scissors

`GraphicsDrawState` and `TriangleState` carry `viewport_origin: [x, y]` and an
optional `scissor: [x, y, width, height]`, in nonnegative integer target pixels.
The origin defaults to (0, 0); absent scissors cover the whole target. Each draw
sets its own scissor, so disabling it restores full-target clipping for later draws.
Zero-width or zero-height scissors suppress rasterization, including depth writes;
input validation and vertex-buffer offset advancement still occur.

Origin-plus-size and scissor bounds use checked arithmetic. Both rectangles must
fit the fixed target; overflow and out-of-bounds input fail before GPU dispatch.
For the loader, a bound texture determines the target extent. Otherwise the first
nonempty draw's viewport right/bottom edges determine it, including its origin. Native passes continue to take explicit
extents. Scissors never resize the target. Legacy/software callbacks reject
nondefault region state instead of silently ignoring it.

The application fixture adapter carries both fields end-to-end. Native tests cover
untouched pixels, per-draw reset, empty scissors before later geometry, initial
viewport offsets, malformed-region recovery and overflow rejection. Fractional or
negative viewport origins, arbitrary target resize and a general D3D12 viewport ABI
are outside this bounded interface. The bounded rendering milestone received
independent local acceptance; see the [checkpoint summary](testing/local-graphics-checkpoint.md).


### Explicit host-backed resource transfers

`GraphicsContext::copy_guest_to_resource` uploads guest bytes into a host-backed
resource in exactly COPY_DEST state. `copy_resource_to_guest` copies bytes from
a host-backed resource in exactly COPY_SOURCE state. Both take a byte offset,
guest address, byte count and guest memory; command barriers change resource
states only after successful submission. These are synchronous host APIs.

Transfers reject invalid handles, guest-backed resources, wrong states and
overflowing or out-of-resource ranges. Guest-memory permissions and the entire
span are checked before any destination bytes change, including spans across
regions. Zero-byte transfers still require a valid resource range/state and a
mapped guest address, matching GuestMemory semantics. No implicit synchronization
of guest-backed targets or texture layout conversion is introduced by these host
APIs. The separate queued whole-resource copy API is described below. Local
synthetic tests are not independent acceptance.


### Synchronous loader resource destruction

`GraphicsContext::destroy_resource` removes a resource and drops its host storage
only when no recorded command list references its handle. Vertex/index bindings,
render targets and barriers all retain references, including superseded bindings.
Both recording and closed lists are checked. Reset every referring list before
destroying a resource; successful or failed execution does not discard a list,
because closed lists can be submitted again. Live references return
`ResourceLifetimeViolation`; unknown or already destroyed handles return
`InvalidResourceHandle`, without changing resources or presentation state.

The loader submits synchronously, so no backend work remains outstanding when
execution returns. Backend adapters must preserve that completion contract.
Destruction does not unmap or alter caller-owned guest backing memory, and retained
frame pixels remain valid. Newly allocated resources receive fresh handles;
destroyed handles are not recycled by this operation. This API does not implement
asynchronous fence retirement, descriptor references, destruction of devices or
command lists, or a guest resource-release ABI. Native synthetic round-trip tests
cover destruction after completion and retained presentation pixels; fault-injected
failure tests cover reference retention. These bounded lifetime contracts are covered
by the independently accepted local graphics checkpoint; see the
[evidence summary](testing/local-graphics-checkpoint.md).


### Bounded loader resource allocation

Resource creation supports nonempty byte buffers (height 1, no RGBA8 format) and
nonempty RGBA8 2D textures. Buffers require one mip; textures permit a nonzero mip
count no greater than the full chain for their dimensions. Both require one array/depth
element. Unknown dimensions, 1D/3D textures, arrays and other texture formats return
InvalidDescriptor. Each mip stores max(1,width>>level) × max(1,height>>level) × 4 bytes;
levels are tightly packed consecutively, with checked total-size arithmetic.
Guest backing end addresses must not overflow; mapping and permissions are still
checked when accessing guest bytes, not at resource creation.

MAX_RESOURCE_BYTES is 64 MiB; MAX_CONTEXT_RESOURCE_BYTES is 256 MiB of live host
resource storage, including storage reserved for guest-backed resources. Exceeding
the per-resource limit returns InvalidDescriptor. Context-budget exhaustion or
a failed allocator reservation returns GraphicsError::Memory(AllocationFailed).
Both checks precede insertion and handle allocation. Storage starts zeroed;
destruction releases budget. These limits bound the experimental loader contract
and do not describe Metal limits or process-wide memory use. Temporary transfers,
backend allocations and retained presentation pixels are outside this budget.


### Configurable depth clear and explicit texture extent

`cmd_set_depth_clear` records a finite clear value in [0, 1], defaulting to 1.0
after reset. A different clear cannot be recorded after geometry for the same
pass. `set_metal_pass_with_depth_backend` receives that value as its fourth
argument; the existing three-argument setter rejects nondefault values rather
than ignoring them. `FixtureRenderer::render_with_depth_clear` exposes this
through the production fixture adapter. Depth storage remains transient.

A bound Texture2D resource defines the full target extent; each draw's viewport
fits within it. Clearing and readback cover the full texture, including pixels
outside smaller viewports. Without a texture target, the first draw still fixes
the extent. Legacy callbacks reject drawing with different viewport/target sizes.
Native targets are bounded to 1,048,576 pixels and 4,096 per axis; software targets remain bounded to 4,096 pixels.

### Integer vector graphics operations

Graphics supports 19 existing decoded integer vector encodings: signed/unsigned
24-bit low/high multiplication; signed/unsigned 32-bit min/max; AND, OR, XOR and
XNOR; logical/arithmetic reverse shifts; wrapping ADD, SUB and SUBREV; and the
existing VOP3 ADD encoding. Sources are captured before destination writes and
SCC remains unchanged. All supported integer VOP2 SRC0 operands accept initialized
VGPRs, ordinary scalar uniforms/locals, inline words or a literal; VSRC1 remains
a VGPR. VOP3 ADD retains its register-only operand restrictions. Pixel v0/v1 remain
reserved. No EXEC/VCC or divergent execution support is added.

Integer results use exact bit reinterpretation through the graphics register
representation, including copies, joins and bounded countdown loops. They may
feed integer arithmetic, uint color exports and supported uint storage. They
cannot feed floating arithmetic, texture coordinates, floating attachments or
vertex position/parameter exports without an explicit conversion contract.
The direct uniform-to-vector array-layer transport and original pixel-coordinate
metadata cannot be used as integer arithmetic inputs; ordinary scalar uniform
words instead enter through an integer operation's scalar source. Computed
layers, descriptors and store addresses retain their existing restrictions.

### Pixel arithmetic and stateful sampled passes

The graphics compiler supports the existing register-register FP32 add/multiply
forms in either vertex or pixel stages. Pixel v0/v1 remain reserved; inputs must
be initialized, and arithmetic must precede exports. Operations are separate and
use Metal safe/precise math plus function-start no-contraction/no-reassociation
pragmas. Local known-answer tests cover exact binary finite arithmetic and RGBA8
conversion, not general AMD rounding, denormals, NaNs or execution masks.

`render_sampled_triangle_pass_rgba8` accepts one `BoundSampledTexture` per draw
alongside the existing TrianglePass. Bindings retain their native texture and
sampler; only texture/sampler slot zero is supported. Counts and slots are
validated before shader compilation or encoding. Each draw rebinds resources and
retains its own depth/blend/mask/viewport/scissor state. Completion is synchronous.
The original sampled-arithmetic fixture and a 32-frame native scene check changing
uploads, literal pixels, overlap/depth rejection, clipping and additive blending.
This slot-zero trusted native interface remains available alongside the multi-slot
loader/native interface below. General guest descriptor heaps remain open;
independent acceptance is not implied by these local tests.


Native RGBA8 texture descriptors are revalidated at allocation and mip-extent
queries even when constructed through public fields. Each axis is bounded to
16384, with nonzero dimensions and a valid mip-chain length. Packed mip builders
use saturating pitch arithmetic for malformed inputs; upload checks still reject
invalid dimensions, pitches and byte spans before modifying native storage.
These checks are exercised under Metal validation and by CPU overflow regressions.


Barrier `before` values match the current staged state exactly, including COMMON
and its PRESENT alias. The loader does not model implicit promotion/decay by
treating COMMON as a wildcard. A stale barrier or replay fails transactionally.

The stateful sampled scene also switches its third draw to a distinct texture and
a distinct LOD-clamping sampler. Controlled local mutations that skip either
third-draw binding must fail the literal pixel assertion; the restored code passes.
This sensitivity check complements native readback and local independent review,
but does not certify all shader forms or resource synchronization.


### Multiple native color targets and pixel exports

`TriangleTargetsPass` and `render_triangle_targets_rgba8` support 1–8 RGBA8
attachments with a shared extent (at most 1,048,576 pixels per target, 4,096 per axis). Clears and
returned byte arrays are ordered by color slot. These clear-only APIs clear each
attachment once, store it and read it back after command completion. Explicit color
Load support is described below. Per-draw blend/write-mask settings
currently apply to all attachments; depth and rasterization are shared. Existing
single-target APIs retain their signatures through a one-attachment wrapper.
Independent per-attachment blend/mask state remains unsupported.

Decoded EXP instructions retain `done`, and `ExportTarget::RenderTarget(index)`
retains MRT0–MRT7. The bounded pixel ABI requires full uncompressed four-component
exports with VM set on every export, no reserved bits, one export per target and
a contiguous set starting at MRT0. Export order need not match slot order; targets
are never renumbered. Only the final reached pixel export has DONE. Forward unconditional branches may
then skip to termination; reached arithmetic stays before the export suffix. Vertex POS0/PARAM0
rules remain separate, so POS0 DONE does not prohibit the existing PARAM0 export.

These restrictions are a supported subset of the public
[AMD RDNA2 EXP definitions (§§11.1–11.2.1, 12.17, 13.9)](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content),
not claims that hardware universally forbids duplicate or sparse exports. Native
known-color tests cover two targets with different clears/depth/clipping, all eight
slots, additive blending/write masks and restoration on each slot, single-target
wrapper equivalence, and the original assembled eight-MRT shader. Invalid target counts are
rejected before compilation; malformed export sequences fail during decoding or
compilation. This does not establish Xbox equivalence or broad stage acceptance.

### Loader multiple color resources

`cmd_set_color_targets` records 1–8 distinct host-backed RGBA8 Texture2D handles
and equally many clears. All textures must have the same extent. Guest-backed
textures are rejected by this new binding API until their synchronization is
implemented. Targets must be in RENDER_TARGET state at binding and at each draw;
barriers can transition them to COPY_SOURCE after the last draw. The target set
and clears cannot change after drawing starts. Recorded bindings retain references
until command-list reset, including after failure or synchronous completion.

`set_metal_targets_backend` receives ordered draws, extent, a clear slice and depth
clear, and returns an ordered vector of RGBA8 readbacks. Existing single-output
adapter setters wrap this interface and reject multiple-output draws. Each setter
replaces the previous adapter. Clear-only submissions fill all outputs directly.
Before committing any output, the loader checks the output count and every byte
length. A failure preserves every target, pending barrier state, presentation and
queue history. `ExecutionOutcome::color_outputs` contains all outputs; `pixels`
and the displayed presentation retain slot zero. Native tests use original
translated eight-output fixtures and explicit COPY_SOURCE guest readbacks.
This host-driven loader API does not add a general guest shader/pipeline ABI.

### Retained native depth

`create_depth_target` creates a private single-sample Depth32Float surface with
1–1,048,576 pixels, at most 4,096 per axis, and initially undefined contents. `render_triangle_targets_with_depth_rgba8`
takes exclusive mutable access to it and a `DepthLoad::Clear(value)` or `Load`.
The pass's older `clear_depth` field must be None to avoid ambiguous control.
Load is rejected until a successful clear. Extent and device registry ID must match;
clear values must be finite in 0..=1. Per-draw depth state and all color attachment
behavior remain the same as ordinary translated passes.

Each submission allocates a working depth surface. Load blits the prior surface
into it before the render encoder; Clear initializes it directly. The pass stores
depth, waits synchronously for completion, reads color outputs and only then replaces
the retained surface. Validation, compilation and command failures cannot overwrite
the previously committed surface. The exclusive mutable reference and synchronous
completion permit serialized reuse across queues on the same device; this is not
asynchronous fence or cross-queue retirement support. No raw texture alias or clone is exposed. Dropping the owner releases its retained texture. Existing transient
APIs continue to clear and discard their temporary depth attachments.

The implementation follows Apple's [load/store actions](https://developer.apple.com/documentation/metal/setting-load-and-store-actions/)
and [resource synchronization](https://developer.apple.com/documentation/metal/resource-synchronization)
contracts. Native tests use original translated shaders and literal color outcomes
for retained occlusion, multiple depth updates, clear/reversed depth, read-only depth
and failed validation/compilation recovery. GPU execution failure injection is not
covered by those recovery tests. The original guest depth-resource commands are documented below.

### Loader retained depth resources

`create_depth_resource` creates a bounded 1–1,048,576-pixel depth resource (at most 4,096 per axis), initially
DEPTH_WRITE and uninitialized. `cmd_set_depth_target` binds a handle with explicit
Clear or Load (or unbinds it). Load requires a committed snapshot. A retained binding
cannot change after drawing starts and cannot be combined with the transient
`cmd_set_depth_clear` control. Depth/color extents must match. DEPTH_READ permits
Load and depth tests without writes; clears and depth writes require DEPTH_WRITE.
Barriers can transition only between these two states. Bound depth contributes its
logical width*height*4 bytes to the context resource budget, independently of temporary
backend transfer/staging allocations. Command-list references prevent destruction
until all referring lists are reset, including superseded bindings.

The stateful callback receives an immutable prior snapshot and returns staged colors
and optional depth. Adapters must not mutate prior snapshots. Snapshot presence must
match the binding, and every color count/length is checked before ownership, colors,
barrier metadata, presentation or history commit. Legacy adapters reject retained
depth. Clear-only retained passes still invoke the stateful callback. New native
DepthSnapshot owns tightly packed host bytes (transfer buffers use padded rows); snapshot_depth reads completed native depth,
restore_depth uploads it into a new surface. The loader can therefore retain and move
snapshots through its service registry without assuming native texture thread safety.
This currently adds synchronous depth upload/readback work per loader submission;
it is a correctness path, not a GPU-residency or performance claim. The synthetic guest ABI is described below; async fences and broader textured
pipelines remain open.

### Selected graphics pipelines and sampled resources

`compile_graphics_pipeline` returns compiled source plus color-target count and
texture-use metadata and a four-bit `sampled_pairs` mask derived from lifted IR.
Bit `texture_slot * 2 + sampler_slot` records a required sampling pair; the
`uses_texture` compatibility field is true exactly when this mask is nonzero. Existing `compile_graphics` keeps its
return type. Loader `create_graphics_pipeline` compiles original gfx1030 vertex/pixel
bytes before allocating a handle. Each context permits at most 256 pipeline objects,
with generated MSL bounded to 64 KiB each. Pipeline storage is separately bounded
from resource bytes. Each draw snapshots its selected pipeline; pipelines may change
within one pass while attachment counts and load actions remain fixed.

`cmd_set_graphics_pipeline` and `cmd_set_vertex_colors` record the selected pipeline
and optional float4 color/UV buffer. `cmd_set_sampled_bindings` atomically replaces
two texture handles and two independent optional sampler descriptors.
`cmd_set_sampled_texture` remains a slot-zero-only compatibility operation and does
not clear slot one. Indexed colors use the same index/base-vertex
mapping as positions. Omitted colors are zero. Sampled textures must be host-backed
RGBA8 Texture2D resources in PIXEL_SHADER_RESOURCE state; color inputs require
VERTEX_AND_CONSTANT_BUFFER. Pipelines require an exact attachment count. Texture
and sampler presence must exactly match the masks derived from `sampled_pairs`:
texture bit t is required if any pair uses t; sampler bit s if any pair uses s.
Missing and extra entries reject at each actual draw. Every used pair validates
its sampler's explicit mip and LOD range against that texture's mip chain. A sampled resource cannot also be a color target. Explicit
pipeline draws require RENDER_TARGET state even through the older target API. Legacy
backends reject these bindings at each actual draw, including when later unbound.
Command references retain pipelines, color inputs and sampled resources until all
referring lists reset, including superseded or failed lists.

`install_pipeline_backend` in xenora-ffi translates loader draw snapshots into native
Metal state. Every nonempty draw requires an explicit compiled pipeline; the
loader’s legacy pipeline-less draws remain available only to other backends. Empty
passes need no shader. The adapter validates each draw against its own pipeline metadata, uploads packed RGBA8 mip chains once
per distinct pixel snapshot/descriptor within the submission, including sharing
across texture slots. Each draw creates one native sampler per occupied sampler
slot and clones that owner across its used pairs. The adapter calls
`submit_multi_binding_triangle_targets_rgba8`, which compiles distinct source/entry
pairs once per submission and keys native pipeline variants by program, blend and
write mask. All programs are compiled before encoding; a later compilation failure cannot
publish colors or retained depth. Single-source native APIs retain their contracts.
Existing FixtureRenderer records its original constant pipeline
through this adapter. `render_bound_triangle_targets_rgba8` combines ordered sampled
draws, multiple outputs and optional retained depth. Native tests select original
sampled-arithmetic and MRT programs, vary uploads and UVs, select another mip while
reusing a texture, and distinguish retained depth updates over subsequent loads.

The synthetic guest service below exposes these selected operations. Broader
sampling/filter/address modes, general array/view descriptors, broader synchronization and
full shader instruction coverage remain open. No arbitrary trusted MSL is accepted
by the loader's pipeline creation API.

Explicit sampler mip selection pins native min/max LOD to the requested level,
overriding the descriptor's otherwise validated LOD range. Binding rejects levels
outside the texture's mip count. Native pipeline tests exercise both LOD clamping
and explicit mip one while reusing the same uploaded texture.

### Two independent native texture and sampler slots

The graphics sampling ABI maps resource SGPR bases 0 and 12 (`s[0:7]`, `s[12:19]`)
to Metal texture slots 0 and 1, and sampler bases 8 and 20 (`s[8:11]`, `s[20:23]`)
to sampler slots 0 and 1. All four pairings are supported; other bases reject.
This mapping is synthetic, not a general console resource-descriptor contract.

`submit_multi_binding_triangle_targets_rgba8(shaders, pass, bindings, depth,
color_loads)` takes one `TriangleShader` and one `Vec<BoundSampledTexture>` per
draw and returns `PendingRender`. Each vector contains 0–16 pairs using slots 0–3.
The native layer normalizes independent four-entry texture and sampler tables.
The additive [descriptor-table ABI](descriptor-tables.md) uses these extra slots;
the legacy shader compiler retains its original two-slot mapping.
Repeated slots must name the same native object (clones are accepted); different
objects conflict even if their descriptors or contents match. Every supplied pair
is checked for LOD compatibility and texture/sampler/render-device registry identity
before compilation. The trusted native caller supplies the pairs its shader uses;
loader metadata enforces required bindings before reaching this API.

Each draw binds or clears all four texture slots and all four sampler slots, including
when programs change. `PendingRender` owns clones of all pair resources through
completion or draining Drop; input arrays and shader source need not outlive
submission. No new unsafe Send/Sync declarations are used. The older
`submit_multi_pipeline_triangle_targets_rgba8` Option-binding API and single-pair
wrappers preserve their slot-zero restrictions. Empty passes use empty shader
and binding slices; older single-source wrappers retain source validation.

`set_metal_multislot_attachment_backend` enables loader multi-slot snapshots;
older backend registrations reject draws requiring nonzero-slot pairs. Original
native tests cover cross-pairs, sampler LOD rebinding, explicit texture clearing,
conflicts and retained-depth preservation on a later invalid binding. Original
PE guest tests compose all four pairs, rebind samplers and exercise sync/async
execution, including destruction after reset and before async collection. These
local checks do not establish full Stage 2 or Stage 3 acceptance.

Stage 2 still needs general descriptor heaps, texture arrays, broader view/format
support, filter/address modes and wider synchronization. Explicit sampler mip selection
remains LOD clamping; the separate actual texture-view interface below selects storage. Stage 3 still needs general loops, divergent EXEC/VCC paths and reconvergence. Runtime
uniform scalar branches are described below; they do not supply those semantics.

### Selected array-slice read-only views

`GraphicsArraySliceViewDesc { slice, base_mip, mip_count, format }` and
`create_array_slice_view` select one slice of an explicitly typed RGBA8 array.
The resulting view uses existing 2D shader coordinates; child views inherit the
slice and normalize relative mip ranges to the root. Complete backing metadata,
immutable snapshots and whole-backing hazards remain authoritative, including
for one-layer arrays. No writable role or shader array indexing is implied.

Native `MetalRgba8ArrayTexture::upload` validates and initializes every subresource
before exposing immutable storage. Its `array_slice_view(ArraySliceViewDesc)`
creates a shared-storage `MetalTextureView`. The corresponding
`RenderColorOutput::array_slice_view` returns a `RenderColorView` retaining its
exact producer point and storage lease. Existing 2D-only methods reject arrays.
Caches distinguish complete backing identity, slice, mip range and interpretation.
Host views do not expose upload methods; produced views cannot become ordinary
host bindings. Unsupported format families and invalid ranges fail explicitly.

The loader's array-view attachment, asynchronous and queued installers explicitly
enable this contract. Replacing them with older installers revokes support.
Queued sampling consumes exact produced array versions without host reconstruction;
collection and retirement preserve dependencies and complete backing ownership.

### Immutable sampled mip-range and linear/sRGB views

`TextureViewDesc { base_mip, mip_count, format }` selects a checked, nonempty mip
range and a compatible linear/sRGB interpretation. RGBA8 storage uses `Rgba8Unorm`
or `Rgba8UnormSrgb`; BC1/BC3 interpretations are described below. `MetalRgba8Texture::view` creates
an actual native shared-storage `MetalTextureView`, not a cropped upload or sampler
clamp. View dimensions are those of the selected base mip, including odd and nonsquare
extents; explicit mip indices and per-pair sampler LOD checks are relative to the view.
The selected range must fit the complete backing mip chain without arithmetic overflow.
These views remain single-sample 2D, slice zero and shader-read-only. Loader child
views normalize parent-relative ranges to the root backing; native views still use
that normalized root range. Destroying a parent handle leaves independent children valid.

Metal's [range-view API](https://developer.apple.com/documentation/metal/mtltexture/maketextureview%28pixelformat%3Atexturetype%3Alevels%3Aslices%3A%29)
shares the parent allocation. Linear/sRGB conversion does not require
[PixelFormatView usage](https://developer.apple.com/documentation/metal/mtltextureusage/pixelformatview),
so this implementation does not add that flag. sRGB sampling decodes RGB while leaving
alpha linear; render outputs remain linear RGBA8. Unsupported reinterpretations are
not inferred from equal byte size. Native view creation failure is reported.

`SampledBinding::Whole` preserves ordinary texture bindings; `SampledBinding::View`
contains an immutable view, sampler and slot indices. The additive
`submit_view_binding_triangle_targets_rgba8` accepts these mixed bindings. Views expose
only descriptor/extent metadata, with no upload, raw texture or public conversion to
`BoundSampledTexture`. Private normalization explicitly retains the parent allocation
inside every cloned native binding, so pending work owns it after outer view handles
are dropped. Owners of ordinary uploadable backing textures must still serialize
uploads with GPU use; a view does not make that backing allocation immutable.

`RenderColorOutput::view` creates a separate opaque `RenderColorView`, retaining its
producer point and native parent. `QueuedSampledBinding::ProducedView` derives waits
from that provenance; it cannot enter the ordinary view/whole-texture APIs.
`QueuedSampledBinding::Viewed` accepts uploaded-resource views. Existing Uploaded and
Produced render outputs have one mip, so only `base_mip=0, mip_count=1` is valid
for those outputs. The explicit mip-copy extension below also produces full 2D
mip chains; its views select real retained levels. Missing mips never come from
host snapshots.
Generation rejection and failure propagation apply equally to produced views.

The loader exposes `create_texture_view`, `destroy_texture_view` and atomic
`cmd_set_sampled_views` for two view slots and independent samplers. View-aware backend
registration is explicit through `set_metal_view_attachment_backend`,
`set_metal_view_async_backend` and `set_metal_view_queued_backend`; older backends reject
view use. Guest opcodes 37/38/39 create, destroy and bind views; existing opcodes 21/34
retain their whole-resource semantics. See the [guest descriptor ABI](graphics-call-v1.md).

The additive `create_child_texture_view` API and guest opcode 45 create an
independent read-only child from a parent view. The nonempty relative mip range
must fit entirely within the parent; checked arithmetic normalizes it to the root
backing and absolute base mip. Children retain root storage identity rather than
depending on parent-handle lifetime, so destroying an otherwise destroyable parent
does not invalidate them. The existing format-family, object-budget and backing
alias rules apply; no second pixel backing is allocated. This extension adds no
array or writable-view support. Targeted guest validation passed; service reruns
and full gates remain pending. See the [validation guide](testing/local-graphics-checkpoint.md).

Views retain backing identity for state, alias and lifetime checks. The initial policy
uses whole-resource hazard exclusion, including views of different mip ranges.
Recorded bindings, including superseded ones, prevent view destruction until list
reset; pending references can retire destroyed views and backing owners until safe
release. Live view handles prevent backing destruction. Views count toward object
limits while shared backing bytes are charged once. Host snapshots remain owned
submission versions, so later guest uploads cannot change an already submitted view.

FFI caches backing uploads by pixel Arc identity and full backing metadata, and native
views by that identity/metadata plus the complete view descriptor, including format.
Produced-view caches use produced-version identity plus the complete descriptor.
Cross-pairs clone the same cached native view, preserving the duplicate-slot identity
contract without allocating or uploading a texture for every draw.

Local native coverage includes literal mip rebasing, view-relative dimensions and
spatial patterns, odd nonsquare extents, linear/sRGB RGB values with unchanged alpha,
late-invalid-LOD depth preservation, and gated produced-view sampling. An internal
identity test checks Metal's exact parent allocation and explicit parent retention
after normalization. Original PE guest and queued FFI tests exercise view sampling
and host-publication failure. Compile-fail examples guard mutation and ordinary-binding
escapes. These results do not establish general view support or full-stage acceptance;
mutation experiments have separate evidence and are not presumed complete here.

Arrays, writable views, arbitrary format casts and formats beyond the bounded
RGBA8/BC1/BC3 sampled-resource contract remain open. The older host BC readback helpers
remain separate from the persistent resource interface below. Produced color Load
uses the queued attachment API below; dependency-backed produced depth is described
in the typed depth section. Unordered overlapping depth remains unsupported.
Stage 3 general loops and divergence remain open.

### Persistent typed BC1 and BC3 sampled textures

The shared `xenora_texture::TextureFormat` distinguishes `Rgba8Unorm`,
`Bc1RgbaUnorm` and `Bc3RgbaUnorm`. `PackedTextureLayout::new` defines canonical tight
mip storage, exposing dimensions, offset, row bytes, row count and byte length for
each level. BC1 uses eight-byte blocks and BC3 sixteen-byte blocks; complete 4×4
edge blocks remain allocated even for a 1×1 mip tail. Dimensions halve and clamp to
one. Zero extents, excessive mip counts and arithmetic overflow reject. Canonical
guest storage is distinct from the older common-pitch/slice upload-helper layout.

`MetalBcTexture::upload(device, BcTextureDesc, &[BcMipUpload])` creates an immutable,
fully initialized native compressed texture. Each upload supplies a level, row pitch
and bytes. Every level must appear exactly once. Preflight checks all levels before
native allocation: pitch must cover the canonical row and be block-size aligned;
the byte span must equal checked pitch times block rows, including padding. Both
canonical storage and total padded input are bounded to 64 MiB, with an explicit
16,384-axis native policy limit. Allocation failure remains an error; the policy
limit is not a claim that every device accepts every size.

Original compressed blocks reach Metal unchanged. The owner exposes descriptor and
immutable view creation, with no raw texture, mutation or post-publication upload
entry point. Changed content creates a new owner. Private sampled storage carries
its actual format; BC data is never relabeled as `MetalRgba8Texture`. Native uploads
cover complete logical mips and edge-block storage; a single block row uses zero
native bytes-per-row as required by Apple's
[compressed upload contract](https://developer.apple.com/documentation/metal/mtltexture/replace%28region%3Amipmaplevel%3Awithbytes%3Abytesperrow%3A%29).

`MetalBcTexture::view` creates actual shared-storage mip-range views using
`Bc1RgbaUnorm`/`Bc1RgbaUnormSrgb` or `Bc3RgbaUnorm`/`Bc3RgbaUnormSrgb`.
Only linear/sRGB reinterpretation within the same storage family is accepted;
BC1-to-BC3 and RGBA8-to-BC casts reject. View-relative dimensions and sampler LOD,
explicit parent retention and private normalization follow the existing immutable
view contract. Full-range BC views use `SampledBinding::View` and queued `Viewed`.
Color targets, produced outputs and `ProducedView` remain explicitly RGBA8; BC is
sampled-only and cannot become a render/depth attachment or produced compressed output.

`NativeCompute::supports_bc_textures()` queries the actual device's
[supportsBCTextureCompression](https://developer.apple.com/documentation/metal/mtldevice/supportsbctexturecompression)
capability. Unsupported hardware returns an explicit capability error: there is no
CPU decompression fallback. All three FFI adapters—synchronous, single-submission
async and queued—query their actual rendering device, including the worker-owned
device, and advertise capabilities through the loader's typed backend installers.
Older capability-less backends remain RGBA8-only.

The loader's additive `create_texture_resource` uses `GraphicsTextureDesc` and
preserves the legacy `ResourceDesc`/opcode 6 contract. Guest opcode 40 takes device,
width, height, mip count, storage format and initial state; storage codes 0/1/2 mean
RGBA8/BC1/BC3. Upload and Readback transfer canonical compressed bytes exactly.
View opcode 37 additionally accepts BC1 linear/sRGB codes 2/3 and BC3 codes 4/5.
Authoritative format metadata travels with snapshots; BC resources do not set the
legacy RGBA8 flag. Resource budgets, whole-backing hazards, alias exclusion and
retirement apply to compressed resources as to RGBA8. Legacy sampled projections
remain RGBA8-only. FFI upload/view caches include storage format, complete backing
metadata, snapshot identity and the full view descriptor; queued consumers can mix
host BC views with dependency-protected produced RGBA8 textures.

Local targeted evidence includes four passing native cases, original PE BC1/BC3
sync/async and queued FFI BC-to-produced-RGBA integrations. Independent reviewers
resolved interpolated-value expectations using the reference specification rather
than fitting observed hardware bytes. For the original alpha palettes240/30 and
0/250, only interpolated selectors permit errors7 and9 respectively. White/black
selector3 retains ideal RGB85(linear)/23(sRGB) with independently composed byte
intervals76..94 and18..29. Exact extrema, endpoint regression checks and alpha
preservation remain separately asserted. These disjoint color intervals still
reject a lost sRGB interpretation; an adjacent-alpha-selector fixture fault also
fails. These are [reference-derived compatibility envelopes](https://microsoft.github.io/DirectX-Specs/d3d/archive/D3D11_3_FunctionalSpec.htm),
not Metal precision guarantees. Scripted capability rejection is not unsupported-
device hardware evidence. Full local validation gates passed; none of this
establishes Xbox hardware equivalence or completion of Stage2/3.

### Original guest graphics calls

The versioned [GraphicsCallV1 contract](graphics-call-v1.md) exposes selected
pipelines, resources, transfers, command recording and submission to original
guest programs through import dispatch. It remains a synthetic research ABI;
backend capabilities and unsupported forms are validated explicitly.

### Uniform forward branches in original graphics stages

Vertex and pixel lifting follows unconditional `S_BRANCH` targets using the public
[AMD RDNA2 SOPP definition (§12.5)](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf):
PC + 4 + sign-extended SIMM16 × 4. Forward targets must land at decoded instruction boundaries inside the stage;
the bounded countdown extension below also admits specific backward targets. The entire byte stream is still
decoded and validated, including skipped instructions and branch destinations. Reached
instructions retain register initialization, adjacent interpolation pairs and export
requirements. Branches do not reset export/DONE tracking. Reaching an early terminator
with trailing instructions is rejected.

Graphics stages also accept the twelve decoded signed/unsigned 32-bit scalar
comparisons with immediate, literal, uniform or initialized local operands. Following AMD §12.4, each replaces
the scalar condition code (SCC); integer operand bits are not subject to FP32 finite
checks. `S_CBRANCH_SCC0` selects its forward target when initialized SCC is false;
`S_CBRANCH_SCC1` selects it when SCC is true. Both preserve SCC.
Both stages start with unknown SCC, vector operations preserve it, and a skipped
comparison cannot initialize it. Only s24..s31 are initialized scalar register
inputs in this graphics contract. Even untaken conditional targets must satisfy
instruction-boundary and loop-structure checks.

Immediate-only conditions select paths at translation time. Runtime scalar inputs
use an acyclic control-flow graph with definite-assignment checks and explicit
VGPR, scalar-local and SCC joins. All feasible paths must reach the common export suffix. Resource
and uniform requirements are unions across runtime paths; constant-dead paths do
not contribute. Samples remain inside their conditional blocks. General backward
control flow, EXEC/VCC divergence and reconvergence remain unsupported; the bounded
countdown extension below is the explicit exception.

Graphics scalar locals s32..s105 start undefined independently in each stage.
`S_MOV_B32`, all 36 decoded 32-bit SOP2 binary forms and all 11 decoded
SOP1 unary forms operate on exact 32-bit integer bits. The
[scalar contract](shaders/scalar-alu.md#graphics-scalar-execution) lists their
arithmetic, shift, bit-field, bit-scan and condition-code behavior.
Inline floating selectors 240..248 supply exact 32-bit words in the existing
scalar source readers, vector moves and registry-backed vector integer/compare
readers. They consume no literal word; integer inline one remains distinct from
the floating 1.0 bit pattern. Other operand and precision restrictions remain.
Descriptor registers s0..s23 and uniform inputs s24..s31 cannot be destinations;
special registers and scalar-to-vector transfers remain unsupported. Every source,
including both CSELECT operands, must be defined on every feasible incoming path.
MOV and CSELECT preserve SCC; CSELECT and carry/borrow operations require it to be
initialized. Signed overflow is computed from wrapped integer bits, without native
signed overflow. Integer, boolean and floating-point joins retain distinct types.
Constant scalar expressions preserve dead-path pruning. Every reachable uniform
read contributes its mask, including reads whose values are later overwritten.

`CompiledGraphics` exposes vertex/pixel uniform masks and their union. Bits 0..7
represent s24..s31. Loader pipelines retain the union; each draw owns an optional
eight-u32 snapshot. Native `DrawUniforms` carries `required_mask` and `values` through
the additive uniform submission API and `RenderCohort::submit_with_uniforms`.
Reserved buffer 2 is bound or cleared for both stages on every draw. Whole-batch
presence validation precedes encoding, and pending tickets retain native buffers.
Native trusted-MSL callers must supply truthful requirement metadata; this API does
not infer requirements from source text. Guest pipelines derive masks from compiler
analysis. Older loader backend registrations reset uniform capability and reject
required inputs; the three production FFI adapters opt in explicitly.

Original native fixtures cover all twelve signed/unsigned comparisons, nested SCC
and VGPR joins, and multiple targets. Original PE cases vary geometry and color on
one compiled pipeline through sync/async/queued adapters. All nine local gates passed; four production mutations failed observable assertions,
and two independent reviews accepted this increment. Buffer2 clearing was source-
reviewed, not directly observed by a shader without that input. No Xbox hardware
equivalence is established.

### Bounded countdown loops in original graphics stages

The graphics compiler admits contiguous loops with strictly nested or disjoint
intervals. Each header tests an initialized local counter using unsigned `!= 0`
or signed `> 0` (including reversed `0 < counter`), followed immediately by an SCC0
exit. The complementary unsigned `== 0` or signed `<= 0` (including reversed
`0 >= counter`) with an SCC1 exit is also admitted. Proven zero-trip headers
preserve the appropriate exit SCC value. An unconditional backward branch returns
to that header. Exactly one direct
`counter = counter - 1` update must execute on every path that returns to the latch,
with no other counter writes, interior entries or body exports. A body branch may
exit to the innermost loop's existing continuation immediately after its latch.
Conditional and unconditional exits are admitted; exits across loop levels or to
arbitrary continuations reject. Nested loops use distinct counters;
a child cannot modify an ancestor counter or repeat its decrement. The original preheader must prove
a maximum count using a constant or matching unsigned/signed MIN; the compiler inserts no clamp.
An independently proven unsigned upper bound below the sign bit can also bound a
signed-positive loop.
A bound may also survive a carried value when every possible write is an already
validated, nonwrapping countdown decrement. Increasing or resetting writes invalidate
that inference. Child counters may be reinitialized by actual shader code or retain
their depleted value; every invocation must have a proven bound.

Signed trip bounds are `max(signed_upper, 0)` and remain separate from unsigned
value bounds. A negative signed counter skips the body and retains its exact bits;
a proven-false header sets SCC false without replacing the counter with zero.
Signed-positive guards allow either signed or unsigned subtraction by one, retaining
the original opcode and flag behavior. Both decrease unsigned and signed order in
that guarded range. An unsigned nonzero guard alone cannot preserve signed bounds:
`0x80000000 - 1` becomes signed INT_MAX. A signed bound therefore never authorizes an
unsigned countdown without an independent unsigned proof. Signed `>= 0`, other
strides and general induction remain unsupported.

Generated Metal executes a real loop. Typed scalar/vector header joins retain
values across iterations, with parallel assignments on selected loop edges. Body
branches retain scalar, vector and SCC joins; the mandatory header comparison
replaces SCC each iteration and on normal exhaustion. An early exit carries its
actual body values and SCC without an extra comparison or decrement. Forward joins
and header initialization use ordered groups of typed parallel copies on the
selected edge. Arbitrary SCC values carried
into header reads are not supported. Missing first-iteration definitions reject.
Runtime bodies contribute their feasible resource and uniform requirements;
proven-zero-trip loops execute the header once and omit dead body requirements,
while preserving input requirements needed for live-out values. A
structurally valid loop interval with no emitted blocks needs no semantic entry
bound and contributes no execution cost. This includes descendants skipped by a
constant break or a proven-zero ancestor; structural validation still runs first.
A runtime-dependent break retains both feasible successors and cannot grant this
exemption. A counter merged at an early exit currently needs a fresh explicit
bound, such as unsigned MIN, before reuse as another loop counter.

Admission remains limited to 1,024 input bytes, eight loops and an individual proven
trip bound of 256. Checked execution accounting multiplies enclosing loop bounds;
a loop header executes up to K+1 times per invocation, its body up to K. The
conservative total must not exceed 65,536 decoded instruction executions. Original
fixtures admit exactly 65,536 and reject 65,537; separate coverage checks final
child-header execution costs. Literal extension words are part of their instruction,
while the two interpolation phases are distinct instructions.
Unsupported cycles reject even when skipped. Other unsupported decoded operations
on skipped forward paths retain the existing selected-path validation policy.

Original local tests exercise literal pixels, geometry, scalar swaps, body diamonds,
preheader merges, MIN flags, final SCC and accumulated texture samples. Nested
cases cover three loop levels, sibling children, changing child counts, retained
counters and alias snapshots. Early-exit cases cover breaks before and after the
decrement, high-bit integer swaps, SCC live-outs, recurring inner exit sites,
sampling requirements and vertex movement. Original PE guests vary counts and stop
values through all three
native submission modes. These results do
not establish general loop support, divergent wave execution or Xbox equivalence.

### Preserving color attachments across submissions

`GraphicsColorLoad::Clear` and `Load` select per-slot initialization through
`cmd_set_color_targets_with_loads`. Existing color-target APIs remain Clear wrappers.
Loads read committed host-backed RGBA8 mip0 bytes at execution time, leaving mip tails
unchanged. New zero-initialized host textures can be loaded. Target identity and load
mode remain fixed after drawing starts; states, extents, uniqueness and recorded
reference lifetimes follow existing target validation.

`set_metal_attachment_backend` receives immutable `GraphicsColorBinding` snapshots.
The prior stateful callback retains its signature but rejects drawn loaded attachments
before invoking an old backend. Empty passes without depth can copy loaded bytes or
fill clears directly. Native `render_loaded_triangle_targets_rgba8` validates exact
slot counts and packed byte lengths, uploads into fresh shared attachments and selects
Load/Clear independently. It never writes into input snapshots. Validated outputs and
depth commit together; failed passes retain prior content and presentation.

The original guest uses opcode29 to combine depth-tested color preservation with a
later clipped overlay from a distinct original translated pipeline. This supports
separate pipeline submissions over one scene. The host adapter also supports
per-draw pipeline changes within one submission, and both synchronous and asynchronous
paths have original mixed-program tests. Completion and scale bounds are described below.

### Asynchronous native rendering and loader receipts

`PendingRender` owns submitted Metal commands and inputs, borrowing only exclusive
access to a retained depth destination. `submit_loaded_triangle_targets_rgba8` commits
without waiting. `is_complete` polls native status; `wait` reads colors and publishes
staged depth only after success. Dropping drains GPU work and discards staged depth.
Synchronous rendering APIs wrap submission plus wait. This follows Apple's
[command-buffer lifecycle](https://developer.apple.com/documentation/metal/mtlcommandbuffer)
and [retained resource contract](https://developer.apple.com/documentation/metal/mtlcommandbuffer/retainedreferences).

The loader prepares owned draws, sampled/color/depth snapshots and commit metadata.
One pending receipt retains all recorded references, including superseded bindings.
An async backend must return terminal results only after native resource use drains.
Poll/Wait validate all output lengths and depth presence, acquire presentation access,
and then atomically commit colors, depth, barriers, history and presentation. Destroyed
pinned resources move to retirement storage until completion; their handles never return
to use. Object, resource-byte and pipeline limits include retired storage. Reset cannot
change submitted inputs; pinned transfers and concurrent submissions are rejected.

`install_async_pipeline_backend` creates a dedicated native worker. Only owned host
snapshots and completion messages cross threads; no new unsafe thread-trait declaration
is required. Acknowledgment follows native commit and precedes waiting. Worker setup
failures reject submission; later failures become terminal receipt errors. Context and
ticket destruction drain work. Native tests use a real shared-event gate according to
Apple's [queue ordering contract](https://developer.apple.com/documentation/metal/about-synchronization-events),
with bounded rescue to expose accidental blocking without hanging the GPU.

The original guest supports Submit/Poll/Wait/ReleaseSubmission, separately from
synchronous Execute. The existing single-submission async mode still permits only one outstanding
receipt. The separate opt-in queued mode below adds a bounded two-queue contract;
neither mode completes Stage 2.

### Bounded two-queue rendering and produced texture versions

`NativeCompute::new_render_cohort` creates a `RenderCohort` with two distinct reusable
queues on the same device. `submit` accepts a queue, per-draw shaders and
`QueuedSampledBinding` collections, optional `OwnedDepthInput`, color loads and prior
`RenderPoint` dependencies. It returns after native commit, with at most one uncollected
submission per queue. Points are opaque, generation-specific and issued only after
commit; each queue has its own checked increasing shared-event timeline. Foreign or
stale points reject. Edges can name only earlier committed work, preventing future
waits and cycles. GPU waits precede upload/blit/render encoders; signals follow them.

`color_output(point, slot)` exports an opaque `RenderColorOutput` immediately after
submission. Its clones retain the same RGBA8 render texture with RenderTarget and
ShaderRead usage; consumer submission does not read it back or upload another copy.
The handle exposes extent and producer identity, but no upload, raw texture, ordinary
binding conversion or readback API. `QueuedSampledBinding::Uploaded` accepts an existing
`BoundSampledTexture`; `Produced` accepts the opaque output, sampler and slot indices.
Private normalization derives waits from output provenance and applies the existing
slot-conflict, device and per-pair LOD validation. Older rendering APIs cannot accept
Produced bindings. External output clones pin versions independently of the two live
submission slots, so their owner must budget and release them.

`OwnedDepthInput { snapshot, load }` supplies private depth for an owned submission.
Load requires a matching snapshot; Clear may start without one. Snapshot bytes are
staged into a retained buffer and uploaded in the submitted command, without a CPU
completion wait in `submit`. `collect` returns `OwnedRenderOutput { colors, depth }`;
depth is exported only when owned depth was requested, not for transient pass depth.
Borrowed-depth and synchronous wrappers preserve their existing contracts. All native
objects stay on one worker using ordinary ownership; no unsafe Send/Sync declaration
or self-referential pending container is needed.

`GraphicsContext::set_metal_queued_backend` opts into this legacy mode.
The FFI `install_queued_pipeline_backend` now uses the multi-node graph capability
described below. The legacy loader contract supports at most two
retained receipts, including released ancestors still needed by a dependent, and one
pending submission per queue. `submission_point` identifies an existing receipt;
`submit_command_list_after` accepts up to two retained dependency points. Same-queue
ordering is implicit; conflicting cross-queue access requires explicit dependency
coverage. Preparation uses dependencies' planned resource states and produced-version
identities while public resource state and bytes remain unchanged. Sampled producer
outputs resolve to native versions rather than stale committed host snapshots.
This initial render-produced contract exposes single-mip RGBA8 textures. The explicit
mip-copy extension below additionally retains complete produced mip chains.

The worker acknowledges commit before completion, captures output handles before
collection, and retains its ticket/version records until loader ticket release.
Collection resolves dependencies before publication and validates outputs before
committing colors, depth, resource states, history and presentation. Reset and logical
destruction cannot change submitted inputs; per-receipt pins retain and charge retired
resources. Releasing one receipt cannot erase provenance needed by another receipt.

Event progress proves ordering, not successful execution or host publication. Native
polling checks command and ancestor outcomes. Failure poisons the cohort; draining
visits producers before consumers and advances a missing event signal from the host
only after that producer's command is terminal. It never releases a consumer while
its producer may still write. Terminal outcomes are cached and resolved ancestor links
are released. `abort_and_drain` remains available after errors; `reinitialize` requires
drained/collected work and creates fresh events and a new generation. Drop uses the
same ordered drain. External test gates require their own bounded release watchdog.

A loader publication failure rejects pending dependents, including a dependent whose
native success was already cached. It cannot publish that cached result. Rejection
notification is nonblocking, but a dependent receipt remains pending until its native
work drains. Previously committed public results remain intact. Recovery requires
release of all retained failed-generation tickets before a fresh generation can start;
old native output handles and points cannot cross that boundary.

Coverage includes native and FFI shared-event gates proving both submissions commit
before producer release, literal sampling of produced colors, both collection orders,
reset/destruction retention and owned-depth persistence. Controlled host publication
rejection exercises a separately gated live consumer and fresh-generation recovery.
Deterministic scripted failure tests share the production drain traversal and inject a
terminal producer error with a missing GPU signal; they check terminal-before-signal
ordering, dependent failure propagation, caching and ancestor release. These scripted
and host-rejection tests are not evidence of an actual GPU/device fault.

This is a bounded foundation. Produced color Load copies into a fresh output through
the attachment API below; in-place mutation and unordered overlapping depth use reject.
General queue graphs, formats, descriptor heaps, arrays/writable views, overlapping
copy/render writes and broader synchronization remain open Stage 2 work. Stage 3
still lacks general loops, divergent EXEC/VCC execution and reconvergence. Local test coverage does not
constitute full-stage acceptance or replace independent review.

### Translated scene scale and allocation bounds

Translated native submissions support at most 65,535 expanded vertices, 256 nonempty
draws and 1,048,576 pixels per target, with each dimension in 1–4,096. These are explicit
interface bounds, not reported hardware maxima. Indexed draws validate offsets, signed
base vertices and both index widths before gathering; total vertex and draw limits are
checked before fetching the next draw. Zero-count draws do not consume the draw budget.
Loader and Metal constants are compared by the production integration test.

Software-reference execution retains 768 vertices and 4,096 pixels, including direct
public helper calls. The original FixtureRenderer retains its 768-vertex input contract.
Older native indexed/blend fixture APIs retain their independent smaller bounds.

At maximum translated extent, eight RGBA8 native targets occupy 32 MiB, with another
32 MiB in output readbacks; loaded host snapshots and existing resource storage can
coexist. Native position/color buffers total at most 2 MiB. Working and retained depth
surfaces and snapshots add storage; padded depth-transfer buffers remain below 5 MiB
each. Sampled uploads, driver/compiler allocations and other contexts add more. The
256 MiB context resource budget is not a total process or GPU-memory ceiling.

An original 512×512 tile scene uses 6,144 indices with padded attribute strides,
nonzero buffer/start offsets and signed base adjustment in both index widths. Split
draws test scissor restoration; 32 frames per format test changing uploads and loaded
depth/color preservation. A bad final index must preserve the prior frame. Separate
native tests successfully draw 65,535 vertices at both 4,096×256 and 256×4,096, then
snapshot/restore and probe depth against literal outcomes. Only the final triangle is
visible, avoiding a misleading high-overdraw stress workload.

### Integrated rendering regression coverage

The original PE `GraphicsCallV1` tests exercise both index widths with nonzero view,
start and signed-base offsets, padded attributes, rejected final-index readbacks and
successful recovery. Guest blending cases distinguish RGB and alpha equations,
unsaturated additive alpha, masked channels and restoring Replace/full-mask state.
Both synchronous and asynchronous adapters run these guest paths.

A separate host-driven async scene combines indexed geometry, sampled and unsampled
programs, two differently ordered MRT exports, per-draw state, and retained depth.
It checks every attachment pixel across Clear/Load passes, a failed final indexed
submission, recovery and a later depth probe. The failure happens during validation
before GPU submission; this is not a forced GPU-fault test. These original regression
scenes do not demonstrate commercial-game or Xbox hardware compatibility.

### Queued produced color attachment Load

`GraphicsQueuedColorInput` distinguishes a clear color, an immutable host snapshot,
and an exact `GraphicsProducedVersion`. The opt-in
`set_metal_produced_color_queued_backend` accepts the latter; replacing it with an
older installer revokes that capability. Ordinary synchronous attachment inputs
remain unchanged. The queued native adapter resolves `(queue point, attachment)`
to an opaque retained output; it never substitutes committed host bytes for a
produced input. Existing guest ColorTargetsWithLoads and SubmitAfter express this
operation without a new guest opcode.

`RenderCohort::submit_with_color_loads` accepts `QueuedColorLoad::{Clear, Host,
Produced}`. A produced input must belong to the cohort's current generation and
match the device, whole single-mip RGBA8 format and target extent. Its provenance
adds a dependency even when the caller supplies no explicit dependency list.
The submission waits for producers, copies each produced source into a fresh
attachment with a GPU blit, then renders with Load/Store and signals completion.
Producer surfaces remain immutable. Same-pass sampling/output aliasing still
rejects. Whole-input validation precedes native commit.

Loader collection validates and publishes ancestors before descendants. Collecting
an already terminal producer does not replay its resource writes. A producer
publication failure invalidates dependent results even when their GPU completion
was cached. Invalid consumer output cannot partially publish multiple attachments.
Reset and logical destruction do not alter admitted inputs or resurrect deleted
resources at collection.

Two separate reservations bound owned payloads. The loader reserves queued color
outputs, expected readbacks, presentation-copy allowance and host snapshots until
its receipt nodes are pruned (200 MiB ceiling). Native cohorts reserve color
versions and conservative geometry/uniform/depth/readback payload allowances
(256 MiB ceiling); shared leases remain with command-owning points and opaque
outputs, including across cohort reinitialization. The native ceiling excludes
borrowed caller uploads, host results after ownership transfer and opaque driver
or pipeline overhead; it is not a total-memory or VRAM limit. These reservations
do not replace existing logical resource-byte limits.

All nine local gates passed, with two independent scoped reviews. Original native,
worker and PE cases verify mixed initializations, partial overlays, exact versions,
failed ancestry and lifetime/accounting. Five production mutations fail observable
GPU-progress, pixel, slot and ownership assertions. The gated wait observation is
timing-sensitive; its omitted-wait mutation produced an early GPU signal, and moving
the wait after the copy produced incorrect pixels. Neither an unobserved signal nor
a timeout alone is counted as proof. Produced depth uses the typed API below; queued
partial copies, subresource hazards and general queue graphs remain open Stage 2
requirements; whole-resource copies are described below.

### Typed queued depth versions

Produced versions distinguish `GraphicsProducedOutput::Color(slot)` from `Depth`.
A queued request lists its ordered color outputs and an optional depth output;
consumers resolve the exact typed point rather than interpreting a depth surface
as a color slot. The new produced-attachment backend opts into depth inputs;
replacing it with a color-only or older backend revokes that capability.

`GraphicsQueuedDepthInput` represents Clear, a committed host snapshot or a produced
version. Pending initialization can satisfy a dependent Load before host publication.
Clear needs no previous contents, but still requires DEPTH_WRITE. DEPTH_READ accepts
Load only with depth writes disabled. Every bound depth pass creates a fresh output
version, including read-only and zero-draw passes; same-backing publication therefore
remains dependency ordered even when draws do not write depth.

The native `RenderDepthOutput` retains opaque immutable storage and provenance.
`QueuedDepthInput` and `RenderCohort::submit_with_attachments` validate and copy a
Produced depth source directly into fresh working Depth32Float storage after its
GPU wait, with no host readback/upload or additional import allocation. Explicit
owned depth is stored and exportable; transient pass depth is not. Existing owned
snapshot wrappers retain their validation behavior. Persistent depth leases share
the existing native payload budget and remain owned by outputs and direct consumer
commands; temporary staging reservations remain separate.

Loader depth reservations are tracked separately from color reservations through
`queued_depth_allocation_bytes` and `MAX_QUEUED_DEPTH_BYTES`; they cover retained
working/readback, host snapshot/import and padded transfer allowances until node
pruning. Native depth leases share the existing 256 MiB payload ledger, without
raising its ceiling. These counters cover distinct ownership domains, not total memory.

All nine local gates and two independent reviews passed. Original native and guest
probes distinguish .25/.75 planes, read-only preservation, writable .25/.5 results,
older exact versions and Clear .875. Worker tests cover gated live-consumer rejection
and recovery; CPU tests separately verify snapshot rollback and cached-success
rejection. Five production faults fail GPU-progress, pixel or ownership assertions.
The wait observation is timing-sensitive with demonstrated mutation detection; no
actual device-fault recovery is claimed. Whole-resource copies are described below;
depth sampling/views, subresource concurrency and general queue graphs remain open. This increment does
not establish Stage 2 or Stage 3 completion.


### Typed queued whole-resource copies

`set_metal_copy_queued_backend` accepts `GraphicsQueuedWorkRequest::{Render, Copy}`
and `GraphicsQueuedPendingWork` completions. Existing render-only installers retain
their public request/ticket APIs through an adapter and revoke copy capability.
`GraphicsQueuedCopyRequest` carries generation, queue point, dependencies, an exact
`GraphicsCopyLayout`, a host snapshot or produced source, and a `Copy(0)` output
identity. Authoritative per-version layout metadata distinguishes buffers, RGBA8 and
depth; equal byte counts do not authorize a different layout or output kind.

The native cohort provides `submit_copy`, `copy_output` and `collect_copy`. Produced
buffers and textures retain opaque native versions; downstream copies and texture
sampling/Load use these versions directly. Native encoding orders dependency waits,
blits and the completion signal. Buffer allocation and blits use four-byte padding;
readback returns the exact logical length. Retained output and direct-source leases
share the bounded payload ledger, including across generation changes.

Loader validation precedes publication of any destination bytes or states. Copy
outcomes are separately available through `submission_copy_outcome`; they do not
manufacture render outcomes, acquire the presentation lock or advance presentation
serials. Opcode 42 exposes recording to original guest programs. Four original guest
tests cover buffer chains, render-to-copy, copy-to-sampling and copy-to-color-Load in
both collection orders. These synthetic tests do not demonstrate Xbox game support.

The initial contract permits one whole-resource copy plus barriers per copy-only list,
owned buffers and single-mip RGBA8 textures, within the existing two-node cohort.
Buffer regions and RGBA8 image transfers are described below. Additional
subresources/formats, guest-memory synchronization and general queue graphs remain
Stage 2 work.


### Queued buffer regions

`GraphicsQueuedBufferRegionRequest` and `GraphicsQueuedWorkRequest::BufferRegion`
carry independent source and prior-destination inputs, full logical lengths, byte
offsets and byte count. `set_metal_buffer_region_queued_backend` enables aligned
regions. `set_metal_unaligned_buffer_region_queued_backend` additionally enables
arbitrary-byte regions and image copies. Replacing it with an older installer
revokes the additional capability; unaligned submission then rejects before dispatch.
Older whole-copy installers revoke all region support. The loader resolves each produced input from authoritative
metadata and requires its dependency. Destination preservation is recorded as a read
alongside the destination write. States are validated at the recorded copy command.

`QueuedBufferRegionCopy` and `RenderCohort::submit_buffer_region_copy` wait for both
inputs, clone the full padded prior destination into fresh storage, then overlay the
requested bytes before signaling completion. Aligned patches use a blit; arbitrary-byte
patches use a GPU kernel with one invocation per destination word, preserving bytes
outside the logical interval. Both direct input leases survive
ancestor pruning. Bounds use logical lengths, so padding cannot become addressable.
The output is a complete immutable buffer version; collection returns its exact
logical byte count and uses the same atomic publication path as whole copies.

For full padded source size S and destination size D, payload reservation is
2D plus 2S for a host source plus 2D for a host prior destination. Produced inputs
retain existing leases. Unaligned patches additionally reserve 16 copied parameter
bytes. This conservatively covers output, readback/initialization
scratch and host uploads; it is not a measurement of total process or GPU memory.
One copy plus barriers per list still applies. Legacy installers retain the two-node
limit; the graph installer described below permits sixteen retained nodes.
Guest opcode43 records this operation. Texture rectangles and pitched transfers
use the image-copy API below. Broader queue graphs remain open.


### Queued image regions and pitched transfers

`GraphicsQueuedImageCopyRequest` and `GraphicsQueuedWorkRequest::ImageCopy` extend
copy work with independent full source/destination layouts, exact host or produced
inputs, endpoint locations and a positive texel extent. `GraphicsImageCopyLocation`
distinguishes an RGBA8 texel origin from a buffer byte offset and row pitch.
`set_metal_image_copy_queued_backend` enables this contract; replacing it with an
older installer revokes image support while retaining that installer's earlier modes.

`QueuedImageCopy` and `RenderCohort::submit_image_copy` support single-mip RGBA8
texture rectangles, pitched buffer→texture and texture→buffer transfers. They wait
for both inputs, clone the complete prior destination into fresh storage, then apply
the transfer. Buffer results retain full logical length and untouched row gaps and
tails; texture results retain full dimensions and unchanged borders. Both native
leases survive ancestor pruning; no produced input is reconstructed through host data.
The resulting texture version can feed native sampling or color attachment Load.

RGBA8 buffer offsets and pitches are multiples of four, with pitch at least the row
payload and at most the software cap of MAX_RENDER_DIMENSION×4. Logical footprint
validation uses offset+(height−1)×pitch+width×4. Native tests with API/GPU validation
cover 12/20-byte pitches, nonzero offsets and exact final-row bounds. Native calls use
bytesPerImage0 and depth1 for these 2D transfers. This is local evidence, not a claim
of validation on every Apple GPU or Xbox hardware.

Payload reservation is 2D plus host-source import cost plus host-prior-destination
import cost, using full physical backing bytes. A host buffer import reserves twice
its padded size; a host texture import reserves its full packed size. Produced inputs
retain their existing leases. Output publication validates full kind/layout/length
before any bytes/state mutation and never updates presentation. Destination preservation
is a read hazard as well as a write; failure of either ancestor rejects cached success.

Guest opcode44 exposes the bounded operation. One copy plus barriers per list and
the legacy two-node limit or opt-in graph limit applies. Explicit mip/slice transfers
are described below; compressed/depth transfers and general graphs remain open.


### Opt-in multi-node queued graphs

`GraphicsContext::set_metal_graph_queued_backend` enables up to 16 retained
receipts, including released ancestors, with multiple pending submissions on the
same logical queue. The native worker maps at most four logical queues to four Metal
queues. Older installers retain the two-node and one-pending-per-queue contracts
in the preceding sections. Replacing the graph installer revokes its larger limits.
Existing payload ceilings remain unchanged; the node limit does not multiply them.

Each receipt stores direct dependencies, including its latest retained same-queue
predecessor. The loader resolves their bounded transitive closure before checking
hazards and selecting resources. State and content definitions are separate: a
barrier changes state without inventing a new output version. A unique descendant
writer supersedes its ancestors; incomparable writers reject, regardless of the
order of the wait records. Worker requests contain the closure so exact ancestor
outputs remain addressable. Collection validates and publishes ancestors first;
a failed ancestor cannot be hidden by an already completed descendant.

`NativeCompute::new_render_graph_cohort` permits 16 uncollected native entries,
identified by opaque submission points rather than queue slots. The legacy
`new_render_cohort` constructor retains its previous limits. Submission, output
lookup and collection preserve exact point identity. Same-queue predecessor edges
also order failure draining, so the host cannot signal past an earlier pending
writer. Successful terminal collection may release an obsolete predecessor owner;
remaining descendants retain their own dependencies and resource leases.

Guest opcode35 accepts up to 16 checked dependency records; the installed loader
backend still decides admission. Local validation covers original native and guest graph tests, including color/depth
rendering, ancestor failure and ownership. No general game or console compatibility
follows from these synthetic tests.


### GPU-produced geometry in queued draws

`GraphicsContext::set_metal_geometry_graph_backend` adds exact produced buffer
inputs for positions, optional colors and R16/R32 indices. The production queued
worker uses this installer. Existing binding and draw opcodes express the inputs;
no new guest opcode is required. Legacy installers retain expanded host geometry
and reject produced geometry before dispatch. Host-only queued draws retain their
existing CPU validation path.

Each gathered draw describes logical buffer lengths, independent attribute offsets
and strides, optional index format/offset, start/count and signed base vertex.
A mixed draw snapshots its host inputs and resolves each produced input through the
submission dependency closure. The worker checks exact point, output kind and
logical length; it never substitutes current host bytes for a produced version.
Existing triangle, instance, draw and aggregate vertex limits remain in force.

After waiting on each producer, an original compute kernel gathers byte-addressed
inputs into packed float4 position/color buffers for the existing vertex ABI.
Every complete index and attribute range is checked before access. R32 indices
remain unsigned during signed base adjustment. All supplied attribute components
must be finite; logical lengths exclude allocation padding. Missing colors retain
the existing zero default. Invalid lanes write initialized finite placeholders and
set an atomic failure flag; excess dispatch lanes return without accessing storage.
Rendering follows the gather in the same command submission. Geometry payloads
remain on the GPU; completion reads only the validation flag in addition to normal
attachment readback.

The flag participates in native point polling and ordered draining before terminal
success can be cached or ancestry pruned. Invalid geometry prevents publication of
the entire pass and its descendants, including color, depth, state and presentation.
Direct source leases, packed scratch and status ownership follow their actual
command/point lifetimes. Admission preflights cumulative host snapshots; native
reservations include packed outputs, host staging, both parameter copies and the
shared status buffer. Host input occurrences are snapshotted, uploaded and charged
separately, including aliases; produced backing leases are shared by identity. The
host allowance conservatively covers three padded copies, rather than only the
uploaded buffer. Existing payload ceilings and generation recovery rules apply.

Local validation passes all nine gates, including eight native geometry tests and
two original guest tests. Eight injected fault types are detected; the initial
surviving poll-validation mutation and its successful retest remain recorded. Final
independent bounded acceptance is summarized in the
[local graphics checkpoint](testing/local-graphics-checkpoint.md); supporting raw
logs are not distributed. This extension preserves the current geometry formats
and shader ABI; additional formats,
instancing and indirect draws require separate work.


### Reusable immutable sampler descriptors

`create_sampler` stores a validated `SamplerDesc` in a counted live handle;
`destroy_sampler` removes it immediately. `cmd_set_sampled_view_handles` resolves
both view slots and both sampler slots before recording, then reuses the existing
value-based sampled-view command. Descriptors are copied at recording time, so
handle destruction cannot change recorded draws or pending submissions. Stale or
wrong-kind handles reject future use. Slots unbind independently and retain existing
cross-pair and view-relative LOD checks. Guest opcodes46–48 expose this interface;
see the [guest ABI](graphics-call-v1.md#reusable-immutable-sampler-handles).
The native backend and shader ABI are unchanged. General descriptor heaps and
additional filtering/addressing modes remain outside this increment.


### Guest-backed whole-buffer copy snapshots

Guest opcode49 creates a buffer referring to caller-owned guest memory without
reading it. Queued whole-buffer copies accept it only as a source. Submission
validates the complete payload budget before copying the checked logical range
into an immutable `GraphicsCopyInput::Host`. Recording does not capture bytes;
post-submission changes and handle destruction cannot alter the pending snapshot.
Failed admission leaves queue points and destination contents unchanged, and a
retry captures fresh bytes. Guest-backed destinations remain rejected; pitched
image-source transfers are supported as described below. This is bounded submission-time copying, not coherent guest/GPU
memory; see the [guest ABI](graphics-call-v1.md#guest-backed-whole-buffer-copy-sources).


Buffer-region copies use the same full-source guest snapshot contract through
opcode43. The region's source and destination bounds are checked before capture;
the full logical source must be readable and is fully charged to the payload budget.
The existing native region operation retains the exact owned destination-prior
version and preserves bytes outside the patch. Existing arbitrary-byte capability
checks and Produced destination dependencies remain unchanged.


### Linear R8 sampled storage

`TextureFormat::R8Unorm` adds canonical one-byte texels to `PackedTextureLayout`.
Guest storage code3 and view code6 select this format. `GraphicsTextureCapabilities.r8`
is independent of BC support and defaults false; native adapters enable it, while
legacy installers and unsupported draw paths fail closed. R8 views share their
backing and allow only R8 interpretation, including nested mip ranges.

`MetalR8Texture::upload(device, R8TextureDesc, &[R8MipUpload])` validates every mip
before native allocation. Each mip occurs exactly once; pitch is at least its width
and less than Int32MAX, with an exact checked `row_pitch * height` byte span. Canonical
and total padded inputs are bounded to 64MiB. The native dimension policy remains
16,384 per axis, subject to allocation success. The texture is fully initialized and
immutable, exposes read-only views, and retains actual `MTLPixelFormatR8Unorm` storage.
It has no raw-texture or post-publication upload API. Single-row uploads pass zero
native bytes-per-row, following Apple's
[texture upload contract](https://developer.apple.com/documentation/metal/mtltexture/replace(region:mipmaplevel:withbytes:bytesperrow:)).

The FFI upload/view caches retain explicit storage format and complete snapshot/view
identity. Implicit R8 bindings use full-range views; selected views preserve mip-relative
sampling. Sync, async and queued adapters share this typed upload path. Shader binding
syntax is unchanged. Render/depth attachments, Produced R8 outputs, queued R8 texture
copies and cross-family reinterpretation are outside this sampled-only extension.

R8 sampling returns normalized red with absent green/blue components zero and alpha
one, as specified by the [Metal Shading Language specification](https://developer.apple.com/metal/Metal-Shading-Language-Specification.pdf)
(page240). Nearest, unblended samples of bytes0,17,128,255 into a linear RGBA8 target
therefore have expected bytes `[r,0,0,255]`. This contract does not replicate red
into the other color channels or apply sRGB decoding.


### Explicit queued mip transfers

`GraphicsContext::cmd_copy_image_mips` and guest opcode50 record RGBA8 transfers
with `GraphicsImageCopyLocation::Rgba8Mip { mip, slice, origin }` endpoints.
`set_metal_mip_copy_queued_backend` opts into this capability; older installers
revoke it. Legacy image-copy entry points keep their single-level contract.
The mip increment requires slice0, with buffer endpoints retaining offset/pitch.

`GraphicsCopyLayout::Rgba8MipChain` and the matching produced layout carry base
extent and mip count. Canonical `PackedTextureLayout` footprints determine the
complete byte length and selected mip dimensions. Equal byte counts alone never
establish layout compatibility. Single-mip resources retain legacy RGBA8 identity.

Native `RenderCohort::submit_mip_image_copy` imports complete host chains or retains
exact produced GPU storage. It waits for both inputs, clones every destination mip,
patches the selected rectangle and returns the complete packed chain. Untouched
mips and borders are preserved. Direct source leases and full-chain payload charges
remain live through command ownership; publication validates full layout and length.
Produced mip views retain the producer and use actual native mip storage without
host readback/reupload. Multi-mip attachment Load remains rejected.

Metal defines source/destination mip and slice independently in its
[texture blit contract](https://developer.apple.com/documentation/metal/mtlblitcommandencoder/copy(from:sourceslice:sourcelevel:sourceorigin:sourcesize:to:destinationslice:destinationlevel:destinationorigin:)).
Xenora's packed offsets describe host transfers, not Metal's physical texture layout.
The existing whole-backing hazards, copy-list limits and queue budgets apply.


### RGBA8 array transfers

`PackedTextureArrayLayout` wraps a canonical mip chain with a nonzero layer count,
checked complete byte length and absolute `(slice,mip)` footprints. No texel storage
is allocated by the helper. `GraphicsTextureArrayDesc` and
`create_texture_array_resource` add explicitly typed RGBA8 array storage; existing
resource creation retains its restrictions. Layer-major bytes are a host transfer
representation, not a claim about the native texture's physical layout.

`GraphicsCopyLayout::Rgba8Array` and the matching produced layout carry extent,
mip count and layer count. `cmd_copy_image_subresources` and opcode50 use the
existing explicit mip/slice endpoint representation. The array backend installer
opts into this mode; mip-only and older installers reject arrays, even slice0 of a
one-layer array. Array identity never follows solely from dimensions or byte length.

Native `HostRgba8Array` inputs allocate actual Metal 2D-array textures.
`RenderCohort::submit_array_image_copy` validates complete layouts, waits for both
producers, clones all destination slices/mips and patches the selected rectangle.
Readback returns the complete layer-major backing as `OwnedCopyOutput::Rgba8Array`.
Existing direct leases, whole-backing hazards and payload ceilings apply. All
array sizes include every layer and mip in resource and transfer accounting.

`RenderColorOutput::is_array()` distinguishes arrays from 2D storage, even with one
layer; `array_layers()` and `mip_levels()` expose validated metadata. Array views
and attachment roles reject. A transfer to a 2D destination creates a 2D output
which can feed produced sampled views. Array shader views remain separate work.
Policy limits are 256 layers, 4096 subresources and the existing resource/context
byte budgets; these are Xenora bounds, not Metal hardware capability claims.

Metal's [array length](https://developer.apple.com/documentation/metal/mtltexturedescriptor/arraylength)
and [texture blit](https://developer.apple.com/documentation/metal/mtlblitcommandencoder/copy(from:sourceslice:sourcelevel:sourceorigin:sourcesize:to:destinationslice:destinationlevel:destinationorigin:))
contracts distinguish slice selection from the 2D origin and depth. Each transfer
uses one slice with origin z0 and depth1; buffer row-pitch rules remain unchanged.

### Guest-backed image source snapshots

The source-aware copy-layout path accepts guest-backed buffers for legacy, mip/array
and typed image transfers. Destinations and commit identity checks continue using
owned-only layouts. The native request stays unchanged: checked guest bytes become
an immutable Host input, charged for the full logical buffer before capture. Exact
Produced destination dependencies and complete-backing preservation remain intact.
See [guest image sources](graphics-call-v1.md#guest-backed-buffer-image-sources).

### Explicit guest resource mirrors

`bind_guest_memory` associates an idle owned root with a complete checked guest
mapping. `synchronize_guest_memory` imports or exports 1–16 full roots at a time.
Imports stage all payloads and advance host publication epochs only after every
validation succeeds; exports preflight every mapped writable span. Duplicate and
overlapping bindings reject within a batch. Retained submissions block binding
and synchronization. Guest mapping identities prevent writeback through recycled
addresses; explicit rebind selects replacement storage. Destruction drops the
association, and no asynchronous callback holds a guest pointer.

The GPU continues to own immutable snapshots. This is explicit synchronization,
not automatic shared-memory coherence. See the [ABI contract](graphics-call-v1.md#explicit-guest-resource-synchronization).

### Four-queue graph increment

Graph cohorts add `RenderQueue::Third` and `Fourth`, each with its own event,
sequence and pending tail. Legacy cohorts reject these lanes before admission;
their two-node limit remains intact. The cohort owns all four physical queues,
including when using legacy admission. Worker mapping supports four distinct
logical queues in a generation across render, copy and attachment paths. A fifth
queue rejects without consuming a logical point or committing resource states.
Mapping survives successful collection; generation reinitialization clears it.
All four timelines are recreated during recovery, and old-generation points and
outputs remain invalid. Payload ceilings and the sixteen-node graph limit do not
increase. The initial increment is recorded in its
[plan](superpowers/plans/2026-11-27-four-queue-graphs.md); subsequent bounded S2.7
acceptance is recorded in the [local checkpoint](testing/local-graphics-checkpoint.md#s27-acceptance-bounded-mixed-resource-graphs).


### Bounded resource dependency contract

The opt-in graph API is a directed acyclic graph of already admitted submissions.
Guest waits name retained queue points in the current generation; unknown, future,
duplicate and failed points reject before admission. The loader adds the latest
retained same-queue predecessor and computes a bounded transitive closure. Native
produced inputs also retain and infer their producer edges. Previously obtained native output clones and dependency points can remain usable
after collection within the same healthy generation; collection removes the entry
needed for another collection or output lookup. Guest waits can name released
receipts while descendants retain them, but reject their points after pruning.
Neither API supports waiting on future signals or forming cycles.

Resource state and content lineage are distinct. A barrier-only use does not
create a new content version. A unique descendant definition wins over its
ancestors; incomparable definitions reject independently of wait-record order.
Selected-root composition separately names semantic ancestors (shader inputs and
attachment Load) and composition ancestors (the destination backing for copy-back).
An ordering edge alone must not substitute newer pixels for a selected semantic
version. Hazards remain conservative at whole-root granularity.

The graph has two independent node limits: sixteen retained logical receipts
(including released ancestors still referenced by descendants), and sixteen
uncollected native stages. An attachment transaction consumes seed, render and
copy-back stages, so it can hit native capacity before sixteen logical receipts.
Collection frees native slots; guest release/pruning frees logical receipts.
Four logical queue mappings persist for a worker generation, even after all work
has completed. A fifth distinct queue rejects until generation recovery. Existing
payload ceilings apply across all resource roles; external native output leases
continue to charge their backings after collection.

Native polling and draining preserve ancestor failure and same-queue order. Host
recovery signals cannot pass a producer that is still writing. Guest publication
validates ancestors first; a failed ancestor invalidates pending descendants even
if their native commands have completed. Failure poisons the generation and blocks
new admission until all retained receipts have been released and pruned, including
previously successful receipts. Recovery creates fresh event
timelines and queue mappings; stale native points and produced outputs reject.
Already published successful submissions are not rolled back by a later failure.

Descriptor revisions, geometry inputs, selected attachments and shader storage
retain exact input versions under their existing finite contracts. Explicit guest
mirror import/export remains an idle boundary operation: retained submission
references prevent synchronization, and GPU completion does not write guest memory.
Unbounded graphs, subresource-parallel hazards, automatic coherence and console
execution are outside this contract. Acceptance evidence is recorded in the
[local checkpoint](testing/local-graphics-checkpoint.md) and the
[acceptance checklist](superpowers/plans/2026-11-27-resource-graph-acceptance.md).

## Dedicated compute load wait

`S_WAITCNT_VMCNT null,0` retains a distinct decoded identity and shares the existing
synthetic compute pending-load completion transition with `S_WAITCNT vmcnt(0)`.
It does not complete pending stores or modify SCC, and redundant waits are allowed.
Only exact word `0xBC7D0000` is admitted; other registers/thresholds and graphics
use are rejected. This is not a general hardware counter or memory-barrier model.
