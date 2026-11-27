// SPDX-License-Identifier: GPL-3.0-or-later
//! One logical receipt publishes complete writable roots only after every native stage drains.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsWritableSelection {
    pub root: GraphicsHandle,
    pub desc: GraphicsTypedTextureDesc,
    pub mip: u32,
    pub slice: u32,
}
#[derive(Clone)]
pub enum GraphicsAttachmentRootInput {
    Host(Arc<[u8]>),
    /// Initialized opaque legacy depth, restricted to an ordinary full surface.
    HostDepth(GraphicsDepthSnapshot),
    Produced(GraphicsProducedVersion),
}
pub struct GraphicsAttachmentRoot {
    pub destination_base: Option<GraphicsProducedVersion>,
    pub desc: GraphicsTypedTextureDesc,
    pub prior: GraphicsAttachmentRootInput,
    pub output: GraphicsProducedVersion,
}
#[derive(Clone, Debug)]
pub struct GraphicsSelectedColor {
    pub root_index: usize,
    pub mip: u32,
    pub slice: u32,
    pub attachment: GraphicsTypedColorAttachment,
    pub load: bool,
}
#[derive(Clone, Debug)]
pub struct GraphicsSelectedDepth {
    pub root_index: usize,
    pub mip: u32,
    pub slice: u32,
    pub load: GraphicsDepthLoad,
}
#[derive(Clone, Debug)]
pub struct GraphicsSelectedStorage {
    pub root_index: usize,
    pub mip: u32,
    pub slice: u32,
}
pub struct GraphicsAttachmentTransactionRequest {
    pub storage: Option<GraphicsSelectedStorage>,
    pub generation: u64,
    pub point: GraphicsQueuePoint,
    pub waits: Vec<GraphicsQueuePoint>,
    pub draws: Vec<GraphicsQueuedDraw>,
    pub extent: (usize, usize),
    pub roots: Vec<GraphicsAttachmentRoot>,
    pub colors: Vec<GraphicsSelectedColor>,
    pub depth: Option<GraphicsSelectedDepth>,
}
pub struct GraphicsAttachmentRootResult {
    pub output: GraphicsProducedVersion,
    pub desc: GraphicsTypedTextureDesc,
    pub bytes: Vec<u8>,
}
pub(super) struct AttachmentPlan {
    pub storage: Option<GraphicsSelectedStorage>,
    pub commands: Vec<GraphicsCommand>,
    pub roots: Vec<GraphicsWritableSelection>,
    pub colors: Vec<GraphicsSelectedColor>,
    pub depth: Option<GraphicsSelectedDepth>,
    pub extent: (usize, usize),
}
pub(super) struct AttachmentCommit {
    pub render: SubmissionCommit,
    pub roots: Vec<GraphicsWritableSelection>,
    pub outputs: Vec<GraphicsProducedVersion>,
    pub colors: Vec<GraphicsSelectedColor>,
}
impl GraphicsWritableSelection {
    pub(super) fn extent(self) -> Result<(usize, usize), GraphicsError> {
        let f = self
            .desc
            .packed()?
            .footprint(self.slice, self.mip)
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
        Ok((f.width, f.height))
    }
}
impl GraphicsContext {
    /// Opt in after installing a queued backend. A failed native admission may
    /// consume internal timeline values, but must drain and publish no guest roots.
    pub fn enable_attachment_transactions(&mut self) -> Result<(), GraphicsError> {
        if self.queued_backend.is_none() {
            return Err(GraphicsError::NativeMetalRequired);
        }
        self.attachment_transactions = true;
        Ok(())
    }
    pub fn create_writable_attachment_view(
        &mut self,
        device: GraphicsHandle,
        backing: GraphicsHandle,
        mip: u32,
        slice: u32,
        depth: bool,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let desc = self.sampled_resource_desc(backing)?;
        if depth != (desc.format == TextureFormat::Depth32Float)
            || matches!(
                desc.format,
                TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let view = GraphicsWritableSelection {
            root: backing,
            desc,
            mip,
            slice,
        };
        view.extent()?;
        let handle = self.allocate_handle();
        self.writable_views.insert(handle, view);
        Ok(handle)
    }
    pub fn destroy_writable_attachment_view(
        &mut self,
        handle: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        if !self.writable_views.contains_key(&handle) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if self.command_lists.values().any(|l| {
            l.commands.iter().any(|c| match c {
                GraphicsCommand::SetWritableColorTargets { views, .. } => views.contains(&handle),
                GraphicsCommand::SetWritableDepthTarget { view, .. } => *view == Some(handle),
                _ => false,
            })
        }) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let view = self.writable_views.remove(&handle).expect("validated view");
        if self.is_pinned(handle) {
            self.retired_writable_views.insert(handle, view);
        }
        Ok(())
    }
    pub(crate) fn writable_selection(
        &self,
        view: GraphicsHandle,
    ) -> Result<GraphicsWritableSelection, GraphicsError> {
        self.writable_views
            .get(&view)
            .copied()
            .ok_or(GraphicsError::InvalidResourceHandle)
    }
    pub fn cmd_set_writable_color_targets(
        &mut self,
        list: GraphicsHandle,
        views: &[GraphicsHandle],
        loads: &[GraphicsColorLoad],
    ) -> Result<(), GraphicsError> {
        if !(1..=8).contains(&views.len()) || views.len() != loads.len() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut selected = HashSet::new();
        let mut extent = None;
        for (handle, load) in views.iter().zip(loads) {
            let view = self.writable_selection(*handle)?;
            let GraphicsColorLoad::Typed { attachment, .. } = load else {
                return Err(GraphicsError::InvalidDescriptor);
            };
            attachment.validate()?;
            if attachment.format != view.desc.format
                || !selected.insert((view.root, view.mip, view.slice))
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let e = view.extent()?;
            if extent.is_some_and(|old| old != e) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            extent = Some(e);
        }
        let commands = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        commands
            .commands
            .push(GraphicsCommand::SetWritableColorTargets {
                views: views.to_vec(),
                loads: loads.to_vec(),
            });
        Ok(())
    }
    pub fn cmd_set_writable_depth_target(
        &mut self,
        list: GraphicsHandle,
        view: Option<GraphicsHandle>,
        load: GraphicsDepthLoad,
    ) -> Result<(), GraphicsError> {
        if let GraphicsDepthLoad::Clear(v) = load
            && (!v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if view.is_none() && load == GraphicsDepthLoad::Load {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if let Some(handle) = view
            && self.writable_selection(handle)?.desc.format != TextureFormat::Depth32Float
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let commands = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        commands
            .commands
            .push(GraphicsCommand::SetWritableDepthTarget { view, load });
        Ok(())
    }
    pub(super) fn attachment_plan(
        &self,
        commands: &[GraphicsCommand],
    ) -> Result<Option<AttachmentPlan>, GraphicsError> {
        if !commands.iter().any(|c| {
            matches!(
                c,
                GraphicsCommand::SetWritableColorTargets { .. }
                    | GraphicsCommand::SetWritableDepthTarget { .. }
                    | GraphicsCommand::SetStorageView(_)
            )
        }) {
            return Ok(None);
        }
        if !self.attachment_transactions || self.queued_backend.is_none() {
            return Err(GraphicsError::NativeMetalRequired);
        }
        let mut color_binding = None;
        let mut depth_binding = None;
        let mut storage_binding = None;
        let mut drawn = false;
        let mut normalized = Vec::with_capacity(commands.len());
        for command in commands {
            match command {
                GraphicsCommand::SetStorageView(view) => {
                    if !self.shader_storage_views || (drawn && storage_binding != *view) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    storage_binding = *view;
                    normalized.push(command.clone());
                }
                GraphicsCommand::SetWritableColorTargets { views, loads } => {
                    if drawn && color_binding.as_ref() != Some(&(views.clone(), loads.clone())) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    color_binding = Some((views.clone(), loads.clone()));
                    normalized.push(GraphicsCommand::SetColorTargets {
                        targets: views
                            .iter()
                            .map(|v| self.writable_selection(*v).map(|s| s.root))
                            .collect::<Result<_, _>>()?,
                        loads: loads.clone(),
                    });
                }
                GraphicsCommand::SetWritableDepthTarget { view, load } => {
                    if drawn && depth_binding != Some((*view, *load)) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    depth_binding = Some((*view, *load));
                    normalized.push(GraphicsCommand::SetDepthTarget {
                        target: view
                            .map(|v| self.writable_selection(v).map(|s| s.root))
                            .transpose()?,
                        load: *load,
                    });
                }
                GraphicsCommand::SetColorTargets { .. }
                | GraphicsCommand::SetRenderTargets { .. }
                | GraphicsCommand::SetDepthTarget { .. } => {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                GraphicsCommand::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    ..
                } => {
                    drawn |= *vertex_count_per_instance != 0 && *instance_count != 0;
                    normalized.push(command.clone());
                }
                GraphicsCommand::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    ..
                } => {
                    drawn |= *index_count_per_instance != 0 && *instance_count != 0;
                    normalized.push(command.clone());
                }
                _ => normalized.push(command.clone()),
            }
        }
        let (views, loads) = color_binding.ok_or(GraphicsError::InvalidDescriptor)?;
        let mut roots = Vec::<GraphicsWritableSelection>::new();
        let mut colors = Vec::new();
        let mut extent = None;
        let mut unique = HashSet::new();
        for (handle, load) in views.iter().zip(loads) {
            let selection = self.writable_selection(*handle)?;
            let GraphicsColorLoad::Typed { attachment, load } = load else {
                return Err(GraphicsError::InvalidDescriptor);
            };
            attachment.validate()?;
            if !self.resource_roles.color(attachment.format)
                || attachment.format != selection.desc.format
                || !unique.insert((selection.root, selection.mip, selection.slice))
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let e = selection.extent()?;
            if extent.is_some_and(|old| old != e) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            extent = Some(e);
            let root_index = roots
                .iter()
                .position(|s| s.root == selection.root)
                .unwrap_or_else(|| {
                    roots.push(selection);
                    roots.len() - 1
                });
            colors.push(GraphicsSelectedColor {
                root_index,
                mip: selection.mip,
                slice: selection.slice,
                attachment,
                load,
            });
        }
        let depth = if let Some((Some(handle), load)) = depth_binding {
            let s = self.writable_selection(handle)?;
            if s.desc.format != TextureFormat::Depth32Float || Some(s.extent()?) != extent {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let root_index = roots.len();
            roots.push(s);
            Some(GraphicsSelectedDepth {
                root_index,
                mip: s.mip,
                slice: s.slice,
                load,
            })
        } else {
            None
        };
        let storage = if let Some(handle) = storage_binding {
            let selected = self.storage_selection(handle)?;
            if depth.is_some()
                || Some(selected.extent()?) != extent
                || roots.iter().any(|root| root.root == selected.root)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let root_index = roots.len();
            roots.push(selected);
            Some(GraphicsSelectedStorage {
                root_index,
                mip: selected.mip,
                slice: selected.slice,
            })
        } else {
            None
        };
        Ok(Some(AttachmentPlan {
            storage,
            commands: normalized,
            roots,
            colors,
            depth,
            extent: extent.ok_or(GraphicsError::InvalidDescriptor)?,
        }))
    }
    pub(super) fn attachment_budget(
        &self,
        plan: &AttachmentPlan,
        preparation: &QueuedPreparation,
    ) -> Result<usize, GraphicsError> {
        let host_count = plan
            .roots
            .iter()
            .filter(|r| !preparation.produced.contains_key(&r.root))
            .count();
        if host_count
            + 1
            + plan.colors.len()
            + usize::from(plan.depth.is_some())
            + usize::from(plan.storage.is_some())
            > MAX_QUEUED_GRAPH_NODES
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        // Selected storage is a fresh persistent GPU surface, with no CPU readback.
        let mut total = if plan.storage.is_some() {
            plan.extent
                .0
                .checked_mul(plan.extent.1)
                .and_then(|n| n.checked_mul(4))
                .ok_or(GraphicsError::InvalidDescriptor)?
        } else {
            0
        };
        for (index, root) in plan.roots.iter().enumerate() {
            if self.typed_copy_capabilities.formats
                & (1 << typed_copy::format_code(root.desc.format))
                == 0
            {
                return Err(GraphicsError::NativeMetalRequired);
            }

            let bytes = root.desc.byte_len()?;
            let layout = root.desc.packed()?;
            let mut staging = 0usize;
            for slice in 0..root.desc.array_layers.unwrap_or(1) {
                for mip in 0..root.desc.mip_levels {
                    let f = layout
                        .footprint(slice, mip)
                        .map_err(|_| GraphicsError::InvalidDescriptor)?;
                    staging = staging
                        .checked_add(
                            f.row_bytes
                                .div_ceil(256)
                                .checked_mul(256)
                                .and_then(|n| n.checked_mul(f.rows))
                                .ok_or(GraphicsError::InvalidDescriptor)?,
                        )
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                }
            }
            let patches = plan.colors.iter().filter(|c| c.root_index == index).count()
                + usize::from(plan.depth.as_ref().is_some_and(|d| d.root_index == index))
                + usize::from(plan.storage.as_ref().is_some_and(|d| d.root_index == index));
            let per_patch = bytes
                .checked_mul(2)
                .and_then(|n| n.checked_add(staging))
                .ok_or(GraphicsError::InvalidDescriptor)?;
            total = total
                .checked_add(
                    per_patch
                        .checked_mul(patches)
                        .ok_or(GraphicsError::InvalidDescriptor)?,
                )
                .ok_or(GraphicsError::InvalidDescriptor)?;
            if !preparation.produced.contains_key(&root.root) {
                // Host seed: two host inputs, staging, packed readback and a fresh root.
                total = total
                    .checked_add(
                        bytes
                            .checked_mul(4)
                            .and_then(|n| staging.checked_mul(5).and_then(|p| n.checked_add(p)))
                            .ok_or(GraphicsError::InvalidDescriptor)?,
                    )
                    .ok_or(GraphicsError::InvalidDescriptor)?;
            }
        }
        self.validate_queued_work_budget(total)?;
        Ok(total)
    }
}

impl GraphicsContext {
    #[allow(clippy::too_many_arguments)] // Admission carries validated graph/recording state without mutating it.
    pub(super) fn admit_attachment_transaction(
        &mut self,
        plan: AttachmentPlan,
        mut render: SubmissionCommit,
        draws: Vec<GraphicsQueuedDraw>,
        generation: u64,
        point: GraphicsQueuePoint,
        serial: u64,
        dependencies: Vec<GraphicsHandle>,
        closure: Vec<GraphicsHandle>,
        preparation: QueuedPreparation,
        reads: HashSet<GraphicsHandle>,
        mut writes: HashSet<GraphicsHandle>,
        mut state_definitions: HashMap<GraphicsHandle, ResourceStates>,
        render_bytes: usize,
    ) -> Result<GraphicsHandle, GraphicsError> {
        // Expanded CPU/GPU geometry and per-draw uniform copies coexist with
        // every retained render/copy stage. Gather payloads are in render_bytes.
        let vertices = draws
            .iter()
            .try_fold(0usize, |sum, draw| sum.checked_add(draw.vertices.len()))
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let render_scratch = vertices
            .max(1)
            .checked_mul(64)
            .and_then(|n| draws.len().checked_mul(64).and_then(|u| n.checked_add(u)))
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let allocation = self
            .attachment_budget(&plan, &preparation)?
            .checked_add(render_bytes)
            .and_then(|n| n.checked_add(render_scratch))
            .ok_or(GraphicsError::InvalidDescriptor)?;
        self.validate_queued_work_budget(allocation)?;
        let host_count = plan
            .roots
            .iter()
            .filter(|r| !preparation.produced.contains_key(&r.root))
            .count();
        if host_count
            + 1
            + plan.colors.len()
            + usize::from(plan.depth.is_some())
            + usize::from(plan.storage.is_some())
            > MAX_QUEUED_GRAPH_NODES
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let mut roots = Vec::with_capacity(plan.roots.len());
        let mut outputs = HashMap::new();
        let mut layouts = HashMap::new();
        for (index, root) in plan.roots.iter().enumerate() {
            if self.sampled_resource_desc(root.root)? != root.desc {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let prior = if let Some(version) = preparation.produced.get(&root.root) {
                if preparation.layouts.get(&root.root) != Some(&root.desc.produced())
                    || !matches!(
                        version.output,
                        GraphicsProducedOutput::Color(_)
                            | GraphicsProducedOutput::Depth
                            | GraphicsProducedOutput::Copy(0)
                            | GraphicsProducedOutput::AttachmentRoot(_)
                    )
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                GraphicsAttachmentRootInput::Produced(*version)
            } else {
                // Complete cumulative reservation precedes any root snapshot allocation.
                if let Some(depth) = self.depth_resources.get(&root.root) {
                    if let Some(snapshot) = &depth.snapshot {
                        if snapshot
                            .downcast_ref::<GraphicsTypedDepthSnapshot>()
                            .is_some()
                        {
                            GraphicsAttachmentRootInput::Host(
                                self.sampled_resource_bytes(root.root)?,
                            )
                        } else {
                            if root.desc.mip_levels != 1 || root.desc.array_layers.is_some() {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            GraphicsAttachmentRootInput::HostDepth(snapshot.clone())
                        }
                    } else {
                        // Clear replaces every texel of this ordinary surface. The
                        // seed is never published as prior depth and cannot satisfy Load.
                        if root.desc.mip_levels != 1
                            || root.desc.array_layers.is_some()
                            || !plan.depth.as_ref().is_some_and(|selection| {
                                selection.root_index == index
                                    && selection.mip == 0
                                    && selection.slice == 0
                                    && matches!(selection.load, GraphicsDepthLoad::Clear(_))
                            })
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                        let len = root.desc.byte_len()?;
                        let mut bytes = Vec::new();
                        bytes
                            .try_reserve_exact(len)
                            .map_err(|_| MemoryError::AllocationFailed)?;
                        bytes.resize(len, 0);
                        GraphicsAttachmentRootInput::Host(Arc::from(bytes))
                    }
                } else {
                    let bytes = self.sampled_resource_bytes(root.root)?;
                    if bytes.len() != root.desc.byte_len()? {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    GraphicsAttachmentRootInput::Host(bytes)
                }
            };
            let output = GraphicsProducedVersion {
                point,
                output: GraphicsProducedOutput::AttachmentRoot(index as u8),
            };
            roots.push(GraphicsAttachmentRoot {
                destination_base: preparation.destination_bases.get(&root.root).copied(),
                desc: root.desc,
                prior,
                output,
            });
            outputs.insert(root.root, output);
            layouts.insert(root.root, root.desc.produced());
            writes.insert(root.root); // Even read-only depth publishes a fresh immutable root.
            let state = render
                .states
                .get(&root.root)
                .copied()
                .or_else(|| preparation.states.get(&root.root).copied())
                .or_else(|| self.resources.get(&root.root).map(|r| r.state))
                .or_else(|| self.depth_resources.get(&root.root).map(|r| r.state))
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            state_definitions.insert(root.root, state);
        }
        // All roots, including read-only depth, conflict against uncovered writers/readers.
        for (handle, node) in &self.queued {
            if node.receipt.status == GraphicsSubmissionStatus::Pending
                && !closure.contains(handle)
                && (preparation.ranges.conflicts(&node.ranges)
                    || writes.iter().any(|h| node.writes.contains(h)))
            {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
        }
        let request = GraphicsAttachmentTransactionRequest {
            generation,
            point,
            waits: closure.iter().map(|h| self.queued[h].point).collect(),
            draws,
            extent: plan.extent,
            roots,
            colors: plan.colors.clone(),
            depth: plan.depth,
            storage: plan.storage,
        };
        let ticket = self
            .queued_backend
            .clone()
            .ok_or(GraphicsError::NativeMetalRequired)?(
            GraphicsQueuedWorkRequest::AttachmentTransaction(request),
        )?;
        let references = self.recorded_references(render.list);
        let handle = self.allocate_handle();
        render.serial = Some(serial);
        render
            .states
            .retain(|h, _| state_definitions.contains_key(h));
        let commit = AttachmentCommit {
            render,
            roots: plan.roots,
            outputs: (0..outputs.len())
                .map(|i| GraphicsProducedVersion {
                    point,
                    output: GraphicsProducedOutput::AttachmentRoot(i as u8),
                })
                .collect(),
            colors: plan.colors,
        };
        let descriptor_revisions = self.recorded_descriptor_revisions(commit.render.list);
        self.queued.insert(
            handle,
            QueuedReceipt {
                publication_bases: self.publication_bases(outputs.keys().copied()),
                depth_allocation_bytes: 0,
                color_allocation_bytes: 0,
                copy_allocation_bytes: allocation,
                layouts,
                commit: Some(QueuedCommit::Attachment(commit)),
                copy_outcome: None,
                point,
                generation,
                serial,
                semantic_dependencies: preparation.semantic_dependencies.clone(),
                ranges: preparation.ranges.clone(),
                dependencies,
                ticket,
                reads,
                writes,
                state_definitions,
                outputs,
                released: false,
                rejected_dependencies: HashSet::new(),
                receipt: SubmissionReceipt {
                    descriptor_revisions,
                    handle,
                    pending: None,
                    commit: None,
                    sync_references: references.clone(),
                    references,
                    status: GraphicsSubmissionStatus::Pending,
                    error: None,
                    outcome: None,
                },
            },
        );
        self.queue_values.insert(point.queue, point.value);
        self.queued_serial = serial;
        self.queued_generation = generation;
        self.queued_failed = false;
        Ok(handle)
    }
    pub(super) fn commit_attachment_transaction(
        &mut self,
        commit: AttachmentCommit,
        results: Vec<GraphicsAttachmentRootResult>,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        if results.len() != commit.roots.len() {
            return Err(GraphicsError::BackendError(
                "attachment root cardinality".into(),
            ));
        }
        for ((result, root), output) in results.iter().zip(&commit.roots).zip(&commit.outputs) {
            if result.output != *output
                || result.desc != root.desc
                || result.bytes.len() != root.desc.byte_len()?
            {
                return Err(GraphicsError::BackendError(
                    "attachment root identity/layout/length".into(),
                ));
            }
            if (self.resources.contains_key(&root.root)
                || self.depth_resources.contains_key(&root.root))
                && self.sampled_resource_desc(root.root)? != root.desc
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        // Derive selected presentation bytes only from validated completed full roots.
        let mut colors = Vec::with_capacity(commit.colors.len());
        for color in &commit.colors {
            let root = &results[color.root_index];
            let f = root
                .desc
                .packed()?
                .footprint(color.slice, color.mip)
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
            colors.push(root.bytes[f.offset..f.offset + f.byte_len].to_vec());
        }
        let mut presentation = self
            .last_presentation
            .lock()
            .map_err(|_| GraphicsError::BackendError("presentation lock poisoned".into()))?;
        for (root, result) in commit.roots.iter().zip(results) {
            if let Some(resource) = self.resources.get_mut(&root.root) {
                resource.data.copy_from_slice(&result.bytes);
            } else if let Some(depth) = self.depth_resources.get_mut(&root.root) {
                depth.snapshot = Some(Arc::new(GraphicsTypedDepthSnapshot {
                    desc: result.desc,
                    bytes: Arc::from(result.bytes),
                }));
            }
        }
        for (handle, state) in commit.render.states {
            if let Some(resource) = self.resources.get_mut(&handle) {
                resource.state = state;
            } else if let Some(depth) = self.depth_resources.get_mut(&handle) {
                depth.state = state;
            }
        }
        let history = &mut self
            .queues
            .get_mut(&commit.render.queue)
            .expect("validated queue")
            .submitted_lists;
        if history.len() == 256 {
            history.remove(0);
        }
        history.push(commit.render.list);
        let pixels = colors[0].clone();
        if commit.colors[0].attachment.format == TextureFormat::Rgba8Unorm
            && commit
                .render
                .serial
                .is_none_or(|serial| serial >= self.published_serial)
        {
            if let Some(serial) = commit.render.serial {
                self.published_serial = serial;
            }
            presentation.width = commit.render.extent.0;
            presentation.height = commit.render.extent.1;
            presentation.pixels = pixels.clone();
            presentation.frame_index += 1;
            presentation.backend = Some(commit.render.backend);
        }
        Ok(ExecutionOutcome {
            backend: commit.render.backend,
            pixels,
            color_outputs: colors,
            width: commit.render.extent.0,
            height: commit.render.extent.1,
            draw_count: commit.render.draw_count,
        })
    }
}
