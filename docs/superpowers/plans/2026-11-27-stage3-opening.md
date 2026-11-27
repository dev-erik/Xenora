# S3 control, wave and memory opening plan

**Goal:** begin S3.2, S3.4 and S3.5 with audited scope and a validated scalar-branch increment.

**Architecture:** typed gfx1030 decoder, existing verified compute/graphics lowering, original fixtures and native ARM64 Metal. See [design](../specs/2026-11-27-stage3-control-wave-memory.md).

**Tech stack:** Rust, Python inventory generation, LLVM encoding oracle, Metal.

- [x] Audit the provisional stage classifier; include SOPP and memory families and separate DS permutations.
- [x] Add inventory regression tests in `tests/qa/test_shader_checklist.py`; regenerate `docs/shaders/shader-checklist.md` and its graph without inflating implementation counts.
- [x] S3.2: add SCC1 typed instruction, opcode generation, decoder target checks, compute predicate, graphics predicate and CFG handling. Files: `crates/isa/src/lib.rs`, `crates/isa/generate_rdna2.py`, generated opcode data, `crates/decoder/src/lib.rs`, `crates/shader/src/lib.rs`, `crates/shader/src/graphics.rs`, `crates/shader/src/graphics/cfg.rs`.
- [x] Independently test SCC1 taken/untaken behavior, unknown SCC, signed targets, invalid targets and cycle restrictions; validate LLVM bytes and native compute/graphics readback.
- [x] S3.4: record exact lane-operation prerequisites and first safe implementation slice, including inactive and partial-wave behavior.
- [x] S3.5: record full memory/interface families, synchronization contracts and first safe implementation slice.
- [x] Run relevant native ARM64 checks with at most four Cargo jobs and one GPU job, then two independent reviews.
- [x] Update precise evidence, limitations and changelog; regenerate maps; signed-off commit and push to the authorized fork.

No stage is closed by this opening plan. Subsequent wave and memory increments need their own exact operand/semantic contracts and tests. The broader stage feature denominator remains under audit.

Validated first increments and exact limitations are recorded in the [checkpoint](../../testing/stage3-control-memory-opening.md). S3.4 audit completion does not mean implementation completion.

## Continuation: Wave32 masked bit count

The request to complete S3.2, S3.4 and S3.5 remains broader than this increment.
This checklist does not redefine full-stage acceptance as decoder coverage.

- [x] Check public AMD semantics for MBCNT LO/HI and the existing Wave32 ABI.
- [x] Implement typed decoding and lane-aware compute lowering; reject graphics.
- [x] Add independently derived CPU execution, original LLVM assembly and readback cases.
- [x] Validate source modes, aliasing, partial EXEC and final partial waves on Metal.
- [x] Verify lane-dependent values inside recomputed comparison expressions.
- [x] Run the LLVM oracle, regression gates and two independent reviews.
- [x] Refresh recorded evidence, remaining scope and both generated shader maps; commit.

Next control-flow work must retain target-boundary and termination guarantees.
Next memory work must specify descriptors, address spaces and visibility before
admitting additional forms. Cross-lane reads require a separate contract for
inactive and absent source lanes; masked bit count alone does not provide it.

## Continuation: nonzero mask branches and narrow buffer loads

- [x] Add Wave32 compute EXECNZ/VCCNZ with unchanged forward/runtime-region bounds.
- [x] Validate independent references, LLVM encodings, native merge polarity,
  partial waves and malformed/skipped regions; reject both graphics stages.
- [x] Add byte/halfword signed and unsigned loads with the existing four-byte
  input stride, single pending load, full EXEC and zero-offset restrictions.
- [x] Validate sign boundaries, high-bit contamination, waits, source readiness,
  descriptor/index rejection and lane-derived comparison recomputation.
- [x] Review each increment independently, update evidence/maps and commit.

Full closure of S3.2/S3.4/S3.5 still requires the architectural work listed in the
[scope document](../../shaders/control-wave-memory.md#remaining-closure-work).
