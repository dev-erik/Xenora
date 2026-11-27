# S3.2 / S3.5 first bounded increments

27 November 2026. Desktop gfx1030 original programs on native ARM64 Metal.
No Xbox execution or full-stage completion is claimed. The [masked-bit-count
continuation](#wave32-masked-bit-count-continuation) below records the later S3.4 increment.

## Accepted behavior

- `S_CBRANCH_SCC1`: complement of SCC0, preserving SCC. Compute retains its forward-only execution contract; decoder accepts signed targets subject to full-stream boundaries. Graphics admits constant/runtime SCC and complementary bounded countdown exits. Proven zero-trip paths preserve the correct exit predicate, including signed negative bounds. General indirect calls, loops and divergence remain unsupported.
- `S_WAITCNT_VMCNT null,0`: exact `0xBC7D0000`, distinct instruction identity, completes the synthetic compute pending load only. Does not complete stores or change SCC. Redundant waits work; other operands/thresholds and graphics use are rejected.

## Full validation

`CARGO_BUILD_JOBS=2 CARGO_BUILD_TARGET=aarch64-apple-darwin LLVM_BIN=/opt/homebrew/opt/llvm/bin ./scripts/validate-graphics.sh full`
passed all nine gates on Apple M4 Max / macOS 27.2.1. Workspace: 1,328 passed,
0 failed, 174 ignored. GPU gate: 513 passed, 0 failed, 1 ignored. Ignored tests
are not counted as verified. Logs: `artifacts/validation-full.MBLFp4`.
This validates the bounded existing project suites, not complete S3 or Xbox support.

## Targeted evidence

- Independent SCC1 suite: 11 passed including native readback; zero skipped when explicitly run with `--include-ignored`.
- Independent VMCNT suite: 11 passed including native readback; zero skipped with `--include-ignored`. Each independent file also includes four existing reference self-tests; these totals are not all newly authored tests.
- Dedicated Metal SCC1 suite: 2 passed, zero skipped; 12 compute dispatches and eight runtime graphics draws. Metal API and shader validation enabled.
- LLVM: 1,205 three-way instruction comparisons across 166 forms passed. This validates encoding, not Xbox execution.
- Two semantic mutation kills: reversed SCC1 detected by native literal output; incorrectly completing stores in a load wait detected by independent rejection assertions. Production source restored exactly. The CPU-only SCC1 compile/reference test did not detect the production branch inversion; native comparison is required and is part of the GPU gate.
- Five Python stage-inventory regression tests passed.
- All nine full validation gates passed; logs: `artifacts/validation-full.gT975A`.
- Two independent static reviewers accepted production semantics and the zero-trip correction. Their acceptance is bounded to the contracts above.

Original assembly and binaries: `fixtures/rdna2/scc1-branch.*` and
`fixtures/rdna2/wait-vmcnt.*`; both are registered in `scripts/assemble-fixture.sh`.
The standalone SCC1 native suite is included in `scripts/test-gpu.sh`; the existing
independent-review GPU gate runs both new ignored native tests.

## Reproduce targeted checks

```sh
CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-shader
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test sopp_scc1_independent --test sopk_vmcnt_independent -- --include-ignored --test-threads=1
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-metal --features gpu-tests --test scc1_branch -- --test-threads=1
LLVM_BIN=/opt/homebrew/opt/llvm/bin CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-decoder --test gfx1030 compares_fixture_instructions_against_llvm -- --ignored --nocapture
```

## Remaining stage work

At the first-increment checkpoint, the provisional family-inclusive partition recorded S3.2
6/24 decoded, S3.4 0/11 and S3.5 10/509. These are not accepted-mode completion
fractions. State assignment and operand-mode requirements remain separate; scalar comparisons
retain their historical S3.1 assignment pending an ownership audit.
The prior 7/8/12 totals were incomplete arithmetic-family buckets; correcting the
partition does not add or remove implementation support.

## Wave32 masked-bit-count continuation

27 November 2026. `V_MBCNT_LO_U32_B32` and `V_MBCNT_HI_U32_B32` now have
typed decoding, bounded Wave32 compute lowering and independent CPU execution.
The [contract](../shaders/control-wave-memory.md#s34-wave-and-lane) specifies
source admission, wrapping accumulation, inactive destination retention and
comparison recomputation. Both graphics stages reject the forms, including
unreachable occurrences. Wave64 and cross-lane movement remain unsupported.

- Independent suite: 6 passed, zero ignored with native execution explicitly enabled.
  It executes 248 original programs on Metal with API and shader validation enabled.
  Cases cover lengths 1/31/32/33/63/65; lane 0/1/31 and repeated wave boundaries;
  scalar/inline/literal sources; wrapping, sparse inputs and source/destination
  aliasing; EXEC masks and retained sentinels; constant-source LO and masked HI
  comparison cones with literal expected masks.
- LLVM: 1,214 three-way instruction comparisons across 168 forms passed. The
  expanded bitfield source matrix contains 536 byte-and-reference comparisons.
- Original fixture reassembly passed; the fixture is registered in the standard
  assembly check and the native test is picked up by the independent GPU gate.
- All nine full validation gates passed; logs: `artifacts/validation-full.gT975A`.
- Two independent static reviewers found no blocking defect in the bounded contract;
  one also inspected the native log and test case count. The separate historical
  second reference interpreter does not yet implement MBCNT; this is not a claim
  of agreement between both reference interpreters.
- An incorrect LO lane calculation was detected by the independent native
  readback assertion; production source was restored exactly. Evidence:
  `artifacts/s34-mbcnt/mutation.json` and `mutation-lane.log`.
- Logs: `artifacts/s34-mbcnt/native.log`, `cpu.log`, `fixtures.log`.

```sh
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test vop3_mbcnt_independent -- --include-ignored --nocapture --test-threads=1
LLVM_BIN=/opt/homebrew/opt/llvm/bin CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-decoder --test gfx1030 llvm -- --ignored --nocapture
```

S3.4 now has 2/11 forms decoded under this bounded compute contract. S3.2 remains
6/24 and S3.5 10/509. No stage is complete; decoder counts exclude remaining
operand-mode, graphics, Wave64 and synchronization obligations.

## Nonzero mask branch continuation

`S_CBRANCH_EXECNZ` and `S_CBRANCH_VCCNZ` are implemented under the bounded
Wave32 compute contract. Known masks resolve statically; lane-derived VCC uses
polarity-aware runtime branches and register joins. The previous VCCZ behavior
and region whitelist are retained. Both graphics stages reject the new forms,
including skipped instructions. General loops and divergence remain unsupported.

- Native independent suite: 13 passed, zero ignored, including 224 original Metal
  programs with API/shader validation. Four tests are existing reference self-tests.
- Both independently implemented references agree with separately derived expected
  outputs, including partial EXEC sentinel retention and post-join comparisons.
- Decoder/compiler/reference regressions: 375 passed, 15 optional LLVM tests ignored.
  Separately, the LLVM fixture oracle passed 1,230 comparisons across 170 forms.
- Two independent static reviewers found no blocking defect in the bounded contract.
- Native readback caught an intentionally inverted runtime VCCNZ predicate;
  source was restored and all 13 tests passed again. Workspace Clippy, formatting,
  original fixture reassembly and generated-document checks passed.
- Logs: `artifacts/s32-mask-nonzero/native.log`, `restored-native.log`, `cpu.log`,
  `llvm.log`, `targets.log`, `mutation.json`, `clippy.log`.

```sh
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test branch_mask_nonzero -- --include-ignored --nocapture --test-threads=1
```

S3.2 now has 8/24 decoded forms, S3.4 has 2/11 and S3.5 has 10/509. These remain
open workstreams; the totals do not include operand-mode and architecture debt.

## Narrow buffer-load continuation

`BUFFER_LOAD_UBYTE`, `BUFFER_LOAD_SBYTE`, `BUFFER_LOAD_USHORT` and
`BUFFER_LOAD_SSHORT` now use the existing synthetic four-byte input stride.
Each reads its invocation's low byte/halfword and zero/sign-extends it into
one VGPR. Full EXEC, zero offsets, fixed bindings/index, one pending load and
separate store completion remain required. Graphics rejects all four forms,
including skipped instructions. The [memory contract](../shaders/control-wave-memory.md)
and [ABI decision](../adr/0003-demo-memory-model.md) retain these limits.

- Independent suite: 11 passed, zero ignored with native execution enabled.
  It includes 310 Metal programs, API/shader validation enabled, and four existing
  reference self-tests. Both independent references match literal expected values.
- Cases cover lengths 1/31/32/33/65, signed boundaries, independently varied upper
  bits, direct output and comparison masks, both load waits, all 25 ordered pairs
  of DWORD/narrow loads, and initialized-but-pending destination hazards.
- LLVM oracle: 1,248 three-way comparisons across 174 forms passed.
- Two independent static reviewers found no blocking defect in the bounded contract.
- A deliberate zero-extension mutation for signed byte loads was caught by native
  output assertions; production source was restored exactly.
- Logs: `artifacts/s35-narrow-load/native.log`, `llvm.log`, `references.log`,
  `mutation.json` and `mutation.log`.

```sh
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test buffer_narrow_load -- --include-ignored --nocapture --test-threads=1
```

Current decoded counts: S3.2 **8/24**, S3.4 **2/11**, S3.5 **14/509**.
All three remain open. General control flow, the other lane operations,
address spaces, descriptors, atomics, synchronization and broader image/stage
interfaces still require separate implementation and acceptance.

The generated semantic-coverage report previously lagged at 134 decoded forms.
Its evidence parser also rejected older MSL lowering descriptions that lacked
its recognized prefix. Those descriptions now use the existing schema; the
parser and its rejection rules were not weakened. The full validation status
gate now checks this report as well as the instruction checklist and maps.

Final combined validation: all nine gates passed on native ARM64 Metal. Workspace:
1,380 passed, zero failed, 177 ignored. GPU gate:
516 passed, zero failed, 1 ignored. Ignored tests do not count as verified.
Logs: `artifacts/validation-full.L6zk4W`. Source hashes and gate counts:
`artifacts/s35-narrow-load/verified-source.json`.


## Lane-zero read continuation

VOP1 READFIRSTLANE and VOP3 READLANE now admit provably selected lane zero
under the [bounded interface](../interfaces.md#bounded-wave32-lane-read-scalar-values).
The immutable source version is replayed at the logical Wave32 base; runtime
scalar results may be consumed through vector moves. EXEC zero and inactive
source retention are handled explicitly. Nonzero selected lanes, Wave64,
VOP3 READFIRSTLANE, graphics and unsupported scalar consumers remain rejected.

- Independent suite: 7 passed, zero ignored, including 312 original native
  programs with Metal API and shader validation enabled. Expected outputs are
  derived in the test independently of the primary reference and production
  compiler; the separate secondary interpreter is not extended by this increment.
- Six dispatch lengths cover 1/31/32/33/63/65, retained inactive lane zero,
  selector masking, repeated snapshots, conditional scalar overwrites, comparison
  masks and VCC branch joins. Malformed operands and unready sources reject.
- LLVM oracle: 28 tests passed, including 1,261 three-way instruction comparisons
  across 176 forms. Original fixture reassembly passed.
- Two independent static reviews found no remaining blocker in the bounded slice.
- A deliberately wrong generated-MSL lane selection was caught by the native
  output assertion. The temporary test mutation was restored exactly; this is
  fault injection into generated MSL, not a full production-source mutation campaign.
- Logs: `artifacts/s34-lane-read/native.log`, `llvm.log`, `assembly.log` and
  `wrong-lane-mutation.log`.

```sh
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test lane_read_independent -- --include-ignored --nocapture --test-threads=1
```

S3.4 has **4/11 decoded forms**, with these two new forms accepted only for the
lane-zero subset. Seven encoding forms and substantial operand/architecture
work remain. S3.2 remains **8/24**, deferred behind the requested S3.4 work.
Full-stage completion requires explicit wave configuration, broader scalar and
lane state, graphics participation, and the remaining transfer/permute modes.

Full validation for this increment: all nine gates passed. Workspace: **1,402
passed, zero failed, 178 ignored**. GPU gate: **517 passed, zero failed, one
ignored**. Ignored tests are not verification. Logs: `artifacts/validation-full.TsSVQl`.


## Selected-lane write continuation

VOP3 WRITELANE now writes an ordinary scalar value to one selected Wave32 lane
independently of EXEC. Other lanes retain the captured destination version.
The bounded contract supports constant or previously captured lane-read data,
a known ordinary SGPR selector, and initialized v1–v255 destinations. Absent
destination lanes leave present invocations unchanged. Wider operand modes,
runtime selectors, graphics and runtime-region lane writes remain unsupported.

- Independent native suite: **6 passed, zero ignored**, covering **504 original
  programs** with Metal API/shader validation enabled. Nine dispatch lengths and
  eight selector values exercise row/wave boundaries and absent destinations.
- Coverage includes EXEC zero, partial masks, retained lanes, scalar snapshots,
  repeated writes, comparison and runtime-join replay, and decoder/readiness errors.
- Two independent static reviewers found no blocking defect in the bounded slice.
- Native output checks detected an intentional production mutation that
  incorrectly intersected the write mask with EXEC. Production was restored
  exactly; both lane-read and lane-write native suites then passed (**13 tests,
  zero ignored**, 312 read programs plus 504 write programs).
- Logs: `artifacts/s34-lane-write/native.log`, `native-restored.log` and
  `exec-mutation.log`. Expected outputs come from test-local independent
  calculations; the separate secondary interpreter is not extended here.

```sh
MTL_DEBUG_LAYER=1 MTL_SHADER_VALIDATION=1 CARGO_BUILD_JOBS=2 cargo test --locked --target aarch64-apple-darwin -p xenora-semantic-review --test lane_write_independent -- --include-ignored --nocapture --test-threads=1
```

S3.4 is **5/11 decoded forms**, not a completed stage. Six forms plus broader
lane selection, operand modes, Wave64, dynamic state and graphics participation
remain. S3.2 remains **8/24**; no control-flow scope was silently dropped.

Focused regression validation for the write increment: **715 passed, zero
failed, 49 ignored** across decoder/compiler/reference and independent CPU tests.
The separate LLVM run passed **29 tests**, including **1,274 comparisons across
177 forms**. Workspace all-targets Clippy, formatting, fixture reassembly, five
inventory tests, generated-map consistency, claims and dependency audits passed.
The nine-gate full-suite result above belongs to commit `3465da0`; it is not
represented as a fresh full-suite result for this subsequent write increment.


## Full-wave lane-read continuation

An explicit full-Wave32 launch requirement now permits every constant lane
selection and initialized runtime ordinary-SGPR selectors captured by earlier
lane reads. The default partial-wave entry points remain unchanged. Typed
Metal execution rejects invalid lengths before submission. The independent
reference declares and enforces its own launch options.

The independent native corpus passed **6 tests, zero ignored**, covering
**127 original programs and 139 dispatches**, plus five host launch-rejection
checks. Coverage includes all 32 selections, wrapped selectors, source/selector
snapshots and aliasing, retained inactive values, comparison joins and threadgroup
widths 2/17/32/64. An initial width-one harness configuration exceeded the existing
64-group cap; only that test configuration was corrected. The cap was preserved.

Two independent static reviewers approved this bounded contract. A deliberate
production mutation that replayed lane zero instead of the selected lane failed
an independent native output assertion. The original source was restored exactly.
Logs: `artifacts/s34-full-wave/native-restored.log` and
`artifacts/s34-full-wave/wrong-lane-mutation.log`. These checks do not establish
Wave64, graphics participation, general scalar execution or full S3.4 closure.


## Full-wave transfer continuation

The bounded transfer layer adds DS_BPERMUTE, DS_SWIZZLE, PERMLANE16 and
PERMLANEX16, plus an independently decoded LGKM-zero wait. It also adds a
spec-derived VOP3 READFIRSTLANE subset. Native execution remains synthetic
Wave32 compute; see the exact [interface contracts](../interfaces.md#full-wave-transfers-and-ds-completion).

- Independent CPU coverage includes 183 transfer programs and 36 e64 READFIRST
  programs, plus readiness, scalar, graphics, encoding and disputed-mode rejections.
- The combined native run passed **29 tests, zero ignored**, with **1,093 GPU
  dispatches**: 138 transfer/e64 dispatches, 139 general lane-read dispatches,
  312 original lane-read programs and 504 lane-write programs. Metal API and
  shader validation were enabled.
- A deliberate identity-routing mutation failed an independent native output
  assertion. The production source was restored exactly afterward.
- Independent review identified the masked-rotation source contradiction.
  Compiler and reference now reject nonzero-mask rotation, including unreachable
  occurrences, instead of silently choosing one of the inconsistent descriptions.
  Both reviewers approved the resulting bounded layer.
- Logs: `artifacts/s34-transfers/native.log` and
  `artifacts/s34-transfers/identity-route-mutation.log`.

S3.4 is **10/11 decoded forms**. DS_PERMUTE, masked rotation, broader operand
and EXEC modes, Wave64 and graphics/helper-lane semantics remain open. S3.2
is still **8/24 decoded forms**; no control-flow completion is claimed.
The e64 READFIRST form has **no LLVM or native AMD encoding validation**.
Its Metal result establishes translated behavior only.

Full regression acceptance for this continuation: **all nine gates passed**.
Workspace: **1,465 passed, zero failed, 182 ignored**. GPU gate: **520 passed,
zero failed, one ignored**. Ignored tests are not counted as verification.
Logs: `artifacts/validation-full.BgbmCy`. The separate encoding run passed
**40 tests**, including **1,311 three-way instruction comparisons across
181 forms** (`artifacts/s34-transfers/llvm-restored.log`). E64 READFIRST is
excluded from that LLVM claim. The fixture inventory still requires every
declared form; spec-derived raw words carry explicit expected mnemonics and
are checked against byte identification, rather than being excluded.
