// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent composition lineage/publication tests; controlled CPU backend only.
use std::sync::{Arc, Mutex};
use xenora_loader::graphics::*;
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{GuestMemory, MemoryPermissions};
const DESC: u64 = 0x1000;
const OUT: u64 = 0x2000;
const DATA: u64 = 0x3000;
const SENTINEL: u64 = 0x9182736455463728;
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

struct Completion {
    ready: bool,
    result: Option<Result<GraphicsQueuedWorkOutput, GraphicsError>>,
    rejected: bool,
    taken: bool,
}
struct ControlledTicket(Arc<Mutex<Completion>>);
impl GraphicsQueuedPendingWork for ControlledTicket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        let mut state = self.0.lock().unwrap();
        if !state.ready {
            return None;
        }
        state.taken = true;
        state.result.take()
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll()
            .expect("test must explicitly release completion")
    }
    fn reject_dependency(&mut self, _: GraphicsQueuePoint) {
        // Deliberately keep a cached successful result: loader must suppress it.
        self.0.lock().unwrap().rejected = true;
    }
}
#[derive(Default)]
struct Observed {
    requests: Vec<GraphicsAttachmentTransactionRequest>,
    completions: Vec<Arc<Mutex<Completion>>>,
}
struct Scene {
    h: Harness,
    d: u64,
    queues: [u64; 2],
    root: u64,
    views: [u64; 4], // slice0 mip0/mip1, slice1 mip0/mip1
    observed: Arc<Mutex<Observed>>,
}
impl Scene {
    fn new() -> Self {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let queues = [h.ok(2, &[d]), h.ok(2, &[d])];
        let root = h.ok(53, &[d, 4, 2, 2, 2, 0, 0x400]);
        h.upload(root, &[17; 80]);
        let views = [
            h.ok(61, &[d, root, 0, 0, 0]),
            h.ok(61, &[d, root, 1, 0, 0]),
            h.ok(61, &[d, root, 0, 1, 0]),
            h.ok(61, &[d, root, 1, 1, 0]),
        ];
        let observed = Arc::new(Mutex::new(Observed::default()));
        let backend = observed.clone();
        h.ctx.set_metal_typed_copy_queued_backend(
            GraphicsTextureCapabilities::default(),
            GraphicsTypedCopyCapabilities { formats: 0x3ff },
            move |work| {
                let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                    panic!("transaction only")
                };
                let mut seen = backend.lock().unwrap();
                let byte = 31 + seen.requests.len() as u8;
                let output = GraphicsQueuedWorkOutput::AttachmentTransaction(
                    r.roots
                        .iter()
                        .map(|root| GraphicsAttachmentRootResult {
                            output: root.output,
                            desc: root.desc,
                            bytes: vec![byte; 80],
                        })
                        .collect(),
                );
                let state = Arc::new(Mutex::new(Completion {
                    ready: true,
                    result: Some(Ok(output)),
                    rejected: false,
                    taken: false,
                }));
                seen.requests.push(r);
                seen.completions.push(state.clone());
                Ok(Box::new(ControlledTicket(state)))
            },
        );
        enable(&mut h);
        Self {
            h,
            d,
            queues,
            root,
            views,
            observed,
        }
    }
    fn list(&mut self, view: usize, initial: bool, load: bool) -> u64 {
        let a = self.h.ok(3, &[self.d]);
        let l = self.h.ok(4, &[self.d, a]);
        if initial {
            self.h.ok(13, &[l, self.root, 0x400, 4]);
        }
        bind(&mut self.h, l, &[self.views[view]], load);
        let (w, h) = if view.is_multiple_of(2) {
            (4, 2)
        } else {
            (2, 1)
        };
        self.h.ok(17, &[l, w, h]);
        self.h.ok(12, &[l]);
        l
    }
    fn table(&mut self, slot: u64, queue: usize, receipt: u64) -> u64 {
        let point = self.h.ok(36, &[receipt]);
        let address = DATA + 1024 + slot * 16;
        self.h
            .memory
            .write_bytes(address, &self.queues[queue].to_le_bytes())
            .unwrap();
        self.h
            .memory
            .write_bytes(address + 8, &point.to_le_bytes())
            .unwrap();
        address
    }
    fn seed(&mut self) -> (u64, u64) {
        let list = self.list(3, true, false);
        let a = self.h.ok(35, &[self.queues[0], list, 0, 0]);
        let sem = self.table(0, 0, a);
        (a, sem)
    }
    fn calls(&self) -> usize {
        self.observed.lock().unwrap().requests.len()
    }
    fn read_published(&mut self) -> [u8; 80] {
        // All receipts are released first; switch solely for CPU barrier-only readback.
        self.h
            .ctx
            .set_metal_backend(|_, _, _| panic!("barrier must not render"));
        let a = self.h.ok(3, &[self.d]);
        let l = self.h.ok(4, &[self.d, a]);
        self.h.ok(13, &[l, self.root, 4, 0x800]);
        self.h.ok(12, &[l]);
        self.h.ok(25, &[self.queues[0], l]);
        self.h.ok(10, &[self.root, 0, DATA, 80]);
        let mut bytes = [0; 80];
        self.h.memory.read_bytes(DATA, &mut bytes).unwrap();
        bytes
    }
}

#[test]
fn cpu_attachment_composition_transitive_overlap_cannot_hide_behind_disjoint_tip() {
    let mut s = Scene::new();
    let (_, sem) = s.seed();
    let overlap = s.list(3, false, false);
    let b = s.h.ok(35, &[s.queues[1], overlap, sem, 1]);
    let bwait = s.table(1, 1, b);
    let disjoint = s.list(0, false, false);
    let d = s.h.ok(35, &[s.queues[1], disjoint, bwait, 1]);
    let tip = s.table(2, 1, d);
    let c = s.list(3, false, true);
    let charge = s.h.ctx.queued_copy_allocation_bytes();
    assert_eq!(s.h.call(65, &[s.queues[1], c, sem, 1, tip, 1]), 2);
    assert_eq!(
        s.calls(),
        3,
        "overlap in intermediate B must reject before backend"
    );
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), charge);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_attachment_composition_adjacent_mips_accept_exact_ancestor_load() {
    let mut s = Scene::new();
    let (a, sem) = s.seed();
    let bl = s.list(2, false, false); // same slice1, different mip0
    let b = s.h.ok(35, &[s.queues[1], bl, sem, 1]);
    let comp = s.table(1, 1, b);
    let cl = s.list(3, false, true); // slice1 mip1
    let c = s.h.ok(65, &[s.queues[1], cl, sem, 1, comp, 1]);
    {
        let seen = s.observed.lock().unwrap();
        let r = &seen.requests[2];
        let GraphicsAttachmentRootInput::Produced(prior) = &r.roots[0].prior else {
            panic!("produced")
        };
        assert_eq!(*prior, seen.requests[0].roots[0].output);
        assert_eq!(
            r.roots[0].destination_base,
            Some(seen.requests[1].roots[0].output)
        );
        assert_eq!((r.colors[0].slice, r.colors[0].mip), (1, 1));
    }
    for receipt in [c, b, a] {
        assert_eq!(s.h.ok(32, &[receipt]), 1);
    }
    for receipt in [a, b, c] {
        s.h.ok(33, &[receipt]);
    }
    assert_eq!(s.read_published(), [33; 80]);
}

fn published_without_composition(release_b: bool) {
    let mut s = Scene::new();
    let (a, sem) = s.seed();
    let bl = s.list(0, false, false);
    let b = s.h.ok(35, &[s.queues[1], bl, sem, 1]);
    assert_eq!(s.h.ok(32, &[b]), 1); // A and B published, A remains retained.
    if release_b {
        s.h.ok(33, &[b]);
    }
    let charge = s.h.ctx.queued_copy_allocation_bytes();
    let cl = s.list(3, false, true);
    assert_eq!(s.h.call(65, &[s.queues[1], cl, sem, 1, 0, 0]), 2);
    assert_eq!(s.calls(), 2, "published B cannot be overwritten from old A");
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), charge);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 2);
    if !release_b {
        s.h.ok(33, &[b]);
    }
    s.h.ok(33, &[a]);
    assert_eq!(
        s.read_published(),
        [32; 80],
        "B publication survives rejected C"
    );
}
#[test]
fn cpu_attachment_composition_published_released_base_cannot_be_silently_lost() {
    published_without_composition(true);
}
#[test]
fn cpu_attachment_composition_published_retained_base_still_requires_authorization() {
    published_without_composition(false);
}

#[test]
fn cpu_attachment_composition_failed_base_suppresses_cached_successful_consumer() {
    let mut s = Scene::new();
    let (a, sem) = s.seed();
    let bl = s.list(0, false, false);
    let b = s.h.ok(35, &[s.queues[1], bl, sem, 1]);
    let comp = s.table(1, 1, b);
    let cl = s.list(3, false, true);
    let c = s.h.ok(65, &[s.queues[1], cl, sem, 1, comp, 1]);
    let states = s.observed.lock().unwrap().completions.clone();
    states[1].lock().unwrap().ready = false;
    // C's backend success already exists, while B has not completed.
    assert!(states[2].lock().unwrap().result.as_ref().unwrap().is_ok());
    assert_eq!(s.h.ok(31, &[c]), 0);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 1); // A only
    {
        let mut b = states[1].lock().unwrap();
        b.result = Some(Err(GraphicsError::BackendError(
            "controlled composition failure".into(),
        )));
        b.ready = true;
    }
    assert_eq!(s.h.ok(32, &[c]), 2);
    assert!(states[2].lock().unwrap().rejected);
    assert!(
        states[2].lock().unwrap().taken,
        "consume cached success after rejecting dependency"
    );
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 1);
    for receipt in [a, b, c] {
        s.h.ok(33, &[receipt]);
    }
    assert_eq!(
        s.read_published(),
        [31; 80],
        "neither failed B nor cached C may publish"
    );
}

#[test]
fn cpu_attachment_composition_unrelated_root_receipt_is_not_destination_lineage() {
    let mut s = Scene::new();
    let (a, sem) = s.seed();
    let bl = s.list(0, false, false);
    let b = s.h.ok(35, &[s.queues[1], bl, sem, 1]);
    let correct = s.table(1, 1, b);
    let unrelated = s.h.ok(53, &[s.d, 4, 2, 2, 2, 0, 4]);
    let view = s.h.ok(61, &[s.d, unrelated, 1, 1, 0]);
    let allocator = s.h.ok(3, &[s.d]);
    let list = s.h.ok(4, &[s.d, allocator]);
    bind(&mut s.h, list, &[view], false);
    s.h.ok(17, &[list, 2, 1]);
    s.h.ok(12, &[list]);
    let other = s.h.ok(35, &[s.queues[0], list, sem, 1]);
    let wrong = s.table(2, 0, other);
    let cl = s.list(3, false, true);
    let charge = s.h.ctx.queued_copy_allocation_bytes();
    assert_eq!(s.h.call(65, &[s.queues[1], cl, sem, 1, wrong, 1]), 2);
    assert_eq!(s.calls(), 3);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), charge);
    // Rejection does not consume the recording or poison valid lineage.
    let c = s.h.ok(65, &[s.queues[1], cl, sem, 1, correct, 1]);
    {
        let seen = s.observed.lock().unwrap();
        assert_eq!(
            seen.requests[3].roots[0].destination_base,
            Some(seen.requests[1].roots[0].output)
        );
        assert_ne!(
            seen.requests[3].roots[0].destination_base,
            Some(seen.requests[2].roots[0].output)
        );
    }
    for receipt in [c, other, b, a] {
        assert_eq!(s.h.ok(32, &[receipt]), 1);
    }
    for receipt in [a, b, other, c] {
        s.h.ok(33, &[receipt]);
    }
    assert_eq!(s.read_published(), [34; 80]);
}

#[test]
fn cpu_attachment_composition_retired_uploaded_ancestor_does_not_block_unrelated_queue_work() {
    let mut s = Scene::new(); // Uploads 80 bytes to R: its mutation epoch is nonzero.
    let allocator = s.h.ok(3, &[s.d]);
    let list = s.h.ok(4, &[s.d, allocator]);
    s.h.ok(13, &[list, s.root, 0x400, 4]);
    bind(&mut s.h, list, &[s.views[3]], false);
    s.h.ok(17, &[list, 2, 1]);
    s.h.ok(12, &[list]);
    let a = s.h.ok(35, &[s.queues[0], list, 0, 0]);
    let a_point = s.observed.lock().unwrap().requests[0].point;
    s.h.ok(11, &[list, allocator]);
    for view in s.views {
        s.h.ok(62, &[view]);
    }
    s.h.ok(26, &[s.root]); // Native/receipt ownership remains; public R is absent.
    assert_eq!(s.h.call(26, &[s.root]), 2);

    let unrelated = s.h.ok(53, &[s.d, 4, 2, 2, 2, 0, 4]);
    let view = s.h.ok(61, &[s.d, unrelated, 1, 1, 0]);
    bind(&mut s.h, list, &[view], false);
    s.h.ok(17, &[list, 2, 1]);
    s.h.ok(12, &[list]);
    // No explicit waits: ordinary35 must preserve its implicit queue ordering.
    // R's absent public epoch must not be substituted with zero and compared to A.
    let b = s.h.ok(35, &[s.queues[0], list, 0, 0]);
    {
        let seen = s.observed.lock().unwrap();
        assert_eq!(seen.requests.len(), 2);
        assert!(seen.requests[1].waits.contains(&a_point));
        let GraphicsAttachmentRootInput::Host(bytes) = &seen.requests[1].roots[0].prior else {
            panic!("unrelated S must use its own host root")
        };
        assert_eq!(bytes.as_ref(), &[0; 80]);
    }
    assert_eq!(s.h.ok(32, &[b]), 1);
    assert_eq!(s.h.ok(32, &[a]), 1);
    s.h.ok(33, &[a]);
    s.h.ok(33, &[b]);
    s.root = unrelated;
    assert_eq!(s.read_published(), [32; 80]);
}
