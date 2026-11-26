// SPDX-License-Identifier: GPL-3.0-or-later
//! Selected attachment transactions. Native admission is incremental; guest
//! publication is atomic. Every internal stage uses one queue and is retained
//! until its terminal point completes. Exceptional partial admission drains.
use super::*;
use xenora_loader::graphics::{
    GraphicsAttachmentRootInput, GraphicsAttachmentRootResult,
    GraphicsAttachmentTransactionRequest, GraphicsProducedOutput,
};
use xenora_metal::{
    QueuedTypedImageCopy, RenderTypedTextureOutput, RenderTypedTextureView, TypedCopyInput,
    TypedImageCopyLocation,
};

pub(super) struct Transaction {
    points: Vec<(RenderPoint, Stage)>,
    pub(super) roots: Vec<RenderTypedTextureOutput>,
    versions: Vec<GraphicsProducedVersion>,
}
enum Stage {
    Seed,
    Render,
    Patch(usize),
}
fn location(mip: u32, slice: u32) -> TypedImageCopyLocation {
    TypedImageCopyLocation::Texture {
        mip,
        slice,
        origin: [0, 0],
    }
}
fn layout(
    desc: xenora_loader::graphics::GraphicsTypedTextureDesc,
) -> Result<PackedTextureArrayLayout, GraphicsError> {
    PackedTextureArrayLayout::new(
        desc.format,
        desc.extent.0,
        desc.extent.1,
        desc.mip_levels,
        desc.array_layers.unwrap_or(1),
    )
    .map_err(|_| GraphicsError::InvalidDescriptor)
}
fn view(
    root: &RenderTypedTextureOutput,
    mip: u32,
    slice: u32,
) -> Result<RenderTypedTextureView, GraphicsError> {
    let format = typed_native_view_format(root.desc().format);
    if root.desc().array_layers.is_some() {
        root.array_slice_view(ArraySliceViewDesc {
            slice,
            base_mip: mip,
            mip_count: 1,
            format,
        })
    } else {
        if slice != 0 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        root.view(TextureViewDesc {
            base_mip: mip,
            mip_count: 1,
            format,
        })
    }
    .map_err(metal_error)
}
fn validate(request: &GraphicsAttachmentTransactionRequest) -> Result<usize, GraphicsError> {
    if !(1..=8).contains(&request.colors.len())
        || !(1..=9).contains(&request.roots.len())
        || request.waits.len() > MAX_QUEUED_GRAPH_NODES
    {
        return Err(GraphicsError::InvalidDescriptor);
    }
    for (i, root) in request.roots.iter().enumerate() {
        let packed = layout(root.desc)?;
        if root.output
            != (GraphicsProducedVersion {
                point: request.point,
                output: GraphicsProducedOutput::AttachmentRoot(i as u8),
            })
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if let Some(base) = root.destination_base
            && (!matches!(root.prior, GraphicsAttachmentRootInput::Produced(_))
                || !request.waits.contains(&base.point))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        match &root.prior {
            GraphicsAttachmentRootInput::Host(bytes) if bytes.len() != packed.byte_len() => {
                return Err(GraphicsError::InvalidDescriptor);
            }
            GraphicsAttachmentRootInput::Produced(v) if !request.waits.contains(&v.point) => {
                return Err(GraphicsError::InvalidDescriptor);
            }
            _ => {}
        }
    }
    let mut selected = std::collections::HashSet::new();
    let mut used = std::collections::HashSet::new();
    let mut check =
        |index: usize, mip: u32, slice: u32, format: TextureFormat| -> Result<(), GraphicsError> {
            let root = request
                .roots
                .get(index)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            let footprint = layout(root.desc)?
                .footprint(slice, mip)
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
            if root.desc.format != format
                || (footprint.width, footprint.height) != request.extent
                || !selected.insert((index, mip, slice))
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            used.insert(index);
            Ok(())
        };
    for color in &request.colors {
        if matches!(
            color.attachment.format,
            TextureFormat::Depth32Float | TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
        ) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        check(
            color.root_index,
            color.mip,
            color.slice,
            color.attachment.format,
        )?;
    }
    if let Some(depth) = &request.depth {
        check(
            depth.root_index,
            depth.mip,
            depth.slice,
            TextureFormat::Depth32Float,
        )?;
        if let GraphicsDepthLoad::Clear(value) = depth.load
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
    }
    if used.len() != request.roots.len() {
        return Err(GraphicsError::InvalidDescriptor);
    }
    Ok(1 + request.colors.len()
        + usize::from(request.depth.is_some())
        + request
            .roots
            .iter()
            .filter(|r| !matches!(r.prior, GraphicsAttachmentRootInput::Produced(_)))
            .count())
}

impl State {
    pub(super) fn submit_attachments(
        &mut self,
        request: &GraphicsAttachmentTransactionRequest,
        completed: mpsc::Sender<Completion>,
    ) -> Result<(), GraphicsError> {
        let needed = validate(request)?;
        let occupied: usize = self
            .entries
            .values()
            .filter(|e| e.completed.is_some())
            .map(|e| e.transaction.as_ref().map_or(1, |t| t.points.len()))
            .sum();
        if self.entries.contains_key(&request.point)
            || self.entries.len() >= MAX_QUEUED_GRAPH_NODES
            || occupied
                .checked_add(needed)
                .is_none_or(|n| n > MAX_QUEUED_GRAPH_NODES)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if self.generation != Some(request.generation) {
            if !self.entries.is_empty() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            self.cohort.reinitialize().map_err(metal_error)?;
            self.queues.clear();
            self.generation = Some(request.generation);
            self.poisoned = false;
        }
        if self.poisoned {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let queue = match self.queues.get(&request.point.queue) {
            Some(q) => *q,
            None => match self.queues.len() {
                0 => RenderQueue::First,
                1 => RenderQueue::Second,
                _ => return Err(GraphicsError::ResourceLifetimeViolation),
            },
        };
        let dependencies = request
            .waits
            .iter()
            .map(|p| {
                self.entries
                    .get(p)
                    .map(|e| e.point.clone())
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Resolve every produced capability before committing any host import.
        let resolved = request
            .roots
            .iter()
            .map(|r| match &r.prior {
                GraphicsAttachmentRootInput::Host(_)
                | GraphicsAttachmentRootInput::HostDepth(_) => Ok(None),
                GraphicsAttachmentRootInput::Produced(v) => {
                    let output = produced_typed_output(&self.entries, *v)?;
                    if output.desc() != native_desc(r.desc) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    Ok(Some(output))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Resolve the separately authorized composition base before any native
        // admission. It preserves unrelated writes; it must never replace Load's
        // semantic input above.
        let destinations = request
            .roots
            .iter()
            .map(|root| {
                root.destination_base
                    .map(|version| {
                        let output = produced_typed_output(&self.entries, version)?;
                        if output.desc() != native_desc(root.desc) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        Ok(output)
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, GraphicsError>>()?;
        let host_bytes = request
            .roots
            .iter()
            .map(|root| match &root.prior {
                GraphicsAttachmentRootInput::Host(bytes) => Ok(Some(bytes.clone())),
                GraphicsAttachmentRootInput::Produced(_) => Ok(None),
                GraphicsAttachmentRootInput::HostDepth(snapshot) => {
                    if root.desc.format != TextureFormat::Depth32Float
                        || root.desc.mip_levels != 1
                        || root.desc.array_layers.is_some()
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let native = native_depth_snapshot(snapshot)?;
                    if native.extent() != [root.desc.extent.0, root.desc.extent.1]
                        || native.byte_len() != layout(root.desc)?.byte_len()
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    Ok(Some(Arc::<[u8]>::from(native.to_packed_bytes())))
                }
            })
            .collect::<Result<Vec<_>, GraphicsError>>()?;
        let mut points = Vec::new();
        let result = (|| {
            let mut roots = Vec::new();
            for ((root, produced), host) in request.roots.iter().zip(resolved).zip(&host_bytes) {
                if let Some(output) = produced {
                    roots.push(output);
                    continue;
                }
                let bytes = host.as_ref().ok_or(GraphicsError::InvalidDescriptor)?;
                let desc = native_desc(root.desc);
                let point = self
                    .cohort
                    .submit_typed_image_copy(
                        queue,
                        QueuedTypedImageCopy {
                            source: TypedCopyInput::HostTexture { desc, bytes },
                            destination: TypedCopyInput::HostTexture { desc, bytes },
                            source_location: location(0, 0),
                            destination_location: location(0, 0),
                            extent: desc.extent,
                        },
                        &dependencies,
                    )
                    .map_err(metal_error)?;
                points.push((point.clone(), Stage::Seed));
                let RenderCopyOutput::TypedTexture(output) =
                    self.cohort.copy_output(&point).map_err(metal_error)?
                else {
                    return Err(GraphicsError::InvalidDescriptor);
                };
                roots.push(output);
            }
            let loads = request
                .colors
                .iter()
                .map(|c| {
                    if c.load {
                        view(&roots[c.root_index], c.mip, c.slice)
                            .map(QueuedColorLoad::ProducedTypedView)
                    } else {
                        Ok(QueuedColorLoad::Clear)
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            let depth = request
                .depth
                .as_ref()
                .map(|d| match d.load {
                    GraphicsDepthLoad::Clear(v) => Ok(QueuedDepthInput::Clear(v)),
                    GraphicsDepthLoad::Load => view(&roots[d.root_index], d.mip, d.slice)
                        .map(QueuedDepthInput::ProducedTypedView),
                })
                .transpose()?;
            let render = GraphicsQueuedSubmissionRequest {
                color_types: request.colors.iter().map(|c| Some(c.attachment)).collect(),
                generation: request.generation,
                point: request.point,
                waits: request.waits.clone(),
                draws: request.draws.clone(),
                extent: request.extent,
                colors: request
                    .colors
                    .iter()
                    .map(|_| GraphicsQueuedColorInput::Clear([0; 4]))
                    .collect(),
                clear_depth: 1.0,
                depth: request
                    .depth
                    .as_ref()
                    .map(|_| GraphicsQueuedDepthInput::Clear(1.0)),
                outputs: (0..request.colors.len())
                    .map(|slot| GraphicsProducedVersion {
                        point: request.point,
                        output: GraphicsProducedOutput::Color(slot as u8),
                    })
                    .collect(),
                depth_output: request.depth.as_ref().map(|_| GraphicsProducedVersion {
                    point: request.point,
                    output: GraphicsProducedOutput::Depth,
                }),
            };
            let rendered = submit_native_inputs(
                &self.gpu,
                &mut self.cohort,
                queue,
                &render,
                &self.entries,
                &dependencies,
                Some(AttachmentInputs {
                    colors: loads,
                    depth,
                }),
            )?;
            points.push((rendered.clone(), Stage::Render));
            let mut patches = request
                .colors
                .iter()
                .enumerate()
                .map(|(slot, c)| {
                    self.cohort
                        .typed_color_output(&rendered, slot)
                        .map_err(metal_error)
                        .map(|o| (c.root_index, c.mip, c.slice, o))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(d) = &request.depth {
                let output = self
                    .cohort
                    .depth_output(&rendered)
                    .map_err(metal_error)?
                    .typed_output()
                    .map_err(metal_error)?;
                patches.push((d.root_index, d.mip, d.slice, output));
            }
            for (root, destination) in roots.iter_mut().zip(destinations) {
                if let Some(destination) = destination {
                    *root = destination;
                }
            }
            for (index, mip, slice, output) in patches {
                let point = self
                    .cohort
                    .submit_typed_image_copy(
                        queue,
                        QueuedTypedImageCopy {
                            source: TypedCopyInput::ProducedTexture(output),
                            destination: TypedCopyInput::ProducedTexture(roots[index].clone()),
                            source_location: location(0, 0),
                            destination_location: location(mip, slice),
                            extent: [request.extent.0, request.extent.1],
                        },
                        &[],
                    )
                    .map_err(metal_error)?;
                points.push((point.clone(), Stage::Patch(index)));
                let RenderCopyOutput::TypedTexture(output) =
                    self.cohort.copy_output(&point).map_err(metal_error)?
                else {
                    return Err(GraphicsError::InvalidDescriptor);
                };
                roots[index] = output;
                #[cfg(test)]
                if self.fail_after_attachment_patch
                    == Some(
                        points
                            .iter()
                            .filter(|(_, stage)| matches!(stage, Stage::Patch(_)))
                            .count(),
                    )
                {
                    return Err(GraphicsError::BackendError(
                        "controlled failure after committed patch".into(),
                    ));
                }
            }
            Ok(roots)
        })();
        let roots = match result {
            Ok(roots) => roots,
            Err(error) => {
                if !points.is_empty() {
                    self.fail(error.clone());
                }
                return Err(error);
            }
        };
        // Same-queue ordering makes this point the fence for every stage/root.
        let point = points
            .last()
            .expect("nonempty validated transaction")
            .0
            .clone();
        self.queues.insert(request.point.queue, queue);
        self.entries.insert(
            request.point,
            Entry {
                point,
                outputs: Vec::new(),
                typed_outputs: Vec::new(),
                depth_output: None,
                copy_output: None,
                completed: Some(completed),
                transaction: Some(Transaction {
                    points,
                    roots,
                    versions: request.roots.iter().map(|r| r.output).collect(),
                }),
            },
        );
        Ok(())
    }
}

pub(super) fn collect(cohort: &mut RenderCohort, transaction: &Transaction) -> Completion {
    let mut results: Vec<Option<Vec<u8>>> = vec![None; transaction.roots.len()];
    for (point, stage) in &transaction.points {
        match stage {
            Stage::Render => {
                cohort.collect(point).map_err(metal_error)?;
            }
            Stage::Seed => {
                cohort.collect_copy(point).map_err(metal_error)?;
            }
            Stage::Patch(index) => {
                let OwnedCopyOutput::TypedTexture { desc, bytes } =
                    cohort.collect_copy(point).map_err(metal_error)?
                else {
                    return Err(GraphicsError::InvalidDescriptor);
                };
                if desc != transaction.roots[*index].desc()
                    || bytes.len() != layout(loader_desc(desc))?.byte_len()
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                results[*index] = Some(bytes);
            }
        }
    }
    let results = results
        .into_iter()
        .enumerate()
        .map(|(i, bytes)| {
            Ok(GraphicsAttachmentRootResult {
                output: transaction.versions[i],
                desc: loader_desc(transaction.roots[i].desc()),
                bytes: bytes.ok_or(GraphicsError::InvalidDescriptor)?,
            })
        })
        .collect::<Result<Vec<_>, GraphicsError>>()?;
    Ok(GraphicsQueuedWorkOutput::AttachmentTransaction(results))
}

#[cfg(all(test, feature = "gpu-tests"))]
mod tests {
    use super::*;
    use xenora_loader::graphics::{
        GraphicsAttachmentRoot, GraphicsQueuedDraw, GraphicsSelectedColor,
        GraphicsTypedColorAttachment, GraphicsTypedColorClear, GraphicsTypedTextureDesc,
    };
    fn request(generation: u64, invalid_draw: bool) -> GraphicsAttachmentTransactionRequest {
        let point = GraphicsQueuePoint {
            queue: GraphicsHandle {
                id: 1,
                generation: 1,
            },
            value: generation,
        };
        GraphicsAttachmentTransactionRequest {
            generation,
            point,
            waits: vec![],
            extent: (2, 1),
            depth: None,
            roots: vec![GraphicsAttachmentRoot {
                desc: GraphicsTypedTextureDesc {
                    format: TextureFormat::Rgba8Unorm,
                    extent: (4, 2),
                    mip_levels: 2,
                    array_layers: Some(2),
                },
                destination_base: None,
                prior: GraphicsAttachmentRootInput::Host(Arc::from([17u8; 80])),
                output: GraphicsProducedVersion {
                    point,
                    output: GraphicsProducedOutput::AttachmentRoot(0),
                },
            }],
            colors: vec![GraphicsSelectedColor {
                root_index: 0,
                mip: 1,
                slice: 1,
                load: false,
                attachment: GraphicsTypedColorAttachment {
                    format: TextureFormat::Rgba8Unorm,
                    clear: GraphicsTypedColorClear::Float([1., 0., 0., 1.]),
                    blend_enabled: false,
                },
            }],
            draws: if invalid_draw {
                vec![GraphicsQueuedDraw {
                    sampled_ranges: [None; 2],
                    array_views: [None; 2],
                    geometry: None,
                    uniforms: None,
                    views: [None; 2],
                    vertices: vec![[0.; 4]; 3],
                    colors: vec![[0.; 4]; 3],
                    pipeline: None,
                    textures: [None, None],
                    samplers: [None; 2],
                    viewport: (2, 1),
                    state: GraphicsDrawState::default(),
                }]
            } else {
                vec![]
            },
        }
    }
    #[test]
    fn queued_attachment_transaction_partial_admission_drains_without_success_and_recovers() {
        let mut state = State::new().unwrap();
        let (completed, result) = mpsc::channel();
        // Host import commits before render preparation rejects the missing
        // pipeline. This is controlled setup failure, not a hardware fault.
        assert!(
            state
                .submit_attachments(&request(1, true), completed)
                .is_err()
        );
        assert!(
            state.poisoned,
            "a committed host seed requires failure drainage"
        );
        assert!(
            state.entries.is_empty(),
            "no partially admitted logical output"
        );
        assert!(matches!(
            result.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        let (completed, _) = mpsc::channel();
        assert!(
            state
                .submit_attachments(&request(1, false), completed)
                .is_err()
        );
        // A new generation can reuse the drained cohort without leaking a slot.
        let (completed, result) = mpsc::channel();
        state
            .submit_attachments(&request(2, false), completed)
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let output = loop {
            state.progress();
            match result.try_recv() {
                Ok(v) => break v.unwrap(),
                Err(mpsc::TryRecvError::Empty) => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "transaction completion timeout"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("completion disconnected: {e}"),
            }
        };
        let GraphicsQueuedWorkOutput::AttachmentTransaction(roots) = output else {
            panic!("transaction result")
        };
        let mut expected = [17u8; 80];
        expected[72..80].copy_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255]);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].bytes, expected);
    }
    #[test]
    fn queued_attachment_transaction_each_copyback_failure_rejects_all_outputs() {
        for fail_after in 1..=2 {
            let mut state = State::new().unwrap();
            let (prior_sender, prior_receiver) = mpsc::channel();
            state
                .submit_attachments(&request(1, false), prior_sender)
                .unwrap();
            let mut bad = request(1, false);
            bad.point.value = 2;
            bad.roots[0].output.point = bad.point;
            bad.colors.push(bad.colors[0].clone());
            bad.colors[0].slice = 0;
            state.fail_after_attachment_patch = Some(fail_after);
            let (sender, receiver) = mpsc::channel();
            assert!(state.submit_attachments(&bad, sender).is_err());
            assert!(state.poisoned);
            assert!(!state.entries.contains_key(&bad.point));
            assert!(matches!(
                receiver.try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ));
            assert!(
                prior_receiver
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .is_err(),
                "cohort failure invalidates other unpublished tickets"
            );
            state.entries.clear(); // Simulate release of the failed logical receipt.
            state.fail_after_attachment_patch = None;
            let (sender, receiver) = mpsc::channel();
            let recovery = request(2, false);
            state.submit_attachments(&recovery, sender).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                state.progress();
                match receiver.try_recv() {
                    Ok(result) => {
                        assert!(result.is_ok());
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    Err(e) => panic!("recovery disconnected: {e}"),
                }
            }
        }
    }
    #[test]
    fn queued_attachment_transaction_capacity_counts_internal_points_before_admission() {
        let mut state = State::new().unwrap();
        let mut receivers = Vec::new();
        for value in 1..=5 {
            let mut r = request(1, false);
            r.point.value = value;
            r.roots[0].output.point = r.point;
            let (sender, receiver) = mpsc::channel();
            state.submit_attachments(&r, sender).unwrap();
            receivers.push(receiver);
        }
        // Five receipts retain fifteen native stages. A sixth needs three,
        // despite the logical receipt count remaining below sixteen.
        let mut sixth = request(1, false);
        sixth.point.value = 6;
        sixth.roots[0].output.point = sixth.point;
        let (sender, receiver) = mpsc::channel();
        assert!(state.submit_attachments(&sixth, sender).is_err());
        assert!(
            !state.poisoned,
            "capacity preflight must not partially admit"
        );
        assert_eq!(state.entries.len(), 5);
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while state
            .entries
            .values()
            .any(|entry| entry.completed.is_some())
        {
            state.progress();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        for receiver in receivers {
            assert!(receiver.recv().unwrap().is_ok());
        }
        let (sender, _receiver) = mpsc::channel();
        state.submit_attachments(&sixth, sender).unwrap();
        state.fail(GraphicsError::BackendError("test cleanup".into()));
    }
    #[test]
    fn queued_attachment_transaction_opaque_depth_requires_exact_shape() {
        use xenora_loader::graphics::GraphicsSelectedDepth;
        let mut state = State::new().unwrap();
        let mut r = request(1, false);
        let bytes = [0u8, 0, 128, 62, 0, 0, 64, 63]; // .25 and .75
        r.roots.push(GraphicsAttachmentRoot {
            desc: GraphicsTypedTextureDesc {
                format: TextureFormat::Depth32Float,
                extent: (2, 1),
                mip_levels: 1,
                array_layers: None,
            },
            destination_base: None,
            prior: GraphicsAttachmentRootInput::HostDepth(Arc::new(
                DepthSnapshot::from_packed_bytes([1, 2], &bytes).unwrap(),
            )),
            output: GraphicsProducedVersion {
                point: r.point,
                output: GraphicsProducedOutput::AttachmentRoot(1),
            },
        });
        r.depth = Some(GraphicsSelectedDepth {
            root_index: 1,
            mip: 0,
            slice: 0,
            load: GraphicsDepthLoad::Load,
        });
        let (sender, _) = mpsc::channel();
        assert!(state.submit_attachments(&r, sender).is_err());
        assert!(!state.poisoned);
        assert!(state.entries.is_empty());
        r.roots[1].prior = GraphicsAttachmentRootInput::HostDepth(Arc::new(
            DepthSnapshot::from_packed_bytes([2, 1], &bytes).unwrap(),
        ));
        let (sender, receiver) = mpsc::channel();
        state.submit_attachments(&r, sender).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let result = loop {
            state.progress();
            if let Ok(result) = receiver.try_recv() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        };
        let GraphicsQueuedWorkOutput::AttachmentTransaction(roots) = result else {
            panic!("root output")
        };
        assert_eq!(roots[1].bytes, bytes);
    }
    #[test]
    fn queued_attachment_transaction_composition_base_never_replaces_load_source() {
        // Backend role-separation test. Loader range proofs are exercised by
        // guest tests; here differing selected contents expose any source swap.
        let mut state = State::new().unwrap();
        let a = request(1, false);
        let (sender_a, receiver_a) = mpsc::channel();
        state.submit_attachments(&a, sender_a).unwrap();
        let mut b = request(1, false);
        b.point.value = 2;
        b.roots[0].output.point = b.point;
        b.roots[0].prior = GraphicsAttachmentRootInput::Produced(a.roots[0].output);
        b.waits = vec![a.point];
        b.colors[0].slice = 0;
        b.colors[0].attachment.clear = GraphicsTypedColorClear::Float([0., 1., 0., 1.]);
        let mut other = b.colors[0].clone();
        other.slice = 1;
        other.attachment.clear = GraphicsTypedColorClear::Float([1., 1., 0., 1.]);
        b.colors.push(other);
        let (sender_b, receiver_b) = mpsc::channel();
        state.submit_attachments(&b, sender_b).unwrap();
        let mut c = request(1, false);
        c.point.value = 3;
        c.roots[0].output.point = c.point;
        c.roots[0].prior = GraphicsAttachmentRootInput::Produced(a.roots[0].output);
        c.roots[0].destination_base = Some(b.roots[0].output);
        c.colors[0].load = true;
        c.waits = vec![a.point];
        let (invalid_sender, _) = mpsc::channel();
        assert!(state.submit_attachments(&c, invalid_sender).is_err());
        assert_eq!(state.entries.len(), 2);
        assert!(
            !state.poisoned,
            "missing base dependency must reject before admission"
        );
        c.waits = vec![a.point, b.point];
        let (sender_c, receiver_c) = mpsc::channel();
        state.submit_attachments(&c, sender_c).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while state.entries.values().any(|e| e.completed.is_some()) {
            state.progress();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let mut expected_a = [17u8; 80];
        expected_a[72..80].copy_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255]);
        let mut expected_b = expected_a;
        expected_b[32..40].copy_from_slice(&[0, 255, 0, 255, 0, 255, 0, 255]);
        expected_b[72..80].copy_from_slice(&[255, 255, 0, 255, 255, 255, 0, 255]);
        let mut expected_c = expected_b;
        expected_c[72..80].copy_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255]);
        for (receiver, expected) in [
            (receiver_c, expected_c),
            (receiver_b, expected_b),
            (receiver_a, expected_a),
        ] {
            let GraphicsQueuedWorkOutput::AttachmentTransaction(roots) =
                receiver.recv().unwrap().unwrap()
            else {
                panic!("transaction result")
            };
            assert_eq!(roots[0].bytes, expected);
        }
    }
}
