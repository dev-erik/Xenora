// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent explicit guest synchronization contract tests. CPU only; no GPU evidence.
//! Expected byte counts and payloads are independent of production layout helpers.

use xenora_loader::graphics::{GraphicsContext, GraphicsHandle, ResourceStates};
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{ExitReason, GuestMemory, MemoryPermissions};

const DESC: u64 = 0x1000;
const OUT: u64 = 0x2000;
const HANDLES: u64 = 0x3000;
const SCRATCH: u64 = 0x4000;
const BACKING: u64 = 0x8000;
const SENTINEL: u64 = 0x9182_7364_5546_3728;

struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
    device: u64,
}

impl Harness {
    fn new() -> Self {
        let mut memory = GuestMemory::new();
        for (base, len) in [(DESC, 128), (OUT, 8), (HANDLES, 256), (SCRATCH, 4096)] {
            memory
                .map_region(base, len, MemoryPermissions::READ_WRITE)
                .unwrap();
        }
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap().to_u64();
        Self {
            ctx,
            memory,
            device,
        }
    }

    fn prepare(&mut self, opcode: u64, args: &[u64]) {
        let mut words = [0u64; 16];
        words[0] = 1;
        words[1] = opcode;
        words[2..2 + args.len()].copy_from_slice(args);
        let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        self.memory.write_bytes(DESC, &bytes).unwrap();
        self.memory.write_u64(OUT, SENTINEL).unwrap();
    }

    fn call(&mut self, opcode: u64, args: &[u64]) -> Result<u64, ExitReason> {
        self.prepare(opcode, args);
        graphics_call_v1(&mut self.ctx, &mut self.memory, DESC, 128, OUT)
    }

    fn ok(&mut self, opcode: u64, args: &[u64]) -> u64 {
        assert_eq!(self.call(opcode, args), Ok(0), "opcode {opcode}");
        self.memory.read_u64(OUT).unwrap()
    }

    fn buffer(&mut self, len: usize, state: ResourceStates) -> u64 {
        self.ok(5, &[self.device, len as u64, u64::from(state.0)])
    }

    fn map(&mut self, address: u64, bytes: &[u8], permissions: MemoryPermissions) {
        self.memory
            .map_region_with_data(address, bytes.len(), permissions, bytes)
            .unwrap();
    }

    fn bytes(&self, address: u64, len: usize) -> Vec<u8> {
        let mut bytes = vec![0; len];
        self.memory.read_bytes(address, &mut bytes).unwrap();
        bytes
    }

    fn sync(
        &mut self,
        roots: &[u64],
        import: bool,
    ) -> Result<(), xenora_loader::graphics::GraphicsError> {
        let roots: Vec<_> = roots
            .iter()
            .copied()
            .map(GraphicsHandle::from_u64)
            .collect();
        self.ctx
            .synchronize_guest_memory(&roots, import, &mut self.memory)
    }

    fn bind(&mut self, root: u64, address: u64) {
        self.ctx
            .bind_guest_memory(GraphicsHandle::from_u64(root), address, &self.memory)
            .unwrap();
    }

    fn upload(&mut self, root: u64, bytes: &[u8]) {
        self.memory.write_bytes(SCRATCH, bytes).unwrap();
        self.ok(9, &[root, 0, SCRATCH, bytes.len() as u64]);
    }

    fn transition(&mut self, roots: &[u64], before: ResourceStates, after: ResourceStates) {
        let queue = self.ok(2, &[self.device]);
        let allocator = self.ok(3, &[self.device]);
        let list = self.ok(4, &[self.device, allocator]);
        for root in roots {
            self.ok(13, &[list, *root, u64::from(before.0), u64::from(after.0)]);
        }
        self.ok(12, &[list]);
        self.ok(25, &[queue, list]);
        // Release recorded references so destruction tests exercise binding lifetime.
        self.ok(11, &[list, allocator]);
    }

    fn read_owned(&mut self, root: u64, len: usize) -> Vec<u8> {
        self.ok(10, &[root, 0, SCRATCH, len as u64]);
        self.bytes(SCRATCH, len)
    }
}

#[test]
fn cpu_guest_sync_all_typed_formats_full_mip_array_and_depth_roundtrip() {
    // 5x3, 2x1, 1x1 = 18 texels per slice; two slices = 36 texels.
    // BC roots have four blocks per slice (2 + 1 + 1), eight blocks total.
    // Literal guest format codes, sizes and raw patterns include signed zero / NaN bits.
    let cases: &[(u64, usize, &[u8])] = &[
        (0, 144, &[17, 53, 127, 193]),
        (1, 64, &[0, 248, 224, 7, 228, 27, 177, 78]),
        (
            2,
            128,
            &[
                231, 17, 136, 198, 250, 136, 198, 250, 31, 0, 0, 248, 228, 27, 177, 78,
            ],
        ),
        (3, 36, &[127]),
        (4, 72, &[17, 193]),
        (5, 144, &[7, 11, 13, 19]),
        (6, 288, &[0, 188, 0, 64, 0, 128, 53, 126]),
        (7, 144, &[69, 35, 193, 127]),
        (8, 144, &[1, 0, 0, 1]),
        (9, 144, &[0, 0, 0, 128]),
    ];
    for &(code, len, pattern) in cases {
        let mut h = Harness::new();
        let root = h.ok(53, &[h.device, 5, 3, 3, 2, code, 0x400]);
        let mut expected: Vec<_> = pattern.iter().copied().cycle().take(len).collect();
        // Distinguish the final mip/slice and catch truncated full-root transfers.
        expected[len - 1] = 0x5b;
        h.map(BACKING, &expected, MemoryPermissions::READ_WRITE);
        h.ok(75, &[root, BACKING]);
        h.memory.write_u64(HANDLES, root).unwrap();
        h.ok(76, &[HANDLES, 1, 1]);
        h.memory.write_bytes(BACKING, &vec![0xa5; len]).unwrap();
        h.transition(
            &[root],
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        assert_eq!(h.read_owned(root, len), expected, "import format {code}");
        assert_eq!(
            h.bytes(BACKING, len),
            vec![0xa5; len],
            "no automatic export"
        );
        h.ok(76, &[HANDLES, 1, 0]);
        assert_eq!(h.bytes(BACKING, len), expected, "export format {code}");
    }
}

#[test]
fn cpu_guest_sync_bind_is_metadata_only_and_changes_need_explicit_import() {
    let mut h = Harness::new();
    let root = h.buffer(8, ResourceStates::COPY_DEST);
    let original = [2, 3, 5, 7, 11, 13, 17, 19];
    let guest = [23, 29, 31, 37, 41, 43, 47, 53];
    h.upload(root, &original);
    h.map(BACKING, &guest, MemoryPermissions::READ_WRITE);
    h.bind(root, BACKING);
    h.transition(
        &[root],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    assert_eq!(h.read_owned(root, 8), original);
    assert_eq!(h.bytes(BACKING, 8), guest);
    h.transition(
        &[root],
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_DEST,
    );
    h.sync(&[root], true).unwrap();
    h.memory.write_bytes(BACKING, &[0xee; 8]).unwrap();
    h.transition(
        &[root],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    assert_eq!(h.read_owned(root, 8), guest);
    h.sync(&[root], false).unwrap();
    assert_eq!(h.bytes(BACKING, 8), guest);
}

#[test]
fn cpu_guest_sync_last_unreadable_root_preserves_buffer_and_depth_imports() {
    let mut h = Harness::new();
    let buffer = h.buffer(8, ResourceStates::COPY_DEST);
    let depth = h.ok(53, &[h.device, 1, 1, 1, 0, 9, 0x400]);
    let bad = h.buffer(8, ResourceStates::COPY_DEST);
    h.upload(buffer, &[0x17; 8]);
    h.upload(depth, &[0, 0, 128, 62]); // literal 0.25f32 bits
    h.map(BACKING, &[0x29; 8], MemoryPermissions::READ_ONLY);
    h.map(
        BACKING + 0x100,
        &[0, 0, 64, 63],
        MemoryPermissions::READ_ONLY,
    );
    h.map(BACKING + 0x200, &[0x31; 8], MemoryPermissions::NO_ACCESS);
    for (root, address) in [
        (buffer, BACKING),
        (depth, BACKING + 0x100),
        (bad, BACKING + 0x200),
    ] {
        h.bind(root, address); // Binding requires mapping, not read permission.
    }
    assert!(h.sync(&[buffer, depth, bad], true).is_err());
    h.transition(
        &[buffer, depth],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    assert_eq!(h.read_owned(buffer, 8), [0x17; 8]);
    assert_eq!(h.read_owned(depth, 4), [0, 0, 128, 62]);
}

#[test]
fn cpu_guest_sync_last_readonly_tail_preserves_entire_export_batch() {
    let mut h = Harness::new();
    let first = h.buffer(8, ResourceStates::COPY_DEST);
    let last = h.buffer(8, ResourceStates::COPY_DEST);
    h.upload(first, &[0x11; 8]);
    h.upload(last, &[0x22; 8]);
    h.map(BACKING, &[0xa1; 8], MemoryPermissions::READ_WRITE);
    h.map(BACKING + 0x100, &[0xb2; 4], MemoryPermissions::READ_WRITE);
    h.map(BACKING + 0x104, &[0xc3; 4], MemoryPermissions::READ_ONLY);
    h.bind(first, BACKING);
    h.bind(last, BACKING + 0x100);
    h.transition(
        &[first, last],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    assert!(h.sync(&[first, last], false).is_err());
    assert_eq!(h.bytes(BACKING, 8), [0xa1; 8]);
    assert_eq!(
        h.bytes(BACKING + 0x100, 8),
        [0xb2, 0xb2, 0xb2, 0xb2, 0xc3, 0xc3, 0xc3, 0xc3]
    );
}

#[test]
fn cpu_guest_sync_duplicate_overlap_wrong_state_and_handles_reject_atomically() {
    for import in [true, false] {
        let mut h = Harness::new();
        let a = h.buffer(8, ResourceStates::COPY_DEST);
        let b = h.buffer(8, ResourceStates::COPY_DEST);
        let unbound = h.buffer(8, ResourceStates::COPY_DEST);
        let wrong = h.buffer(8, ResourceStates::COMMON);
        for root in [a, b, unbound] {
            h.upload(root, &[0x17; 8]);
        }
        h.map(BACKING, &[0x29; 32], MemoryPermissions::READ_WRITE);
        h.bind(a, BACKING);
        h.bind(b, BACKING + 4);
        h.bind(wrong, BACKING + 16);
        if !import {
            h.transition(
                &[a, b, unbound],
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
        }
        for roots in [
            vec![],
            vec![a, a],
            vec![a, b],
            vec![a, wrong],
            vec![a, unbound],
            vec![a, h.device],
            vec![a, 0],
            vec![a; 17],
        ] {
            assert!(
                h.sync(&roots, import).is_err(),
                "import={import}, roots={roots:?}"
            );
            assert_eq!(h.bytes(BACKING, 32), [0x29; 32]);
        }
        if import {
            h.transition(
                &[a, b],
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
        }
        assert_eq!(h.read_owned(a, 8), [0x17; 8]);
        assert_eq!(h.read_owned(b, 8), [0x17; 8]);
    }
}

#[test]
fn cpu_guest_sync_bind_rejects_nonroot_streaming_unmapped_and_destroyed_handles() {
    let mut h = Harness::new();
    h.map(BACKING, &[0x29; 32], MemoryPermissions::READ_WRITE);
    let streaming = h.ok(49, &[h.device, 8, 0x800, BACKING]);
    let root = h.buffer(8, ResourceStates::COPY_DEST);
    h.bind(root, BACKING);
    for (handle, address) in [
        (h.device, BACKING),
        (streaming, BACKING),
        (root, 0xf000),
        (root, u64::MAX - 3),
    ] {
        assert!(
            h.ctx
                .bind_guest_memory(GraphicsHandle::from_u64(handle), address, &h.memory)
                .is_err()
        );
    }
    // Failed replacement must preserve the previous binding.
    h.sync(&[root], true).unwrap();
    h.ok(26, &[root]);
    assert!(h.sync(&[root], true).is_err());
    assert!(
        h.ctx
            .bind_guest_memory(GraphicsHandle::from_u64(root), BACKING, &h.memory)
            .is_err()
    );
    let replacement = h.buffer(8, ResourceStates::COPY_DEST);
    assert!(h.sync(&[replacement], true).is_err());
    assert_eq!(h.bytes(BACKING, 32), [0x29; 32]);
}

#[test]
fn cpu_guest_sync_export_result_slot_overlap_and_noncanonical_direction_reject() {
    let mut h = Harness::new();
    let root = h.buffer(16, ResourceStates::COPY_DEST);
    h.upload(root, &[0x17; 16]);
    h.map(BACKING, &[0x29; 16], MemoryPermissions::READ_WRITE);
    h.bind(root, BACKING);
    h.transition(
        &[root],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    h.memory.write_u64(HANDLES, root).unwrap();
    for offset in [0, 4, 8] {
        h.prepare(76, &[HANDLES, 1, 0]);
        let result = graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, BACKING + offset);
        assert!(matches!(result, Ok(status) if status != 0));
        assert_eq!(h.bytes(BACKING, 16), [0x29; 16]);
    }
    for args in [
        [HANDLES, 1, 2],
        [HANDLES, 1, 1u64 << 32],
        [HANDLES, 0, 0],
        [HANDLES, 17, 0],
    ] {
        assert!(matches!(h.call(76, &args), Ok(status) if status != 0));
        assert_eq!(h.memory.read_u64(OUT).unwrap(), SENTINEL);
        assert_eq!(h.bytes(BACKING, 16), [0x29; 16]);
    }
    h.ok(76, &[HANDLES, 1, 0]);
    assert_eq!(h.bytes(BACKING, 16), [0x17; 16]);
}

#[test]
fn cpu_guest_sync_clone_invalidates_both_directions_until_explicit_rebind() {
    for import in [true, false] {
        let mut h = Harness::new();
        let a = h.buffer(8, ResourceStates::COPY_DEST);
        let b = h.buffer(8, ResourceStates::COPY_DEST);
        h.upload(a, &[0x17; 8]);
        h.upload(b, &[0x19; 8]);
        h.map(BACKING, &[0x29; 8], MemoryPermissions::READ_WRITE);
        h.map(BACKING + 8, &[0x31; 8], MemoryPermissions::READ_WRITE);
        h.bind(a, BACKING);
        h.bind(b, BACKING + 8);
        if !import {
            h.transition(
                &[a, b],
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
        }
        let original = h.memory.clone();
        h.memory = h.memory.clone();
        assert!(h.sync(&[a, b], import).is_err());
        h.bind(a, BACKING);
        // A valid first root followed by a stale last root must still be atomic.
        assert!(h.sync(&[a, b], import).is_err());
        assert_eq!(
            h.bytes(BACKING, 16),
            [
                0x29, 0x29, 0x29, 0x29, 0x29, 0x29, 0x29, 0x29, 0x31, 0x31, 0x31, 0x31, 0x31, 0x31,
                0x31, 0x31
            ]
        );
        if import {
            h.transition(
                &[a, b],
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
            assert_eq!(h.read_owned(a, 8), [0x17; 8]);
            assert_eq!(h.read_owned(b, 8), [0x19; 8]);
            h.transition(
                &[a, b],
                ResourceStates::COPY_SOURCE,
                ResourceStates::COPY_DEST,
            );
        }
        h.bind(b, BACKING + 8);
        h.sync(&[a, b], import).unwrap();
        if import {
            h.transition(
                &[a, b],
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
            assert_eq!(h.read_owned(a, 8), [0x29; 8]);
            assert_eq!(h.read_owned(b, 8), [0x31; 8]);
        } else {
            assert_eq!(h.bytes(BACKING, 8), [0x17; 8]);
            assert_eq!(h.bytes(BACKING + 8, 8), [0x19; 8]);
        }
        // Rebinding one address space must never authorize another clone.
        h.memory = original;
        assert!(h.sync(&[a, b], false).is_err());
    }
}

#[test]
fn cpu_guest_sync_sixteen_distinct_roots_allowed_seventeenth_rejected() {
    let mut h = Harness::new();
    let mut roots = Vec::new();
    for i in 0..17 {
        let root = h.buffer(8, ResourceStates::COPY_DEST);
        h.upload(root, &[0x17; 8]);
        let address = BACKING + i * 8;
        h.map(address, &[0x29; 8], MemoryPermissions::READ_WRITE);
        h.bind(root, address);
        roots.push(root);
    }
    assert!(h.sync(&roots, true).is_err());
    h.transition(
        &roots,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    for root in &roots {
        assert_eq!(h.read_owned(*root, 8), [0x17; 8]);
    }
    h.transition(
        &roots,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_DEST,
    );
    h.sync(&roots[..16], true).unwrap();
    h.transition(
        &roots,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    for root in &roots[..16] {
        assert_eq!(h.read_owned(*root, 8), [0x29; 8]);
    }
    assert_eq!(h.read_owned(roots[16], 8), [0x17; 8]);
    for i in 0..17 {
        h.memory.write_bytes(BACKING + i * 8, &[0x39; 8]).unwrap();
    }
    assert!(h.sync(&roots, false).is_err());
    assert_eq!(h.bytes(BACKING, 136), [0x39; 136]);
    h.sync(&roots[..16], false).unwrap();
    assert_eq!(h.bytes(BACKING, 128), [0x29; 128]);
    assert_eq!(h.bytes(BACKING + 128, 8), [0x39; 8]);
}

#[test]
fn cpu_guest_sync_direction_specific_permissions_and_exact_span_guards() {
    let mut h = Harness::new();
    let root = h.buffer(8, ResourceStates::COPY_DEST);
    h.map(BACKING, &[0x29; 8], MemoryPermissions::READ_ONLY);
    h.bind(root, BACKING);
    h.sync(&[root], true).unwrap();
    h.map(
        BACKING + 0x100,
        &[0xa5; 16],
        MemoryPermissions {
            read: false,
            write: true,
            execute: false,
        },
    );
    h.bind(root, BACKING + 0x104);
    assert!(h.sync(&[root], true).is_err());
    h.transition(
        &[root],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    assert_eq!(h.read_owned(root, 8), [0x29; 8]);
    h.sync(&[root], false).unwrap();
    // Loader bypass is used only to inspect the write-only destination in this test.
    assert_eq!(
        h.memory
            .read_u64_bypass(BACKING + 0x100)
            .unwrap()
            .to_le_bytes(),
        [0xa5, 0xa5, 0xa5, 0xa5, 0x29, 0x29, 0x29, 0x29]
    );
    assert_eq!(
        h.memory
            .read_u64_bypass(BACKING + 0x108)
            .unwrap()
            .to_le_bytes(),
        [0x29, 0x29, 0x29, 0x29, 0xa5, 0xa5, 0xa5, 0xa5]
    );
}

#[test]
fn cpu_guest_sync_queued_copy_pins_roots_and_completion_does_not_export() {
    use xenora_loader::graphics::{
        GraphicsCopyInput, GraphicsError, GraphicsQueuePoint, GraphicsQueuedPendingWork,
        GraphicsQueuedWorkOutput, GraphicsQueuedWorkRequest,
    };
    struct Ticket(Option<GraphicsQueuedWorkOutput>);
    impl GraphicsQueuedPendingWork for Ticket {
        fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
            self.0.take().map(Ok)
        }
        fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
            self.poll().expect("controlled CPU completion")
        }
        fn reject_dependency(&mut self, _: GraphicsQueuePoint) {}
    }
    let mut h = Harness::new();
    let source = h.buffer(8, ResourceStates::COPY_DEST);
    let destination = h.buffer(8, ResourceStates::COPY_DEST);
    h.map(BACKING, &[0x17; 8], MemoryPermissions::READ_WRITE);
    h.map(BACKING + 8, &[0xa5; 8], MemoryPermissions::READ_WRITE);
    h.bind(source, BACKING);
    h.bind(destination, BACKING + 8);
    h.sync(&[source], true).unwrap();
    h.transition(
        &[source],
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    // The copy must use the explicit imported bytes, not the newly changed guest bytes.
    h.memory.write_bytes(BACKING, &[0x29; 8]).unwrap();
    h.ctx
        .set_metal_copy_queued_backend(Default::default(), |request| {
            let GraphicsQueuedWorkRequest::Copy(request) = request else {
                panic!("copy request")
            };
            let GraphicsCopyInput::Host(bytes) = request.source else {
                panic!("owned source")
            };
            assert_eq!(bytes.as_ref(), &[0x17; 8]);
            Ok(Box::new(Ticket(Some(GraphicsQueuedWorkOutput::Copy {
                layout: request.layout,
                bytes: vec![0x17; 8],
            }))))
        });
    let queue = h.ok(2, &[h.device]);
    let allocator = h.ok(3, &[h.device]);
    let list = h.ok(4, &[h.device, allocator]);
    h.ctx
        .cmd_copy_resource(
            GraphicsHandle::from_u64(list),
            GraphicsHandle::from_u64(destination),
            GraphicsHandle::from_u64(source),
        )
        .unwrap();
    h.ok(13, &[list, destination, 0x400, 0x800]);
    h.ok(12, &[list]);
    let receipt = h.ok(35, &[queue, list, 0, 0]);
    assert!(h.sync(&[source], false).is_err());
    assert!(h.sync(&[destination], true).is_err());
    for root in [source, destination] {
        assert!(
            h.ctx
                .bind_guest_memory(GraphicsHandle::from_u64(root), BACKING, &h.memory)
                .is_err()
        );
    }
    assert_eq!(h.ok(32, &[receipt]), 1);
    assert_eq!(h.bytes(BACKING + 8, 8), [0xa5; 8]);
    assert!(
        h.sync(&[destination], false).is_err(),
        "retained output still pinned"
    );
    assert!(
        h.ctx
            .bind_guest_memory(
                GraphicsHandle::from_u64(destination),
                BACKING + 8,
                &h.memory
            )
            .is_err()
    );
    h.ok(33, &[receipt]);
    h.sync(&[destination], false).unwrap();
    assert_eq!(h.bytes(BACKING + 8, 8), [0x17; 8]);
}

#[test]
fn cpu_guest_sync_nonqueued_async_pins_survive_terminal_until_release() {
    use xenora_loader::graphics::{
        GraphicsError, GraphicsPassOutput, GraphicsPendingSubmission, GraphicsSubmissionStatus,
    };

    // Poll stays Pending deterministically. Wait drains without sleeps or GPU work,
    // including on context destruction if an assertion fails before explicit Wait.
    struct Pending {
        fail: bool,
    }
    impl GraphicsPendingSubmission for Pending {
        fn poll(&mut self) -> Option<Result<GraphicsPassOutput, GraphicsError>> {
            None
        }
        fn wait(&mut self) -> Result<GraphicsPassOutput, GraphicsError> {
            if self.fail {
                Err(GraphicsError::BackendError(
                    "controlled async failure".into(),
                ))
            } else {
                Ok(GraphicsPassOutput {
                    color_layouts: None,
                    colors: vec![vec![17, 53, 127, 193]],
                    depth: None,
                })
            }
        }
    }

    for fail in [false, true] {
        let mut h = Harness::new();
        let importer = h.buffer(8, ResourceStates::COPY_DEST);
        let exporter = h.buffer(8, ResourceStates::COPY_DEST);
        let vertex = h.buffer(48, ResourceStates::COPY_DEST);
        h.upload(importer, &[0x17; 8]);
        h.upload(exporter, &[0x29; 8]);
        let vertices: [f32; 12] = [-1., -1., 0., 1., 1., -1., 0., 1., 0., 1., 0., 1.];
        let bytes: Vec<_> = vertices.iter().flat_map(|v| v.to_le_bytes()).collect();
        h.upload(vertex, &bytes);
        h.transition(
            &[exporter],
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        h.transition(
            &[vertex],
            ResourceStates::COPY_DEST,
            ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
        );
        let target = h.ok(
            6,
            &[
                h.device,
                1,
                1,
                1,
                u64::from(ResourceStates::RENDER_TARGET.0),
            ],
        );
        h.map(BACKING, &[0x31; 8], MemoryPermissions::READ_WRITE);
        h.map(BACKING + 8, &[0xa5; 8], MemoryPermissions::READ_WRITE);
        h.bind(importer, BACKING);
        h.bind(exporter, BACKING + 8);

        // Explicitly select the nonqueued async path under review.
        h.ctx
            .set_metal_async_backend(move |_| Ok(Box::new(Pending { fail })));
        let queue = h.ok(2, &[h.device]);
        let allocator = h.ok(3, &[h.device]);
        let list = h.ok(4, &[h.device, allocator]);
        let list_handle = GraphicsHandle::from_u64(list);
        h.ctx
            .cmd_set_render_targets(list_handle, Some(GraphicsHandle::from_u64(target)), [0; 4])
            .unwrap();
        h.ctx.cmd_set_viewport(list_handle, 1, 1).unwrap();
        h.ctx
            .cmd_set_vertex_buffer(list_handle, GraphicsHandle::from_u64(vertex), 0, 16)
            .unwrap();
        h.ctx.cmd_draw_instanced(list_handle, 3, 1, 0, 0).unwrap();
        // Both resources are referenced by the submission, with valid sync states
        // before AND after completion. A wrong-state error cannot mask missing pins.
        h.ok(13, &[list, importer, 0x400, 0x400]);
        h.ok(13, &[list, exporter, 0x800, 0x800]);
        h.ok(12, &[list]);
        let receipt = GraphicsHandle::from_u64(h.ok(30, &[queue, list]));
        h.ok(11, &[list, allocator]);
        assert_eq!(
            h.ctx.poll_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Pending
        );
        assert!(h.ctx.release_submission(receipt).is_err());

        for terminal in [false, true] {
            if terminal {
                let expected = if fail {
                    GraphicsSubmissionStatus::Failed
                } else {
                    GraphicsSubmissionStatus::Complete
                };
                assert_eq!(h.ctx.wait_submission(receipt).unwrap(), expected);
                assert_eq!(h.ctx.poll_submission(receipt).unwrap(), expected);
            }
            for (root, address) in [(importer, BACKING), (exporter, BACKING + 8)] {
                assert_eq!(
                    h.ctx
                        .bind_guest_memory(GraphicsHandle::from_u64(root), address, &h.memory),
                    Err(GraphicsError::ResourceLifetimeViolation),
                    "bind fail={fail} terminal={terminal}",
                );
            }
            assert_eq!(
                h.sync(&[importer], true),
                Err(GraphicsError::ResourceLifetimeViolation),
                "import fail={fail} terminal={terminal}"
            );
            assert_eq!(
                h.sync(&[exporter], false),
                Err(GraphicsError::ResourceLifetimeViolation),
                "export fail={fail} terminal={terminal}"
            );
            assert_eq!(h.bytes(BACKING, 8), [0x31; 8]);
            assert_eq!(h.bytes(BACKING + 8, 8), [0xa5; 8]);
        }

        h.ctx.release_submission(receipt).unwrap();
        // Release, not Wait, authorizes all three operations again.
        h.bind(importer, BACKING);
        h.bind(exporter, BACKING + 8);
        h.sync(&[importer], true).unwrap();
        h.sync(&[exporter], false).unwrap();
        assert_eq!(h.bytes(BACKING, 8), [0x31; 8]);
        assert_eq!(h.bytes(BACKING + 8, 8), [0x29; 8]);
    }
}
