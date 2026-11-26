// SPDX-License-Identifier: GPL-3.0-or-later
//! Original trusted shaders and literal depth planes; native component evidence.
use super::unaligned_copy::Gate;
use xenora_metal::*;
use xenora_texture::TextureFormat;
const SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;
struct V {float4 p [[position]];};
vertex V vs(uint i [[vertex_id]],device const float4 *p [[buffer(0)]]) {return {p[i]};}
fragment float4 ps() {return float4(1,0,0,1);}
";
fn loc(mip: u32, slice: u32) -> TypedImageCopyLocation {
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
                source_location: loc(0, 0),
                destination_location: loc(0, 0),
                extent: desc.extent,
            },
            &[],
        )
        .unwrap();
    let RenderCopyOutput::TypedTexture(out) = c.copy_output(&p).unwrap() else {
        panic!("typed")
    };
    (p, out)
}
fn probe(
    c: &mut RenderCohort,
    input: QueuedDepthInput,
    z: f32,
    write: bool,
    empty: bool,
    extent: [usize; 2],
) -> Result<RenderPoint, MetalError> {
    let positions = [[-1., -1., z, 1.], [3., -1., z, 1.], [-1., 3., z, 1.]];
    let draws = [TriangleDraw {
        vertices: &positions,
        colors: &[[0.; 4]; 3],
        viewport: extent,
    }];
    let states = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write,
        },
        ..TriangleState::default()
    }];
    let shaders = [TriangleShader {
        source: SOURCE,
        entries: ["vs", "ps"],
    }];
    let bindings = [vec![]];
    let uniforms = [DrawUniforms::default()];
    let geometry = [None];
    c.submit_with_typed_attachments(
        RenderQueue::Second,
        if empty { &[] } else { &shaders },
        &TypedTriangleTargetsPass {
            draws: if empty { &[] } else { &draws },
            states: if empty { &[] } else { &states },
            extent,
            colors: &[TypedColorAttachment {
                format: TextureFormat::Rgba8Unorm,
                clear: TypedColorClear::Float([0., 0., 0., 1.]),
                blend_enabled: false,
            }],
            clear_depth: None,
        },
        if empty { &[] } else { &bindings },
        Some(input),
        &[QueuedColorLoad::Clear],
        &[],
        if empty { &[] } else { &uniforms },
        if empty { &[] } else { &geometry },
    )
}
fn colors(result: OwnedRenderOutput, right: bool) {
    assert_eq!(
        result.colors,
        vec![if right {
            vec![0, 0, 0, 255, 255, 0, 0, 255]
        } else {
            vec![0, 0, 0, 255, 0, 0, 0, 255]
        }]
    );
    assert_eq!(result.depth.unwrap().byte_len(), 8);
}
fn selected(root: &RenderTypedTextureOutput, count: u32) -> RenderTypedTextureView {
    root.array_slice_view(ArraySliceViewDesc {
        slice: 1,
        base_mip: 1,
        mip_count: count,
        format: TextureViewFormat::Depth32Float,
    })
    .unwrap()
}
#[test]
fn native_depth_attachment_views_pending_readonly_write_exact_versions_copyback() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let gate = Gate::new(&gpu, &c, RenderQueue::First);
    let desc = TypedTextureDesc {
        format: TextureFormat::Depth32Float,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    // 20 packed texels; slice1/mip1 is texels18,19. Unselected planes .875.
    let mut bytes = [0, 0, 96, 63].repeat(20);
    bytes[72..80].copy_from_slice(&[0, 0, 128, 62, 0, 0, 64, 63]);
    let original = bytes.clone();
    let (producer, root) = seed(&mut c, desc, &bytes);
    let old_view = selected(&root, 1);
    let read = probe(
        &mut c,
        QueuedDepthInput::ProducedTypedView(old_view.clone()),
        0.5,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    let readonly = c.depth_output(&read).unwrap();
    bytes.fill(0xee);
    gate.observe_then_release(&gpu, &c, RenderQueue::Second);
    colors(c.collect(&read).unwrap(), true);
    let OwnedCopyOutput::TypedTexture { bytes, .. } = c.collect_copy(&producer).unwrap() else {
        panic!("root")
    };
    assert_eq!(bytes, original);
    drop(producer);
    drop(read);
    // Exercise the selected-view path itself with no draws, not only an
    // ordinary Produced depth output from a preceding rendered pass.
    let selected_empty = probe(
        &mut c,
        QueuedDepthInput::ProducedTypedView(old_view.clone()),
        0.0,
        true,
        true,
        [2, 1],
    )
    .unwrap();
    let selected_preserved = c.depth_output(&selected_empty).unwrap();
    colors(c.collect(&selected_empty).unwrap(), false);
    let selected_probe = probe(
        &mut c,
        QueuedDepthInput::Produced(selected_preserved),
        0.625,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    colors(c.collect(&selected_probe).unwrap(), true);
    // A .625 probe distinguishes read-only preservation from an accidental .5 write.
    let p = probe(
        &mut c,
        QueuedDepthInput::Produced(readonly.clone()),
        0.625,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    colors(c.collect(&p).unwrap(), true);
    let empty = probe(
        &mut c,
        QueuedDepthInput::Produced(readonly),
        0.0,
        true,
        true,
        [2, 1],
    )
    .unwrap();
    let preserved = c.depth_output(&empty).unwrap();
    colors(c.collect(&empty).unwrap(), false);
    let write = probe(
        &mut c,
        QueuedDepthInput::Produced(preserved),
        0.5,
        true,
        false,
        [2, 1],
    )
    .unwrap();
    let written = c.depth_output(&write).unwrap();
    colors(c.collect(&write).unwrap(), true);
    let rejected = probe(
        &mut c,
        QueuedDepthInput::Produced(written.clone()),
        0.625,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    colors(c.collect(&rejected).unwrap(), false);
    let old = probe(
        &mut c,
        QueuedDepthInput::ProducedTypedView(old_view),
        0.625,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    colors(c.collect(&old).unwrap(), true);
    let copied = c
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::ProducedTexture(written.typed_output().unwrap()),
                destination: TypedCopyInput::ProducedTexture(root.clone()),
                source_location: loc(0, 0),
                destination_location: loc(1, 1),
                extent: [2, 1],
            },
            &[],
        )
        .unwrap();
    let mut expected = original.clone();
    expected[76..80].copy_from_slice(&[0, 0, 0, 63]);
    let OwnedCopyOutput::TypedTexture { bytes, .. } = c.collect_copy(&copied).unwrap() else {
        panic!("copyback")
    };
    assert_eq!(bytes, expected);
    // Root output survives receipts and replacement copyback; its bytes remain old.
    let unchanged = c
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::ProducedTexture(root.clone()),
                destination: TypedCopyInput::HostTexture {
                    desc,
                    bytes: &original,
                },
                source_location: loc(1, 1),
                destination_location: loc(1, 1),
                extent: [2, 1],
            },
            &[],
        )
        .unwrap();
    drop(root);
    let OwnedCopyOutput::TypedTexture { bytes, .. } = c.collect_copy(&unchanged).unwrap() else {
        panic!("old root")
    };
    assert_eq!(bytes, original);
}
#[test]
fn native_depth_attachment_views_invalid_selection_recovery_and_generation() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let desc = TypedTextureDesc {
        format: TextureFormat::Depth32Float,
        extent: [4, 2],
        mip_levels: 3,
        array_layers: Some(2),
    };
    let bytes = [0, 0, 64, 63].repeat(22);
    let (p, root) = seed(&mut c, desc, &bytes);
    c.collect_copy(&p).unwrap();
    assert!(
        probe(
            &mut c,
            QueuedDepthInput::ProducedTypedView(selected(&root, 2)),
            0.5,
            false,
            false,
            [2, 1]
        )
        .is_err()
    );
    assert!(
        probe(
            &mut c,
            QueuedDepthInput::ProducedTypedView(selected(&root, 1)),
            0.5,
            false,
            false,
            [1, 1]
        )
        .is_err()
    );
    let mut foreign = gpu.new_render_graph_cohort().unwrap();
    assert!(
        probe(
            &mut foreign,
            QueuedDepthInput::ProducedTypedView(selected(&root, 1)),
            0.5,
            false,
            false,
            [2, 1]
        )
        .is_err()
    );
    let bad_desc = TypedTextureDesc {
        format: TextureFormat::R32Float,
        extent: [2, 1],
        mip_levels: 1,
        array_layers: None,
    };
    let (bp, bad) = seed(&mut c, bad_desc, &[0, 0, 64, 63, 0, 0, 64, 63]);
    c.collect_copy(&bp).unwrap();
    let bad_view = bad
        .view(TextureViewDesc {
            base_mip: 0,
            mip_count: 1,
            format: TextureViewFormat::R32Float,
        })
        .unwrap();
    assert!(
        probe(
            &mut c,
            QueuedDepthInput::ProducedTypedView(bad_view),
            0.5,
            false,
            false,
            [2, 1]
        )
        .is_err()
    );
    let good = probe(
        &mut c,
        QueuedDepthInput::ProducedTypedView(selected(&root, 1)),
        0.5,
        false,
        false,
        [2, 1],
    )
    .unwrap();
    assert_eq!(
        c.collect(&good).unwrap().colors,
        vec![[255, 0, 0, 255].repeat(2)]
    );
    c.reinitialize().unwrap();
    assert!(
        probe(
            &mut c,
            QueuedDepthInput::ProducedTypedView(selected(&root, 1)),
            0.5,
            false,
            false,
            [2, 1]
        )
        .is_err()
    );
}
