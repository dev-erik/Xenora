# Xbox Series X|S compatibility target and coverage map

## Agreed objective

Xenora targets comprehensive Xbox Series X **and** Series S application and game
compatibility on native Apple Silicon macOS, with graphics translated to Metal.
A test scene, an instruction counter, or a bounded synthetic milestone does not
replace that objective. Prioritizing a feature changes its implementation order,
not whether required behavior remains in scope.

AnyPS5 was supplied solely as a reference for presenting progress in a README.
Its implementation, PS5 interfaces and compatibility inventory do **not** define
Xenora's requirements. No AnyPS5 emulator source is used here. Xbox requirements
must be established from applicable public sources and independent evidence.

This is the master coverage map. It inventories required **areas**, explicit
research gaps and acceptance obligations. It is **not yet an exhaustive,
validated enumeration of every Xbox interface, opcode variant or system call**.
Those contracts are not all known. “Unknown” is open work, never evidence that a
feature is unnecessary or already supported. No overall completion percentage
is justified while the requirement inventory is incomplete.

## Scope and evidence rules

- Keep separate target records for Series X and Series S, including guest-visible
  capabilities, memory limits, formats, versions and device behavior. One passing
  target does not certify the other.
- Desktop gfx1030 is the current synthetic shader target. Custom Xbox GPU opcode,
  entry-state, descriptor and submission contracts need separate evidence.
- Public Windows Direct3D 12 behavior can inform research; it does not establish
  Xbox driver, command-packet or binary layouts.
- Metal is the native rendering/compute backend. CPU execution and operating-system
  services need their own Apple Silicon implementations; Metal does not supply them.
- A host feature requires runtime GPU/OS capability detection and a documented
  semantic mapping. An equivalent software implementation may be acceptable when
  proven correct. A no-op, a lower-quality approximation or an unsupported error
  does not satisfy a required compatibility feature.
- Record correctness and performance separately. A correct slow path is not
  evidence of playable performance. A fast approximation is not full correctness.
- Preserve the project's [existing clean-room and input scope](legal.md).
  Protected-content support is not implemented. If a target depends on an
  unavailable service, format or protected boot path, record that limitation;
  do not claim universal compatibility.
- Console firmware/dashboard boot and complete physical-device emulation are
  **unresolved scope/architecture requirements**, not delivered capabilities.
  For application compatibility, identify which guest-visible behaviors must be
  reproduced without assuming a full console OS boot is necessary or unnecessary.

Microsoft publicly identifies custom RDNA2 GPUs and features including ray
tracing, variable-rate shading and the Velocity Architecture. These disclosures
establish research areas, not binary ABI specifications.
[Microsoft technology glossary](https://news.xbox.com/en-us/2020/03/16/xbox-series-x-glossary/),
[existing Xbox GPU evidence](research/xbox-gpu.md).
Host capabilities vary by GPU family and software version; mappings must use
[Apple's feature tables](https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf)
and runtime checks, rather than assume all M-series devices expose identical behavior.

## How to read the matrix

**Bounded foundation** means local implementation or synthetic evidence exists
in the linked area; the full Xbox contract is still open. **Open** means no
complete Xbox acceptance is recorded here, even if parts of the implementation
exist. **Unknown contract** means specification/ABI discovery is a blocker.
These labels deliberately do not infer acceptance from method names or metadata.
Every row remains open for full compatibility until its completion evidence is
recorded. “Required coverage” is a research and validation checklist, not a
claim that each listed API spelling or hardware mechanism exists unchanged on Xbox.

### Target, loading and CPU execution

| ID | Area | Required coverage / completion evidence | Current basis |
| --- | --- | --- | --- |
| XBOX-01 | Target profiles | Separate Series X/S profiles; guest-visible CPU/GPU features, memory budgets, versions and capability queries; independently justified values | Unknown contract; [hardware research](research/xbox-gpu.md) |
| XBOX-02 | Executable and module loading | Actual supported Xbox executable/module formats, relocations, imports, initialization, TLS and dependencies; malformed-input handling and startup evidence | Bounded PE32+ foundation; [interfaces](interfaces.md); authentic loading open |
| XBOX-03 | Packages and content | Package metadata, mounts, install/update layout, resource lookup, content versions and supported input boundaries | Unknown contract; synthetic PE loading is not package compatibility |
| XBOX-04 | Process and calling conventions | Entry state, ABI, stack/unwind rules, register preservation, dynamic libraries, process lifecycle and exit behavior | Bounded synthetic Win64 foundation; [CPU registry](cpu/instruction-registry.md) |
| XBOX-05 | Integer CPU execution | Required x86-64 encodings, prefixes, flags, addressing, exceptions and boundary cases; interpreter/native translator agreement | Bounded foundation; [CPU registry](cpu/instruction-registry.md) |
| XBOX-06 | CPU floating point and SIMD | Required x87/SSE/AVX and other vector forms; rounding, exceptions, NaNs, denormals, conversions and state save/restore | Bounded foundation; full target coverage open |
| XBOX-07 | CPU atomics and memory order | Atomic widths/alignment, locked operations, fences, cross-core visibility and contention; guest memory-order litmus tests on ARM64 | Open; [CPU boundary](research/cpu-boundary.md) |
| XBOX-08 | Translation lifecycle | Executable-page changes, self-modifying code, cache invalidation, code publication, interpreter/translator transitions and precise faults | Bounded native translator foundation; full acceptance open |
| XBOX-09 | Virtual memory | Guest page sizes, address-space layout, reservations, commits, protection, guard pages, faults, aliases and CPU/GPU mappings | Bounded foundation; console contract open |
| XBOX-10 | Threads and synchronization | Scheduling-visible behavior, TLS, events, waits, mutexes, semaphores, condition variables, wakeup ordering, cancellation and teardown | Bounded services; [runtime registry](runtime/registry.md) |
| XBOX-11 | Timing and exceptions | Clock domains, timers, timeouts, exception delivery/unwind, asynchronous completion and error codes | Bounded services; console equivalence open |

### Xbox graphics boundary and resource system

| ID | Area | Required coverage / completion evidence | Current basis |
| --- | --- | --- | --- |
| XBOX-12 | Graphics API/driver entry points | Enumerate actual Xbox exports, signatures, structures, ownership, callbacks, errors and version differences | Unknown contract; [GDK research](research/xbox-gdk.md) |
| XBOX-13 | Command submission | Actual command streams or applicable interception interface; graphics/compute/copy work, ordering, queues, synchronization and recovery | Synthetic command/graph foundation; console packets unknown |
| XBOX-14 | Shader/pipeline containers | Validate actual payloads, shader extraction, stage metadata, pipeline serialization and versioning; establish code versus data boundaries | Unknown console layout; [container research](research/shader-containers.md) |
| XBOX-15 | Shader entry ABI | Registers, stage inputs/outputs, wave state, descriptors, constants, built-ins and guest GPU address rules | Synthetic ABI only; [interfaces](interfaces.md) |
| XBOX-16 | Resource creation and lifetime | Buffers/textures, heaps, allocation alignment, aliasing, residency, ownership, destruction with in-flight use and failure atomicity | Bounded S2 foundation; [integrated acceptance](testing/stage2-integrated-acceptance.md) |
| XBOX-17 | Formats and layouts | Complete target format/role matrix, compression, packing, tiling/swizzles, pitches, mips, slices, samples and reinterpretation | Finite [format matrix](resource-formats.md); console layout open |
| XBOX-18 | Resource views | SRV/UAV/attachment-equivalent behavior, ranges, reinterpretation, arrays, cubes, 3D, depth/stencil aspects and overlap | Bounded S2.4; full console coverage open |
| XBOX-19 | Descriptors and binding | Actual heap/table/root binding contracts, dynamic indexing, address and lifetime rules, null descriptors and bounds | Bounded S2.5; [descriptor tables](descriptor-tables.md); console layouts unknown |
| XBOX-20 | Uploads, copies and readback | All required transfer directions, subresources, conversions, overlap rules and observable untouched data | Bounded S2.2/S2.3; full matrix open |
| XBOX-21 | Coherence and barriers | CPU/GPU visibility, caches, resource transitions, UAV/alias barriers, ownership and completion/error propagation | Bounded explicit mirrors/graphs; console semantics open |
| XBOX-22 | Sparse and streamed resources | Tiled/sparse mappings, residency changes, sampler-feedback interactions, eviction and failure behavior where required | Open |
| XBOX-23 | Indirect work and queries | Indirect draw/dispatch, counters, predicates, timestamps, occlusion/statistics and observable ordering | Open beyond bounded foundations |

### Shader translation and native Metal behavior

| ID | Area | Required coverage / completion evidence | Current basis |
| --- | --- | --- | --- |
| XBOX-24 | Instruction identities | Every required console encoding; reconcile desktop inventory with actual target differences | [Full desktop checklist](shaders/shader-checklist.md); console completeness unknown |
| XBOX-25 | Operands and modifiers | Source classes, literals, register pairs, aliasing, width/packing, clamps, swizzles, DPP/SDWA and special registers where applicable | Partial contracts; a decoded mnemonic does not close this row |
| XBOX-26 | Arithmetic and conversions | Required integer/FP16/FP32/FP64/packed/mixed forms, conversions and bit operations across applicable stages | S3.1 in progress; [checkpoints](testing/local-graphics-checkpoint.md) |
| XBOX-27 | Control flow | Branches, loops, calls, dynamic paths, reconvergence, termination and instruction-boundary validation | Bounded scalar/graphics paths; S3.2 open |
| XBOX-28 | Predicates, masks and flags | SCC/VCC/EXEC behavior, inactive lanes, conditional writes, carries, comparisons, divergence and helper lanes | Partial subset; S3.3 open |
| XBOX-29 | Wave and lane behavior | Target wave sizes, lane numbering, shuffles, ballots, reductions, quad operations and cross-lane dependencies | Wave32 synthetic subset; S3.4 open |
| XBOX-30 | Shader memory | Buffer/image loads/stores, address spaces, bounds, atomics, LDS/shared memory, barriers and ordering | Bounded memory subset; S3.5 open |
| XBOX-31 | Sampling | Texture dimensions/formats, samplers, addressing, filtering, LOD/gradients, gather, comparison, anisotropy and edge semantics | Selected 2D/array paths; full matrix open |
| XBOX-32 | Numerical equivalence | NaN/Inf/signed zero, denormals, rounding, precision, overflow, fused operations, contraction and transcendental requirements | Restricted evidence; S3.6 open |
| XBOX-33 | Shader stages | All target stages and their linkage, interpolation, built-ins and stream/output rules; geometry/tessellation/mesh/task-equivalent paths where required | Bounded vertex/pixel/compute foundation; broader stages open |
| XBOX-34 | Rasterization and output | Topologies, clipping/culling, viewport/scissor, provoking/interpolation rules, depth/stencil, blending, MRT and sample behavior | Bounded S1 foundation; console-wide acceptance open |
| XBOX-35 | Multisampling and precision | Sample patterns, coverage, sample-frequency shading, resolves, depth/stencil resolves and format precision | Open |
| XBOX-36 | Ray tracing | Pipeline and inline-query interfaces, acceleration structures/build/update, traversal, hit/miss behavior and synchronization | Open; host mapping must be proven, not inferred from Metal feature names |
| XBOX-37 | Variable-rate shading | Required rate images, combiners, coverage and derivative interactions; exact target-to-host mapping | Open |
| XBOX-38 | Advanced bindings and execution | Required bindless/resource indexing, mesh-driven/indirect GPU work, pipeline caching and capability reporting | Bounded indexing/cache foundations; broader behavior open |
| XBOX-39 | Presentation | Swapchain-equivalent behavior, pacing, buffering, vsync/VRR, color spaces/HDR, resizing, focus and device loss | Offscreen synthetic validation does not certify presentation |
| XBOX-40 | Host compatibility | Per-chip/OS Metal capability matrix, correct fallback paths, limits, compiler behavior and recovery; native ARM64 dependency audit | Runtime gating foundations; full host matrix open |

### Runtime, media and platform behavior

| ID | Area | Required coverage / completion evidence | Current basis |
| --- | --- | --- | --- |
| XBOX-41 | Runtime/system services | Enumerate actual imports and guest-visible contracts, handles, strings, errors, callbacks and asynchronous behavior | [Synthetic service registry](runtime/registry.md) and [inventory](runtime/services.md); console completeness unknown |
| XBOX-42 | Filesystem and save data | Paths, mounts, permissions, file/directory operations, async I/O, persistent saves, quotas and failure handling | Full Xbox contract open |
| XBOX-43 | Streaming and decompression | Storage request interfaces, completion ordering, required codecs/data formats, cancellations and resource-streaming integration | Open; Velocity Architecture disclosure alone is not an interface spec |
| XBOX-44 | Audio | Target APIs, codecs, channel layouts, mixing, resampling, spatial audio, clocks/latency and device changes | Full Xbox acceptance open |
| XBOX-45 | Input and peripherals | Controllers, hotplug, mappings, rumble, trigger feedback, keyboard/mouse/accessibility and polling/timestamp semantics where exposed | Full Xbox acceptance open |
| XBOX-46 | Video and media | Required decode/playback APIs, codecs, A/V synchronization, subtitles and surface sharing | Full Xbox acceptance open |
| XBOX-47 | Networking | Sockets/transports, name resolution, asynchronous I/O, timing, disconnect/error semantics and title-specific dependencies | Full Xbox acceptance open |
| XBOX-48 | Identity and online services | Guest-visible users, authentication-dependent APIs, multiplayer, achievements, cloud saves and unavailable-service behavior | Unknown/externally dependent; offline success cannot certify online compatibility |
| XBOX-49 | Lifecycle and suspend/resume | Launch/exit, focus, suspend/resume, device changes, retained state and Quick Resume-related observable requirements | Full Xbox acceptance open |
| XBOX-50 | Locale, accessibility and settings | Language/region, text/Unicode, time zone, accessibility, display/audio settings and configuration changes | Full Xbox acceptance open |
| XBOX-51 | Installation and updates | Content/DLC discovery, versions, deployment/update behavior and title-specific dependencies | Unknown contract; do not infer from PC packaging |
| XBOX-52 | Firmware and system environment | Determine necessary kernel/system interfaces, privileged operations and any full-boot requirements; record unavailable dependencies | Research unresolved; no firmware/dashboard compatibility claim |

### Integration, verification and usability

| ID | Area | Required coverage / completion evidence | Current basis |
| --- | --- | --- | --- |
| XBOX-53 | Per-feature acceptance | Versioned contract, applicable targets/hosts, implementation paths, independent oracle, boundary/negative tests, native execution, limitations and review | Existing synthetic acceptance practice; extend to all requirements |
| XBOX-54 | Original integrated workloads | Sustained CPU + graphics + resources + input + audio + storage execution; concurrency, teardown and failure recovery | Bounded original workloads; [synthetic integration](research/synthetic-integration.md) |
| XBOX-55 | Per-title compatibility | Record load, startup, first frame, interaction, gameplay, save/load, audio/input and sustained stability separately, for each title/version and target | Commercial game execution not demonstrated |
| XBOX-56 | Performance and stability | Frame-time distributions, shader/CPU compilation, memory growth, caches, streaming stalls, latency, long sessions and reproducible host configuration | Bounded benchmarks only; no general playability claim |
| XBOX-57 | Diagnostics and packaging | Reproducible launches, precise unsupported-feature reports, crash diagnostics, validation tooling, native packaging and installation | Research tools exist; end-user compatibility workflow open |
| XBOX-58 | Inventory closure | Reconcile every discovered interface/form/mode with a requirement and evidence row; audit missing categories and untracked behavior | Open; this map starts the system-wide ledger, not its certification |

## Required detail for each feature

Each area must expand into versioned, named requirements before it can close:

1. Xbox applicability and public evidence, with separate X/S differences and
   unknowns; include source/version and confidence.
2. Exact API/encoding/format and all relevant operand/state/error cases. Add
   feature variants to the inventory rather than hide them behind one green box.
3. Guest behavior and proposed host implementation, including Metal/GPU/OS
   requirements, fallback semantics and performance limits.
4. Current implementation paths; distinguish decoding, lowering, execution,
   integration and independent acceptance.
5. Reproducible tests and independent expected results. Label native Apple
   evidence separately from any console-equivalence evidence.
6. Known omissions, external dependencies and remaining research. An unavailable
   service or unresolved ABI remains visible and prevents an applicable full claim.

The shader checklist, [CPU registry](cpu/instruction-registry.md),
[runtime inventory](runtime/services.md) and [format matrix](resource-formats.md)
are separate supporting inventories. None is currently a complete Xbox ABI catalog.
Instruction coverage must not be used as a substitute for system-wide acceptance.

## Roadmap alignment and completion rules

The existing sequence remains useful for organizing work:

| Stage | Focus | What completing the bounded local milestone does not establish |
| --- | --- | --- |
| S1 | Rendering correctness | Complete Xbox rasterization, presentation or every pipeline feature |
| S2 | Resource correctness | Complete Xbox heaps, descriptors, formats, residency or coherence |
| S3.1–S3.6 | Arithmetic, control flow, masks, lanes, memory/interfaces, numerical correctness | Console shader ABI coverage merely by completing desktop opcode tables |
| S4 | CPU execution | Console processes, imports and services |
| S5 | Runtime and platform services | All external services or all title versions |
| S6 | Integrated original applications | Commercial-title compatibility |
| S7 | Per-target/per-title compatibility | Every game from one successful boot or scene |
| S8 | Performance and usability | Correctness from speed alone |

Workstreams may proceed in parallel when dependencies are explicit. Xbox ABI
research must continue alongside S3 arithmetic; it must not be postponed until
all desktop opcodes are implemented. Demos and representative applications are
validation steps toward comprehensive coverage, not a narrower replacement goal.
No feature is removed solely because an early application does not exercise it.

**Full compatibility is not currently claimed or guaranteed.** It requires
closing the applicable requirement inventories, validating their mappings,
resolving or explicitly bounding external dependencies, and demonstrating
end-to-end behavior on the declared targets. Until then, publish concrete tested
subsets, open gaps and per-title results rather than a universal percentage.
