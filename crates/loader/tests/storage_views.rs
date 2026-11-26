// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent storage service/transaction tests; mock output is not GPU evidence.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use xenora_loader::graphics::*;
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{GuestMemory, MemoryPermissions};
use xenora_texture::TextureFormat;
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

fn prior() -> Vec<u8> {
    let mut bytes = vec![0x59; 144];
    bytes[132..140].copy_from_slice(&[17, 53, 127, 193, 29, 71, 149, 211]);
    bytes
}
struct Scene {
    h: Harness,
    d: u64,
    q: u64,
    a: u64,
    l: u64,
    root: u64,
    view: u64,
    color: u64,
    cv: u64,
    pipeline: u64,
    vb: u64,
    calls: Arc<AtomicUsize>,
    bad: Arc<std::sync::atomic::AtomicBool>,
}
impl Scene {
    fn new() -> Self {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let q = h.ok(2, &[d]);
        let a = h.ok(3, &[d]);
        let l = h.ok(4, &[d, a]);
        let root = h.ok(53, &[d, 5, 3, 3, 2, 0, 0x400]);
        h.upload(root, &prior());
        let view = h.ok(67, &[d, root, 1, 1]);
        let color = h.ok(6, &[d, 2, 1, 1, 4]);
        let cv = h.ok(61, &[d, color, 0, 0, 0]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-storage-branch-ps.bin");
        h.memory.write_bytes(DATA, vs).unwrap();
        h.memory.write_bytes(DATA + 1024, ps).unwrap();
        let pipeline = h.ok(
            55,
            &[d, DATA, vs.len() as u64, DATA + 1024, ps.len() as u64, 0x40],
        );
        let vb = h.ok(5, &[d, 96, 0x400]);
        let tri = [
            [-1f32, -1., 0.5, 1.],
            [3., -1., 0.5, 1.],
            [-1., 3., 0.5, 1.],
        ];
        h.upload(
            vb,
            &tri.into_iter()
                .chain(tri)
                .flatten()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let bad = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fail = bad.clone();
        h.ctx.set_metal_typed_copy_queued_backend(
            GraphicsTextureCapabilities::default(),
            GraphicsTypedCopyCapabilities { formats: 0x3ff },
            move |work| {
                let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                    panic!("storage transaction")
                };
                count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(r.draws.len(), 1);
                assert_eq!(r.extent, (2, 1));
                assert!(r.depth.is_none());
                let storage = r.storage.as_ref().unwrap();
                assert_eq!((storage.root_index, storage.mip, storage.slice), (1, 1, 1));
                assert_eq!(r.roots.len(), 2);
                assert_eq!(
                    r.draws[0].pipeline.as_ref().unwrap().storage,
                    Some(xenora_shader::GraphicsStorageType::Float)
                );
                let GraphicsAttachmentRootInput::Host(bytes) = &r.roots[1].prior else {
                    panic!("full semantic root")
                };
                assert_eq!(bytes.as_ref(), prior());
                let mut expected = prior();
                expected[132..140].copy_from_slice(&[64, 128, 191, 255, 64, 128, 191, 255]);
                let mut output: Vec<_> = r
                    .roots
                    .iter()
                    .enumerate()
                    .map(|(i, root)| GraphicsAttachmentRootResult {
                        output: root.output,
                        desc: root.desc,
                        bytes: if i == 0 {
                            [255, 0, 0, 255].repeat(2)
                        } else {
                            expected.clone()
                        },
                    })
                    .collect();
                if fail.load(Ordering::SeqCst) {
                    output[1].desc.format = TextureFormat::Bgra8Unorm;
                }
                Ok(Box::new(Ticket(Some(
                    GraphicsQueuedWorkOutput::AttachmentTransaction(output),
                ))))
            },
        );
        h.ctx
            .enable_resource_roles(GraphicsResourceRoleCapabilities {
                sampled_formats: 0x3ff,
                color_formats: 0x1f9,
            })
            .unwrap();
        h.ctx.enable_attachment_transactions().unwrap();
        h.ctx.enable_shader_storage_views().unwrap();
        Self {
            h,
            d,
            q,
            a,
            l,
            root,
            view,
            color,
            cv,
            pipeline,
            vb,
            calls,
            bad,
        }
    }
    fn record(&mut self, variant: &str) {
        self.h.ok(
            13,
            &[
                self.l,
                self.root,
                0x400,
                if variant == "wrong_state" { 0x80 } else { 8 },
            ],
        );
        self.h.ok(13, &[self.l, self.vb, 0x400, 1]);
        let cv = if variant == "color_feedback" {
            self.h.ok(61, &[self.d, self.root, 1, 1, 0])
        } else {
            self.cv
        };
        self.h
            .memory
            .write_bytes(
                DATA + 2048,
                &[cv, 0, 0, 0, 0, 0, 0, 1f64.to_bits()]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        self.h.ok(63, &[self.l, 1, DATA + 2048]);
        self.h.ok(
            69,
            &[self.l, if variant == "missing" { 0 } else { self.view }],
        );
        self.h
            .ok(17, &[self.l, if variant == "extent" { 1 } else { 2 }, 1]);
        self.h.ok(14, &[self.l, self.pipeline]);
        self.h.ok(18, &[self.l, self.vb, 0, 16]);
        self.h
            .memory
            .write_bytes(
                DATA + 3072,
                &[1u32, 0, 0, 0, 0, 0, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        self.h.ok(41, &[self.l, DATA + 3072, 1]);
        if variant == "sampled_feedback" {
            let view = self.h.ok(52, &[self.d, self.root, 1, 1, 1, 0]);
            self.h
                .memory
                .write_bytes(
                    DATA + 2048,
                    &[0u64, view, 0, 0, 0, 0, 1, 0, 0, 0]
                        .into_iter()
                        .flat_map(u64::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            self.h.ok(39, &[self.l, DATA + 2048]);
        }
        self.h.ok(
            22,
            &[
                self.l,
                if variant == "depth" { 2 } else { 0 },
                0,
                0,
                15,
                u64::from(variant == "origin"),
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        );
        if matches!(variant, "indexed_three" | "indexed_six") {
            let indices = self.h.ok(5, &[self.d, 12, 0x400]);
            self.h
                .upload(indices, &[0, 0, 1, 0, 2, 0, 0, 0, 1, 0, 2, 0]);
            self.h.ok(13, &[self.l, indices, 0x400, 2]);
            self.h.ok(20, &[self.l, indices, 0, 57]); // R16_UINT
            self.h.ok(
                24,
                &[
                    self.l,
                    if variant == "indexed_six" { 6 } else { 3 },
                    1,
                    0,
                    0,
                    0,
                ],
            );
        } else {
            self.h.ok(
                23,
                &[
                    self.l,
                    if variant == "six_vertices" { 6 } else { 3 },
                    if variant == "instances" { 2 } else { 1 },
                    0,
                    0,
                ],
            );
        }
        if variant == "two_draws" {
            self.h.ok(23, &[self.l, 3, 1, 0, 0]);
        }
        self.h.ok(
            13,
            &[
                self.l,
                self.root,
                if variant == "wrong_state" { 0x80 } else { 8 },
                0x800,
            ],
        );
        self.h.ok(13, &[self.l, self.color, 4, 0x800]);
        self.h.ok(12, &[self.l]);
    }
}
#[test]
fn cpu_storage_views_invalid_shape_feedback_and_capability_reject_before_dispatch() {
    for variant in [
        "wrong_state",
        "missing",
        "extent",
        "origin",
        "depth",
        "six_vertices",
        "indexed_six",
        "sampled_feedback",
        "instances",
        "two_draws",
        "color_feedback",
        "downgrade",
    ] {
        let mut s = Scene::new();
        s.record(variant);
        if variant == "downgrade" {
            s.h.ctx.set_metal_typed_copy_queued_backend(
                GraphicsTextureCapabilities::default(),
                GraphicsTypedCopyCapabilities { formats: 0x3ff },
                |_| panic!("downgraded storage dispatch"),
            );
        }
        assert_eq!(s.h.call(35, &[s.q, s.l, 0, 0]), 2, "{variant}");
        assert_eq!(s.calls.load(Ordering::SeqCst), 0, "{variant}");
        assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
        s.h.upload(s.root, &[0x59; 4]);
    }
}
#[test]
fn cpu_storage_views_later_root_schema_failure_is_atomic_then_recovers() {
    let mut s = Scene::new();
    s.record("valid");
    s.bad.store(true, Ordering::SeqCst);
    let first = s.h.ok(35, &[s.q, s.l, 0, 0]);
    assert_eq!(s.h.ok(32, &[first]), 2);
    s.h.ok(33, &[first]);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    s.h.upload(s.root, &[0x59; 4]); // Failed final COPY_SOURCE transition did not publish.
    s.bad.store(false, Ordering::SeqCst);
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    s.h.ok(11, &[s.l, s.a]);
    s.h.ok(68, &[s.view]);
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
    s.h.ok(10, &[s.root, 0, DATA, 144]);
    let mut bytes = [0; 144];
    s.h.memory.read_bytes(DATA, &mut bytes).unwrap();
    let mut expected = prior();
    expected[132..140].copy_from_slice(&[64, 128, 191, 255, 64, 128, 191, 255]);
    assert_eq!(bytes.as_slice(), expected);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    s.h.ok(10, &[s.color, 0, DATA, 8]);
    let mut color = [0; 8];
    s.h.memory.read_bytes(DATA, &mut color).unwrap();
    assert_eq!(color, [255, 0, 0, 255, 255, 0, 0, 255]);
    s.h.ok(26, &[s.root]);
}
#[test]
fn cpu_storage_views_descriptor_lifetime_and_result_preflight() {
    let mut s = Scene::new();
    for (mip, slice) in [(3, 0), (0, 2), (u64::MAX, 0), (0, u64::MAX)] {
        assert_eq!(s.h.call(67, &[s.d, s.root, mip, slice]), 2);
    }
    for format in [1, 2, 3, 4, 5, 6, 7, 9] {
        let root = s.h.ok(53, &[s.d, 4, 4, 1, 0, format, 0x400]);
        assert_eq!(s.h.call(67, &[s.d, root, 0, 0]), 2);
        s.h.ok(26, &[root]);
    }
    let uint = s.h.ok(53, &[s.d, 2, 1, 1, 0, 8, 0x400]);
    let view = s.h.ok(67, &[s.d, uint, 0, 0]);
    s.h.ok(68, &[view]);
    s.h.ok(26, &[uint]);
    assert_eq!(s.h.call(26, &[s.root]), 2, "live storage view pins root");
    s.h.prepare(&[s.view]);
    s.h.memory
        .write_bytes(DESC + 8, &68u64.to_le_bytes())
        .unwrap();
    assert!(graphics_call_v1(&mut s.h.ctx, &mut s.h.memory, DESC, 128, OUT + 4).is_err());
    assert_eq!(s.h.result(), SENTINEL);
    s.record("valid");
    assert_eq!(s.h.call(68, &[s.view]), 2, "recorded view retained");
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    s.h.ok(11, &[s.l, s.a]);
    s.h.ok(68, &[s.view]);
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
    s.h.ok(26, &[s.root]);
}

#[test]
fn cpu_storage_views_indexed_triangle_is_valid() {
    let mut s = Scene::new();
    s.record("indexed_three");
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    s.h.ok(10, &[s.root, 132, DATA, 8]);
    let mut bytes = [0; 8];
    s.h.memory.read_bytes(DATA, &mut bytes).unwrap();
    assert_eq!(bytes, [64, 128, 191, 255, 64, 128, 191, 255]);
}

#[test]
fn cpu_storage_views_uint_pipeline_rejects_rgba_storage_before_dispatch() {
    let mut s = Scene::new();
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-storage-uint-sampled-ps.bin");
    s.h.memory.write_bytes(DATA, vs).unwrap();
    s.h.memory.write_bytes(DATA + 1024, ps).unwrap();
    s.pipeline = s.h.ok(
        55,
        &[
            s.d,
            DATA,
            vs.len() as u64,
            DATA + 1024,
            ps.len() as u64,
            0x145,
        ],
    );
    // Compatible Uint color and sample bindings isolate the incompatible storage family.
    s.color = s.h.ok(53, &[s.d, 2, 1, 1, 0, 8, 4]);
    s.cv = s.h.ok(61, &[s.d, s.color, 0, 0, 0]);
    let source = s.h.ok(53, &[s.d, 1, 1, 1, 0, 8, 0x400]);
    s.h.upload(source, &[1, 0, 0, 1]);
    let view = s.h.ok(37, &[s.d, source, 0, 1, 12]);
    s.h.ok(13, &[s.l, s.root, 0x400, 8]);
    s.h.ok(13, &[s.l, s.vb, 0x400, 1]);
    s.h.ok(13, &[s.l, source, 0x400, 0x80]);
    s.h.memory
        .write_bytes(
            DATA,
            &[s.cv, 0, 0, 1, 0, 0, 0, 0]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    s.h.ok(63, &[s.l, 1, DATA]);
    s.h.memory
        .write_bytes(
            DATA,
            &[0u64, view, 0, 0, 0, 0, 1, 0, 0, 0]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    s.h.ok(39, &[s.l, DATA]);
    s.h.ok(69, &[s.l, s.view]);
    s.h.ok(17, &[s.l, 2, 1]);
    s.h.ok(14, &[s.l, s.pipeline]);
    s.h.ok(18, &[s.l, s.vb, 0, 16]);
    s.h.memory
        .write_bytes(
            DATA,
            &[1u32, 0, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    s.h.ok(41, &[s.l, DATA, 1]);
    s.h.ok(23, &[s.l, 3, 1, 0, 0]);
    s.h.ok(12, &[s.l]);
    assert_eq!(s.h.call(35, &[s.q, s.l, 0, 0]), 2);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_storage_views_full_root_budget_rejects_and_refunds_retained_receipts() {
    const MIB: usize = 1024 * 1024;
    // Rows are already 256-byte aligned. Full roots are 4 MiB color + 8 MiB
    // storage (two slices), although only one 4 MiB storage slice is written.
    // Each host seed reserves 9 root copies and each patch 3 root copies:
    // 12*(4+8) + selected storage 4 + render 12 = 160 MiB, plus 256 geometry bytes.
    // Two receipts exceed 256 MiB. Charging only the selected storage slice
    // instead of its full root would incorrectly admit both (224 MiB + 512).
    const CHARGE: usize = 160 * MIB + 256;
    let mut s = Scene::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    s.h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        move |work| {
            let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                panic!("expected storage transaction")
            };
            observed.fetch_add(1, Ordering::SeqCst);
            assert_eq!(r.extent, (1024, 1024));
            assert_eq!(r.roots.len(), 2);
            let storage = r.storage.as_ref().unwrap();
            assert_eq!((storage.root_index, storage.mip, storage.slice), (1, 0, 1));
            let output = r
                .roots
                .iter()
                .enumerate()
                .map(|(i, root)| {
                    let GraphicsAttachmentRootInput::Host(prior) = &root.prior else {
                        panic!("independent roots must have host seeds")
                    };
                    assert_eq!(prior.len(), if i == 0 { 4 * MIB } else { 8 * MIB });
                    assert!(prior.iter().all(|&b| b == 0));
                    let mut bytes = prior.to_vec();
                    if i == 0 {
                        bytes.fill(73);
                    } else {
                        bytes[4 * MIB..].fill(73);
                    }
                    GraphicsAttachmentRootResult {
                        output: root.output,
                        desc: root.desc,
                        bytes,
                    }
                })
                .collect();
            Ok(Box::new(Ticket(Some(
                GraphicsQueuedWorkOutput::AttachmentTransaction(output),
            ))))
        },
    );
    s.h.ctx
        .enable_resource_roles(GraphicsResourceRoleCapabilities {
            sampled_formats: 0x3ff,
            color_formats: 0x1f9,
        })
        .unwrap();
    s.h.ctx.enable_attachment_transactions().unwrap();
    s.h.ctx.enable_shader_storage_views().unwrap();
    let second_queue = s.h.ok(2, &[s.d]);
    let mut cases = Vec::new();
    for queue in [s.q, second_queue] {
        let allocator = s.h.ok(3, &[s.d]);
        let list = s.h.ok(4, &[s.d, allocator]);
        let color = s.h.ok(53, &[s.d, 1024, 1024, 1, 0, 0, 0x400]);
        let root = s.h.ok(53, &[s.d, 1024, 1024, 1, 2, 0, 0x400]);
        let cv = s.h.ok(61, &[s.d, color, 0, 0, 0]);
        let storage = s.h.ok(67, &[s.d, root, 0, 1]);
        let vb = s.h.ok(5, &[s.d, 48, 0x400]);
        s.h.upload(
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
        s.h.ok(13, &[list, color, 0x400, 4]);
        s.h.ok(13, &[list, root, 0x400, 8]);
        s.h.ok(13, &[list, vb, 0x400, 1]);
        s.h.memory
            .write_bytes(
                DATA,
                &[cv, 0, 0, 0, 0, 0, 0, 1f64.to_bits()]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        s.h.ok(63, &[list, 1, DATA]);
        s.h.ok(69, &[list, storage]);
        s.h.ok(17, &[list, 1024, 1024]);
        s.h.ok(14, &[list, s.pipeline]);
        s.h.ok(18, &[list, vb, 0, 16]);
        s.h.memory
            .write_bytes(
                DATA,
                &[1u32, 0, 0, 0, 0, 0, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        s.h.ok(41, &[list, DATA, 1]);
        s.h.ok(23, &[list, 3, 1, 0, 0]);
        s.h.ok(13, &[list, root, 8, 0x800]);
        s.h.ok(13, &[list, color, 4, 0x800]);
        s.h.ok(12, &[list]);
        cases.push((queue, list, root));
    }
    let (q0, l0, _) = cases[0];
    let (q1, l1, root1) = cases[1];
    let first = s.h.ok(35, &[q0, l0, 0, 0]);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(s.h.call(35, &[q1, l1, 0, 0]), 2);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "budget rejection must precede dispatch"
    );
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    s.h.upload(root1, &[0; 4]); // Rejected final state was not published.
    assert_eq!(s.h.ok(32, &[first]), 1);
    assert_eq!(
        s.h.ctx.queued_copy_allocation_bytes(),
        CHARGE,
        "completion alone does not refund"
    );
    assert_eq!(s.h.call(35, &[q1, l1, 0, 0]), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    s.h.ok(33, &[first]);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), 0);
    let second = s.h.ok(35, &[q1, l1, 0, 0]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(s.h.ok(32, &[second]), 1);
    s.h.ok(33, &[second]);
    assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), 0);
    for (offset, expected) in [(0, [0; 4]), ((4 * MIB) as u64, [73; 4])] {
        s.h.ok(10, &[root1, offset, DATA, 4]);
        let mut actual = [0; 4];
        s.h.memory.read_bytes(DATA, &mut actual).unwrap();
        assert_eq!(actual, expected, "full-root slice at byte {offset}");
    }
}
