// SPDX-License-Identifier: GPL-3.0-or-later
//! Original DIM5 shader and PE commands; literal mip/layer/texel oracles.
use super::*;

fn array_pixels(uint: bool) -> Vec<u8> {
    if uint {
        [
            0x0100_0001u32,
            0x7fc1_2345,
            0xffff_ffff,
            0x8000_0001,
            0x0123_4567,
            0xdead_beef,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
    } else {
        vec![
            11, 23, 47, 255, 59, 71, 83, 255, 97, 109, 127, 255, 139, 151, 163, 255, 179, 191, 211,
            255, 223, 227, 239, 255,
        ]
    }
}
fn run_array_sampling(one_layer: bool, uint: bool, produced: bool, slot1: bool) {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let list = g.handle(4, &[H(d), H(a)]);
    let layer_count = if one_layer { 1 } else { 5 };
    let selected = if one_layer { 1 } else { 3 };
    let pixels = array_pixels(uint);
    let expected = pixels[..selected * 8].to_vec();
    let mut packed = vec![17; layer_count * 72];
    for i in 0..selected {
        let offset = (if one_layer { 0 } else { i + 2 }) * 72 + 60;
        packed[offset..offset + 8].copy_from_slice(&pixels[i * 8..i * 8 + 8]);
    }
    let format = if uint { 8 } else { 0 };
    let root = g.handle(
        53,
        &[
            H(d),
            I(5),
            I(3),
            I(3),
            I(layer_count as u64),
            I(format),
            I(COPY_DEST),
        ],
    );
    let mut initial = packed.clone();
    if produced {
        initial[276..284].fill(0x39);
    }
    let data = g.bytes(&initial);
    g.ok(9, &[H(root), I(0), I(data), I(initial.len() as u64)]);
    let range = if one_layer {
        g.handle(
            57,
            &[
                H(d),
                H(root),
                I(0),
                I(1),
                I(1),
                I(2),
                I(if uint { 12 } else { 0 }),
            ],
        )
    } else {
        let parent = g.handle(
            57,
            &[
                H(d),
                H(root),
                I(1),
                I(4),
                I(0),
                I(3),
                I(if uint { 12 } else { 0 }),
            ],
        );
        let child = g.handle(
            58,
            &[
                H(d),
                H(parent),
                I(1),
                I(3),
                I(1),
                I(2),
                I(if uint { 12 } else { 0 }),
            ],
        );
        g.ok(60, &[H(parent)]);
        child
    };
    g.call(26, &[H(root)], 2, None); // Live child independently retains the full root.
    let vs = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps: &[u8] = if slot1 {
        include_bytes!("../../../../fixtures/rdna2/x9-array-slot1-ps.bin")
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-array-uniform-ps.bin")
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
            I(if slot1 {
                if uint { 0x124 } else { 0x20 }
            } else if uint {
                0x111
            } else {
                0x10
            }),
        ],
    );
    let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
    let cb = g.handle(5, &[H(d), I(96), I(COPY_DEST)]);
    let pos = g.bytes(&floats(&[
        [-1., -1., 0.5, 1.],
        [3., -1., 0.5, 1.],
        [-1., 3., 0.5, 1.],
    ]));
    let uv = g.bytes(&floats(
        &[[0.25, 0.5, 0., 1.]; 3]
            .into_iter()
            .chain([[0.75, 0.5, 0., 1.]; 3])
            .collect::<Vec<_>>(),
    ));
    g.ok(9, &[H(vb), I(0), I(pos), I(48)]);
    g.ok(9, &[H(cb), I(0), I(uv), I(96)]);
    let target = g.handle(
        53,
        &[
            H(d),
            I((selected * 2) as u64),
            I(1),
            I(1),
            I(0),
            I(format),
            I(RENDER_TARGET),
        ],
    );
    let records = g.bytes(
        &[0u64, 0, 0, u64::from(uint), 0, 0, 0, 0]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(target, records);
    g.ok(56, &[H(list), I(1), I(records)]);
    g.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
    g.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
    if !produced {
        g.barrier(list, root, COPY_DEST, PIXEL_RESOURCE);
    }
    g.ok(17, &[H(list), I((selected * 2) as u64), I(1)]);
    g.ok(14, &[H(list), H(pipeline)]);
    g.ok(18, &[H(list), H(vb), I(0), I(16)]);
    let bindings = if slot1 {
        sampled_table(&mut g, [None, Some(range)], [None, Some(0)], false)
    } else {
        sampled_table(&mut g, [Some(range), None], [Some(0), None], false)
    };
    g.ok(66, &[H(list), I(bindings)]);
    for x in 0..selected * 2 {
        g.ok(19, &[H(list), H(cb), I((x % 2 * 48) as u64), I(16)]);
        let mut words = [0u32; 8];
        words[0] = ((x / 2) as f32).to_bits();
        let uniforms = g.bytes(
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        g.ok(41, &[H(list), I(uniforms), I(1)]);
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
                I(x as u64),
                I(0),
                I(1),
                I(1),
            ],
        );
        g.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
    }
    g.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
    g.ok(12, &[H(list)]);
    let mut predecessor = None;
    let mut wait = 0;
    if produced {
        let pq = g.handle(2, &[H(d)]);
        let alloc = g.handle(3, &[H(d)]);
        let pl = g.handle(4, &[H(d), H(alloc)]);
        let source = g.handle(53, &[H(d), I(5), I(3), I(3), I(5), I(format), I(COPY_DEST)]);
        let bytes = g.bytes(&packed);
        g.ok(9, &[H(source), I(0), I(bytes), I(360)]);
        g.barrier(pl, source, COPY_DEST, COPY_SOURCE);
        g.ok(
            54,
            &[
                H(pl),
                H(root),
                I(0),
                I((3u64 << 32) | 1),
                I(0),
                I(0),
                H(source),
                I(0),
                I((3u64 << 32) | 1),
                I(0),
                I(0),
                I(2),
                I(1),
            ],
        );
        g.barrier(pl, root, COPY_DEST, PIXEL_RESOURCE);
        g.ok(12, &[H(pl)]);
        let receipt = g.handle(35, &[H(pq), H(pl), I(0), I(0)]);
        let point = g.call(36, &[H(receipt)], 0, Some(1));
        wait = g.bytes(&[0; 16]);
        g.copy_result(pq, wait);
        g.copy_result(point, wait + 8);
        g.ok(11, &[H(pl), H(alloc)]);
        g.ok(26, &[H(source)]);
        predecessor = Some(receipt);
    }
    let receipt = g.handle(35, &[H(q), H(list), I(wait), I(u64::from(produced))]);
    g.ok(11, &[H(list), H(a)]);
    g.ok(60, &[H(range)]);
    for resource in [root, vb, cb] {
        g.ok(26, &[H(resource)]);
    }
    g.ok(27, &[H(pipeline)]);
    g.call(32, &[H(receipt)], 0, Some(1));
    if let Some(p) = predecessor {
        g.call(32, &[H(p)], 0, Some(1));
        g.ok(33, &[H(p)]);
    }
    g.ok(33, &[H(receipt)]);
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
fn original_guest_array_sampling_nested_uniform_layers_and_one_layer_identity() {
    run_array_sampling(false, false, false, false);
    run_array_sampling(true, false, false, false);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_sampling_uint_bits_and_pending_copy_version() {
    run_array_sampling(false, true, false, false);
    run_array_sampling(false, false, true, false);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_sampling_second_texture_and_sampler_slots() {
    run_array_sampling(false, false, false, true);
    run_array_sampling(false, true, false, true);
}
