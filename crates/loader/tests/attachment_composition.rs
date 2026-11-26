// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent semantic-versus-composition edge tests; mock output is not GPU evidence.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use xenora_loader::graphics::*;
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{GuestMemory, MemoryPermissions};
const DESC: u64 = 0x1000;
const OUT: u64 = 0x2000;
const DATA: u64 = 0x3000;
const SENTINEL: u64 = 0x9182736455463728;
struct Ticket(Option<GraphicsQueuedWorkOutput>);
impl GraphicsQueuedPendingWork for Ticket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.take().map(Ok)
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().unwrap()
    }
    fn reject_dependency(&mut self, _: GraphicsQueuePoint) {}
}
struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
}
impl Harness {
    fn new() -> Self {
        let mut memory = GuestMemory::new();
        for (a, n) in [(DESC, 128), (OUT, 8), (DATA, 4096)] {
            memory
                .map_region(a, n, MemoryPermissions::READ_WRITE)
                .unwrap();
        }
        Self {
            ctx: GraphicsContext::new(),
            memory,
        }
    }
    fn prepare(&mut self, args: &[u64]) {
        let mut words = [0u64; 16];
        words[0] = 1;
        words[1] = 50;
        words[2..2 + args.len()].copy_from_slice(args);
        self.memory
            .write_bytes(
                DESC,
                &words
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        self.memory
            .write_bytes(OUT, &SENTINEL.to_le_bytes())
            .unwrap();
    }
    fn call(&mut self, op: u64, args: &[u64]) -> u64 {
        self.prepare(args);
        self.memory
            .write_bytes(DESC + 8, &op.to_le_bytes())
            .unwrap();
        graphics_call_v1(&mut self.ctx, &mut self.memory, DESC, 128, OUT).unwrap()
    }
    fn result(&self) -> u64 {
        self.memory.read_u64(OUT).unwrap()
    }
    fn ok(&mut self, op: u64, args: &[u64]) -> u64 {
        assert_eq!(self.call(op, args), 0, "opcode {op}");
        self.result()
    }
    fn upload(&mut self, r: u64, b: &[u8]) {
        self.memory.write_bytes(DATA, b).unwrap();
        self.ok(9, &[r, 0, DATA, b.len() as u64]);
    }
}

fn bind(h: &mut Harness, l: u64, views: &[u64], load: bool) {
    let mut words = Vec::new();
    for (i, v) in views.iter().enumerate() {
        words.extend_from_slice(&[
            *v,
            u64::from(load),
            0,
            0,
            if i == 0 { 1f64.to_bits() } else { 0 },
            if i == 1 { 1f64.to_bits() } else { 0 },
            0,
            1f64.to_bits(),
        ]);
    }
    h.memory
        .write_bytes(
            DATA,
            &words
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    h.ok(63, &[l, views.len() as u64, DATA]);
}
fn enable(h: &mut Harness) {
    h.ctx
        .enable_resource_roles(GraphicsResourceRoleCapabilities {
            sampled_formats: 0x3ff,
            color_formats: 0x1f9,
        })
        .unwrap();
    h.ctx.enable_attachment_transactions().unwrap();
}

fn wait_table(h: &mut Harness, address: u64, queue: u64, receipt: u64) {
    let value = h.ok(36, &[receipt]);
    h.memory.write_bytes(address, &queue.to_le_bytes()).unwrap();
    h.memory
        .write_bytes(address + 8, &value.to_le_bytes())
        .unwrap();
}

#[test]
fn cpu_attachment_composition_exact_semantic_prior_disjoint_base_and_collection_orders() {
    for ancestor_first in [false, true] {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let queues = [h.ok(2, &[d]), h.ok(2, &[d])];
        let root = h.ok(53, &[d, 2, 1, 1, 2, 0, 0x400]);
        h.upload(root, &[17; 16]);
        let views = [h.ok(61, &[d, root, 0, 0, 0]), h.ok(61, &[d, root, 0, 1, 0])];
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let versions = Arc::new(std::sync::Mutex::new(Vec::<GraphicsProducedVersion>::new()));
        let seen = versions.clone();
        h.ctx.set_metal_typed_copy_queued_backend(
            GraphicsTextureCapabilities::default(),
            GraphicsTypedCopyCapabilities { formats: 0x3ff },
            move |work| {
                if let GraphicsQueuedWorkRequest::Render(r) = work {
                    assert_eq!(counter.load(Ordering::SeqCst), 3);
                    assert!(r.draws.is_empty());
                    assert_eq!(r.extent, (4, 3));
                    assert!(r.depth.is_none());
                    return Ok(Box::new(Ticket(Some(GraphicsQueuedWorkOutput::Render(
                        GraphicsPassOutput {
                            color_layouts: None,
                            colors: vec![[0, 0, 0, 255].repeat(12)],
                            depth: None,
                        },
                    )))));
                }
                let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                    panic!("transaction")
                };
                let n = counter.fetch_add(1, Ordering::SeqCst);
                let mut versions = seen.lock().unwrap();
                assert_eq!(r.roots.len(), 1);
                let root = &r.roots[0];
                if n == 0 {
                    let GraphicsAttachmentRootInput::Host(bytes) = &root.prior else {
                        panic!("host A")
                    };
                    assert_eq!(bytes.as_ref(), &[17; 16]);
                    assert!(root.destination_base.is_none());
                } else {
                    let GraphicsAttachmentRootInput::Produced(prior) = &root.prior else {
                        panic!("exact produced A")
                    };
                    assert_eq!(
                        *prior, versions[0],
                        "composition must not replace semantic A"
                    );
                    if n == 2 {
                        assert_eq!(root.destination_base, Some(versions[1]));
                        assert_eq!(r.colors.len(), 1);
                        assert_eq!(r.colors[0].slice, 1);
                        assert!(r.colors[0].load);
                    }
                }
                // Literal full roots: A red/green; B and C blue/green.
                // C's empty Load preserves A's selected green slice and B's blue slice.
                let bytes = if n == 0 {
                    vec![
                        255, 0, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
                    ]
                } else {
                    vec![
                        0, 0, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255, 0, 255, 0, 255,
                    ]
                };
                versions.push(root.output);
                Ok(Box::new(Ticket(Some(
                    GraphicsQueuedWorkOutput::AttachmentTransaction(vec![
                        GraphicsAttachmentRootResult {
                            output: root.output,
                            desc: root.desc,
                            bytes,
                        },
                    ]),
                ))))
            },
        );
        enable(&mut h);
        let mut lists = Vec::new();
        let mut allocators = Vec::new();
        for _ in 0..4 {
            let a = h.ok(3, &[d]);
            allocators.push(a);
            lists.push(h.ok(4, &[d, a]));
        }
        h.ok(13, &[lists[0], root, 0x400, 4]);
        bind(&mut h, lists[0], &views, false);
        h.ok(17, &[lists[0], 2, 1]);
        h.ok(12, &[lists[0]]);
        bind(&mut h, lists[1], &views[..1], false);
        h.ok(17, &[lists[1], 2, 1]);
        h.ok(12, &[lists[1]]);
        for (list, view) in [(lists[2], views[1]), (lists[3], views[0])] {
            bind(&mut h, list, &[view], true);
            h.ok(17, &[list, 2, 1]);
            h.ok(12, &[list]);
        }
        let a = h.ok(35, &[queues[0], lists[0], 0, 0]);
        let semantic = DATA + 1024;
        let composition = DATA + 1040;
        wait_table(&mut h, semantic, queues[0], a);
        let b = h.ok(35, &[queues[1], lists[1], semantic, 1]);
        wait_table(&mut h, composition, queues[1], b);
        // Neither omitted composition nor overlapping Load may dispatch C.
        assert_eq!(h.call(65, &[queues[1], lists[2], semantic, 1, 0, 0]), 2);
        assert_eq!(
            h.call(65, &[queues[1], lists[3], semantic, 1, composition, 1]),
            2
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
        let c = h.ok(65, &[queues[1], lists[2], semantic, 1, composition, 1]);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        for (list, allocator) in lists.iter().zip(allocators.iter()) {
            h.ok(11, &[*list, *allocator]);
        }
        for view in views {
            h.ok(62, &[view]);
        }
        for receipt in if ancestor_first { [a, b, c] } else { [c, b, a] } {
            assert_eq!(h.ok(32, &[receipt]), 1);
        }
        for receipt in [a, b, c] {
            h.ok(33, &[receipt]);
        }
        h.ok(13, &[lists[0], root, 4, 0x800]);
        h.ok(12, &[lists[0]]);
        h.ok(25, &[queues[0], lists[0]]);
        h.ok(10, &[root, 0, DATA, 16]);
        let mut actual = [0; 16];
        h.memory.read_bytes(DATA, &mut actual).unwrap();
        assert_eq!(
            actual,
            [
                0, 0, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255, 0, 255, 0, 255
            ]
        );
        h.ok(11, &[lists[0], allocators[0]]);
        h.ok(26, &[root]);
    }
}
