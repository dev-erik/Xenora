// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal packed output oracles; original RDNA shaders exercise typed exports.
use xenora_metal::*;
use xenora_shader::{
    Architecture::Gfx1030, GraphicsExportType, GraphicsPipelineTypes, GraphicsSampleType,
    PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics_pipeline_typed,
};
use xenora_texture::{SamplerDesc, TextureFormat};
const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");
const PS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin");
const SAMPLE: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-view-sample-ps.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
fn render(
    gpu: &NativeCompute,
    ps: &[u8],
    types: GraphicsPipelineTypes,
    color: [f32; 4],
    attachment: TypedColorAttachment,
    bindings: Vec<SampledBinding>,
) -> Vec<u8> {
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, ps, types).unwrap();
    gpu.submit_typed_triangle_targets(
        &[TriangleShader {
            source: &compiled.shader.msl,
            entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
        }],
        &TypedTriangleTargetsPass {
            draws: &[TriangleDraw {
                vertices: &POS,
                colors: &[color; 3],
                viewport: [1, 1],
            }],
            states: &[TriangleState::default()],
            extent: [1, 1],
            colors: &[attachment],
            clear_depth: None,
        },
        &[bindings],
        None,
        &[None],
        &[DrawUniforms::default()],
    )
    .unwrap()
    .wait()
    .unwrap()
    .remove(0)
}
#[test]
fn native_typed_attachment_seven_formats_literal_exports() {
    let gpu = NativeCompute::new().unwrap();
    for (format, expected) in [
        (TextureFormat::R8Unorm, vec![64]),
        (TextureFormat::Rg8Unorm, vec![64, 128]),
        (TextureFormat::Rgba8Unorm, vec![64, 128, 191, 255]),
        (TextureFormat::Bgra8Unorm, vec![191, 128, 64, 255]),
        (TextureFormat::Rgba16Float, vec![0, 52, 0, 56, 0, 58, 0, 60]),
        (TextureFormat::R32Float, vec![0, 0, 128, 62]),
    ] {
        assert_eq!(
            render(
                &gpu,
                PS,
                GraphicsPipelineTypes::default(),
                [0.25, 0.5, 0.75, 1.],
                TypedColorAttachment {
                    format,
                    clear: TypedColorClear::Float([0.; 4]),
                    blend_enabled: false
                },
                vec![]
            ),
            expected,
            "{format:?}"
        );
    }
    let mut types = GraphicsPipelineTypes::default();
    types.colors[0] = GraphicsExportType::Uint;
    // Float 1.0's bit pattern is exported as uint, not numerically converted to one.
    assert_eq!(
        render(
            &gpu,
            PS,
            types,
            [1., 0., 0., 1.],
            TypedColorAttachment {
                format: TextureFormat::R32Uint,
                clear: TypedColorClear::Uint([0; 4]),
                blend_enabled: false
            },
            vec![]
        ),
        [0, 0, 128, 63]
    );
    assert_eq!(
        render(
            &gpu,
            PS,
            GraphicsPipelineTypes::default(),
            [-1., 2., 0., 0.5],
            TypedColorAttachment {
                format: TextureFormat::Rgba16Float,
                clear: TypedColorClear::Float([0.; 4]),
                blend_enabled: false
            },
            vec![]
        ),
        [0, 188, 0, 64, 0, 0, 0, 56]
    );
}
#[test]
fn native_typed_attachment_uint_clear_and_host_load_are_bit_exact() {
    let gpu = NativeCompute::new().unwrap();
    for (value, expected) in [
        (0x01000001, [1, 0, 0, 1]),
        (u32::MAX, [255; 4]),
        (0x7fc12345, [69, 35, 193, 127]),
    ] {
        let colors = [TypedColorAttachment {
            format: TextureFormat::R32Uint,
            clear: TypedColorClear::Uint([value, 0, 0, 0]),
            blend_enabled: false,
        }];
        let pass = TypedTriangleTargetsPass {
            draws: &[],
            states: &[],
            extent: [2, 1],
            colors: &colors,
            clear_depth: None,
        };
        let actual = gpu
            .submit_typed_triangle_targets(&[], &pass, &[], None, &[None], &[])
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(actual, vec![expected.repeat(2)]);
        let prior = [17, 53, 127, 193, 1, 0, 0, 1];
        assert_eq!(
            gpu.submit_typed_triangle_targets(&[], &pass, &[], None, &[Some(&prior)], &[])
                .unwrap()
                .wait()
                .unwrap(),
            vec![prior.to_vec()]
        );
    }
}
#[test]
fn native_typed_sample_uint_depth_selected_array_mip_owner_drop() {
    let gpu = NativeCompute::new().unwrap();
    for (format, view_format, sample_type, export_type, words, expected) in [
        (
            TextureFormat::R32Uint,
            TextureViewFormat::R32Uint,
            GraphicsSampleType::Uint,
            GraphicsExportType::Uint,
            [0x01000001u32, 0x7fc12345],
            vec![vec![1, 0, 0, 1], vec![69, 35, 193, 127]],
        ),
        (
            TextureFormat::Depth32Float,
            TextureViewFormat::Depth32Float,
            GraphicsSampleType::Depth,
            GraphicsExportType::Float,
            [0x3e800000u32, 0x3f400000],
            vec![vec![64, 0, 0, 255], vec![191, 0, 0, 255]],
        ),
    ] {
        // Two layers, each a 2x1 root plus 1x1 mip. Selected values differ from roots.
        let packed = [0u32, 0, words[0], 0, 0, words[1]]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let owner = MetalTypedTexture::upload(
            &gpu,
            TypedTextureDesc {
                format,
                extent: [2, 1],
                mip_levels: 2,
                array_layers: Some(2),
            },
            &packed,
        )
        .unwrap();
        let views = (0..2)
            .map(|slice| {
                owner
                    .array_slice_view(ArraySliceViewDesc {
                        slice,
                        base_mip: 1,
                        mip_count: 1,
                        format: view_format,
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>();
        drop(owner);
        for (view, expected) in views.into_iter().zip(expected) {
            let sampler = MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap();
            let mut types = GraphicsPipelineTypes::default();
            types.textures[0] = sample_type;
            types.colors[0] = export_type;
            let attachment = if export_type == GraphicsExportType::Uint {
                TypedColorAttachment {
                    format: TextureFormat::R32Uint,
                    clear: TypedColorClear::Uint([0; 4]),
                    blend_enabled: false,
                }
            } else {
                TypedColorAttachment {
                    format: TextureFormat::Rgba8Unorm,
                    clear: TypedColorClear::Float([0.; 4]),
                    blend_enabled: false,
                }
            };
            assert_eq!(
                render(
                    &gpu,
                    SAMPLE,
                    types,
                    [0.5, 0.5, 0., 1.],
                    attachment,
                    vec![SampledBinding::View {
                        view,
                        sampler,
                        texture_slot: BindingSlot::new(0),
                        sampler_slot: BindingSlot::new(0)
                    }]
                ),
                expected
            );
        }
    }
}

#[test]
fn native_typed_sample_six_float_formats_literal_channels() {
    let gpu = NativeCompute::new().unwrap();
    for (format, view_format, bytes, expected) in [
        (
            TextureFormat::R8Unorm,
            TextureViewFormat::R8Unorm,
            vec![128],
            vec![128, 0, 0, 255],
        ),
        (
            TextureFormat::Rg8Unorm,
            TextureViewFormat::Rg8Unorm,
            vec![17, 193],
            vec![17, 193, 0, 255],
        ),
        (
            TextureFormat::Rgba8Unorm,
            TextureViewFormat::Rgba8Unorm,
            vec![17, 53, 127, 193],
            vec![17, 53, 127, 193],
        ),
        (
            TextureFormat::Bgra8Unorm,
            TextureViewFormat::Bgra8Unorm,
            vec![17, 53, 127, 193],
            vec![127, 53, 17, 193],
        ),
        (
            TextureFormat::Rgba16Float,
            TextureViewFormat::Rgba16Float,
            vec![0, 52, 0, 56, 0, 58, 0, 60],
            vec![64, 128, 191, 255],
        ),
        (
            TextureFormat::R32Float,
            TextureViewFormat::R32Float,
            vec![0, 0, 64, 63],
            vec![191, 0, 0, 255],
        ),
        (
            TextureFormat::Bgra8Unorm,
            TextureViewFormat::Bgra8UnormSrgb,
            vec![0, 128, 255, 17],
            vec![255, 55, 0, 17],
        ),
    ] {
        let owner = MetalTypedTexture::upload(
            &gpu,
            TypedTextureDesc {
                format,
                extent: [1, 1],
                mip_levels: 1,
                array_layers: None,
            },
            &bytes,
        )
        .unwrap();
        let view = owner
            .view(TextureViewDesc {
                base_mip: 0,
                mip_count: 1,
                format: view_format,
            })
            .unwrap();
        drop(owner);
        let sampler = MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap();
        assert_eq!(
            render(
                &gpu,
                SAMPLE,
                GraphicsPipelineTypes::default(),
                [0.5, 0.5, 0., 1.],
                TypedColorAttachment {
                    format: TextureFormat::Rgba8Unorm,
                    clear: TypedColorClear::Float([0.; 4]),
                    blend_enabled: false
                },
                vec![SampledBinding::View {
                    view,
                    sampler,
                    texture_slot: BindingSlot::new(0),
                    sampler_slot: BindingSlot::new(0)
                }]
            ),
            expected,
            "{format:?}/{view_format:?}"
        );
    }
}

const MIXED: &str = r"
#include <metal_stdlib>
using namespace metal;
struct V { float4 p [[position]]; };
struct O { float4 f [[color(0)]]; uint4 u [[color(1)]]; };
vertex V vs(uint i [[vertex_id]], device const float4 *p [[buffer(0)]]) { return {p[i]}; }
fragment O ps() { return {float4(.25,.5,.75,1),uint4(0x7fc12345u,0,0,1)}; }
";
#[test]
fn native_typed_mixed_mrt_blend_mask_load_and_depth_rollback() {
    let gpu = NativeCompute::new().unwrap();
    let mut depth = gpu.create_depth_target([2, 1]).unwrap();
    let colors = [
        TypedColorAttachment {
            format: TextureFormat::Rgba16Float,
            clear: TypedColorClear::Float([0.; 4]),
            blend_enabled: true,
        },
        TypedColorAttachment {
            format: TextureFormat::R32Uint,
            clear: TypedColorClear::Uint([0; 4]),
            blend_enabled: false,
        },
    ];
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &[[0.; 4]; 3],
        viewport: [2, 1],
    }];
    let states = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write: true,
        },
        blend: Some(BlendState {
            source_rgb: BlendFactor::One,
            destination_rgb: BlendFactor::One,
            rgb_operation: BlendOperation::Add,
            source_alpha: BlendFactor::One,
            destination_alpha: BlendFactor::One,
            alpha_operation: BlendOperation::Add,
        }),
        write_mask: ColorWriteMask {
            red: true,
            green: false,
            blue: true,
            alpha: false,
        },
        scissor: Some([0, 0, 1, 1]),
        ..TriangleState::default()
    }];
    let prior_f = [0, 52, 0, 56, 0, 52, 0, 60, 0, 60, 0, 64, 0, 66, 0, 68];
    let prior_u = [1, 0, 0, 1, 17, 53, 127, 193];
    let pass = TypedTriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [2, 1],
        colors: &colors,
        clear_depth: None,
    };
    let shaders = [TriangleShader {
        source: MIXED,
        entries: ["vs", "ps"],
    }];
    let actual = gpu
        .submit_typed_triangle_targets(
            &shaders,
            &pass,
            &[vec![]],
            Some((&mut depth, DepthLoad::Clear(1.))),
            &[Some(&prior_f), Some(&prior_u)],
            &[DrawUniforms::default()],
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            vec![0, 56, 0, 56, 0, 60, 0, 60, 0, 60, 0, 64, 0, 66, 0, 68],
            vec![69, 35, 193, 127, 17, 53, 127, 193]
        ]
    );
    // Malformed final Load must not publish the requested zero depth clear.
    assert!(
        gpu.submit_typed_triangle_targets(
            &shaders,
            &pass,
            &[vec![]],
            Some((&mut depth, DepthLoad::Clear(0.))),
            &[Some(&prior_f), Some(&prior_u[..7])],
            &[DrawUniforms::default()]
        )
        .is_err()
    );
    // Same .5 draw rejects left (stored .5) and passes right (stored 1).
    let probe_states = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write: false,
        },
        ..TriangleState::default()
    }];
    let probe = TypedTriangleTargetsPass {
        states: &probe_states,
        ..pass
    };
    let actual = gpu
        .submit_typed_triangle_targets(
            &shaders,
            &probe,
            &[vec![]],
            Some((&mut depth, DepthLoad::Load)),
            &[None, None],
            &[DrawUniforms::default()],
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            vec![0; 8]
                .into_iter()
                .chain([0, 52, 0, 56, 0, 58, 0, 60])
                .collect::<Vec<_>>(),
            vec![0, 0, 0, 0, 69, 35, 193, 127]
        ]
    );
    for invalid in [
        TypedColorAttachment {
            blend_enabled: true,
            ..colors[1]
        },
        TypedColorAttachment {
            clear: TypedColorClear::Float([0.; 4]),
            ..colors[1]
        },
        TypedColorAttachment {
            clear: TypedColorClear::Uint([0; 4]),
            ..colors[0]
        },
        TypedColorAttachment {
            format: TextureFormat::Depth32Float,
            ..colors[0]
        },
        TypedColorAttachment {
            format: TextureFormat::Bc1RgbaUnorm,
            ..colors[0]
        },
    ] {
        let bad = [colors[0], invalid];
        let invalid_pass = TypedTriangleTargetsPass {
            colors: &bad,
            ..probe
        };
        assert!(
            gpu.submit_typed_triangle_targets(
                &shaders,
                &invalid_pass,
                &[vec![]],
                None,
                &[None, None],
                &[DrawUniforms::default()]
            )
            .is_err()
        );
    }
}

#[test]
fn native_typed_bc_srgb_views_keep_endpoint_channels_and_alpha() {
    let gpu = NativeCompute::new().unwrap();
    if !gpu.supports_bc_textures() {
        return;
    }
    // Noninterpolated white endpoint: sRGB endpoints are exact; BC3 alpha is linear.
    for (format, view_format, bytes, alpha) in [
        (
            TextureFormat::Bc1RgbaUnorm,
            TextureViewFormat::Bc1RgbaUnormSrgb,
            vec![255, 255, 0, 0, 0, 0, 0, 0],
            255,
        ),
        (
            TextureFormat::Bc3RgbaUnorm,
            TextureViewFormat::Bc3RgbaUnormSrgb,
            vec![17, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0],
            17,
        ),
    ] {
        let owner = MetalTypedTexture::upload(
            &gpu,
            TypedTextureDesc {
                format,
                extent: [4, 4],
                mip_levels: 1,
                array_layers: None,
            },
            &bytes,
        )
        .unwrap();
        let view = owner
            .view(TextureViewDesc {
                base_mip: 0,
                mip_count: 1,
                format: view_format,
            })
            .unwrap();
        drop(owner);
        let sampler = MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap();
        assert_eq!(
            render(
                &gpu,
                SAMPLE,
                GraphicsPipelineTypes::default(),
                [0.5, 0.5, 0., 1.],
                TypedColorAttachment {
                    format: TextureFormat::Rgba8Unorm,
                    clear: TypedColorClear::Float([0.; 4]),
                    blend_enabled: false
                },
                vec![SampledBinding::View {
                    view,
                    sampler,
                    texture_slot: BindingSlot::new(0),
                    sampler_slot: BindingSlot::new(0)
                }]
            ),
            [255, 255, 255, alpha]
        );
    }
}

#[test]
fn native_typed_pending_uint_and_depth_views_descendant_first() {
    use super::unaligned_copy::Gate;
    let gpu = NativeCompute::new().unwrap();
    for depth_sample in [false, true] {
        let mut cohort = gpu.new_render_graph_cohort().unwrap();
        let gate = Gate::new(&gpu, &cohort, RenderQueue::First);
        let colors = [TypedColorAttachment {
            format: TextureFormat::R32Uint,
            clear: TypedColorClear::Uint([0x7fc12345, 0, 0, 0]),
            blend_enabled: false,
        }];
        let producer = cohort
            .submit_with_typed_attachments(
                RenderQueue::First,
                &[],
                &TypedTriangleTargetsPass {
                    draws: &[],
                    states: &[],
                    extent: [1, 1],
                    colors: &colors,
                    clear_depth: None,
                },
                &[],
                Some(QueuedDepthInput::Clear(0.25)),
                &[QueuedColorLoad::Clear],
                &[],
                &[],
                &[],
            )
            .unwrap();
        let output = if depth_sample {
            cohort
                .depth_output(&producer)
                .unwrap()
                .typed_output()
                .unwrap()
        } else {
            cohort.typed_color_output(&producer, 0).unwrap()
        };
        let view = output
            .view(TextureViewDesc {
                base_mip: 0,
                mip_count: 1,
                format: if depth_sample {
                    TextureViewFormat::Depth32Float
                } else {
                    TextureViewFormat::R32Uint
                },
            })
            .unwrap();
        drop(output);
        let mut types = GraphicsPipelineTypes::default();
        types.textures[0] = if depth_sample {
            GraphicsSampleType::Depth
        } else {
            GraphicsSampleType::Uint
        };
        if !depth_sample {
            types.colors[0] = GraphicsExportType::Uint;
        }
        let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, SAMPLE, types).unwrap();
        let target = if depth_sample {
            TypedColorAttachment {
                format: TextureFormat::Rgba8Unorm,
                clear: TypedColorClear::Float([0.; 4]),
                blend_enabled: false,
            }
        } else {
            colors[0]
        };
        let sampler = MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap();
        let consumer = cohort
            .submit_with_typed_attachments(
                RenderQueue::Second,
                &[TriangleShader {
                    source: &compiled.shader.msl,
                    entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                }],
                &TypedTriangleTargetsPass {
                    draws: &[TriangleDraw {
                        vertices: &POS,
                        colors: &[[0.5, 0.5, 0., 1.]; 3],
                        viewport: [1, 1],
                    }],
                    states: &[TriangleState::default()],
                    extent: [1, 1],
                    colors: &[target],
                    clear_depth: None,
                },
                &[vec![QueuedSampledBinding::ProducedTypedView {
                    view,
                    sampler,
                    texture_slot: BindingSlot::new(0),
                    sampler_slot: BindingSlot::new(0),
                }]],
                None,
                &[QueuedColorLoad::Clear],
                &[],
                &[DrawUniforms::default()],
                &[None],
            )
            .unwrap();
        gate.observe_then_release(&gpu, &cohort, RenderQueue::Second);
        assert_eq!(
            cohort.collect(&consumer).unwrap().colors,
            vec![if depth_sample {
                vec![64, 0, 0, 255]
            } else {
                vec![69, 35, 193, 127]
            }]
        );
        cohort.collect(&producer).unwrap();
    }
}

#[test]
fn native_typed_produced_load_keeps_exact_old_version() {
    use super::unaligned_copy::Gate;
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let gate = Gate::new(&gpu, &cohort, RenderQueue::First);
    let target = [TypedColorAttachment {
        format: TextureFormat::R32Uint,
        clear: TypedColorClear::Uint([0x01000001, 0, 0, 0]),
        blend_enabled: false,
    }];
    let empty = TypedTriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [2, 1],
        colors: &target,
        clear_depth: None,
    };
    let old_point = cohort
        .submit_with_typed_attachments(
            RenderQueue::First,
            &[],
            &empty,
            &[],
            None,
            &[QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    let old = cohort.typed_color_output(&old_point, 0).unwrap();
    const UINT: &str = r"
#include <metal_stdlib>
using namespace metal;
struct V { float4 p [[position]]; };
vertex V vs(uint i [[vertex_id]],device const float4 *p [[buffer(0)]]) {return {p[i]};}
fragment uint4 ps() {return uint4(0x7fc12345u,0,0,1);}
";
    let changed = cohort
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[TriangleShader {
                source: UINT,
                entries: ["vs", "ps"],
            }],
            &TypedTriangleTargetsPass {
                draws: &[TriangleDraw {
                    vertices: &POS,
                    colors: &[[0.; 4]; 3],
                    viewport: [2, 1],
                }],
                states: &[TriangleState {
                    scissor: Some([0, 0, 1, 1]),
                    ..TriangleState::default()
                }],
                extent: [2, 1],
                colors: &target,
                clear_depth: None,
            },
            &[vec![]],
            None,
            &[QueuedColorLoad::ProducedTyped(old.clone())],
            &[],
            &[DrawUniforms::default()],
            &[None],
        )
        .unwrap();
    let newer = cohort.typed_color_output(&changed, 0).unwrap();
    gate.observe_then_release(&gpu, &cohort, RenderQueue::Second);
    assert_eq!(
        cohort.collect(&changed).unwrap().colors,
        vec![vec![69, 35, 193, 127, 1, 0, 0, 1]]
    );
    cohort.collect(&old_point).unwrap();
    drop(old_point);
    drop(changed);
    // Empty-draw Load must store the selected immutable version, even after
    // both source receipts were collected and a newer version exists.
    for (source, expected) in [
        (old, vec![1, 0, 0, 1, 1, 0, 0, 1]),
        (newer, vec![69, 35, 193, 127, 1, 0, 0, 1]),
    ] {
        let p = cohort
            .submit_with_typed_attachments(
                RenderQueue::First,
                &[],
                &empty,
                &[],
                None,
                &[QueuedColorLoad::ProducedTyped(source)],
                &[],
                &[],
                &[],
            )
            .unwrap();
        assert_eq!(cohort.collect(&p).unwrap().colors, vec![expected]);
    }
}

#[test]
fn native_typed_uint_runtime_join_preserves_integer_bits() {
    let gpu = NativeCompute::new().unwrap();
    let mut types = GraphicsPipelineTypes {
        textures: [GraphicsSampleType::Uint; 2],
        ..GraphicsPipelineTypes::default()
    };
    types.colors[0] = GraphicsExportType::Uint;
    let compiled = compile_graphics_pipeline_typed(
        Gfx1030,
        VS,
        include_bytes!("../../../../fixtures/rdna2/x9-runtime-sampled-ps.bin"),
        types,
    )
    .unwrap();
    let mut bindings = Vec::new();
    for (slot, bytes) in [[1, 0, 0, 1], [69, 35, 193, 127]].into_iter().enumerate() {
        let owner = MetalTypedTexture::upload(
            &gpu,
            TypedTextureDesc {
                format: TextureFormat::R32Uint,
                extent: [1, 1],
                mip_levels: 1,
                array_layers: None,
            },
            &bytes,
        )
        .unwrap();
        let view = owner
            .view(TextureViewDesc {
                base_mip: 0,
                mip_count: 1,
                format: TextureViewFormat::R32Uint,
            })
            .unwrap();
        bindings.push(SampledBinding::View {
            view,
            sampler: MetalSampler::new(gpu.device(), SamplerDesc::nearest_clamp()).unwrap(),
            texture_slot: BindingSlot::new(slot as u32),
            sampler_slot: BindingSlot::new(slot as u32),
        });
    }
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &[[0.5, 0.5, 0., 1.]; 3],
        viewport: [3, 1],
    }; 3];
    let states = std::array::from_fn::<_, 3, _>(|x| TriangleState {
        scissor: Some([x, 0, 1, 1]),
        ..TriangleState::default()
    });
    let uniforms = [0, 1, 0].map(|v| DrawUniforms {
        required_mask: 1,
        values: Some([v, 0, 0, 0, 0, 0, 0, 0]),
    });
    let shaders = [TriangleShader {
        source: &compiled.shader.msl,
        entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
    }; 3];
    let output = gpu
        .submit_typed_triangle_targets(
            &shaders,
            &TypedTriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [3, 1],
                colors: &[TypedColorAttachment {
                    format: TextureFormat::R32Uint,
                    clear: TypedColorClear::Uint([0; 4]),
                    blend_enabled: false,
                }],
                clear_depth: None,
            },
            &[bindings.clone(), bindings.clone(), bindings],
            None,
            &[None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(output, vec![vec![1, 0, 0, 1, 69, 35, 193, 127, 1, 0, 0, 1]]);
}

#[test]
fn native_typed_blend_host_load_r8_rg8_bgra_r32_literals() {
    let gpu = NativeCompute::new().unwrap();
    let compiled =
        compile_graphics_pipeline_typed(Gfx1030, VS, PS, GraphicsPipelineTypes::default()).unwrap();
    for (format, prior, expected) in [
        // Binary-exact .25 + normalized 64 gives 128 after UNORM rounding.
        (TextureFormat::R8Unorm, vec![64, 17], vec![128, 17]),
        (
            TextureFormat::Rg8Unorm,
            vec![64, 32, 17, 193],
            vec![128, 96, 17, 193],
        ),
        // BGRA storage order: blue .25 +64; green .25+32; red .25+64.
        (
            TextureFormat::Bgra8Unorm,
            vec![64, 32, 64, 0, 17, 53, 127, 193],
            vec![128, 96, 128, 255, 17, 53, 127, 193],
        ),
        (
            TextureFormat::R32Float,
            vec![0, 0, 128, 62, 0, 0, 64, 63],
            vec![0, 0, 0, 63, 0, 0, 64, 63],
        ),
    ] {
        let state = TriangleState {
            blend: Some(BlendState {
                source_rgb: BlendFactor::One,
                destination_rgb: BlendFactor::One,
                rgb_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
            }),
            scissor: Some([0, 0, 1, 1]),
            ..TriangleState::default()
        };
        let output = gpu
            .submit_typed_triangle_targets(
                &[TriangleShader {
                    source: &compiled.shader.msl,
                    entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                }],
                &TypedTriangleTargetsPass {
                    draws: &[TriangleDraw {
                        vertices: &POS,
                        colors: &[[0.25, 0.25, 0.25, 1.]; 3],
                        viewport: [2, 1],
                    }],
                    states: &[state],
                    extent: [2, 1],
                    colors: &[TypedColorAttachment {
                        format,
                        clear: TypedColorClear::Float([0.; 4]),
                        blend_enabled: true,
                    }],
                    clear_depth: None,
                },
                &[vec![]],
                None,
                &[Some(&prior)],
                &[DrawUniforms::default()],
            )
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(output, vec![expected], "{format:?}");
    }
}
