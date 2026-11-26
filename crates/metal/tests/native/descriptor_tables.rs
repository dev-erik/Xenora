// SPDX-License-Identifier: GPL-3.0-or-later
//! Original four-slot shaders and literal pixels, not a guest descriptor implementation.
use super::unaligned_copy::Gate;
use xenora_metal::*;
use xenora_texture::{SamplerDesc, TextureFormat};
const POS: [[f32; 4]; 3] = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
const SHADER: &str = r"#include <metal_stdlib>
using namespace metal;
vertex float4 vs(uint i [[vertex_id]],device const float4*p [[buffer(0)]]){return p[i];}
fragment float4 cross(texture2d<float>a [[texture(0)]],texture2d<float>b [[texture(1)]],texture2d<float>c [[texture(2)]],texture2d<float>d [[texture(3)]],sampler s0 [[sampler(0)]],sampler s1 [[sampler(1)]],sampler s2 [[sampler(2)]],sampler s3 [[sampler(3)]]){
return float4(a.sample(s3,float2(.5),level(0.0)).r,b.sample(s2,float2(.5),level(0.0)).g,c.sample(s1,float2(.5),level(0.0)).b,d.sample(s0,float2(.5),level(0.0)).r);}
fragment float4 nulls(texture2d<float>a [[texture(0)]],texture2d<float>b [[texture(1)]],texture2d<float>c [[texture(2)]],texture2d<float>d [[texture(3)]]){return float4(is_null_texture(a),is_null_texture(b),is_null_texture(c),is_null_texture(d));}
fragment float4 third(texture2d<float>t [[texture(3)]],sampler s [[sampler(3)]]){return t.sample(s,float2(.5),level(0.0));}
fragment float4 plain(){return float4(1,0,0,1);}";
fn resources(gpu: &NativeCompute) -> ([MetalRgba8Texture; 4], [MetalSampler; 4]) {
    let textures = [
        [
            [11, 13, 17, 255],
            [21, 23, 27, 255],
            [31, 33, 37, 255],
            [41, 43, 47, 255],
        ],
        [
            [19, 23, 29, 255],
            [49, 53, 59, 255],
            [79, 83, 89, 255],
            [99, 103, 109, 255],
        ],
        [
            [31, 33, 37, 255],
            [113, 119, 127, 255],
            [133, 139, 147, 255],
            [153, 159, 167, 255],
        ],
        [
            [173, 179, 181, 255],
            [183, 189, 191, 255],
            [193, 199, 201, 255],
            [203, 209, 211, 255],
        ],
    ]
    .map(|texels| {
        let bytes: Vec<_> = texels
            .into_iter()
            .enumerate()
            .map(|(level, texel)| {
                let size = 8usize >> level;
                texel.repeat(size * size)
            })
            .collect();
        let levels: Vec<_> = bytes
            .iter()
            .enumerate()
            .map(|(i, b)| Rgba8MipLevel::new_packed(i as u32, 8 >> i, 8 >> i, b))
            .collect();
        MetalRgba8Texture::upload(
            gpu.device(),
            MetalRgba8TextureDesc::new(8, 8, 4).unwrap(),
            &levels,
        )
        .unwrap()
    });
    (
        textures,
        [0, 1, 2, 3].map(|m| {
            MetalSampler::new(
                gpu.device(),
                SamplerDesc::nearest_clamp().with_explicit_mip(m),
            )
            .unwrap()
        }),
    )
}
fn pair(t: &MetalRgba8Texture, s: &MetalSampler, ti: u32, si: u32) -> BoundSampledTexture {
    BoundSampledTexture::bind(
        t.clone(),
        s.clone(),
        BindingSlot::new(ti),
        BindingSlot::new(si),
    )
    .unwrap()
}
#[test]
fn native_descriptor_tables_four_cross_pairs_sixteen_bindings_clear_and_owners() {
    let gpu = NativeCompute::new().unwrap();
    let (textures, samplers) = resources(&gpu);
    let all = |base: bool| {
        (0..4)
            .flat_map(|t| {
                let textures = &textures;
                let samplers = &samplers;
                (0..4).map(move |s| {
                    pair(
                        &textures[t],
                        &samplers[if base { 0 } else { s }],
                        t as u32,
                        s as u32,
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let bindings = [
        all(false),
        all(true),
        vec![],
        vec![pair(&textures[0], &samplers[0], 0, 0)],
    ];
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &[[0.; 4]; 3],
        viewport: [4, 1],
    }; 4];
    let states = [0, 1, 2, 3].map(|x| TriangleState {
        scissor: Some([x, 0, 1, 1]),
        ..Default::default()
    });
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 1],
        clears: &[[0; 4]],
        clear_depth: None,
    };
    let shaders = ["cross", "cross", "nulls", "nulls"].map(|entry| TriangleShader {
        source: SHADER,
        entries: ["vs", entry],
    });
    let pending = gpu
        .submit_multi_binding_triangle_targets_rgba8(&shaders, &pass, &bindings, None, &[None])
        .unwrap();
    drop(bindings);
    drop(textures);
    drop(samplers);
    drop(gpu);
    assert_eq!(
        pending.wait().unwrap(),
        vec![vec![
            41, 83, 127, 173, 11, 23, 37, 173, 255, 255, 255, 255, 0, 255, 255, 255
        ]]
    );
}
#[test]
fn native_descriptor_tables_reject_bounds_conflicts_and_keep_legacy_slot_zero() {
    let gpu = NativeCompute::new().unwrap();
    let (textures, samplers) = resources(&gpu);
    let pass = TriangleTargetsPass {
        draws: &[TriangleDraw {
            vertices: &POS,
            colors: &[[0.; 4]; 3],
            viewport: [1, 1],
        }],
        states: &[TriangleState::default()],
        extent: [1, 1],
        clears: &[[0; 4]],
        clear_depth: None,
    };
    let shaders = [TriangleShader {
        source: SHADER,
        entries: ["vs", "plain"],
    }];
    for bindings in [
        vec![pair(&textures[0], &samplers[0], 4, 0)],
        vec![pair(&textures[0], &samplers[0], 0, 4)],
        vec![
            pair(&textures[0], &samplers[0], 3, 0),
            pair(&textures[1], &samplers[0], 3, 1),
        ],
        vec![
            pair(&textures[0], &samplers[0], 0, 3),
            pair(&textures[0], &samplers[1], 1, 3),
        ],
        vec![pair(&textures[0], &samplers[0], 0, 0); 17],
    ] {
        assert!(
            gpu.submit_multi_binding_triangle_targets_rgba8(
                &shaders,
                &pass,
                &[bindings],
                None,
                &[None]
            )
            .is_err()
        );
    }
    assert!(
        gpu.submit_multi_pipeline_triangle_targets_rgba8(
            &shaders,
            &pass,
            &[Some(pair(&textures[0], &samplers[0], 3, 3))],
            None,
            &[None]
        )
        .is_err()
    );
    assert_eq!(
        gpu.submit_multi_pipeline_triangle_targets_rgba8(&shaders, &pass, &[None], None, &[None])
            .unwrap()
            .wait()
            .unwrap(),
        vec![vec![255, 0, 0, 255]]
    );
}
#[test]
fn native_descriptor_tables_slot_three_produced_gate_and_storage_alias() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let gate = Gate::new(&gpu, &c, RenderQueue::First);
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    let mut bytes = [17u8; 80];
    bytes[72..80].copy_from_slice(&[29, 71, 131, 197, 29, 71, 131, 197]);
    let loc = TypedImageCopyLocation::Texture {
        mip: 0,
        slice: 0,
        origin: [0, 0],
    };
    let producer = c
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::HostTexture {
                    desc,
                    bytes: &bytes,
                },
                destination: TypedCopyInput::HostTexture {
                    desc,
                    bytes: &bytes,
                },
                source_location: loc,
                destination_location: loc,
                extent: [4, 2],
            },
            &[],
        )
        .unwrap();
    let RenderCopyOutput::TypedTexture(root) = c.copy_output(&producer).unwrap() else {
        panic!("typed")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 1,
            mip_count: 1,
            format: TextureViewFormat::Rgba8Unorm,
        })
        .unwrap();
    let binding = QueuedSampledBinding::ProducedTypedView {
        view: view.clone(),
        sampler: MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap(),
        texture_slot: BindingSlot::new(3),
        sampler_slot: BindingSlot::new(3),
    };
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &[[0.; 4]; 3],
        viewport: [2, 1],
    }];
    let states = [TriangleState::default()];
    let shaders = [TriangleShader {
        source: SHADER,
        entries: ["vs", "third"],
    }];
    let typed = TypedTriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [2, 1],
        colors: &[TypedColorAttachment {
            format: TextureFormat::Rgba8Unorm,
            clear: TypedColorClear::Float([0.; 4]),
            blend_enabled: false,
        }],
        clear_depth: None,
    };
    assert!(
        c.submit_with_storage(
            RenderQueue::Second,
            &shaders,
            &typed,
            &[vec![binding.clone()]],
            None,
            &[QueuedColorLoad::Clear],
            &[],
            &[DrawUniforms::default()],
            &[None],
            QueuedStorageInput::ProducedTypedView(view.clone())
        )
        .is_err(),
        "slot3 same-root sampling must reject storage feedback"
    );
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [2, 1],
        clears: &[[0; 4]],
        clear_depth: None,
    };
    let consumer = c
        .submit_with_attachments(
            RenderQueue::Second,
            &shaders,
            &pass,
            &[vec![binding]],
            None,
            &[QueuedColorLoad::Clear],
            &[],
            &[DrawUniforms::default()],
        )
        .unwrap();
    drop(view);
    drop(root);
    gate.observe_then_release(&gpu, &c, RenderQueue::Second);
    c.collect_copy(&producer).unwrap();
    drop(producer);
    assert_eq!(
        c.collect(&consumer).unwrap().colors,
        vec![[29, 71, 131, 197].repeat(2)]
    );
}
