# Engineering progress

Xenora is an experimental research project. Verified work is restricted to
independently authored synthetic programs and shaders; commercial game
execution and authentic Xbox startup are unsupported. Progress is measured by
reproducible tests, not by estimated game compatibility percentages.

The [integrated Stage 2 resource acceptance](testing/stage2-integrated-acceptance.md)
records the current bounded resource milestone separately from shader coverage.

## Reading the progress graphic

- **Decoded forms** counts rows marked decoded in the [status registry](status.toml), out of all rows in that reviewed gfx1030 instruction inventory.
- **Accepted Metal forms** counts rows with accepted native fixture readback, over the same inventory. See the [form-by-form results](shaders/instruction-coverage.md) and [implementation checkpoint](testing/phase-02-baseline.md#implementation-checkpoint) for historical class A evidence from Apple M5. The scalar packing, signed-immediate and immediate-comparison increments in the
  [local checkpoint](testing/local-graphics-checkpoint.md#stage-31-increment-scalar-immediate-comparisons) record Apple M4 Max evidence separately.

## Feature status

<!-- status:features:start -->
| Feature | Target | Implementation | Correctness | Hardware | Evidence | Limits |
| --- | --- | --- | --- | --- | --- | --- |
| native-compute | arm64 macOS | implemented | initial-native-suite-passed | verified on Apple M5 | A | Trusted MSL with bounded word buffers and an original native offscreen rendering test on Apple M5. |
| synthetic-gfx1030 | gfx1030 Wave32 | three-fixture-demo-implemented | three-way-output-passed | verified on Apple M5 | A, B | Synthetic memory contract, statically resolved forward control flow, exact positive normal FP32 only. |
| xbox-execution | Xbox Series X\|S | unsupported | untested | unverified | U | Game execution, proprietary guest containers and full guest execution-mask divergence are unsupported. The console launch ABI is unknown. |
<!-- status:features:end -->

## Reproduce

Run on native Apple Silicon with Rust 1.100.0, Xcode 27.2, Python 3.11 or later and LLVM 23.1.6. The verified host was Apple M5 with macOS 27.3. Metal's safe and precise compile options must be available. Use up to four Cargo jobs total across at most two builds, one GPU workload and one fuzz or mutation campaign at a time. The final checkpoint used two Cargo jobs.

```sh
export CARGO_BUILD_JOBS=2
scripts/bootstrap.sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
scripts/assemble-fixture.sh --check
LLVM_BIN="$(brew --prefix llvm)/bin" cargo test --locked -p xenora-decoder --test gfx1030 -- --include-ignored --nocapture
scripts/test-gpu.sh
scripts/demo.sh
scripts/audit/native.sh
scripts/audit/claims.sh
scripts/audit/dependencies.sh
```

The independent CPU validation tests are part of the workspace. The native suite and optional LLVM oracle run separately so CPU-only CI can report them accurately. Final local logs are under ignored `artifacts/`. Keep the historical lab and candidate logs separate.

## Remaining work

See the [current limitations](../README.md#current-limitations) for the outstanding items. The active research steps are: finish the remaining RDNA2 wave execution and mesh stages, translated texture sampling, broader resource binding and synchronization. Close independent claims, status, history and review findings against the current candidates, and verify the [runtime redistribution blockers](licenses.md#runtime-redistribution-review) before closing the redistribution gate (G11).
