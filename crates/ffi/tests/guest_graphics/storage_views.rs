// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE and original shaders; full-root literals independent of layout helpers.
use super::*;
fn storage_prior() -> Vec<u8> {
    let mut b = vec![0x59; 144];
    b[132..140].copy_from_slice(&[17, 53, 127, 193, 29, 71, 149, 211]);
    b
}
fn run_storage(ps: &[u8], count: u32, uint: bool, produced: bool, stored: Option<[u8; 4]>) {
    eprintln!(
        "storage guest case: count={count} ({count:#010x}), uint={uint}, produced={produced}"
    );
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let format = if uint { 8 } else { 0 };
    let root = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(format), I(COPY_DEST)]);
    let original = storage_prior();
    let mut host = original.clone();
    if produced {
        host[136..140].copy_from_slice(&[3, 5, 7, 11]);
    }
    let bytes = g.bytes(&host);
    g.ok(9, &[H(root), I(0), I(bytes), I(144)]);
    let storage = g.handle(67, &[H(d), H(root), I(1), I(1)]);
    g.call(26, &[H(root)], 2, None);
    let color = g.handle(
        53,
        &[H(d), I(2), I(1), I(1), I(0), I(format), I(RENDER_TARGET)],
    );
    let cv = g.handle(61, &[H(d), H(color), I(0), I(0), I(0)]);
    let records = g.bytes(
        &[
            0u64,
            0,
            0,
            u64::from(uint),
            0,
            0,
            0,
            if uint { 0 } else { 1f64.to_bits() },
        ]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect::<Vec<_>>(),
    );
    g.copy_result(cv, records);
    g.ok(63, &[H(l), I(1), I(records)]);
    let vs: &[u8] = if uint {
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin")
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin")
    };
    let va = g.bytes(vs);
    let pa = g.bytes(ps);
    let pipeline = g.handle(
        55,
        &[
            H(d),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
            I(if uint { 0x145 } else { 0x40 }),
        ],
    );
    let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
    let cb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
    let pos = g.bytes(&floats(&[
        [-1., -1., 0.5, 1.],
        [3., -1., 0.5, 1.],
        [-1., 3., 0.5, 1.],
    ]));
    let uv = g.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
    g.ok(9, &[H(vb), I(0), I(pos), I(48)]);
    g.ok(9, &[H(cb), I(0), I(uv), I(48)]);
    g.barrier(l, vb, COPY_DEST, VERTEX_BUFFER);
    g.barrier(l, cb, COPY_DEST, VERTEX_BUFFER);
    if !produced {
        g.barrier(l, root, COPY_DEST, 0x8);
    }
    g.ok(69, &[H(l), H(storage)]);
    g.ok(17, &[H(l), I(2), I(1)]);
    g.ok(14, &[H(l), H(pipeline)]);
    g.ok(18, &[H(l), H(vb), I(0), I(16)]);
    g.ok(19, &[H(l), H(cb), I(0), I(16)]);
    let mut sampled = None;
    if uint {
        let source = g.handle(53, &[H(d), I(1), I(1), I(1), I(0), I(8), I(COPY_DEST)]);
        let word = g.bytes(&[1, 0, 0, 1]);
        g.ok(9, &[H(source), I(0), I(word), I(4)]);
        g.barrier(l, source, COPY_DEST, PIXEL_RESOURCE);
        let view = g.handle(37, &[H(d), H(source), I(0), I(1), I(12)]);
        let table = sampled_table(&mut g, [None, Some(view)], [None, Some(0)], false);
        g.ok(39, &[H(l), I(table)]);
        sampled = Some((source, view));
    }
    let uniforms = g.bytes(
        &[count, 0, 0, 0, 0, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.ok(41, &[H(l), I(uniforms), I(1)]);
    if produced {
        g.ok(
            22,
            &[
                H(l),
                I(0),
                I(0),
                I(0),
                I(15),
                I(0),
                I(0),
                I(1),
                I(0),
                I(0),
                I(1),
                I(1),
            ],
        );
    }
    g.ok(23, &[H(l), I(3), I(1), I(0), I(0)]);
    g.barrier(l, root, 0x8, COPY_SOURCE);
    g.barrier(l, color, RENDER_TARGET, COPY_SOURCE);
    g.ok(12, &[H(l)]);
    let mut producer = None;
    let mut wait = 0;
    if produced {
        let pq = g.handle(2, &[H(d)]);
        let alloc = g.handle(3, &[H(d)]);
        let pl = g.handle(4, &[H(d), H(alloc)]);
        let source = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(format), I(COPY_DEST)]);
        let bytes = g.bytes(&original);
        g.ok(9, &[H(source), I(0), I(bytes), I(144)]);
        g.barrier(pl, source, COPY_DEST, COPY_SOURCE);
        g.ok(
            54,
            &[
                H(pl),
                H(root),
                I(0),
                I((1u64 << 32) | 1),
                I(0),
                I(0),
                H(source),
                I(0),
                I((1u64 << 32) | 1),
                I(0),
                I(0),
                I(2),
                I(1),
            ],
        );
        g.barrier(pl, root, COPY_DEST, 0x8);
        g.ok(12, &[H(pl)]);
        let receipt = g.handle(35, &[H(pq), H(pl), I(0), I(0)]);
        let point = g.call(36, &[H(receipt)], 0, Some(1));
        wait = g.bytes(&[0; 16]);
        g.copy_result(pq, wait);
        g.copy_result(point, wait + 8);
        g.ok(11, &[H(pl), H(alloc)]);
        g.ok(26, &[H(source)]);
        producer = Some(receipt);
    }
    let receipt = g.handle(35, &[H(q), H(l), I(wait), I(u64::from(produced))]);
    g.ok(11, &[H(l), H(a)]);
    g.ok(68, &[H(storage)]);
    g.ok(62, &[H(cv)]);
    if let Some((source, view)) = sampled {
        g.ok(38, &[H(view)]);
        g.ok(26, &[H(source)]);
    }
    for r in [vb, cb] {
        g.ok(26, &[H(r)]);
    }
    g.ok(27, &[H(pipeline)]);
    g.call(32, &[H(receipt)], 0, Some(1));
    if let Some(p) = producer {
        g.call(32, &[H(p)], 0, Some(1));
        g.ok(33, &[H(p)]);
    }
    g.ok(33, &[H(receipt)]);
    let mut expected = original;
    if let Some(pixel) = stored {
        expected[132..136].copy_from_slice(&pixel);
        if !produced {
            expected[136..140].copy_from_slice(&pixel);
        }
    }
    let c = if uint { [1, 0, 0, 1] } else { [255, 0, 0, 255] };
    let mut colors = c.repeat(2);
    if produced {
        colors[4..8].copy_from_slice(if uint { &[0, 0, 0, 0] } else { &[0, 0, 0, 255] });
    }
    let out = g.bytes(&[0xa5; 160]);
    let cout = g.bytes(&[0xa5; 24]);
    g.ok(10, &[H(root), I(0), I(out + 8), I(144)]);
    g.ok(10, &[H(color), I(0), I(cout + 8), I(8)]);
    check_original_guest_backend(
        g,
        true,
        true,
        &[(out, expected), (cout, colors.clone())],
        u64::from(!uint),
        if uint { &[] } else { &colors },
    );
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_storage_views_branch_preserves_full_root_and_writes_literals() {
    let ps = include_bytes!("../../../../fixtures/rdna2/x9-storage-branch-ps.bin");
    run_storage(ps, 0, false, false, None);
    run_storage(ps, 1, false, false, Some([64, 128, 191, 255]));
    run_storage(ps, 0, false, true, None);
    run_storage(ps, 1, false, true, Some([64, 128, 191, 255]));
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_storage_views_loop_zero_and_ordered_repeated_stores() {
    let ps = include_bytes!("../../../../fixtures/rdna2/x9-storage-loop-ps.bin");
    for (n, pixel) in [
        (0, None),
        (1, Some([64, 128, 191, 255])),
        (3, Some([191, 128, 191, 255])),
        (4, Some([255, 128, 191, 255])),
        (u32::MAX, Some([255, 128, 191, 255])),
    ] {
        run_storage(ps, n, false, false, pixel);
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_storage_views_uint_sample_bits_are_not_float_converted() {
    let ps = include_bytes!("../../../../fixtures/rdna2/x9-storage-uint-sampled-ps.bin");
    run_storage(ps, 0, true, false, None);
    run_storage(ps, 1, true, false, Some([1, 0, 0, 1]));
}
