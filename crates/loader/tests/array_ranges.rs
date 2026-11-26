// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent range rebasing, lifetime and guest service validation; no GPU evidence.
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

fn record_draw(h: &mut Harness, d: u64, l: u64, texture: u64, view: u64) {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
    h.memory.write_bytes(DATA, vs).unwrap();
    h.memory.write_bytes(DATA + 1024, ps).unwrap();
    let pipeline = h.ok(8, &[d, DATA, vs.len() as u64, DATA + 1024, ps.len() as u64]);
    let vb = h.ok(5, &[d, 48, 0x400]);
    let cb = h.ok(5, &[d, 48, 0x400]);
    let positions = [[-1f32, -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
    let colors = [[0.25f32, 0.5, 0., 1.]; 3];
    h.upload(
        vb,
        &positions
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    h.upload(
        cb,
        &colors
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let target = h.ok(6, &[d, 2, 1, 1, 4]);
    let mut targets = [0u8; 16];
    targets[..8].copy_from_slice(&target.to_le_bytes());
    h.memory.write_bytes(DATA + 4096, &targets).unwrap();
    h.ok(15, &[l, 1, DATA + 4096]);
    h.ok(13, &[l, texture, 0x400, 0x80]);
    h.ok(13, &[l, vb, 0x400, 1]);
    h.ok(13, &[l, cb, 0x400, 1]);
    h.ok(17, &[l, 2, 1]);
    h.ok(14, &[l, pipeline]);
    h.ok(18, &[l, vb, 0, 16]);
    h.ok(19, &[l, cb, 0, 16]);
    let mut table = [0u64; 10];
    table[0] = view;
    table[2] = 1;
    h.memory
        .write_bytes(
            DATA + 4096,
            &table
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    h.ok(39, &[l, DATA + 4096]);
    h.ok(23, &[l, 3, 1, 0, 0]);
    h.ok(13, &[l, texture, 0x80, 0x400]);
    h.ok(12, &[l]);
}
#[test]
fn cpu_array_ranges_nested_canonical_root_and_independent_lifetime() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let q = h.ok(2, &[d]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let root = h.ok(51, &[d, 8, 4, 4, 4, 0x400]);
    // Root slices1..4/mips1..4, child slices2..4/mips2..4;
    // selected child slice1/mip1 is absolute root slice3/mip3.
    let parent = h.ok(57, &[d, root, 1, 3, 1, 3, 1]);
    let child = h.ok(58, &[d, parent, 1, 2, 1, 2, 0]);
    let selected = h.ok(59, &[d, child, 1, 1, 1, 0]);
    h.ok(60, &[parent]);
    assert_eq!(h.call(26, &[root]), 2);
    h.ok(60, &[child]);
    assert_eq!(h.call(26, &[root]), 2);
    let grandchild = h.ok(45, &[d, selected, 0, 1, 1]);
    h.ok(38, &[selected]);
    let seen = Arc::new(Mutex::new(false));
    let capture = seen.clone();
    h.ctx.set_metal_array_view_attachment_backend(
        GraphicsTextureCapabilities::default(),
        move |draws, _, _, _, _| {
            let b = &draws[0].sampled_bindings;
            let view = b.array_views[0].unwrap();
            assert_eq!((view.slice, view.base_mip, view.mip_count), (3, 3, 1));
            assert_eq!(view.format, GraphicsTextureViewFormat::Rgba8UnormSrgb);
            assert!(b.views[0].is_none());
            let root = b.textures[0].as_ref().unwrap();
            assert_eq!(
                (root.width, root.height, root.mip_levels, root.array_layers),
                (8, 4, 4, Some(4))
            );
            assert_eq!(root.pixels.len(), 688); // (128+32+8+4)*4, not a cropped slice.
            assert_eq!(&root.pixels[684..688], &[17, 53, 127, 193]);
            *capture.lock().unwrap() = true;
            Ok(GraphicsPassOutput {
                colors: vec![vec![19; 8]],
                depth: None,
                color_layouts: None,
            })
        },
    );
    let mut bytes = vec![0x59; 688];
    bytes[684..].copy_from_slice(&[17, 53, 127, 193]);
    h.upload(root, &bytes);
    record_draw(&mut h, d, l, root, grandchild);
    h.ok(25, &[q, l]);
    assert!(*seen.lock().unwrap());
    h.ok(11, &[l, a]);
    assert_eq!(h.call(26, &[root]), 2);
    h.ok(38, &[grandchild]);
    h.ok(26, &[root]);
    assert_eq!(h.call(60, &[child]), 2);
}

#[test]
fn cpu_array_ranges_relative_bounds_formats_and_one_layer() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let root = h.ok(51, &[d, 5, 3, 3, 4, 0x400]);
    for (base, count, mip, mips, format) in [
        (0, 0, 0, 1, 0),
        (0, 1, 0, 0, 0),
        (4, 1, 0, 1, 0),
        (3, 2, 0, 1, 0),
        (0, 1, 3, 1, 0),
        (0, 1, 2, 2, 0),
        (u64::from(u32::MAX), 2, 0, 1, 0),
        (0, 1, u64::from(u32::MAX), 2, 0),
        (1u64 << 32, 1, 0, 1, 0),
        (0, 1, 0, 1, 12),
        (0, 1, 0, 1, 99),
    ] {
        assert_eq!(h.call(57, &[d, root, base, count, mip, mips, format]), 2);
        assert_eq!(h.result(), SENTINEL);
    }
    // Resources are context-wide; existing view APIs validate device existence,
    // not per-resource device ownership. Test a nonexistent handle only.
    assert_eq!(h.call(57, &[u64::MAX, root, 0, 1, 0, 1, 0]), 2);
    let parent = h.ok(57, &[d, root, 1, 2, 1, 2, 0]);
    for args in [
        [d, parent, 2, 1, 0, 1, 0],
        [d, parent, 1, 2, 0, 1, 0],
        [d, parent, 0, 1, 2, 1, 0],
        [d, parent, 0, 1, 1, 2, 0],
    ] {
        assert_eq!(h.call(58, &args), 2);
    }
    for args in [
        [d, parent, 2, 0, 1, 0],
        [d, parent, 0, 2, 1, 0],
        [d, parent, 0, 1, 2, 0],
        [d, parent, 0, 0, 0, 0],
    ] {
        assert_eq!(h.call(59, &args), 2);
    }
    let ordinary = h.ok(6, &[d, 5, 3, 3, 0x400]);
    assert_eq!(h.call(57, &[d, ordinary, 0, 1, 0, 1, 0]), 2);
    let one = h.ok(51, &[d, 5, 3, 3, 1, 0x400]);
    let range = h.ok(57, &[d, one, 0, 1, 0, 3, 0]);
    let selected = h.ok(59, &[d, range, 0, 2, 1, 0]);
    h.ok(60, &[range]);
    assert_eq!(h.call(26, &[one]), 2);
    h.ok(38, &[selected]);
    h.ok(26, &[one]);
}

#[test]
fn cpu_array_ranges_not_direct_2d_binding_and_service_envelope() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let a = h.ok(3, &[d]);
    let l = h.ok(4, &[d, a]);
    let root = h.ok(51, &[d, 5, 3, 3, 2, 0x400]);
    let range = h.ok(57, &[d, root, 0, 2, 0, 3, 0]);
    let table = [range, 0, 1, 0, 0, 0, 0, 0, 0, 0];
    h.memory
        .write_bytes(
            DATA,
            &table
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(h.call(39, &[l, DATA]), 2);
    assert_eq!(h.call(21, &[l, range, 0, 0, 0]), 2);
    assert_eq!(h.call(38, &[range]), 2);
    assert_eq!(h.call(45, &[d, range, 0, 1, 0]), 2);
    for (opcode, args, reserved) in [
        (57u64, vec![d, root, 0, 2, 0, 3, 0], 72),
        (58, vec![d, range, 0, 1, 0, 1, 0], 72),
        (59, vec![d, range, 0, 0, 1, 0], 64),
        (60, vec![range], 24),
    ] {
        h.prepare(&args);
        h.memory
            .write_bytes(DESC + 8, &opcode.to_le_bytes())
            .unwrap();
        h.memory
            .write_bytes(DESC + reserved, &1u64.to_le_bytes())
            .unwrap();
        assert_eq!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT).unwrap(),
            1
        );
        assert_eq!(h.result(), SENTINEL);
    }
    h.prepare(&[d, range, 0, 0, 1, 0]);
    h.memory
        .write_bytes(DESC + 8, &59u64.to_le_bytes())
        .unwrap();
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 127, OUT).unwrap(),
        1
    );
    h.ok(60, &[range]);
    h.ok(26, &[root]);
}

#[test]
fn cpu_array_ranges_object_budget_failure_refund_and_no_root_leak() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let root = h.ok(51, &[d, 1, 1, 1, 1, 0x400]);
    let mut ranges = Vec::new();
    // Exactly 4096 live guest objects: device + backing + 4094 ranges.
    for _ in 0..4094 {
        ranges.push(h.ok(57, &[d, root, 0, 1, 0, 1, 0]));
    }
    assert_eq!(h.call(57, &[d, root, 0, 1, 0, 1, 0]), 2);
    assert_eq!(h.result(), SENTINEL);
    assert_eq!(h.call(58, &[d, ranges[0], 0, 1, 0, 1, 0]), 2);
    assert_eq!(h.call(59, &[d, ranges[0], 0, 0, 1, 0]), 2);
    let last = ranges.pop().unwrap();
    h.ok(60, &[last]);
    let selected = h.ok(59, &[d, ranges[0], 0, 0, 1, 0]);
    for range in ranges {
        h.ok(60, &[range]);
    }
    assert_eq!(h.call(26, &[root]), 2);
    h.ok(38, &[selected]);
    h.ok(26, &[root]);
    let replacement = h.ok(51, &[d, 1, 1, 1, 1, 0x400]);
    let range = h.ok(57, &[d, replacement, 0, 1, 0, 1, 0]);
    h.ok(60, &[range]);
    h.ok(26, &[replacement]);
}

#[test]
fn cpu_array_ranges_ten_format_families_nested_selection_and_root_retention() {
    // Stable guest storage/view codes, independently listed rather than obtained
    // from the format implementation. This is metadata evidence, not sampling.
    for (storage, view) in [
        (0, 0),
        (1, 2),
        (2, 4),
        (3, 6),
        (4, 7),
        (5, 8),
        (6, 10),
        (7, 11),
        (8, 12),
        (9, 13),
    ] {
        let mut h = Harness::new();
        let d = h.ok(1, &[]);
        let root = h.ok(53, &[d, 5, 3, 3, 3, storage, 0x400]);
        let parent = h.ok(57, &[d, root, 1, 2, 1, 2, view]);
        let incompatible = if storage == 0 { 12 } else { 0 };
        assert_eq!(
            h.call(58, &[d, parent, 0, 1, 0, 1, incompatible]),
            2,
            "child storage={storage}"
        );
        assert_eq!(
            h.call(59, &[d, parent, 0, 0, 1, incompatible]),
            2,
            "selected storage={storage}"
        );
        let child = h.ok(58, &[d, parent, 1, 1, 1, 1, view]);
        let selected = h.ok(59, &[d, child, 0, 0, 1, view]);
        // The selected view references root slice2/mip2 directly. Removing both
        // range handles cannot invalidate it or release its root reference.
        h.ok(60, &[parent]);
        h.ok(60, &[child]);
        assert_eq!(h.call(26, &[root]), 2, "root storage={storage}");
        let nested = h.ok(45, &[d, selected, 0, 1, view]);
        h.ok(38, &[selected]);
        assert_eq!(h.call(26, &[root]), 2);
        h.ok(38, &[nested]);
        h.ok(26, &[root]);
    }
}

#[test]
fn cpu_array_ranges_bad_result_span_preflights_creation_and_destroy() {
    let mut h = Harness::new();
    let d = h.ok(1, &[]);
    let root = h.ok(51, &[d, 1, 1, 1, 1, 0x400]);
    let parent = h.ok(57, &[d, root, 0, 1, 0, 1, 0]);
    for (opcode, args) in [
        (57u64, vec![d, root, 0, 1, 0, 1, 0]),
        (58, vec![d, parent, 0, 1, 0, 1, 0]),
        (59, vec![d, parent, 0, 0, 1, 0]),
        (60, vec![parent]),
    ] {
        h.prepare(&args);
        h.memory
            .write_bytes(DESC + 8, &opcode.to_le_bytes())
            .unwrap();
        // OUT is only eight bytes; OUT+4 has four writable bytes, not eight.
        assert!(graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 4).is_err());
        assert_eq!(h.result(), SENTINEL);
    }
    // Destroy did not run: the parent still selects a valid child.
    let selected = h.ok(59, &[d, parent, 0, 0, 1, 0]);
    h.ok(38, &[selected]);
    // No hidden allocation from any of the three failed creation calls:
    // device + root + parent + 4093 ranges exactly fills the object budget.
    let mut ranges = Vec::new();
    for _ in 0..4093 {
        ranges.push(h.ok(57, &[d, root, 0, 1, 0, 1, 0]));
    }
    assert_eq!(h.call(57, &[d, root, 0, 1, 0, 1, 0]), 2);
    for range in ranges {
        h.ok(60, &[range]);
    }
    h.ok(60, &[parent]);
    h.ok(26, &[root]);
}
