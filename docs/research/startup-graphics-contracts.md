# Xbox startup and graphics boundary: public contract ledger

Reviewed 27 November 2026 against Xenora `844000d9ab6f4d9a259abd8037ead099aa931d3f`.
This advances R1–R3 in the [feasibility backlog](compatibility-feasibility.md).
All three remain open. No console binary or Microsoft sample was executed.
Public sample code was read for observations; no implementation was copied.

## Startup contracts

These are bounded public requirements, not a recovered console loader ABI.

| ID | Publicly established behavior | Missing evidence / implementation acceptance |
| --- | --- | --- |
| START-01 | `MicrosoftGame.config` identifies executable names and target families: `XboxOne`, `Scarlett`, `PC`. Names are relative to the game root. Packaging requires a consistent target family. [Executable element][manifest] | An original metadata parser can validate those fields and reject contradictory or unsupported targets. It must not infer a supported executable/container format from a manifest alone. No parser is added here. |
| START-02 | The build copies `MicrosoftGameConfig.mgc` to `MicrosoftGame.config`. [Overview][overview] | Recognizing metadata does not implement package extraction, registration, identity, entry state or protected-content support. The Xbox executable/startup contract remains unresolved. |
| START-03 | `HRESULT XGameRuntimeInitialize()` initializes the runtime. Each linked binary using it should initialize early; uninitialization belongs before exit. [Initialization][init] | Inventory the matching shutdown, repeated-call, threading and failure contracts before implementing lifecycle state. Do not invent reference-count semantics from this description. Test initialization failures and per-module ordering against explicit public requirements. |
| START-04 | `XSystemGetDeviceType()` is callable before runtime initialization; failure to identify a device yields `Unknown`. It reports console mode, which can differ from physical hardware. [Device query][device] | Model a guest target profile independently of the host Mac. Test pre-initialization calls and unknown profiles. Numeric enum values and import linkage require separate verification. |
| START-05 | An Xbox One-targeted title does not ordinarily see Series device codes unless its `XboxCompatibility` setting is `Scarlett`. [Device query][device] | Test title target, compatibility setting and configured console mode together. Do not hard-code Series X merely because the emulator targets Series consoles. |

The manifest reference also lists ARM64 as an allowed architecture value. That
cross-platform schema entry does **not** establish that native Xbox Series games
use ARM64. Keep platform applicability explicit. [Executable element][manifest]

## Graphics boundary observations

The following five rows derive from one pinned Microsoft sample, not an API
specification. [Source and exact revision][sample]

| ID | Observed call behavior | Unresolved contract |
| --- | --- | --- |
| GFX-01 | `D3D12XboxCreateDevice` receives a creation-parameter structure with an SDK version and queue/allocation settings. | Structure layout, version acceptance, errors and binary linkage. A Windows feature-level parameter is not an interchangeable contract. |
| GFX-02 | `PresentX` receives a plane containing a frame token, resource count and resource pointers after command submission. | Full layout, token validity, ownership, ordering and display semantics. |
| GFX-03 | `SuspendX` / `ResumeX` bracket suspension; resume re-registers frame events. | Thread coordination, pending work and event lifetime. |
| GFX-04 | Cleanup waits for GPU work and calls `PresentX` with zero planes before releasing display resources. | General lifetime and failure guarantees beyond this example. |
| GFX-05 | Xbox One and Scarlett builds link different graphics libraries; the sample diagnoses driver-version mismatch. | Actual import versus static/inlined boundaries for each binary/version. |

**Inference:** API interception remains a candidate, but discovering a C++ name
is insufficient to bind an Xbox binary. Public call sites do not reveal all
structure offsets, virtual-method slots or command-stream representations.

## What the current code actually proves

The [runtime registry](../../crates/loader/runtime/services.txt) and loader source
were searched for the runtime/device-query and console graphics names above.
No implementations of these exact Xbox APIs were identified in that audit.
This is a scoped source audit, not a proof that every equivalent helper is absent.

There is a particularly misleading existing test name:
`test_d3d12_xbox_create_device_service_through_iat` in
[graphics_d3d12.rs](../../crates/loader/tests/graphics_d3d12.rs).
Its constructed import is **`xenora_fixture.dll!CreateGraphicsDevice`**, not
`D3D12XboxCreateDevice`. The test exercises an original fixture interface;
it cannot be counted as support for the public console creation contract.
The test name/comment is retained in this research-only change and recorded here
as documentation debt. Existing resource/Metal tests remain useful foundation
coverage, but do not close any of GFX-01–05.

The [loader contract](../interfaces.md#pe32-fixture-startup-and-static-import-reports)
already labels entry, TLS and imports as synthetic startup. A static import report
cannot establish call order or find every statically linked graphics operation.

## Reference evidence that is actually available

| Reference | Available in this investigation | What it can establish |
| --- | --- | --- |
| Public Microsoft contract pages | Read without credentials; links below | Spec-derived expectations within their stated platform/version scope |
| Pinned Microsoft sample source | Read only, not compiled or executed | Observable source-level call patterns; no binary ABI or runtime proof |
| Xenora original fixtures / native Metal tests | Existing local infrastructure; not rerun for this documentation change | Internal contracts and host execution when run; no independent Xbox oracle |
| Desktop LLVM encoding reference | Existing project tooling; not rerun here | Selected desktop instruction encodings, not console execution semantics |
| Actual Series X/S differential results | Unavailable; project has no Xbox hardware access | No claims of target-equivalent behavior can be made from this investigation |

An eventual external reference result must record the original workload, target
and compatibility mode, toolchain version, exact input, output and timing/order
observations, repetitions and limitations. A screenshot alone cannot validate
resource lifetime, memory ordering or startup ABI. Any experiment must fit the
[public-source rules](../legal.md); restricted tooling is not presumed available.

## Next implementable slice and its stop conditions

1. Specify and implement an **explicit target-profile metadata reader** using
   original manifest fixtures. Test PC/XboxOne/Scarlett separation, mixed targets,
   missing fields, malformed XML and relative-path handling. Treat unsupported
   schema versions and unresolved defaults explicitly; do not fabricate platform
   semantics. This is tooling, not authentic Xbox startup.
2. Expand the public runtime ledger for initialization/uninitialization and device
   enumeration, including exact values and failure contracts. Only register guest
   imports after their ABI/linkage is supported by evidence.
3. Expand GFX-01–05 with independently sourced layouts/signatures where public.
   If a required layout cannot be established, keep it unresolved and do not add
   a guessed console alias to the synthetic graphics service.

R1 still needs executable/container and entry contracts plus parser fixtures.
R2 still needs usable binary interfaces and discriminating tests. R3 still lacks
an independent console execution reference. This ledger narrows the work without
claiming those research tasks or full Xbox compatibility are complete.

[manifest]: https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/microsoftgameconfig/elements/microsoftgameconfig-element-executable?view=gdk-2604
[overview]: https://learn.microsoft.com/en-us/gaming/gdk/docs/features/common/game-config/microsoftgameconfig-overview?view=gdk-2604
[init]: https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/xgameruntimeinit/functions/xgameruntimeinitialize?view=gdk-2604
[device]: https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/xsystem/functions/xsystemgetdevicetype?view=gdk-2604
[sample]: https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/IntroGraphics/SimpleDeviceAndSwapChain/SimpleDeviceAndSwapChain.cpp
