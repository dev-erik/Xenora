// SPDX-License-Identifier: GPL-3.0-or-later
//! Logical payload ownership; this is not a measurement of driver memory.
use super::*;
use crate::{
    ArraySliceViewDesc, QueuedTypedImageCopy, TypedCopyInput, TypedImageCopyLocation,
    TypedTextureDesc,
};

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_attachment_view_load_retains_full_root_and_refunds_after_reset() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    // Two layers each contain eight base texels and two mip1 texels:80 bytes.
    let bytes = [17, 53, 127, 193].repeat(20);
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
    let crate::RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&producer).unwrap() else {
        panic!("typed output")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 1,
            mip_count: 1,
            format: crate::TextureViewFormat::Rgba8Unorm,
        })
        .unwrap();
    cohort.collect_copy(&producer).unwrap();
    drop(producer);
    drop(root);
    let source_charge = budget.get();
    assert!(
        source_charge >= 80,
        "a selected8-byte view retains its full80-byte root"
    );
    let colors = [crate::TypedColorAttachment {
        format: TextureFormat::Rgba8Unorm,
        clear: crate::TypedColorClear::Float([0.; 4]),
        blend_enabled: false,
    }];
    let pass = crate::TypedTriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [2, 1],
        colors: &colors,
        clear_depth: None,
    };
    let consumer = cohort
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[],
            &pass,
            &[],
            None,
            &[crate::QueuedColorLoad::ProducedTypedView(view.clone())],
            &[],
            &[],
            &[],
        )
        .unwrap();
    drop(view);
    assert!(
        budget.get() >= 88,
        "consumer keeps the full root and fresh selected output"
    );
    assert_eq!(
        cohort.collect(&consumer).unwrap().colors,
        vec![[17, 53, 127, 193].repeat(2)]
    );
    cohort.reinitialize().unwrap();
    assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
    let held = budget.get();
    assert!(
        held >= 88,
        "reset cannot forgive a command's retained root/output"
    );
    let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - held).unwrap();
    let sequence = cohort.sequence;
    assert!(
        cohort
            .submit_with_typed_attachments(
                RenderQueue::First,
                &[],
                &pass,
                &[],
                None,
                &[crate::QueuedColorLoad::Clear],
                &[],
                &[],
                &[]
            )
            .is_err()
    );
    assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
    assert_eq!(
        cohort.sequence, sequence,
        "rejected allocation must not publish a point"
    );
    drop(filler);
    drop(consumer);
    assert_eq!(
        budget.get(),
        0,
        "final owner drop refunds full root and temporary charges"
    );
    let recovery = cohort
        .submit_with_typed_attachments(
            RenderQueue::First,
            &[],
            &pass,
            &[],
            None,
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    cohort.collect(&recovery).unwrap();
    drop(recovery);
    assert_eq!(budget.get(), 0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_attachment_view_load_rejects_controlled_failed_producer_without_publication() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let desc = TypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    let bytes = [17, 53, 127, 193].repeat(20);
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
    let crate::RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&producer).unwrap() else {
        panic!("typed")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 1,
            mip_count: 1,
            format: crate::TextureViewFormat::Rgba8Unorm,
        })
        .unwrap();
    // Complete actual GPU writes before injecting failure metadata. This verifies
    // cross-queue dependency rejection, not device-fault recovery.
    producer.inner.command.waitUntilCompleted();
    producer.inner.terminal.set(Some(false));
    let sequence = cohort.sequence;
    let charged = budget.get();
    let colors = [crate::TypedColorAttachment {
        format: TextureFormat::Rgba8Unorm,
        clear: crate::TypedColorClear::Float([0.; 4]),
        blend_enabled: false,
    }];
    let pass = crate::TypedTriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [2, 1],
        colors: &colors,
        clear_depth: None,
    };
    assert!(
        cohort
            .submit_with_typed_attachments(
                RenderQueue::Second,
                &[],
                &pass,
                &[],
                None,
                &[crate::QueuedColorLoad::ProducedTypedView(view.clone())],
                &[],
                &[],
                &[]
            )
            .is_err()
    );
    assert_eq!(cohort.sequence, sequence);
    assert_eq!(budget.get(), charged);
    assert_eq!(cohort.entries.iter().flatten().count(), 1);
    assert!(cohort.generation.poisoned.get());
    cohort.abort_and_drain("controlled source failure").unwrap();
    drop(view);
    drop(root);
    drop(producer);
    assert_eq!(budget.get(), 0);
    cohort.reinitialize().unwrap();
    let recovery = cohort
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[],
            &pass,
            &[],
            None,
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    cohort.collect(&recovery).unwrap();
    drop(recovery);
    assert_eq!(budget.get(), 0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_depth_attachment_view_load_retains_full_root_and_refunds_after_reset() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let desc = TypedTextureDesc {
        format: TextureFormat::Depth32Float,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    // Two layers each contain eight base texels and two mip1 texels:80 bytes.
    let bytes = 0.5f32.to_le_bytes().repeat(20);
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
    let crate::RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&producer).unwrap() else {
        panic!("typed output")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 1,
            mip_count: 1,
            format: crate::TextureViewFormat::Depth32Float,
        })
        .unwrap();
    cohort.collect_copy(&producer).unwrap();
    drop(producer);
    drop(root);
    let source_charge = budget.get();
    assert!(
        source_charge >= 80,
        "a selected8-byte view retains its full80-byte root"
    );
    let colors = [crate::TypedColorAttachment {
        format: TextureFormat::Rgba8Unorm,
        clear: crate::TypedColorClear::Float([0.; 4]),
        blend_enabled: false,
    }];
    let pass = crate::TypedTriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [2, 1],
        colors: &colors,
        clear_depth: None,
    };
    // Measure the same output/scratch ownership without a source root. A low
    // absolute threshold could be satisfied by scratch alone after losing it.
    let mut baseline = gpu.new_render_graph_cohort().unwrap();
    let baseline_budget = baseline.color_bytes.clone();
    let baseline_point = baseline
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[],
            &pass,
            &[],
            Some(crate::QueuedDepthInput::Clear(0.5)),
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    baseline.collect(&baseline_point).unwrap();
    baseline.reinitialize().unwrap();
    let output_charge = baseline_budget.get();
    drop(baseline_point);
    assert_eq!(baseline_budget.get(), 0);
    let consumer = cohort
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[],
            &pass,
            &[],
            Some(crate::QueuedDepthInput::ProducedTypedView(view.clone())),
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    drop(view);
    assert!(
        budget.get() >= 96,
        "consumer keeps the full depth root and fresh color/depth outputs"
    );
    assert_eq!(cohort.collect(&consumer).unwrap().colors, vec![vec![0; 8]]);
    cohort.reinitialize().unwrap();
    assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
    let held = budget.get();
    assert_eq!(
        held,
        output_charge + 80,
        "reset cannot forgive the full80-byte root beyond output/scratch charges"
    );
    let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - held).unwrap();
    let sequence = cohort.sequence;
    assert!(
        cohort
            .submit_with_typed_attachments(
                RenderQueue::First,
                &[],
                &pass,
                &[],
                None,
                &[crate::QueuedColorLoad::Clear],
                &[],
                &[],
                &[]
            )
            .is_err()
    );
    assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
    assert_eq!(
        cohort.sequence, sequence,
        "rejected allocation must not publish a point"
    );
    drop(filler);
    drop(consumer);
    assert_eq!(
        budget.get(),
        0,
        "final owner drop refunds full root and temporary charges"
    );
    let recovery = cohort
        .submit_with_typed_attachments(
            RenderQueue::First,
            &[],
            &pass,
            &[],
            None,
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    cohort.collect(&recovery).unwrap();
    drop(recovery);
    assert_eq!(budget.get(), 0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn native_depth_attachment_view_load_rejects_controlled_failed_producer_without_publication() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let budget = cohort.color_bytes.clone();
    let desc = TypedTextureDesc {
        format: TextureFormat::Depth32Float,
        extent: [4, 2],
        mip_levels: 2,
        array_layers: Some(2),
    };
    let bytes = 0.5f32.to_le_bytes().repeat(20);
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
    let crate::RenderCopyOutput::TypedTexture(root) = cohort.copy_output(&producer).unwrap() else {
        panic!("typed")
    };
    let view = root
        .array_slice_view(ArraySliceViewDesc {
            slice: 1,
            base_mip: 1,
            mip_count: 1,
            format: crate::TextureViewFormat::Depth32Float,
        })
        .unwrap();
    // Complete actual GPU writes before injecting failure metadata. This verifies
    // cross-queue dependency rejection, not device-fault recovery.
    producer.inner.command.waitUntilCompleted();
    producer.inner.terminal.set(Some(false));
    let sequence = cohort.sequence;
    let charged = budget.get();
    let colors = [crate::TypedColorAttachment {
        format: TextureFormat::Rgba8Unorm,
        clear: crate::TypedColorClear::Float([0.; 4]),
        blend_enabled: false,
    }];
    let pass = crate::TypedTriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [2, 1],
        colors: &colors,
        clear_depth: None,
    };
    assert!(
        cohort
            .submit_with_typed_attachments(
                RenderQueue::Second,
                &[],
                &pass,
                &[],
                Some(crate::QueuedDepthInput::ProducedTypedView(view.clone())),
                &[crate::QueuedColorLoad::Clear],
                &[],
                &[],
                &[]
            )
            .is_err()
    );
    assert_eq!(cohort.sequence, sequence);
    assert_eq!(budget.get(), charged);
    assert_eq!(cohort.entries.iter().flatten().count(), 1);
    assert!(cohort.generation.poisoned.get());
    cohort.abort_and_drain("controlled source failure").unwrap();
    drop(view);
    drop(root);
    drop(producer);
    assert_eq!(budget.get(), 0);
    cohort.reinitialize().unwrap();
    let recovery = cohort
        .submit_with_typed_attachments(
            RenderQueue::Second,
            &[],
            &pass,
            &[],
            None,
            &[crate::QueuedColorLoad::Clear],
            &[],
            &[],
            &[],
        )
        .unwrap();
    cohort.collect(&recovery).unwrap();
    drop(recovery);
    assert_eq!(budget.get(), 0);
}
