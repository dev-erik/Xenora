// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsTypedTextureDesc {
    pub format: TextureFormat,
    pub extent: (usize, usize),
    pub mip_levels: u32,
    pub array_layers: Option<u32>,
}
impl GraphicsTypedTextureDesc {
    pub(super) fn packed(self) -> Result<PackedTextureArrayLayout, GraphicsError> {
        let layers = self.array_layers.unwrap_or(1);
        if self.extent.0 > MAX_RENDER_DIMENSION
            || self.extent.1 > MAX_RENDER_DIMENSION
            || layers == 0
            || layers > 256
            || self.mip_levels.checked_mul(layers).is_none_or(|n| n > 4096)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let layout = PackedTextureArrayLayout::new(
            self.format,
            self.extent.0,
            self.extent.1,
            self.mip_levels,
            layers,
        )
        .map_err(|_| GraphicsError::InvalidDescriptor)?;
        if layout.byte_len() > MAX_RESOURCE_BYTES {
            return Err(GraphicsError::InvalidDescriptor);
        }
        Ok(layout)
    }
    pub(super) fn byte_len(self) -> Result<usize, GraphicsError> {
        Ok(self.packed()?.byte_len())
    }
    pub(super) fn produced(self) -> GraphicsProducedLayout {
        if self.format == TextureFormat::Depth32Float
            && self.mip_levels == 1
            && self.array_layers.is_none()
        {
            GraphicsProducedLayout::Depth32Float {
                extent: self.extent,
            }
        } else if self.format == TextureFormat::Rgba8Unorm {
            if let Some(array_layers) = self.array_layers {
                GraphicsProducedLayout::Rgba8Array {
                    extent: self.extent,
                    mip_levels: self.mip_levels,
                    array_layers,
                }
            } else if self.mip_levels == 1 {
                GraphicsProducedLayout::Rgba8 {
                    extent: self.extent,
                }
            } else {
                GraphicsProducedLayout::Rgba8MipChain {
                    extent: self.extent,
                    mip_levels: self.mip_levels,
                }
            }
        } else {
            GraphicsProducedLayout::TypedTexture(self)
        }
    }
    fn staging(self) -> Result<usize, GraphicsError> {
        let l = self.packed()?;
        let mut bytes = 0usize;
        for slice in 0..self.array_layers.unwrap_or(1) {
            for mip in 0..self.mip_levels {
                let f = l
                    .footprint(slice, mip)
                    .map_err(|_| GraphicsError::InvalidDescriptor)?;
                let pitch = f
                    .row_bytes
                    .checked_add(255)
                    .map(|n| n / 256 * 256)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                bytes = bytes
                    .checked_add(
                        pitch
                            .checked_mul(f.rows)
                            .ok_or(GraphicsError::InvalidDescriptor)?,
                    )
                    .ok_or(GraphicsError::InvalidDescriptor)?;
            }
        }
        Ok(bytes)
    }
}
/// Bit positions are the stable guest format codes, not Rust discriminants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphicsTypedCopyCapabilities {
    pub formats: u16,
}
impl GraphicsTypedCopyCapabilities {
    fn supports(self, f: TextureFormat) -> bool {
        self.formats & (1 << format_code(f)) != 0 && self.formats & !0x3ff == 0
    }
}
pub(super) fn format_code(f: TextureFormat) -> u32 {
    match f {
        TextureFormat::Rgba8Unorm => 0,
        TextureFormat::Bc1RgbaUnorm => 1,
        TextureFormat::Bc3RgbaUnorm => 2,
        TextureFormat::R8Unorm => 3,
        TextureFormat::Rg8Unorm => 4,
        TextureFormat::Bgra8Unorm => 5,
        TextureFormat::Rgba16Float => 6,
        TextureFormat::R32Float => 7,
        TextureFormat::R32Uint => 8,
        TextureFormat::Depth32Float => 9,
    }
}
/// Copy-produced depth bytes. Native adapters convert this to their retained depth snapshot.
/// Transfer semantics preserve bits; this is not a claim that every bit pattern is a valid draw depth.
#[derive(Clone, Debug)]
pub struct GraphicsTypedDepthSnapshot {
    pub desc: GraphicsTypedTextureDesc,
    pub bytes: Arc<[u8]>,
}

pub(super) fn copy_reservation(
    source: GraphicsCopyLayout,
    destination: GraphicsCopyLayout,
    source_host: bool,
    destination_host: bool,
) -> Result<usize, GraphicsError> {
    let physical = |l: GraphicsCopyLayout| -> Result<usize, GraphicsError> {
        let n = l.byte_len()?;
        if matches!(l, GraphicsCopyLayout::Buffer { .. }) {
            n.checked_add(3)
                .map(|v| v & !3)
                .ok_or(GraphicsError::InvalidDescriptor)
        } else {
            Ok(n)
        }
    };
    let staging = |l: GraphicsCopyLayout| -> Result<usize, GraphicsError> {
        match l {
            GraphicsCopyLayout::TypedTexture(d) => d.staging(),
            _ => physical(l),
        }
    };
    let host = |l: GraphicsCopyLayout, yes: bool| -> Result<usize, GraphicsError> {
        if !yes {
            return Ok(0);
        }
        match l {
            GraphicsCopyLayout::TypedTexture(_) => staging(l)?
                .checked_mul(2)
                .and_then(|n| n.checked_add(l.byte_len().ok()?))
                .ok_or(GraphicsError::InvalidDescriptor),
            _ => physical(l)?
                .checked_mul(2)
                .ok_or(GraphicsError::InvalidDescriptor),
        }
    };
    physical(destination)?
        .checked_add(staging(destination)?)
        .and_then(|n| n.checked_add(destination.byte_len().ok()?))
        .and_then(|n| n.checked_add(host(source, source_host).ok()?))
        .and_then(|n| n.checked_add(host(destination, destination_host).ok()?))
        .ok_or(GraphicsError::InvalidDescriptor)
}
impl GraphicsContext {
    pub fn set_metal_typed_copy_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        typed: GraphicsTypedCopyCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_array_view_queued_backend(capabilities, renderer);
        self.typed_copy_capabilities = typed;
    }
    pub fn create_typed_texture_resource(
        &mut self,
        device: GraphicsHandle,
        desc: GraphicsTypedTextureDesc,
        state: ResourceStates,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let bytes = desc.byte_len()?;
        let generic = [
            ResourceStates::COMMON,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_DEST,
            ResourceStates::PIXEL_SHADER_RESOURCE,
        ]
        .contains(&state);
        let role = if desc.format == TextureFormat::Depth32Float {
            [ResourceStates::DEPTH_READ, ResourceStates::DEPTH_WRITE].contains(&state)
        } else {
            state == ResourceStates::RENDER_TARGET
                && !matches!(
                    desc.format,
                    TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
                )
        };
        if !generic && !role {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if desc.format == TextureFormat::Depth32Float {
            let live = self.resources.values().map(|r| r.data.len()).sum::<usize>()
                + self
                    .depth_resources
                    .values()
                    .map(|r| r.typed_desc.byte_len().expect("validated depth"))
                    .sum::<usize>();
            if bytes > MAX_CONTEXT_RESOURCE_BYTES.saturating_sub(live + self.retired_bytes()) {
                return Err(MemoryError::AllocationFailed.into());
            }
            let mut data = Vec::new();
            data.try_reserve_exact(bytes)
                .map_err(|_| MemoryError::AllocationFailed)?;
            data.resize(bytes, 0);
            let snapshot: GraphicsDepthSnapshot = Arc::new(GraphicsTypedDepthSnapshot {
                desc,
                bytes: Arc::from(data),
            });
            let handle = self.allocate_handle();
            self.depth_resources.insert(
                handle,
                DepthResource {
                    typed_desc: desc,
                    extent: desc.extent,
                    state,
                    snapshot: Some(snapshot),
                },
            );
            return Ok(handle);
        }
        self.create_resource_storage_typed(
            device,
            ResourceDesc {
                dimension: ResourceDimension::Texture2D,
                width: desc.extent.0 as u64,
                height: desc.extent.1 as u32,
                depth_or_array_size: desc.array_layers.unwrap_or(1) as u16,
                mip_levels: desc.mip_levels as u16,
                format_rgba8: desc.format == TextureFormat::Rgba8Unorm,
            },
            state,
            None,
            Some(desc.format),
            desc.array_layers.is_some(),
        )
    }
    pub(super) fn copy_resource_state(
        &self,
        h: GraphicsHandle,
    ) -> Result<ResourceStates, GraphicsError> {
        self.resources
            .get(&h)
            .map(|r| r.state)
            .or_else(|| self.depth_resources.get(&h).map(|r| r.state))
            .ok_or(GraphicsError::InvalidResourceHandle)
    }
    pub(super) fn typed_copy_layout(
        &self,
        h: GraphicsHandle,
    ) -> Result<GraphicsCopyLayout, GraphicsError> {
        self.typed_copy_layout_with_guest_source(h, false)
    }

    fn typed_copy_layout_with_guest_source(
        &self,
        h: GraphicsHandle,
        allow_guest_source: bool,
    ) -> Result<GraphicsCopyLayout, GraphicsError> {
        if let Some(d) = self.depth_resources.get(&h) {
            return Ok(GraphicsCopyLayout::TypedTexture(d.typed_desc));
        }
        let r = self
            .resources
            .get(&h)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if r.desc.dimension == ResourceDimension::Buffer {
            return self.copy_layout_with_guest_source(h, allow_guest_source);
        }
        if r.guest_backing_addr.is_some() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let desc = GraphicsTypedTextureDesc {
            format: r.texture_format.ok_or(GraphicsError::InvalidDescriptor)?,
            extent: (
                usize::try_from(r.desc.width).map_err(|_| GraphicsError::InvalidDescriptor)?,
                r.desc.height as usize,
            ),
            mip_levels: r.desc.mip_levels as u32,
            array_layers: r
                .is_texture_array
                .then_some(r.desc.depth_or_array_size as u32),
        };
        if desc.byte_len()? != r.data.len() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        Ok(GraphicsCopyLayout::TypedTexture(desc))
    }
    pub(super) fn copy_submission_layout(
        &self,
        h: GraphicsHandle,
        guest: bool,
        mips: bool,
        typed: bool,
    ) -> Result<GraphicsCopyLayout, GraphicsError> {
        if typed {
            self.typed_copy_layout_with_guest_source(h, guest)
        } else {
            self.copy_layout_for_transfer(h, guest, mips, mips)
        }
    }
    pub(super) fn validate_typed_pair(
        &self,
        s: GraphicsCopyLayout,
        d: GraphicsCopyLayout,
        sl: GraphicsImageCopyLocation,
        dl: GraphicsImageCopyLocation,
        extent: (usize, usize),
        capability: bool,
    ) -> Result<(), GraphicsError> {
        let format = match (s, d) {
            (GraphicsCopyLayout::TypedTexture(s), GraphicsCopyLayout::TypedTexture(d))
                if s.format == d.format =>
            {
                s.format
            }
            (GraphicsCopyLayout::TypedTexture(t), GraphicsCopyLayout::Buffer { .. })
            | (GraphicsCopyLayout::Buffer { .. }, GraphicsCopyLayout::TypedTexture(t)) => t.format,
            _ => return Err(GraphicsError::InvalidDescriptor),
        };
        if capability && !self.typed_copy_capabilities.supports(format) {
            return Err(GraphicsError::NativeMetalRequired);
        }
        let (bw, bh) = format.block_extent();
        let block = format.block_bytes();
        if extent.0 == 0 || extent.1 == 0 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        for (l, location) in [(s, sl), (d, dl)] {
            match (l, location) {
                (
                    GraphicsCopyLayout::TypedTexture(t),
                    GraphicsImageCopyLocation::Texture {
                        mip,
                        slice,
                        origin: (x, y),
                    },
                ) => {
                    let f = t
                        .packed()?
                        .footprint(slice, mip)
                        .map_err(|_| GraphicsError::InvalidDescriptor)?;
                    let endx = x
                        .checked_add(extent.0)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    let endy = y
                        .checked_add(extent.1)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    if endx > f.width
                        || endy > f.height
                        || !x.is_multiple_of(bw)
                        || !y.is_multiple_of(bh)
                        || (!extent.0.is_multiple_of(bw) && endx != f.width)
                        || (!extent.1.is_multiple_of(bh) && endy != f.height)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                }
                (
                    GraphicsCopyLayout::Buffer { byte_len },
                    GraphicsImageCopyLocation::Buffer {
                        offset,
                        bytes_per_row,
                    },
                ) => {
                    let row = extent
                        .0
                        .div_ceil(bw)
                        .checked_mul(block)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    let rows = extent.1.div_ceil(bh);
                    if !offset.is_multiple_of(block)
                        || !bytes_per_row.is_multiple_of(block)
                        || bytes_per_row < row
                        || bytes_per_row > MAX_RENDER_DIMENSION * block
                        || bytes_per_row
                            .checked_mul(rows - 1)
                            .and_then(|n| n.checked_add(offset))
                            .and_then(|n| n.checked_add(row))
                            .is_none_or(|n| n > byte_len)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                }
                _ => return Err(GraphicsError::InvalidDescriptor),
            }
        }
        Ok(())
    }
    pub fn cmd_copy_typed_image(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
    ) -> Result<(), GraphicsError> {
        let l = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !l.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if source == destination {
            return Err(GraphicsError::InvalidDescriptor);
        }
        self.validate_typed_pair(
            self.typed_copy_layout_with_guest_source(source, true)?,
            self.typed_copy_layout(destination)?,
            source_location,
            destination_location,
            extent,
            false,
        )?;
        self.command_lists
            .get_mut(&list)
            .expect("validated list")
            .commands
            .push(GraphicsCommand::CopyImage {
                destination,
                destination_location,
                source,
                source_location,
                extent,
            });
        Ok(())
    }
    pub(super) fn commit_typed_copy(
        &mut self,
        commit: CopyCommit,
        layout: GraphicsCopyLayout,
        bytes: Vec<u8>,
    ) -> Result<GraphicsCopyOutcome, GraphicsError> {
        if (self.resources.contains_key(&commit.destination)
            || self.depth_resources.contains_key(&commit.destination))
            && self.typed_copy_layout(commit.destination)? != layout
        {
            return Err(GraphicsError::BackendError(
                "typed destination identity changed".into(),
            ));
        }
        if let Some(d) = self.depth_resources.get_mut(&commit.destination) {
            d.snapshot = Some(Arc::new(GraphicsTypedDepthSnapshot {
                desc: d.typed_desc,
                bytes: Arc::from(bytes.as_slice()),
            }));
        } else if let Some(r) = self.resources.get_mut(&commit.destination) {
            r.data.copy_from_slice(&bytes);
        }
        for (h, state) in commit.states {
            if let Some(r) = self.resources.get_mut(&h) {
                r.state = state;
            }
            if let Some(r) = self.depth_resources.get_mut(&h) {
                r.state = state;
            }
        }
        let history = &mut self
            .queues
            .get_mut(&commit.queue)
            .expect("validated queue")
            .submitted_lists;
        if history.len() == 256 {
            history.remove(0);
        }
        history.push(commit.list);
        Ok(GraphicsCopyOutcome { layout, bytes })
    }
}
