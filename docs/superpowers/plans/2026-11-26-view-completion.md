# Resource-view completion plan

S2.4 is complete within the finite synthetic contracts below. Every checklist item
has implementation, regression evidence and independent component acceptance.
The native color and depth Load primitives are prerequisites, not guest writable
view support. Existing synthetic shader contracts remain explicit; this plan makes
no claim about console execution.

## Acceptance checklist

- [x] Nested read-only mip ranges and selected array slices retain their root
  independently of parent lifetime.
- [x] Immutable array-range descriptors compose checked relative slice/mip ranges.
- [x] Native selected color Load preserves exact GPU-produced contents and supports
  explicit copy-back into a fresh full backing.
- [x] Native selected depth Load preserves depth-test/write semantics, immutable
  ancestors, exact pending producers, full-root accounting and failure drainage.
- [x] Guest writable color/depth selections support recording, destruction,
  pinning, capability rejection and result-buffer validation.
- [x] Guest submissions own every intermediate native point and publish all full
  backing results atomically. Failed copy-back cannot publish a rendered fragment.
- [x] Multiple writable selections of one backing compose deterministically;
  sampling observes the requested produced ancestor, independent of collection order.
- [x] Range-aware access checks distinguish slice/mip overlaps. Independent writes
  cannot lose each other's changes through whole-root snapshot replacement.
- [x] Shader-indexed array sampling carries array type, coordinate/index semantics,
  range bounds and exact producer ownership through decoder/compiler/guest/native APIs.
- [x] Writable storage views have an explicit implemented shader instruction and
  resource contract, independent expected results and rejection of unsupported forms.
- [x] Original guest integration, failure/rollback, lifetime churn, meaningful fault
  mutations and native validation pass, with two independent acceptance reviews.

## Implementation order

1. Validate native depth Load alongside the accepted color prerequisite.
2. Introduce guest writable descriptors and coordinated publication representation.
   Separate selected pass extent from complete backing layout; preserve exact typed
   metadata, root identity, allocation charges and ownership across worker boundaries.
3. Implement composite render/copy ownership and failure handling. Retain and drain
   every intermediate point. Publish a single guest completion only after all results
   validate. Do not reconstruct pending GPU storage from host snapshots.
4. Add range-aware accesses and version composition together. Merely permitting
   disjoint mip writes while publishing competing whole-root versions loses data.
5. Implement array sampling and storage instruction dependencies with original
   fixtures and public specifications. No silently ignored unsupported instructions.
6. Run integration/regression gates, independent reviews and signed-off commits.

Native depth validation uses selected slice1/mip1 values .25/.75, read-only depth
comparisons, write-enabled .5 output, a later .625 probe, complete-backing byte
comparisons, stale/foreign producer rejection, queue gating, full-root budget
retention and controlled producer-failure propagation. These are native synthetic
checks, not translated guest or hardware-fault recovery evidence.


## Completion evidence — 26 November 2026

The final nine-gate run passed with 960 workspace tests and 430 tests in the GPU
script, zero failures. The suites overlap; 128 workspace cases and one manual GPU
case were ignored and are not counted as verified GPU behavior. Local logs are in
`artifacts/validation-full.D3FtXT`. The [checkpoint](../../testing/local-graphics-checkpoint.md)
records focused counts, six storage fault mutations, restoration hashes and scope.

Array layers support checked constants/direct uniforms. Storage supports selected
RGBA8Unorm/R32Uint 2D mip/slices, ordered stores in supported branches/bounded loops,
and a single-triangle pixel-address contract. Arbitrary storage addressing, atomics,
array-dimension stores and general divergent shaders are outside these contracts;
this completion does not close all of Stage 2 or Stage 3 or establish game execution.
