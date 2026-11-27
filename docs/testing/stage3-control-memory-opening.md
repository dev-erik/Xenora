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
