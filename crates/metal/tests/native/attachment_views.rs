// SPDX-License-Identifier: GPL-3.0-or-later
//! Original trusted MSL, literal storage bytes; native selected-color evidence only.
use super::unaligned_copy::Gate;
use xenora_metal::*;
use xenora_texture::TextureFormat;
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const SHADER: &str = r"
#include <metal_stdlib>
using namespace metal;
struct V {float4 p [[position]];};
vertex V vs(uint i [[vertex_id]],device const float4 *p [[buffer(0)]]) {return {p[i]};}
fragment float4 ps() {return float4(.25,.25,.25,1);}
fragment uint4 psu() {return uint4(0x7fc12345u,0,0,1);}
";
fn location(mip: u32, slice: u32) -> TypedImageCopyLocation {
    TypedImageCopyLocation::Texture {
        mip,
        slice,
        origin: [0, 0],
    }
}
fn seed(
    c: &mut RenderCohort,
    desc: TypedTextureDesc,
    bytes: &[u8],
) -> (RenderPoint, RenderTypedTextureOutput) {
    let prior = bytes.to_vec();
    let p = c
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::HostTexture { desc, bytes },
                destination: TypedCopyInput::HostTexture {
                    desc,
                    bytes: &prior,
                },
                source_location: location(0, 0),
                destination_location: location(0, 0),
                extent: desc.extent,
            },
            &[],
        )
        .unwrap();
    let RenderCopyOutput::TypedTexture(output) = c.copy_output(&p).unwrap() else {
        panic!("typed root")
    };
    (p, output)
}
fn attachment(format: TextureFormat) -> TypedColorAttachment {
    TypedColorAttachment {
        format,
        clear: if format == TextureFormat::R32Uint {
            TypedColorClear::Uint([0; 4])
        } else {
            TypedColorClear::Float([0.; 4])
        },
        blend_enabled: format != TextureFormat::R32Uint,
    }
}
fn render(
    c: &mut RenderCohort,
    view: RenderTypedTextureView,
    format: TextureFormat,
    extent: [usize; 2],
    draw: bool,
) -> Result<RenderPoint, MetalError> {
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &[[0.; 4]; 3],
        viewport: extent,
    }];
    let states = [TriangleState {
        scissor: Some([0, 0, 1, 1]),
        blend: Some(BlendState {
            source_rgb: BlendFactor::One,
            destination_rgb: BlendFactor::One,
            rgb_operation: BlendOperation::Add,
            source_alpha: BlendFactor::One,
            destination_alpha: BlendFactor::One,
            alpha_operation: BlendOperation::Add,
        }),
        ..TriangleState::default()
    }];
    let shader = [TriangleShader {
        source: SHADER,
        entries: [
            "vs",
            if format == TextureFormat::R32Uint {
                "psu"
            } else {
                "ps"
            },
        ],
    }];
    let bindings = [vec![]];
    let uniforms = [DrawUniforms::default()];
    let geometry = [None];
    c.submit_with_typed_attachments(
        RenderQueue::Second,
        if draw { &shader } else { &[] },
        &TypedTriangleTargetsPass {
            draws: if draw { &draws } else { &[] },
            states: if draw { &states } else { &[] },
            extent,
            colors: &[attachment(format)],
            clear_depth: None,
        },
        if draw { &bindings } else { &[] },
        None,
        &[QueuedColorLoad::ProducedTypedView(view)],
        &[],
        if draw { &uniforms } else { &[] },
        if draw { &geometry } else { &[] },
    )
}
#[test]
fn native_attachment_views_seven_formats_pending_load_blend_copyback() {
    let gpu = NativeCompute::new().unwrap();
    for (format, view_format, old, new) in [
        (
            TextureFormat::R8Unorm,
            TextureViewFormat::R8Unorm,
            vec![64],
            vec![128],
        ),
        (
            TextureFormat::Rg8Unorm,
            TextureViewFormat::Rg8Unorm,
            vec![64, 32],
            vec![128, 96],
        ),
        (
            TextureFormat::Rgba8Unorm,
            TextureViewFormat::Rgba8Unorm,
            vec![64, 32, 64, 0],
            vec![128, 96, 128, 255],
        ),
        (
            TextureFormat::Bgra8Unorm,
            TextureViewFormat::Bgra8Unorm,
            vec![32, 64, 17, 0],
            vec![96, 128, 81, 255],
        ),
        (
            TextureFormat::Rgba16Float,
            TextureViewFormat::Rgba16Float,
            vec![0, 52, 0, 52, 0, 52, 0, 0],
            vec![0, 56, 0, 56, 0, 56, 0, 60],
        ),
        (
            TextureFormat::R32Float,
            TextureViewFormat::R32Float,
            vec![0, 0, 128, 62],
            vec![0, 0, 0, 63],
        ),
        (
            TextureFormat::R32Uint,
            TextureViewFormat::R32Uint,
            vec![1, 0, 0, 1],
            vec![69, 35, 193, 127],
        ),
    ] {
        let mut c = gpu.new_render_graph_cohort().unwrap();
        let gate = Gate::new(&gpu, &c, RenderQueue::First);
        let desc = TypedTextureDesc {
            format,
            extent: [4, 2],
            mip_levels: 2,
            array_layers: Some(2),
        };
        // Each layer has 8 base texels +2 mip1 texels; selected slice1/mip1
        // starts at texel18. Every unselected byte is an independent guard.
        let mut bytes = (0..20 * old.len())
            .map(|i| (i % 173 + 19) as u8)
            .collect::<Vec<_>>();
        bytes[18 * old.len()..].copy_from_slice(&old.repeat(2));
        let original = bytes.clone();
        let mut expected = bytes.clone();
        expected[18 * old.len()..19 * old.len()].copy_from_slice(&new);
        let (producer, root) = seed(&mut c, desc, &bytes);
        let view = root
            .array_slice_view(ArraySliceViewDesc {
                slice: 1,
                base_mip: 1,
                mip_count: 1,
                format: view_format,
            })
            .unwrap();
        let rendered = render(&mut c, view, format, [2, 1], true).unwrap();
        let rendered_output = c.typed_color_output(&rendered, 0).unwrap();
        // Selected mip is only 2x1. Rejection must not poison the pending
        // rendered output or its root prior; the valid retry below must work.
        assert!(
            c.submit_typed_image_copy(
                RenderQueue::First,
                QueuedTypedImageCopy {
                    source: TypedCopyInput::ProducedTexture(rendered_output.clone()),
                    destination: TypedCopyInput::ProducedTexture(root.clone()),
                    source_location: location(0, 0),
                    destination_location: location(1, 1),
                    extent: [3, 1],
                },
                &[],
            )
            .is_err()
        );
        let copyback = c
            .submit_typed_image_copy(
                RenderQueue::First,
                QueuedTypedImageCopy {
                    source: TypedCopyInput::ProducedTexture(rendered_output),
                    destination: TypedCopyInput::ProducedTexture(root.clone()),
                    source_location: location(0, 0),
                    destination_location: location(1, 1),
                    extent: [2, 1],
                },
                &[],
            )
            .unwrap();
        drop(root);
        bytes.fill(0xee);
        gate.observe_then_release(&gpu, &c, RenderQueue::Second);
        let OwnedCopyOutput::TypedTexture {
            desc: actual,
            bytes,
        } = c.collect_copy(&copyback).unwrap()
        else {
            panic!("typed")
        };
        assert_eq!(actual, desc);
        assert_eq!(bytes, expected, "{format:?}");
        assert_eq!(
            c.collect(&rendered).unwrap().colors,
            vec![[new, old].concat()]
        );
        let OwnedCopyOutput::TypedTexture {
            desc: original_desc,
            bytes: original_bytes,
        } = c.collect_copy(&producer).unwrap()
        else {
            panic!("original typed root")
        };
        assert_eq!(original_desc, desc);
        assert_eq!(
            original_bytes, original,
            "copyback mutated source root: {format:?}"
        );
    }
}
#[test]
fn native_attachment_views_reject_invalid_selection_then_recover() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    let bytes = [17, 53, 127, 193].repeat(20);
    let (p, root) = seed(&mut c, desc, &bytes);
    c.collect_copy(&p).unwrap();
    let make = |count, format| {
        root.array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 0,
            mip_count: count,
            format,
        })
        .unwrap()
    };
    assert!(
        render(
            &mut c,
            make(2, TextureViewFormat::Rgba8Unorm),
            TextureFormat::Rgba8Unorm,
            [4, 2],
            false
        )
        .is_err()
    );
    assert!(
        render(
            &mut c,
            make(1, TextureViewFormat::Rgba8UnormSrgb),
            TextureFormat::Rgba8Unorm,
            [4, 2],
            false
        )
        .is_err()
    );
    assert!(
        render(
            &mut c,
            make(1, TextureViewFormat::Rgba8Unorm),
            TextureFormat::Bgra8Unorm,
            [4, 2],
            false
        )
        .is_err()
    );
    assert!(
        render(
            &mut c,
            make(1, TextureViewFormat::Rgba8Unorm),
            TextureFormat::Rgba8Unorm,
            [2, 1],
            false
        )
        .is_err()
    );
    let mut foreign = gpu.new_render_graph_cohort().unwrap();
    assert!(
        render(
            &mut foreign,
            make(1, TextureViewFormat::Rgba8Unorm),
            TextureFormat::Rgba8Unorm,
            [4, 2],
            false
        )
        .is_err()
    );
    for (format, view_format, raw) in [
        (
            TextureFormat::Depth32Float,
            TextureViewFormat::Depth32Float,
            vec![0, 0, 128, 62],
        ),
        (
            TextureFormat::Bc1RgbaUnorm,
            TextureViewFormat::Bc1RgbaUnorm,
            vec![255, 255, 0, 0, 0, 0, 0, 0],
        ),
    ] {
        if format == TextureFormat::Bc1RgbaUnorm && !gpu.supports_bc_textures() {
            continue;
        }
        let bad_desc = TypedTextureDesc {
            format,
            extent: [1, 1],
            mip_levels: 1,
            array_layers: Some(1),
        };
        let (bp, bad) = seed(&mut c, bad_desc, &raw);
        c.collect_copy(&bp).unwrap();
        let view = bad
            .array_slice_view(ArraySliceViewDesc {
                slice: 0,
                base_mip: 0,
                mip_count: 1,
                format: view_format,
            })
            .unwrap();
        assert!(render(&mut c, view, TextureFormat::Rgba8Unorm, [1, 1], false).is_err());
    }
    // A zero-draw selected Load still stores every selected pixel.
    let good = render(
        &mut c,
        make(1, TextureViewFormat::Rgba8Unorm),
        TextureFormat::Rgba8Unorm,
        [4, 2],
        false,
    )
    .unwrap();
    assert_eq!(
        c.collect(&good).unwrap().colors,
        vec![[17, 53, 127, 193].repeat(8)]
    );
    c.reinitialize().unwrap();
    assert!(
        render(
            &mut c,
            make(1, TextureViewFormat::Rgba8Unorm),
            TextureFormat::Rgba8Unorm,
            [4, 2],
            false
        )
        .is_err()
    );
}
