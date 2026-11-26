// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent worker-boundary tests. These require native Metal; no CPU GPU mock.
use super::*;
use xenora_loader::graphics::{
    GraphicsAttachmentRoot, GraphicsDescriptorBindings, GraphicsDescriptorRequirements,
    GraphicsPipeline, GraphicsQueuedDraw, GraphicsSelectedColor, GraphicsTextureSnapshot,
    GraphicsTextureViewDesc, GraphicsTextureViewFormat, GraphicsTypedColorAttachment,
    GraphicsTypedColorClear, GraphicsTypedTextureDesc,
};

fn point(value: u64) -> GraphicsQueuePoint {
    GraphicsQueuePoint {
        queue: GraphicsHandle {
            id: 71,
            generation: 1,
        },
        value,
    }
}
fn desc() -> GraphicsTypedTextureDesc {
    GraphicsTypedTextureDesc {
        format: TextureFormat::Rgba8Unorm,
        extent: (2, 1),
        mip_levels: 1,
        array_layers: None,
    }
}
fn sampled_pipeline() -> Arc<GraphicsPipeline> {
    let c = xenora_shader::compile_graphics_pipeline_typed4(
        xenora_shader::Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-descriptor-cross-ps.bin"),
        Default::default(),
    )
    .unwrap();
    // Independent fixture contract: (t0,s3), (t1,s2), (t2,s1), (t3,s0).
    assert_eq!(c.sampled_pairs, 0x1248);
    assert_eq!(c.descriptor_index_uniform_mask, 0);
    assert_eq!(c.required_uniform_mask, 0);
    Arc::new(GraphicsPipeline {
        descriptor_requirements: Some(GraphicsDescriptorRequirements {
            types: c.types,
            sampled_pairs: 0x1248,
            array_layers: Default::default(),
            descriptor_index_uniform_mask: 0,
        }),
        storage: None,
        array_layers: Default::default(),
        types: Default::default(),
        required_uniform_mask: 0,
        msl: c.shader.msl,
        color_targets: 1,
        uses_texture: true,
        sampled_pairs: 0,
    })
}
fn request(
    pipeline: Arc<GraphicsPipeline>,
    producer: GraphicsProducedVersion,
) -> GraphicsAttachmentTransactionRequest {
    let mut bindings = GraphicsDescriptorBindings::default();
    for slot in 0..4 {
        bindings.textures[slot] = Some(GraphicsTextureInput::Host(GraphicsTextureSnapshot {
            array_layers: None,
            format: TextureFormat::Rgba8Unorm,
            width: 2,
            height: 1,
            pixels: Arc::from([255u8, 21, 43, 65, 255, 21, 43, 65]),
            mip_levels: 1,
        }));
        bindings.views[slot] = Some(GraphicsTextureViewDesc {
            base_mip: 0,
            mip_count: 1,
            format: GraphicsTextureViewFormat::Rgba8Unorm,
        });
        bindings.samplers[slot] = Some(xenora_texture::SamplerDesc::nearest_clamp());
    }
    bindings.textures[3] = Some(GraphicsTextureInput::ProducedTyped {
        version: producer,
        desc: desc(),
    });
    GraphicsAttachmentTransactionRequest {
        generation: 1,
        point: point(2),
        waits: vec![producer.point],
        extent: (2, 1),
        depth: None,
        storage: None,
        roots: vec![GraphicsAttachmentRoot {
            desc: desc(),
            destination_base: None,
            prior: GraphicsAttachmentRootInput::Host(Arc::from([17u8; 8])),
            output: GraphicsProducedVersion {
                point: point(2),
                output: GraphicsProducedOutput::AttachmentRoot(0),
            },
        }],
        colors: vec![GraphicsSelectedColor {
            root_index: 0,
            mip: 0,
            slice: 0,
            load: false,
            attachment: GraphicsTypedColorAttachment {
                format: TextureFormat::Rgba8Unorm,
                clear: GraphicsTypedColorClear::Float([0., 0., 0., 0.]),
                blend_enabled: false,
            },
        }],
        draws: vec![GraphicsQueuedDraw {
            descriptor_bindings: Some(bindings),
            sampled_ranges: [None; 2],
            array_views: [None; 2],
            views: [None; 2],
            geometry: None,
            uniforms: None,
            vertices: vec![[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]],
            colors: vec![[0.5, 0.5, 0., 1.]; 3],
            pipeline: Some(pipeline),
            textures: [None, None],
            samplers: [None; 2],
            viewport: (2, 1),
            state: GraphicsDrawState::default(),
        }],
    }
}
fn source(state: &mut State) -> (GraphicsProducedVersion, mpsc::Receiver<Completion>) {
    let version = GraphicsProducedVersion {
        point: point(1),
        output: GraphicsProducedOutput::Copy(0),
    };
    let (sender, receiver) = mpsc::channel();
    state
        .submit_copy(
            &GraphicsQueuedCopyRequest {
                generation: 1,
                point: version.point,
                waits: vec![],
                layout: GraphicsCopyLayout::Rgba8 { extent: (2, 1) },
                source: GraphicsCopyInput::Host(Arc::from([255u8, 21, 43, 65, 255, 21, 43, 65])),
                output: version,
            },
            sender,
        )
        .unwrap();
    (version, receiver)
}
fn completion(
    state: &mut State,
    receiver: &mpsc::Receiver<Completion>,
) -> GraphicsQueuedWorkOutput {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        state.progress();
        match receiver.try_recv() {
            Ok(v) => return v.unwrap(),
            Err(mpsc::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "native descriptor test completion timeout"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => panic!("completion disconnected: {e}"),
        }
    }
}
fn assert_rejected_without_admission(
    state: &mut State,
    request: &GraphicsAttachmentTransactionRequest,
) {
    let entries = state.entries.len();
    let queues = state.queues.clone();
    let generation = state.generation;
    let (sender, receiver) = mpsc::channel();
    assert!(state.submit_attachments(request, sender).is_err());
    assert!(
        !state.poisoned,
        "invalid descriptor must reject before host seed, not admit then drain"
    );
    assert_eq!(state.entries.len(), entries, "no new logical admission");
    assert!(!state.entries.contains_key(&request.point));
    assert_eq!(state.queues, queues, "no queue mapping consumed");
    assert_eq!(state.generation, generation);
    assert!(
        state.entries[&point(1)].completed.is_some(),
        "producer must not be failed/drained by invalid consumer"
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Disconnected)
    ));
}
#[test]
fn queued_descriptor_produced_slot3_invalid_capabilities_reject_before_host_seed() {
    let pipeline = sampled_pipeline();
    for case in [
        "missing-wait",
        "stale-point",
        "wrong-slot",
        "same-bytes-wrong-layout",
    ] {
        let mut state = State::new().unwrap();
        let (producer, producer_result) = source(&mut state);
        let good = request(pipeline.clone(), producer);
        let mut bad = request(good.draws[0].pipeline.as_ref().unwrap().clone(), producer);
        let bindings = bad.draws[0].descriptor_bindings.as_mut().unwrap();
        let Some(GraphicsTextureInput::ProducedTyped { version, desc }) = &mut bindings.textures[3]
        else {
            panic!("slot3 produced fixture")
        };
        match case {
            "missing-wait" => bad.waits.clear(),
            "stale-point" => {
                version.point.value = 999;
                bad.waits = vec![version.point];
            }
            "wrong-slot" => version.output = GraphicsProducedOutput::Copy(1),
            "same-bytes-wrong-layout" => desc.extent = (1, 2),
            _ => unreachable!(),
        }
        // Local shape/type validation must not substitute for capability lookup.
        assert!(
            validate(&bad).is_ok(),
            "{case}: otherwise rejection is unrelated to produced preflight"
        );
        assert_rejected_without_admission(&mut state, &bad);
        let (sender, result) = mpsc::channel();
        state.submit_attachments(&good, sender).unwrap();
        let GraphicsQueuedWorkOutput::AttachmentTransaction(roots) =
            completion(&mut state, &result)
        else {
            panic!("root result")
        };
        // Each exported component is the red=255 channel of a distinct sampled slot.
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].bytes, [255u8; 8],
            "{case}: unchanged point/generation retry"
        );
        let GraphicsQueuedWorkOutput::Copy { bytes, .. } = completion(&mut state, &producer_result)
        else {
            panic!("producer copy")
        };
        assert_eq!(bytes, [255, 21, 43, 65, 255, 21, 43, 65]);
    }
}
#[test]
fn queued_descriptor_missing_nonselector_uniform_rejects_before_host_seed() {
    let mut state = State::new().unwrap();
    let (producer, producer_result) = source(&mut state);
    let mut pipeline = sampled_pipeline();
    // Exercise the worker's ordinary-uniform contract independently of selector
    // validation. The sampler fixture needs no selector, and expected pixels stay literal.
    Arc::get_mut(&mut pipeline).unwrap().required_uniform_mask = 0x80;
    let mut good = request(pipeline, producer);
    good.draws[0].uniforms = Some([0; 8]);
    let mut bad = request(good.draws[0].pipeline.as_ref().unwrap().clone(), producer);
    bad.draws[0].uniforms = None;
    assert_rejected_without_admission(&mut state, &bad);
    let (sender, result) = mpsc::channel();
    state.submit_attachments(&good, sender).unwrap();
    let GraphicsQueuedWorkOutput::AttachmentTransaction(roots) = completion(&mut state, &result)
    else {
        panic!("root result")
    };
    assert_eq!(roots[0].bytes, [255u8; 8]);
    assert!(matches!(
        completion(&mut state, &producer_result),
        GraphicsQueuedWorkOutput::Copy { .. }
    ));
}
