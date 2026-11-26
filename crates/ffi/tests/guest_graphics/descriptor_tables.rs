// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE + original four-slot shaders; literal pixels do not use copy/layout helpers.
use super::*;

const RED: [[u8; 4]; 4] = [
    [11, 13, 15, 17],
    [47, 49, 53, 59],
    [113, 127, 131, 137],
    [193, 197, 199, 211],
];
const WORDS: [[u32; 4]; 4] = [
    [0x01000001, 0x01000003, 0x01000005, 0x01000007],
    [0x7fc12345, 0x7fc12347, 0x7fc12349, 0x7fc1234b],
    [0x80000001, 0x80000003, 0x80000005, 0x80000007],
    [0xffffff01, 0xffffff03, 0xffffff05, 0xffffff07],
];
fn update_table(g: &mut GuestScript, table: u64, first: u64, entries: &[(u64, Option<u64>)]) {
    let records = g.bytes(
        &entries
            .iter()
            .flat_map(|(kind, _)| [*kind, 0u64])
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    for (i, (_, handle)) in entries.iter().enumerate() {
        if let Some(handle) = handle {
            g.copy_result(*handle, records + i as u64 * 16 + 8);
        }
    }
    // Revisions are nonzero opaque results; no dependence on initial numbering.
    g.handle(
        71,
        &[H(table), I(first), I(entries.len() as u64), I(records)],
    );
}
fn run_descriptors(dynamic: bool, uint: bool) {
    run_descriptors_produced(dynamic, uint, None);
}
fn run_descriptors_produced(dynamic: bool, uint: bool, producer_first: Option<bool>) {
    run_descriptor_variant(dynamic, uint, producer_first, 0);
}
// Modes: 1 fixed texture, 2 fixed sampler, 3 diagonal, 4 runtime branch,
// 5 selected array range, 6 depth. Existing modes/oracles remain unchanged.
fn run_descriptor_variant(dynamic: bool, uint: bool, producer_first: Option<bool>, mode: u8) {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let table = g.handle(70, &[H(d)]);
    let mut roots = Vec::new();
    let mut views = Vec::new();
    let mut samplers = Vec::new();
    for t in 0..4 {
        let root = g.handle(
            53,
            &[
                H(d),
                I(8),
                I(8),
                I(4),
                I(if mode == 5 { 3 } else { 0 }),
                I(if mode == 6 {
                    9
                } else if uint {
                    8
                } else {
                    0
                }),
                I(COPY_DEST),
            ],
        );
        let mut bytes = Vec::new();
        for (m, pixels) in [64, 16, 4, 1].into_iter().enumerate() {
            let pixel = if mode == 6 {
                [0f32, 0.25, 0.5, 0.75][(t + m) % 4].to_le_bytes()
            } else if uint {
                WORDS[t][m].to_le_bytes()
            } else {
                [RED[t][m], 0, 0, 255]
            };
            bytes.extend_from_slice(&pixel.repeat(pixels));
        }
        if producer_first.is_some() && t == 3 {
            bytes[336..340].copy_from_slice(&[3, 5, 7, 11]);
        }
        if mode == 5 {
            let first = bytes.clone();
            bytes = vec![7; 340]; // Unselected slice0 must never be sampled.
            bytes.extend_from_slice(&first);
            for (m, pixels) in [64, 16, 4, 1].into_iter().enumerate() {
                let pixel = if uint {
                    (WORDS[t][m] ^ 0x00010000).to_le_bytes()
                } else {
                    [255 - RED[t][m], 0, 0, 255]
                };
                bytes.extend_from_slice(&pixel.repeat(pixels));
            }
        }
        let payload = g.bytes(&bytes);
        g.ok(9, &[H(root), I(0), I(payload), I(bytes.len() as u64)]);
        let view = if mode == 5 {
            g.handle(
                57,
                &[
                    H(d),
                    H(root),
                    I(1),
                    I(2),
                    I(0),
                    I(4),
                    I(if uint { 12 } else { 0 }),
                ],
            )
        } else {
            g.handle(
                37,
                &[
                    H(d),
                    H(root),
                    I(0),
                    I(4),
                    I(if mode == 6 {
                        13
                    } else if uint {
                        12
                    } else {
                        0
                    }),
                ],
            )
        };
        let sampler = g.handle(46, &[H(d), I(t as u64), I(0), I(3f32.to_bits() as u64)]);
        if producer_first.is_none() || t != 3 {
            g.barrier(l, root, COPY_DEST, PIXEL_RESOURCE);
        }
        roots.push(root);
        views.push(view);
        samplers.push(sampler);
    }
    let mut records: Vec<_> = views
        .iter()
        .map(|v| (if mode == 5 { 2 } else { 1 }, Some(*v)))
        .collect();
    records.extend(samplers.iter().map(|s| (3, Some(*s))));
    update_table(&mut g, table, 0, &records);
    let vs = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");
    let generated = descriptor_variant_shader(mode);
    let ps: &[u8] = if mode != 0 {
        &generated
    } else if dynamic {
        include_bytes!("../../../../fixtures/rdna2/x9-descriptor-dynamic-ps.bin")
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-descriptor-cross-ps.bin")
    };
    let va = g.bytes(vs);
    let pa = g.bytes(ps);
    let mut types = [0u64; 12];
    if mode == 5 {
        types[..4].fill(if uint { 4 } else { 3 });
    }
    if mode == 6 {
        types[..4].fill(2);
    }
    if uint {
        if mode != 5 {
            types[..4].fill(1);
        }
        types[4] = 1;
    }
    let types = g.bytes(
        &types
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let pipeline = g.handle(
        74,
        &[
            H(d),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
            I(types),
        ],
    );
    let width = if dynamic { 16 } else { 3 };
    let target = g.handle(
        53,
        &[
            H(d),
            I(width),
            I(1),
            I(1),
            I(0),
            I(if uint { 8 } else { 0 }),
            I(RENDER_TARGET),
        ],
    );
    let clear = g.bytes(
        &[0u64, 0, 0, u64::from(uint), 0, 0, 0, 0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(target, clear);
    g.ok(56, &[H(l), I(1), I(clear)]);
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
    g.ok(17, &[H(l), I(width), I(1)]);
    g.ok(14, &[H(l), H(pipeline)]);
    g.ok(18, &[H(l), H(vb), I(0), I(16)]);
    g.ok(19, &[H(l), H(cb), I(0), I(16)]);
    let mut expected = Vec::new();
    for x in 0..width {
        g.ok(73, &[H(l), H(table)]);
        if !dynamic && x < 2 {
            // Update AFTER bind but BEFORE draw: captured A/B must remain immutable.
            let order = if x == 0 { [3, 1, 2, 0] } else { [0, 1, 2, 3] };
            update_table(&mut g, table, 0, &order.map(|i| (1, Some(views[i]))));
        }
        if dynamic {
            let t = x as usize / 4;
            let m = x as usize % 4;
            let uniforms = g.bytes(
                &[
                    t as u32,
                    m as u32,
                    if mode == 5 {
                        ((x as u32 % 2) as f32).to_bits()
                    } else {
                        x as u32 % 2
                    },
                    0,
                    0,
                    0,
                    0,
                    0,
                ]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
            );
            g.ok(41, &[H(l), I(uniforms), I(1)]);
            let (tex, mip) = match mode {
                1 => (3, m),
                2 => (t, 1),
                3 => (t, t),
                4 if x % 2 == 0 => (t, 0),
                4 => (3, m),
                _ => (t, m),
            };
            if mode == 6 {
                expected.extend_from_slice(&[[0, 64, 128, 191][(tex + mip) % 4], 0, 0, 255]);
            } else if uint {
                let word = WORDS[tex][mip]
                    ^ if mode == 5 && x % 2 == 1 {
                        0x00010000
                    } else {
                        0
                    };
                expected.extend_from_slice(&word.to_le_bytes());
            } else {
                let red = if mode == 5 && x % 2 == 1 {
                    255 - RED[tex][mip]
                } else {
                    RED[tex][mip]
                };
                expected.extend_from_slice(&[red, 0, 0, 255]);
            }
        } else {
            expected.extend_from_slice(if x == 1 {
                &[211, 53, 127, 11]
            } else {
                &[17, 53, 127, 193]
            });
        }
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
                I(x),
                I(0),
                I(1),
                I(1),
            ],
        );
        g.ok(23, &[H(l), I(3), I(1), I(0), I(0)]);
    }
    for v in views {
        g.ok(if mode == 5 { 60 } else { 38 }, &[H(v)]);
    }
    for s in samplers {
        g.ok(47, &[H(s)]);
    }
    g.barrier(l, target, RENDER_TARGET, COPY_SOURCE);
    g.ok(12, &[H(l)]);
    let mut producer = None;
    let mut waits = 0;
    if producer_first.is_some() {
        let pq = g.handle(2, &[H(d)]);
        let aa = g.handle(3, &[H(d)]);
        let pl = g.handle(4, &[H(d), H(aa)]);
        let source = g.handle(53, &[H(d), I(1), I(1), I(1), I(0), I(0), I(COPY_DEST)]);
        let bytes = g.bytes(&[211, 0, 0, 255]);
        g.ok(9, &[H(source), I(0), I(bytes), I(4)]);
        g.barrier(pl, source, COPY_DEST, COPY_SOURCE);
        g.ok(
            54,
            &[
                H(pl),
                H(roots[3]),
                I(0),
                I(3),
                I(0),
                I(0),
                H(source),
                I(0),
                I(0),
                I(0),
                I(0),
                I(1),
                I(1),
            ],
        );
        g.barrier(pl, roots[3], COPY_DEST, PIXEL_RESOURCE);
        g.ok(12, &[H(pl)]);
        let r = g.handle(35, &[H(pq), H(pl), I(0), I(0)]);
        let point = g.call(36, &[H(r)], 0, Some(1));
        waits = g.bytes(&[0; 16]);
        g.copy_result(pq, waits);
        g.copy_result(point, waits + 8);
        g.ok(11, &[H(pl), H(aa)]);
        g.ok(26, &[H(source)]);
        producer = Some(r);
    }
    let receipt = g.handle(
        35,
        &[H(q), H(l), I(waits), I(u64::from(producer.is_some()))],
    );
    update_table(&mut g, table, 0, &[(0, None); 8]); // After submit cannot alter pending work.
    g.ok(72, &[H(table)]);
    g.ok(11, &[H(l), H(a)]);
    for root in roots {
        g.ok(26, &[H(root)]);
    }
    for r in [vb, cb] {
        g.ok(26, &[H(r)]);
    }
    g.ok(27, &[H(pipeline)]);
    if producer_first == Some(true) {
        g.call(32, &[H(producer.unwrap())], 0, Some(1));
    }
    g.call(32, &[H(receipt)], 0, Some(1));
    if producer_first == Some(false) {
        g.call(32, &[H(producer.unwrap())], 0, Some(1));
    }
    g.ok(33, &[H(receipt)]);
    if let Some(r) = producer {
        g.ok(33, &[H(r)]);
    }
    let out = g.bytes(&vec![0xa5; expected.len() + 16]);
    g.ok(10, &[H(target), I(0), I(out + 8), I(expected.len() as u64)]);
    check_original_guest_backend(
        g,
        true,
        true,
        &[(out, expected.clone())],
        u64::from(!uint),
        if uint { &[] } else { &expected },
    );
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_four_cross_pairs_recorded_revisions() {
    run_descriptors(false, false);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_runtime_independent_texture_sampler_indices() {
    run_descriptors(true, false);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_runtime_uint_bits() {
    run_descriptors(true, true);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_pending_produced_slot_three_both_collection_orders() {
    for producer_first in [false, true] {
        run_descriptors_produced(true, false, Some(producer_first));
    }
}

// Original words use RDNA2 SOP1/MIMG/SOPP fields, with sample layout also
// independently anchored by the LLVM-assembled x9-descriptor-dynamic-ps fixture.
fn descriptor_variant_shader(mode: u8) -> Vec<u8> {
    if mode == 0 {
        return Vec::new();
    }
    let mut words = vec![
        0xc8080000u32,
        0xc8090001,
        0xc80c0100,
        0xc80d0101,
        0xbebc0318,
        if mode == 3 { 0xbec40318 } else { 0xbec40319 },
    ];
    if mode == 5 {
        words.push(0x7e08021a);
    } // v4=s26, float range-relative layer.
    let sample = |texture: u32, sampler: u32, array: bool| {
        [
            if array { 0xf09c0f28 } else { 0xf09c0f08 },
            0x00000802 | ((texture / 4) << 16) | ((sampler / 4) << 21),
        ]
    };
    if mode == 4 {
        // cmp s26==0; false jumps over first sample+branch to the second arm.
        words.extend_from_slice(&[0xbf06801a, 0xbf840003]);
        words.extend_from_slice(&sample(60, 8, false));
        words.push(0xbf820002);
        words.extend_from_slice(&sample(44, 68, false));
    } else {
        words.extend_from_slice(&sample(
            if mode == 1 { 44 } else { 60 },
            if mode == 2 { 20 } else { 68 },
            mode == 5,
        ));
    }
    words.extend_from_slice(&[0xf800180f, 0x0b0a0908, 0xbf810000]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_one_sided_diagonal_and_branch_selectors() {
    for mode in 1..=4 {
        run_descriptor_variant(true, false, None, mode);
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_dynamic_selected_array_float_and_uint() {
    for uint in [false, true] {
        run_descriptor_variant(true, uint, None, 5);
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_descriptor_tables_dynamic_depth_samples() {
    run_descriptor_variant(true, false, None, 6);
}
