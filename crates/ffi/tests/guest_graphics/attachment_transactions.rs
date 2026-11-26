// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE commands; expected full-root bytes do not use production layouts.
use super::*;
fn colors(g: &mut GuestScript, list: u64, views: &[u64], load: bool) {
    let mut words = Vec::new();
    for (i, _) in views.iter().enumerate() {
        words.extend_from_slice(&[
            0,
            u64::from(load),
            0,
            0,
            if i == 0 { 1f64.to_bits() } else { 0 },
            if i == 1 { 1f64.to_bits() } else { 0 },
            0,
            1f64.to_bits(),
        ]);
    }
    let records = g.bytes(
        &words
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    for (i, v) in views.iter().enumerate() {
        g.copy_result(*v, records + i as u64 * 64);
    }
    g.ok(63, &[H(list), I(views.len() as u64), I(records)]);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_attachment_transactions_same_root_mrt_clear_load_versions() {
    for producer_first in [false, true] {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let queues = [g.handle(2, &[H(d)]), g.handle(2, &[H(d)])];
        let alloc = [g.handle(3, &[H(d)]), g.handle(3, &[H(d)])];
        let lists = [
            g.handle(4, &[H(d), H(alloc[0])]),
            g.handle(4, &[H(d), H(alloc[1])]),
        ];
        let root = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(0), I(COPY_DEST)]);
        let initial = g.bytes(&[17; 144]);
        g.ok(9, &[H(root), I(0), I(initial), I(144)]);
        let views = [
            g.handle(61, &[H(d), H(root), I(1), I(0), I(0)]),
            g.handle(61, &[H(d), H(root), I(1), I(1), I(0)]),
        ];
        g.barrier(lists[0], root, COPY_DEST, RENDER_TARGET);
        colors(&mut g, lists[0], &views, false);
        g.ok(17, &[H(lists[0]), I(2), I(1)]);
        g.ok(12, &[H(lists[0])]);
        let vs = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps = include_bytes!("../../../../fixtures/rdna2/x9-scene-varying-mrt-ps.bin");
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
                I(0),
            ],
        );
        let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
        let cb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
        let pos = g.bytes(&floats(&[
            [-1., -1., 0.5, 1.],
            [3., -1., 0.5, 1.],
            [-1., 3., 0.5, 1.],
        ]));
        let col = g.bytes(&floats(&[[0.25, 0.5, 0.75, 1.]; 3]));
        g.ok(9, &[H(vb), I(0), I(pos), I(48)]);
        g.ok(9, &[H(cb), I(0), I(col), I(48)]);
        g.barrier(lists[1], vb, COPY_DEST, VERTEX_BUFFER);
        g.barrier(lists[1], cb, COPY_DEST, VERTEX_BUFFER);
        colors(&mut g, lists[1], &views, true);
        g.ok(17, &[H(lists[1]), I(2), I(1)]);
        g.ok(14, &[H(lists[1]), H(pipeline)]);
        g.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
        g.ok(19, &[H(lists[1]), H(cb), I(0), I(16)]);
        g.ok(
            22,
            &[
                H(lists[1]),
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
        g.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
        g.barrier(lists[1], root, RENDER_TARGET, COPY_SOURCE);
        g.ok(12, &[H(lists[1])]);
        let a = g.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let point = g.call(36, &[H(a)], 0, Some(1));
        let wait = g.bytes(&[0; 16]);
        g.copy_result(queues[0], wait);
        g.copy_result(point, wait + 8);
        let b = g.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for i in 0..2 {
            g.ok(11, &[H(lists[i]), H(alloc[i])]);
        }
        for view in views {
            g.ok(62, &[H(view)]);
        }
        for r in [vb, cb] {
            g.ok(26, &[H(r)]);
        }
        g.ok(27, &[H(pipeline)]);
        for receipt in if producer_first { [a, b] } else { [b, a] } {
            g.call(32, &[H(receipt)], 0, Some(1));
        }
        g.ok(33, &[H(a)]);
        g.ok(33, &[H(b)]);
        let out = g.bytes(&[0xa5; 160]);
        g.ok(10, &[H(root), I(0), I(out + 8), I(144)]);
        let mut expected = vec![17; 144];
        expected[60..68].copy_from_slice(&[64, 128, 191, 255, 255, 0, 0, 255]);
        expected[132..140].copy_from_slice(&[191, 64, 128, 255, 0, 255, 0, 255]);
        check_original_guest_backend(
            g,
            true,
            true,
            &[(out, expected)],
            2,
            &[64, 128, 191, 255, 255, 0, 0, 255],
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_attachment_transactions_selected_depth_clear_readonly_write() {
    for producer_first in [false, true] {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let queues = [g.handle(2, &[H(d)]), g.handle(2, &[H(d)])];
        let color = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(0), I(COPY_DEST)]);
        let depth = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(9), I(COPY_DEST)]);
        let cbytes = g.bytes(&[17; 144]);
        let zbytes = g.bytes(&[0, 0, 96, 63].repeat(36));
        g.ok(9, &[H(color), I(0), I(cbytes), I(144)]);
        g.ok(9, &[H(depth), I(0), I(zbytes), I(144)]);
        let cv = g.handle(61, &[H(d), H(color), I(1), I(1), I(0)]);
        let zv = g.handle(61, &[H(d), H(depth), I(1), I(1), I(1)]);
        let vs = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin");
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
                I(0),
            ],
        );
        let vb = g.handle(5, &[H(d), I(48 * 5), I(COPY_DEST)]);
        let mut vertices = Vec::new();
        for z in [0.25, 0.5, 0.625, 0.5, 0.625] {
            vertices.extend([[-1., -1., z, 1.], [3., -1., z, 1.], [-1., 3., z, 1.]]);
        }
        let addr = g.bytes(&floats(&vertices));
        g.ok(9, &[H(vb), I(0), I(addr), I(240)]);
        let records = g.bytes(
            &[0u64, 0, 0, 0, 0, 0, 0, 1f64.to_bits()]
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        g.copy_result(cv, records);
        let mut first = None;
        let mut waits = None;
        let mut reads = Vec::new();
        for phase in 0..5 {
            let alloc = g.handle(3, &[H(d)]);
            let list = g.handle(4, &[H(d), H(alloc)]);
            if phase == 0 {
                g.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            }
            for (root, state) in [(color, RENDER_TARGET), (depth, 0x10)] {
                g.barrier(
                    list,
                    root,
                    if phase == 0 { COPY_DEST } else { COPY_SOURCE },
                    state,
                );
            }
            g.ok(63, &[H(list), I(1), I(records)]);
            g.ok(
                64,
                &[
                    H(list),
                    H(zv),
                    I(u64::from(phase != 0)),
                    I(if phase == 0 {
                        u64::from(0.75f32.to_bits())
                    } else {
                        0
                    }),
                ],
            );
            g.ok(14, &[H(list), H(pipeline)]);
            g.ok(17, &[H(list), I(2), I(1)]);
            g.ok(18, &[H(list), H(vb), I(phase * 48), I(16)]);
            g.ok(
                22,
                &[
                    H(list),
                    I(2),
                    I(u64::from(phase == 0 || phase == 3)),
                    I(0),
                    I(15),
                    I(0),
                    I(0),
                    I(u64::from(phase == 0)),
                    I(0),
                    I(0),
                    I(u64::from(phase == 0)),
                    I(u64::from(phase == 0)),
                ],
            );
            g.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            g.barrier(list, color, RENDER_TARGET, COPY_SOURCE);
            g.barrier(list, depth, 0x10, COPY_SOURCE);
            g.ok(12, &[H(list)]);
            let receipt = if phase == 1 {
                g.handle(35, &[H(queues[1]), H(list), I(waits.unwrap()), I(1)])
            } else {
                g.handle(35, &[H(queues[0]), H(list), I(0), I(0)])
            };
            g.ok(11, &[H(list), H(alloc)]);
            if phase == 0 {
                let point = g.call(36, &[H(receipt)], 0, Some(1));
                let wait = g.bytes(&[0; 16]);
                g.copy_result(queues[0], wait);
                g.copy_result(point, wait + 8);
                first = Some(receipt);
                waits = Some(wait);
                continue;
            }
            if phase == 1 {
                for r in if producer_first {
                    [first.unwrap(), receipt]
                } else {
                    [receipt, first.unwrap()]
                } {
                    g.call(32, &[H(r)], 0, Some(1));
                }
                g.ok(33, &[H(first.unwrap())]);
            } else {
                g.call(32, &[H(receipt)], 0, Some(1));
            }
            g.ok(33, &[H(receipt)]);
            let pixel = if phase == 4 {
                [0, 0, 0, 255, 0, 0, 0, 255]
            } else {
                [0, 0, 0, 255, 255, 0, 0, 255]
            };
            let mut expected = vec![17; 144];
            expected[132..140].copy_from_slice(&pixel);
            let out = g.bytes(&[0xa5; 160]);
            g.ok(10, &[H(color), I(0), I(out + 8), I(144)]);
            reads.push((out, expected));
            let mut zexpected = [0, 0, 96, 63].repeat(36);
            zexpected[132..136].copy_from_slice(&[0, 0, 128, 62]);
            zexpected[136..140].copy_from_slice(&if phase < 3 {
                [0, 0, 64, 63]
            } else {
                [0, 0, 0, 63]
            });
            let out = g.bytes(&[0xa5; 160]);
            g.ok(10, &[H(depth), I(0), I(out + 8), I(144)]);
            reads.push((out, zexpected));
        }
        for view in [cv, zv] {
            g.ok(62, &[H(view)]);
        }
        for r in [vb, color, depth] {
            g.ok(26, &[H(r)]);
        }
        g.ok(27, &[H(pipeline)]);
        check_original_guest_backend(g, true, true, &reads, 5, &[0, 0, 0, 255, 0, 0, 0, 255]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_attachment_transactions_fresh_depth_clear_then_load() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let color = g.handle(6, &[H(d), I(2), I(1), I(1), I(RENDER_TARGET)]);
    let depth = g.handle(7, &[H(d), I(2), I(1)]);
    let cv = g.handle(61, &[H(d), H(color), I(0), I(0), I(0)]);
    let zv = g.handle(61, &[H(d), H(depth), I(0), I(0), I(1)]);
    let record = g.bytes(
        &[0u64, 0, 0, 0, 0, 0, 0, 1f64.to_bits()]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(cv, record);
    g.ok(63, &[H(l), I(1), I(record)]);
    g.ok(64, &[H(l), H(zv), I(0), I(u64::from(0.25f32.to_bits()))]);
    g.ok(17, &[H(l), I(2), I(1)]);
    g.ok(12, &[H(l)]);
    let first = g.handle(35, &[H(q), H(l), I(0), I(0)]);
    g.call(32, &[H(first)], 0, Some(1));
    g.ok(33, &[H(first)]);
    g.ok(11, &[H(l), H(a)]);
    let vs = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin");
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
            I(0),
        ],
    );
    let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
    let pos = g.bytes(&floats(&[
        [-1., -1., 0.5, 1.],
        [3., -1., 0.5, 1.],
        [-1., 3., 0.5, 1.],
    ]));
    g.ok(9, &[H(vb), I(0), I(pos), I(48)]);
    g.barrier(l, vb, COPY_DEST, VERTEX_BUFFER);
    g.ok(63, &[H(l), I(1), I(record)]);
    g.ok(64, &[H(l), H(zv), I(1), I(0)]);
    g.ok(17, &[H(l), I(2), I(1)]);
    g.ok(14, &[H(l), H(pipeline)]);
    g.ok(18, &[H(l), H(vb), I(0), I(16)]);
    g.ok(
        22,
        &[
            H(l),
            I(2),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
        ],
    );
    g.ok(23, &[H(l), I(3), I(1), I(0), I(0)]);
    g.barrier(l, color, RENDER_TARGET, COPY_SOURCE);
    g.barrier(l, depth, 0x10, COPY_SOURCE);
    g.ok(12, &[H(l)]);
    let second = g.handle(35, &[H(q), H(l), I(0), I(0)]);
    g.call(32, &[H(second)], 0, Some(1));
    g.ok(33, &[H(second)]);
    let cout = g.bytes(&[0xa5; 24]);
    let zout = g.bytes(&[0xa5; 24]);
    g.ok(10, &[H(color), I(0), I(cout + 8), I(8)]);
    g.ok(10, &[H(depth), I(0), I(zout + 8), I(8)]);
    let pixels = vec![0, 0, 0, 255, 0, 0, 0, 255];
    check_original_guest_backend(
        g,
        true,
        true,
        &[
            (cout, pixels.clone()),
            (zout, vec![0, 0, 128, 62, 0, 0, 128, 62]),
        ],
        2,
        &pixels,
    );
}
