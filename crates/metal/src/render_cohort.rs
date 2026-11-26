// SPDX-License-Identifier: GPL-3.0-or-later
//! Worker-local two-queue rendering; event progress is never a success result.
use super::*;
use crate::compute::{size, verify_shader_validation};
use crate::{
    OwnedCopyOutput, QueuedColorLoad, QueuedCopyInput, QueuedDepthInput, RenderCopyOutput,
    RenderDepthOutput,
};
use objc2_metal::{
    MTLComputeCommandEncoder, MTLComputePipelineState, MTLEvent, MTLResource, MTLSharedEvent,
    MTLTextureType,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::rc::Rc;
use xenora_texture::{PackedTextureArrayLayout, PackedTextureLayout, TextureFormat};

#[path = "render_geometry.rs"]
pub(super) mod geometry;

#[path = "typed_copy.rs"]
mod typed_copy;
pub use typed_copy::RenderTypedTextureOutput;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderQueue {
    First,
    Second,
}
impl RenderQueue {
    fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

pub struct OwnedDepthInput {
    pub snapshot: Option<DepthSnapshot>,
    pub load: DepthLoad,
}
enum DepthInput {
    Legacy(OwnedDepthInput),
    Queued(QueuedDepthInput),
}
impl RenderDepthOutput {
    pub fn extent(&self) -> [usize; 2] {
        self.extent
    }
    pub fn producer(&self) -> RenderPoint {
        self.point.clone()
    }
}
pub struct OwnedRenderOutput {
    pub colors: Vec<Vec<u8>>,
    pub color_layouts: Vec<crate::TypedTextureDesc>,
    pub depth: Option<DepthSnapshot>,
}
struct Generation {
    poisoned: Cell<bool>,
}
struct Point {
    generation: Rc<Generation>,
    queue: usize,
    value: u64,
    command: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    dependencies: RefCell<Vec<Rc<Point>>>,
    terminal: Cell<Option<bool>>,
    validation: Option<Rc<geometry::Validation>>,
    _leases: Vec<Rc<ColorLease>>,
    depth_lease: Option<Rc<ColorLease>>,
}
/// Opaque committed submission identity. Cannot be constructed from a numeric value.
#[derive(Clone)]
pub struct RenderPoint {
    inner: Rc<Point>,
}
/// Immutable produced surface. No raw texture, upload, or ordinary binding conversion.
/// External clones pin their surface and point independently of live receipt slots;
/// the caller must budget/drop these versions. Terminal points release ancestors.
///
/// ```compile_fail
/// fn cannot_upload(output: xenora_metal::RenderColorOutput, mip: &xenora_metal::Rgba8MipLevel<'_>) {
///     output.upload_mip(mip);
/// }
/// ```
/// ```compile_fail
/// fn cannot_extract(output: xenora_metal::RenderColorOutput) {
///     output.raw();
/// }
/// ```
/// ```compile_fail
/// use xenora_metal::{RenderColorOutput, MetalSampler, BoundSampledTexture, BindingSlot};
/// fn cannot_bind(output: RenderColorOutput, sampler: MetalSampler) {
///     BoundSampledTexture::bind(output, sampler, BindingSlot::new(0), BindingSlot::new(0));
/// }
/// ```
/// ```compile_fail
/// use xenora_metal::{RenderColorOutput, NativeCompute, MetalSampler};
/// fn cannot_read(output: RenderColorOutput, gpu: &NativeCompute, sampler: &MetalSampler) {
///     gpu.sample_rgba8(&output, sampler, &[[0.5, 0.5]]);
/// }
/// ```
#[derive(Clone)]
pub struct RenderColorOutput {
    point: RenderPoint,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    extent: [usize; 2],
    _lease: Rc<ColorLease>,
}
/// Immutable view of a produced surface; producer provenance never becomes an
/// ordinary uploaded view or binding. Only RenderCohort can consume it.
///
/// ```compile_fail
/// fn cannot_extract(v: xenora_metal::RenderColorView) { v.raw(); }
/// ```
/// ```compile_fail
/// use xenora_metal::{RenderColorView, SampledBinding, MetalSampler, BindingSlot};
/// fn cannot_detach(v: RenderColorView, sampler: MetalSampler) {
///     let _ = SampledBinding::View { view: v, sampler, texture_slot: BindingSlot::new(0), sampler_slot: BindingSlot::new(0) };
/// }
/// ```
#[derive(Clone)]
pub struct RenderColorView {
    output: RenderColorOutput,
    view: crate::MetalTextureView,
}
impl RenderColorView {
    pub fn slice(&self) -> Option<u32> {
        self.view.slice()
    }
    pub fn desc(&self) -> crate::TextureViewDesc {
        self.view.desc()
    }
    pub fn extent(&self) -> [usize; 2] {
        self.view.extent()
    }
    pub fn producer(&self) -> RenderPoint {
        self.output.producer()
    }
}
impl RenderColorOutput {
    pub fn array_slice_view(
        &self,
        desc: crate::ArraySliceViewDesc,
    ) -> Result<RenderColorView, MetalError> {
        if !self.is_array() || self.point.inner.generation.poisoned.get() {
            return Err(error("invalid array view backing"));
        }
        let owner = crate::MetalRgba8ArrayTexture::from_render_output(
            self.texture.clone(),
            crate::Rgba8ArrayTextureDesc {
                width: self.extent[0],
                height: self.extent[1],
                mips: self.mip_levels(),
                array_layers: self.array_layers(),
            },
        )?;
        Ok(RenderColorView {
            output: self.clone(),
            view: owner.array_slice_view(desc)?,
        })
    }

    pub fn is_array(&self) -> bool {
        self.texture.textureType() == MTLTextureType::Type2DArray
    }
    pub fn array_layers(&self) -> u32 {
        self.texture.arrayLength() as u32
    }
    pub fn mip_levels(&self) -> u32 {
        self.texture.mipmapLevelCount() as u32
    }
    pub fn view(&self, desc: crate::TextureViewDesc) -> Result<RenderColorView, MetalError> {
        if self.is_array() {
            return Err(error("array views unsupported"));
        }
        if self.point.inner.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let parent = crate::MetalRgba8Texture::from_render_output(
            self.texture.clone(),
            self.extent[0],
            self.extent[1],
        )?;
        Ok(RenderColorView {
            output: self.clone(),
            view: parent.view(desc)?,
        })
    }

    pub fn extent(&self) -> [usize; 2] {
        self.extent
    }
    pub fn producer(&self) -> RenderPoint {
        self.point.clone()
    }
}
#[derive(Clone)]
pub enum QueuedSampledBinding {
    ArrayViewed {
        view: crate::MetalTextureArrayView,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
    ProducedTypedArrayView {
        view: crate::RenderTypedTextureArrayView,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
    ProducedTypedView {
        view: crate::RenderTypedTextureView,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
    Uploaded(crate::BoundSampledTexture),
    Viewed {
        view: crate::MetalTextureView,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
    ProducedView {
        view: RenderColorView,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
    Produced {
        output: RenderColorOutput,
        sampler: crate::MetalSampler,
        texture_slot: crate::BindingSlot,
        sampler_slot: crate::BindingSlot,
    },
}
// A typed pass has at most 8 targets * 1Mi pixels * 8 bytes = 64 MiB.
// Concurrent passes and historical output clones share a conservative 256 MiB
// payload ceiling. One exported slot pins its entire submission charge.
// Also reserve submission geometry/uniform copies, depth staging and readbacks.
// Borrowed caller uploads, returned host results and opaque driver overhead are not
// owned here. Conservative charges remain while command-owning points survive.
const MAX_COLOR_VERSION_BYTES: usize = 256 * 1024 * 1024;
pub(super) struct ColorLease {
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl ColorLease {
    fn reserve(budget: &Rc<Cell<usize>>, bytes: usize) -> Result<Rc<Self>, MetalError> {
        let total = budget
            .get()
            .checked_add(bytes)
            .filter(|total| *total <= MAX_COLOR_VERSION_BYTES)
            .ok_or_else(|| error("submission payload budget exhausted"))?;
        budget.set(total);
        Ok(Rc::new(Self {
            budget: budget.clone(),
            bytes,
        }))
    }
}
impl Drop for ColorLease {
    fn drop(&mut self) {
        self.budget.set(self.budget.get() - self.bytes);
    }
}

/// Immutable produced buffer; padded native storage never escapes this module.
#[derive(Clone)]
pub struct RenderBufferOutput {
    point: RenderPoint,
    byte_len: usize,
    buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    lease: Rc<ColorLease>,
}
impl RenderBufferOutput {
    pub fn byte_len(&self) -> usize {
        self.byte_len
    }
    pub fn producer(&self) -> RenderPoint {
        self.point.clone()
    }
}

#[derive(Clone)]
enum CopyStorage {
    Buffer {
        buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
        byte_len: usize,
    },
    Color {
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        extent: [usize; 2],
    },
}
struct PendingCopy {
    destination: CopyStorage,
    // Retain direct source storage, not an ancestor's historical scratch.
    _source: CopyStorage,
    _prior: Option<CopyStorage>,
    lease: Rc<ColorLease>,
}
enum PendingWork {
    Render(Box<PendingRender<'static>>),
    Copy(PendingCopy),
    TypedCopy(typed_copy::PendingTypedCopy),
}
struct Entry {
    point: RenderPoint,
    pending: PendingWork,
}

/// Owns two reusable queues. Legacy cohorts retain two entries, one per queue;
/// opt-in graph cohorts retain up to sixteen entries across either queue.
/// Rc provenance intentionally keeps this API on one worker thread.
pub struct RenderCohort {
    queues: [NativeCompute; 2],
    events: [Retained<ProtocolObject<dyn MTLSharedEvent>>; 2],
    sequence: [u64; 2],
    generation: Rc<Generation>,
    entries: Vec<Option<Entry>>,
    latest: [Option<RenderPoint>; 2],
    color_bytes: Rc<Cell<usize>>,
    buffer_region_pipeline: Option<Retained<ProtocolObject<dyn MTLComputePipelineState>>>,
}

pub(super) struct SubmissionSync {
    pub geometry: Vec<Option<Rc<geometry::Gather>>>,
    pub validation: Option<Rc<geometry::Validation>>,
    pub depth: Option<(MetalDepthTarget, DepthLoad)>,
    pub upload: Option<Retained<ProtocolObject<dyn MTLBuffer>>>,
    pub depth_clear: Option<f32>,
    pub export_depth: bool,
    pub depth_source_owner: Option<RenderDepthOutput>,
    pub depth_lease: Option<Rc<ColorLease>>,
    pub color_sources: Vec<Option<RenderTypedTextureOutput>>,
    pub color_lease: Rc<ColorLease>,
    pub temporary_lease: Rc<ColorLease>,
    waits: Vec<Rc<Point>>,
    event: Retained<ProtocolObject<dyn MTLSharedEvent>>,
    value: u64,
}
impl SubmissionSync {
    pub(super) fn encode_waits(&self, command: &ProtocolObject<dyn MTLCommandBuffer>) {
        for point in &self.waits {
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*point.event);
            command.encodeWaitForEvent_value(event, point.value);
        }
    }
    pub(super) fn encode_depth_upload(
        &self,
        command: &ProtocolObject<dyn MTLCommandBuffer>,
    ) -> Result<(), MetalError> {
        if let Some(buffer) = &self.upload {
            let depth = &self.depth.as_ref().expect("upload has depth").0;
            let [width, height] = depth.extent;
            let pitch = (width * 4).div_ceil(256) * 256;
            let blit = command
                .blitCommandEncoder()
                .ok_or_else(|| error("depth upload encoder failed"))?;
            // SAFETY: Checked packed snapshot was padded to pitch*height bytes;
            // the matching private texture and buffer stay owned through completion.
            unsafe {
                blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                    buffer, 0, pitch, pitch * height, MTLSize { width, height, depth: 1 }, &depth.texture, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 });
            }
            blit.endEncoding();
        }
        Ok(())
    }
    pub(super) fn encode_color_loads(
        &self,
        command: &ProtocolObject<dyn MTLCommandBuffer>,
        destinations: &[Retained<ProtocolObject<dyn MTLTexture>>],
    ) -> Result<(), MetalError> {
        if self.color_sources.iter().all(Option::is_none) {
            return Ok(());
        }
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("color copy encoder failed"))?;
        for (source, destination) in self.color_sources.iter().zip(destinations) {
            if let Some(source) = source {
                let [width, height] = source.desc.extent;
                // SAFETY: Preflight verifies matching device, exact linear color format and extent.
                // Both fresh destination and immutable source stay owned until completion.
                unsafe {
                    blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                        &source.texture, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 },
                        MTLSize { width, height, depth: 1 }, destination, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 });
                }
            }
        }
        blit.endEncoding();
        Ok(())
    }
    pub(super) fn encode_signal(&self, command: &ProtocolObject<dyn MTLCommandBuffer>) {
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*self.event);
        command.encodeSignalEvent_value(event, self.value);
    }
}
fn copy_chain_layout(extent: [usize; 2], mips: u32) -> Result<PackedTextureLayout, MetalError> {
    translated_pixels(extent, "copy backing")?;
    let layout = PackedTextureLayout::new(TextureFormat::Rgba8Unorm, extent[0], extent[1], mips)
        .map_err(|_| error("invalid copy mip chain"))?;
    if layout.byte_len() > 64 * 1024 * 1024 {
        return Err(error("copy mip chain too large"));
    }
    Ok(layout)
}
fn copy_array_layout(
    extent: [usize; 2],
    mips: u32,
    layers: u32,
) -> Result<PackedTextureArrayLayout, MetalError> {
    translated_pixels(extent, "array copy backing")?;
    if layers == 0 || layers > 256 || mips.checked_mul(layers).is_none_or(|n| n > 4096) {
        return Err(error("array subresource limit"));
    }
    let layout = PackedTextureArrayLayout::new(
        TextureFormat::Rgba8Unorm,
        extent[0],
        extent[1],
        mips,
        layers,
    )
    .map_err(|_| error("invalid array copy layout"))?;
    if layout.byte_len() > 64 * 1024 * 1024 {
        return Err(error("array copy byte limit"));
    }
    Ok(layout)
}
fn copy_texture(
    gpu: &NativeCompute,
    extent: [usize; 2],
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
    copy_mip_texture(gpu, extent, 1)
}
fn copy_mip_texture(
    gpu: &NativeCompute,
    extent: [usize; 2],
    mips: u32,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
    copy_storage_texture(gpu, extent, mips, None)
}
fn copy_storage_texture(
    gpu: &NativeCompute,
    extent: [usize; 2],
    mips: u32,
    array_layers: Option<u32>,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
    if let Some(layers) = array_layers {
        copy_array_layout(extent, mips, layers)?;
    }
    copy_chain_layout(extent, mips)?;
    // SAFETY: Caller validates nonzero RGBA8 dimensions using translated_pixels.
    let descriptor = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            MTLPixelFormat::RGBA8Unorm,
            extent[0],
            extent[1],
            mips > 1,
        )
    };
    // SAFETY: The complete mip count was validated above.
    unsafe {
        descriptor.setMipmapLevelCount(mips as usize);
    }
    if let Some(layers) = array_layers {
        // SAFETY: Nonzero array length and all mip/layer footprints are bounded above.
        unsafe {
            descriptor.setTextureType(MTLTextureType::Type2DArray);
            descriptor.setArrayLength(layers as usize);
        }
    }
    descriptor.setStorageMode(MTLStorageMode::Shared);
    descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
    gpu.device
        .newTextureWithDescriptor(&descriptor)
        .ok_or_else(|| error("copy texture allocation failed"))
}

fn error(detail: &str) -> MetalError {
    MetalError::new("render cohort", detail)
}
fn event(gpu: &NativeCompute) -> Result<Retained<ProtocolObject<dyn MTLSharedEvent>>, MetalError> {
    gpu.device
        .newSharedEvent()
        .ok_or_else(|| error("shared events unavailable"))
}
impl NativeCompute {
    pub fn new_render_cohort(&self) -> Result<RenderCohort, MetalError> {
        self.render_cohort_with_capacity(2)
    }
    /// Opt into sixteen retained submissions across two queues. Payload budgets
    /// are unchanged. Latest pending queue points retain ordering independently
    /// of receipt storage and are released after successful collection.
    pub fn new_render_graph_cohort(&self) -> Result<RenderCohort, MetalError> {
        self.render_cohort_with_capacity(16)
    }
    fn render_cohort_with_capacity(&self, capacity: usize) -> Result<RenderCohort, MetalError> {
        let new_queue = || -> Result<NativeCompute, MetalError> {
            Ok(NativeCompute {
                compute_pipelines: Default::default(),
                device: self.device.clone(),
                queue: self
                    .device
                    .newCommandQueue()
                    .ok_or_else(|| error("queue creation failed"))?,
            })
        };
        let queues = [new_queue()?, new_queue()?];
        let events = [event(&queues[0])?, event(&queues[1])?];
        Ok(RenderCohort {
            queues,
            events,
            sequence: [0; 2],
            generation: Rc::new(Generation {
                poisoned: Cell::new(false),
            }),
            entries: (0..capacity).map(|_| None).collect(),
            latest: [None, None],
            color_bytes: Rc::new(Cell::new(0)),
            buffer_region_pipeline: None,
        })
    }
}
impl RenderCohort {
    fn admission_slot(&self, queue: usize) -> Result<usize, MetalError> {
        if self.entries.len() == 2
            && self
                .entries
                .iter()
                .flatten()
                .any(|entry| entry.point.inner.queue == queue)
        {
            return Err(error("queue already has an uncollected submission"));
        }
        self.entries
            .iter()
            .position(Option::is_none)
            .ok_or_else(|| error("cohort submission capacity exhausted"))
    }
    fn entry_slot(&self, point: &RenderPoint) -> Result<usize, MetalError> {
        self.entries
            .iter()
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| Rc::ptr_eq(&entry.point.inner, &point.inner))
            })
            .ok_or_else(|| error("submission already collected"))
    }
    fn take_entry(&mut self, slot: usize) -> Entry {
        self.entries[slot].take().expect("validated entry")
    }
    fn release_collected_latest(&mut self, point: &RenderPoint) {
        // Successful collection proved this writer and every ancestor terminal
        // and successful. Retain a later queue point, if any; its directed edges
        // independently keep whatever ancestry it still needs. Do not pin payloads
        // solely to remember a successfully collected queue tail.
        if self.latest[point.inner.queue]
            .as_ref()
            .is_some_and(|latest| Rc::ptr_eq(&latest.inner, &point.inner))
        {
            self.latest[point.inner.queue] = None;
        }
    }
    /// Native queue access for externally controlled test gates. Gates must have a
    /// watchdog: cohort cancellation cannot release unrelated external events.
    pub fn queue(&self, queue: RenderQueue) -> &ProtocolObject<dyn MTLCommandQueue> {
        self.queues[queue.index()].queue()
    }
    fn validate_point(&self, point: &RenderPoint) -> Result<(), MetalError> {
        if !Rc::ptr_eq(&point.inner.generation, &self.generation) {
            return Err(error("foreign or stale generation"));
        }
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        Ok(())
    }
    /// Copy a bounded RGBA8 rectangle, preserving the full destination backing.
    pub fn submit_image_copy(
        &mut self,
        queue: RenderQueue,
        copy: crate::QueuedImageCopy<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_image_copy_internal(queue, copy, dependencies, false, false)
    }
    /// Copies a selected mip while preserving the complete destination chain.
    pub fn submit_mip_image_copy(
        &mut self,
        queue: RenderQueue,
        copy: crate::QueuedImageCopy<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_image_copy_internal(queue, copy, dependencies, true, false)
    }
    /// Copy one selected array subresource while preserving every other layer and mip.
    pub fn submit_array_image_copy(
        &mut self,
        queue: RenderQueue,
        copy: crate::QueuedImageCopy<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_image_copy_internal(queue, copy, dependencies, true, true)
    }
    fn submit_image_copy_internal(
        &mut self,
        queue: RenderQueue,
        copy: crate::QueuedImageCopy<'_>,
        dependencies: &[RenderPoint],
        allow_mips: bool,
        allow_arrays: bool,
    ) -> Result<RenderPoint, MetalError> {
        use crate::ImageCopyLocation;
        let endpoint =
            |location: ImageCopyLocation| -> Result<(ImageCopyLocation, usize, usize), MetalError> {
                match location {
                    ImageCopyLocation::Rgba8Mip { mip, slice, origin }
                        if allow_mips && (slice == 0 || allow_arrays) =>
                    {
                        Ok((
                            ImageCopyLocation::Rgba8 { origin },
                            mip as usize,
                            slice as usize,
                        ))
                    }
                    ImageCopyLocation::Rgba8Mip { .. } => {
                        Err(error("unsupported copy subresource"))
                    }
                    other => Ok((other, 0, 0)),
                }
            };
        let (source_location, source_level, source_slice) = endpoint(copy.source_location)?;
        let (destination_location, destination_level, destination_slice) =
            endpoint(copy.destination_location)?;
        let index = queue.index();
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let slot = self.admission_slot(index)?;
        if dependencies.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        translated_pixels(copy.extent, "image copy region")?;
        if matches!(
            (copy.source_location, copy.destination_location),
            (
                ImageCopyLocation::Buffer { .. },
                ImageCopyLocation::Buffer { .. }
            )
        ) {
            return Err(error("image copy requires a texture endpoint"));
        }
        // Returns full physical bytes and conservative host-upload charge.
        let validate = |input: &QueuedCopyInput<'_>,
                        location: ImageCopyLocation,
                        selected: usize,
                        slice: usize|
         -> Result<(usize, usize), MetalError> {
            let (logical, extent, host) = match input {
                QueuedCopyInput::HostBuffer(bytes) => (bytes.len(), None, true),
                QueuedCopyInput::ProducedBuffer(output) => (output.byte_len, None, false),
                QueuedCopyInput::HostRgba8Array {
                    extent,
                    mip_levels,
                    array_layers,
                    pixels,
                } => {
                    if !allow_arrays {
                        return Err(error("array copies require explicit entry point"));
                    }
                    let layout = copy_array_layout(*extent, *mip_levels, *array_layers)?;
                    if pixels.len() != layout.byte_len() {
                        return Err(error("invalid array host bytes"));
                    }
                    let level = layout
                        .footprint(slice as u32, selected as u32)
                        .map_err(|_| error("array subresource out of bounds"))?;
                    (layout.byte_len(), Some([level.width, level.height]), true)
                }
                QueuedCopyInput::HostRgba8MipChain {
                    extent,
                    mip_levels,
                    pixels,
                } => {
                    if slice != 0 {
                        return Err(error("2D slice must be zero"));
                    }
                    if !allow_mips {
                        return Err(error("mip copies unsupported by legacy entry point"));
                    }
                    let layout = copy_chain_layout(*extent, *mip_levels)?;
                    if pixels.len() != layout.byte_len() {
                        return Err(error("invalid host mip bytes"));
                    }
                    let level = layout
                        .levels()
                        .get(selected)
                        .ok_or_else(|| error("copy mip out of range"))?;
                    (layout.byte_len(), Some([level.width, level.height]), true)
                }
                QueuedCopyInput::HostRgba8 { extent, pixels } => {
                    if slice != 0 || selected != 0 {
                        return Err(error("copy mip out of range"));
                    }
                    let bytes = translated_pixels(*extent, "image copy backing")? * 4;
                    if pixels.len() != bytes {
                        return Err(error("invalid host copy texture length"));
                    }
                    (bytes, Some(*extent), true)
                }
                QueuedCopyInput::ProducedColor(output) => {
                    if !allow_mips && output.mip_levels() != 1 {
                        return Err(error("legacy copy requires one mip"));
                    }
                    if output.is_array() {
                        if !allow_arrays {
                            return Err(error("array copies require explicit entry point"));
                        }
                        let layout = copy_array_layout(
                            output.extent,
                            output.mip_levels(),
                            output.array_layers(),
                        )?;
                        let level = layout
                            .footprint(slice as u32, selected as u32)
                            .map_err(|_| error("array subresource out of bounds"))?;
                        (layout.byte_len(), Some([level.width, level.height]), false)
                    } else {
                        if slice != 0 {
                            return Err(error("2D slice must be zero"));
                        }
                        let layout = copy_chain_layout(output.extent, output.mip_levels())?;
                        let level = layout
                            .levels()
                            .get(selected)
                            .ok_or_else(|| error("copy mip out of range"))?;
                        (layout.byte_len(), Some([level.width, level.height]), false)
                    }
                }
            };
            let padded = match (extent, location) {
                (
                    None,
                    ImageCopyLocation::Buffer {
                        offset,
                        bytes_per_row,
                    },
                ) => {
                    if logical == 0 || logical > 64 * 1024 * 1024 {
                        return Err(error("copy buffer size out of range"));
                    }
                    let row_bytes = copy.extent[0]
                        .checked_mul(4)
                        .ok_or_else(|| error("copy row overflow"))?;
                    let end = (copy.extent[1] - 1)
                        .checked_mul(bytes_per_row)
                        .and_then(|n| n.checked_add(offset))
                        .and_then(|n| n.checked_add(row_bytes));
                    if !offset.is_multiple_of(4)
                        || !bytes_per_row.is_multiple_of(4)
                        || bytes_per_row < row_bytes
                        || bytes_per_row > MAX_RENDER_DIMENSION * 4
                        || end.is_none_or(|end| end > logical)
                    {
                        return Err(error("invalid image copy buffer footprint"));
                    }
                    logical
                        .checked_add(3)
                        .ok_or_else(|| error("copy size overflow"))?
                        & !3
                }
                (Some(extent), ImageCopyLocation::Rgba8 { origin }) => {
                    if origin[0]
                        .checked_add(copy.extent[0])
                        .is_none_or(|end| end > extent[0])
                        || origin[1]
                            .checked_add(copy.extent[1])
                            .is_none_or(|end| end > extent[1])
                    {
                        return Err(error("image copy texture region out of bounds"));
                    }
                    logical
                }
                _ => return Err(error("image copy location does not match backing")),
            };
            let upload = if !host {
                0
            } else if extent.is_some() {
                padded
            } else {
                padded
                    .checked_mul(2)
                    .ok_or_else(|| error("copy reservation overflow"))?
            };
            Ok((padded, upload))
        };
        let (source_bytes, source_upload) =
            validate(&copy.source, source_location, source_level, source_slice)?;
        let (destination_bytes, destination_upload) = validate(
            &copy.destination,
            destination_location,
            destination_level,
            destination_slice,
        )?;
        if let (QueuedCopyInput::ProducedColor(source), QueuedCopyInput::ProducedColor(destination)) =
            (&copy.source, &copy.destination)
            && std::ptr::eq(&*source.texture, &*destination.texture)
        {
            return Err(error("image copy inputs alias"));
        }
        let mut deps: Vec<Rc<Point>> = self.latest[index]
            .iter()
            .map(|point| point.inner.clone())
            .collect();
        let mut add = |point: &RenderPoint| -> Result<(), MetalError> {
            self.validate_point(point)?;
            if !deps.iter().any(|old| Rc::ptr_eq(old, &point.inner)) {
                deps.push(point.inner.clone());
            }
            Ok(())
        };
        for point in dependencies {
            add(point)?;
        }
        let mut direct_leases = Vec::new();
        for (input, bytes) in [
            (&copy.source, source_bytes),
            (&copy.destination, destination_bytes),
        ] {
            match input {
                QueuedCopyInput::ProducedBuffer(output) => {
                    add(&output.point)?;
                    if output.buffer.length() != bytes
                        || output.buffer.device().registryID()
                            != self.queues[index].device.registryID()
                    {
                        return Err(error("invalid produced buffer storage"));
                    }
                    direct_leases.push(output.lease.clone());
                }
                QueuedCopyInput::ProducedColor(output) => {
                    add(&output.point)?;
                    if output.texture.device().registryID()
                        != self.queues[index].device.registryID()
                        || output.texture.pixelFormat() != MTLPixelFormat::RGBA8Unorm
                        || output.texture.width() != output.extent[0]
                        || output.texture.height() != output.extent[1]
                        || output.texture.mipmapLevelCount() != output.mip_levels() as usize
                        || output.texture.sampleCount() != 1
                    {
                        return Err(error("invalid produced color storage"));
                    }
                    direct_leases.push(output._lease.clone());
                }
                _ => {}
            }
        }
        if deps.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        for point in &deps {
            if Self::poll_point(point)? == Some(false) {
                self.generation.poisoned.set(true);
                return Err(error("producer failed"));
            }
        }
        deps.retain(|point| point.terminal.get() != Some(true));
        let value = self.sequence[index]
            .checked_add(1)
            .ok_or_else(|| error("timeline exhausted"))?;
        let scratch = destination_bytes
            .checked_add(source_upload)
            .and_then(|n| n.checked_add(destination_upload))
            .ok_or_else(|| error("copy reservation overflow"))?;
        let lease = ColorLease::reserve(&self.color_bytes, destination_bytes)?;
        let temporary = ColorLease::reserve(&self.color_bytes, scratch)?;
        let gpu = &self.queues[index];
        let upload = |input: QueuedCopyInput<'_>,
                      bytes: usize|
         -> Result<CopyStorage, MetalError> {
            Ok(match input {
                QueuedCopyInput::HostBuffer(data) => {
                    let mut words = vec![0u32; bytes / 4];
                    for (i, byte) in data.iter().enumerate() {
                        words[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
                    }
                    CopyStorage::Buffer {
                        buffer: gpu.new_buffer(&words)?,
                        byte_len: data.len(),
                    }
                }
                QueuedCopyInput::ProducedBuffer(output) => CopyStorage::Buffer {
                    buffer: output.buffer,
                    byte_len: output.byte_len,
                },
                QueuedCopyInput::HostRgba8Array {
                    extent,
                    mip_levels,
                    array_layers,
                    pixels,
                } => {
                    let layout = copy_array_layout(extent, mip_levels, array_layers)?;
                    let texture =
                        copy_storage_texture(gpu, extent, mip_levels, Some(array_layers))?;
                    for slice in 0..array_layers {
                        for level in 0..mip_levels {
                            let mip = layout
                                .footprint(slice, level)
                                .map_err(|_| error("array footprint"))?;
                            // SAFETY: Full checked packed subresource; new storage has no GPU users.
                            unsafe {
                                texture.replaceRegion_mipmapLevel_slice_withBytes_bytesPerRow_bytesPerImage(whole(mip.width,mip.height),level as usize,slice as usize,NonNull::from(&pixels[mip.offset..mip.offset+mip.byte_len]).cast(),if mip.height==1 {0} else {mip.row_bytes},0);
                            }
                        }
                    }
                    CopyStorage::Color { texture, extent }
                }
                QueuedCopyInput::HostRgba8MipChain {
                    extent,
                    mip_levels,
                    pixels,
                } => {
                    let layout = copy_chain_layout(extent, mip_levels)?;
                    let texture = copy_mip_texture(gpu, extent, mip_levels)?;
                    for (i, mip) in layout.levels().iter().enumerate() {
                        // SAFETY: Every packed mip span was prevalidated; fresh storage has no GPU users.
                        unsafe {
                            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                                whole(mip.width, mip.height),
                                i,
                                NonNull::from(&pixels[mip.offset..mip.offset + mip.byte_len])
                                    .cast(),
                                if mip.height == 1 { 0 } else { mip.row_bytes },
                            );
                        }
                    }
                    CopyStorage::Color { texture, extent }
                }
                QueuedCopyInput::HostRgba8 { extent, pixels } => {
                    let texture = copy_texture(gpu, extent)?;
                    // SAFETY: Validated full packed bytes; fresh shared texture has no GPU users.
                    unsafe {
                        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                            whole(extent[0], extent[1]),
                            0,
                            NonNull::from(pixels).cast(),
                            extent[0] * 4,
                        );
                    }
                    CopyStorage::Color { texture, extent }
                }
                QueuedCopyInput::ProducedColor(output) => CopyStorage::Color {
                    texture: output.texture,
                    extent: output.extent,
                },
            })
        };
        let source = upload(copy.source, source_bytes)?;
        let prior = upload(copy.destination, destination_bytes)?;
        let destination = match &prior {
            CopyStorage::Buffer { byte_len, .. } => CopyStorage::Buffer {
                buffer: gpu.new_buffer(&vec![0u32; destination_bytes / 4])?,
                byte_len: *byte_len,
            },
            CopyStorage::Color { extent, texture } => CopyStorage::Color {
                texture: copy_storage_texture(
                    gpu,
                    *extent,
                    texture.mipmapLevelCount() as u32,
                    (texture.textureType() == MTLTextureType::Type2DArray)
                        .then_some(texture.arrayLength() as u32),
                )?,
                extent: *extent,
            },
        };
        let command = gpu
            .queue
            .commandBuffer()
            .ok_or_else(|| error("copy command allocation failed"))?;
        for point in &deps {
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*point.event);
            command.encodeWaitForEvent_value(event, point.value);
        }
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("copy encoder allocation failed"))?;
        let origin = |xy: [usize; 2]| MTLOrigin {
            x: xy[0],
            y: xy[1],
            z: 0,
        };
        let size = MTLSize {
            width: copy.extent[0],
            height: copy.extent[1],
            depth: 1,
        };
        // SAFETY: All layouts, logical row spans, origins and alignments were checked
        // before allocation. Both producers are waited; fresh output cannot alias.
        unsafe {
            match (&prior, &destination) {
                (
                    CopyStorage::Buffer { buffer: old, .. },
                    CopyStorage::Buffer { buffer: dst, .. },
                ) => blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                    old,
                    0,
                    dst,
                    0,
                    destination_bytes,
                ),
                (
                    CopyStorage::Color {
                        texture: old,
                        extent,
                    },
                    CopyStorage::Color { texture: dst, .. },
                ) => {
                    for slice in 0..old.arrayLength() {
                        for level in 0..old.mipmapLevelCount() {
                            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(old,slice,level,origin([0,0]),MTLSize{width:(extent[0]>>level).max(1),height:(extent[1]>>level).max(1),depth:1},dst,slice,level,origin([0,0]));
                        }
                    }
                }
                _ => unreachable!("matching destination layout"),
            }
            match (&source, &destination, source_location, destination_location) {
                (CopyStorage::Color { texture: src, .. }, CopyStorage::Color { texture: dst, .. }, ImageCopyLocation::Rgba8 { origin: src_origin }, ImageCopyLocation::Rgba8 { origin: dst_origin }) =>
                    blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(src, source_slice, source_level, origin(src_origin), size, dst, destination_slice, destination_level, origin(dst_origin)),
                (CopyStorage::Buffer { buffer: src, .. }, CopyStorage::Color { texture: dst, .. }, ImageCopyLocation::Buffer { offset, bytes_per_row }, ImageCopyLocation::Rgba8 { origin: dst_origin }) =>
                    blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(src, offset, bytes_per_row, 0, size, dst, destination_slice, destination_level, origin(dst_origin)),
                (CopyStorage::Color { texture: src, .. }, CopyStorage::Buffer { buffer: dst, .. }, ImageCopyLocation::Rgba8 { origin: src_origin }, ImageCopyLocation::Buffer { offset, bytes_per_row }) =>
                    blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(src, source_slice, source_level, origin(src_origin), size, dst, offset, bytes_per_row, 0),
                _ => unreachable!("validated image endpoints"),
            }
        }
        blit.endEncoding();
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*self.events[index]);
        command.encodeSignalEvent_value(event, value);
        let mut leases = vec![lease.clone(), temporary];
        leases.extend(direct_leases);
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
                depth_lease: None,
            }),
        };
        command.commit();
        self.sequence[index] = value;
        self.latest[index] = Some(point.clone());
        self.entries[slot] = Some(Entry {
            point: point.clone(),
            pending: PendingWork::Copy(PendingCopy {
                destination,
                _source: source,
                _prior: Some(prior),
                lease,
            }),
        });
        Ok(point)
    }

    fn buffer_region_patch_pipeline(
        &mut self,
    ) -> Result<Retained<ProtocolObject<dyn MTLComputePipelineState>>, MetalError> {
        if let Some(pipeline) = &self.buffer_region_pipeline {
            return Ok(pipeline.clone());
        }
        let gpu = &self.queues[0]; // Both queues share this exact device.
        let options = compile_options()?;
        let library = gpu.compile_library(include_str!("buffer_region_patch.metal"), &options)?;
        let function = find_function(&library, "patch_buffer_region")?;
        let pipeline = gpu.compute_pipeline(&function)?;
        verify_shader_validation(&pipeline)?;
        self.buffer_region_pipeline = Some(pipeline.clone());
        Ok(pipeline)
    }

    /// Copy any nonempty logical byte range into a fresh, preserved destination.
    pub fn submit_buffer_region_copy(
        &mut self,
        queue: RenderQueue,
        copy: crate::QueuedBufferRegionCopy<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        const MAX_BYTES: usize = 64 * 1024 * 1024;
        let index = queue.index();
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let slot = self.admission_slot(index)?;
        if dependencies.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        let layout = |input: &QueuedCopyInput<'_>| -> Result<(usize, usize, bool), MetalError> {
            let (len, host) = match input {
                QueuedCopyInput::HostBuffer(bytes) => (bytes.len(), true),
                QueuedCopyInput::ProducedBuffer(output) => (output.byte_len, false),
                _ => return Err(error("buffer region requires buffer inputs")),
            };
            if len == 0 || len > MAX_BYTES {
                return Err(error("copy buffer size out of range"));
            }
            let padded = len
                .checked_add(3)
                .ok_or_else(|| error("copy size overflow"))?
                & !3;
            Ok((len, padded, host))
        };
        let (source_len, source_padded, source_host) = layout(&copy.source)?;
        let (destination_len, destination_padded, destination_host) = layout(&copy.destination)?;
        if copy.byte_len == 0
            || copy
                .source_offset
                .checked_add(copy.byte_len)
                .is_none_or(|end| end > source_len)
            || copy
                .destination_offset
                .checked_add(copy.byte_len)
                .is_none_or(|end| end > destination_len)
        {
            return Err(error("invalid buffer copy region"));
        }
        let aligned = copy.byte_len.is_multiple_of(4)
            && copy.source_offset.is_multiple_of(4)
            && copy.destination_offset.is_multiple_of(4);
        if let (
            QueuedCopyInput::ProducedBuffer(source),
            QueuedCopyInput::ProducedBuffer(destination),
        ) = (&copy.source, &copy.destination)
            && std::ptr::eq(&*source.buffer, &*destination.buffer)
        {
            return Err(error("buffer region inputs alias"));
        }
        let mut deps: Vec<Rc<Point>> = self.latest[index]
            .iter()
            .map(|point| point.inner.clone())
            .collect();
        let mut add = |point: &RenderPoint| -> Result<(), MetalError> {
            self.validate_point(point)?;
            if !deps.iter().any(|old| Rc::ptr_eq(old, &point.inner)) {
                deps.push(point.inner.clone());
            }
            Ok(())
        };
        for dependency in dependencies {
            add(dependency)?;
        }
        let mut source_leases = Vec::new();
        for (input, padded) in [
            (&copy.source, source_padded),
            (&copy.destination, destination_padded),
        ] {
            if let QueuedCopyInput::ProducedBuffer(output) = input {
                add(&output.point)?;
                if output.buffer.length() != padded
                    || output.buffer.device().registryID() != self.queues[index].device.registryID()
                {
                    return Err(error("invalid produced buffer storage"));
                }
                source_leases.push(output.lease.clone());
            }
        }
        if deps.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        for dependency in &deps {
            if Self::poll_point(dependency)? == Some(false) {
                self.generation.poisoned.set(true);
                return Err(error("producer failed"));
            }
        }
        deps.retain(|point| point.terminal.get() != Some(true));
        let value = self.sequence[index]
            .checked_add(1)
            .ok_or_else(|| error("timeline exhausted"))?;
        // Persistent output D; scratch D plus two padded allocations per host input.
        let temporary_bytes = destination_padded
            .checked_add(if source_host {
                source_padded
                    .checked_mul(2)
                    .ok_or_else(|| error("copy reservation overflow"))?
            } else {
                0
            })
            .and_then(|n| {
                n.checked_add(if destination_host {
                    destination_padded * 2
                } else {
                    0
                })
            })
            .and_then(|n| n.checked_add(if aligned { 0 } else { 16 }))
            .ok_or_else(|| error("copy reservation overflow"))?;
        let lease = ColorLease::reserve(&self.color_bytes, destination_padded)?;
        let temporary = ColorLease::reserve(&self.color_bytes, temporary_bytes)?;
        let patch_pipeline = if aligned {
            None
        } else {
            Some(self.buffer_region_patch_pipeline()?)
        };
        let gpu = &self.queues[index];
        let upload =
            |input: QueuedCopyInput<'_>, padded: usize| -> Result<CopyStorage, MetalError> {
                match input {
                    QueuedCopyInput::HostBuffer(bytes) => {
                        let mut words = vec![0u32; padded / 4];
                        for (i, byte) in bytes.iter().enumerate() {
                            words[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
                        }
                        Ok(CopyStorage::Buffer {
                            buffer: gpu.new_buffer(&words)?,
                            byte_len: bytes.len(),
                        })
                    }
                    QueuedCopyInput::ProducedBuffer(output) => Ok(CopyStorage::Buffer {
                        buffer: output.buffer,
                        byte_len: output.byte_len,
                    }),
                    _ => unreachable!("validated buffer inputs"),
                }
            };
        let source = upload(copy.source, source_padded)?;
        let prior = upload(copy.destination, destination_padded)?;
        let destination = CopyStorage::Buffer {
            buffer: gpu.new_buffer(&vec![0u32; destination_padded / 4])?,
            byte_len: destination_len,
        };
        let command = gpu
            .queue
            .commandBuffer()
            .ok_or_else(|| error("copy command allocation failed"))?;
        for point in &deps {
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*point.event);
            command.encodeWaitForEvent_value(event, point.value);
        }
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("copy encoder allocation failed"))?;
        let (
            CopyStorage::Buffer { buffer: src, .. },
            CopyStorage::Buffer { buffer: old, .. },
            CopyStorage::Buffer { buffer: dst, .. },
        ) = (&source, &prior, &destination)
        else {
            unreachable!("validated buffer storage")
        };
        // SAFETY: Fresh destination and initialized padded storage. Both waits
        // precede cloning; only a fully aligned patch is encoded as a blit.
        unsafe {
            blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                old,
                0,
                dst,
                0,
                destination_padded,
            );
            if aligned {
                blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
                    src,
                    copy.source_offset,
                    dst,
                    copy.destination_offset,
                    copy.byte_len,
                );
            }
        }
        blit.endEncoding();
        if let Some(pipeline) = patch_pipeline {
            let end = copy.destination_offset + copy.byte_len; // checked logical range
            let words = end.div_ceil(4) - copy.destination_offset / 4;
            let parameters = [
                copy.source_offset as u32,
                copy.destination_offset as u32,
                copy.byte_len as u32,
                words as u32,
            ]; // All <=64MiB.
            let width = gpu.pipeline_group_width_limit(&pipeline).min(64);
            if width == 0 {
                return Err(error("buffer patch threadgroup unavailable"));
            }
            let encoder = command
                .computeCommandEncoder()
                .ok_or_else(|| error("buffer patch encoder allocation failed"))?;
            encoder.setComputePipelineState(&pipeline);
            // SAFETY: Same-device tracked buffers are retained through completion;
            // the prior blit ended before compute. setBytes copies all16parameter
            // bytes now. Each guarded invocation updates one allocated u32 word.
            unsafe {
                encoder.setBuffer_offset_atIndex(Some(src), 0, 0);
                encoder.setBuffer_offset_atIndex(Some(dst), 0, 1);
                encoder.setBytes_length_atIndex(
                    NonNull::from(&parameters).cast::<c_void>(),
                    size_of_val(&parameters),
                    2,
                );
            }
            encoder.dispatchThreadgroups_threadsPerThreadgroup(
                size(words.div_ceil(width)),
                size(width),
            );
            encoder.endEncoding();
        }
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*self.events[index]);
        command.encodeSignalEvent_value(event, value);
        let mut leases = vec![lease.clone(), temporary];
        leases.extend(source_leases);
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
                depth_lease: None,
            }),
        };
        command.commit();
        self.sequence[index] = value;
        self.latest[index] = Some(point.clone());
        self.entries[slot] = Some(Entry {
            point: point.clone(),
            pending: PendingWork::Copy(PendingCopy {
                destination,
                _source: source,
                _prior: Some(prior),
                lease,
            }),
        });
        Ok(point)
    }

    /// Submit a bounded whole-resource GPU copy. Buffer logical lengths need not
    /// be aligned: native storage is padded to four bytes with initialized zeros.
    pub fn submit_copy(
        &mut self,
        queue: RenderQueue,
        input: QueuedCopyInput<'_>,
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        const MAX_COPY_BUFFER_BYTES: usize = 64 * 1024 * 1024;
        let index = queue.index();
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let slot = self.admission_slot(index)?;
        if dependencies.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        let (logical, padded, host, color) = match &input {
            QueuedCopyInput::HostRgba8Array { .. } | QueuedCopyInput::HostRgba8MipChain { .. } => {
                return Err(error("whole-copy API requires single mip"));
            }
            QueuedCopyInput::HostBuffer(bytes) => (
                bytes.len(),
                bytes.len().checked_add(3).map(|n| n & !3),
                true,
                false,
            ),
            QueuedCopyInput::ProducedBuffer(output) => (
                output.byte_len,
                output.byte_len.checked_add(3).map(|n| n & !3),
                false,
                false,
            ),
            QueuedCopyInput::HostRgba8 { extent, pixels } => {
                let bytes = translated_pixels(*extent, "copy texture")? * 4;
                if pixels.len() != bytes {
                    return Err(error("invalid host copy texture length"));
                }
                (bytes, Some(bytes), true, true)
            }
            QueuedCopyInput::ProducedColor(output) => {
                if output.is_array() {
                    return Err(error("whole-copy array unsupported"));
                }
                let bytes = translated_pixels(output.extent, "copy texture")? * 4;
                (bytes, Some(bytes), false, true)
            }
        };
        let padded = padded.ok_or_else(|| error("copy size overflow"))?;
        if logical == 0 || (!color && logical > MAX_COPY_BUFFER_BYTES) {
            return Err(error("copy buffer size out of range"));
        }
        let mut deps: Vec<Rc<Point>> = self.latest[index]
            .iter()
            .map(|point| point.inner.clone())
            .collect();
        let mut add = |point: &RenderPoint| -> Result<(), MetalError> {
            self.validate_point(point)?;
            if !deps.iter().any(|old| Rc::ptr_eq(old, &point.inner)) {
                deps.push(point.inner.clone());
            }
            Ok(())
        };
        for dependency in dependencies {
            add(dependency)?;
        }
        let source_lease = match &input {
            QueuedCopyInput::ProducedBuffer(output) => {
                add(&output.point)?;
                if output.buffer.length() != padded
                    || output.buffer.device().registryID() != self.queues[index].device.registryID()
                {
                    return Err(error("invalid produced buffer storage"));
                }
                Some(output.lease.clone())
            }
            QueuedCopyInput::ProducedColor(output) => {
                add(&output.point)?;
                if output.texture.device().registryID() != self.queues[index].device.registryID()
                    || output.texture.pixelFormat() != MTLPixelFormat::RGBA8Unorm
                    || output.texture.width() != output.extent[0]
                    || output.texture.height() != output.extent[1]
                    || output.texture.mipmapLevelCount() != 1
                {
                    return Err(error("invalid produced color storage"));
                }
                Some(output._lease.clone())
            }
            _ => None,
        };
        if deps.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        for dependency in &deps {
            if Self::poll_point(dependency)? == Some(false) {
                self.generation.poisoned.set(true);
                return Err(error("producer failed"));
            }
        }
        deps.retain(|point| point.terminal.get() != Some(true));
        let value = self.sequence[index]
            .checked_add(1)
            .ok_or_else(|| error("timeline exhausted"))?;
        let lease = ColorLease::reserve(&self.color_bytes, padded)?;
        // Buffer: host GPU source + CPU padded allocation + logical readback.
        // Produced buffers need no upload; reserve CPU destination init/readback.
        // Textures use shared source/destination and packed readback, no staging.
        let temporary_bytes = if !color && host {
            padded.checked_mul(3)
        } else if color && host {
            padded.checked_mul(2)
        } else {
            Some(padded)
        }
        .ok_or_else(|| error("copy reservation overflow"))?;
        let temporary = ColorLease::reserve(&self.color_bytes, temporary_bytes)?;
        let gpu = &self.queues[index];
        let source = match input {
            QueuedCopyInput::HostRgba8Array { .. } | QueuedCopyInput::HostRgba8MipChain { .. } => {
                return Err(error("whole-copy API requires single mip"));
            }
            QueuedCopyInput::HostBuffer(bytes) => {
                let mut words = vec![0u32; padded / 4];
                for (i, byte) in bytes.iter().enumerate() {
                    words[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
                }
                CopyStorage::Buffer {
                    buffer: gpu.new_buffer(&words)?,
                    byte_len: logical,
                }
            }
            QueuedCopyInput::ProducedBuffer(output) => CopyStorage::Buffer {
                buffer: output.buffer,
                byte_len: logical,
            },
            QueuedCopyInput::HostRgba8 { extent, pixels } => {
                let texture = copy_texture(gpu, extent)?;
                // SAFETY: Validated exact packed RGBA8 length; fresh shared texture
                // has no GPU users. Metal copies the borrowed initialized bytes.
                unsafe {
                    texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        whole(extent[0], extent[1]),
                        0,
                        NonNull::from(pixels).cast(),
                        extent[0] * 4,
                    );
                }
                CopyStorage::Color { texture, extent }
            }
            QueuedCopyInput::ProducedColor(output) => CopyStorage::Color {
                texture: output.texture,
                extent: output.extent,
            },
        };
        let destination = match &source {
            CopyStorage::Buffer { .. } => CopyStorage::Buffer {
                buffer: gpu.new_buffer(&vec![0u32; padded / 4])?,
                byte_len: logical,
            },
            CopyStorage::Color { extent, .. } => CopyStorage::Color {
                texture: copy_texture(gpu, *extent)?,
                extent: *extent,
            },
        };
        let command = gpu
            .queue
            .commandBuffer()
            .ok_or_else(|| error("copy command allocation failed"))?;
        for point in &deps {
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*point.event);
            command.encodeWaitForEvent_value(event, point.value);
        }
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| error("copy encoder allocation failed"))?;
        // SAFETY: Identical checked layouts, distinct fresh destination, and live
        // retained resources. Buffer sizes/offsets satisfy macOS four-byte rules;
        // padding is initialized. Produced sources are waited before these copies.
        unsafe {
            match (&source, &destination) {
                (CopyStorage::Buffer { buffer: src, .. }, CopyStorage::Buffer { buffer: dst, .. }) =>
                    blit.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(src, 0, dst, 0, padded),
                (CopyStorage::Color { texture: src, extent }, CopyStorage::Color { texture: dst, .. }) =>
                    blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                        src, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 }, MTLSize { width: extent[0], height: extent[1], depth: 1 },
                        dst, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 }),
                _ => unreachable!("matching copy storage"),
            }
        }
        blit.endEncoding();
        let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*self.events[index]);
        command.encodeSignalEvent_value(event, value);
        let mut leases = vec![lease.clone(), temporary];
        leases.extend(source_lease);
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
                depth_lease: None,
            }),
        };
        command.commit();
        self.sequence[index] = value;
        self.latest[index] = Some(point.clone());
        self.entries[slot] = Some(Entry {
            point: point.clone(),
            pending: PendingWork::Copy(PendingCopy {
                destination,
                _source: source,
                _prior: None,
                lease,
            }),
        });
        Ok(point)
    }

    pub fn copy_output(&self, point: &RenderPoint) -> Result<RenderCopyOutput, MetalError> {
        if self.is_typed_copy(point)? {
            return self.typed_copy_output(point);
        }
        self.validate_point(point)?;
        let entry = self.entries[self.entry_slot(point)?]
            .as_ref()
            .expect("validated entry");
        let PendingWork::Copy(copy) = &entry.pending else {
            return Err(error("submission is not a copy"));
        };
        Ok(match &copy.destination {
            CopyStorage::Buffer { buffer, byte_len } => {
                RenderCopyOutput::Buffer(RenderBufferOutput {
                    point: point.clone(),
                    byte_len: *byte_len,
                    buffer: buffer.clone(),
                    lease: copy.lease.clone(),
                })
            }
            CopyStorage::Color { texture, extent } => RenderCopyOutput::Color(RenderColorOutput {
                point: point.clone(),
                texture: texture.clone(),
                extent: *extent,
                _lease: copy.lease.clone(),
            }),
        })
    }

    pub fn collect_copy(&mut self, point: &RenderPoint) -> Result<OwnedCopyOutput, MetalError> {
        if self.is_typed_copy(point)? {
            return self.collect_typed_copy(point);
        }
        self.validate_point(point)?;
        let slot = self.entry_slot(point)?;
        let entry = self.entries[slot].as_ref().expect("validated entry");
        if !matches!(entry.pending, PendingWork::Copy(_)) {
            return Err(error("submission is not a copy"));
        }
        Self::drain_point(&point.inner);
        if !self.is_complete(point)? {
            return Err(error("command not complete"));
        }
        let entry = self.take_entry(slot);
        let PendingWork::Copy(copy) = entry.pending else {
            unreachable!("validated copy")
        };
        let output = match copy.destination {
            CopyStorage::Buffer { buffer, byte_len } => {
                let mut bytes = vec![0u8; byte_len];
                // SAFETY: Ordered completion precedes reading shared storage;
                // logical length is bounded by the initialized padded allocation.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        buffer.contents().cast::<u8>().as_ptr(),
                        bytes.as_mut_ptr(),
                        byte_len,
                    );
                }
                OwnedCopyOutput::Buffer(bytes)
            }
            CopyStorage::Color { texture, extent } => {
                let mip_levels = texture.mipmapLevelCount() as u32;
                let array_layers = texture.arrayLength() as u32;
                let is_array = texture.textureType() == MTLTextureType::Type2DArray;
                let layout = copy_array_layout(extent, mip_levels, array_layers)?;
                let mut pixels = vec![0u8; layout.byte_len()];
                for slice in 0..array_layers {
                    for level in 0..mip_levels {
                        let mip = layout
                            .footprint(slice, level)
                            .map_err(|_| error("array readback footprint"))?;
                        // SAFETY: Completed writes and exact checked destination subresource span.
                        unsafe {
                            texture
                                .getBytes_bytesPerRow_bytesPerImage_fromRegion_mipmapLevel_slice(
                                    NonNull::from(
                                        &mut pixels[mip.offset..mip.offset + mip.byte_len],
                                    )
                                    .cast(),
                                    mip.row_bytes,
                                    0,
                                    whole(mip.width, mip.height),
                                    level as usize,
                                    slice as usize,
                                );
                        }
                    }
                }
                if is_array {
                    OwnedCopyOutput::Rgba8Array {
                        extent,
                        mip_levels,
                        array_layers,
                        pixels,
                    }
                } else if mip_levels == 1 {
                    OwnedCopyOutput::Rgba8 { extent, pixels }
                } else {
                    OwnedCopyOutput::Rgba8MipChain {
                        extent,
                        mip_levels,
                        pixels,
                    }
                }
            }
        };
        self.release_collected_latest(point);
        Ok(output)
    }

    pub fn color_output(
        &self,
        point: &RenderPoint,
        slot: usize,
    ) -> Result<RenderColorOutput, MetalError> {
        self.validate_point(point)?;
        let entry = self.entries[self.entry_slot(point)?]
            .as_ref()
            .expect("validated entry");
        let PendingWork::Render(pending) = &entry.pending else {
            return Err(error("submission is not a render"));
        };
        let texture = pending
            .textures
            .get(slot)
            .ok_or_else(|| error("color slot out of range"))?
            .clone();
        if pending.color_formats.get(slot) != Some(&TextureFormat::Rgba8Unorm) {
            return Err(error("legacy color output requires RGBA8"));
        }
        Ok(RenderColorOutput {
            point: point.clone(),
            texture,
            extent: pending.extent,
            _lease: pending
                ._color_lease
                .as_ref()
                .expect("cohort charge")
                .clone(),
        })
    }
    pub fn typed_color_output(
        &self,
        point: &RenderPoint,
        slot: usize,
    ) -> Result<RenderTypedTextureOutput, MetalError> {
        self.validate_point(point)?;
        let PendingWork::Render(pending) = &self.entries[self.entry_slot(point)?]
            .as_ref()
            .expect("entry")
            .pending
        else {
            return Err(error("not a render"));
        };
        let texture = pending
            .textures
            .get(slot)
            .ok_or_else(|| error("color slot out of range"))?
            .clone();
        let format = *pending
            .color_formats
            .get(slot)
            .ok_or_else(|| error("color format missing"))?;
        Ok(RenderTypedTextureOutput {
            point: point.clone(),
            texture,
            desc: crate::TypedTextureDesc {
                format,
                extent: pending.extent,
                mip_levels: 1,
                array_layers: None,
            },
            lease: pending
                ._color_lease
                .as_ref()
                .expect("cohort charge")
                .clone(),
        })
    }
    /// Acquire the exact pending owned depth version before collecting its receipt.
    pub fn depth_output(&self, point: &RenderPoint) -> Result<RenderDepthOutput, MetalError> {
        self.validate_point(point)?;
        let entry = self.entries[self.entry_slot(point)?]
            .as_ref()
            .expect("validated entry");
        let PendingWork::Render(pending) = &entry.pending else {
            return Err(error("submission is not a render"));
        };
        if !pending._export_depth {
            return Err(error("submission has no owned depth output"));
        }
        Ok(RenderDepthOutput {
            point: point.clone(),
            texture: pending
                .working_depth
                .as_ref()
                .ok_or_else(|| error("missing depth output"))?
                .clone(),
            extent: pending.extent,
        })
    }

    /// All produced-input waits are derived from opaque provenance. No CPU wait,
    /// render readback, or produced-texture upload occurs on the submission path.
    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<OwnedDepthInput>,
        color_loads: &[Option<&[u8]>],
        dependencies: &[RenderPoint],
    ) -> Result<RenderPoint, MetalError> {
        let loads: Vec<_> = color_loads
            .iter()
            .map(|load| match load {
                Some(bytes) => QueuedColorLoad::Host(bytes),
                None => QueuedColorLoad::Clear,
            })
            .collect();
        self.submit_internal(
            queue,
            shaders,
            pass,
            bindings,
            depth.map(DepthInput::Legacy),
            &loads,
            dependencies,
            None,
            None,
            None,
        )
    }

    /// Submit owned per-draw uniforms without changing dependency or depth ownership.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_uniforms(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<OwnedDepthInput>,
        color_loads: &[Option<&[u8]>],
        dependencies: &[RenderPoint],
        uniforms: &[DrawUniforms],
    ) -> Result<RenderPoint, MetalError> {
        let loads: Vec<_> = color_loads
            .iter()
            .map(|load| match load {
                Some(bytes) => QueuedColorLoad::Host(bytes),
                None => QueuedColorLoad::Clear,
            })
            .collect();
        self.submit_internal(
            queue,
            shaders,
            pass,
            bindings,
            depth.map(DepthInput::Legacy),
            &loads,
            dependencies,
            Some(uniforms),
            None,
            None,
        )
    }

    /// GPU-copy immutable produced versions after their waits, then render Load.
    /// All destinations are fresh. Owned payload reservations are limited to 256 MiB per cohort,
    /// including external output/point clones across generation resets. This is not
    /// an actual VRAM limit: driver/pipeline overhead and caller-owned uploads are
    /// excluded. Returned host Vec storage leaves accounting upon ownership transfer.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_color_loads(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<OwnedDepthInput>,
        color_loads: &[QueuedColorLoad<'_>],
        dependencies: &[RenderPoint],
        uniforms: &[DrawUniforms],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_internal(
            queue,
            shaders,
            pass,
            bindings,
            depth.map(DepthInput::Legacy),
            color_loads,
            dependencies,
            Some(uniforms),
            None,
            None,
        )
    }

    /// Submit exact produced color/depth versions into fresh attachments.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_attachments(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<QueuedDepthInput>,
        color_loads: &[QueuedColorLoad<'_>],
        dependencies: &[RenderPoint],
        uniforms: &[DrawUniforms],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_internal(
            queue,
            shaders,
            pass,
            bindings,
            depth.map(DepthInput::Queued),
            color_loads,
            dependencies,
            Some(uniforms),
            None,
            None,
        )
    }

    /// Submit GPU-resident geometry through guarded gather, retaining exact sources.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_geometry(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<QueuedDepthInput>,
        color_loads: &[QueuedColorLoad<'_>],
        dependencies: &[RenderPoint],
        uniforms: &[DrawUniforms],
        geometry: &[Option<crate::QueuedGeometry<'_>>],
    ) -> Result<RenderPoint, MetalError> {
        self.submit_internal(
            queue,
            shaders,
            pass,
            bindings,
            depth.map(DepthInput::Queued),
            color_loads,
            dependencies,
            Some(uniforms),
            Some(geometry),
            None,
        )
    }

    /// Typed attachments retain the same queue graph, geometry and failure contract.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_typed_attachments(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TypedTriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<QueuedDepthInput>,
        color_loads: &[QueuedColorLoad<'_>],
        dependencies: &[RenderPoint],
        uniforms: &[DrawUniforms],
        geometry: &[Option<crate::QueuedGeometry<'_>>],
    ) -> Result<RenderPoint, MetalError> {
        let clears = vec![[0; 4]; pass.colors.len()];
        let legacy = crate::TriangleTargetsPass {
            draws: pass.draws,
            states: pass.states,
            extent: pass.extent,
            clears: &clears,
            clear_depth: pass.clear_depth,
        };
        self.submit_internal(
            queue,
            shaders,
            &legacy,
            bindings,
            depth.map(DepthInput::Queued),
            color_loads,
            dependencies,
            Some(uniforms),
            Some(geometry),
            Some(pass.colors),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_internal(
        &mut self,
        queue: RenderQueue,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<QueuedSampledBinding>],
        depth: Option<DepthInput>,
        color_loads: &[QueuedColorLoad<'_>],
        dependencies: &[RenderPoint],
        uniforms: Option<&[DrawUniforms]>,
        geometry: Option<&[Option<crate::QueuedGeometry<'_>>]>,
        typed_colors: Option<&[crate::TypedColorAttachment]>,
    ) -> Result<RenderPoint, MetalError> {
        validate_uniforms(pass.draws.len(), uniforms)?;
        if geometry.is_some_and(|g| {
            g.len() != pass.draws.len()
                || g.iter()
                    .zip(pass.draws)
                    .any(|(g, d)| g.is_some() && (!d.vertices.is_empty() || !d.colors.is_empty()))
        }) {
            return Err(error("ambiguous geometry descriptors"));
        }
        let index = queue.index();
        if self.generation.poisoned.get() {
            return Err(error("cohort poisoned"));
        }
        let slot = self.admission_slot(index)?;
        if pass.draws.len() > MAX_RENDER_DRAWS
            || dependencies.len() > self.entries.len()
            || bindings.len() != pass.draws.len()
            || bindings.iter().any(|b| b.len() > 4)
        {
            return Err(error("invalid dependency or binding count"));
        }
        if depth.is_some() && pass.clear_depth.is_some() {
            return Err(error("ambiguous depth clear"));
        }
        let pixels = translated_pixels(pass.extent, "color load")?;
        if !(1..=8).contains(&pass.clears.len()) || color_loads.len() != pass.clears.len() {
            return Err(error("invalid color load count"));
        }
        if let Some(colors) = typed_colors {
            if colors.len() != pass.clears.len() {
                return Err(error("typed color count"));
            }
            for color in colors {
                color.validate()?;
            }
        }
        let formats: Vec<_> = (0..pass.clears.len())
            .map(|i| typed_colors.map_or(TextureFormat::Rgba8Unorm, |c| c[i].format))
            .collect();
        let mut host_loads = Vec::with_capacity(color_loads.len());
        let mut color_sources = Vec::with_capacity(color_loads.len());
        for (slot, load) in color_loads.iter().enumerate() {
            match load {
                QueuedColorLoad::Clear => {
                    host_loads.push(None);
                    color_sources.push(None);
                }
                QueuedColorLoad::Host(bytes) => {
                    if bytes.len() != pixels * formats[slot].block_bytes() {
                        return Err(error("color load byte length mismatch"));
                    }
                    host_loads.push(Some(*bytes));
                    color_sources.push(None);
                }
                QueuedColorLoad::ProducedTypedView(view) => {
                    if typed_colors.is_none() {
                        return Err(error("typed view Load requires typed pass"));
                    }
                    self.validate_point(&view.output.point)?;
                    let texture = view.view.retained_texture();
                    let format = formats[slot];
                    if view.output.desc.format != format
                        || view.desc().mip_count != 1
                        || view.extent() != pass.extent
                        || texture.textureType() != MTLTextureType::Type2D
                        || texture.mipmapLevelCount() != 1
                        || texture.width() != pass.extent[0]
                        || texture.height() != pass.extent[1]
                        || texture.device().registryID() != self.queues[index].device.registryID()
                        || view.output.texture.device().registryID()
                            != self.queues[index].device.registryID()
                        || texture.pixelFormat()
                            != crate::texture_pipeline::storage_pixel_format(format)
                        || matches!(
                            format,
                            TextureFormat::Bc1RgbaUnorm
                                | TextureFormat::Bc3RgbaUnorm
                                | TextureFormat::Depth32Float
                        )
                    {
                        return Err(error("typed view Load backing mismatch"));
                    }
                    host_loads.push(None);
                    // The selected native view rebases its mip/slice to zero, but
                    // dependency identity and accounting remain the full root's.
                    color_sources.push(Some(RenderTypedTextureOutput {
                        point: view.output.point.clone(),
                        texture,
                        desc: crate::TypedTextureDesc {
                            format,
                            extent: pass.extent,
                            mip_levels: 1,
                            array_layers: None,
                        },
                        lease: view.output.lease.clone(),
                    }));
                }
                QueuedColorLoad::ProducedTyped(output) => {
                    if typed_colors.is_none() {
                        return Err(error("typed Load requires typed pass"));
                    }
                    self.validate_point(&output.point)?;
                    if output.desc
                        != (crate::TypedTextureDesc {
                            format: formats[slot],
                            extent: pass.extent,
                            mip_levels: 1,
                            array_layers: None,
                        })
                        || output.texture.device().registryID()
                            != self.queues[index].device.registryID()
                        || output.texture.pixelFormat()
                            != crate::texture_pipeline::storage_pixel_format(formats[slot])
                    {
                        return Err(error("typed Load backing mismatch"));
                    }
                    host_loads.push(None);
                    color_sources.push(Some(output.clone()));
                }
                QueuedColorLoad::Produced(output) => {
                    self.validate_point(&output.point)?;
                    if formats[slot] != TextureFormat::Rgba8Unorm
                        || output.extent != pass.extent
                        || output.is_array()
                        || output.mip_levels() != 1
                        || output.texture.device().registryID()
                            != self.queues[index].device.registryID()
                        || output.texture.pixelFormat() != PIXEL_FORMAT
                    {
                        return Err(error("produced color device/format/extent mismatch"));
                    }
                    host_loads.push(None);
                    color_sources.push(Some(RenderTypedTextureOutput {
                        point: output.point.clone(),
                        texture: output.texture.clone(),
                        desc: crate::TypedTextureDesc {
                            format: TextureFormat::Rgba8Unorm,
                            extent: output.extent,
                            mip_levels: 1,
                            array_layers: None,
                        },
                        lease: output._lease.clone(),
                    }));
                }
            }
        }
        // Selected views keep the full root's charge, including typed-copy roots
        // whose point does not carry an ordinary render-depth lease.
        let mut depth_view_lease = None;
        let depth = match depth {
            Some(DepthInput::Queued(QueuedDepthInput::ProducedTypedView(view))) => {
                self.validate_point(&view.output.point)?;
                let texture = view.view.retained_texture();
                if view.output.desc.format != TextureFormat::Depth32Float
                    || view.desc().format != crate::TextureViewFormat::Depth32Float
                    || view.desc().mip_count != 1
                    || view.extent() != pass.extent
                    || texture.textureType() != MTLTextureType::Type2D
                    || texture.mipmapLevelCount() != 1
                    || texture.width() != pass.extent[0]
                    || texture.height() != pass.extent[1]
                    || texture.pixelFormat() != DEPTH_FORMAT
                    || texture.device().registryID() != self.queues[index].device.registryID()
                    || view.output.texture.device().registryID()
                        != self.queues[index].device.registryID()
                {
                    return Err(error("typed depth view Load backing mismatch"));
                }
                depth_view_lease = Some(view.output.lease.clone());
                Some(DepthInput::Queued(QueuedDepthInput::Produced(
                    RenderDepthOutput {
                        point: view.output.point.clone(),
                        texture,
                        extent: pass.extent,
                    },
                )))
            }
            other => other,
        };
        let export_depth = depth.is_some();
        let depth_source_owner = match &depth {
            Some(DepthInput::Queued(QueuedDepthInput::Produced(output))) => {
                self.validate_point(&output.point)?;
                if output.extent != pass.extent
                    || output.texture.device().registryID()
                        != self.queues[index].device.registryID()
                    || output.texture.pixelFormat() != DEPTH_FORMAT
                {
                    return Err(error("produced depth device/format/extent mismatch"));
                }
                Some(output.clone())
            }
            _ => None,
        };
        let geometry_inputs = geometry.unwrap_or(&[]);
        let aggregate = pass
            .draws
            .iter()
            .enumerate()
            .try_fold(0usize, |sum, (i, draw)| {
                sum.checked_add(
                    geometry_inputs
                        .get(i)
                        .and_then(Option::as_ref)
                        .map_or(draw.vertices.len(), |g| g.count as usize),
                )
                .filter(|n| *n <= MAX_RENDER_VERTICES)
            })
            .ok_or_else(|| error("aggregate vertex limit exceeded"))?;
        let _ = aggregate;
        let (geometry, validation, geometry_sources) =
            self.prepare_geometry(index, geometry_inputs)?;
        let bytes = formats
            .iter()
            .try_fold(0usize, |total, f| {
                pixels
                    .checked_mul(f.block_bytes())
                    .and_then(|n| total.checked_add(n))
            })
            .ok_or_else(|| error("typed color bytes overflow"))?;
        let color_lease = ColorLease::reserve(&self.color_bytes, bytes)?;
        let vertices = pass
            .draws
            .iter()
            .try_fold(0usize, |total, draw| {
                total
                    .checked_add(draw.vertices.len())
                    .filter(|n| *n <= MAX_RENDER_VERTICES)
            })
            .ok_or_else(|| error("aggregate vertex limit exceeded"))?;
        // CPU flattening plus GPU positions/colors; uniforms have CPU+GPU copies.
        // Color readback is one packed Vec per attachment. Host Load is borrowed
        // and copied directly into the already-charged destination, with no staging.
        let mut temporary = vertices.max(1) * 64 + pass.draws.len() * 64 + bytes;
        if depth.is_some() || pass.clear_depth.is_some() {
            let padded = (pass.extent[0] * 4).div_ceil(256) * 256 * pass.extent[1];
            // Owned+working depth, input/output packed snapshots, CPU/GPU upload
            // and CPU/GPU readback. Reserve all even when a particular path omits one.
            temporary += pixels * 16 + padded * 4;
        }
        // Working owned depth is persistent; do not hide it in scratch accounting.
        let depth_lease = if export_depth {
            temporary -= pixels * 4;
            Some(ColorLease::reserve(&self.color_bytes, pixels * 4)?)
        } else {
            None
        };
        let temporary_lease = ColorLease::reserve(&self.color_bytes, temporary)?;
        let value = self.sequence[index]
            .checked_add(1)
            .ok_or_else(|| error("timeline exhausted"))?;
        let mut deps: Vec<Rc<Point>> = self.latest[index]
            .iter()
            .map(|point| point.inner.clone())
            .collect();
        let mut add = |p: &RenderPoint| -> Result<(), MetalError> {
            self.validate_point(p)?;
            if !deps.iter().any(|d| Rc::ptr_eq(d, &p.inner)) {
                deps.push(p.inner.clone());
            }
            Ok(())
        };
        if let Some(source) = &depth_source_owner {
            add(&source.point)?;
        }
        for source in color_sources.iter().flatten() {
            add(&source.point)?;
        }
        for source in &geometry_sources {
            add(&source.point)?;
        }
        for dependency in dependencies {
            add(dependency)?;
        }
        let mut normalized = Vec::with_capacity(bindings.len());
        for draw in bindings {
            let mut pairs = Vec::with_capacity(draw.len());
            for binding in draw {
                pairs.push(match binding {
                    QueuedSampledBinding::ArrayViewed {
                        view,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => view.bind(sampler.clone(), *texture_slot, *sampler_slot)?,
                    QueuedSampledBinding::ProducedTypedArrayView {
                        view,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => {
                        add(&view.output.point)?;
                        view.view
                            .bind(sampler.clone(), *texture_slot, *sampler_slot)?
                    }
                    QueuedSampledBinding::Uploaded(pair) => NativeSampledBinding::from_whole(pair),
                    QueuedSampledBinding::Viewed {
                        view,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => view.bind(sampler.clone(), *texture_slot, *sampler_slot)?,
                    QueuedSampledBinding::ProducedTypedView {
                        view,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => {
                        add(&view.output.point)?;
                        view.view
                            .bind(sampler.clone(), *texture_slot, *sampler_slot)?
                    }
                    QueuedSampledBinding::ProducedView {
                        view,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => {
                        add(&view.output.point)?;
                        view.view
                            .bind(sampler.clone(), *texture_slot, *sampler_slot)?
                    }
                    QueuedSampledBinding::Produced {
                        output,
                        sampler,
                        texture_slot,
                        sampler_slot,
                    } => {
                        if output.is_array() {
                            return Err(error("implicit array sampling unsupported"));
                        }
                        add(&output.point)?;
                        let texture = crate::MetalRgba8Texture::from_render_output(
                            output.texture.clone(),
                            output.extent[0],
                            output.extent[1],
                        )?;
                        NativeSampledBinding::from_whole(&crate::BoundSampledTexture::bind(
                            texture,
                            sampler.clone(),
                            *texture_slot,
                            *sampler_slot,
                        )?)
                    }
                });
            }
            normalized.push(pairs);
        }
        // Bound total explicit, derived and implicit queue provenance before pruning.
        if deps.len() > self.entries.len() {
            return Err(error("too many producer dependencies"));
        }
        for dependency in &deps {
            if Self::poll_point(dependency)? == Some(false) {
                self.generation.poisoned.set(true);
                return Err(error("producer failed"));
            }
        }
        // Command buffers retain their own resources and direct source textures,
        // not the source's command buffer. Carry direct color leases only; copying
        // all ancestor reservations would accumulate historical scratch forever.
        let mut leases = vec![color_lease.clone(), temporary_lease.clone()];
        // This command retains its own packed buffers/uploads after collection.
        // Keep their charge here, without inheriting producer scratch leases.
        leases.extend(
            geometry
                .iter()
                .flatten()
                .map(|gather| gather.allocation_lease()),
        );
        if let Some(lease) = &depth_lease {
            leases.push(lease.clone());
        }
        if let Some(source) = &depth_source_owner {
            leases.push(
                depth_view_lease
                    .as_ref()
                    .or(source.point.inner.depth_lease.as_ref())
                    .expect("validated depth source lease")
                    .clone(),
            );
        }
        for source in &geometry_sources {
            if !leases.iter().any(|old| Rc::ptr_eq(old, &source.lease)) {
                leases.push(source.lease.clone());
            }
        }
        let mut retain_color = |output: &RenderColorOutput| {
            if !leases.iter().any(|old| Rc::ptr_eq(old, &output._lease)) {
                leases.push(output._lease.clone());
            }
        };
        for binding in bindings.iter().flatten() {
            match binding {
                QueuedSampledBinding::Produced { output, .. } => retain_color(output),
                QueuedSampledBinding::ProducedView { view, .. } => retain_color(&view.output),
                _ => {}
            }
        }
        for source in color_sources.iter().flatten() {
            if !leases.iter().any(|old| Rc::ptr_eq(old, &source.lease)) {
                leases.push(source.lease.clone());
            }
        }
        for binding in bindings.iter().flatten() {
            let lease = match binding {
                QueuedSampledBinding::ProducedTypedView { view, .. } => Some(&view.output.lease),
                QueuedSampledBinding::ProducedTypedArrayView { view, .. } => {
                    Some(&view.output.lease)
                }
                _ => None,
            };
            if let Some(lease) = lease
                && !leases.iter().any(|old| Rc::ptr_eq(old, lease))
            {
                leases.push(lease.clone());
            }
        }
        // Completed successful ancestors need no stored edge: the generation
        // retains cancellation provenance, and this bounds the live DAG to two nodes.
        deps.retain(|p| p.terminal.get() != Some(true));
        let gpu = &self.queues[index];
        let mut upload = None;
        let depth = match depth {
            Some(DepthInput::Queued(QueuedDepthInput::Host(snapshot))) => {
                Some(DepthInput::Legacy(OwnedDepthInput {
                    snapshot: Some(snapshot),
                    load: DepthLoad::Load,
                }))
            }
            other => other,
        };
        let mut depth_clear = None;
        let owned_depth = match depth {
            Some(DepthInput::Queued(QueuedDepthInput::Clear(value))) => {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(error("invalid depth clear"));
                }
                depth_clear = Some(value);
                None
            }
            Some(DepthInput::Queued(QueuedDepthInput::Produced(output))) => {
                // Borrow immutable producer storage as the copy source, allocating
                // no import texture. Event wait precedes the direct working-depth blit.
                Some((
                    MetalDepthTarget {
                        texture: output.texture,
                        extent: output.extent,
                        device_id: gpu.device.registryID(),
                        initialized: true,
                    },
                    DepthLoad::Load,
                ))
            }
            Some(DepthInput::Queued(
                QueuedDepthInput::Host(_) | QueuedDepthInput::ProducedTypedView(_),
            )) => {
                unreachable!("normalized queued depth")
            }
            None => None,
            Some(DepthInput::Legacy(input)) => {
                let target = match input.snapshot {
                    Some(snapshot) => {
                        let pixels = translated_pixels(snapshot.extent, "depth snapshot")?;
                        if snapshot.extent != pass.extent || snapshot.words.len() != pixels {
                            return Err(error("depth snapshot extent/storage mismatch"));
                        }
                        let mut target = gpu.create_depth_target(snapshot.extent)?;
                        if input.load == DepthLoad::Load {
                            let [width, height] = snapshot.extent;
                            let stride = (width * 4).div_ceil(256) * 64;
                            let mut words = vec![0; stride * height];
                            for (row, source) in words
                                .chunks_exact_mut(stride)
                                .zip(snapshot.words.chunks_exact(width))
                            {
                                row[..width].copy_from_slice(source);
                            }
                            upload = Some(gpu.new_buffer(&words)?);
                            target.initialized = true; // The same submitted command uploads before reading.
                        }
                        target
                    }
                    None if input.load == DepthLoad::Load => {
                        return Err(error("depth Load requires snapshot"));
                    }
                    None => gpu.create_depth_target(pass.extent)?,
                };
                Some((target, input.load))
            }
        };
        let sync = SubmissionSync {
            geometry,
            validation: validation.clone(),
            depth: owned_depth,
            depth_clear,
            export_depth,
            depth_source_owner,
            depth_lease: depth_lease.clone(),
            upload,
            color_sources,
            color_lease: color_lease.clone(),
            temporary_lease: temporary_lease.clone(),
            waits: deps.clone(),
            event: self.events[index].clone(),
            value,
        };
        let pending = gpu.submit_targets_typed_internal(
            "",
            ["", ""],
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            None,
            None,
            Some(&host_loads),
            Some((shaders, &normalized)),
            Some(sync),
            uniforms,
            typed_colors,
        )?;
        let point = RenderPoint {
            inner: Rc::new(Point {
                generation: self.generation.clone(),
                queue: index,
                value,
                command: pending.command.clone(),
                event: self.events[index].clone(),
                dependencies: RefCell::new(deps),
                terminal: Cell::new(None),
                validation,
                _leases: leases,
                depth_lease,
            }),
        };
        self.sequence[index] = value;
        self.latest[index] = Some(point.clone());
        self.entries[slot] = Some(Entry {
            point: point.clone(),
            pending: PendingWork::Render(Box::new(pending)),
        });
        Ok(point)
    }
    // None means pending; Some(false) means an actual command or ancestor failed.
    fn poll_point(point: &Point) -> Result<Option<bool>, MetalError> {
        Self::poll_point_memo(point, &mut HashMap::new())
    }
    fn poll_point_memo(
        point: &Point,
        memo: &mut HashMap<*const Point, Option<bool>>,
    ) -> Result<Option<bool>, MetalError> {
        let key = std::ptr::from_ref(point);
        if let Some(result) = memo.get(&key) {
            return Ok(*result);
        }
        if let Some(success) = point.terminal.get() {
            memo.insert(key, Some(success));
            return Ok(Some(success));
        }
        let mut pending = false;
        let parents = point.dependencies.borrow().clone();
        for parent in &parents {
            match Self::poll_point_memo(parent, memo)? {
                Some(false) => {
                    memo.insert(key, Some(false));
                    return Ok(Some(false));
                }
                None => pending = true,
                Some(true) => {}
            }
        }
        let result = match point.command.status() {
            MTLCommandBufferStatus::Error => Some(false),
            MTLCommandBufferStatus::Completed if !pending => {
                Some(point.validation.as_ref().is_none_or(|v| v.succeeded()))
            }
            _ => None,
        };
        if result == Some(true) {
            point.terminal.set(result);
            point.dependencies.borrow_mut().clear();
        }
        // Failed/pending writers are memoized only for this traversal: their
        // ancestry must remain intact until producer-first draining is complete.
        memo.insert(key, result);
        Ok(result)
    }
    pub fn is_complete(&mut self, point: &RenderPoint) -> Result<bool, MetalError> {
        self.validate_point(point)?;
        match Self::poll_point(&point.inner)? {
            Some(false) => {
                self.generation.poisoned.set(true);
                Err(error("producer or command failed"))
            }
            Some(true) => Ok(true),
            None => Ok(false),
        }
    }
    // The producer is terminal before a missing GPU signal is supplied by the
    // host. Never release a consumer while its source could still be written.
    fn drain_point(point: &Point) {
        drain_ordered(point);
    }

    pub fn collect(&mut self, point: &RenderPoint) -> Result<OwnedRenderOutput, MetalError> {
        self.validate_point(point)?;
        let index = point.inner.queue;
        let slot = self.entry_slot(point)?;
        if !matches!(
            self.entries[slot].as_ref().unwrap().pending,
            PendingWork::Render(_)
        ) {
            return Err(error("submission is not a render"));
        }
        Self::drain_point(&point.inner);
        if !self.is_complete(point)? {
            return Err(error("command not complete"));
        }
        let entry = self.take_entry(slot);
        let PendingWork::Render(pending) = entry.pending else {
            unreachable!("validated render")
        };
        let result = (|| {
            let depth = if let Some(texture) = pending
                .working_depth
                .as_ref()
                .filter(|_| pending._export_depth)
            {
                let target = MetalDepthTarget {
                    texture: texture.clone(),
                    extent: pending.extent,
                    device_id: self.queues[index].device.registryID(),
                    initialized: true,
                };
                Some(self.queues[index].snapshot_depth(&target)?)
            } else {
                None
            };
            let color_layouts = pending.color_layouts();
            let colors = (*pending).wait()?;
            Ok(OwnedRenderOutput {
                colors,
                color_layouts,
                depth,
            })
        })();
        if result.is_err() {
            self.generation.poisoned.set(true);
        } else {
            self.release_collected_latest(point);
        }
        result
    }
    /// Invalidate publication immediately, then drain producers before consumers.
    pub fn abort_and_drain(&mut self, _reason: &str) -> Result<(), MetalError> {
        self.generation.poisoned.set(true);
        for entry in self.entries.iter().flatten() {
            Self::drain_point(&entry.point.inner);
        }
        for entry in &mut self.entries {
            *entry = None;
        }
        self.latest = [None, None];
        Ok(())
    }
    pub fn reinitialize(&mut self) -> Result<(), MetalError> {
        if self.entries.iter().any(Option::is_some) {
            return Err(error("drain/collect before reinitialize"));
        }
        let events = [event(&self.queues[0])?, event(&self.queues[1])?];
        self.events = events;
        self.sequence = [0; 2];
        self.latest = [None, None];
        self.generation = Rc::new(Generation {
            poisoned: Cell::new(false),
        });
        Ok(())
    }
}
impl Drop for RenderCohort {
    fn drop(&mut self) {
        let _ = self.abort_and_drain("cohort dropped");
    }
}

// Both Metal and the deterministic failure driver use this traversal. Keeping
// event advancement here (rather than in the driver) makes its ordering testable.
trait DrainNode: Sized {
    fn terminal(&self) -> &Cell<Option<bool>>;
    fn dependencies(&self) -> &RefCell<Vec<Rc<Self>>>;
    /// Block until this writer is terminal; return command success, not event progress.
    fn wait_terminal(&self) -> bool;
    fn signal_value(&self) -> u64;
    fn target_value(&self) -> u64;
    fn advance_signal(&self, value: u64);
}
fn drain_ordered<N: DrainNode>(point: &N) {
    // Terminal outcomes are the traversal memo: even a failed shared ancestor is
    // visited once. Edges only target prior committed points, so cycles cannot form.
    if point.terminal().get().is_some() {
        return;
    }
    let parents = point.dependencies().borrow().clone();
    for parent in &parents {
        drain_ordered(&**parent);
    }
    let command_succeeded = point.wait_terminal();
    // No host signal may precede this writer's terminal wait, even on cancel.
    if point.signal_value() < point.target_value() {
        point.advance_signal(point.target_value());
    }
    let success = command_succeeded
        && parents
            .iter()
            .all(|parent| parent.terminal().get() == Some(true));
    point.terminal().set(Some(success));
    point.dependencies().borrow_mut().clear();
}
impl DrainNode for Point {
    fn terminal(&self) -> &Cell<Option<bool>> {
        &self.terminal
    }
    fn dependencies(&self) -> &RefCell<Vec<Rc<Self>>> {
        &self.dependencies
    }
    fn wait_terminal(&self) -> bool {
        self.command.waitUntilCompleted();
        self.command.status() == MTLCommandBufferStatus::Completed
            && self.validation.as_ref().is_none_or(|v| v.succeeded())
    }
    fn signal_value(&self) -> u64 {
        self.event.signaledValue()
    }
    fn target_value(&self) -> u64 {
        self.value
    }
    fn advance_signal(&self, value: u64) {
        self.event.setSignaledValue(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Scripted terminal results and missing GPU signals, never a real GPU fault.
    struct ScriptedPoint {
        name: &'static str,
        terminal: Cell<Option<bool>>,
        dependencies: RefCell<Vec<Rc<Self>>>,
        writer_terminal: Cell<bool>,
        succeeds: bool,
        gpu_signals: bool,
        signal: Rc<Cell<u64>>,
        target: u64,
        log: Rc<RefCell<Vec<String>>>,
    }
    impl ScriptedPoint {
        fn new(
            name: &'static str,
            parents: Vec<Rc<Self>>,
            succeeds: bool,
            gpu_signals: bool,
            log: &Rc<RefCell<Vec<String>>>,
        ) -> Rc<Self> {
            Rc::new(Self {
                name,
                terminal: Cell::new(None),
                dependencies: RefCell::new(parents),
                writer_terminal: Cell::new(false),
                succeeds,
                gpu_signals,
                signal: Rc::new(Cell::new(0)),
                target: 7,
                log: log.clone(),
            })
        }
        fn record(&self, action: &str) {
            self.log
                .borrow_mut()
                .push(format!("{}:{action}", self.name));
        }
    }
    impl DrainNode for ScriptedPoint {
        fn terminal(&self) -> &Cell<Option<bool>> {
            &self.terminal
        }
        fn dependencies(&self) -> &RefCell<Vec<Rc<Self>>> {
            &self.dependencies
        }
        fn wait_terminal(&self) -> bool {
            for parent in self.dependencies.borrow().iter() {
                assert!(
                    parent.writer_terminal.get(),
                    "consumer waited before producer terminal"
                );
                assert!(
                    parent.signal.get() >= parent.target,
                    "consumer would remain blocked by missing producer signal"
                );
                assert!(
                    parent.terminal.get().is_some(),
                    "producer outcome must be cached first"
                );
            }
            self.record("wait");
            self.writer_terminal.set(true);
            self.record(if self.succeeds { "completed" } else { "error" });
            if self.gpu_signals {
                self.signal.set(self.target);
            }
            self.succeeds
        }
        fn signal_value(&self) -> u64 {
            self.signal.get()
        }
        fn target_value(&self) -> u64 {
            self.target
        }
        fn advance_signal(&self, value: u64) {
            assert!(
                self.writer_terminal.get(),
                "host signaled while writer still pending"
            );
            assert!(
                value > self.signal.get(),
                "host signal must strictly increase"
            );
            self.record("host_signal");
            self.signal.set(value);
        }
    }
    #[test]
    fn graph_same_queue_failure_drains_earlier_writer_before_advancing_timeline() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let earlier = ScriptedPoint::new("earlier", vec![], false, false, &log);
        let mut later = ScriptedPoint::new("later", vec![earlier.clone()], true, false, &log);
        let node = Rc::get_mut(&mut later).unwrap();
        node.signal = earlier.signal.clone();
        node.target = earlier.target + 1;
        // The edge models the implicit same-queue predecessor, even when no
        // explicit resource dependency was supplied. Shared event values are real
        // timeline state in this deterministic driver, not completion evidence.
        drain_ordered(&*later);
        assert_eq!(earlier.terminal.get(), Some(false));
        assert_eq!(later.terminal.get(), Some(false));
        assert_eq!(earlier.signal.get(), 8);
        assert_eq!(
            *log.borrow(),
            [
                "earlier:wait",
                "earlier:error",
                "earlier:host_signal",
                "later:wait",
                "later:completed",
                "later:host_signal"
            ]
        );
    }

    #[test]
    fn graph_cached_success_predecessor_does_not_mask_later_same_queue_failure() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let first = ScriptedPoint::new("first", vec![], true, true, &log);
        drain_ordered(&*first);
        let mut failed = ScriptedPoint::new("failed", vec![first.clone()], false, false, &log);
        let node = Rc::get_mut(&mut failed).unwrap();
        node.signal = first.signal.clone();
        node.target = 8;
        let mut dependent = ScriptedPoint::new("dependent", vec![failed.clone()], true, true, &log);
        let node = Rc::get_mut(&mut dependent).unwrap();
        node.signal = first.signal.clone();
        node.target = 9;
        drain_ordered(&*dependent);
        assert_eq!(first.terminal.get(), Some(true));
        assert_eq!(failed.terminal.get(), Some(false));
        assert_eq!(dependent.terminal.get(), Some(false));
        assert_eq!(first.signal.get(), 9, "event progress is not success");
        assert_eq!(
            *log.borrow(),
            [
                "first:wait",
                "first:completed",
                "failed:wait",
                "failed:error",
                "failed:host_signal",
                "dependent:wait",
                "dependent:completed"
            ]
        );
    }

    #[test]
    fn graph_diamond_drain_memoizes_shared_success_and_failure() {
        for succeeds in [false, true] {
            let log = Rc::new(RefCell::new(Vec::new()));
            let root = ScriptedPoint::new("root", vec![], succeeds, succeeds, &log);
            let left = ScriptedPoint::new("left", vec![root.clone()], true, true, &log);
            let right = ScriptedPoint::new("right", vec![root.clone()], true, true, &log);
            let join =
                ScriptedPoint::new("join", vec![left.clone(), right.clone()], true, true, &log);
            drain_ordered(&*join);
            for node in [&root, &left, &right, &join] {
                assert_eq!(node.terminal.get(), Some(succeeds));
                assert_eq!(
                    log.borrow()
                        .iter()
                        .filter(|event| **event == format!("{}:wait", node.name))
                        .count(),
                    1
                );
                assert!(node.dependencies.borrow().is_empty());
            }
            let count = log.borrow().len();
            drain_ordered(&*join);
            assert_eq!(log.borrow().len(), count);
        }
    }

    #[test]
    fn missing_signal_drains_failed_producer_before_successful_consumer() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let producer = ScriptedPoint::new("producer", vec![], false, false, &log);
        let consumer = ScriptedPoint::new("consumer", vec![producer.clone()], true, true, &log);
        assert!(!producer.writer_terminal.get());
        drain_ordered(&*consumer); // Reverse collection order.
        assert_eq!(
            *log.borrow(),
            [
                "producer:wait",
                "producer:error",
                "producer:host_signal",
                "consumer:wait",
                "consumer:completed"
            ]
        );
        assert_eq!(producer.terminal.get(), Some(false));
        assert_eq!(consumer.terminal.get(), Some(false));
        assert!(consumer.dependencies.borrow().is_empty());
        let count = log.borrow().len();
        drain_ordered(&*producer);
        drain_ordered(&*consumer);
        assert_eq!(
            log.borrow().len(),
            count,
            "cached failure must not rerun or become success"
        );
    }
    #[test]
    fn successful_gpu_signal_and_cached_parent_need_no_host_signal() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let producer = ScriptedPoint::new("producer", vec![], true, true, &log);
        let consumer = ScriptedPoint::new("consumer", vec![producer.clone()], true, true, &log);
        drain_ordered(&*producer); // Producer collected first.
        drain_ordered(&*consumer);
        assert_eq!(
            *log.borrow(),
            [
                "producer:wait",
                "producer:completed",
                "consumer:wait",
                "consumer:completed"
            ]
        );
        assert_eq!(consumer.terminal.get(), Some(true));
        assert!(consumer.dependencies.borrow().is_empty());
        assert_eq!(
            Rc::strong_count(&producer),
            1,
            "resolved consumer must release ancestor"
        );
    }
    #[test]
    fn higher_event_progress_does_not_convert_failed_command_to_success() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let producer = ScriptedPoint::new("producer", vec![], false, false, &log);
        producer.signal.set(99);
        drain_ordered(&*producer);
        assert_eq!(producer.terminal.get(), Some(false));
        assert_eq!(*log.borrow(), ["producer:wait", "producer:error"]);
        assert_eq!(producer.signal.get(), 99);
    }
}

#[cfg(test)]
mod color_budget_tests {
    use super::*;

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_payload_budget_follows_points_outputs_views_and_consumer_commands() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let clears = [[17, 83, 149, 255]];
        let pass = crate::TriangleTargetsPass {
            draws: &[],
            states: &[],
            extent: [2, 2],
            clears: &clears,
            clear_depth: None,
        };
        let producer = cohort
            .submit(RenderQueue::First, &[], &pass, &[], None, &[None], &[])
            .unwrap();
        let output = cohort.color_output(&producer, 0).unwrap();
        let output_clone = output.clone();
        let view = output
            .view(crate::TextureViewDesc {
                base_mip: 0,
                mip_count: 1,
                format: crate::TextureViewFormat::Rgba8Unorm,
            })
            .unwrap();
        let producer_charge = budget.get();
        assert!(
            producer_charge > 16,
            "texture plus submission/readback reservations"
        );
        assert_eq!(
            cohort.collect(&producer).unwrap().colors,
            vec![[17, 83, 149, 255].repeat(4)]
        );
        assert_eq!(
            budget.get(),
            producer_charge,
            "retained point still owns its command"
        );
        drop(producer);
        drop(output);
        assert_eq!(
            budget.get(),
            producer_charge,
            "output/view retain command-owning provenance"
        );
        let consumer = cohort
            .submit_with_color_loads(
                RenderQueue::Second,
                &[],
                &pass,
                &[],
                None,
                &[QueuedColorLoad::Produced(output_clone.clone())],
                &[],
                &[],
            )
            .unwrap();
        let both_charge = budget.get();
        assert!(both_charge > producer_charge);
        assert_eq!(
            cohort.collect(&consumer).unwrap().colors,
            vec![[17, 83, 149, 255].repeat(4)]
        );
        assert_eq!(budget.get(), both_charge);
        cohort.reinitialize().unwrap();
        assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
        assert_eq!(
            budget.get(),
            both_charge,
            "reset cannot forgive live owners"
        );
        // Saturate accounting without allocating 256 MiB. A real native owner
        // remains live, and a real submission must reject/refund atomically.
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - both_charge).unwrap();
        assert!(
            cohort
                .submit(RenderQueue::First, &[], &pass, &[], None, &[None], &[])
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        drop(filler);
        assert_eq!(budget.get(), both_charge);
        drop(output_clone);
        assert_eq!(
            budget.get(),
            both_charge,
            "immutable view is still an owner"
        );
        drop(view);
        let consumer_charge = budget.get();
        assert!(
            consumer_charge < both_charge,
            "producer scratch released with final producer point"
        );
        assert!(
            consumer_charge > producer_charge,
            "consumer command still pins original source texture"
        );
        drop(consumer);
        assert_eq!(
            budget.get(),
            0,
            "final command owner refunds all source/destination payloads"
        );
        let recovery = cohort
            .submit(RenderQueue::First, &[], &pass, &[], None, &[None], &[])
            .unwrap();
        cohort.collect(&recovery).unwrap();
        drop(recovery);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_depth_version_budget_survives_collection_reset_and_direct_consumer() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let pass = crate::TriangleTargetsPass {
            draws: &[],
            states: &[],
            extent: [2, 2],
            clears: &[[7, 11, 13, 255]],
            clear_depth: None,
        };
        let producer = cohort
            .submit_with_attachments(
                RenderQueue::First,
                &[],
                &pass,
                &[],
                Some(QueuedDepthInput::Clear(0.25)),
                &[QueuedColorLoad::Clear],
                &[],
                &[],
            )
            .unwrap();
        let output = cohort.depth_output(&producer).unwrap();
        let clone = output.clone();
        let first_charge = budget.get();
        assert!(first_charge > 16);
        cohort.collect(&producer).unwrap();
        drop(producer);
        assert_eq!(budget.get(), first_charge);
        let consumer = cohort
            .submit_with_attachments(
                RenderQueue::Second,
                &[],
                &pass,
                &[],
                Some(QueuedDepthInput::Produced(output.clone())),
                &[QueuedColorLoad::Clear],
                &[],
                &[],
            )
            .unwrap();
        let consumer_output = cohort.depth_output(&consumer).unwrap();
        let own_consumer_charge = budget.get() - first_charge;
        assert_eq!(
            own_consumer_charge, first_charge,
            "same extent and conservative scratch budget"
        );
        let snapshot = cohort.collect(&consumer).unwrap().depth.unwrap();
        assert!(
            consumer.inner.dependencies.borrow().is_empty(),
            "successful collection releases ancestor edges before lease checks"
        );
        assert_eq!(snapshot.words, vec![0.25f32.to_bits(); 4]);
        drop(output);
        assert_eq!(budget.get(), first_charge + own_consumer_charge);
        drop(clone);
        assert_eq!(
            budget.get(),
            own_consumer_charge + 16,
            "consumer retains direct source depth only, not historical scratch"
        );
        cohort.reinitialize().unwrap();
        assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
        let held = budget.get();
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - held).unwrap();
        assert!(
            cohort
                .submit_with_attachments(
                    RenderQueue::First,
                    &[],
                    &pass,
                    &[],
                    Some(QueuedDepthInput::Clear(1.)),
                    &[QueuedColorLoad::Clear],
                    &[],
                    &[]
                )
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        drop(filler);
        assert_eq!(budget.get(), held);
        drop(consumer);
        assert_eq!(
            budget.get(),
            held,
            "depth output still owns exact version and command"
        );
        drop(consumer_output);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_copy_budget_retains_direct_source_after_ancestor_pruning_and_refunds_clones() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        // Five logical bytes occupy eight native bytes, including initialized padding.
        let literal = [17, 83, 149, 231, 9];
        let producer = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&literal),
                &[],
            )
            .unwrap();
        let source = match cohort.copy_output(&producer).unwrap() {
            RenderCopyOutput::Buffer(output) => output,
            _ => panic!("expected buffer output"),
        };
        let historical = source.clone();
        assert_eq!(source.byte_len(), 5);
        assert_eq!(source.buffer.length(), 8);
        assert_eq!(
            budget.get(),
            32,
            "eight-byte output plus padded host scratch/readback"
        );
        let consumer = cohort
            .submit_copy(
                RenderQueue::Second,
                QueuedCopyInput::ProducedBuffer(source.clone()),
                &[],
            )
            .unwrap();
        let output = match cohort.copy_output(&consumer).unwrap() {
            RenderCopyOutput::Buffer(output) => output,
            _ => panic!("expected buffer output"),
        };
        let output_clone = output.clone();
        assert_eq!(
            budget.get(),
            48,
            "source lease is shared, not charged twice"
        );
        for point in [&consumer, &producer] {
            match cohort.collect_copy(point).unwrap() {
                OwnedCopyOutput::Buffer(bytes) => assert_eq!(bytes, literal),
                _ => panic!("expected buffer bytes"),
            }
        }
        assert!(consumer.inner.dependencies.borrow().is_empty());
        assert!(producer.inner.dependencies.borrow().is_empty());
        // Inspect the real completed allocation, including its non-public tail.
        // SAFETY: Both commands completed; the owned shared buffer is eight bytes.
        let native = unsafe {
            std::slice::from_raw_parts(output.buffer.contents().cast::<u8>().as_ptr(), 8)
        };
        assert_eq!(native, &[17, 83, 149, 231, 9, 0, 0, 0]);
        drop(producer);
        drop(source);
        assert_eq!(
            budget.get(),
            48,
            "historical clone still owns producer scratch"
        );
        drop(historical);
        assert_eq!(
            budget.get(),
            24,
            "consumer retains direct eight-byte source lease after ancestor pruning"
        );
        cohort.reinitialize().unwrap();
        assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
        assert_eq!(
            budget.get(),
            24,
            "generation reset cannot forgive historical native owners"
        );
        let sequence = cohort.sequence;
        assert!(
            cohort
                .submit_copy(
                    RenderQueue::First,
                    QueuedCopyInput::ProducedBuffer(output.clone()),
                    &[]
                )
                .is_err()
        );
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_none));
        assert_eq!(budget.get(), 24);
        // Leave room for destination reservation only. Scratch reservation must
        // fail and refund that partial charge before consuming slot or timeline.
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 24 - 8).unwrap();
        assert!(
            cohort
                .submit_copy(
                    RenderQueue::First,
                    QueuedCopyInput::HostBuffer(&literal),
                    &[]
                )
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 8);
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_none));
        drop(filler);
        assert_eq!(budget.get(), 24);
        drop(consumer);
        drop(output);
        assert_eq!(
            budget.get(),
            24,
            "final output clone retains destination and direct source"
        );
        drop(output_clone);
        assert_eq!(budget.get(), 0);
        let recovered = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&literal),
                &[],
            )
            .unwrap();
        assert_eq!(
            recovered.inner.value, 1,
            "rejected submissions consumed no timeline value"
        );
        match cohort.collect_copy(&recovered).unwrap() {
            OwnedCopyOutput::Buffer(bytes) => assert_eq!(bytes, literal),
            _ => panic!("expected buffer bytes"),
        }
        drop(recovered);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_region_budget_retains_both_direct_inputs_after_pruning() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let source_point = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[17, 83, 149, 231, 9]),
                &[],
            )
            .unwrap();
        let destination_point = cohort
            .submit_copy(
                RenderQueue::Second,
                QueuedCopyInput::HostBuffer(&[1, 2, 3, 4, 5, 6, 7, 8, 99]),
                &[],
            )
            .unwrap();
        let RenderCopyOutput::Buffer(source) = cohort.copy_output(&source_point).unwrap() else {
            panic!("buffer")
        };
        let RenderCopyOutput::Buffer(destination) = cohort.copy_output(&destination_point).unwrap()
        else {
            panic!("buffer")
        };
        assert_eq!(budget.get(), 80); // Whole-copy charges: 4*8 + 4*12.
        let sequence = cohort.sequence;
        let request = || crate::QueuedBufferRegionCopy {
            source: QueuedCopyInput::ProducedBuffer(source.clone()),
            destination: QueuedCopyInput::ProducedBuffer(destination.clone()),
            source_offset: 0,
            destination_offset: 4,
            byte_len: 4,
        };
        assert!(
            cohort
                .submit_buffer_region_copy(RenderQueue::First, request(), &[])
                .is_err()
        );
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_some));
        assert_eq!(
            budget.get(),
            80,
            "capacity rejection neither collects nor reserves"
        );
        cohort.collect_copy(&source_point).unwrap();
        cohort.collect_copy(&destination_point).unwrap();
        let consumer = cohort
            .submit_buffer_region_copy(RenderQueue::First, request(), &[])
            .unwrap();
        let RenderCopyOutput::Buffer(output) = cohort.copy_output(&consumer).unwrap() else {
            panic!("buffer")
        };
        let clone = output.clone();
        assert_eq!(
            budget.get(),
            104,
            "produced inputs add only 2*12 destination bytes"
        );
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&consumer).unwrap() else {
            panic!("buffer")
        };
        assert_eq!(bytes, [1, 2, 3, 4, 17, 83, 149, 231, 99]);
        assert!(consumer.inner.dependencies.borrow().is_empty());
        // SAFETY: Completed shared allocation contains twelve initialized bytes.
        let native = unsafe {
            std::slice::from_raw_parts(output.buffer.contents().cast::<u8>().as_ptr(), 12)
        };
        assert_eq!(native, &[1, 2, 3, 4, 17, 83, 149, 231, 99, 0, 0, 0]);
        drop(source_point);
        drop(destination_point);
        drop(source);
        drop(destination);
        assert_eq!(
            budget.get(),
            44,
            "24 consumer + 8 source + 12 prior destination"
        );
        cohort.reinitialize().unwrap();
        assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
        assert_eq!(budget.get(), 44);
        drop(consumer);
        drop(output);
        assert_eq!(
            budget.get(),
            44,
            "historical output clone retains both direct leases"
        );
        drop(clone);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_region_host_budget_partial_reservation_rollback_and_refund() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let source = [17, 83, 149, 231, 9];
        let destination = [1, 2, 3, 4, 5, 6, 7, 8, 99];
        let request = || crate::QueuedBufferRegionCopy {
            source: QueuedCopyInput::HostBuffer(&source),
            destination: QueuedCopyInput::HostBuffer(&destination),
            source_offset: 0,
            destination_offset: 4,
            byte_len: 4,
        };
        // Permit the twelve-byte output reservation, but not host/scratch storage.
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 12).unwrap();
        let sequence = cohort.sequence;
        assert!(
            cohort
                .submit_buffer_region_copy(RenderQueue::First, request(), &[])
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 12);
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_none));
        drop(filler);
        assert_eq!(budget.get(), 0);
        let point = cohort
            .submit_buffer_region_copy(RenderQueue::First, request(), &[])
            .unwrap();
        assert_eq!(point.inner.value, 1);
        assert_eq!(budget.get(), 64, "2D + 2S + 2D = 24 + 16 + 24");
        let RenderCopyOutput::Buffer(output) = cohort.copy_output(&point).unwrap() else {
            panic!("buffer")
        };
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&point).unwrap() else {
            panic!("buffer")
        };
        assert_eq!(bytes, [1, 2, 3, 4, 17, 83, 149, 231, 99]);
        assert_eq!(output.byte_len(), 9);
        assert_eq!(output.buffer.length(), 12);
        // SAFETY: Collection completed all writes to the owned shared allocation.
        let native = unsafe {
            std::slice::from_raw_parts(output.buffer.contents().cast::<u8>().as_ptr(), 12)
        };
        assert_eq!(native, &[1, 2, 3, 4, 17, 83, 149, 231, 99, 0, 0, 0]);
        drop(point);
        assert_eq!(budget.get(), 64);
        drop(output);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_image_budget_retains_buffer_and_color_leases_after_pruning() {
        use crate::{ImageCopyLocation, QueuedImageCopy};
        for to_buffer in [false, true] {
            let gpu = NativeCompute::new().unwrap();
            let mut cohort = gpu.new_render_cohort().unwrap();
            let budget = cohort.color_bytes.clone();
            let buffer_point = cohort
                .submit_copy(
                    RenderQueue::First,
                    QueuedCopyInput::HostBuffer(&[1, 2, 3, 4, 5, 6, 7, 8, 99]),
                    &[],
                )
                .unwrap();
            let color_point = cohort
                .submit_copy(
                    RenderQueue::Second,
                    QueuedCopyInput::HostRgba8 {
                        extent: [2, 1],
                        pixels: &[17, 83, 149, 231, 31, 47, 61, 79],
                    },
                    &[],
                )
                .unwrap();
            let RenderCopyOutput::Buffer(buffer) = cohort.copy_output(&buffer_point).unwrap()
            else {
                panic!("buffer")
            };
            let RenderCopyOutput::Color(color) = cohort.copy_output(&color_point).unwrap() else {
                panic!("color")
            };
            assert_eq!(budget.get(), 72); // 4*12 buffer + 3*8 color.
            cohort.collect_copy(&buffer_point).unwrap();
            cohort.collect_copy(&color_point).unwrap();
            let buffer_location = ImageCopyLocation::Buffer {
                offset: 4,
                bytes_per_row: 4,
            };
            let color_location = ImageCopyLocation::Rgba8 { origin: [1, 0] };
            let (source, destination, source_location, destination_location) = if to_buffer {
                (
                    QueuedCopyInput::ProducedColor(color.clone()),
                    QueuedCopyInput::ProducedBuffer(buffer.clone()),
                    color_location,
                    buffer_location,
                )
            } else {
                (
                    QueuedCopyInput::ProducedBuffer(buffer.clone()),
                    QueuedCopyInput::ProducedColor(color.clone()),
                    buffer_location,
                    color_location,
                )
            };
            let consumer = cohort
                .submit_image_copy(
                    RenderQueue::First,
                    QueuedImageCopy {
                        source,
                        destination,
                        source_location,
                        destination_location,
                        extent: [1, 1],
                    },
                    &[],
                )
                .unwrap();
            let output = cohort.copy_output(&consumer).unwrap();
            assert_eq!(budget.get(), if to_buffer { 96 } else { 88 });
            match cohort.collect_copy(&consumer).unwrap() {
                OwnedCopyOutput::Buffer(bytes) => {
                    assert!(to_buffer);
                    assert_eq!(bytes, [1, 2, 3, 4, 31, 47, 61, 79, 99]);
                }
                OwnedCopyOutput::TypedTexture { .. } => {
                    panic!("legacy image copy returned typed texture")
                }
                OwnedCopyOutput::Rgba8Array { .. } | OwnedCopyOutput::Rgba8MipChain { .. } => {
                    panic!("legacy copy returned mip chain")
                }
                OwnedCopyOutput::Rgba8 { extent, pixels } => {
                    assert!(!to_buffer);
                    assert_eq!(extent, [2, 1]);
                    assert_eq!(pixels, [17, 83, 149, 231, 5, 6, 7, 8]);
                }
            }
            assert!(consumer.inner.dependencies.borrow().is_empty());
            drop(buffer_point);
            drop(color_point);
            drop(buffer);
            drop(color);
            assert_eq!(
                budget.get(),
                if to_buffer { 44 } else { 36 },
                "consumer plus direct buffer12 and color8 only"
            );
            cohort.reinitialize().unwrap();
            assert!(Rc::ptr_eq(&budget, &cohort.color_bytes));
            drop(consumer);
            assert_eq!(budget.get(), if to_buffer { 44 } else { 36 });
            drop(output);
            assert_eq!(budget.get(), 0);
        }
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_image_host_charges_partial_rollback_and_full_output_refund() {
        use crate::{ImageCopyLocation, QueuedImageCopy};
        for direction in 0..3 {
            let gpu = NativeCompute::new().unwrap();
            let mut cohort = gpu.new_render_cohort().unwrap();
            let budget = cohort.color_bytes.clone();
            let buffer = [1, 2, 3, 4, 5, 6, 7, 8, 99];
            let color = [17, 83, 149, 231, 31, 47, 61, 79];
            let prior_color = [101, 102, 103, 104, 105, 106, 107, 108];
            let make = || {
                let b = ImageCopyLocation::Buffer {
                    offset: 4,
                    bytes_per_row: 4,
                };
                let t = ImageCopyLocation::Rgba8 { origin: [1, 0] };
                let (source, destination, source_location, destination_location) = match direction {
                    0 => (
                        QueuedCopyInput::HostBuffer(&buffer),
                        QueuedCopyInput::HostRgba8 {
                            extent: [2, 1],
                            pixels: &prior_color,
                        },
                        b,
                        t,
                    ),
                    1 => (
                        QueuedCopyInput::HostRgba8 {
                            extent: [2, 1],
                            pixels: &color,
                        },
                        QueuedCopyInput::HostBuffer(&buffer),
                        t,
                        b,
                    ),
                    _ => (
                        QueuedCopyInput::HostRgba8 {
                            extent: [2, 1],
                            pixels: &color,
                        },
                        QueuedCopyInput::HostRgba8 {
                            extent: [2, 1],
                            pixels: &prior_color,
                        },
                        t,
                        t,
                    ),
                };
                QueuedImageCopy {
                    source,
                    destination,
                    source_location,
                    destination_location,
                    extent: [1, 1],
                }
            };
            let destination_bytes = if direction == 1 { 12 } else { 8 };
            let filler =
                ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - destination_bytes).unwrap();
            let sequence = cohort.sequence;
            assert!(
                cohort
                    .submit_image_copy(RenderQueue::First, make(), &[])
                    .is_err()
            );
            assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - destination_bytes);
            assert_eq!(cohort.sequence, sequence);
            assert!(cohort.entries.iter().all(Option::is_none));
            drop(filler);
            assert_eq!(budget.get(), 0);
            let point = cohort
                .submit_image_copy(RenderQueue::First, make(), &[])
                .unwrap();
            assert_eq!(point.inner.value, 1);
            let charge = [48, 56, 32][direction];
            assert_eq!(budget.get(), charge);
            let output = cohort.copy_output(&point).unwrap();
            match cohort.collect_copy(&point).unwrap() {
                OwnedCopyOutput::Buffer(bytes) => {
                    assert_eq!(direction, 1);
                    assert_eq!(bytes, [1, 2, 3, 4, 31, 47, 61, 79, 99]);
                    let RenderCopyOutput::Buffer(native) = &output else {
                        panic!("buffer")
                    };
                    // SAFETY: Completed owned shared buffer has twelve initialized bytes.
                    let padded = unsafe {
                        std::slice::from_raw_parts(
                            native.buffer.contents().cast::<u8>().as_ptr(),
                            12,
                        )
                    };
                    assert_eq!(padded, &[1, 2, 3, 4, 31, 47, 61, 79, 99, 0, 0, 0]);
                }
                OwnedCopyOutput::TypedTexture { .. } => {
                    panic!("legacy image copy returned typed texture")
                }
                OwnedCopyOutput::Rgba8Array { .. } | OwnedCopyOutput::Rgba8MipChain { .. } => {
                    panic!("legacy copy returned mip chain")
                }
                OwnedCopyOutput::Rgba8 { extent, pixels } => {
                    assert_eq!(extent, [2, 1]);
                    assert_eq!(
                        pixels,
                        if direction == 0 {
                            [101, 102, 103, 104, 5, 6, 7, 8]
                        } else {
                            [101, 102, 103, 104, 31, 47, 61, 79]
                        }
                    );
                }
            }
            drop(point);
            assert_eq!(budget.get(), charge);
            drop(output);
            assert_eq!(budget.get(), 0);
        }
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_copy_real_retained_outputs_exhaust_shared_budget_and_refund() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let mut host = vec![0x53u8; 16 * 1024 * 1024];
        host[0] = 17;
        let last = host.len() - 1;
        host[last] = 231;
        let mut outputs = Vec::new();
        for count in 1..=4 {
            let point = cohort
                .submit_copy(RenderQueue::First, QueuedCopyInput::HostBuffer(&host), &[])
                .unwrap();
            let output = match cohort.copy_output(&point).unwrap() {
                RenderCopyOutput::Buffer(output) => output,
                _ => panic!("expected buffer output"),
            };
            match cohort.collect_copy(&point).unwrap() {
                OwnedCopyOutput::Buffer(bytes) => {
                    assert_eq!(bytes.len(), 16 * 1024 * 1024);
                    assert_eq!(bytes[0], 17);
                    assert_eq!(bytes[1], 83);
                    assert_eq!(bytes[last], 231);
                }
                _ => panic!("expected buffer bytes"),
            }
            outputs.push(output);
            drop(point);
            assert_eq!(budget.get(), count * 64 * 1024 * 1024);
        }
        drop(host);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        let sequence = cohort.sequence;
        assert!(
            cohort
                .submit_copy(RenderQueue::Second, QueuedCopyInput::HostBuffer(&[7]), &[])
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_none));
        drop(outputs.pop().unwrap());
        assert_eq!(budget.get(), 192 * 1024 * 1024);
        let recovered = cohort
            .submit_copy(RenderQueue::Second, QueuedCopyInput::HostBuffer(&[7]), &[])
            .unwrap();
        assert_eq!(recovered.inner.value, 1);
        match cohort.collect_copy(&recovered).unwrap() {
            OwnedCopyOutput::Buffer(bytes) => assert_eq!(bytes, [7]),
            _ => panic!("expected buffer bytes"),
        }
        drop(recovered);
        assert_eq!(budget.get(), 192 * 1024 * 1024);
        drop(outputs);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    fn payload_leases_share_charge_reject_atomically_and_refund_last_owner() {
        let budget = Rc::new(Cell::new(0));
        let retained = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 64).unwrap();
        let clone = retained.clone();
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 64);
        {
            let _color = ColorLease::reserve(&budget, 32).unwrap();
            assert!(ColorLease::reserve(&budget, 33).is_err());
            assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 32);
            assert!(ColorLease::reserve(&budget, usize::MAX).is_err());
        } // Setup rejection rolls back the partial reservation.
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 64);
        drop(retained);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 64);
        drop(clone);
        assert_eq!(budget.get(), 0);
        let full = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES).unwrap();
        assert!(ColorLease::reserve(&budget, 1).is_err());
        drop(full);
        assert_eq!(budget.get(), 0);
    }
    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_unaligned_region_parameter_budget_boundary_and_padding() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let source = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
        let destination = [
            101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
        ];
        let request = |unaligned| crate::QueuedBufferRegionCopy {
            source: QueuedCopyInput::HostBuffer(&source),
            destination: QueuedCopyInput::HostBuffer(&destination),
            source_offset: if unaligned { 1 } else { 0 },
            destination_offset: if unaligned { 3 } else { 4 },
            byte_len: if unaligned { 9 } else { 4 },
        };
        // Padded S=16,D=20: persistent D + scratch D + 2S + 2D =112.
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 112).unwrap();
        let aligned = cohort
            .submit_buffer_region_copy(RenderQueue::First, request(false), &[])
            .unwrap();
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        cohort.collect_copy(&aligned).unwrap();
        drop(aligned);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 112);
        let sequence = cohort.sequence;
        assert!(
            cohort
                .submit_buffer_region_copy(RenderQueue::First, request(true), &[])
                .is_err()
        );
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 112);
        assert_eq!(cohort.sequence, sequence);
        assert!(cohort.entries.iter().all(Option::is_none));
        assert!(cohort.buffer_region_pipeline.is_none());
        drop(filler);
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 128).unwrap();
        let point = cohort
            .submit_buffer_region_copy(RenderQueue::First, request(true), &[])
            .unwrap();
        assert_eq!(
            budget.get(),
            MAX_COLOR_VERSION_BYTES,
            "unaligned path adds16copied parameter bytes"
        );
        assert!(cohort.buffer_region_pipeline.is_some());
        let RenderCopyOutput::Buffer(output) = cohort.copy_output(&point).unwrap() else {
            panic!("buffer")
        };
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&point).unwrap() else {
            panic!("buffer")
        };
        assert_eq!(
            bytes,
            [
                101, 102, 103, 2, 3, 4, 5, 6, 7, 8, 9, 10, 113, 114, 115, 116, 117
            ]
        );
        // SAFETY: Collection completed the initialized twenty-byte allocation.
        let padded = unsafe {
            std::slice::from_raw_parts(output.buffer.contents().cast::<u8>().as_ptr(), 20)
        };
        assert_eq!(&padded[17..], &[0, 0, 0]);
        drop(point);
        drop(output);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 128);
        drop(filler);
        assert_eq!(budget.get(), 0);
    }
    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_unaligned_region_retains_both_direct_produced_leases() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let a = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[17, 83, 149, 231, 9]),
                &[],
            )
            .unwrap();
        let b = cohort
            .submit_copy(
                RenderQueue::Second,
                QueuedCopyInput::HostBuffer(&[1, 2, 3, 4, 5, 6, 7, 8, 99]),
                &[],
            )
            .unwrap();
        let RenderCopyOutput::Buffer(source) = cohort.copy_output(&a).unwrap() else {
            panic!("buffer")
        };
        let RenderCopyOutput::Buffer(prior) = cohort.copy_output(&b).unwrap() else {
            panic!("buffer")
        };
        cohort.collect_copy(&a).unwrap();
        cohort.collect_copy(&b).unwrap();
        let point = cohort
            .submit_buffer_region_copy(
                RenderQueue::First,
                crate::QueuedBufferRegionCopy {
                    source: QueuedCopyInput::ProducedBuffer(source.clone()),
                    destination: QueuedCopyInput::ProducedBuffer(prior.clone()),
                    source_offset: 1,
                    destination_offset: 3,
                    byte_len: 3,
                },
                &[],
            )
            .unwrap();
        drop(a);
        drop(b);
        drop(source);
        drop(prior);
        // D12 + scratchD12 + parameters16 + direct source8 + direct prior12.
        assert_eq!(budget.get(), 60);
        let RenderCopyOutput::Buffer(output) = cohort.copy_output(&point).unwrap() else {
            panic!("buffer")
        };
        let clone = output.clone();
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&point).unwrap() else {
            panic!("buffer")
        };
        assert_eq!(bytes, [1, 2, 3, 83, 149, 231, 7, 8, 99]);
        assert!(point.inner.dependencies.borrow().is_empty());
        cohort.reinitialize().unwrap();
        drop(point);
        drop(output);
        assert_eq!(
            budget.get(),
            60,
            "output clone retains both direct leases after pruning and reinitialization"
        );
        drop(clone);
        assert_eq!(budget.get(), 0);
    }
}

#[cfg(test)]
mod graph_admission_tests {
    use super::*;

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_graph_collection_refunds_latest_and_reuses_budget_without_reset() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_graph_cohort().unwrap();
        let budget = cohort.color_bytes.clone();
        let first = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[9; 5]),
                &[],
            )
            .unwrap();
        let RenderCopyOutput::Buffer(output) = cohort.copy_output(&first).unwrap() else {
            panic!("buffer");
        };
        let clone = output.clone();
        assert_eq!(budget.get(), 32, "eight-byte output plus 24-byte scratch");
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&first).unwrap() else {
            panic!("buffer");
        };
        assert_eq!(bytes, [9; 5]);
        assert!(cohort.latest[0].is_none());
        drop(first);
        drop(output);
        assert_eq!(budget.get(), 32, "external output clone remains charged");
        drop(clone);
        assert_eq!(
            budget.get(),
            0,
            "no hidden queue-tail owner after collection"
        );
        // Exactly one further native copy fits. A retained completed tail would
        // make this otherwise admissible operation fail before replacing latest.
        let filler = ColorLease::reserve(&budget, MAX_COLOR_VERSION_BYTES - 32).unwrap();
        let second = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[17; 5]),
                &[],
            )
            .unwrap();
        assert_eq!(second.inner.value, 2);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES);
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&second).unwrap() else {
            panic!("buffer");
        };
        assert_eq!(bytes, [17; 5]);
        drop(second);
        assert_eq!(budget.get(), MAX_COLOR_VERSION_BYTES - 32);
        drop(filler);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_graph_capacity_and_wrong_kind_rejections_are_atomic() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_graph_cohort().unwrap();
        let mut points = Vec::new();
        for value in 0..16u8 {
            let queue = if value % 2 == 0 {
                RenderQueue::First
            } else {
                RenderQueue::Second
            };
            points.push(
                cohort
                    .submit_copy(queue, QueuedCopyInput::HostBuffer(&[value, 91, 17]), &[])
                    .unwrap(),
            );
        }
        let snapshot = |cohort: &RenderCohort| {
            (
                cohort.sequence,
                cohort
                    .latest
                    .each_ref()
                    .map(|point| point.as_ref().map(|point| Rc::as_ptr(&point.inner))),
                cohort.color_bytes.get(),
                cohort
                    .entries
                    .iter()
                    .map(|entry| entry.as_ref().map(|entry| Rc::as_ptr(&entry.point.inner)))
                    .collect::<Vec<_>>(),
            )
        };
        let before = snapshot(&cohort);
        for queue in [RenderQueue::First, RenderQueue::Second] {
            assert!(
                cohort
                    .submit_copy(queue, QueuedCopyInput::HostBuffer(&[255]), &[])
                    .is_err()
            );
            assert_eq!(snapshot(&cohort), before);
        }
        for point in [&points[0], &points[15]] {
            assert!(cohort.collect(point).is_err());
            assert!(cohort.color_output(point, 0).is_err());
            assert!(cohort.depth_output(point).is_err());
            assert_eq!(
                snapshot(&cohort),
                before,
                "wrong kind must neither drain nor consume"
            );
        }
        for (value, point) in points.iter().enumerate().rev() {
            match cohort.collect_copy(point).unwrap() {
                OwnedCopyOutput::Buffer(bytes) => assert_eq!(bytes, [value as u8, 91, 17]),
                _ => panic!("expected buffer"),
            }
        }
        // Successfully collected queue tails no longer pin payloads. Reverse
        // collection validated their ancestors before clearing these identities.
        assert!(cohort.latest.iter().all(Option::is_none));
        let next = cohort
            .submit_copy(RenderQueue::First, QueuedCopyInput::HostBuffer(&[42]), &[])
            .unwrap();
        assert_eq!(next.inner.value, 9);
        cohort.collect_copy(&next).unwrap();
        drop(next);
        drop(points);
        let budget = cohort.color_bytes.clone();
        assert_eq!(
            budget.get(),
            0,
            "collection plus final owner drop refunds without reset"
        );
        cohort.reinitialize().unwrap();
        assert_eq!(cohort.sequence, [0, 0]);
        assert!(cohort.latest.iter().all(Option::is_none));
        assert_eq!(budget.get(), 0);
    }
    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_graph_submit_inherits_controlled_same_queue_failure() {
        let gpu = NativeCompute::new().unwrap();
        let mut cohort = gpu.new_render_graph_cohort().unwrap();
        let first = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[19, 71, 137, 239]),
                &[],
            )
            .unwrap();
        // Finish the real writer before injecting terminal failure metadata. This
        // is controlled failure propagation, not a claim of a hardware fault.
        first.inner.command.waitUntilCompleted();
        first.inner.terminal.set(Some(false));
        let sequence = cohort.sequence;
        let charged = cohort.color_bytes.get();
        assert!(
            cohort
                .submit_copy(
                    RenderQueue::First,
                    QueuedCopyInput::HostBuffer(&[3, 5, 7, 11]),
                    &[]
                )
                .is_err(),
            "implicit same-queue predecessor failure must reject admission"
        );
        assert_eq!(cohort.sequence, sequence);
        assert_eq!(cohort.color_bytes.get(), charged);
        assert_eq!(cohort.entries.iter().flatten().count(), 1);
        assert!(cohort.generation.poisoned.get());
        cohort
            .abort_and_drain("controlled terminal failure")
            .unwrap();
        drop(first);
        assert_eq!(cohort.color_bytes.get(), 0);
        cohort.reinitialize().unwrap();
        let next = cohort
            .submit_copy(
                RenderQueue::First,
                QueuedCopyInput::HostBuffer(&[3, 5, 7, 11]),
                &[],
            )
            .unwrap();
        let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(&next).unwrap() else {
            panic!("buffer");
        };
        assert_eq!(bytes, [3, 5, 7, 11]);
    }
}

#[cfg(test)]
#[path = "mip_array_budget_tests.rs"]
mod mip_array_budget_tests;

#[cfg(test)]
#[path = "typed_copy_budget_tests.rs"]
mod typed_copy_budget_tests;

#[cfg(test)]
#[path = "attachment_view_budget_tests.rs"]
mod attachment_view_budget_tests;

#[cfg(test)]
#[path = "shader_array_budget_tests.rs"]
mod shader_array_budget_tests;
