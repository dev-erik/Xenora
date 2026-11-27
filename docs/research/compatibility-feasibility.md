# Research: a path toward Xbox Series X|S compatibility

Reviewed 27 November 2026. This is a public-source feasibility assessment and
research plan, not a demonstration of commercial Xbox execution. It supports the
[comprehensive compatibility target](../xbox-compatibility.md), without changing
that target to PC games, UWP applications or a synthetic demo.

## Finding

There is a concrete research and engineering path toward broader compatibility.
There is **no verified recipe in the reviewed evidence for complete compatibility**,
and no basis to guarantee every title/version or online feature. The immediate
risk is the unresolved Xbox application/graphics boundary, not merely the number
of unimplemented desktop RDNA2 instructions.

The current code can validate independently authored contracts on Apple Silicon.
That does not establish that those contracts match console binaries. Research
must connect target inputs, guest interfaces, execution behavior and host mappings
with independent evidence. Additional opcode work remains useful, but it should
not defer this investigation until the desktop inventory is complete.

## Public findings and limits

| Evidence | What it establishes | What it does not establish |
| --- | --- | --- |
| [Microsoft public GDK README](https://github.com/microsoft/GDK/blob/Main/README.MD), current public page | The public kit differs from Xbox Extensions; console graphics driver/compiler and environment components are not all supplied publicly | A complete console ABI or a public kit that can produce every console binary |
| [Microsoft resource access documentation](https://learn.microsoft.com/en-us/xbox/gdk/docs/gdk-dev/development-downloads/access-resources?view=gdk-2604) | Some console-specific graphics resources require partner access | That restricted material is available for Xenora's clean-room implementation |
| [Microsoft HlslCompile sample documentation](https://learn.microsoft.com/en-us/samples/microsoft/xbox-gdk-samples/hlslcompile/), dated 5 November 2026 | The Series X/S ACO example uses offline pipeline serialization and runtime deserialization; assets depend on the producing GDK version | A published complete serialized layout, or that every title uses this optional path |
| [Apple Metal Shader Converter](https://developer.apple.com/metal/shader-converter/), current public page | A DXIL-to-metallib route exists, with documented device/OS requirements and supported shader models | Conversion of arbitrary Xbox native shader binaries, console pipeline layouts, executables, or system services |
| [Microsoft D3D12 binding specification](https://microsoft.github.io/DirectX-Specs/d3d/ResourceBinding.html), v1.24 | Public behavior for descriptors, root signatures and resource binding provides a useful reference contract | Xbox descriptor bytes, command packet layouts or every console-specific extension |
| [LLVM AMDGPU guide](https://llvm.org/docs/AMDGPUUsage.html#processors) | An assembler/disassembler reference for explicitly selected AMD targets, including the desktop target used by Xenora | Runtime arithmetic correctness or Xbox custom target equivalence |
| [Microsoft PE specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format) | Public executable/object structure for Windows tools | Complete Xbox loading, packaging, startup or environment behavior |
| [Apple Metal feature tables](https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf) | Feature support differs by GPU family and software version | Semantic equivalence of similarly named Xbox/Metal features or an automatic correct fallback |

The sample README was also checked at repository commit
`b1e77b6ac10f95b07296025fda171ca2ddc42dbe`:
[pinned public README](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/Graphics/HlslCompile/readme.md).
Only public documentation and repository metadata were inspected; no sample code
was copied or executed.

The HlslCompile documentation also says its PC-side library cannot deserialize
Xbox root signatures in that sample. **Inference:** PC graphics tools cannot be
assumed to decode console assets merely because the API family is similar.
The sample's version dependency makes toolchain/version identification a required
part of an input contract. This is evidence about that example, not all retail titles.

Public GDK sample descriptions cover graphics, audio, system and service areas,
and are useful for discovering interfaces and observable behavior. Public source
availability does not imply that their console builds run without the required
platform toolchain. No such sample was built or executed for this assessment.
[Official sample repository](https://github.com/microsoft/Xbox-GDK-Samples/).

## Architecture hypotheses to test

These are research alternatives, not implementation decisions or working paths.

| Hypothesis | Potential benefit | Required evidence before choosing it |
| --- | --- | --- |
| Reimplement applicable Xbox API behavior above the driver | Translate guest-visible resource and draw operations directly into Xenora/Metal; avoid reproducing irrelevant hardware internals | A stable reachable boundary, complete signatures/structures, version handling, callbacks and observable semantics. Investigate inlining/static linkage; do not assume all calls are named imports |
| Translate console command streams and native shaders | Handle workloads where the higher-level boundary is unavailable | Actual packet/descriptor layouts, GPU address rules, shader metadata/entry ABI, synchronization, target opcode differences and validation inputs |
| Use DXIL-to-Metal for inputs that demonstrably contain supported DXIL | Potentially reuse an existing compiler path and reduce handwritten shader lowering for those inputs | Verified payload/container identity, reflection/binding mapping, supported operations/numerical behavior, ARM64 tool/runtime audit and redistribution/integration terms |
| Combine high-level interfaces with native-shader translation | Match different input paths while sharing the resource backend | Explicit routing criteria and a common semantic contract; cross-path tests; no fallback that silently changes behavior |

The appropriate boundary could vary by component and version. A source port of a
PC build can be a reference workload but is not execution of the Xbox binary.
Likewise, developer-mode/UWP experiments must be labeled with their actual app
model and target; they do not establish native Series X/S game compatibility.
The reviewed [Microsoft UWP FAQ](https://learn.microsoft.com/en-us/windows/uwp/xbox-apps/frequently-asked-questions)
is Xbox One/UWP-scoped and is not evidence of a Series X/S native-game ABI.

## Priority research backlog

All tasks below are **open**. This assessment identifies evidence and questions;
it does not count the proposed experiments as completed research results.

| Priority / ID | Investigation | Concrete deliverable and completion test | Master-map areas |
| --- | --- | --- | --- |
| P0 / R1 | Actual input and startup contract | A public-evidence matrix of executable/package/container versions and entry requirements; a parser specification with original positive/negative fixtures. Mark unsupported actual formats instead of silently treating them as PE | XBOX-01–04, 14, 51–52 |
| P0 / R2 | Graphics interception boundary | Inventory public console graphics names, observable arguments/ownership and unknown fields; assess API interception versus command translation. Each claimed mapping needs a source and a discriminating test proposal | XBOX-12–15, 19, 21 |
| P0 / R3 | Independent target oracle | Establish which target observations can actually be obtained under the project's public-source rules; define reproducible inputs, target/version, outputs, state and provenance. Explicitly list behavior that cannot yet be checked | XBOX-53, 55, 58 |
| P1 / R4 | Shader payload routing | Document signatures and versions from public evidence, separate DXIL/native/unknown inputs, and design rejection tests. Benchmark an original DXIL fixture through Metal only after tool/terms/ARM64 review; do not assume retail payloads match it | XBOX-14–15, 24–32 |
| P1 / R5 | CPU/runtime interface catalog | Reconcile the current CPU/service registries against known target requirements; inventory missing signatures, error codes, asynchronous semantics and calling conventions. Publish unknowns alongside supported cases | XBOX-04–11, 41–52 |
| P1 / R6 | Xbox-to-Metal semantic gap matrix | For each required graphics feature: native mapping candidate, GPU/OS limits, exactness risks, fallback proposal, test and status. Prioritize waves/memory order, layouts, bindings, advanced stages and ray tracing | XBOX-16–40 |
| P2 / R7 | End-to-end reference workload | Execute the same original, specified workload through a relevant reference path and Xenora; compare observable outputs and lifecycle. Clearly separate PC-reference, synthetic and actual-console evidence | XBOX-53–57 |
| Continuous / R8 | Versioned requirement closure | Every new discovered API/form/mode becomes a tracked requirement with evidence and tests; update source snapshots and per-target/version compatibility. No denominator is called exhaustive without an audit | XBOX-58 and all areas |

R1–R3 address the largest feasibility uncertainty before more instruction counts
are interpreted as progress toward authentic startup. R4–R6 can proceed alongside
bounded arithmetic implementation. No task is closed by a list of API names alone.

## How compatibility can be validated

Use layered evidence rather than one “works” checkbox:

1. **Specification:** record what a public source actually defines, for which
   architecture/version; separate an observation from an inference.
2. **Encoding and structure:** independently validate bytes, field bounds and
   malformed cases. LLVM agreement is useful here, not an execution oracle.
3. **Semantics:** compare original boundary/randomized tests with independent
   mathematical/reference implementations. Account for defined versus undefined
   behavior and permitted floating-point tolerances.
4. **Native host:** execute on Metal and native ARM64, including ordering,
   lifetime, errors and recovery. Report hardware and OS; skipped tests stay skipped.
5. **Target differential evidence:** compare reproducible results on the actual
   applicable target where permitted and available. Desktop AMD results narrow
   uncertainty but do not prove Xbox-specific behavior. There is no console-run
   evidence produced by this research pass.
6. **Integrated applications and titles:** record load, startup, first frame,
   gameplay, audio/input, save/load, online dependencies and sustained stability
   independently for each title/version and X/S target. Coverage grows with a
   corpus; a finite corpus cannot prove every possible program or future update.

No console device/tool access or external service behavior has been established
for this assessment. If it becomes necessary to obtain external reference results,
first identify the exact unanswered question and suitable public/authorized
experiment; do not purchase equipment or access restricted material by assumption.
The repository's [clean-room rules](../legal.md) remain in force. Licensed SDK
access alone is not a license to publish confidential implementation details.

## Decision

Continue pursuing the comprehensive target, but make **Xbox boundary research
and independent reference evidence** first-class workstreams now. Preserve the
existing Metal/shader work as foundations. Do not assume that completing all
1,465 desktop shader forms will unlock Xbox games, and do not claim that a public
DXIL converter resolves console binary compatibility.

The defensible promise is a measurable investigation and progressively validated
compatibility. Universal compatibility cannot honestly be promised from the
current evidence. Online services, unavailable formats and unresolved target
contracts may remain external blockers even after substantial local implementation.
