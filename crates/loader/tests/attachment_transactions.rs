// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent selected-root publication tests; mock output is not GPU evidence.
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
#[test]
fn cpu_attachment_transactions_same_root_disjoint_clear_preserves_full_backing() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let q = h.ok(2, &[d]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let root = h.ok(53, &[d, 5, 3, 3, 2, 0, 0x400]);
    let initial = vec![17; 144];
    h.upload(root, &initial);
    let views = [h.ok(61, &[d, root, 1, 0, 0]), h.ok(61, &[d, root, 1, 1, 0])];
    let mut expected = initial.clone();
    expected[60..68].copy_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255]);
    expected[132..140].copy_from_slice(&[0, 255, 0, 255, 0, 255, 0, 255]);
    let answer = expected.clone();
    h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        move |work| {
            let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                panic!("transaction")
            };
            assert_eq!(r.extent, (2, 1));
            assert_eq!(r.roots.len(), 1);
            assert_eq!(r.colors.len(), 2);
            for (i, color) in r.colors.iter().enumerate() {
                assert_eq!((color.root_index, color.mip, color.slice), (0, 1, i as u32));
                assert!(!color.load);
            }
            assert_eq!(
                r.roots[0].desc,
                GraphicsTypedTextureDesc {
                    format: TextureFormat::Rgba8Unorm,
                    extent: (5, 3),
                    mip_levels: 3,
                    array_layers: Some(2)
                }
            );
            let GraphicsAttachmentRootInput::Host(prior) = &r.roots[0].prior else {
                panic!("host")
            };
            assert_eq!(prior.as_ref(), initial);
            Ok(Box::new(Ticket(Some(
                GraphicsQueuedWorkOutput::AttachmentTransaction(vec![
                    GraphicsAttachmentRootResult {
                        output: r.roots[0].output,
                        desc: r.roots[0].desc,
                        bytes: answer.clone(),
                    },
                ]),
            ))))
        },
    );
    enable(&mut h);
    h.ok(13, &[l, root, 0x400, 4]);
    bind(&mut h, l, &views, false);
    h.ok(17, &[l, 2, 1]);
    h.ok(13, &[l, root, 4, 0x800]);
    h.ok(12, &[l]);
    assert_eq!(h.call(26, &[root]), 2);
    let receipt = h.ok(35, &[q, l, 0, 0]);
    h.ok(11, &[l, a]);
    for view in views {
        h.ok(62, &[view]);
    }
    assert_eq!(h.ok(32, &[receipt]), 1);
    h.ok(33, &[receipt]);
    h.ok(10, &[root, 0, DATA, 144]);
    let mut actual = vec![0; 144];
    h.memory.read_bytes(DATA, &mut actual).unwrap();
    assert_eq!(actual, expected);
    h.ok(26, &[root]);
}

#[test]
fn cpu_attachment_transactions_later_root_bad_metadata_is_atomic_and_retryable() {
    for failure in 0..4 {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let q = h.ok(2, &[d]);
        let a = h.ok(3, &[d]);
        let l = h.ok(4, &[d, a]);
        let roots = [
            h.ok(53, &[d, 5, 3, 3, 2, 0, 0x400]),
            h.ok(53, &[d, 5, 3, 3, 2, 0, 0x400]),
        ];
        for (i, r) in roots.iter().enumerate() {
            h.upload(*r, &[17 + i as u8; 144]);
        }
        let views = [
            h.ok(61, &[d, roots[0], 1, 1, 0]),
            h.ok(61, &[d, roots[1], 1, 1, 0]),
        ];
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = attempts.clone();
        h.ctx.set_metal_typed_copy_queued_backend(
            GraphicsTextureCapabilities::default(),
            GraphicsTypedCopyCapabilities { formats: 0x3ff },
            move |work| {
                let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                    panic!("transaction")
                };
                assert_eq!(r.roots.len(), 2);
                let mut results = Vec::new();
                for (i, root) in r.roots.iter().enumerate() {
                    let GraphicsAttachmentRootInput::Host(prior) = &root.prior else {
                        panic!("host")
                    };
                    assert_eq!(prior.as_ref(), vec![17 + i as u8; 144]);
                    let mut bytes = prior.to_vec();
                    bytes[132..140].copy_from_slice(&if i == 0 {
                        [255, 0, 0, 255, 255, 0, 0, 255]
                    } else {
                        [0, 255, 0, 255, 0, 255, 0, 255]
                    });
                    results.push(GraphicsAttachmentRootResult {
                        output: root.output,
                        desc: root.desc,
                        bytes,
                    });
                }
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    match failure {
                        0 => results[1].desc.format = TextureFormat::Bgra8Unorm,
                        1 => {
                            results[1].bytes.pop();
                        }
                        2 => results[1].output = results[0].output,
                        _ => {
                            results.pop();
                        }
                    }
                }
                Ok(Box::new(Ticket(Some(
                    GraphicsQueuedWorkOutput::AttachmentTransaction(results),
                ))))
            },
        );
        enable(&mut h);
        for root in roots {
            h.ok(13, &[l, root, 0x400, 4]);
        }
        bind(&mut h, l, &views, false);
        h.ok(17, &[l, 2, 1]);
        for root in roots {
            h.ok(13, &[l, root, 4, 0x800]);
        }
        h.ok(12, &[l]);
        let bad = h.ok(35, &[q, l, 0, 0]);
        assert_eq!(h.ok(32, &[bad]), 2);
        h.ok(33, &[bad]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
        let good = h.ok(35, &[q, l, 0, 0]);
        assert_eq!(h.ok(32, &[good]), 1);
        h.ok(33, &[good]);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        for (i, root) in roots.iter().enumerate() {
            let mut expected = vec![17 + i as u8; 144];
            expected[132..140].copy_from_slice(&if i == 0 {
                [255, 0, 0, 255, 255, 0, 0, 255]
            } else {
                [0, 255, 0, 255, 0, 255, 0, 255]
            });
            h.ok(10, &[*root, 0, DATA, 144]);
            let mut bytes = vec![0; 144];
            h.memory.read_bytes(DATA, &mut bytes).unwrap();
            assert_eq!(bytes, expected);
        }
    }
}

#[test]
fn cpu_attachment_transactions_descriptor_roles_overlap_and_result_preflight() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let root = h.ok(53, &[d, 5, 3, 3, 2, 0, 4]);
    let depth = h.ok(53, &[d, 5, 3, 3, 2, 9, 0x10]);
    let bc = h.ok(53, &[d, 4, 4, 1, 0, 1, 0x400]);
    for args in [
        [d, root, 3, 0, 0],
        [d, root, 0, 2, 0],
        [d, root, 1u64 << 32, 0, 0],
        [d, root, 0, 0, 2],
        [u64::MAX, root, 0, 0, 0],
        [d, root, 0, 0, 1],
        [d, depth, 0, 0, 0],
        [d, bc, 0, 0, 0],
    ] {
        assert_eq!(h.call(61, &args), 2);
    }
    // Invalid OUT is checked before creation; no hidden root reference remains.
    h.prepare(&[d, root, 1, 0, 0]);
    h.memory
        .write_bytes(DESC + 8, &61u64.to_le_bytes())
        .unwrap();
    assert!(graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 4).is_err());
    h.ok(26, &[root]);
    let root = h.ok(53, &[d, 5, 3, 3, 2, 0, 4]);
    let v = h.ok(61, &[d, root, 1, 0, 0]);
    let alias = h.ok(61, &[d, root, 1, 0, 0]);
    let other = h.ok(61, &[d, root, 0, 1, 0]);
    let z = h.ok(61, &[d, depth, 1, 1, 1]);
    for pair in [[v, v], [v, alias], [alias, v], [v, other], [v, z]] {
        let mut words = Vec::new();
        for view in pair {
            words.extend_from_slice(&[view, 0, 0, 0, 0, 0, 0, 0]);
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
        assert_eq!(h.call(63, &[l, 2, DATA]), 2);
    }
    assert_eq!(h.call(64, &[l, v, 1, 0]), 2);
    assert_eq!(h.call(64, &[l, z, 0, u64::from(f32::NAN.to_bits())]), 2);
    assert_eq!(h.call(64, &[l, 0, 1, 0]), 2);
    let record = [v, 0, 0, 0, 0, 0, 0, 0];
    h.memory
        .write_bytes(
            DATA,
            &record
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    for (opcode, args) in [
        (62u64, vec![v]),
        (63, vec![l, 1, DATA]),
        (64, vec![l, z, 1, 0]),
    ] {
        h.prepare(&args);
        h.memory
            .write_bytes(DESC + 8, &opcode.to_le_bytes())
            .unwrap();
        assert!(graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 4).is_err());
        assert_eq!(h.result(), SENTINEL);
    }
    // No invalid/preflight-failed binding appended a command pin; failed destroy
    // did not remove the handle either. Every explicit destroy must now succeed.
    for view in [v, alias, other, z] {
        h.ok(62, &[view]);
    }
    h.ok(26, &[root]);
    h.ok(26, &[depth]);
}

#[test]
fn cpu_attachment_transactions_backend_registration_revokes_recorded_capability() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let q = h.ok(2, &[d]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let root = h.ok(53, &[d, 1, 1, 1, 0, 0, 4]);
    let view = h.ok(61, &[d, root, 0, 0, 0]);
    h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        |_| panic!("unexpected dispatch"),
    );
    enable(&mut h);
    bind(&mut h, l, &[view], false);
    h.ok(17, &[l, 1, 1]);
    h.ok(12, &[l]);
    h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        |_| panic!("revoked transaction reached callback"),
    );
    h.ctx
        .enable_resource_roles(GraphicsResourceRoleCapabilities {
            sampled_formats: 0x3ff,
            color_formats: 0x1f9,
        })
        .unwrap();
    assert_eq!(h.call(35, &[q, l, 0, 0]), 2);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_attachment_transactions_fresh_legacy_depth_full_clear_then_load() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let q = h.ok(2, &[d]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let color = h.ok(6, &[d, 1, 1, 1, 4]);
    let depth = h.ok(7, &[d, 1, 1]);
    let cv = h.ok(61, &[d, color, 0, 0, 0]);
    let zv = h.ok(61, &[d, depth, 0, 0, 1]);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        move |work| {
            let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                panic!("transaction")
            };
            let n = count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(r.roots.len(), 2);
            let z = r.depth.unwrap();
            assert_eq!(z.root_index, 1);
            assert_eq!(
                z.load,
                if n == 0 {
                    GraphicsDepthLoad::Clear(0.25)
                } else {
                    GraphicsDepthLoad::Load
                }
            );
            if n == 1 {
                let GraphicsAttachmentRootInput::Host(bytes) = &r.roots[1].prior else {
                    panic!("published host depth")
                };
                assert_eq!(bytes.as_ref(), [0, 0, 128, 62]);
            }
            let results = r
                .roots
                .iter()
                .enumerate()
                .map(|(i, root)| GraphicsAttachmentRootResult {
                    output: root.output,
                    desc: root.desc,
                    bytes: if i == 0 {
                        vec![255, 0, 0, 255]
                    } else {
                        vec![0, 0, 128, 62]
                    },
                })
                .collect();
            Ok(Box::new(Ticket(Some(
                GraphicsQueuedWorkOutput::AttachmentTransaction(results),
            ))))
        },
    );
    enable(&mut h);
    bind(&mut h, l, &[cv], false);
    h.ok(64, &[l, zv, 1, 0]);
    h.ok(17, &[l, 1, 1]);
    h.ok(12, &[l]);
    assert_eq!(h.call(35, &[q, l, 0, 0]), 2);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "uninitialized Load dispatched"
    );
    h.ok(11, &[l, a]);
    for phase in 0..2 {
        bind(&mut h, l, &[cv], false);
        h.ok(
            64,
            &[
                l,
                zv,
                phase,
                if phase == 0 {
                    u64::from(0.25f32.to_bits())
                } else {
                    0
                },
            ],
        );
        h.ok(17, &[l, 1, 1]);
        h.ok(12, &[l]);
        let receipt = h.ok(35, &[q, l, 0, 0]);
        assert_eq!(h.ok(32, &[receipt]), 1);
        h.ok(33, &[receipt]);
        h.ok(11, &[l, a]);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn cpu_attachment_transactions_cumulative_full_root_budget_rejects_and_refunds() {
    // Two 4 MiB slices share one 8 MiB root. Each of two patches retains
    // 2*8 MiB packed payload + 8 MiB staging. Host seeding retains 9*8 MiB.
    // Render outputs/readback reserve 2*8 MiB + 4 MiB, plus 64 scratch bytes.
    // Thus one receipt costs exactly 140 MiB + 64; two exceed 256 MiB.
    const ROOT_BYTES: usize = 8 * 1024 * 1024;
    const CHARGE: usize = 140 * 1024 * 1024 + 64;
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let q = h.ok(2, &[d]);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    h.ctx.set_metal_typed_copy_queued_backend(
        GraphicsTextureCapabilities::default(),
        GraphicsTypedCopyCapabilities { formats: 0x3ff },
        move |work| {
            let GraphicsQueuedWorkRequest::AttachmentTransaction(r) = work else {
                panic!("transaction")
            };
            count.fetch_add(1, Ordering::SeqCst);
            assert_eq!(r.roots.len(), 1);
            assert_eq!(r.colors.len(), 2);
            assert_eq!(r.colors[0].slice, 0);
            assert_eq!(r.colors[1].slice, 1);
            let GraphicsAttachmentRootInput::Host(prior) = &r.roots[0].prior else {
                panic!("independent host root")
            };
            assert_eq!(prior.len(), ROOT_BYTES);
            assert!(
                prior.iter().all(|byte| *byte == 0),
                "rejection published bytes"
            );
            Ok(Box::new(Ticket(Some(
                GraphicsQueuedWorkOutput::AttachmentTransaction(vec![
                    GraphicsAttachmentRootResult {
                        output: r.roots[0].output,
                        desc: r.roots[0].desc,
                        bytes: vec![73; ROOT_BYTES],
                    },
                ]),
            ))))
        },
    );
    enable(&mut h);
    let mut lists = Vec::new();
    let mut roots = Vec::new();
    for _ in 0..2 {
        let a = h.ok(3, &[d]);
        let l = h.ok(4, &[d, a]);
        let root = h.ok(53, &[d, 1024, 1024, 1, 2, 0, 0x400]);
        let views = [h.ok(61, &[d, root, 0, 0, 0]), h.ok(61, &[d, root, 0, 1, 0])];
        h.ok(13, &[l, root, 0x400, 4]);
        bind(&mut h, l, &views, false);
        h.ok(17, &[l, 1024, 1024]);
        h.ok(13, &[l, root, 4, 0x800]);
        h.ok(12, &[l]);
        lists.push(l);
        roots.push(root);
    }
    let first = h.ok(35, &[q, lists[0], 0, 0]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(h.call(35, &[q, lists[1], 0, 0]), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "over-budget dispatch");
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    // The rejected list's final COPY_SOURCE transition must not publish.
    assert_eq!(h.call(10, &[roots[1], 0, DATA, 4]), 2);
    h.upload(roots[1], &[0; 4]); // Still COPY_DEST; leaves literal prior unchanged.
    assert_eq!(h.ok(32, &[first]), 1);
    assert_eq!(
        h.ctx.queued_copy_allocation_bytes(),
        CHARGE,
        "receipt owns charge after completion"
    );
    assert_eq!(h.call(35, &[q, lists[1], 0, 0]), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 1);
    h.ok(33, &[first]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    let second = h.ok(35, &[q, lists[1], 0, 0]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), CHARGE);
    assert_eq!(h.ok(32, &[second]), 1);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), CHARGE);
    // Completion retains the resource pin and charge until receipt release.
    h.ok(33, &[second]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.ok(10, &[roots[1], 0, DATA, 4]);
    let mut actual = [0; 4];
    h.memory.read_bytes(DATA, &mut actual).unwrap();
    assert_eq!(actual, [73; 4]);
}
