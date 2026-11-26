// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE calls with literal full-root composition oracles.
use super::*;

fn bind_colors(g: &mut GuestScript, list: u64, views: &[u64], clears: &[[f64; 4]], load: bool) {
    let mut words = Vec::new();
    for clear in clears {
        words.extend_from_slice(&[0, u64::from(load), 0, 0]);
        words.extend(clear.map(f64::to_bits));
    }
    let address = g.bytes(
        &words
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    for (i, view) in views.iter().enumerate() {
        g.copy_result(*view, address + i as u64 * 64);
    }
    g.ok(63, &[H(list), I(views.len() as u64), I(address)]);
}
fn table(g: &mut GuestScript, queue: u64, receipt: u64) -> u64 {
    let point = g.call(36, &[H(receipt)], 0, None);
    let address = g.bytes(&[0; 16]);
    g.copy_result(queue, address);
    g.copy_result(point, address + 8);
    address
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_attachment_composition_disjoint_versions_load_and_retirement() {
    for ancestor_first in [false, true] {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let queues = [g.handle(2, &[H(d)]), g.handle(2, &[H(d)])];
        let mut allocators = Vec::new();
        let mut lists = Vec::new();
        for _ in 0..3 {
            let a = g.handle(3, &[H(d)]);
            allocators.push(a);
            lists.push(g.handle(4, &[H(d), H(a)]));
        }
        let root = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(0), I(COPY_DEST)]);
        let prior = g.bytes(&[17; 144]);
        g.ok(9, &[H(root), I(0), I(prior), I(144)]);
        let views = [
            g.handle(61, &[H(d), H(root), I(1), I(0), I(0)]),
            g.handle(61, &[H(d), H(root), I(1), I(1), I(0)]),
        ];
        let sampled = g.handle(6, &[H(d), I(2), I(1), I(1), I(COPY_DEST)]);
        let sampled_target = g.handle(61, &[H(d), H(sampled), I(0), I(0), I(0)]);
        let sampled_view = g.handle(37, &[H(d), H(sampled), I(0), I(1), I(0)]);
        let vs = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps = include_bytes!("../../../../fixtures/rdna2/x9-view-sample-ps.bin");
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
        let vertices = g.bytes(&floats(&[
            [-1., -1., 0.5, 1.],
            [3., -1., 0.5, 1.],
            [-1., 3., 0.5, 1.],
        ]));
        g.ok(9, &[H(vb), I(0), I(vertices), I(48)]);
        let cb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
        let coordinates = g.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
        g.ok(9, &[H(cb), I(0), I(coordinates), I(48)]);
        g.barrier(lists[0], cb, COPY_DEST, VERTEX_BUFFER);
        g.barrier(lists[0], sampled, COPY_DEST, RENDER_TARGET);
        g.barrier(lists[0], vb, COPY_DEST, VERTEX_BUFFER);
        g.barrier(lists[0], root, COPY_DEST, RENDER_TARGET);
        bind_colors(
            &mut g,
            lists[0],
            &[views[0], views[1], sampled_target],
            &[[1., 0., 0., 1.], [0., 1., 0., 1.], [1., 1., 0., 1.]],
            false,
        );
        g.ok(17, &[H(lists[0]), I(2), I(1)]);
        g.barrier(lists[0], sampled, RENDER_TARGET, PIXEL_RESOURCE);
        g.ok(12, &[H(lists[0])]);
        g.barrier(lists[1], sampled, PIXEL_RESOURCE, RENDER_TARGET);
        bind_colors(
            &mut g,
            lists[1],
            &[views[0], sampled_target],
            &[[0., 0., 1., 1.], [1., 0., 1., 1.]],
            false,
        );
        g.ok(17, &[H(lists[1]), I(2), I(1)]);
        g.barrier(lists[1], sampled, RENDER_TARGET, PIXEL_RESOURCE);
        g.ok(12, &[H(lists[1])]);
        bind_colors(&mut g, lists[2], &views[1..], &[[0., 0., 0., 1.]], true);
        g.ok(17, &[H(lists[2]), I(2), I(1)]);
        g.ok(14, &[H(lists[2]), H(pipeline)]);
        g.ok(18, &[H(lists[2]), H(vb), I(0), I(16)]);
        g.ok(19, &[H(lists[2]), H(cb), I(0), I(16)]);
        let bindings = sampled_table(&mut g, [Some(sampled_view), None], [Some(0), None], false);
        g.ok(39, &[H(lists[2]), I(bindings)]);
        g.ok(
            22,
            &[
                H(lists[2]),
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
        g.ok(23, &[H(lists[2]), I(3), I(1), I(0), I(0)]);
        g.ok(12, &[H(lists[2])]);
        let a = g.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let sem = table(&mut g, queues[0], a);
        let b = g.handle(35, &[H(queues[1]), H(lists[1]), I(sem), I(1)]);
        let comp = table(&mut g, queues[1], b);
        // Ordering alone cannot authorize preservation of B's disjoint update.
        g.call(
            65,
            &[H(queues[1]), H(lists[2]), I(sem), I(1), I(0), I(0)],
            2,
            None,
        );
        // Same queue as B: execution predecessor must not silently replace semantic A.
        let c = g.handle(
            65,
            &[H(queues[1]), H(lists[2]), I(sem), I(1), I(comp), I(1)],
        );
        for i in 0..3 {
            g.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        for view in views {
            g.ok(62, &[H(view)]);
        }
        g.ok(62, &[H(sampled_target)]);
        g.ok(38, &[H(sampled_view)]);
        g.ok(26, &[H(sampled)]);
        g.ok(26, &[H(cb)]);
        g.ok(26, &[H(vb)]);
        g.ok(27, &[H(pipeline)]);
        for receipt in if ancestor_first { [a, b, c] } else { [c, b, a] } {
            g.call(32, &[H(receipt)], 0, Some(1));
        }
        for receipt in [a, b, c] {
            g.ok(33, &[H(receipt)]);
        }
        // Readback transition occurs only after composition; no whole-root barrier in B/C.
        g.barrier(lists[0], root, RENDER_TARGET, COPY_SOURCE);
        g.ok(12, &[H(lists[0])]);
        g.ok(25, &[H(queues[0]), H(lists[0])]);
        let out = g.bytes(&[0xa5; 160]);
        g.ok(10, &[H(root), I(0), I(out + 8), I(144)]);
        let mut expected = vec![17; 144];
        expected[60..68].copy_from_slice(&[0, 0, 255, 255, 0, 0, 255, 255]);
        expected[132..140].copy_from_slice(&[255, 255, 0, 255, 0, 255, 0, 255]);
        check_original_guest_backend(
            g,
            true,
            true,
            &[(out, expected)],
            // The final queued barrier-only Execute renders its default black pass.
            4,
            &[0, 0, 0, 255].repeat(12),
        );
    }
}
