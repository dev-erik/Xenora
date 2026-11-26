//! Bounded execution of trusted Metal compute kernels and render stages.

#[cfg(not(all(target_arch = "aarch64", target_os = "macos")))]
compile_error!("Xenora's Metal backend requires ARM64 macOS");

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
mod compute;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub mod sampler;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub mod texture_pipeline;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub use compute::{
    BlendDraw, BlendFactor, BlendOperation, BlendPass, BlendState, ColorWriteMask, DepthCompare,
    DepthDraw, DepthFill, DepthLoad, DepthOutput, DepthPass, DepthSnapshot, DepthState,
    DrawUniforms, GuestBuffer, GuestBufferView, IndexBuffer, IndexedDraw, IndexedPass,
    MAX_RENDER_DIMENSION, MAX_RENDER_DRAWS, MAX_RENDER_PIXELS, MAX_RENDER_VERTICES,
    MetalDepthTarget, MetalError, NativeCompute, OwnedDepthInput, OwnedRenderOutput, PendingRender,
    QueuedSampledBinding, RenderBufferOutput, RenderCohort, RenderColorOutput, RenderColorView,
    RenderPoint, RenderQueue, RenderTypedTextureOutput, RunProfile, TriangleShader,
    TypedColorAttachment, TypedColorClear, TypedTriangleTargetsPass,
};

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub use sampler::MetalSampler;

#[cfg(all(target_arch = "aarch64", target_os = "macos"))]
pub use texture_pipeline::{
    ArraySliceViewDesc, ArrayTextureViewDesc, BcMipUpload, BcTextureDesc, BindingSlot,
    BoundSampledTexture, MAX_BC_TEXTURE_BYTES, MAX_R8_TEXTURE_BYTES, MetalBcTexture,
    MetalR8Texture, MetalRgba8ArrayTexture, MetalRgba8Texture, MetalTextureArrayView,
    MetalTextureView, MetalTypedTexture, R8MipUpload, R8TextureDesc, Rgba8ArrayMipUpload,
    Rgba8ArrayTextureDesc, Rgba8MipLevel, Rgba8TextureDesc as MetalRgba8TextureDesc,
    SampledBinding, TextureViewDesc, TextureViewFormat,
};

/// Full immutable texture identity. `Some(1)` is an array, never ordinary 2D.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypedTextureDesc {
    pub format: xenora_texture::TextureFormat,
    pub extent: [usize; 2],
    pub mip_levels: u32,
    pub array_layers: Option<u32>,
}

/// An exact produced texture view; retains both its GPU version and payload lease.
#[derive(Clone)]
pub struct RenderTypedTextureView {
    pub(crate) output: RenderTypedTextureOutput,
    pub(crate) view: MetalTextureView,
}

/// Exact produced array range, retaining the complete immutable output and lease.
#[derive(Clone)]
pub struct RenderTypedTextureArrayView {
    pub(crate) output: RenderTypedTextureOutput,
    pub(crate) view: MetalTextureArrayView,
}

pub enum TypedCopyInput<'a> {
    HostTexture {
        desc: TypedTextureDesc,
        bytes: &'a [u8],
    },
    ProducedTexture(RenderTypedTextureOutput),
    HostBuffer(&'a [u8]),
    ProducedBuffer(RenderBufferOutput),
    ProducedColor(RenderColorOutput),
    ProducedDepth(RenderDepthOutput),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedImageCopyLocation {
    Texture {
        mip: u32,
        slice: u32,
        origin: [usize; 2],
    },
    Buffer {
        offset: usize,
        bytes_per_row: usize,
    },
}

pub struct QueuedTypedImageCopy<'a> {
    pub source: TypedCopyInput<'a>,
    pub destination: TypedCopyInput<'a>,
    pub source_location: TypedImageCopyLocation,
    pub destination_location: TypedImageCopyLocation,
    pub extent: [usize; 2],
}

/// Whole-resource source for a queued GPU copy. Produced inputs retain provenance.
pub enum QueuedCopyInput<'a> {
    HostRgba8Array {
        extent: [usize; 2],
        mip_levels: u32,
        array_layers: u32,
        pixels: &'a [u8],
    },
    HostRgba8MipChain {
        extent: [usize; 2],
        mip_levels: u32,
        pixels: &'a [u8],
    },
    HostBuffer(&'a [u8]),
    ProducedBuffer(RenderBufferOutput),
    HostRgba8 {
        extent: [usize; 2],
        pixels: &'a [u8],
    },
    ProducedColor(RenderColorOutput),
}

/// Arbitrary-byte buffer-region copy into a fresh buffer preserving prior destination bytes.
/// Only buffer variants are accepted for both inputs.
pub struct QueuedBufferRegionCopy<'a> {
    pub source: QueuedCopyInput<'a>,
    pub destination: QueuedCopyInput<'a>,
    pub source_offset: usize,
    pub destination_offset: usize,
    pub byte_len: usize,
}

/// Location within a full backing for a bounded RGBA8 image transfer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageCopyLocation {
    Rgba8Mip {
        mip: u32,
        slice: u32,
        origin: [usize; 2],
    },
    Rgba8 {
        origin: [usize; 2],
    },
    Buffer {
        offset: usize,
        bytes_per_row: usize,
    },
}

/// Image transfer into a fresh destination preserving all prior contents.
/// Buffer-to-buffer transfers use the separate buffer-region API.
pub struct QueuedImageCopy<'a> {
    pub source: QueuedCopyInput<'a>,
    pub destination: QueuedCopyInput<'a>,
    pub source_location: ImageCopyLocation,
    pub destination_location: ImageCopyLocation,
    pub extent: [usize; 2],
}

/// Typed immutable native output acquired before collecting a copy receipt.
pub enum RenderCopyOutput {
    Buffer(RenderBufferOutput),
    Color(RenderColorOutput),
    TypedTexture(RenderTypedTextureOutput),
}

/// Owned host result of collecting a completed GPU copy.
#[derive(Debug)]
pub enum OwnedCopyOutput {
    TypedTexture {
        desc: TypedTextureDesc,
        bytes: Vec<u8>,
    },
    Rgba8Array {
        extent: [usize; 2],
        mip_levels: u32,
        array_layers: u32,
        pixels: Vec<u8>,
    },
    Rgba8MipChain {
        extent: [usize; 2],
        mip_levels: u32,
        pixels: Vec<u8>,
    },
    Buffer(Vec<u8>),
    Rgba8 {
        extent: [usize; 2],
        pixels: Vec<u8>,
    },
}

/// One bounded draw in a trusted-shader RGBA8 render pass.
/// Viewports must fit the target; stateful passes may supply an integer origin.
/// Each draw supplies matching position/color records, with vertex IDs starting at zero.
#[derive(Clone, Copy, Debug)]
pub struct TriangleDraw<'a> {
    pub vertices: &'a [[f32; 4]],
    pub colors: &'a [[f32; 4]],
    pub viewport: [usize; 2],
}

/// Per-draw depth, blending and color-write state for translated triangle passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleState {
    pub depth: DepthState,
    pub blend: Option<BlendState>,
    pub write_mask: ColorWriteMask,
    /// Integer top-left viewport origin in target pixels.
    pub viewport_origin: [usize; 2],
    /// Optional [x, y, width, height] clip rectangle in target pixels.
    /// None covers the target; zero width or height suppresses this draw.
    pub scissor: Option<[usize; 4]>,
}

impl Default for TriangleState {
    fn default() -> Self {
        Self {
            depth: DepthState::DISABLED,
            blend: None,
            write_mask: ColorWriteMask::ALL,
            viewport_origin: [0, 0],
            scissor: None,
        }
    }
}

/// One fixed-target translated pass, with exactly one state per draw.
/// Depth testing/writes require a finite clear depth in 0..=1.
pub struct TrianglePass<'a> {
    pub draws: &'a [TriangleDraw<'a>],
    pub states: &'a [TriangleState],
    pub extent: [usize; 2],
    pub clear: [u8; 4],
    pub clear_depth: Option<f32>,
}

/// Stateful translated pass with 1..=8 RGBA8 color attachments.
/// Clears and returned readbacks use attachment order; all targets share extent.
/// Per-draw blending and write masks currently apply to every color attachment.
pub struct TriangleTargetsPass<'a> {
    pub draws: &'a [TriangleDraw<'a>],
    pub states: &'a [TriangleState],
    pub extent: [usize; 2],
    pub clears: &'a [[u8; 4]],
    pub clear_depth: Option<f32>,
}

/// Initial contents for a fresh queued color attachment. Produced surfaces stay immutable.
#[derive(Clone)]
pub enum QueuedColorLoad<'a> {
    Clear,
    Host(&'a [u8]),
    Produced(RenderColorOutput),
    ProducedTyped(RenderTypedTextureOutput),
    ProducedTypedView(RenderTypedTextureView),
}

/// Immutable exact produced depth version; no raw, upload or sampled conversion.
///
/// ```compile_fail
/// fn cannot_extract(depth: xenora_metal::RenderDepthOutput) { depth.raw(); }
/// ```
/// ```compile_fail
/// fn cannot_upload(depth: xenora_metal::RenderDepthOutput, mip: &xenora_metal::Rgba8MipLevel<'_>) {
///     depth.upload_mip(mip);
/// }
/// ```
#[derive(Clone)]
pub struct RenderDepthOutput {
    pub(crate) point: RenderPoint,
    pub(crate) texture:
        objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>,
    pub(crate) extent: [usize; 2],
}

/// Initial contents of an explicitly owned queued depth attachment.
pub enum QueuedDepthInput {
    Clear(f32),
    Host(DepthSnapshot),
    Produced(RenderDepthOutput),
    /// Load one selected Depth32Float mip while retaining its complete produced root.
    ProducedTypedView(RenderTypedTextureView),
}

/// Buffer-only input for guarded GPU geometry gathering. Produced storage stays on GPU.
pub enum GeometryBuffer<'a> {
    Host(&'a [u8]),
    Produced(RenderBufferOutput),
}
pub struct GeometryAttribute<'a> {
    pub buffer: GeometryBuffer<'a>,
    pub offset: usize,
    pub stride: usize,
}
#[derive(Clone, Copy, Debug)]
pub enum GeometryIndexFormat {
    R16Uint,
    R32Uint,
}
pub struct GeometryIndices<'a> {
    pub buffer: GeometryBuffer<'a>,
    pub offset: usize,
    pub format: GeometryIndexFormat,
}
/// One draw's bounded logical fetch description; no instance expansion.
pub struct QueuedGeometry<'a> {
    pub positions: GeometryAttribute<'a>,
    pub colors: Option<GeometryAttribute<'a>>,
    pub indices: Option<GeometryIndices<'a>>,
    pub start: u32,
    pub count: u32,
    pub base_vertex: i32,
}
