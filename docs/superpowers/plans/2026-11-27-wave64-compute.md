# Explicit Wave64 compute implementation plan

> **For agentic workers:** Use subagent-driven-development for disjoint implementation, independent expected values and independent review. The user authorized completion of wave/lane support before broader control flow.

**Goal:** execute complete synthetic Wave64 compute waves while preserving existing Wave32 entry points.

**Architecture:** configuration selects the guest wave size, independently of Metal SIMD width. Ordinary SSA values stay 32-bit. Logical mask state uses two separately tracked word projections; the independent reference uses real 64-lane arrays and 64-bit masks.

**Tech stack:** Rust, generated MSL and native ARM64 Metal validation.

## Configuration and ownership

Compiler owner: `crates/shader/src/lib.rs`, `crates/shader/src/graphics.rs`, `crates/shader/src/gather_replay_tests.rs`, `crates/shader/tests/wave64.rs`.
Reference owner: `crates/ref/src/lib.rs`, `crates/ref/tests/wave64.rs`. No shared implementation helpers.
Decoder owner: `crates/decoder/src/lib.rs` and focused operand/oracle tests. Admit only audited mask-half operand sources.
Independent tests: `tests/validation/tests/wave64_independent.rs` with handwritten words and expected values.
Lead: interfaces, checkpoint, changelog, generated evidence and native integration.

```rust
pub enum WaveSize { Wave32, Wave64 }
pub enum WaveLaunch { PartialAllowed, FullWavesRequired }
pub use WaveLaunch as Wave32Launch;
pub struct WaveConfig { pub size: WaveSize, pub launch: WaveLaunch }
```

Re-export the enum name rather than using a type alias, so existing variant imports remain valid. Defaults remain Wave32/PartialAllowed. Add `compile_with_wave_config(Architecture, &[u8], WaveConfig)` and independently `execute_with_wave_config(&[u8], &[u32], WaveConfig)`. Existing options retain their fields and delegate to Wave32. CompiledShader stores private configuration, exposes `wave_config()` and `launch_policy()`, and retains the legacy `wave32_launch()` getter as a policy-only accessor. Wave64/PartialAllowed is rejected. Valid input remains 1..=4096 words, with full-wave divisibility checked before GPU submission and in generated MSL before loads.

## Implementation sequence

- [x] Add failing configuration/launch tests, then compatibility wrappers and immutable configuration metadata.
- [x] Introduce distinct mask-pair state with known/runtime/unknown halves. A half write preserves the other value and provenance; compare replaces both. Wave32 consumers ignore the high half. No unknown half becomes zero.
- [x] Emit two 32-lane comparison loops for Wave64 and test zero/nonzero using `(lo | hi)`. Reject Wave64 comparisons explicitly reading VCC_LO/HI while replacing VCC, per AMD §6.2.4. Preserve existing bounded runtime regions.
- [x] Parameterize replay wave base/lane and capture semantics. Read/write selectors mask with63; READFIRST searches all64 EXEC bits and chooses0 when empty. Reject Wave64 READLANE destination/selector SGPR overlap, even for a known selector.
- [x] Extend MBCNT: LO counts all32 low bits for lanes32–63; HI counts below lane−32 only for lanes above32. Keep wrapping accumulation and avoid shifts by32.
- [x] Keep DS routes within32-lane halves, deriving the half from the current replay context. Scatter scans32 writers, with global EXEC bit tests and independent collision policy per half. SWIZZLE preserves bit5; PERMLANEX toggles the16-lane row low bit.
- [x] Preserve distinct VM/DS readiness, pending-register rejection, source/size/replay budgets, whole-stream graphics rejection and unresolved masked-rotation rejection. Charge both comparison loops and all nested replay work.
- [x] Independently implement the same public contract in the reference with64-lane storage and u64 masks, then run separately authored expected-value cases.
- [x] Complete independent spec/code reviews, native mutation tests, regression checks and documentation before accepting the increment.

## Independent boundary matrix

| Case | Expected contract |
| --- | --- |
| Wave64 lengths0/32/63/65/4097 | Reject |
| Complete lengths64/128/192 | Execute separate logical waves |
| EXEC/VCC low0, high1 | Nonzero; EXEC lane32 active |
| Runtime VCC then one constant half write | Preserve the untouched runtime half |
| MBCNT LO(-1,0), lanes31/32/63 | 31/32/32 |
| MBCNT HI(-1,0), lanes31/32/63 | 0/0/31 |
| READLANE selectors31/32/63/64/127 | 31/32/63/0/63 |
| READFIRST high-only EXEC, empty EXEC | First selected high lane, or0 |
| DS gather address128 | Lane0 in lower half, lane32 in upper half |
| DS scatter all addresses128 | Destinations0/32 choose writers31/63 under project policy |
| PERMLANEX identity, lanes31/32/63 | Source15/48/47 |
| Wave64 scalar source/destination overlap | Explicit rejection |
| Nested transfer/compare | Correct context and captured source versions |

## Validation and evidence

Use native ARM64 and at most two concurrent two-job CPU builds; one GPU job. Run focused shader/reference tests, all shader/reference tests, all-target Clippy, applicable LLVM oracle probes, independently authored Metal readback and targeted mutations before repository validation. No expected value may come from production replay code. Update checkpoint with actual commands/results; ignored tests are not native verification.

Primary basis: AMD RDNA2 document70648, §6.2.4 p44 (scalar overlap), §10.4.4 pp96–97 (two32-lane DS domains), instruction descriptions pp137,183–187,205–209. The explicit DS domain restriction governs generic modulo64 pseudocode. Neither modern documentation corrections nor LLVM assembly prove RDNA2 hardware execution.

## Limits

This increment does not close S3.4: graphics/helper lanes, shared VGPR allocation, general dynamic EXEC, partial Wave64 waves, broader operands and unresolved RDNA2 masked rotation/inactive PERMLANE cases remain separately tracked. It does not establish a console launch ABI. S3.2 broader loops, calls, debug/trap and termination protocols follow the wave/lane work; none may become silent no-ops.

Independent source clarification: Wave32 comparisons invalidate knowledge of
VCC_HI; consulted AMD text does not establish its readable numerical value.
Wave32 mask branches ignore that half. Initial Wave32 EXEC_HI remains unknown;
an explicit supported scalar write can establish it. Wave64 initial EXEC is
all64 bits set. Preserve these distinctions in negative and positive tests.
