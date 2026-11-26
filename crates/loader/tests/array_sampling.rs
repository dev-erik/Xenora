// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent runtime-array ABI and pre-dispatch validation; no GPU evidence.
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
struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
}
impl Harness {
    fn new() -> Self {
        let mut memory = GuestMemory::new();
        for (a, n) in [(DESC, 128), (OUT, 8), (DATA, 8192)] {
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
struct Scene {
    h: Harness,
    q: u64,
    a: u64,
    l: u64,
    root: u64,
    range: u64,
    selected: u64,
    pipeline: u64,
    plain: u64,
    vb: u64,
    cb: u64,
    target: u64,
    calls: Arc<AtomicUsize>,
    expected_draws: Arc<AtomicUsize>,
}
impl Scene {
    fn new() -> Self {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let q = h.ok(2, &[d]);
        let a = h.ok(3, &[d]);
        let l = h.ok(4, &[d, a]);
        let root = h.ok(51, &[d, 5, 3, 3, 5, 0x400]);
        h.upload(root, &[17; 360]);
        let parent = h.ok(57, &[d, root, 1, 4, 0, 3, 0]);
        let range = h.ok(58, &[d, parent, 1, 3, 1, 2, 0]);
        h.ok(60, &[parent]);
        let selected = h.ok(59, &[d, range, 0, 0, 1, 0]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let array = include_bytes!("../../../fixtures/rdna2/x9-array-uniform-ps.bin");
        let plain = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
        h.memory.write_bytes(DATA, vs).unwrap();
        h.memory.write_bytes(DATA + 1024, array).unwrap();
        let pipeline = h.ok(
            55,
            &[
                d,
                DATA,
                vs.len() as u64,
                DATA + 1024,
                array.len() as u64,
                0x10,
            ],
        );
        h.memory.write_bytes(DATA + 1024, plain).unwrap();
        let plain = h.ok(
            55,
            &[d, DATA, vs.len() as u64, DATA + 1024, plain.len() as u64, 0],
        );
        let vb = h.ok(5, &[d, 48, 0x400]);
        let cb = h.ok(5, &[d, 48, 0x400]);
        h.upload(
            vb,
            &[
                [-1f32, -1., 0.5, 1.],
                [3., -1., 0.5, 1.],
                [-1., 3., 0.5, 1.],
            ]
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        );
        h.upload(
            cb,
            &[[0.25f32, 0.5, 0., 1.]; 3]
                .into_iter()
                .flatten()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let target = h.ok(6, &[d, 2, 1, 1, 4]);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let expected_draws = Arc::new(AtomicUsize::new(2));
        let draws = expected_draws.clone();
        h.ctx.set_metal_array_view_queued_backend(
            GraphicsTextureCapabilities::default(),
            move |work| {
                let GraphicsQueuedWorkRequest::Render(r) = work else {
                    panic!("render")
                };
                seen.fetch_add(1, Ordering::SeqCst);
                assert_eq!(r.draws.len(), draws.load(Ordering::SeqCst));
                for draw in &r.draws {
                    let range = draw.sampled_ranges[0].unwrap();
                    assert_eq!(
                        (
                            range.base_slice,
                            range.slice_count,
                            range.base_mip,
                            range.mip_count
                        ),
                        (2, 3, 1, 2)
                    );
                    assert!(draw.views[0].is_none());
                    assert!(draw.array_views[0].is_none());
                    let Some(GraphicsTextureInput::Host(snapshot)) = &draw.textures[0] else {
                        panic!("full host snapshot")
                    };
                    assert_eq!(snapshot.pixels.len(), 360);
                    assert_eq!(snapshot.array_layers, Some(5));
                }
                Ok(Box::new(Ticket(Some(GraphicsQueuedWorkOutput::Render(
                    GraphicsPassOutput {
                        color_layouts: None,
                        colors: vec![vec![11, 23, 47, 255, 59, 71, 83, 255]],
                        depth: None,
                    },
                )))))
            },
        );
        h.ctx.enable_shader_array_views().unwrap();
        Self {
            h,
            q,
            a,
            l,
            root,
            range,
            selected,
            pipeline,
            plain,
            vb,
            cb,
            target,
            calls,
            expected_draws,
        }
    }
    fn table(&mut self, range: u64, second: u64) -> u64 {
        let words = [range, second, 1, 0, 0, 0, 0, 0, 0, 0];
        self.h
            .memory
            .write_bytes(
                DATA + 4096,
                &words
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        DATA + 4096
    }
    fn record(&mut self, bits: u32, present: bool, array_pipeline: bool, array_binding: bool) {
        self.h.ok(13, &[self.l, self.root, 0x400, 0x80]);
        self.h.ok(13, &[self.l, self.vb, 0x400, 1]);
        self.h.ok(13, &[self.l, self.cb, 0x400, 1]);
        self.h
            .memory
            .write_bytes(
                DATA + 2048,
                &[self.target, 0]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        self.h.ok(15, &[self.l, 1, DATA + 2048]);
        self.h.ok(17, &[self.l, 2, 1]);
        self.h.ok(
            14,
            &[
                self.l,
                if array_pipeline {
                    self.pipeline
                } else {
                    self.plain
                },
            ],
        );
        self.h.ok(18, &[self.l, self.vb, 0, 16]);
        self.h.ok(19, &[self.l, self.cb, 0, 16]);
        let address = self.table(
            if array_binding {
                self.range
            } else {
                self.selected
            },
            0,
        );
        self.h
            .ok(if array_binding { 66 } else { 39 }, &[self.l, address]);
        for word in [0, bits] {
            self.h
                .memory
                .write_bytes(
                    DATA + 3072,
                    &[word, 0, 0, 0, 0, 0, 0, 0]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            self.h.ok(
                41,
                &[
                    self.l,
                    if present { DATA + 3072 } else { 0 },
                    u64::from(present),
                ],
            );
            self.h.ok(23, &[self.l, 3, 1, 0, 0]);
        }
    }
    fn close(&mut self) {
        self.h.ok(12, &[self.l]);
    }
    fn reject(&mut self) {
        assert_eq!(self.h.call(35, &[self.q, self.l, 0, 0]), 2);
        assert_eq!(self.calls.load(Ordering::SeqCst), 0);
        assert_eq!(self.h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(self.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
        self.h.upload(self.root, &[17; 4]); // No staged transition committed.
    }
}
#[test]
fn cpu_array_sampling_uniform_layer_domain_late_invalid_is_atomic() {
    for bits in [
        0.5f32.to_bits(),
        (-1f32).to_bits(),
        3f32.to_bits(),
        f32::NAN.to_bits(),
        f32::INFINITY.to_bits(),
        f32::NEG_INFINITY.to_bits(),
        1,
    ] {
        let mut s = Scene::new();
        s.record(bits, true, true, true);
        s.close();
        s.reject();
        s.h.ok(11, &[s.l, s.a]);
        s.record(2f32.to_bits(), true, true, true);
        s.close();
        let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
        assert_eq!(s.h.ok(32, &[receipt]), 1);
        s.h.ok(33, &[receipt]);
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    }
    for bits in [0, (-0f32).to_bits(), 1f32.to_bits(), 2f32.to_bits()] {
        let mut s = Scene::new();
        s.record(bits, true, true, true);
        s.close();
        let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
        assert_eq!(s.h.ok(32, &[receipt]), 1);
        s.h.ok(33, &[receipt]);
    }
    let mut s = Scene::new();
    s.record(0, false, true, true);
    s.close();
    s.reject();
}
#[test]
fn cpu_array_sampling_binding_dimension_and_capability_downgrade_reject() {
    for (pipeline, binding) in [(true, false), (false, true)] {
        let mut s = Scene::new();
        s.record(0, true, pipeline, binding);
        s.close();
        s.reject();
    }
    let mut s = Scene::new();
    s.record(0, true, true, true);
    s.close();
    s.h.ctx
        .set_metal_array_view_queued_backend(GraphicsTextureCapabilities::default(), |_| {
            panic!("downgraded dispatch")
        });
    s.reject();
}
#[test]
fn cpu_array_sampling_invalid_second_binding_and_result_span_preserve_previous_table() {
    let mut s = Scene::new();
    s.record(0, true, true, true);
    let invalid = s.table(s.range, u64::MAX);
    assert_eq!(s.h.call(66, &[s.l, invalid]), 2);
    let valid = s.table(s.range, 0);
    s.h.prepare(&[s.l, valid]);
    s.h.memory
        .write_bytes(DESC + 8, &66u64.to_le_bytes())
        .unwrap();
    assert!(graphics_call_v1(&mut s.h.ctx, &mut s.h.memory, DESC, 128, OUT + 4).is_err());
    assert_eq!(s.h.result(), SENTINEL);
    let old = s.table(s.range, 0);
    assert_eq!(s.h.call(39, &[s.l, old]), 2);
    // A later draw must still snapshot the original valid complete table.
    s.expected_draws.store(3, Ordering::SeqCst);
    s.h.ok(23, &[s.l, 3, 1, 0, 0]);
    s.close();
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn cpu_array_sampling_branch_layer_union_validates_unselected_high_uniform() {
    let mut s = Scene::new();
    let d = s.h.ok(1, &[]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-array-branch-ps.bin");
    s.h.memory.write_bytes(DATA, vs).unwrap();
    s.h.memory.write_bytes(DATA + 1024, ps).unwrap();
    let branch = s.h.ok(
        55,
        &[d, DATA, vs.len() as u64, DATA + 1024, ps.len() as u64, 0x10],
    );
    let calls = s.calls.clone();
    s.h.ctx.set_metal_array_view_queued_backend(
        GraphicsTextureCapabilities::default(),
        move |work| {
            let GraphicsQueuedWorkRequest::Render(r) = work else {
                panic!("render")
            };
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(r.draws.len(), 3);
            let draw = &r.draws[2];
            let pipeline = draw.pipeline.as_ref().unwrap();
            // s24 and s31 supply layers; s25 controls the branch. All are required.
            assert_eq!(pipeline.required_uniform_mask, 0x83);
            let layers = &pipeline.array_layers[0];
            assert_eq!(layers.len(), 2);
            assert!(layers.contains(&xenora_shader::GraphicsArrayLayer::Uniform(0)));
            assert!(layers.contains(&xenora_shader::GraphicsArrayLayer::Uniform(7)));
            assert!(pipeline.array_layers[1].is_empty());
            assert_eq!(
                draw.uniforms.unwrap(),
                [0, 0, 0, 0, 0, 0, 0, 2f32.to_bits()]
            );
            for draw in &r.draws {
                let range = draw.sampled_ranges[0].unwrap();
                assert_eq!(
                    (
                        range.base_slice,
                        range.slice_count,
                        range.base_mip,
                        range.mip_count
                    ),
                    (2, 3, 1, 2)
                );
                assert!(draw.sampled_ranges[1].is_none());
            }
            Ok(Box::new(Ticket(Some(GraphicsQueuedWorkOutput::Render(
                GraphicsPassOutput {
                    color_layouts: None,
                    colors: vec![vec![11, 23, 47, 255, 59, 71, 83, 255]],
                    depth: None,
                },
            )))))
        },
    );
    s.h.ctx.enable_shader_array_views().unwrap();
    for invalid in [true, false] {
        s.record(0, true, true, true); // Two valid draws establish the binding table.
        s.h.ok(14, &[s.l, branch]);
        let words = [
            0u32,
            0,
            0,
            0,
            0,
            0,
            0,
            if invalid { 0x7fc1_2345 } else { 2f32.to_bits() },
        ];
        s.h.memory
            .write_bytes(
                DATA + 3072,
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        s.h.ok(41, &[s.l, DATA + 3072, 1]);
        s.h.ok(23, &[s.l, 3, 1, 0, 0]);
        s.close();
        if invalid {
            // s25==0 selects valid s24; invalid s31 must nevertheless reject.
            s.reject();
            assert_eq!(
                s.h.call(60, &[s.range]),
                2,
                "rejection must not discard recorded range binding"
            );
            s.h.ok(11, &[s.l, s.a]);
        } else {
            let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
            assert_eq!(s.h.ok(32, &[receipt]), 1);
            s.h.ok(33, &[receipt]);
            assert_eq!(s.calls.load(Ordering::SeqCst), 1);
        }
    }
}
