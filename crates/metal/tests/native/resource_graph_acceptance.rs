// SPDX-License-Identifier: GPL-3.0-or-later
//! Original bounded graph oracles. No readback is recycled as a graph input.
use super::unaligned_copy::Gate;
use xenora_metal::*;
use xenora_texture::{SamplerDesc, TextureFormat};

const QUEUES: [RenderQueue; 4] = [
    RenderQueue::First,
    RenderQueue::Second,
    RenderQueue::Third,
    RenderQueue::Fourth,
];
const POS: [[f32; 4]; 3] = [
    [-1., -1., 0.25, 1.],
    [3., -1., 0.25, 1.],
    [-1., 3., 0.25, 1.],
];
const STORED: [u8; 8] = [0, 0, 255, 255, 61, 73, 89, 97];
const JOINED: [u8; 8] = [0, 0, 255, 255, 0, 255, 0, 255];
const SHADER: &str = r"#include <metal_stdlib>
using namespace metal;
struct V { float4 p [[position]]; float4 c; };
vertex V vs(uint i [[vertex_id]], device const float4*p [[buffer(0)]], device const float4*c [[buffer(1)]]) { return {p[i],c[i]}; }
fragment float4 color(V v [[stage_in]]) { return v.c; }
fragment float4 green() { return float4(0,1,0,1); }
fragment float4 blue() { return float4(0,0,1,1); }
fragment float4 sampled(V v [[stage_in]], texture2d<float> t [[texture(3)]], sampler s [[sampler(2)]]) { return t.sample(s,v.p.xy/float2(2,1),level(0.0)); }
fragment float4 stored(V v [[stage_in]], texture2d<float,access::write> dst [[texture(0)]], texture2d<float> src [[texture(3)]], sampler s [[sampler(2)]]) {
 float4 c=src.sample(s,v.p.xy/float2(2,1),level(0.0)); if(uint(v.p.x)==0u) dst.write(c,uint2(v.p.xy)); return c;
}";
fn records(v: &[[f32; 4]]) -> Vec<u8> {
    v.iter().flatten().flat_map(|x| x.to_le_bytes()).collect()
}
fn buffer(c: &RenderCohort, p: &RenderPoint) -> RenderBufferOutput {
    let RenderCopyOutput::Buffer(b) = c.copy_output(p).unwrap() else {
        panic!("buffer")
    };
    b
}
fn copied_color(c: &RenderCohort, p: &RenderPoint) -> RenderColorOutput {
    let RenderCopyOutput::Color(b) = c.copy_output(p).unwrap() else {
        panic!("color")
    };
    b
}
fn typed(c: &RenderCohort, p: &RenderPoint) -> RenderTypedTextureOutput {
    let RenderCopyOutput::TypedTexture(b) = c.copy_output(p).unwrap() else {
        panic!("typed")
    };
    b
}
fn view(t: &RenderTypedTextureOutput) -> RenderTypedTextureView {
    t.view(TextureViewDesc {
        base_mip: 0,
        mip_count: 1,
        format: TextureViewFormat::Rgba8Unorm,
    })
    .unwrap()
}
fn sample(gpu: &NativeCompute, t: RenderColorOutput) -> QueuedSampledBinding {
    QueuedSampledBinding::Produced {
        output: t,
        sampler: MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap(),
        texture_slot: BindingSlot::new(3),
        sampler_slot: BindingSlot::new(2),
    }
}
fn sample_typed(gpu: &NativeCompute, t: &RenderTypedTextureOutput) -> QueuedSampledBinding {
    QueuedSampledBinding::ProducedTypedView {
        view: view(t),
        sampler: MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap(),
        texture_slot: BindingSlot::new(3),
        sampler_slot: BindingSlot::new(2),
    }
}
fn geometry(
    p: RenderBufferOutput,
    c: RenderBufferOutput,
    i: RenderBufferOutput,
) -> QueuedGeometry<'static> {
    QueuedGeometry {
        positions: GeometryAttribute {
            buffer: GeometryBuffer::Produced(p),
            offset: 0,
            stride: 16,
        },
        colors: Some(GeometryAttribute {
            buffer: GeometryBuffer::Produced(c),
            offset: 0,
            stride: 16,
        }),
        indices: Some(GeometryIndices {
            buffer: GeometryBuffer::Produced(i),
            offset: 0,
            format: GeometryIndexFormat::R32Uint,
        }),
        start: 0,
        count: 3,
        base_vertex: 0,
    }
}
#[allow(clippy::too_many_arguments)]
fn draw(
    c: &mut RenderCohort,
    queue: RenderQueue,
    entry: &str,
    load: QueuedColorLoad<'_>,
    bindings: Vec<QueuedSampledBinding>,
    geometry: Option<QueuedGeometry<'_>>,
    scissor: Option<[usize; 4]>,
    storage: Option<RenderTypedTextureView>,
) -> Result<RenderPoint, MetalError> {
    let shaders = [TriangleShader {
        source: SHADER,
        entries: ["vs", entry],
    }];
    let draws = [TriangleDraw {
        vertices: if geometry.is_some() { &[] } else { &POS },
        colors: if geometry.is_some() {
            &[]
        } else {
            &[[1., 0., 0., 1.]; 3]
        },
        viewport: [2, 1],
    }];
    let states = [TriangleState {
        scissor,
        ..Default::default()
    }];
    let bindings = [bindings];
    let loads = [load];
    let uniforms = [DrawUniforms::default()];
    let geometry = [geometry];
    if let Some(storage) = storage {
        c.submit_with_storage(
            queue,
            &shaders,
            &TypedTriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [2, 1],
                colors: &[TypedColorAttachment {
                    format: TextureFormat::Rgba8Unorm,
                    clear: TypedColorClear::Float([0.; 4]),
                    blend_enabled: false,
                }],
                clear_depth: None,
            },
            &bindings,
            None,
            &loads,
            &[],
            &uniforms,
            &geometry,
            QueuedStorageInput::ProducedTypedView(storage),
        )
    } else {
        c.submit_with_geometry(
            queue,
            &shaders,
            &TriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [2, 1],
                clears: &[[7, 11, 13, 255]],
                clear_depth: None,
            },
            &bindings,
            None,
            &loads,
            &[],
            &uniforms,
            &geometry,
        )
    }
}
fn texture_copy(
    c: &mut RenderCohort,
    queue: RenderQueue,
    source: TypedCopyInput<'_>,
) -> RenderPoint {
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [2, 1],
        mip_levels: 1,
        array_layers: None,
    };
    let loc = TypedImageCopyLocation::Texture {
        mip: 0,
        slice: 0,
        origin: [0, 0],
    };
    c.submit_typed_image_copy(
        queue,
        QueuedTypedImageCopy {
            source,
            destination: TypedCopyInput::HostTexture {
                desc,
                bytes: &[19; 8],
            },
            source_location: loc,
            destination_location: loc,
            extent: [2, 1],
        },
        &[],
    )
    .unwrap()
}
fn pixels(c: &mut RenderCohort, p: &RenderPoint, expected: &[u8]) {
    match c.collect_copy(p).unwrap() {
        OwnedCopyOutput::Rgba8 { extent, pixels } => {
            assert_eq!(extent, [2, 1]);
            assert_eq!(pixels, expected);
        }
        OwnedCopyOutput::TypedTexture { bytes, .. } => assert_eq!(bytes, expected),
        _ => panic!("texture readback"),
    }
}

#[test]
fn native_resource_graph_sixteen_mixed_nodes_fork_join_and_retained_versions() {
    for (ancestor_first, gated) in [(false, 0), (true, 1), (true, 2), (false, 3), (true, 4)] {
        let gpu = NativeCompute::new().unwrap();
        let mut c = gpu.new_render_graph_cohort().unwrap();
        let mut gate = (gated < 3).then(|| Gate::new(&gpu, &c, QUEUES[gated]));
        let seeds = [
            records(&POS),
            records(&[[1., 0., 0., 1.]; 3]),
            vec![0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
        ];
        let mut p = Vec::new();
        for i in 0..3 {
            p.push(
                c.submit_copy(QUEUES[i], QueuedCopyInput::HostBuffer(&seeds[i]), &[])
                    .unwrap(),
            );
        }
        let desc = TypedTextureDesc {
            format: TextureFormat::Rgba8Unorm,
            extent: [2, 1],
            mip_levels: 1,
            array_layers: None,
        };
        p.push(texture_copy(
            &mut c,
            QUEUES[3],
            TypedCopyInput::HostTexture {
                desc,
                bytes: &[23, 31, 47, 59, 61, 73, 89, 97],
            },
        ));
        let storage = typed(&c, &p[3]);
        let geom = geometry(buffer(&c, &p[0]), buffer(&c, &p[1]), buffer(&c, &p[2]));
        p.push(
            draw(
                &mut c,
                QUEUES[3],
                "color",
                QueuedColorLoad::Clear,
                vec![],
                Some(geom),
                None,
                None,
            )
            .unwrap(),
        );
        // Observe the gather itself before later same-queue work can mask a missing edge.
        if gated < 3 {
            gate.take()
                .unwrap()
                .observe_then_release(&gpu, &c, QUEUES[3]);
        }
        let red = c.color_output(&p[4], 0).unwrap();
        if gated >= 3 {
            gate = Some(Gate::new(&gpu, &c, QUEUES[gated - 3]));
        }
        p.push(
            draw(
                &mut c,
                QUEUES[0],
                "green",
                QueuedColorLoad::Produced(red.clone()),
                vec![],
                None,
                Some([1, 0, 1, 1]),
                None,
            )
            .unwrap(),
        );
        p.push(
            draw(
                &mut c,
                QUEUES[1],
                "blue",
                QueuedColorLoad::Produced(red.clone()),
                vec![],
                None,
                Some([0, 0, 1, 1]),
                None,
            )
            .unwrap(),
        );
        let right = c.color_output(&p[5], 0).unwrap();
        let left = c.color_output(&p[6], 0).unwrap();
        p.push(
            draw(
                &mut c,
                QUEUES[2],
                "sampled",
                QueuedColorLoad::Produced(left),
                vec![sample(&gpu, right)],
                None,
                Some([1, 0, 1, 1]),
                None,
            )
            .unwrap(),
        );
        // Observe the join before p9/p10 introduce unrelated waits on these lanes.
        if gated >= 3 {
            gate.take()
                .unwrap()
                .observe_then_release(&gpu, &c, QUEUES[2]);
        }
        let joined = c.color_output(&p[7], 0).unwrap();
        p.push(
            draw(
                &mut c,
                QUEUES[3],
                "stored",
                QueuedColorLoad::Clear,
                vec![sample(&gpu, joined)],
                None,
                None,
                Some(view(&storage)),
            )
            .unwrap(),
        );
        let stored = c.storage_output(&p[8]).unwrap();
        p.push(texture_copy(
            &mut c,
            QUEUES[0],
            TypedCopyInput::ProducedTexture(stored.clone()),
        ));
        let copied = typed(&c, &p[9]);
        p.push(
            draw(
                &mut c,
                QUEUES[1],
                "sampled",
                QueuedColorLoad::Clear,
                vec![sample_typed(&gpu, &copied)],
                None,
                None,
                None,
            )
            .unwrap(),
        );
        let mut tail = c.color_output(&p[10], 0).unwrap();
        for i in 11..16 {
            p.push(
                c.submit_copy(QUEUES[i % 4], QueuedCopyInput::ProducedColor(tail), &[])
                    .unwrap(),
            );
            tail = copied_color(&c, &p[i]);
        }
        assert_eq!(p.len(), 16);
        for queue in QUEUES {
            assert!(
                c.submit_copy(queue, QueuedCopyInput::ProducedColor(tail.clone()), &[])
                    .is_err()
            );
        }
        let order: Vec<_> = if ancestor_first {
            (0..16).collect()
        } else {
            (0..16).rev().collect()
        };
        for i in order {
            match i {
                0..=2 => {
                    let OwnedCopyOutput::Buffer(bytes) = c.collect_copy(&p[i]).unwrap() else {
                        panic!("buffer")
                    };
                    assert_eq!(bytes, seeds[i]);
                }
                3 => pixels(&mut c, &p[i], &[23, 31, 47, 59, 61, 73, 89, 97]),
                4..=8 | 10 => {
                    let expected: &[u8] = match i {
                        4 => &[255, 0, 0, 255, 255, 0, 0, 255],
                        5 => &[255, 0, 0, 255, 0, 255, 0, 255],
                        6 => &[0, 0, 255, 255, 255, 0, 0, 255],
                        10 => &STORED,
                        _ => &JOINED,
                    };
                    assert_eq!(c.collect(&p[i]).unwrap().colors, vec![expected.to_vec()]);
                }
                _ => pixels(&mut c, &p[i], &STORED),
            }
        }
        drop(p);
        // Reuse retained outputs only after every ancestor receipt was collected.
        let retained = c
            .submit_copy(QUEUES[2], QueuedCopyInput::ProducedColor(red), &[])
            .unwrap();
        pixels(&mut c, &retained, &[255, 0, 0, 255, 255, 0, 0, 255]);
        let retained = texture_copy(&mut c, QUEUES[1], TypedCopyInput::ProducedTexture(stored));
        pixels(&mut c, &retained, &STORED);
    }
}

#[test]
fn native_resource_graph_invalid_geometry_poison_reaches_fourth_queue_and_recovers() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let gate = Gate::new(&gpu, &c, QUEUES[0]);
    let mut roots = Vec::new();
    for bytes in [
        records(&POS),
        records(&[[1., 0., 0., 1.]; 3]),
        vec![0, 0, 0, 0, 1, 0, 0, 0, 99, 0, 0, 0],
    ] {
        roots.push(
            c.submit_copy(QUEUES[0], QueuedCopyInput::HostBuffer(&bytes), &[])
                .unwrap(),
        );
    }
    let old_buffer = buffer(&c, &roots[0]);
    let geom = geometry(
        old_buffer.clone(),
        buffer(&c, &roots[1]),
        buffer(&c, &roots[2]),
    );
    let bad = draw(
        &mut c,
        QUEUES[1],
        "color",
        QueuedColorLoad::Clear,
        vec![],
        Some(geom),
        None,
        None,
    )
    .unwrap();
    let old_color = c.color_output(&bad, 0).unwrap();
    let branch = c
        .submit_copy(
            QUEUES[2],
            QueuedCopyInput::ProducedColor(old_color.clone()),
            &[],
        )
        .unwrap();
    let sampled = copied_color(&c, &branch);
    let leaf = draw(
        &mut c,
        QUEUES[3],
        "sampled",
        QueuedColorLoad::Clear,
        vec![sample(&gpu, sampled)],
        None,
        None,
        None,
    )
    .unwrap();
    gate.observe_then_release(&gpu, &c, QUEUES[3]);
    // The final point must validate its complete ancestry, even when collected first.
    assert!(c.collect(&leaf).is_err());
    assert!(c.collect_copy(&branch).is_err());
    assert!(c.collect(&bad).is_err());
    c.abort_and_drain("expected out-of-range produced index")
        .unwrap();
    c.reinitialize().unwrap();
    let fresh: Vec<_> = QUEUES
        .into_iter()
        .map(|q| {
            c.submit_copy(q, QueuedCopyInput::HostBuffer(&[13, 71, 199]), &[])
                .unwrap()
        })
        .collect();
    roots.extend([bad, branch, leaf]);
    for old in &roots {
        assert!(c.is_complete(old).is_err());
        assert!(c.copy_output(old).is_err());
        assert!(c.color_output(old, 0).is_err());
        for q in QUEUES {
            assert!(
                c.submit_copy(
                    q,
                    QueuedCopyInput::HostBuffer(&[1]),
                    std::slice::from_ref(old)
                )
                .is_err()
            );
        }
    }
    for q in QUEUES {
        assert!(
            c.submit_copy(q, QueuedCopyInput::ProducedBuffer(old_buffer.clone()), &[])
                .is_err()
        );
        assert!(
            c.submit_copy(q, QueuedCopyInput::ProducedColor(old_color.clone()), &[])
                .is_err()
        );
    }
    for p in fresh {
        let OwnedCopyOutput::Buffer(b) = c.collect_copy(&p).unwrap() else {
            panic!("buffer")
        };
        assert_eq!(b, [13, 71, 199]);
    }
}
