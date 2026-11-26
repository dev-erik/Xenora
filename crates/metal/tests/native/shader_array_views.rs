// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal array texels and an observable cross-queue producer gate.
use super::unaligned_copy::Gate;
use xenora_metal::*;
use xenora_texture::TextureFormat;
const POS: [[f32; 4]; 3] = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
const SHADER: &str = r"#include <metal_stdlib>
using namespace metal; struct V {float4 p [[position]];};
vertex V vs(uint i [[vertex_id]],device const float4*p [[buffer(0)]]){return {p[i]};}
fragment float4 ps(V v [[stage_in]],texture2d_array<float>t [[texture(0)]],sampler s [[sampler(0)]]){return t.sample(s,v.p.xy/float2(2.,1.),1u,level(0.0f));}";
fn range() -> ArrayTextureViewDesc {
    ArrayTextureViewDesc {
        base_slice: 1,
        slice_count: 2,
        base_mip: 1,
        mip_count: 1,
        format: TextureViewFormat::Rgba8Unorm,
    }
}
fn render(
    gpu: &NativeCompute,
    cohort: &mut RenderCohort,
    view: RenderTypedTextureArrayView,
) -> Result<RenderPoint, MetalError> {
    cohort.submit_with_attachments(
        RenderQueue::Second,
        &[TriangleShader {
            source: SHADER,
            entries: ["vs", "ps"],
        }],
        &TriangleTargetsPass {
            draws: &[TriangleDraw {
                vertices: &POS,
                colors: &[[0.; 4]; 3],
                viewport: [2, 1],
            }],
            states: &[TriangleState::default()],
            extent: [2, 1],
            clears: &[[0; 4]],
            clear_depth: None,
        },
        &[vec![QueuedSampledBinding::ProducedTypedArrayView {
            view,
            sampler: MetalSampler::new(gpu.device(), xenora_texture::SamplerDesc::nearest_clamp())
                .unwrap(),
            texture_slot: BindingSlot::new(0),
            sampler_slot: BindingSlot::new(0),
        }]],
        None,
        &[QueuedColorLoad::Clear],
        &[],
        &[DrawUniforms::default()],
    )
}
#[test]
fn native_shader_array_views_pending_gate_exact_range_and_foreign_stale_rejection() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let gate = Gate::new(&gpu, &cohort, RenderQueue::First);
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(3),
    };
    let mut bytes = [17u8; 120];
    let expected = [29, 71, 131, 197, 101, 103, 107, 109];
    bytes[112..120].copy_from_slice(&expected);
    let location = TypedImageCopyLocation::Texture {
        mip: 0,
        slice: 0,
        origin: [0, 0],
    };
    let producer = cohort
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
                source_location: location,
                destination_location: location,
                extent: [4, 2],
            },
            &[],
        )
        .unwrap();
    let RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&producer).unwrap() else {
        panic!("typed root")
    };
    let view = root.array_range_view(range()).unwrap();
    assert_eq!(view.desc(), range());
    assert_eq!(view.extent(), [2, 1]);
    let mut foreign = gpu.new_render_graph_cohort().unwrap();
    assert!(render(&gpu, &mut foreign, view.clone()).is_err());
    let changed = [211, 7, 13, 255, 5, 223, 19, 255];
    let next = cohort
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::HostBuffer(&changed),
                destination: TypedCopyInput::ProducedTexture(root.clone()),
                source_location: TypedImageCopyLocation::Buffer {
                    offset: 0,
                    bytes_per_row: 8,
                },
                destination_location: TypedImageCopyLocation::Texture {
                    mip: 1,
                    slice: 2,
                    origin: [0, 0],
                },
                extent: [2, 1],
            },
            &[],
        )
        .unwrap();
    let RenderCopyOutput::TypedTexture(new_root) = cohort.copy_output(&next).unwrap() else {
        panic!("new root")
    };
    let consumer = render(&gpu, &mut cohort, view.clone()).unwrap();
    let new_consumer = render(
        &gpu,
        &mut cohort,
        new_root.array_range_view(range()).unwrap(),
    )
    .unwrap();
    drop(new_root);
    drop(root);
    gate.observe_then_release(&gpu, &cohort, RenderQueue::Second);
    assert_eq!(
        cohort.collect(&new_consumer).unwrap().colors,
        vec![changed.to_vec()]
    );
    assert_eq!(
        cohort.collect(&consumer).unwrap().colors,
        vec![expected.to_vec()]
    );
    let OwnedCopyOutput::TypedTexture { bytes: actual, .. } =
        cohort.collect_copy(&producer).unwrap()
    else {
        panic!("typed root")
    };
    assert_eq!(actual, bytes);
    let OwnedCopyOutput::TypedTexture { bytes: updated, .. } = cohort.collect_copy(&next).unwrap()
    else {
        panic!("new root")
    };
    bytes[112..120].copy_from_slice(&changed);
    assert_eq!(updated, bytes);
    cohort.reinitialize().unwrap();
    assert!(render(&gpu, &mut cohort, view).is_err());
}
#[test]
fn native_shader_array_views_one_layer_identity_and_bounds() {
    let gpu = NativeCompute::new().unwrap();
    let owner = MetalTypedTexture::upload(
        &gpu,
        TypedTextureDesc {
            format: TextureFormat::Rgba8Unorm,
            extent: [2, 1],
            mip_levels: 1,
            array_layers: Some(1),
        },
        &[17, 53, 127, 193, 29, 71, 131, 197],
    )
    .unwrap();
    let desc = ArrayTextureViewDesc {
        base_slice: 0,
        slice_count: 1,
        base_mip: 0,
        mip_count: 1,
        format: TextureViewFormat::Rgba8Unorm,
    };
    for bad in [
        ArrayTextureViewDesc {
            slice_count: 0,
            ..desc
        },
        ArrayTextureViewDesc {
            base_slice: 1,
            ..desc
        },
        ArrayTextureViewDesc {
            base_slice: u32::MAX,
            slice_count: 2,
            ..desc
        },
        ArrayTextureViewDesc {
            mip_count: 2,
            ..desc
        },
        ArrayTextureViewDesc {
            base_mip: u32::MAX,
            mip_count: 2,
            ..desc
        },
        ArrayTextureViewDesc {
            format: TextureViewFormat::R32Uint,
            ..desc
        },
    ] {
        assert!(owner.array_range_view(bad).is_err());
    }
    let ordinary = MetalTypedTexture::upload(
        &gpu,
        TypedTextureDesc {
            format: TextureFormat::Rgba8Unorm,
            extent: [2, 1],
            mip_levels: 1,
            array_layers: None,
        },
        &[17u8; 8],
    )
    .unwrap();
    assert!(ordinary.array_range_view(desc).is_err());
    let depth = MetalTypedTexture::upload(
        &gpu,
        TypedTextureDesc {
            format: TextureFormat::Depth32Float,
            extent: [2, 1],
            mip_levels: 1,
            array_layers: Some(1),
        },
        &[0u8; 8],
    )
    .unwrap();
    assert!(
        depth
            .array_range_view(ArrayTextureViewDesc {
                format: TextureViewFormat::Depth32Float,
                ..desc
            })
            .is_err()
    );
    let view = owner.array_range_view(desc).unwrap();
    drop(owner);
    let shader = SHADER.replace("1u,level", "0u,level");
    let result = gpu
        .submit_view_binding_triangle_targets_rgba8(
            &[TriangleShader {
                source: &shader,
                entries: ["vs", "ps"],
            }],
            &TriangleTargetsPass {
                draws: &[TriangleDraw {
                    vertices: &POS,
                    colors: &[[0.; 4]; 3],
                    viewport: [2, 1],
                }],
                states: &[TriangleState::default()],
                extent: [2, 1],
                clears: &[[0; 4]],
                clear_depth: None,
            },
            &[vec![SampledBinding::ArrayView {
                view,
                sampler: MetalSampler::new(
                    gpu.device(),
                    xenora_texture::SamplerDesc::nearest_clamp(),
                )
                .unwrap(),
                texture_slot: BindingSlot::new(0),
                sampler_slot: BindingSlot::new(0),
            }]],
            None,
            &[None],
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(result, vec![vec![17, 53, 127, 193, 29, 71, 131, 197]]);
}
