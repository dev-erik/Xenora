# Explicit Guest Resource Synchronization Implementation Plan

**Goal:** Finish the finite S2.6 contract for packed guest-backed resource mirrors, safe GPU-result writeback and explicit CPU/GPU synchronization.
**Architecture:** Bind existing owned buffer/texture/depth roots to a checked guest mapping. Full-root batch import/export is the coherence boundary; ordinary GPU execution uses immutable owned snapshots. Opcode49 streaming buffers retain their existing submission-time reads.
**Tech Stack:** Rust checked GuestMemory, original synthetic ABI, ARM64 Metal.

## Completion contract

- Existing buffer and typed texture creation followed by opcode75 binds the entire packed root. This supports mip/array layouts and the existing ten-format matrix without adding a second texture layout.
- Bind records mapping identities, not contents; it requires no retained submission references. Remapping or cloning memory invalidates synchronization until an explicit rebind. Permission changes are checked at each sync.
- Opcode76 takes a pointer to 1–16 root handles and direction (1 import, 0 export). Imports require COPY_DEST; exports COPY_SOURCE. Every root must be idle, including release of retained completed receipts. No implicit waiting or automatic completion writeback.
- Duplicate roots and overlapping guest spans reject. All handles, mappings, states and export permissions are preflighted. Imports stage every complete payload and depth snapshot before committing any root. Exports validate all destinations before writing under exclusive memory access. Last-root failures leave the entire batch unchanged.
- Imports advance host publication epochs, invalidating stale produced ancestry. Pending native work never aliases guest memory. Destroying a root drops its binding; delayed completion cannot write through it.
- Packed depth is supported. Opaque native depth must first be copied into a packed transfer root using the existing typed copy path.
- Explicit boundary coherence completes this milestone. Automatic shared-memory coherence, concurrent external writers, unrestricted aliasing and general dependency graphs are not part of this contract; S2.7 remains separate.

## Implementation and acceptance

- [x] Add stable region identities and distinct clone identities in crates/loader/src/memory.rs; test permission changes, spans, remapping and cloning.
- [x] Add crates/loader/src/graphics_guest_sync.rs; register bindings in GraphicsContext and remove them on destruction. Keep staging inside the existing bounded work budget.
- [x] Add opcodes75/76 in graphics_service.rs, including bounded descriptor decoding and export result-slot overlap rejection. Move unknown-opcode regression to77.
- [x] Independently test full packed roundtrips, typed/depth layouts, atomic failure, stale mappings, explicit rebind, alias denial, states and object lifetime in loader/tests/guest_sync.rs.
- [x] Run original native guest import/copy/export scenarios and repeated execution in ffi/tests/guest_graphics/guest_sync.rs. Assert literal bytes and guards, not implementation-derived expectations.
- [x] Inject decisive isolated faults in identity/preflight/publication behavior; require assertion failures and restore exact source bytes.
- [x] Obtain two independent reviews of components and evidence. Verify LOW reviewer configurations.
- [x] Update ABI/interface docs, changelog, index, checkpoint and focused validation filters; run all nine gates with one GPU workload and at most four Cargo jobs.
- [x] Create signed-off commit, push the existing fork branch, verify remote hash and clean tree. Report finite completion and outstanding S2.7/integrated Stage2 scope.

Commands: `CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=4 cargo test --locked -p xenora-loader --test guest_sync`; memory identity unit tests; native guest filter `original_guest_sync_` with Metal validation and `--test-threads=1`; `scripts/validate-graphics.sh full`.
