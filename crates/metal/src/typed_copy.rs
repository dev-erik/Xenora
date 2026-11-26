// SPDX-License-Identifier: GPL-3.0-or-later
//! Format-exact copies. Private textures use explicit staging, including depth.
use super::*;
use crate::{QueuedTypedImageCopy, TypedCopyInput, TypedImageCopyLocation, TypedTextureDesc};
use xenora_texture::TextureMipFootprint;

#[derive(Clone)]
pub struct RenderTypedTextureOutput {
    pub(super) point: RenderPoint,
    pub(super) texture: Retained<ProtocolObject<dyn MTLTexture>>,
    pub(super) desc: TypedTextureDesc,
    pub(super) lease: Rc<ColorLease>,
}
impl RenderTypedTextureOutput {
    pub fn array_range_view(
        &self,
        desc: crate::ArrayTextureViewDesc,
    ) -> Result<crate::RenderTypedTextureArrayView, MetalError> {
        if self.desc.array_layers.is_none() || self.point.inner.generation.poisoned.get() {
            return Err(error("invalid typed array view backing"));
        }
        Ok(crate::RenderTypedTextureArrayView {
            output: self.clone(),
            view: crate::texture_pipeline::typed_texture_array_view(
                self.texture.clone(),
                self.desc,
                desc,
            )?,
        })
    }

    pub fn desc(&self) -> TypedTextureDesc {
        self.desc
    }
    pub fn producer(&self) -> RenderPoint {
        self.point.clone()
    }
    /// Only the ordinary one-level depth surface can enter the legacy attachment API.
    pub fn depth_output(&self) -> Result<RenderDepthOutput, MetalError> {
        if self.desc.format != TextureFormat::Depth32Float
            || self.desc.mip_levels != 1
            || self.desc.array_layers.is_some()
        {
            return Err(error("typed output is not an ordinary depth attachment"));
        }
        Ok(RenderDepthOutput {
            point: self.point.clone(),
            texture: self.texture.clone(),
            extent: self.desc.extent,
        })
    }
}

#[derive(Clone)]
enum Storage {
    Buffer {
        buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
        len: usize,
    },
    Texture {
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        desc: TypedTextureDesc,
    },
}
pub(super) struct PendingTypedCopy {
    destination: Storage,
    _source: Storage,
    _prior: Storage,
    _uploads: Vec<Retained<ProtocolObject<dyn MTLBuffer>>>,
    readback: Retained<ProtocolObject<dyn MTLBuffer>>,
    lease: Rc<ColorLease>,
}
struct StageLevel {
    footprint: TextureMipFootprint,
    offset: usize,
    pitch: usize,
    mip: usize,
    slice: usize,
}
struct Layout {
    levels: Vec<StageLevel>,
    bytes: usize,
    staging: usize,
}
fn align(value: usize, alignment: usize) -> Result<usize, MetalError> {
    value
        .checked_add(alignment - 1)
        .map(|v| v / alignment * alignment)
        .ok_or_else(|| error("typed copy size overflow"))
}
fn layout(desc: TypedTextureDesc) -> Result<Layout, MetalError> {
    let [w, h] = desc.extent;
    let layers = desc.array_layers.unwrap_or(1);
    if w == 0
        || h == 0
        || w > MAX_RENDER_DIMENSION
        || h > MAX_RENDER_DIMENSION
        || layers == 0
        || layers > 256
        || desc.mip_levels.checked_mul(layers).is_none_or(|n| n > 4096)
    {
        return Err(error("typed texture dimensions"));
    }
    let packed = PackedTextureArrayLayout::new(desc.format, w, h, desc.mip_levels, layers)
        .map_err(|_| error("typed texture layout"))?;
    if packed.byte_len() > 64 * 1024 * 1024 {
        return Err(error("typed texture too large"));
    }
    let mut levels = Vec::new();
    let mut staging = 0usize;
    for slice in 0..layers {
        for mip in 0..desc.mip_levels {
            let footprint = packed
                .footprint(slice, mip)
                .map_err(|_| error("typed subresource"))?;
            let pitch = align(footprint.row_bytes, 256)?;
            let offset = staging;
            staging = staging
                .checked_add(
                    pitch
                        .checked_mul(footprint.rows)
                        .ok_or_else(|| error("staging overflow"))?,
                )
                .ok_or_else(|| error("staging overflow"))?;
            levels.push(StageLevel {
                footprint,
                offset,
                pitch,
                mip: mip as usize,
                slice: slice as usize,
            });
        }
    }
    Ok(Layout {
        levels,
        bytes: packed.byte_len(),
        staging,
    })
}
fn pixel_format(format: TextureFormat) -> MTLPixelFormat {
    match format {
        TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        TextureFormat::R8Unorm => MTLPixelFormat::R8Unorm,
        TextureFormat::Rg8Unorm => MTLPixelFormat::RG8Unorm,
        TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        TextureFormat::R32Float => MTLPixelFormat::R32Float,
        TextureFormat::R32Uint => MTLPixelFormat::R32Uint,
        TextureFormat::Depth32Float => MTLPixelFormat::Depth32Float,
        TextureFormat::Bc1RgbaUnorm => MTLPixelFormat::BC1_RGBA,
        TextureFormat::Bc3RgbaUnorm => MTLPixelFormat::BC3_RGBA,
    }
}
fn texture(
    gpu: &NativeCompute,
    desc: TypedTextureDesc,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
    // SAFETY: The descriptor was completely validated before reservation/allocation.
    let d = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            pixel_format(desc.format),
            desc.extent[0],
            desc.extent[1],
            desc.mip_levels > 1,
        )
    };
    unsafe {
        d.setMipmapLevelCount(desc.mip_levels as usize);
    }
    if let Some(layers) = desc.array_layers {
        unsafe {
            d.setTextureType(MTLTextureType::Type2DArray);
            d.setArrayLength(layers as usize);
        }
    }
    d.setStorageMode(MTLStorageMode::Private);
    let usage = if matches!(
        desc.format,
        TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
    ) {
        MTLTextureUsage::ShaderRead
    } else {
        MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget
    };
    d.setUsage(usage);
    gpu.device
        .newTextureWithDescriptor(&d)
        .ok_or_else(|| error("typed texture allocation"))
}
fn descriptor(
    input: &TypedCopyInput<'_>,
) -> Result<(Option<TypedTextureDesc>, usize, bool), MetalError> {
    let (desc, len, host) = match input {
        TypedCopyInput::HostBuffer(b) => (None, b.len(), true),
        TypedCopyInput::ProducedBuffer(b) => (None, b.byte_len, false),
        TypedCopyInput::HostTexture { desc, bytes } => (Some(*desc), bytes.len(), true),
        TypedCopyInput::ProducedTexture(t) => (Some(t.desc), layout(t.desc)?.bytes, false),
        TypedCopyInput::ProducedColor(t) => {
            let desc = TypedTextureDesc {
                format: TextureFormat::Rgba8Unorm,
                extent: t.extent,
                mip_levels: t.mip_levels(),
                array_layers: t.is_array().then_some(t.array_layers()),
            };
            (Some(desc), layout(desc)?.bytes, false)
        }
        TypedCopyInput::ProducedDepth(t) => (
            Some(TypedTextureDesc {
                format: TextureFormat::Depth32Float,
                extent: t.extent,
                mip_levels: 1,
                array_layers: None,
            }),
            t.extent[0]
                .checked_mul(t.extent[1])
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| error("depth length"))?,
            false,
        ),
    };
    if len == 0
        || len > 64 * 1024 * 1024
        || desc.is_some_and(|d| layout(d).is_ok_and(|l| l.bytes != len))
    {
        return Err(error("typed copy byte length"));
    }
    if let Some(d) = desc {
        layout(d)?;
    }
    Ok((desc, len, host))
}
fn validate_location(
    desc: Option<TypedTextureDesc>,
    len: usize,
    location: TypedImageCopyLocation,
    extent: [usize; 2],
    format: TextureFormat,
) -> Result<(), MetalError> {
    let (bw, bh) = format.block_extent();
    let block = format.block_bytes();
    if extent.contains(&0) {
        return Err(error("empty typed region"));
    }
    match (desc, location) {
        (Some(d), TypedImageCopyLocation::Texture { mip, slice, origin }) => {
            let l = layout(d)?;
            let f = &l
                .levels
                .iter()
                .find(|v| v.mip == mip as usize && v.slice == slice as usize)
                .ok_or_else(|| error("typed subresource"))?
                .footprint;
            let x = origin[0]
                .checked_add(extent[0])
                .ok_or_else(|| error("typed x overflow"))?;
            let y = origin[1]
                .checked_add(extent[1])
                .ok_or_else(|| error("typed y overflow"))?;
            if d.format != format
                || x > f.width
                || y > f.height
                || !origin[0].is_multiple_of(bw)
                || !origin[1].is_multiple_of(bh)
                || (!extent[0].is_multiple_of(bw) && x != f.width)
                || (!extent[1].is_multiple_of(bh) && y != f.height)
            {
                return Err(error("typed region bounds or block alignment"));
            }
        }
        (
            None,
            TypedImageCopyLocation::Buffer {
                offset,
                bytes_per_row,
            },
        ) => {
            let row = extent[0]
                .div_ceil(bw)
                .checked_mul(block)
                .ok_or_else(|| error("typed row overflow"))?;
            let rows = extent[1].div_ceil(bh);
            let end = bytes_per_row
                .checked_mul(rows - 1)
                .and_then(|n| n.checked_add(row))
                .and_then(|n| n.checked_add(offset))
                .ok_or_else(|| error("typed buffer range overflow"))?;
            if !offset.is_multiple_of(block)
                || !bytes_per_row.is_multiple_of(block)
                || bytes_per_row < row
                || bytes_per_row > MAX_RENDER_DIMENSION * block
                || end > len
            {
                return Err(error("typed buffer bounds or pitch"));
            }
        }
        _ => return Err(error("typed endpoint kind mismatch")),
    }
    Ok(())
}
fn origin2(o: [usize; 2]) -> MTLOrigin {
    MTLOrigin {
        x: o[0],
        y: o[1],
        z: 0,
    }
}
fn region_size(e: [usize; 2]) -> MTLSize {
    MTLSize {
        width: e[0],
        height: e[1],
        depth: 1,
    }
}

impl RenderCohort {
    pub(super) fn is_typed_copy(&self, p: &RenderPoint) -> Result<bool, MetalError> {
        self.validate_point(p)?;
        Ok(matches!(
            self.entries[self.entry_slot(p)?]
                .as_ref()
                .expect("entry")
                .pending,
            PendingWork::TypedCopy(_)
        ))
    }
    pub(super) fn typed_copy_output(
        &self,
        p: &RenderPoint,
    ) -> Result<RenderCopyOutput, MetalError> {
        self.validate_point(p)?;
        let PendingWork::TypedCopy(c) = &self.entries[self.entry_slot(p)?]
            .as_ref()
            .expect("entry")
            .pending
        else {
            return Err(error("not typed copy"));
        };
        Ok(match &c.destination {
            Storage::Buffer { buffer, len } => RenderCopyOutput::Buffer(RenderBufferOutput {
                point: p.clone(),
                buffer: buffer.clone(),
                byte_len: *len,
                lease: c.lease.clone(),
            }),
            Storage::Texture { texture, desc } => {
                RenderCopyOutput::TypedTexture(RenderTypedTextureOutput {
                    point: p.clone(),
                    texture: texture.clone(),
                    desc: *desc,
                    lease: c.lease.clone(),
                })
            }
        })
    }
    pub(super) fn collect_typed_copy(
        &mut self,
        p: &RenderPoint,
    ) -> Result<OwnedCopyOutput, MetalError> {
        self.validate_point(p)?;
        let slot = self.entry_slot(p)?;
        if !matches!(
            self.entries[slot].as_ref().expect("entry").pending,
            PendingWork::TypedCopy(_)
        ) {
            return Err(error("not typed copy"));
        }
        Self::drain_point(&p.inner);
        if !self.is_complete(p)? {
            return Err(error("typed copy incomplete"));
        }
        let PendingWork::TypedCopy(copy) = self.take_entry(slot).pending else {
            unreachable!("checked kind")
        };
        let result = match copy.destination {
            Storage::Buffer { len, .. } => {
                let mut bytes = vec![0; len];
                // SAFETY: Drained successful command; readback contains the complete initialized buffer.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        copy.readback.contents().cast::<u8>().as_ptr(),
                        bytes.as_mut_ptr(),
                        len,
                    );
                }
                OwnedCopyOutput::Buffer(bytes)
            }
            Storage::Texture { desc, .. } => {
                let l = layout(desc)?;
                let mut bytes = vec![0; l.bytes];
                for level in &l.levels {
                    for row in 0..level.footprint.rows {
                        // SAFETY: Each checked row belongs to the completed shared staging allocation.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                copy.readback
                                    .contents()
                                    .cast::<u8>()
                                    .as_ptr()
                                    .add(level.offset + row * level.pitch),
                                bytes
                                    .as_mut_ptr()
                                    .add(level.footprint.offset + row * level.footprint.row_bytes),
                                level.footprint.row_bytes,
                            );
                        }
                    }
                }
                OwnedCopyOutput::TypedTexture { desc, bytes }
            }
        };
        self.release_collected_latest(p);
        Ok(result)
    }

    pub fn submit_typed_image_copy(
        &mut self,
        queue: RenderQueue,
        copy: QueuedTypedImageCopy<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        let index = queue.index();
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let slot = self.admission_slot(index)?;
        let (sd, sl, sh) = descriptor(&copy.source)?;
        let (dd, dl, dh) = descriptor(&copy.destination)?;
        let format = sd
            .or(dd)
            .ok_or_else(|| error("typed image copy needs texture"))?
            .format;
        if sd.zip(dd).is_some_and(|(s, d)| s.format != d.format) {
            return Err(error("typed copy format mismatch"));
        }
        validate_location(sd, sl, copy.source_location, copy.extent, format)?;
        validate_location(dd, dl, copy.destination_location, copy.extent, format)?;
        let gpu = &self.queues[index];
        if matches!(
            format,
            TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
        ) && !gpu.device.supportsBCTextureCompression()
        {
            return Err(error("BC copy unsupported by device"));
        }
        let mut deps: Vec<Rc<Point>> = self.latest[index].iter().map(|p| p.inner.clone()).collect();
        let mut leases = Vec::new();
        let mut add = |p: &RenderPoint| -> Result<(), MetalError> {
            self.validate_point(p)?;
            if !deps.iter().any(|v| Rc::ptr_eq(v, &p.inner)) {
                deps.push(p.inner.clone());
            }
            Ok(())
        };
        for p in dependencies {
            add(p)?;
        }
        let mut identities = Vec::new();
        for input in [&copy.source, &copy.destination] {
            match input {
                TypedCopyInput::ProducedBuffer(b) => {
                    add(&b.point)?;
                    if b.buffer.device().registryID() != gpu.device.registryID() {
                        return Err(error("foreign typed buffer"));
                    }
                    identities.push((&*b.buffer as *const _ as *const ()) as usize);
                    leases.push(b.lease.clone());
                }
                TypedCopyInput::ProducedTexture(t) => {
                    add(&t.point)?;
                    if t.texture.device().registryID() != gpu.device.registryID() {
                        return Err(error("foreign typed texture"));
                    }
                    identities.push((&*t.texture as *const _ as *const ()) as usize);
                    leases.push(t.lease.clone());
                }
                TypedCopyInput::ProducedColor(t) => {
                    add(&t.point)?;
                    if t.texture.device().registryID() != gpu.device.registryID() {
                        return Err(error("foreign color texture"));
                    }
                    identities.push((&*t.texture as *const _ as *const ()) as usize);
                    leases.push(t._lease.clone());
                }
                TypedCopyInput::ProducedDepth(t) => {
                    add(&t.point)?;
                    if t.texture.device().registryID() != gpu.device.registryID() {
                        return Err(error("foreign depth texture"));
                    }
                    identities.push((&*t.texture as *const _ as *const ()) as usize);
                    leases.push(
                        t.point
                            .inner
                            .depth_lease
                            .clone()
                            .ok_or_else(|| error("missing depth lease"))?,
                    );
                }
                _ => {}
            }
        }
        if identities.len() == 2 && identities[0] == identities[1] {
            return Err(error("typed copy aliases backing"));
        }
        if deps.len() > self.entries.len() {
            return Err(error("too many dependencies"));
        }
        for p in &deps {
            if Self::poll_point(p)? == Some(false) {
                self.generation.poisoned.set(true);
                return Err(error("producer failed"));
            }
        }
        deps.retain(|p| p.terminal.get() != Some(true));
        let value = self.sequence[index]
            .checked_add(1)
            .ok_or_else(|| error("timeline exhausted"))?;
        let output_bytes = if dd.is_some() { dl } else { align(dl, 4)? };
        let readback_bytes = if let Some(d) = dd {
            layout(d)?.staging
        } else {
            align(dl, 4)?
        };
        let host_cost =
            |d: Option<TypedTextureDesc>, len: usize, host: bool| -> Result<usize, MetalError> {
                if !host {
                    return Ok(0);
                }
                if let Some(d) = d {
                    layout(d)?
                        .staging
                        .checked_mul(2)
                        .and_then(|n| n.checked_add(len))
                        .ok_or_else(|| error("host staging charge"))
                } else {
                    align(len, 4)?
                        .checked_mul(2)
                        .ok_or_else(|| error("host buffer charge"))
                }
            };
        let scratch = readback_bytes
            .checked_add(dl)
            .and_then(|n| n.checked_add(host_cost(sd, sl, sh).ok()?))
            .and_then(|n| n.checked_add(host_cost(dd, dl, dh).ok()?))
            .ok_or_else(|| error("typed copy reservation"))?;
        let lease = ColorLease::reserve(&self.color_bytes, output_bytes)?;
        let temporary = ColorLease::reserve(&self.color_bytes, scratch)?;
        let command = gpu
            .queue
            .commandBuffer()
            .ok_or_else(|| error("typed command allocation"))?;
        for p in &deps {
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*p.event);
            command.encodeWaitForEvent_value(event, p.value);
        }
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("typed blit allocation"))?;
        let mut uploads = Vec::new();
        let source = upload(gpu, copy.source, &blit, &mut uploads)?;
        let prior = upload(gpu, copy.destination, &blit, &mut uploads)?;
        let destination = match &prior {
            Storage::Buffer { len, .. } => Storage::Buffer {
                buffer: gpu
                    .device
                    .newBufferWithLength_options(
                        align(*len, 4)?,
                        objc2_metal::MTLResourceOptions::StorageModeShared,
                    )
                    .ok_or_else(|| error("typed destination buffer allocation"))?,
                len: *len,
            },
            Storage::Texture { desc, .. } => Storage::Texture {
                texture: texture(gpu, *desc)?,
                desc: *desc,
            },
        };
        // Allocate directly: a copied zero Vec would add a second padded staging
        // allocation not represented by the readback-plus-packed-result charge.
        // Every byte collected below is written by the GPU before successful
        // completion. Texture row padding is neither read nor published.
        let readback = gpu
            .device
            .newBufferWithLength_options(
                readback_bytes,
                objc2_metal::MTLResourceOptions::StorageModeShared,
            )
            .ok_or_else(|| error("typed readback allocation"))?;
        // SAFETY: All layouts, regions, formats and allocations were validated above; waits precede every blit.
        unsafe {
            match (&prior, &destination) {
                (Storage::Buffer { buffer: s, len }, Storage::Buffer { buffer: d, .. }) => blit
                    .copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                        s,
                        0,
                        d,
                        0,
                        align(*len, 4)?,
                    ),
                (Storage::Texture { texture: s, desc }, Storage::Texture { texture: d, .. }) => {
                    for l in layout(*desc)?.levels {
                        blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(s,l.slice,l.mip,origin2([0,0]),region_size([l.footprint.width,l.footprint.height]),d,l.slice,l.mip,origin2([0,0]));
                    }
                }
                _ => unreachable!("destination matches prior"),
            }
            match (&source,&destination,copy.source_location,copy.destination_location) {
                (Storage::Texture{texture:s,..},Storage::Texture{texture:d,..},TypedImageCopyLocation::Texture{mip:sm,slice:ss,origin:so},TypedImageCopyLocation::Texture{mip:dm,slice:ds,origin:do_})=>blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(s,ss as usize,sm as usize,origin2(so),region_size(copy.extent),d,ds as usize,dm as usize,origin2(do_)),
                (Storage::Buffer{buffer:s,..},Storage::Texture{texture:d,..},TypedImageCopyLocation::Buffer{offset,bytes_per_row},TypedImageCopyLocation::Texture{mip,slice,origin})=>blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(s,offset,bytes_per_row,0,region_size(copy.extent),d,slice as usize,mip as usize,origin2(origin)),
                (Storage::Texture{texture:s,..},Storage::Buffer{buffer:d,..},TypedImageCopyLocation::Texture{mip,slice,origin},TypedImageCopyLocation::Buffer{offset,bytes_per_row})=>blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(s,slice as usize,mip as usize,origin2(origin),region_size(copy.extent),d,offset,bytes_per_row,0),
                _=>unreachable!("validated typed endpoints"),
            }
            match &destination {
                Storage::Buffer { buffer, len } => blit
                    .copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                        buffer,
                        0,
                        &readback,
                        0,
                        align(*len, 4)?,
                    ),
                Storage::Texture { texture, desc } => {
                    for l in layout(*desc)?.levels {
                        blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(texture,l.slice,l.mip,origin2([0,0]),region_size([l.footprint.width,l.footprint.height]),&readback,l.offset,l.pitch,0);
                    }
                }
            }
        }
        blit.endEncoding();
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*self.events[index]);
        command.encodeSignalEvent_value(event, value);
        leases.push(lease.clone());
        leases.push(temporary);
        let point = RenderPoint {
            inner: Rc::new(Point {
                generation: self.generation.clone(),
                queue: index,
                value,
                command: command.clone(),
                event: self.events[index].clone(),
                dependencies: RefCell::new(deps),
                terminal: Cell::new(None),
                validation: None,
                _leases: leases,
                depth_lease: dd
                    .filter(|d| d.format == TextureFormat::Depth32Float)
                    .map(|_| lease.clone()),
            }),
        };
        command.commit();
        self.sequence[index] = value;
        self.latest[index] = Some(point.clone());
        self.entries[slot] = Some(Entry {
            point: point.clone(),
            pending: PendingWork::TypedCopy(PendingTypedCopy {
                destination,
                _source: source,
                _prior: prior,
                _uploads: uploads,
                readback,
                lease,
            }),
        });
        Ok(point)
    }
}
fn upload(
    gpu: &NativeCompute,
    input: TypedCopyInput<'_>,
    blit: &ProtocolObject<dyn MTLBlitCommandEncoder>,
    uploads: &mut Vec<Retained<ProtocolObject<dyn MTLBuffer>>>,
) -> Result<Storage, MetalError> {
    Ok(match input {
        TypedCopyInput::ProducedBuffer(b) => Storage::Buffer {
            buffer: b.buffer,
            len: b.byte_len,
        },
        TypedCopyInput::ProducedTexture(t) => Storage::Texture {
            texture: t.texture,
            desc: t.desc,
        },
        TypedCopyInput::ProducedColor(t) => {
            let desc = TypedTextureDesc {
                format: TextureFormat::Rgba8Unorm,
                extent: t.extent,
                mip_levels: t.mip_levels(),
                array_layers: t.is_array().then_some(t.array_layers()),
            };
            Storage::Texture {
                texture: t.texture,
                desc,
            }
        }
        TypedCopyInput::ProducedDepth(t) => Storage::Texture {
            texture: t.texture,
            desc: TypedTextureDesc {
                format: TextureFormat::Depth32Float,
                extent: t.extent,
                mip_levels: 1,
                array_layers: None,
            },
        },
        TypedCopyInput::HostBuffer(bytes) => {
            let mut words = vec![0u32; align(bytes.len(), 4)? / 4];
            for (i, b) in bytes.iter().enumerate() {
                words[i / 4] |= u32::from(*b) << ((i % 4) * 8);
            }
            Storage::Buffer {
                buffer: gpu.new_buffer(&words)?,
                len: bytes.len(),
            }
        }
        TypedCopyInput::HostTexture { desc, bytes } => {
            let l = layout(desc)?;
            let mut words = vec![0u32; l.staging / 4];
            for level in &l.levels {
                for row in 0..level.footprint.rows {
                    for col in 0..level.footprint.row_bytes {
                        let i = level.offset + row * level.pitch + col;
                        words[i / 4] |= u32::from(
                            bytes[level.footprint.offset + row * level.footprint.row_bytes + col],
                        ) << ((i % 4) * 8);
                    }
                }
            }
            let staging = gpu.new_buffer(&words)?;
            let t = texture(gpu, desc)?;
            for level in &l.levels {
                // SAFETY: Complete initialized staging, checked subresource footprints and private destination.
                unsafe {
                    blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(&staging,level.offset,level.pitch,0,region_size([level.footprint.width,level.footprint.height]),&t,level.slice,level.mip,origin2([0,0]));
                }
            }
            uploads.push(staging);
            Storage::Texture { texture: t, desc }
        }
    })
}

impl NativeCompute {
    /// Guest format-code bits 0..9 supported by the exact-copy backend.
    /// BC remains an explicit runtime capability; no decompression fallback exists.
    pub fn typed_copy_capabilities(&self) -> u16 {
        let all = (1u16 << 10) - 1;
        if self.device.supportsBCTextureCompression() {
            all
        } else {
            all & !((1 << 1) | (1 << 2))
        }
    }
}

impl NativeCompute {
    /// Host-only immutable upload. Produced textures must use their exact cohort output.
    pub(crate) fn upload_typed_texture_storage(
        &self,
        desc: TypedTextureDesc,
        bytes: &[u8],
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
        let l = layout(desc)?;
        if bytes.len() != l.bytes
            || l.staging
                .checked_mul(2)
                .and_then(|n| n.checked_add(l.bytes))
                .is_none_or(|n| n > 256 * 1024 * 1024)
        {
            return Err(error("typed sampled upload footprint"));
        }
        if matches!(
            desc.format,
            TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm
        ) && !self.device.supportsBCTextureCompression()
        {
            return Err(error("BC textures unsupported"));
        }
        let command = self
            .queue
            .commandBuffer()
            .ok_or_else(|| error("typed upload command"))?;
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("typed upload encoder"))?;
        let mut uploads = Vec::new();
        let storage = upload(
            self,
            TypedCopyInput::HostTexture { desc, bytes },
            &blit,
            &mut uploads,
        )?;
        blit.endEncoding();
        command.commit();
        command.waitUntilCompleted();
        if command.status() != MTLCommandBufferStatus::Completed {
            return Err(error("typed upload failed"));
        }
        let Storage::Texture { texture, .. } = storage else {
            unreachable!("texture upload")
        };
        Ok(texture)
    }
}

impl RenderTypedTextureOutput {
    pub fn view(
        &self,
        desc: crate::TextureViewDesc,
    ) -> Result<crate::RenderTypedTextureView, MetalError> {
        if self.desc.array_layers.is_some() || self.point.inner.generation.poisoned.get() {
            return Err(error("invalid typed 2D view backing"));
        }
        Ok(crate::RenderTypedTextureView {
            output: self.clone(),
            view: crate::texture_pipeline::typed_texture_view(
                self.texture.clone(),
                self.desc,
                desc,
                None,
            )?,
        })
    }
    pub fn array_slice_view(
        &self,
        desc: crate::ArraySliceViewDesc,
    ) -> Result<crate::RenderTypedTextureView, MetalError> {
        if self.desc.array_layers.is_none() || self.point.inner.generation.poisoned.get() {
            return Err(error("invalid typed array view backing"));
        }
        let view = crate::TextureViewDesc {
            base_mip: desc.base_mip,
            mip_count: desc.mip_count,
            format: desc.format,
        };
        Ok(crate::RenderTypedTextureView {
            output: self.clone(),
            view: crate::texture_pipeline::typed_texture_view(
                self.texture.clone(),
                self.desc,
                view,
                Some(desc.slice),
            )?,
        })
    }
}
impl crate::RenderTypedTextureView {
    pub fn desc(&self) -> crate::TextureViewDesc {
        self.view.desc()
    }
    pub fn slice(&self) -> Option<u32> {
        self.view.slice()
    }
    pub fn extent(&self) -> [usize; 2] {
        self.view.extent()
    }
    pub fn producer(&self) -> RenderPoint {
        self.output.producer()
    }
}
impl RenderDepthOutput {
    /// Borrow the exact rendered depth storage as typed sampling input, without a copy.
    pub fn typed_output(&self) -> Result<RenderTypedTextureOutput, MetalError> {
        if self.point.inner.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        Ok(RenderTypedTextureOutput {
            point: self.point.clone(),
            texture: self.texture.clone(),
            desc: TypedTextureDesc {
                format: TextureFormat::Depth32Float,
                extent: self.extent,
                mip_levels: 1,
                array_layers: None,
            },
            lease: self
                .point
                .inner
                .depth_lease
                .clone()
                .ok_or_else(|| error("depth lease missing"))?,
        })
    }
}

impl RenderTypedTextureOutput {
    pub fn rgba8_output(&self) -> Result<RenderColorOutput, MetalError> {
        if self.desc.format != TextureFormat::Rgba8Unorm
            || self.point.inner.generation.poisoned.get()
        {
            return Err(error("typed output is not RGBA8"));
        }
        Ok(RenderColorOutput {
            point: self.point.clone(),
            texture: self.texture.clone(),
            extent: self.desc.extent,
            _lease: self.lease.clone(),
        })
    }
}
impl RenderColorOutput {
    pub fn typed_output(&self) -> Result<RenderTypedTextureOutput, MetalError> {
        if self.point.inner.generation.poisoned.get()
            || self.texture.pixelFormat() != MTLPixelFormat::RGBA8Unorm
        {
            return Err(error("invalid RGBA8 output"));
        }
        Ok(RenderTypedTextureOutput {
            point: self.point.clone(),
            texture: self.texture.clone(),
            desc: TypedTextureDesc {
                format: TextureFormat::Rgba8Unorm,
                extent: self.extent,
                mip_levels: self.mip_levels(),
                array_layers: self.is_array().then_some(self.array_layers()),
            },
            lease: self._lease.clone(),
        })
    }
}

impl crate::RenderTypedTextureArrayView {
    pub fn desc(&self) -> crate::ArrayTextureViewDesc {
        self.view.desc()
    }
    pub fn extent(&self) -> [usize; 2] {
        self.view.extent()
    }
    pub fn producer(&self) -> RenderPoint {
        self.output.producer()
    }
}
