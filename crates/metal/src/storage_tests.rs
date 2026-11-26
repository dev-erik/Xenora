// SPDX-License-Identifier: GPL-3.0-or-later
//! Real GPU storage writes, source waits, and logical payload ownership.
use super::*;
use crate::{
    ArraySliceViewDesc, QueuedStorageInput, QueuedTypedImageCopy, TypedCopyInput,
    TypedImageCopyLocation, TypedTextureDesc,
};
const POS: [[f32; 4]; 3] = [[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]];
const SHADER: &str = r"#include <metal_stdlib>
using namespace metal; struct V {float4 p [[position]];};
vertex V vs(uint i [[vertex_id]], device const float4*p [[buffer(0)]]) {return {p[i]};}
fragment float4 ps(V v [[stage_in]], texture2d<float,access::write> t [[texture(0)]]) {
 if(uint(v.p.x)==0u) t.write(float4(1.,0.,0.,1.),uint2(v.p.xy)); return float4(0.,1.,0.,1.); }
fragment float4 psu(V v [[stage_in]], texture2d<uint,access::write> t [[texture(0)]]) {
 if(uint(v.p.x)==0u) t.write(uint4(0x7fc12345u,0,0,1),uint2(v.p.xy)); return float4(0.,1.,0.,1.); }";
fn loc() -> TypedImageCopyLocation {
    TypedImageCopyLocation::Texture {
        mip: 0,
        slice: 0,
        origin: [0, 0],
    }
}
fn seed(
    c: &mut RenderCohort,
    format: TextureFormat,
    large: bool,
) -> (RenderPoint, crate::RenderTypedTextureView) {
    let desc = TypedTextureDesc {
        format,
        extent: if large { [4, 2] } else { [2, 1] },
        mip_levels: if large { 2 } else { 1 },
        array_layers: Some(if large { 2 } else { 1 }),
    };
    let bytes = if format == TextureFormat::R32Uint {
        [0x78, 0x56, 0x34, 0x12].repeat(if large { 20 } else { 2 })
    } else {
        [17, 53, 127, 193].repeat(if large { 20 } else { 2 })
    };
    let p = c
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
                source_location: loc(),
                destination_location: loc(),
                extent: desc.extent,
            },
            &[],
        )
        .unwrap();
    let crate::RenderCopyOutput::TypedTexture(root) = c.copy_output(&p).unwrap() else {
        panic!("typed")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: u32::from(large),
            base_mip: u32::from(large),
            mip_count: 1,
            format: if format == TextureFormat::R32Uint {
                crate::TextureViewFormat::R32Uint
            } else {
                crate::TextureViewFormat::Rgba8Unorm
            },
        })
        .unwrap();
    (p, view)
}
fn render(
    c: &mut RenderCohort,
    view: crate::RenderTypedTextureView,
) -> Result<RenderPoint, MetalError> {
    let entry = if view.output.desc.format == TextureFormat::R32Uint {
        "psu"
    } else {
        "ps"
    };
    c.submit_with_storage(
        RenderQueue::Second,
        &[TriangleShader {
            source: SHADER,
            entries: ["vs", entry],
        }],
        &crate::TypedTriangleTargetsPass {
            draws: &[crate::TriangleDraw {
                vertices: &POS,
                colors: &[[0.; 4]; 3],
                viewport: [2, 1],
            }],
            states: &[crate::TriangleState::default()],
            extent: [2, 1],
            colors: &[crate::TypedColorAttachment {
                format: TextureFormat::Rgba8Unorm,
                clear: crate::TypedColorClear::Float([0.; 4]),
                blend_enabled: false,
            }],
            clear_depth: None,
        },
        &[vec![]],
        None,
        &[QueuedColorLoad::Clear],
        &[],
        &[DrawUniforms::default()],
        &[None],
        QueuedStorageInput::ProducedTypedView(view),
    )
}
struct Gate {
    event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    done: Option<std::sync::mpsc::Sender<()>>,
    rescue: Option<std::thread::JoinHandle<bool>>,
}
impl Gate {
    fn new(gpu: &NativeCompute, c: &RenderCohort) -> Self {
        let event = gpu.device().newSharedEvent().unwrap();
        let rescue = event.clone();
        let (done, receive) = std::sync::mpsc::channel();
        let rescue = std::thread::spawn(move || {
            if receive
                .recv_timeout(std::time::Duration::from_secs(10))
                .is_err()
            {
                rescue.setSignaledValue(1);
                true
            } else {
                false
            }
        });
        let command = c.queue(RenderQueue::First).commandBuffer().unwrap();
        command.encodeWaitForEvent_value(ProtocolObject::from_ref(&*event), 1);
        command.commit();
        Self {
            event,
            done: Some(done),
            rescue: Some(rescue),
        }
    }
    fn release(&mut self) -> bool {
        self.event.setSignaledValue(1);
        if let Some(done) = self.done.take() {
            let _ = done.send(());
        }
        self.rescue.take().is_some_and(|t| t.join().unwrap())
    }
}
impl Drop for Gate {
    fn drop(&mut self) {
        self.release();
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_storage_gate_and_literal_float_uint_preservation() {
    let gpu = NativeCompute::new().unwrap();
    for format in [TextureFormat::Rgba8Unorm, TextureFormat::R32Uint] {
        let mut c = gpu.new_render_graph_cohort().unwrap();
        let mut gate = Gate::new(&gpu, &c);
        let (producer, view) = seed(&mut c, format, true);
        let consumer = render(&mut c, view).unwrap();
        let output = c.storage_output(&consumer).unwrap();
        assert_eq!(output.desc().extent, [2, 1]);
        let marker = gpu.device().newSharedEvent().unwrap();
        let command = c.queue(RenderQueue::Second).commandBuffer().unwrap();
        command.encodeSignalEvent_value(ProtocolObject::from_ref(&*marker), 1);
        command.commit();
        std::thread::sleep(std::time::Duration::from_millis(250));
        let early = marker.signaledValue() != 0;
        let held = gate.event.signaledValue() == 0;
        let rescued = gate.release();
        assert!(held && !rescued, "watchdog invalidated gate");
        assert!(!early, "storage consumer ran before producer release");
        c.collect_copy(&producer).unwrap();
        drop(producer);
        assert_eq!(
            c.collect(&consumer).unwrap().colors,
            vec![[0, 255, 0, 255].repeat(2)]
        );
        drop(consumer);
        let desc = output.desc();
        let destination = [0u8; 8];
        let copy = c
            .submit_typed_image_copy(
                RenderQueue::Second,
                QueuedTypedImageCopy {
                    source: TypedCopyInput::ProducedTexture(output),
                    destination: TypedCopyInput::HostTexture {
                        desc,
                        bytes: &destination,
                    },
                    source_location: loc(),
                    destination_location: loc(),
                    extent: [2, 1],
                },
                &[],
            )
            .unwrap();
        let crate::OwnedCopyOutput::TypedTexture { bytes, .. } = c.collect_copy(&copy).unwrap()
        else {
            panic!("typed")
        };
        assert_eq!(
            bytes,
            if format == TextureFormat::R32Uint {
                vec![0x45, 0x23, 0xc1, 0x7f, 0x78, 0x56, 0x34, 0x12]
            } else {
                vec![255, 0, 0, 255, 17, 53, 127, 193]
            }
        );
    }
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_storage_full_root_and_selected_output_leases_survive_reset() {
    let gpu = NativeCompute::new().unwrap();
    let mut charges = Vec::new();
    for large in [false, true] {
        let mut c = gpu.new_render_graph_cohort().unwrap();
        let budget = c.color_bytes.clone();
        let (p, v) = seed(&mut c, TextureFormat::Rgba8Unorm, large);
        c.collect_copy(&p).unwrap();
        drop(p);
        let p = render(&mut c, v).unwrap();
        let out = c.storage_output(&p).unwrap();
        assert_eq!(out.lease.bytes, 8);
        c.collect(&p).unwrap();
        drop(p);
        c.reinitialize().unwrap();
        charges.push(budget.get());
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - budget.get()).unwrap();
        assert!(ColorLease::reserve(&budget, 1).is_err());
        drop(filler);
        drop(out);
        assert_eq!(
            budget.get(),
            0,
            "last output owner refunds root and selected output"
        );
    }
    assert_eq!(
        charges[1] - charges[0],
        72,
        "80-byte root vs8-byte root, identical selected output and render scratch"
    );
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_storage_failed_source_and_budget_reject_without_admission() {
    let gpu = NativeCompute::new().unwrap();
    let mut c = gpu.new_render_graph_cohort().unwrap();
    let budget = c.color_bytes.clone();
    let (p, v) = seed(&mut c, TextureFormat::Rgba8Unorm, true);
    p.inner.command.waitUntilCompleted();
    let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - budget.get() - 7).unwrap();
    let charged = budget.get();
    let sequence = c.sequence;
    assert!(render(&mut c, v.clone()).is_err());
    assert_eq!(budget.get(), charged);
    assert_eq!(c.sequence, sequence);
    drop(filler);
    p.inner.terminal.set(Some(false));
    let charged = budget.get();
    assert!(render(&mut c, v.clone()).is_err());
    assert_eq!(budget.get(), charged);
    assert_eq!(c.sequence, sequence);
    c.abort_and_drain("controlled storage source failure")
        .unwrap();
    drop(v);
    drop(p);
    assert_eq!(budget.get(), 0);
}
