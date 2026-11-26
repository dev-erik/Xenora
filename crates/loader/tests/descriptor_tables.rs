// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent descriptor revision/lifetime ABI tests; no GPU evidence.
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
}

fn update(h: &mut Harness, table: u64, first: u64, records: &[[u64; 2]]) -> u64 {
    h.memory
        .write_bytes(
            DATA,
            &records
                .iter()
                .flatten()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    h.ok(71, &[table, first, records.len() as u64, DATA])
}

#[test]
fn cpu_descriptor_tables_atomic_update_and_result_preflight() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let table = h.ok(70, &[d]);
    let first = update(&mut h, table, 0, &[[0, 0]; 8]);
    for records in [[[0, 0], [3, u64::MAX]], [[0, 0], [0, 1]], [[0, 0], [9, 0]]] {
        h.memory
            .write_bytes(
                DATA,
                &records
                    .into_iter()
                    .flatten()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(h.call(71, &[table, 0, 2, DATA]), 2);
    }
    h.prepare(&[table, 0, 1, DATA]);
    h.memory
        .write_bytes(DESC + 8, &71u64.to_le_bytes())
        .unwrap();
    h.memory.write_bytes(DATA, &[0; 16]).unwrap();
    assert!(graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 4).is_err());
    assert_eq!(h.result(), SENTINEL);
    assert_eq!(
        update(&mut h, table, 0, &[[0, 0]]),
        first + 1,
        "failed updates consumed revisions"
    );
    for (first, count) in [(8, 1), (7, 2), (u64::MAX, 2)] {
        assert_eq!(h.call(71, &[table, first, count, DATA]), 2);
    }
    h.prepare(&[table]);
    h.memory
        .write_bytes(DESC + 8, &72u64.to_le_bytes())
        .unwrap();
    assert!(graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 4).is_err());
    h.ok(72, &[table]);
    assert_eq!(h.call(72, &[table]), 2);
}

#[test]
fn cpu_descriptor_tables_normalized_handles_and_superseded_recorded_roots() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let table = h.ok(70, &[d]);
    let root = h.ok(53, &[d, 2, 2, 1, 0, 0, 0x400]);
    let view = h.ok(37, &[d, root, 0, 1, 0]);
    let sampler = h.ok(46, &[d, 0, 0, 0]);
    update(&mut h, table, 0, &[[1, view]]);
    update(&mut h, table, 4, &[[3, sampler]]);
    h.ok(38, &[view]);
    h.ok(47, &[sampler]);
    assert_eq!(h.call(26, &[root]), 2, "live normalized table retains root");
    h.ok(73, &[l, table]);
    update(&mut h, table, 0, &[[0, 0]; 8]);
    h.ok(73, &[l, table]);
    h.ok(72, &[table]);
    assert_eq!(
        h.call(26, &[root]),
        2,
        "superseded recorded revision retains root"
    );
    h.ok(12, &[l]);
    assert_eq!(h.call(26, &[root]), 2);
    h.ok(11, &[l, a]);
    h.ok(26, &[root]);
}

#[test]
fn cpu_descriptor_tables_revision_cap_refunds_after_recording_reset() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let table = h.ok(70, &[d]);
    h.ok(73, &[l, table]); // Capture initial revision, too.
    for _ in 1..256 {
        update(&mut h, table, 0, &[[0, 0]]);
        h.ok(73, &[l, table]);
    }
    h.memory.write_bytes(DATA, &[0; 16]).unwrap();
    assert_eq!(
        h.call(71, &[table, 0, 1, DATA]),
        2,
        "257th retained revision"
    );
    h.ok(12, &[l]);
    h.ok(11, &[l, a]);
    update(&mut h, table, 0, &[[0, 0]]);
    h.ok(72, &[table]);
}

struct Ready(Option<GraphicsQueuedWorkOutput>);
impl GraphicsQueuedPendingWork for Ready {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.take().map(Ok)
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().unwrap()
    }
    fn reject_dependency(&mut self, _: GraphicsQueuePoint) {}
}
struct DrawScene {
    h: Harness,
    d: u64,
    q: u64,
    a: u64,
    l: u64,
    table: u64,
    roots: Vec<u64>,
    views: Vec<u64>,
    samplers: Vec<u64>,
    target: u64,
    seen: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
}
impl DrawScene {
    fn new() -> Self {
        Self::with_all_slots(true)
    }
    fn with_all_slots(all_slots: bool) -> Self {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let q = h.ok(2, &[d]);
        let a = h.ok(3, &[d]);
        let l = h.ok(4, &[d, a]);
        let table = h.ok(70, &[d]);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = seen.clone();
        h.ctx.set_metal_array_view_queued_backend(
            GraphicsTextureCapabilities::default(),
            move |work| {
                let r = match work {
                    GraphicsQueuedWorkRequest::Render(r) => r,
                    GraphicsQueuedWorkRequest::Copy(r) => {
                        let GraphicsCopyInput::Host(bytes) = r.source else {
                            panic!("independent descendant host copy")
                        };
                        return Ok(Box::new(Ready(Some(GraphicsQueuedWorkOutput::Copy {
                            layout: r.layout,
                            bytes: bytes.to_vec(),
                        }))));
                    }
                    _ => panic!("render or descendant copy"),
                };
                for draw in &r.draws {
                    let descriptors = draw
                        .descriptor_bindings
                        .as_ref()
                        .expect("four-slot descriptor mode");
                    let mut values = Vec::new();
                    for slot in 0..4 {
                        if !all_slots && slot > 0 {
                            assert!(descriptors.textures[slot].is_none(), "unused slot uploaded");
                            continue;
                        }
                        let Some(GraphicsTextureInput::Host(snapshot)) =
                            &descriptors.textures[slot]
                        else {
                            panic!("host slot {slot}")
                        };
                        assert!(descriptors.samplers[slot].is_some());
                        values.push(snapshot.pixels[0]);
                    }
                    observed.lock().unwrap().push(values);
                }
                Ok(Box::new(Ready(Some(GraphicsQueuedWorkOutput::Render(
                    GraphicsPassOutput {
                        colors: vec![vec![17, 53, 127, 193]],
                        color_layouts: None,
                        depth: None,
                    },
                )))))
            },
        );
        h.ctx.enable_descriptor_tables().unwrap();
        let mut roots = Vec::new();
        let mut views = Vec::new();
        let mut samplers = Vec::new();
        for value in [17u8, 53, 127, 193] {
            let root = h.ok(53, &[d, 1, 1, 1, 0, 0, 0x400]);
            h.memory.write_bytes(DATA, &[value, 0, 0, 255]).unwrap();
            h.ok(9, &[root, 0, DATA, 4]);
            let view = h.ok(37, &[d, root, 0, 1, 0]);
            let sampler = h.ok(46, &[d, 0, 0, 0]);
            if all_slots || roots.is_empty() {
                h.ok(13, &[l, root, 0x400, 0x80]);
            }
            roots.push(root);
            views.push(view);
            samplers.push(sampler);
        }
        let mut records: Vec<_> = views.iter().map(|v| [1, *v]).collect();
        records.extend(samplers.iter().map(|s| [3, *s]));
        update(&mut h, table, 0, &records);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps: &[u8] = if all_slots {
            include_bytes!("../../../fixtures/rdna2/x9-descriptor-dynamic-ps.bin")
        } else {
            include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin")
        };
        h.memory.write_bytes(DATA, vs).unwrap();
        h.memory.write_bytes(DATA + 1024, ps).unwrap();
        h.memory.write_bytes(DATA + 2048, &[0; 96]).unwrap();
        let pipeline = h.ok(
            74,
            &[
                d,
                DATA,
                vs.len() as u64,
                DATA + 1024,
                ps.len() as u64,
                DATA + 2048,
            ],
        );
        let vb = h.ok(5, &[d, 48, 0x400]);
        let cb = h.ok(5, &[d, 48, 0x400]);
        for (resource, vertices) in [
            (
                vb,
                [
                    [-1f32, -1., 0.5, 1.],
                    [3., -1., 0.5, 1.],
                    [-1., 3., 0.5, 1.],
                ],
            ),
            (cb, [[0.5, 0.5, 0., 1.]; 3]),
        ] {
            h.memory
                .write_bytes(
                    DATA,
                    &vertices
                        .into_iter()
                        .flatten()
                        .flat_map(f32::to_le_bytes)
                        .collect::<Vec<_>>(),
                )
                .unwrap();
            h.ok(9, &[resource, 0, DATA, 48]);
            h.ok(13, &[l, resource, 0x400, 1]);
        }
        let target = h.ok(6, &[d, 1, 1, 1, 4]);
        h.memory
            .write_bytes(
                DATA,
                &[target, 0]
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        h.ok(15, &[l, 1, DATA]);
        h.ok(17, &[l, 1, 1]);
        h.ok(14, &[l, pipeline]);
        h.ok(18, &[l, vb, 0, 16]);
        h.ok(19, &[l, cb, 0, 16]);
        Self {
            h,
            d,
            q,
            a,
            l,
            table,
            roots,
            views,
            samplers,
            target,
            seen,
        }
    }
    fn draw(&mut self, texture: u32, sampler: u32) {
        self.h
            .memory
            .write_bytes(
                DATA,
                &[texture, sampler, 0, 0, 0, 0, 0, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        self.h.ok(41, &[self.l, DATA, 1]);
        self.h.ok(23, &[self.l, 3, 1, 0, 0]);
    }
    fn close(&mut self) {
        self.h.ok(13, &[self.l, self.target, 4, 0x800]);
        self.h.ok(12, &[self.l]);
    }
}

#[test]
fn cpu_descriptor_tables_required_union_and_selector_reject_before_dispatch() {
    for (texture, sampler, missing) in [
        (4, 0, None),
        (u32::MAX, 0, None),
        (1f32.to_bits(), 0, None),
        (0, 4, None),
        (0, u32::MAX, None),
        (0, 1f32.to_bits(), None),
        (0, 0, Some(3)),
        (0, 0, Some(7)),
    ] {
        let mut s = DrawScene::new();
        if let Some(slot) = missing {
            update(&mut s.h, s.table, slot, &[[0, 0]]);
        }
        s.h.ok(73, &[s.l, s.table]);
        s.draw(texture, sampler);
        s.close();
        assert_eq!(
            s.h.call(35, &[s.q, s.l, 0, 0]),
            2,
            "texture={texture:#x} sampler={sampler:#x} missing={missing:?}"
        );
        assert!(s.seen.lock().unwrap().is_empty());
        assert_eq!(s.h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(s.h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn cpu_descriptor_tables_recorded_revisions_resolve_pixels_at_submission() {
    let mut s = DrawScene::new();
    s.h.ok(73, &[s.l, s.table]);
    update(
        &mut s.h,
        s.table,
        0,
        &[
            [1, s.views[3]],
            [1, s.views[1]],
            [1, s.views[2]],
            [1, s.views[0]],
        ],
    );
    s.draw(0, 0); // Captured A even though live table is now B.
    s.h.ok(73, &[s.l, s.table]);
    update(&mut s.h, s.table, 0, &[[0, 0]; 8]);
    s.draw(3, 3); // Captured B survives live table becoming null.
    for view in &s.views {
        s.h.ok(38, &[*view]);
    }
    for sampler in &s.samplers {
        s.h.ok(47, &[*sampler]);
    }
    s.h.ok(72, &[s.table]);
    // Pixels snapshot at submission, not descriptor update/bind time.
    s.h.memory.write_bytes(DATA, &[29, 0, 0, 255]).unwrap();
    s.h.ok(9, &[s.roots[0], 0, DATA, 4]);
    s.close();
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    assert_eq!(
        *s.seen.lock().unwrap(),
        vec![vec![29, 53, 127, 193], vec![193, 53, 127, 29]]
    );
    s.h.ok(11, &[s.l, s.a]);
    for root in &s.roots {
        s.h.ok(26, &[*root]);
    }
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
}

#[test]
fn cpu_descriptor_tables_backend_replacement_revokes_support() {
    let mut s = DrawScene::new();
    s.h.ok(73, &[s.l, s.table]);
    s.draw(0, 0);
    s.close();
    s.h.ctx
        .set_metal_array_view_queued_backend(GraphicsTextureCapabilities::default(), |_| {
            panic!("legacy dispatch")
        });
    assert_eq!(s.h.call(35, &[s.q, s.l, 0, 0]), 2);
    assert!(s.seen.lock().unwrap().is_empty());
}

#[test]
fn cpu_descriptor_tables_wrong_typed_candidate_rejects_even_when_unselected() {
    let mut s = DrawScene::new();
    let root = s.h.ok(53, &[s.d, 1, 1, 1, 0, 8, 0x80]);
    let view = s.h.ok(37, &[s.d, root, 0, 1, 12]);
    update(&mut s.h, s.table, 3, &[[1, view]]);
    s.h.ok(73, &[s.l, s.table]);
    s.draw(0, 0);
    s.close();
    assert_eq!(s.h.call(35, &[s.q, s.l, 0, 0]), 2);
    assert!(s.seen.lock().unwrap().is_empty());
}

#[test]
fn cpu_descriptor_tables_unused_entries_retain_roots_without_sample_uploads() {
    let mut s = DrawScene::with_all_slots(false);
    // Slots1..3 deliberately stay COPY_DEST: requiring sampled state would be wrong.
    s.h.ok(73, &[s.l, s.table]);
    s.draw(0, 0);
    s.close();
    for view in &s.views {
        s.h.ok(38, &[*view]);
    }
    assert_eq!(
        s.h.call(26, &[s.roots[3]]),
        2,
        "unused normalized entry retains backing"
    );
    s.h.ok(72, &[s.table]);
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    assert_eq!(*s.seen.lock().unwrap(), vec![vec![17]]);
    s.h.ok(11, &[s.l, s.a]);
    for root in &s.roots {
        s.h.ok(26, &[*root]);
    }
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    s.h.ok(33, &[receipt]);
}

#[test]
fn cpu_descriptor_tables_pending_revisions_hold_cap_until_receipt_release() {
    let mut s = DrawScene::new();
    s.h.ok(73, &[s.l, s.table]);
    for _ in 1..256 {
        update(&mut s.h, s.table, 0, &[[1, s.views[0]]]);
        s.h.ok(73, &[s.l, s.table]);
    }
    s.draw(0, 0);
    s.close();
    let receipt = s.h.ok(35, &[s.q, s.l, 0, 0]);
    s.h.ok(11, &[s.l, s.a]);
    s.h.memory.write_bytes(DATA, &[0; 16]).unwrap();
    assert_eq!(
        s.h.call(71, &[s.table, 0, 1, DATA]),
        2,
        "pending ownership survives reset"
    );
    assert_eq!(s.h.ok(32, &[receipt]), 1);
    assert_eq!(
        s.h.call(71, &[s.table, 0, 1, DATA]),
        2,
        "completed receipt still owns revisions"
    );
    s.h.ok(33, &[receipt]);
    update(&mut s.h, s.table, 0, &[[0, 0]]);
}

#[test]
fn cpu_descriptor_tables_mixed_revision_owners_count_once_and_net_replacement() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let recorded = h.ok(70, &[d]);
    h.ok(73, &[l, recorded]);
    for _ in 1..255 {
        update(&mut h, recorded, 0, &[[0, 0]]);
        h.ok(73, &[l, recorded]);
    }
    assert_eq!(h.ctx.descriptor_revision_count(), 255);
    let current_only = h.ok(70, &[d]);
    assert_eq!(h.ctx.descriptor_revision_count(), 256);
    // Multiple Arc owners of one captured revision must not consume more slots.
    for _ in 0..8 {
        h.ok(73, &[l, recorded]);
    }
    assert_eq!(h.ctx.descriptor_revision_count(), 256);
    // Atomic replacement drops the sole owner of the old current-only revision.
    // Its net live count is unchanged, so a full ledger must still admit it.
    for _ in 0..3 {
        update(&mut h, current_only, 0, &[[0, 0]]);
        assert_eq!(h.ctx.descriptor_revision_count(), 256);
    }
    h.memory.write_bytes(DATA, &[0; 16]).unwrap();
    assert_eq!(
        h.call(71, &[recorded, 0, 1, DATA]),
        2,
        "captured old revision cannot be refunded"
    );
    assert_eq!(h.ctx.descriptor_revision_count(), 256);
    h.ok(72, &[current_only]);
    assert_eq!(h.ctx.descriptor_revision_count(), 255);
    update(&mut h, recorded, 0, &[[0, 0]]);
    assert_eq!(h.ctx.descriptor_revision_count(), 256);
    h.ok(12, &[l]);
    h.ok(11, &[l, a]);
    assert_eq!(h.ctx.descriptor_revision_count(), 1);
    h.ok(72, &[recorded]);
    assert_eq!(h.ctx.descriptor_revision_count(), 0);
}

#[test]
fn cpu_descriptor_tables_released_ancestor_revision_survives_retained_descendant() {
    let mut s = DrawScene::new();
    s.h.ok(73, &[s.l, s.table]);
    s.draw(0, 0);
    s.close();
    let ancestor = s.h.ok(35, &[s.q, s.l, 0, 0]);
    let point = s.h.ok(36, &[ancestor]);
    let q = s.h.ok(2, &[s.d]);
    let a = s.h.ok(3, &[s.d]);
    let l = s.h.ok(4, &[s.d, a]);
    let source = s.h.ok(5, &[s.d, 4, 0x400]);
    let dest = s.h.ok(5, &[s.d, 4, 0x400]);
    s.h.memory.write_bytes(DATA, &[19, 71, 137, 233]).unwrap();
    s.h.ok(9, &[source, 0, DATA, 4]);
    s.h.ok(13, &[l, source, 0x400, 0x800]);
    s.h.ok(42, &[l, dest, source]);
    s.h.ok(13, &[l, dest, 0x400, 0x800]);
    s.h.ok(12, &[l]);
    s.h.memory
        .write_bytes(
            DATA,
            &[s.q, point]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let descendant = s.h.ok(35, &[q, l, DATA, 1]);
    s.h.ok(11, &[s.l, s.a]);
    s.h.ok(11, &[l, a]);
    for view in &s.views {
        s.h.ok(38, &[*view]);
    }
    for sampler in &s.samplers {
        s.h.ok(47, &[*sampler]);
    }
    s.h.ok(72, &[s.table]);
    for root in &s.roots {
        s.h.ok(26, &[*root]);
    } // Retire public roots while receipts own them.
    assert_eq!(s.h.ok(32, &[descendant]), 1);
    s.h.ok(33, &[ancestor]);
    assert_eq!(
        s.h.ctx.descriptor_revision_count(),
        1,
        "released ancestor remains owned by completed descendant"
    );
    s.h.ok(33, &[descendant]);
    assert_eq!(
        s.h.ctx.descriptor_revision_count(),
        0,
        "final descendant release drops ancestor revision/root owners"
    );
    s.h.ok(10, &[dest, 0, DATA, 4]);
    let mut bytes = [0; 4];
    s.h.memory.read_bytes(DATA, &mut bytes).unwrap();
    assert_eq!(bytes, [19, 71, 137, 233]);
}
