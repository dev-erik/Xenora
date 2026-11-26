// SPDX-License-Identifier: GPL-3.0-or-later
//! Payload accounting and controlled failure for produced shader array bindings.
use super::*;
use crate::{
    ArrayTextureViewDesc, QueuedTypedImageCopy, TypedCopyInput, TypedImageCopyLocation,
    TypedTextureDesc,
};
const POS: [[f32; 4]; 3] = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
const SHADER: &str = r"#include <metal_stdlib>
using namespace metal; struct V {float4 p [[position]];};
vertex V vs(uint i [[vertex_id]],device const float4*p [[buffer(0)]]){return {p[i]};}
fragment float4 ps(V v [[stage_in]],texture2d_array<float>t [[texture(0)]],sampler s [[sampler(0)]]){return t.sample(s,v.p.xy/float2(2.,1.),1u,level(0.0f));}";
fn desc() -> TypedTextureDesc {
    TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(3),
    }
}
fn range() -> ArrayTextureViewDesc {
    ArrayTextureViewDesc {
        base_slice: 1,
        slice_count: 2,
        base_mip: 1,
        mip_count: 1,
        format: crate::TextureViewFormat::Rgba8Unorm,
    }
}
fn render(
    cohort: &mut RenderCohort,
    binding: QueuedSampledBinding,
) -> Result<RenderPoint, MetalError> {
    cohort.submit_with_attachments(
        RenderQueue::Second,
        &[TriangleShader {
            source: SHADER,
            entries: ["vs", "ps"],
        }],
        &crate::TriangleTargetsPass {
            draws: &[crate::TriangleDraw {
                vertices: &POS,
                colors: &[[0.; 4]; 3],
                viewport: [2, 1],
            }],
            states: &[crate::TriangleState::default()],
            extent: [2, 1],
            clears: &[[0; 4]],
            clear_depth: None,
        },
        &[vec![binding]],
        None,
        &[QueuedColorLoad::Clear],
        &[],
        &[DrawUniforms::default()],
    )
}
fn producer(cohort: &mut RenderCohort) -> (RenderPoint, crate::RenderTypedTextureArrayView) {
    let bytes = [17u8; 120];
    let d = desc();
    let loc = TypedImageCopyLocation::Texture {
        mip: 0,
        slice: 0,
        origin: [0, 0],
    };
    let point = cohort
        .submit_typed_image_copy(
            RenderQueue::First,
            QueuedTypedImageCopy {
                source: TypedCopyInput::HostTexture {
                    desc: d,
                    bytes: &bytes,
                },
                destination: TypedCopyInput::HostTexture {
                    desc: d,
                    bytes: &bytes,
                },
                source_location: loc,
                destination_location: loc,
                extent: [4, 2],
            },
            &[],
        )
        .unwrap();
    let crate::RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&point).unwrap() else {
        panic!("root")
    };
    (point, root.array_range_view(range()).unwrap())
}
fn binding(gpu: &NativeCompute, view: crate::RenderTypedTextureArrayView) -> QueuedSampledBinding {
    QueuedSampledBinding::ProducedTypedArrayView {
        view,
        sampler: crate::MetalSampler::new(
            gpu.device(),
            xenora_texture::SamplerDesc::nearest_clamp(),
        )
        .unwrap(),
        texture_slot: crate::BindingSlot::new(0),
        sampler_slot: crate::BindingSlot::new(0),
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_shader_array_views_full_root_lease_survives_pruning_and_reset() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let host = crate::MetalTypedTexture::upload(&gpu, desc(), &[17u8; 120]).unwrap();
    let baseline = render(
        &mut cohort,
        QueuedSampledBinding::ArrayViewed {
            view: host.array_range_view(range()).unwrap(),
            sampler: crate::MetalSampler::new(
                gpu.device(),
                xenora_texture::SamplerDesc::nearest_clamp(),
            )
            .unwrap(),
            texture_slot: crate::BindingSlot::new(0),
            sampler_slot: crate::BindingSlot::new(0),
        },
    )
    .unwrap();
    cohort.collect(&baseline).unwrap();
    let render_charge = budget.get();
    drop(baseline);
    drop(host);
    assert_eq!(budget.get(), 0);
    let (point, view) = producer(&mut cohort);
    cohort.collect_copy(&point).unwrap();
    drop(point);
    let source_charge = budget.get();
    assert!(
        source_charge >= 120,
        "selected16bytes must retain complete120byte root"
    );
    // The producer point also owns transient copy charges. Once pruned, only
    // the complete root lease remains: three layers * (8 + 2) texels * 4 bytes.
    let consumer = render(&mut cohort, binding(&gpu, view)).unwrap();
    cohort.collect(&consumer).unwrap();
    assert_eq!(
        budget.get(),
        render_charge + 120,
        "consumer must retain the root after successful ancestor pruning"
    );
    cohort.reinitialize().unwrap();
    assert_eq!(budget.get(), render_charge + 120);
    let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - budget.get()).unwrap();
    assert!(ColorLease::reserve(&budget, 1).is_err());
    drop(filler);
    drop(consumer);
    assert_eq!(budget.get(), 0);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_shader_array_views_failed_producer_rejects_before_admission() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let (point, view) = producer(&mut cohort);
    point.inner.command.waitUntilCompleted();
    point.inner.terminal.set(Some(false));
    let charged = budget.get();
    let sequence = cohort.sequence;
    assert!(render(&mut cohort, binding(&gpu, view.clone())).is_err());
    assert_eq!(cohort.sequence, sequence);
    assert_eq!(budget.get(), charged);
    cohort
        .abort_and_drain("controlled array producer failure")
        .unwrap();
    drop(view);
    drop(point);
    assert_eq!(budget.get(), 0);
}
