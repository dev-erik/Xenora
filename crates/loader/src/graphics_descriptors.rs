// SPDX-License-Identifier: GPL-3.0-or-later
//! Immutable descriptor revisions captured at command recording.
use super::*;

pub const MAX_DESCRIPTOR_REVISIONS: usize = 256;
#[derive(Clone, Debug)]
pub struct GraphicsDescriptorRequirements {
    pub types: xenora_shader::GraphicsPipelineTypes4,
    pub sampled_pairs: u16,
    pub array_layers: [Vec<xenora_shader::GraphicsArrayLayer>; 4],
    pub descriptor_index_uniform_mask: u8,
}
#[derive(Clone, Debug)]
pub struct GraphicsDescriptorBindings<T> {
    pub textures: [Option<T>; 4],
    pub samplers: [Option<xenora_texture::SamplerDesc>; 4],
    pub views: [Option<GraphicsTextureViewDesc>; 4],
    pub array_views: [Option<GraphicsArraySliceViewDesc>; 4],
    pub sampled_ranges: [Option<GraphicsArrayRangeViewDesc>; 4],
}
impl<T> Default for GraphicsDescriptorBindings<T> {
    fn default() -> Self {
        Self {
            textures: std::array::from_fn(|_| None),
            samplers: [None; 4],
            views: [None; 4],
            array_views: [None; 4],
            sampled_ranges: [None; 4],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum GraphicsDescriptorEntry {
    Null,
    TextureView(GraphicsHandle),
    ArrayRange(GraphicsHandle),
    Sampler(GraphicsHandle),
}
#[derive(Clone, Debug)]
pub struct GraphicsDescriptorRevision {
    pub revision: u64,
    pub(super) bindings: GraphicsDescriptorBindings<GraphicsHandle>,
}
impl GraphicsDescriptorRevision {
    pub(super) fn roots(&self) -> impl Iterator<Item = GraphicsHandle> + '_ {
        self.bindings.textures.iter().flatten().copied()
    }
}
impl GraphicsContext {
    pub fn enable_descriptor_tables(&mut self) -> Result<(), GraphicsError> {
        if self.queued_backend.is_none() {
            return Err(GraphicsError::NativeMetalRequired);
        }
        self.descriptor_backend = true;
        Ok(())
    }
    pub fn descriptor_revision_count(&self) -> usize {
        self.descriptor_revisions
            .iter()
            .filter(|w| w.strong_count() != 0)
            .count()
    }
    fn reserve_descriptor_revision(&mut self) -> Result<(), GraphicsError> {
        self.descriptor_revisions.retain(|w| w.strong_count() != 0);
        if self.descriptor_revisions.len() >= MAX_DESCRIPTOR_REVISIONS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        Ok(())
    }
    pub fn create_descriptor_table(
        &mut self,
        device: GraphicsHandle,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        self.reserve_descriptor_revision()?;
        let revision = Arc::new(GraphicsDescriptorRevision {
            revision: 0,
            bindings: Default::default(),
        });
        self.descriptor_revisions.push(Arc::downgrade(&revision));
        let handle = self.allocate_handle();
        self.descriptor_tables.insert(handle, revision);
        Ok(handle)
    }
    pub fn update_descriptor_table(
        &mut self,
        table: GraphicsHandle,
        first: usize,
        entries: &[GraphicsDescriptorEntry],
    ) -> Result<u64, GraphicsError> {
        let old = self
            .descriptor_tables
            .get(&table)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if entries.is_empty() || first.checked_add(entries.len()).is_none_or(|end| end > 8) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let revision = old
            .revision
            .checked_add(1)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        // An unrecorded current revision is replaced, not retained. Weak ledger
        // entries do not own revisions and therefore do not prevent this refund.
        let replaces_unretained = Arc::strong_count(old) == 1;
        let mut bindings = old.bindings.clone();
        for (at, entry) in entries.iter().enumerate() {
            let slot = first + at;
            if slot < 4 {
                bindings.textures[slot] = None;
                bindings.views[slot] = None;
                bindings.array_views[slot] = None;
                bindings.sampled_ranges[slot] = None;
                match entry {
                    GraphicsDescriptorEntry::Null => {}
                    GraphicsDescriptorEntry::TextureView(h) => {
                        let view = self
                            .texture_views
                            .get(h)
                            .ok_or(GraphicsError::InvalidResourceHandle)?;
                        bindings.textures[slot] = Some(view.backing);
                        if let Some(slice) = view.slice {
                            bindings.array_views[slot] = Some(GraphicsArraySliceViewDesc {
                                slice,
                                base_mip: view.desc.base_mip,
                                mip_count: view.desc.mip_count,
                                format: view.desc.format,
                            });
                        } else {
                            bindings.views[slot] = Some(view.desc);
                        }
                    }
                    GraphicsDescriptorEntry::ArrayRange(h) => {
                        let view = self
                            .array_range_views
                            .get(h)
                            .ok_or(GraphicsError::InvalidResourceHandle)?;
                        bindings.textures[slot] = Some(view.backing);
                        bindings.sampled_ranges[slot] = Some(view.desc);
                    }
                    _ => return Err(GraphicsError::InvalidDescriptor),
                }
            } else {
                bindings.samplers[slot - 4] = match entry {
                    GraphicsDescriptorEntry::Null => None,
                    GraphicsDescriptorEntry::Sampler(h) => Some(
                        *self
                            .samplers
                            .get(h)
                            .ok_or(GraphicsError::InvalidResourceHandle)?,
                    ),
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
            }
        }
        self.descriptor_revisions.retain(|w| w.strong_count() != 0);
        let retained = self.descriptor_revisions.len() - usize::from(replaces_unretained);
        if retained >= MAX_DESCRIPTOR_REVISIONS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let next = Arc::new(GraphicsDescriptorRevision { revision, bindings });
        self.descriptor_revisions.push(Arc::downgrade(&next));
        self.descriptor_tables.insert(table, next);
        Ok(revision)
    }
    pub fn destroy_descriptor_table(&mut self, table: GraphicsHandle) -> Result<(), GraphicsError> {
        self.descriptor_tables
            .remove(&table)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        Ok(())
    }
    pub fn cmd_set_descriptor_table(
        &mut self,
        list: GraphicsHandle,
        table: Option<GraphicsHandle>,
    ) -> Result<(), GraphicsError> {
        let revision = table
            .map(|h| {
                self.descriptor_tables
                    .get(&h)
                    .cloned()
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .transpose()?;
        let list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if list.commands.len() >= 4096 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        list.commands
            .push(GraphicsCommand::SetDescriptorTable(revision));
        Ok(())
    }
    pub(super) fn recorded_descriptor_revisions(
        &self,
        list: GraphicsHandle,
    ) -> Vec<Arc<GraphicsDescriptorRevision>> {
        self.command_lists[&list]
            .commands
            .iter()
            .filter_map(|c| match c {
                GraphicsCommand::SetDescriptorTable(Some(r)) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }
    pub fn create_typed_graphics_pipeline4(
        &mut self,
        device: GraphicsHandle,
        vertex: &[u8],
        pixel: &[u8],
        types: xenora_shader::GraphicsPipelineTypes4,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.pipelines.len() + self.retired_pipelines.len() >= 256
            || self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let c = xenora_shader::compile_graphics_pipeline_typed4(
            xenora_shader::Architecture::Gfx1030,
            vertex,
            pixel,
            types,
        )
        .map_err(|e| GraphicsError::BackendError(format!("shader translation: {e}")))?;
        if c.shader.msl.len() > 65536 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut required_uniform_mask = c.required_uniform_mask | c.descriptor_index_uniform_mask;
        for layer in c.array_layers.iter().flatten() {
            if let xenora_shader::GraphicsArrayLayer::Uniform(i) = layer {
                if *i >= 8 {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                required_uniform_mask |= 1 << i;
            }
        }
        let legacy = xenora_shader::GraphicsPipelineTypes {
            textures: [c.types.textures[0], c.types.textures[1]],
            colors: c.types.colors,
        };
        let pipeline = GraphicsPipeline {
            descriptor_requirements: Some(GraphicsDescriptorRequirements {
                types: c.types,
                sampled_pairs: c.sampled_pairs,
                array_layers: c.array_layers,
                descriptor_index_uniform_mask: c.descriptor_index_uniform_mask,
            }),
            storage: c.storage,
            array_layers: Default::default(),
            types: legacy,
            required_uniform_mask,
            msl: c.shader.msl,
            color_targets: c.color_targets,
            uses_texture: c.uses_texture,
            sampled_pairs: 0,
        };
        let h = self.allocate_handle();
        self.pipelines.insert(h, Arc::new(pipeline));
        Ok(h)
    }
    pub(super) fn descriptor_roots(
        &self,
        revision: Option<&GraphicsDescriptorRevision>,
        pipeline: Option<GraphicsHandle>,
    ) -> [Option<GraphicsHandle>; 4] {
        let pairs = pipeline
            .and_then(|h| self.pipelines.get(&h))
            .and_then(|p| p.descriptor_requirements.as_ref())
            .map_or(0, |r| r.sampled_pairs);
        std::array::from_fn(|s| {
            if pairs & (15 << (s * 4)) != 0 {
                revision.and_then(|r| r.bindings.textures[s])
            } else {
                None
            }
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn descriptor_draw_bindings(
        &self,
        revision: Option<&GraphicsDescriptorRevision>,
        pipeline: Option<GraphicsHandle>,
        uniforms: Option<[u32; 8]>,
        states: &HashMap<GraphicsHandle, ResourceStates>,
        snapshots: &mut HashMap<GraphicsHandle, Arc<[u8]>>,
        loads: &[GraphicsColorLoad],
        queued: Option<&QueuedPreparation>,
    ) -> Result<GraphicsDescriptorBindings<GraphicsTextureSnapshot>, GraphicsError> {
        if !self.descriptor_backend || queued.is_none() {
            return Err(GraphicsError::NativeMetalRequired);
        }
        let p = self
            .pipelines
            .get(&pipeline.ok_or(GraphicsError::InvalidDescriptor)?)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let req = p
            .descriptor_requirements
            .as_ref()
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let rev = revision.ok_or(GraphicsError::InvalidDescriptor)?;
        if p.color_targets != loads.len()
            || (p.required_uniform_mask != 0 && (!self.uniform_backend || uniforms.is_none()))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        for i in 0..8 {
            if req.descriptor_index_uniform_mask & (1 << i) != 0
                && uniforms.is_none_or(|u| u[i] >= 4)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        for (i, load) in loads.iter().enumerate() {
            let uint = matches!(load,GraphicsColorLoad::Typed {attachment,..} if attachment.format==TextureFormat::R32Uint);
            if uint != (req.types.colors[i] == xenora_shader::GraphicsExportType::Uint) {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        let mut out = GraphicsDescriptorBindings::default();
        for s in 0..4 {
            if req.sampled_pairs & (0x1111 << s) != 0 {
                out.samplers[s] =
                    Some(rev.bindings.samplers[s].ok_or(GraphicsError::InvalidDescriptor)?);
            }
        }
        for s in 0..4 {
            if req.sampled_pairs & (15 << (s * 4)) == 0 {
                continue;
            }
            let h = rev.bindings.textures[s].ok_or(GraphicsError::InvalidDescriptor)?;
            let desc = self.sampled_resource_desc(h)?;
            if !self.texture_capabilities.supports(desc.format)
                && !self.resource_roles.sampled(desc.format)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let range = rev.bindings.sampled_ranges[s];
            let slice = rev.bindings.array_views[s];
            let view = rev.bindings.views[s];
            if (range.is_some() && !self.shader_array_views)
                || (slice.is_some() && !self.array_view_backend)
                || (view.is_some() && !self.view_backend)
            {
                return Err(GraphicsError::NativeMetalRequired);
            }
            let expected = match (desc.format, range.is_some()) {
                (TextureFormat::R32Uint, true) => xenora_shader::GraphicsSampleType::UintArray,
                (TextureFormat::R32Uint, false) => xenora_shader::GraphicsSampleType::Uint,
                (TextureFormat::Depth32Float, false) => xenora_shader::GraphicsSampleType::Depth,
                (TextureFormat::Depth32Float, true) => {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                (_, true) => xenora_shader::GraphicsSampleType::FloatArray,
                (_, false) => xenora_shader::GraphicsSampleType::Float,
            };
            if req.types.textures[s] != expected
                || desc.array_layers.is_some() != (range.is_some() || slice.is_some())
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let state = self
                .resources
                .get(&h)
                .map(|r| r.state)
                .or_else(|| self.depth_resources.get(&h).map(|r| r.state))
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            if states.get(&h).copied().unwrap_or(state) != ResourceStates::PIXEL_SHADER_RESOURCE {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            if let Some(r) = range {
                for l in &req.array_layers[s] {
                    let valid = match *l {
                        xenora_shader::GraphicsArrayLayer::Constant(v) => v < r.slice_count,
                        xenora_shader::GraphicsArrayLayer::Uniform(i) => {
                            let v = f32::from_bits(
                                uniforms
                                    .and_then(|u| u.get(i as usize).copied())
                                    .ok_or(GraphicsError::InvalidDescriptor)?,
                            );
                            v.is_finite()
                                && v >= 0.0
                                && v.fract() == 0.0
                                && f64::from(v) < f64::from(r.slice_count)
                        }
                    };
                    if !valid {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                }
            }
            let (base, mips) = view
                .map(|v| (v.base_mip, v.mip_count))
                .or(slice.map(|v| (v.base_mip, v.mip_count)))
                .or(range.map(|v| (v.base_mip, v.mip_count)))
                .ok_or(GraphicsError::InvalidDescriptor)?;
            let metadata = xenora_texture::TextureMetadata::new(
                (desc.extent.0 >> base).max(1),
                (desc.extent.1 >> base).max(1),
                mips,
            )
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
            for (t, sampler) in out.samplers.iter().enumerate() {
                if req.sampled_pairs & (1 << (s * 4 + t)) != 0 {
                    sampler
                        .as_ref()
                        .ok_or(GraphicsError::InvalidDescriptor)?
                        .validate_against_texture(&metadata)
                        .map_err(|_| GraphicsError::InvalidDescriptor)?;
                }
            }
            let pixels = if queued.is_some_and(|q| q.produced.contains_key(&h)) {
                Arc::from([])
            } else if let Some(b) = snapshots.get(&h) {
                b.clone()
            } else {
                let b = self.sampled_resource_bytes(h)?;
                snapshots.insert(h, b.clone());
                b
            };
            out.textures[s] = Some(GraphicsTextureSnapshot {
                array_layers: desc.array_layers,
                format: desc.format,
                width: desc.extent.0,
                height: desc.extent.1,
                mip_levels: desc.mip_levels,
                pixels,
            });
            out.views[s] = view;
            out.array_views[s] = slice;
            out.sampled_ranges[s] = range;
        }
        Ok(out)
    }
    pub(super) fn queued_descriptor_bindings(
        &self,
        b: GraphicsDescriptorBindings<GraphicsTextureSnapshot>,
        handles: [Option<GraphicsHandle>; 4],
        q: &QueuedPreparation,
    ) -> Result<GraphicsDescriptorBindings<GraphicsTextureInput>, GraphicsError> {
        let mut out = GraphicsDescriptorBindings {
            samplers: b.samplers,
            views: b.views,
            array_views: b.array_views,
            sampled_ranges: b.sampled_ranges,
            ..Default::default()
        };
        for (s, t) in b.textures.into_iter().enumerate() {
            if let Some(t) = t {
                out.textures[s] = Some(
                    if let Some(v) = handles[s].and_then(|h| q.produced.get(&h)) {
                        let desc = GraphicsTypedTextureDesc {
                            format: t.format,
                            extent: (t.width, t.height),
                            mip_levels: t.mip_levels,
                            array_layers: t.array_layers,
                        };
                        if q.layouts
                            .get(&handles[s].ok_or(GraphicsError::InvalidDescriptor)?)
                            != Some(&desc.produced())
                            || !matches!(
                                v.output,
                                GraphicsProducedOutput::Color(_)
                                    | GraphicsProducedOutput::Depth
                                    | GraphicsProducedOutput::Copy(0)
                                    | GraphicsProducedOutput::AttachmentRoot(_)
                            )
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        GraphicsTextureInput::ProducedTyped { version: *v, desc }
                    } else {
                        GraphicsTextureInput::Host(t)
                    },
                );
            }
        }
        Ok(out)
    }
}
