// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE graph. Literal pixel/guard oracles; produced bytes never roundtrip through host inputs.
use super::*;

fn waits(g: &mut GuestScript, deps: &[(u64, u64)]) -> u64 {
    if deps.is_empty() {
        return 0;
    }
    let table = g.bytes(&vec![0; deps.len() * 16]);
    for (i, (queue, receipt)) in deps.iter().enumerate() {
        let point = g.call(36, &[H(*receipt)], 0, None);
        g.copy_result(*queue, table + i as u64 * 16);
        g.copy_result(point, table + i as u64 * 16 + 8);
    }
    table
}
fn pipeline(g: &mut GuestScript, device: u64, storage: bool) -> u64 {
    let vs: &[u8] = if storage {
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin")
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin")
    };
    let ps: &[u8] = if storage {
        include_bytes!("../../../../fixtures/rdna2/x9-storage-branch-ps.bin")
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-view-sample-ps.bin")
    };
    let va = g.bytes(vs);
    let pa = g.bytes(ps);
    if storage {
        g.handle(
            55,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
                I(0x40),
            ],
        )
    } else {
        let types = g.bytes(&[0; 96]);
        g.handle(
            74,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
                I(types),
            ],
        )
    }
}
fn descriptor(g: &mut GuestScript, device: u64, texture: u64, sampler: u64) -> (u64, u64) {
    let view = g.handle(37, &[H(device), H(texture), I(0), I(1), I(0)]);
    let table = g.handle(70, &[H(device)]);
    let mut records = [0u64; 16];
    records[0] = 1;
    records[8] = 3;
    let bytes = g.bytes(
        &records
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(view, bytes + 8);
    g.copy_result(sampler, bytes + 72);
    g.handle(71, &[H(table), I(0), I(8), I(bytes)]);
    (table, view)
}
fn attachment(g: &mut GuestScript, list: u64, view: u64, load: bool) {
    let words = [0, u64::from(load), 0, 0, 0, 0, 0, 1f64.to_bits()];
    let bytes = g.bytes(
        &words
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(view, bytes);
    g.ok(63, &[H(list), I(1), I(bytes)]);
}
fn draw_setup(
    g: &mut GuestScript,
    list: u64,
    pipeline: u64,
    vb: u64,
    cb: u64,
    column: Option<u64>,
) {
    g.ok(17, &[H(list), I(2), I(1)]);
    g.ok(14, &[H(list), H(pipeline)]);
    g.ok(18, &[H(list), H(vb), I(0), I(16)]);
    g.ok(19, &[H(list), H(cb), I(0), I(16)]);
    if let Some(x) = column {
        g.ok(
            22,
            &[
                H(list),
                I(0),
                I(0),
                I(0),
                I(15),
                I(0),
                I(0),
                I(1),
                I(x),
                I(0),
                I(1),
                I(1),
            ],
        );
    }
}
fn texture(g: &mut GuestScript, d: u64, state: u64) -> u64 {
    g.handle(53, &[H(d), I(2), I(1), I(1), I(0), I(0), I(state)])
}
fn guarded(g: &mut GuestScript, bytes: &[u8]) -> u64 {
    let mut v = vec![0xa5; 8];
    v.extend_from_slice(bytes);
    v.extend_from_slice(&[0xa5; 8]);
    g.bytes(&v)
}
fn snapshot(g: &mut GuestScript, address: u64) -> u64 {
    let copy = g.bytes(&[0; 24]);
    for i in [0, 8, 16] {
        g.copy_result(address + i, copy + i);
    }
    copy
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_resource_graph_four_queues_sixteen_receipts_mirrors_and_retirement() {
    for ancestor_first in [true, false] {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let queues: Vec<_> = (0..4).map(|_| g.handle(2, &[H(d)])).collect();
        let alloc: Vec<_> = (0..17).map(|_| g.handle(3, &[H(d)])).collect();
        let lists: Vec<_> = alloc.iter().map(|a| g.handle(4, &[H(d), H(*a)])).collect();
        let mut seed_buffers = Vec::new();
        let mut geometry = Vec::new();
        for (i, data) in [
            floats(&[[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]]),
            floats(&[[0.25, 0.5, 0., 1.]; 3]),
        ]
        .iter()
        .enumerate()
        {
            let seed = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
            let dst = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
            let bytes = g.bytes(data);
            g.ok(9, &[H(seed), I(0), I(bytes), I(48)]);
            g.barrier(lists[i], seed, COPY_DEST, COPY_SOURCE);
            g.ok(42, &[H(lists[i]), H(dst), H(seed)]);
            g.barrier(lists[i], dst, COPY_DEST, VERTEX_BUFFER);
            seed_buffers.push(seed);
            geometry.push(dst);
        }
        let seed = texture(&mut g, d, COPY_DEST);
        let root = texture(&mut g, d, COPY_DEST);
        let original = [17, 53, 127, 193, 29, 71, 149, 211];
        let bytes = g.bytes(&original);
        g.ok(9, &[H(seed), I(0), I(bytes), I(8)]);
        g.barrier(lists[2], seed, COPY_DEST, COPY_SOURCE);
        g.ok(42, &[H(lists[2]), H(root), H(seed)]);
        g.barrier(lists[2], root, COPY_DEST, 8);
        let storage = g.handle(67, &[H(d), H(root), I(0), I(0)]);
        let color_a = texture(&mut g, d, RENDER_TARGET);
        let color_b = texture(&mut g, d, RENDER_TARGET);
        let color_c = texture(&mut g, d, COPY_DEST);
        let a_view = g.handle(61, &[H(d), H(color_a), I(0), I(0), I(0)]);
        let b_view = g.handle(61, &[H(d), H(color_b), I(0), I(0), I(0)]);
        let c_view = g.handle(61, &[H(d), H(color_c), I(0), I(0), I(0)]);
        let store_pipeline = pipeline(&mut g, d, true);
        let sample_pipeline = pipeline(&mut g, d, false);
        attachment(&mut g, lists[3], a_view, false);
        draw_setup(
            &mut g,
            lists[3],
            store_pipeline,
            geometry[0],
            geometry[1],
            Some(0),
        );
        g.ok(69, &[H(lists[3]), H(storage)]);
        let uniforms = g.bytes(
            &[1u32, 0, 0, 0, 0, 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        g.ok(41, &[H(lists[3]), I(uniforms), I(1)]);
        g.ok(23, &[H(lists[3]), I(3), I(1), I(0), I(0)]);
        g.barrier(lists[3], root, 8, COPY_SOURCE);
        let sampled = texture(&mut g, d, COPY_DEST);
        g.ok(42, &[H(lists[4]), H(sampled), H(root)]);
        g.barrier(lists[4], sampled, COPY_DEST, PIXEL_RESOURCE);
        let sampler = g.handle(46, &[H(d), I(0), I(0), I(0)]);
        let (stored_table, stored_view) = descriptor(&mut g, d, sampled, sampler);
        let (a_table, a_sample) = descriptor(&mut g, d, color_a, sampler);
        for (index, target, load, column) in [(5, b_view, false, None), (6, a_view, true, Some(1))]
        {
            attachment(&mut g, lists[index], target, load);
            draw_setup(
                &mut g,
                lists[index],
                sample_pipeline,
                geometry[0],
                geometry[1],
                column,
            );
            g.ok(73, &[H(lists[index]), H(stored_table)]);
            g.ok(23, &[H(lists[index]), I(3), I(1), I(0), I(0)]);
        }
        g.barrier(lists[5], color_b, RENDER_TARGET, COPY_SOURCE);
        g.barrier(lists[6], color_a, RENDER_TARGET, PIXEL_RESOURCE);
        g.ok(42, &[H(lists[7]), H(color_c), H(color_b)]);
        g.barrier(lists[7], color_c, COPY_DEST, RENDER_TARGET);
        attachment(&mut g, lists[8], c_view, true);
        draw_setup(
            &mut g,
            lists[8],
            sample_pipeline,
            geometry[0],
            geometry[1],
            Some(0),
        );
        g.ok(73, &[H(lists[8]), H(a_table)]);
        g.ok(23, &[H(lists[8]), I(3), I(1), I(0), I(0)]);
        g.barrier(lists[8], color_c, RENDER_TARGET, COPY_SOURCE);
        let mut tail = color_c;
        let mut copies = Vec::new();
        for list in &lists[9..] {
            let dst = texture(&mut g, d, COPY_DEST);
            g.ok(42, &[H(*list), H(dst), H(tail)]);
            g.barrier(*list, dst, COPY_DEST, COPY_SOURCE);
            copies.push(dst);
            tail = dst;
        }
        let mirror = guarded(&mut g, &[0x72; 8]);
        g.ok(75, &[H(copies[6]), I(mirror + 8)]);
        let mirror_table = g.bytes(&[0; 8]);
        g.copy_result(copies[6], mirror_table);
        for list in &lists {
            g.ok(12, &[H(*list)]);
        }
        // Replace current tables after every draw captured its revision, before
        // any submission. Sampling must still use the recorded nonempty tables.
        let empty_descriptors = g.bytes(&[0; 128]);
        for table in [stored_table, a_table] {
            g.handle(71, &[H(table), I(0), I(8), I(empty_descriptors)]);
        }
        let mut receipts = Vec::new();
        for i in 0..16 {
            let dependencies: Vec<usize> = match i {
                0..=2 => vec![],
                3 => vec![0, 1, 2],
                4 => vec![3],
                5 | 6 => vec![4],
                7 => vec![5],
                8 => vec![6, 7],
                _ => vec![i - 1],
            };
            let deps: Vec<_> = dependencies
                .iter()
                .map(|j| (queues[j % 4], receipts[*j]))
                .collect();
            let table = waits(&mut g, &deps);
            receipts.push(g.handle(
                35,
                &[
                    H(queues[i % 4]),
                    H(lists[i]),
                    I(table),
                    I(deps.len() as u64),
                ],
            ));
            // Native attachment transactions use multiple stages. Drain native
            // points at these boundaries, retaining all logical graph receipts.
            if i == 3 || i == 8 {
                g.call(32, &[H(receipts[i])], 0, Some(1));
            }
        }
        let last_wait = waits(&mut g, &[(queues[3], receipts[15])]);
        g.call(
            35,
            &[H(queues[0]), H(lists[16]), I(last_wait), I(1)],
            2,
            Some(u64::MAX),
        );
        // Released ancestors remain in the retained logical closure of descendants.
        g.call(32, &[H(receipts[0])], 0, Some(1));
        g.ok(33, &[H(receipts[0])]);
        g.call(
            35,
            &[H(queues[0]), H(lists[16]), I(last_wait), I(1)],
            2,
            Some(u64::MAX),
        );
        g.call(76, &[I(mirror_table), I(1), I(0)], 2, Some(u64::MAX));
        let pending_mirror = snapshot(&mut g, mirror);
        for i in 0..16 {
            g.ok(11, &[H(lists[i]), H(alloc[i])]);
        }
        // Recorded revisions and submitted geometry retain their owners after API retirement.
        for table in [stored_table, a_table] {
            g.ok(72, &[H(table)]);
        }
        for view in [stored_view, a_sample] {
            g.ok(38, &[H(view)]);
        }
        g.ok(47, &[H(sampler)]);
        g.ok(68, &[H(storage)]);
        for view in [a_view, b_view, c_view] {
            g.ok(62, &[H(view)]);
        }
        for resource in seed_buffers
            .into_iter()
            .chain(geometry)
            .chain([seed, sampled])
        {
            g.ok(26, &[H(resource)]);
        }
        for pipeline in [store_pipeline, sample_pipeline] {
            g.ok(27, &[H(pipeline)]);
        }
        let order: Vec<_> = if ancestor_first {
            (1..16).collect()
        } else {
            (1..16).rev().collect()
        };
        for i in &order {
            g.call(32, &[H(receipts[*i])], 0, Some(1));
        }
        g.call(76, &[I(mirror_table), I(1), I(0)], 2, Some(u64::MAX));
        let retained_mirror = snapshot(&mut g, mirror);
        for i in order {
            g.ok(33, &[H(receipts[i])]);
        }
        g.ok(76, &[I(mirror_table), I(1), I(0)]);
        // Pruning all descendants frees logical capacity. Retry the unchanged
        // seventeenth list, consuming published GPU output from the old graph.
        let retry = g.handle(35, &[H(queues[0]), H(lists[16]), I(0), I(0)]);
        g.call(36, &[H(retry)], 0, Some(5));
        g.call(32, &[H(retry)], 0, Some(1));
        g.ok(33, &[H(retry)]);
        let stored = guarded(&mut g, &[0; 8]);
        let final_out = guarded(&mut g, &[0; 8]);
        g.ok(10, &[H(root), I(0), I(stored + 8), I(8)]);
        g.ok(10, &[H(tail), I(0), I(final_out + 8), I(8)]);
        let joined = vec![255, 0, 0, 255, 64, 128, 191, 255];
        check_original_guest_backend(
            g,
            true,
            true,
            &[
                (pending_mirror, vec![0x72; 8]),
                (retained_mirror, vec![0x72; 8]),
                (mirror, joined.clone()),
                (stored, vec![64, 128, 191, 255, 29, 71, 149, 211]),
                (final_out, joined.clone()),
            ],
            // Wait(3) presents node3. Wait(8) traverses [6, 7], so node6
            // presents before node7 collects its older producer node5. The
            // documented no-older-presentation-overwrite rule suppresses node5's
            // frame update, while its resource output still feeds node7's copy.
            // Node8 then presents: three updates in both retirement orders.
            3,
            &joined,
        );
    }
}
