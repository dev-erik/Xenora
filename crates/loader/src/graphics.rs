// SPDX-License-Identifier: GPL-3.0-or-later

//! Guest-facing D3D12-style Graphics Runtime Interfaces for Xenora.
//!
//! Provides typed guest graphics objects, handle management, resource validation,
//! command recording, and backend execution dispatch.
//!
//! Architecturally maps guest concepts:
//! - Guest Device (`ID3D12Device` / test device) -> `GraphicsDevice`
//! - Guest Command Queue (`ID3D12CommandQueue`) -> `GraphicsCommandQueue`
//! - Guest Command Allocator (`ID3D12CommandAllocator`) -> `GraphicsCommandAllocator`
//! - Guest Command List (`ID3D12GraphicsCommandList`) -> `GraphicsCommandList`
//! - Guest Resource (Buffer/Texture) -> `GraphicsResource`
//! - Guest Swap Chain / Present -> Frame presentation & framebuffer readback.

use crate::memory::{GuestMemory, MemoryError};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use xenora_texture::{PackedTextureArrayLayout, PackedTextureLayout, TextureFormat};

#[path = "graphics_ranges.rs"]
mod ranges;
use ranges::{RangeAccesses, RootPublication};

#[path = "graphics_attachments.rs"]
mod attachments;
#[path = "graphics_storage.rs"]
mod storage;
pub use attachments::*;
use attachments::{AttachmentCommit, AttachmentPlan};
use storage::StorageDrawValidation;

#[path = "graphics_resource_roles.rs"]
mod resource_roles;
#[path = "graphics_typed_copy.rs"]
mod typed_copy;
pub use resource_roles::{
    GraphicsResourceRoleCapabilities, GraphicsTypedColorAttachment, GraphicsTypedColorClear,
};

pub use typed_copy::{
    GraphicsTypedCopyCapabilities, GraphicsTypedDepthSnapshot, GraphicsTypedTextureDesc,
};

/// Maximum retained nodes for an explicitly graph-capable queued backend.
pub const MAX_QUEUED_GRAPH_NODES: usize = 16;

/// Generation-checked opaque guest graphics handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsHandle {
    pub id: u32,
    pub generation: u32,
}

impl GraphicsHandle {
    pub const NULL: Self = Self {
        id: 0,
        generation: 0,
    };

    pub fn to_u64(self) -> u64 {
        ((self.generation as u64) << 32) | (self.id as u64)
    }

    pub fn from_u64(val: u64) -> Self {
        Self {
            id: val as u32,
            generation: (val >> 32) as u32,
        }
    }
}

/// D3D12 Command List Type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandListType {
    Direct = 0,
    Bundle = 1,
    Compute = 2,
    Copy = 3,
}

/// D3D12 Resource Dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceDimension {
    Unknown = 0,
    Buffer = 1,
    Texture1D = 2,
    Texture2D = 3,
    Texture3D = 4,
}

/// D3D12 Resource State flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceStates(pub u32);

impl ResourceStates {
    pub const COMMON: Self = Self(0);
    pub const VERTEX_AND_CONSTANT_BUFFER: Self = Self(0x1);
    pub const INDEX_BUFFER: Self = Self(0x2);
    pub const RENDER_TARGET: Self = Self(0x4);
    pub const UNORDERED_ACCESS: Self = Self(0x8);
    pub const DEPTH_WRITE: Self = Self(0x10);
    pub const DEPTH_READ: Self = Self(0x20);
    pub const NON_PIXEL_SHADER_RESOURCE: Self = Self(0x40);
    pub const PIXEL_SHADER_RESOURCE: Self = Self(0x80);
    pub const COPY_DEST: Self = Self(0x400);
    pub const COPY_SOURCE: Self = Self(0x800);
    pub const PRESENT: Self = Self(0);
}

/// Identity of the graphics execution backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsBackendId {
    /// Deterministic software reference rasterizer (strictly for tests/CI without Metal).
    SoftwareReference,
    /// Native Apple Silicon Metal GPU rendering pipeline.
    MetalNative,
}

/// Explicit result of graphics command execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionOutcome {
    pub backend: GraphicsBackendId,
    pub pixels: Vec<u8>,
    /// All color outputs in binding order; pixels is output zero.
    pub color_outputs: Vec<Vec<u8>>,
    pub width: usize,
    pub height: usize,
    pub draw_count: usize,
}

/// Typed error for graphics operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphicsError {
    InvalidDeviceHandle,
    InvalidQueueHandle,
    InvalidAllocatorHandle,
    InvalidCommandListHandle,
    InvalidResourceHandle,
    ResourceLifetimeViolation,
    CommandListNotRecording,
    CommandListAlreadyClosed,
    EmptyCommandList,
    InvalidDescriptor,
    InvalidViewport,
    Memory(MemoryError),
    DeviceCreationFailure(String),
    BackendError(String),
    NativeMetalRequired,
    /// An index buffer format other than DXGI `R16_UINT` (57) or `R32_UINT` (42).
    InvalidIndexFormat(u32),
    /// A draw reads outside its index or vertex buffer, or its address overflows.
    IndexOutOfRange,
}

impl std::fmt::Display for GraphicsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDeviceHandle => write!(f, "invalid or stale graphics device handle"),
            Self::InvalidQueueHandle => write!(f, "invalid or stale command queue handle"),
            Self::InvalidAllocatorHandle => write!(f, "invalid or stale command allocator handle"),
            Self::InvalidCommandListHandle => write!(f, "invalid or stale command list handle"),
            Self::InvalidResourceHandle => write!(f, "invalid or stale graphics resource handle"),
            Self::ResourceLifetimeViolation => write!(f, "graphics resource lifetime violation"),
            Self::CommandListNotRecording => write!(f, "command list is not in recording state"),
            Self::CommandListAlreadyClosed => write!(f, "command list has already been closed"),
            Self::EmptyCommandList => write!(f, "attempted to submit an empty command list"),
            Self::InvalidDescriptor => write!(f, "invalid descriptor or descriptor range"),
            Self::InvalidViewport => write!(f, "invalid viewport extent or bounds"),
            Self::Memory(e) => write!(f, "guest memory error: {e}"),
            Self::DeviceCreationFailure(msg) => write!(f, "device creation failure: {msg}"),
            Self::BackendError(msg) => write!(f, "metal backend error: {msg}"),
            Self::NativeMetalRequired => write!(
                f,
                "native Metal backend required but not provided or failed"
            ),
            Self::InvalidIndexFormat(format) => {
                write!(f, "unsupported index buffer format {format}")
            }
            Self::IndexOutOfRange => write!(f, "draw outside its index or vertex buffer"),
        }
    }
}

impl std::error::Error for GraphicsError {}

impl From<MemoryError> for GraphicsError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}

/// Maximum nonempty draws per submission, matching the bounded native pass.
pub const MAX_NATIVE_RENDER_DRAWS: usize = 256;
/// Maximum expanded triangle-list vertices per native submission.
pub const MAX_NATIVE_RENDER_VERTICES: usize = 65_535;
/// Maximum pixels per native attachment.
pub const MAX_NATIVE_RENDER_PIXELS: usize = 1024 * 1024;
/// Conservative queued color-storage reservation: two receipts, eight RGBA8
/// attachments, one native destination, one readback and one optional host Load
/// snapshot per attachment, plus the first-color outcome copy per receipt.
/// Logical backing storage and the single bounded presentation copy are separate.
pub const MAX_QUEUED_COLOR_BYTES: usize = 2 * (8 * 3 + 1) * MAX_NATIVE_RENDER_PIXELS * 4;
/// Maximum viewport or attachment axis for any renderer.
pub const MAX_RENDER_DIMENSION: usize = 4096;
/// Two depth versions: working/readback, Host snapshot/import, and padded upload
/// reservations. Logical backings and native external leases are separate.
pub const MAX_QUEUED_DEPTH_BYTES: usize =
    2 * (5 * MAX_NATIVE_RENDER_PIXELS * 4 + MAX_RENDER_DIMENSION * 252);
const MAX_SOFTWARE_RENDER_VERTICES: usize = 768;
const MAX_SOFTWARE_RENDER_PIXELS: usize = 4096;

/// Maximum host storage for one resource in the bounded loader.
pub const MAX_RESOURCE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum live resource storage per loader graphics context.
pub const MAX_CONTEXT_RESOURCE_BYTES: usize = 256 * 1024 * 1024;

/// Resource description for buffer/texture creation.
#[derive(Clone, Debug)]
pub struct ResourceDesc {
    pub dimension: ResourceDimension,
    pub width: u64,
    pub height: u32,
    pub depth_or_array_size: u16,
    pub mip_levels: u16,
    pub format_rgba8: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct GraphicsTextureDesc {
    pub width: usize,
    pub height: usize,
    pub mip_levels: u32,
    pub format: TextureFormat,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphicsTextureCapabilities {
    pub bc1: bool,
    pub bc3: bool,
    pub r8: bool,
}

impl GraphicsTextureCapabilities {
    fn supports(self, format: TextureFormat) -> bool {
        match format {
            TextureFormat::Rgba8Unorm => true,
            TextureFormat::R8Unorm => self.r8,
            TextureFormat::Bc1RgbaUnorm => self.bc1,
            TextureFormat::Bc3RgbaUnorm => self.bc3,
            _ => false,
        }
    }
}

/// Transfer-only RGBA8 array; one layer retains explicit array identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsTextureArrayDesc {
    pub width: usize,
    pub height: usize,
    pub mip_levels: u32,
    pub array_layers: u32,
}

/// Guest Graphics Resource.
#[derive(Clone, Debug)]
pub struct GraphicsResource {
    pub is_texture_array: bool,
    pub texture_format: Option<TextureFormat>,
    pub handle: GraphicsHandle,
    pub desc: ResourceDesc,
    pub state: ResourceStates,
    pub guest_backing_addr: Option<u64>,
    pub data: Vec<u8>,
}

/// Index size of an index buffer view, as `D3D12_INDEX_BUFFER_VIEW::Format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexFormat {
    R16Uint,
    R32Uint,
}

impl IndexFormat {
    /// The format for a DXGI format code: `R16_UINT` (57) or `R32_UINT` (42).
    pub const fn from_dxgi(format: u32) -> Option<Self> {
        match format {
            57 => Some(Self::R16Uint),
            42 => Some(Self::R32Uint),
            _ => None,
        }
    }

    /// Bytes per index.
    pub const fn size(self) -> usize {
        match self {
            Self::R16Uint => 2,
            Self::R32Uint => 4,
        }
    }
}

/// Primitive Draw Command recorded in a Command List.
#[derive(Clone, Debug)]
pub enum GraphicsCommand {
    SetStorageView(Option<GraphicsHandle>),
    SetWritableColorTargets {
        views: Vec<GraphicsHandle>,
        loads: Vec<GraphicsColorLoad>,
    },
    SetWritableDepthTarget {
        view: Option<GraphicsHandle>,
        load: GraphicsDepthLoad,
    },
    CopyImage {
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
    },
    CopyBufferRegion {
        destination: GraphicsHandle,
        destination_offset: usize,
        source: GraphicsHandle,
        source_offset: usize,
        byte_len: usize,
    },
    CopyResource {
        destination: GraphicsHandle,
        source: GraphicsHandle,
    },
    SetShaderUniforms(Option<[u32; 8]>),
    SetGraphicsPipeline(Option<GraphicsHandle>),
    SetSampledTexture {
        texture: Option<GraphicsHandle>,
        sampler: xenora_texture::SamplerDesc,
    },
    SetSampledArrayRanges {
        ranges: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    },
    SetSampledViews {
        views: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    },
    SetSampledBindings {
        textures: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    },
    SetVertexColors {
        resource: GraphicsHandle,
        offset: usize,
        stride: usize,
    },
    SetDepthTarget {
        target: Option<GraphicsHandle>,
        load: GraphicsDepthLoad,
    },
    SetColorTargets {
        targets: Vec<GraphicsHandle>,
        loads: Vec<GraphicsColorLoad>,
    },
    SetDepthClear(f32),
    SetDrawState(GraphicsDrawState),
    ResourceBarrier {
        resource: GraphicsHandle,
        before: ResourceStates,
        after: ResourceStates,
    },
    SetRenderTargets {
        color_target: Option<GraphicsHandle>,
        clear_color: [u8; 4],
    },
    SetViewport {
        width: usize,
        height: usize,
    },
    SetVertexBuffer {
        resource: GraphicsHandle,
        offset: usize,
        stride: usize,
    },
    DrawInstanced {
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
    },
    SetIndexBuffer {
        resource: GraphicsHandle,
        offset: usize,
        format: IndexFormat,
    },
    DrawIndexedInstanced {
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
    },
}

/// Command List in recording or closed state.
pub struct GraphicsCommandList {
    pub handle: GraphicsHandle,
    pub list_type: CommandListType,
    pub is_recording: bool,
    pub commands: Vec<GraphicsCommand>,
}

/// Command Allocator tracking command recording buffers.
pub struct GraphicsCommandAllocator {
    pub handle: GraphicsHandle,
    pub list_type: CommandListType,
    pub is_reset: bool,
}

/// Command Queue for command list submission and execution.
pub struct GraphicsCommandQueue {
    pub handle: GraphicsHandle,
    pub list_type: CommandListType,
    /// Most recent 256 successfully completed submissions, oldest first.
    pub submitted_lists: Vec<GraphicsHandle>,
}

/// Custom backend renderer signature for testing or Metal dispatch.
pub type BackendRenderer<'a> =
    dyn Fn(&[[f32; 4]], (usize, usize), [u8; 4]) -> Result<Vec<u8>, GraphicsError> + 'a;

/// Shared thread-safe backend renderer instance.
pub type SharedBackendRenderer = Arc<
    dyn Fn(&[[f32; 4]], (usize, usize), [u8; 4]) -> Result<Vec<u8>, GraphicsError> + Send + Sync,
>;

/// Supported depth comparisons in the bounded host graphics ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsDepthCompare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

/// Blend presets; alpha uses source alpha for RGB and source-over for alpha.
/// Additive adds source and destination independently in all channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphicsBlendMode {
    #[default]
    Replace,
    Alpha,
    Additive,
}

/// Per-draw state. Depth clear defaults to 1.0 and is configured per pass.
/// Color mask bits 0..3 enable R, G, B and A respectively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphicsDrawState {
    pub depth_compare: Option<GraphicsDepthCompare>,
    pub depth_write: bool,
    pub blend: GraphicsBlendMode,
    pub color_write_mask: u8,
    pub viewport_origin: [usize; 2],
    /// [x, y, width, height] in target pixels; an empty rectangle suppresses drawing.
    pub scissor: Option<[usize; 4]>,
}

impl Default for GraphicsDrawState {
    fn default() -> Self {
        Self {
            depth_compare: None,
            depth_write: false,
            blend: GraphicsBlendMode::Replace,
            color_write_mask: 15,
            viewport_origin: [0, 0],
            scissor: None,
        }
    }
}

/// Validated positions and viewport for one ordered draw.
#[derive(Debug)]
pub struct GraphicsPipeline {
    pub storage: Option<xenora_shader::GraphicsStorageType>,
    pub array_layers: [Vec<xenora_shader::GraphicsArrayLayer>; 2],
    pub types: xenora_shader::GraphicsPipelineTypes,
    pub required_uniform_mask: u8,
    pub msl: String,
    pub color_targets: usize,
    pub uses_texture: bool,
    pub sampled_pairs: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsTextureViewFormat {
    Rg8Unorm,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16Float,
    R32Float,
    R32Uint,
    Depth32Float,
    R8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bc1RgbaUnorm,
    Bc1RgbaUnormSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaUnormSrgb,
}

impl GraphicsTextureViewFormat {
    fn storage_format(self) -> TextureFormat {
        match self {
            Self::Rg8Unorm => TextureFormat::Rg8Unorm,
            Self::Bgra8Unorm | Self::Bgra8UnormSrgb => TextureFormat::Bgra8Unorm,
            Self::Rgba16Float => TextureFormat::Rgba16Float,
            Self::R32Float => TextureFormat::R32Float,
            Self::R32Uint => TextureFormat::R32Uint,
            Self::Depth32Float => TextureFormat::Depth32Float,
            Self::R8Unorm => TextureFormat::R8Unorm,
            Self::Rgba8Unorm | Self::Rgba8UnormSrgb => TextureFormat::Rgba8Unorm,
            Self::Bc1RgbaUnorm | Self::Bc1RgbaUnormSrgb => TextureFormat::Bc1RgbaUnorm,
            Self::Bc3RgbaUnorm | Self::Bc3RgbaUnormSrgb => TextureFormat::Bc3RgbaUnorm,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsTextureViewDesc {
    pub base_mip: u32,
    pub mip_count: u32,
    pub format: GraphicsTextureViewFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsArraySliceViewDesc {
    pub slice: u32,
    pub base_mip: u32,
    pub mip_count: u32,
    pub format: GraphicsTextureViewFormat,
}

/// A read-only array range. Select a slice child before binding as texture2D.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsArrayRangeViewDesc {
    pub base_slice: u32,
    pub slice_count: u32,
    pub base_mip: u32,
    pub mip_count: u32,
    pub format: GraphicsTextureViewFormat,
}

struct GraphicsArrayRangeView {
    backing: GraphicsHandle,
    desc: GraphicsArrayRangeViewDesc,
}

struct GraphicsTextureView {
    slice: Option<u32>,
    backing: GraphicsHandle,
    desc: GraphicsTextureViewDesc,
}

#[derive(Clone, Debug)]
pub struct GraphicsSampledTexture {
    pub width: usize,
    pub height: usize,
    pub pixels: Arc<[u8]>,
    pub mip_levels: u32,
    pub sampler: xenora_texture::SamplerDesc,
}

#[derive(Clone, Debug)]
pub struct GraphicsTextureSnapshot {
    pub array_layers: Option<u32>,
    pub format: TextureFormat,
    pub width: usize,
    pub height: usize,
    pub pixels: Arc<[u8]>,
    pub mip_levels: u32,
}

#[derive(Clone, Debug, Default)]
pub struct GraphicsSampledBindings {
    pub sampled_ranges: [Option<GraphicsArrayRangeViewDesc>; 2],
    pub array_views: [Option<GraphicsArraySliceViewDesc>; 2],
    pub views: [Option<GraphicsTextureViewDesc>; 2],
    pub textures: [Option<GraphicsTextureSnapshot>; 2],
    pub samplers: [Option<xenora_texture::SamplerDesc>; 2],
}

#[derive(Clone, Debug, Default)]
pub struct GraphicsDraw {
    pub uniforms: Option<[u32; 8]>,
    pub vertices: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub pipeline: Option<Arc<GraphicsPipeline>>,
    pub sampled: Option<GraphicsSampledTexture>,
    pub sampled_bindings: GraphicsSampledBindings,
    pub viewport: (usize, usize),
    pub state: GraphicsDrawState,
}

type RecordedSampledBindings = (
    [Option<GraphicsHandle>; 2],
    [Option<xenora_texture::SamplerDesc>; 2],
    [Option<GraphicsTextureViewDesc>; 2],
    [Option<GraphicsArraySliceViewDesc>; 2],
    [Option<GraphicsArrayRangeViewDesc>; 2],
    Option<[u32; 8]>,
);

type DrawBindings = (
    Option<Arc<GraphicsPipeline>>,
    Option<GraphicsSampledTexture>,
    GraphicsSampledBindings,
);

/// Ordered pass adapter. A bound texture fixes the extent; otherwise the first draw does.
/// Viewports must fit the extent. Draws share target, color clear and depth clear.
pub type PassRenderer = dyn Fn(&[GraphicsDraw], (usize, usize), [u8; 4], f32) -> Result<Vec<u8>, GraphicsError>
    + Send
    + Sync;

/// Ordered native adapter returning one tightly packed RGBA8 output per clear.
pub type TargetsRenderer = dyn Fn(&[GraphicsDraw], (usize, usize), &[[u8; 4]], f32) -> Result<Vec<Vec<u8>>, GraphicsError>
    + Send
    + Sync;

/// Immutable backend snapshot. Adapters must return a fresh snapshot and never
/// mutate the previous one: the loader commits it only after validating all colors.
pub type GraphicsDepthSnapshot = Arc<dyn Any + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GraphicsDepthLoad {
    Clear(f32),
    Load,
}

/// Initial contents of a host-backed RGBA8 color attachment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GraphicsColorLoad {
    Typed {
        attachment: GraphicsTypedColorAttachment,
        load: bool,
    },
    Clear([u8; 4]),
    Load,
}

/// Immutable base-level color snapshot; `clear` applies when `previous` is None.
#[derive(Clone, Debug)]
pub struct GraphicsColorBinding {
    pub typed: Option<GraphicsTypedColorAttachment>,
    pub clear: [u8; 4],
    pub previous: Option<Arc<[u8]>>,
}

pub type AttachmentRenderer = dyn Fn(
        &[GraphicsDraw],
        (usize, usize),
        &[GraphicsColorBinding],
        f32,
        Option<GraphicsDepthBinding>,
    ) -> Result<GraphicsPassOutput, GraphicsError>
    + Send
    + Sync;

pub struct GraphicsDepthBinding {
    pub load: GraphicsDepthLoad,
    pub previous: Option<GraphicsDepthSnapshot>,
}

pub struct GraphicsPassOutput {
    pub color_layouts: Option<Vec<GraphicsTypedTextureDesc>>,
    pub colors: Vec<Vec<u8>>,
    pub depth: Option<GraphicsDepthSnapshot>,
}

/// Fully owned inputs; no guest-memory or context borrows cross submission.
pub struct GraphicsSubmissionRequest {
    pub draws: Vec<GraphicsDraw>,
    pub extent: (usize, usize),
    pub colors: Vec<GraphicsColorBinding>,
    pub clear_depth: f32,
    pub depth: Option<GraphicsDepthBinding>,
}

/// A logical queue timeline point. Values never wrap or reset across native generations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsQueuePoint {
    pub queue: GraphicsHandle,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphicsProducedOutput {
    AttachmentRoot(u8),
    Color(u8),
    Depth,
    Copy(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsProducedVersion {
    pub point: GraphicsQueuePoint,
    pub output: GraphicsProducedOutput,
}

#[derive(Clone, Debug)]
pub enum GraphicsTextureInput {
    ProducedTyped {
        version: GraphicsProducedVersion,
        desc: GraphicsTypedTextureDesc,
    },
    ProducedArray {
        version: GraphicsProducedVersion,
        width: usize,
        height: usize,
        mip_levels: u32,
        array_layers: u32,
    },
    Host(GraphicsTextureSnapshot),
    Produced {
        version: GraphicsProducedVersion,
        width: usize,
        height: usize,
        mip_levels: u32,
    },
}

/// Immutable logical buffer input for queued GPU geometry gathering.
#[derive(Clone, Debug)]
pub struct GraphicsGeometryBuffer {
    pub byte_len: usize,
    pub source: GraphicsCopyInput,
}
#[derive(Clone, Debug)]
pub struct GraphicsAttributeInput {
    pub buffer: GraphicsGeometryBuffer,
    pub offset: usize,
    pub stride: usize,
}
#[derive(Clone, Debug)]
pub struct GraphicsIndexInput {
    pub buffer: GraphicsGeometryBuffer,
    pub offset: usize,
    pub format: IndexFormat,
}
#[derive(Clone, Debug)]
pub struct GraphicsGeometryInput {
    pub positions: GraphicsAttributeInput,
    pub colors: Option<GraphicsAttributeInput>,
    pub indices: Option<GraphicsIndexInput>,
    pub start: u32,
    pub count: u32,
    pub base_vertex: i32,
}
impl GraphicsGeometryInput {
    fn allocation_bytes(&self) -> Option<usize> {
        let mut bytes = (self.count as usize).checked_mul(32)?.checked_add(128)?;
        for buffer in std::iter::once(&self.positions.buffer)
            .chain(self.colors.as_ref().map(|a| &a.buffer))
            .chain(self.indices.as_ref().map(|a| &a.buffer))
        {
            if matches!(buffer.source, GraphicsCopyInput::Host(_)) {
                bytes = bytes.checked_add(buffer.byte_len.div_ceil(4).checked_mul(12)?)?;
            }
        }
        Some(bytes)
    }
}

#[derive(Clone, Debug)]
pub struct GraphicsQueuedDraw {
    pub sampled_ranges: [Option<GraphicsArrayRangeViewDesc>; 2],
    pub array_views: [Option<GraphicsArraySliceViewDesc>; 2],
    /// Some selects GPU gather and requires empty expanded vertices/colors.
    pub geometry: Option<GraphicsGeometryInput>,
    pub uniforms: Option<[u32; 8]>,
    pub views: [Option<GraphicsTextureViewDesc>; 2],
    pub vertices: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub pipeline: Option<Arc<GraphicsPipeline>>,
    pub textures: [Option<GraphicsTextureInput>; 2],
    pub samplers: [Option<xenora_texture::SamplerDesc>; 2],
    pub viewport: (usize, usize),
    pub state: GraphicsDrawState,
}

/// Immutable color initialization for a fresh queued output version.
#[derive(Clone, Debug)]
pub enum GraphicsQueuedColorInput {
    Clear([u8; 4]),
    Host(Arc<[u8]>),
    Produced(GraphicsProducedVersion),
}

/// Immutable initialization of a fresh queued depth version.
#[derive(Clone)]
pub enum GraphicsQueuedDepthInput {
    Clear(f32),
    Host(GraphicsDepthSnapshot),
    Produced(GraphicsProducedVersion),
}

pub struct GraphicsQueuedSubmissionRequest {
    pub color_types: Vec<Option<GraphicsTypedColorAttachment>>,
    pub generation: u64,
    pub point: GraphicsQueuePoint,
    pub waits: Vec<GraphicsQueuePoint>,
    pub draws: Vec<GraphicsQueuedDraw>,
    pub extent: (usize, usize),
    pub colors: Vec<GraphicsQueuedColorInput>,
    pub clear_depth: f32,
    pub depth: Option<GraphicsQueuedDepthInput>,
    pub outputs: Vec<GraphicsProducedVersion>,
    pub depth_output: Option<GraphicsProducedVersion>,
}

/// Exact logical layout for bounded whole-resource copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsCopyLayout {
    TypedTexture(GraphicsTypedTextureDesc),
    Rgba8Array {
        extent: (usize, usize),
        mip_levels: u32,
        array_layers: u32,
    },
    Buffer {
        byte_len: usize,
    },
    Rgba8 {
        extent: (usize, usize),
    },
    Rgba8MipChain {
        extent: (usize, usize),
        mip_levels: u32,
    },
}

impl GraphicsCopyLayout {
    fn byte_len(self) -> Result<usize, GraphicsError> {
        match self {
            Self::TypedTexture(desc) => desc.byte_len(),
            Self::Rgba8Array {
                extent,
                mip_levels,
                array_layers,
            } => PackedTextureArrayLayout::new(
                TextureFormat::Rgba8Unorm,
                extent.0,
                extent.1,
                mip_levels,
                array_layers,
            )
            .map(|l| l.byte_len())
            .map_err(|_| GraphicsError::InvalidDescriptor),
            Self::Buffer { byte_len } => Ok(byte_len),
            Self::Rgba8MipChain { extent, mip_levels } => {
                PackedTextureLayout::new(TextureFormat::Rgba8Unorm, extent.0, extent.1, mip_levels)
                    .map(|l| l.byte_len())
                    .map_err(|_| GraphicsError::InvalidDescriptor)
            }
            Self::Rgba8 {
                extent: (width, height),
            } => width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(4))
                .ok_or(GraphicsError::InvalidDescriptor),
        }
    }
    fn produced(self) -> GraphicsProducedLayout {
        match self {
            Self::TypedTexture(desc) => desc.produced(),
            Self::Rgba8Array {
                extent,
                mip_levels,
                array_layers,
            } => GraphicsProducedLayout::Rgba8Array {
                extent,
                mip_levels,
                array_layers,
            },
            Self::Buffer { byte_len } => GraphicsProducedLayout::Buffer { byte_len },
            Self::Rgba8 { extent } => GraphicsProducedLayout::Rgba8 { extent },
            Self::Rgba8MipChain { extent, mip_levels } => {
                GraphicsProducedLayout::Rgba8MipChain { extent, mip_levels }
            }
        }
    }
}

/// Texel origin or byte-addressed rows for an image-copy endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsImageCopyLocation {
    Texture {
        mip: u32,
        slice: u32,
        origin: (usize, usize),
    },
    Rgba8Mip {
        mip: u32,
        slice: u32,
        origin: (usize, usize),
    },
    Rgba8 {
        origin: (usize, usize),
    },
    Buffer {
        offset: usize,
        bytes_per_row: usize,
    },
}
impl GraphicsImageCopyLocation {
    fn validate(
        self,
        layout: GraphicsCopyLayout,
        extent: (usize, usize),
    ) -> Result<(), GraphicsError> {
        if let Self::Rgba8Mip { mip, slice, origin } = self {
            if let GraphicsCopyLayout::Rgba8Array {
                extent: full,
                mip_levels,
                array_layers,
            } = layout
            {
                let packed = PackedTextureArrayLayout::new(
                    TextureFormat::Rgba8Unorm,
                    full.0,
                    full.1,
                    mip_levels,
                    array_layers,
                )
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
                let level = packed
                    .footprint(slice, mip)
                    .map_err(|_| GraphicsError::InvalidDescriptor)?;
                return Self::Rgba8 { origin }.validate(
                    GraphicsCopyLayout::Rgba8 {
                        extent: (level.width, level.height),
                    },
                    extent,
                );
            }
            if slice != 0 {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let (full, mips) = match layout {
                GraphicsCopyLayout::Rgba8 { extent } => (extent, 1),
                GraphicsCopyLayout::Rgba8MipChain { extent, mip_levels } => (extent, mip_levels),
                _ => return Err(GraphicsError::InvalidDescriptor),
            };
            let packed = PackedTextureLayout::new(TextureFormat::Rgba8Unorm, full.0, full.1, mips)
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
            let level = packed
                .levels()
                .get(mip as usize)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            return Self::Rgba8 { origin }.validate(
                GraphicsCopyLayout::Rgba8 {
                    extent: (level.width, level.height),
                },
                extent,
            );
        }
        let (width, height) = extent;
        if width == 0 || height == 0 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        match (self, layout) {
            (
                Self::Rgba8 { origin: (x, y) },
                GraphicsCopyLayout::Rgba8 {
                    extent: (full_width, full_height),
                },
            ) => {
                if x.checked_add(width).is_none_or(|end| end > full_width)
                    || y.checked_add(height).is_none_or(|end| end > full_height)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
            }
            (
                Self::Buffer {
                    offset,
                    bytes_per_row,
                },
                GraphicsCopyLayout::Buffer { byte_len },
            ) => {
                let row_bytes = width
                    .checked_mul(4)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                if !offset.is_multiple_of(4)
                    || !bytes_per_row.is_multiple_of(4)
                    || bytes_per_row < row_bytes
                    || bytes_per_row > MAX_RENDER_DIMENSION * 4
                    || (height - 1)
                        .checked_mul(bytes_per_row)
                        .and_then(|n| offset.checked_add(n))
                        .and_then(|n| n.checked_add(row_bytes))
                        .is_none_or(|end| end > byte_len)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
            }
            _ => return Err(GraphicsError::InvalidDescriptor),
        }
        Ok(())
    }
}

/// Authoritative produced storage kind, independent of output-slot identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsProducedLayout {
    TypedTexture(GraphicsTypedTextureDesc),
    Rgba8Array {
        extent: (usize, usize),
        mip_levels: u32,
        array_layers: u32,
    },
    Buffer {
        byte_len: usize,
    },
    Rgba8 {
        extent: (usize, usize),
    },
    Rgba8MipChain {
        extent: (usize, usize),
        mip_levels: u32,
    },
    Depth32Float {
        extent: (usize, usize),
    },
}

#[derive(Clone, Debug)]
pub enum GraphicsCopyInput {
    HostDepth(GraphicsDepthSnapshot),
    Host(Arc<[u8]>),
    Produced(GraphicsProducedVersion),
}

#[derive(Clone, Debug)]
pub struct GraphicsQueuedCopyRequest {
    pub generation: u64,
    pub point: GraphicsQueuePoint,
    pub waits: Vec<GraphicsQueuePoint>,
    pub layout: GraphicsCopyLayout,
    pub source: GraphicsCopyInput,
    pub output: GraphicsProducedVersion,
}

/// Owned inputs and exact destination layout for an aligned buffer-region copy.
#[derive(Clone, Debug)]
pub struct GraphicsQueuedBufferRegionRequest {
    pub generation: u64,
    pub point: GraphicsQueuePoint,
    pub waits: Vec<GraphicsQueuePoint>,
    pub source_byte_len: usize,
    pub destination_byte_len: usize,
    pub source: GraphicsCopyInput,
    pub destination: GraphicsCopyInput,
    pub source_offset: usize,
    pub destination_offset: usize,
    pub byte_len: usize,
    pub output: GraphicsProducedVersion,
}

/// Owned full backings and locations for a queued RGBA8 image transfer.
#[derive(Clone, Debug)]
pub struct GraphicsQueuedImageCopyRequest {
    pub generation: u64,
    pub point: GraphicsQueuePoint,
    pub waits: Vec<GraphicsQueuePoint>,
    pub source_layout: GraphicsCopyLayout,
    pub destination_layout: GraphicsCopyLayout,
    pub source: GraphicsCopyInput,
    pub destination: GraphicsCopyInput,
    pub source_location: GraphicsImageCopyLocation,
    pub destination_location: GraphicsImageCopyLocation,
    pub extent: (usize, usize),
    pub output: GraphicsProducedVersion,
}

pub enum GraphicsQueuedWorkRequest {
    AttachmentTransaction(GraphicsAttachmentTransactionRequest),
    Render(GraphicsQueuedSubmissionRequest),
    Copy(GraphicsQueuedCopyRequest),
    BufferRegion(GraphicsQueuedBufferRegionRequest),
    ImageCopy(GraphicsQueuedImageCopyRequest),
}

pub enum GraphicsQueuedWorkOutput {
    AttachmentTransaction(Vec<GraphicsAttachmentRootResult>),
    Render(GraphicsPassOutput),
    Copy {
        layout: GraphicsCopyLayout,
        bytes: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphicsCopyOutcome {
    pub layout: GraphicsCopyLayout,
    pub bytes: Vec<u8>,
}

/// Terminal results guarantee native work has drained. Tickets remain retained
/// until dependency-node pruning, including after completion and guest release.
pub trait GraphicsQueuedPendingWork: Send {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>>;
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError>;
    fn reject_dependency(&mut self, producer: GraphicsQueuePoint);
}

/// Native completion must mean drained. Keep terminal tickets alive until Drop:
/// Drop releases native receipt/produced-version ownership, not merely GPU work.
pub trait GraphicsQueuedPendingSubmission: Send {
    fn poll(&mut self) -> Option<Result<GraphicsPassOutput, GraphicsError>>;
    fn wait(&mut self) -> Result<GraphicsPassOutput, GraphicsError>;
    /// Called only after the producer is terminal and rejected. Nonblocking;
    /// unblock solely for draining and never report the consumer as successful.
    fn reject_dependency(&mut self, producer: GraphicsQueuePoint);
}

type QueuedRenderer = dyn Fn(GraphicsQueuedWorkRequest) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
    + Send
    + Sync;

struct LegacyQueuedTicket(Box<dyn GraphicsQueuedPendingSubmission>);
impl GraphicsQueuedPendingWork for LegacyQueuedTicket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0
            .poll()
            .map(|result| result.map(GraphicsQueuedWorkOutput::Render))
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.0.wait().map(GraphicsQueuedWorkOutput::Render)
    }
    fn reject_dependency(&mut self, producer: GraphicsQueuePoint) {
        self.0.reject_dependency(producer);
    }
}

struct CopyCommit {
    queue: GraphicsHandle,
    list: GraphicsHandle,
    destination: GraphicsHandle,
    layout: GraphicsCopyLayout,
    states: HashMap<GraphicsHandle, ResourceStates>,
}
enum QueuedCommit {
    Attachment(AttachmentCommit),
    Render(SubmissionCommit),
    Copy(CopyCommit),
}
enum QueuedOutcome {
    Render(ExecutionOutcome),
    Copy(GraphicsCopyOutcome),
}

type QueuedAccesses = (
    HashSet<GraphicsHandle>,
    HashSet<GraphicsHandle>,
    Vec<[Option<GraphicsHandle>; 2]>,
);

struct QueuedReceipt {
    depth_allocation_bytes: usize,
    color_allocation_bytes: usize,
    copy_allocation_bytes: usize,
    layouts: HashMap<GraphicsHandle, GraphicsProducedLayout>,
    commit: Option<QueuedCommit>,
    copy_outcome: Option<GraphicsCopyOutcome>,
    point: GraphicsQueuePoint,
    generation: u64,
    serial: u64,
    dependencies: Vec<GraphicsHandle>,
    semantic_dependencies: Vec<GraphicsHandle>,
    ranges: RangeAccesses,
    publication_bases: HashMap<GraphicsHandle, RootPublication>,
    ticket: Box<dyn GraphicsQueuedPendingWork>,
    receipt: SubmissionReceipt,
    writes: HashSet<GraphicsHandle>,
    reads: HashSet<GraphicsHandle>,
    state_definitions: HashMap<GraphicsHandle, ResourceStates>,
    outputs: HashMap<GraphicsHandle, GraphicsProducedVersion>,
    released: bool,
    rejected_dependencies: HashSet<GraphicsQueuePoint>,
}

#[derive(Default)]
struct QueuedPreparation {
    semantic_dependencies: Vec<GraphicsHandle>,
    destination_bases: HashMap<GraphicsHandle, GraphicsProducedVersion>,
    ranges: RangeAccesses,
    states: HashMap<GraphicsHandle, ResourceStates>,
    produced: HashMap<GraphicsHandle, GraphicsProducedVersion>,
    layouts: HashMap<GraphicsHandle, GraphicsProducedLayout>,
}

/// A terminal result guarantees all native work and native resource use is drained.
/// Implementations must retain their inputs until then. Dropping the context waits.
pub trait GraphicsPendingSubmission: Send {
    fn poll(&mut self) -> Option<Result<GraphicsPassOutput, GraphicsError>>;
    fn wait(&mut self) -> Result<GraphicsPassOutput, GraphicsError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsSubmissionStatus {
    Pending,
    Complete,
    Failed,
}

type AsyncRenderer = dyn Fn(GraphicsSubmissionRequest) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
    + Send
    + Sync;

struct SubmissionCommit {
    serial: Option<u64>,
    queue: GraphicsHandle,
    list: GraphicsHandle,
    targets: Vec<GraphicsHandle>,
    depth: Option<GraphicsHandle>,
    states: HashMap<GraphicsHandle, ResourceStates>,
    extent: (usize, usize),
    outputs: usize,
    byte_count: usize,
    color_bytes: Vec<usize>,
    color_formats: Vec<TextureFormat>,
    typed_colors: bool,
    draw_count: usize,
    backend: GraphicsBackendId,
}

struct PreparedSubmission {
    attachments: Option<AttachmentPlan>,
    geometry: Vec<Option<GraphicsGeometryInput>>,
    queued_depth: Option<GraphicsQueuedDepthInput>,
    queued_colors: Vec<GraphicsQueuedColorInput>,
    request: GraphicsSubmissionRequest,
    vertices: Vec<[f32; 4]>,
    commit: SubmissionCommit,
}

struct SubmissionReceipt {
    handle: GraphicsHandle,
    pending: Option<Box<dyn GraphicsPendingSubmission>>,
    commit: Option<SubmissionCommit>,
    references: HashSet<GraphicsHandle>,
    status: GraphicsSubmissionStatus,
    error: Option<GraphicsError>,
    outcome: Option<ExecutionOutcome>,
}

pub type StatefulRenderer = dyn Fn(
        &[GraphicsDraw],
        (usize, usize),
        &[[u8; 4]],
        f32,
        Option<GraphicsDepthBinding>,
    ) -> Result<GraphicsPassOutput, GraphicsError>
    + Send
    + Sync;

struct DepthResource {
    typed_desc: GraphicsTypedTextureDesc,
    extent: (usize, usize),
    state: ResourceStates,
    snapshot: Option<GraphicsDepthSnapshot>,
}

/// Frame presentation capture holding the last presented frame.
#[derive(Clone, Debug, Default)]
pub struct FramePresentation {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    pub frame_index: u64,
    pub backend: Option<GraphicsBackendId>,
}

/// Central Graphics Context maintaining guest-visible objects and mapping them to Metal.
pub struct GraphicsContext {
    next_id: u32,
    devices: HashMap<GraphicsHandle, bool>,
    queues: HashMap<GraphicsHandle, GraphicsCommandQueue>,
    allocators: HashMap<GraphicsHandle, GraphicsCommandAllocator>,
    command_lists: HashMap<GraphicsHandle, GraphicsCommandList>,
    resources: HashMap<GraphicsHandle, GraphicsResource>,
    depth_resources: HashMap<GraphicsHandle, DepthResource>,
    pipelines: HashMap<GraphicsHandle, Arc<GraphicsPipeline>>,
    pub last_presentation: Arc<Mutex<FramePresentation>>,
    /// Optional active backend renderer (e.g. Native Metal).
    backend: Option<SharedBackendRenderer>,
    pass_backend: Option<Arc<AttachmentRenderer>>,
    backend_id: GraphicsBackendId,
    async_backend: Option<Arc<AsyncRenderer>>,
    multislot_backend: bool,
    view_backend: bool,
    array_view_backend: bool,
    shader_array_views: bool,
    shader_storage_views: bool,
    texture_capabilities: GraphicsTextureCapabilities,
    uniform_backend: bool,
    produced_color_backend: bool,
    produced_depth_backend: bool,
    copy_backend: bool,
    buffer_region_backend: bool,
    image_copy_backend: bool,
    mip_copy_backend: bool,
    array_copy_backend: bool,
    typed_copy_capabilities: GraphicsTypedCopyCapabilities,
    resource_roles: GraphicsResourceRoleCapabilities,
    attachment_transactions: bool,
    writable_views: HashMap<GraphicsHandle, GraphicsWritableSelection>,
    storage_views: HashMap<GraphicsHandle, GraphicsWritableSelection>,
    retired_storage_views: HashMap<GraphicsHandle, GraphicsWritableSelection>,
    retired_writable_views: HashMap<GraphicsHandle, GraphicsWritableSelection>,
    unaligned_buffer_region_backend: bool,
    graph_backend: bool,
    geometry_backend: bool,
    texture_views: HashMap<GraphicsHandle, GraphicsTextureView>,
    array_range_views: HashMap<GraphicsHandle, GraphicsArrayRangeView>,
    samplers: HashMap<GraphicsHandle, xenora_texture::SamplerDesc>,
    retired_views: HashMap<GraphicsHandle, GraphicsTextureView>,
    retired_array_ranges: HashMap<GraphicsHandle, GraphicsArrayRangeView>,
    queued_backend: Option<Arc<QueuedRenderer>>,
    queued: HashMap<GraphicsHandle, QueuedReceipt>,
    queue_values: HashMap<GraphicsHandle, u64>,
    queued_generation: u64,
    queued_failed: bool,
    queued_serial: u64,
    published_serial: u64,
    root_publications: HashMap<GraphicsHandle, RootPublication>,
    submission: Option<SubmissionReceipt>,
    retired_resources: HashMap<GraphicsHandle, GraphicsResource>,
    retired_depth: HashMap<GraphicsHandle, DepthResource>,
    retired_pipelines: HashMap<GraphicsHandle, Arc<GraphicsPipeline>>,
}

impl Drop for GraphicsContext {
    fn drop(&mut self) {
        let mut handles: Vec<_> = self.queued.keys().copied().collect();
        handles.sort_by_key(|handle| self.queued[handle].serial);
        for handle in handles {
            let dependencies = self.queued[&handle].dependencies.clone();
            for dependency in dependencies {
                let point = self.queued[&dependency].point;
                self.queued
                    .get_mut(&handle)
                    .unwrap()
                    .ticket
                    .reject_dependency(point);
            }
            let node = self.queued.get_mut(&handle).unwrap();
            if node.receipt.status == GraphicsSubmissionStatus::Pending {
                let _ = node.ticket.wait();
            }
        }

        if let Some(receipt) = self.submission.as_mut()
            && let Some(pending) = receipt.pending.as_mut()
        {
            // Terminal wait is the backend's guarantee that native work drained.
            // Context destruction discards outputs; it does not publish a frame.
            let _ = pending.wait();
        }
    }
}

impl Default for GraphicsContext {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicsContext {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            devices: HashMap::new(),
            queues: HashMap::new(),
            allocators: HashMap::new(),
            command_lists: HashMap::new(),
            resources: HashMap::new(),
            depth_resources: HashMap::new(),
            pipelines: HashMap::new(),
            last_presentation: Arc::new(Mutex::new(FramePresentation::default())),
            backend: None,
            pass_backend: None,
            backend_id: GraphicsBackendId::SoftwareReference,
            async_backend: None,
            multislot_backend: false,
            view_backend: false,
            array_view_backend: false,
            shader_array_views: false,
            shader_storage_views: false,
            texture_capabilities: GraphicsTextureCapabilities::default(),
            uniform_backend: false,
            produced_color_backend: false,
            produced_depth_backend: false,
            copy_backend: false,
            buffer_region_backend: false,
            image_copy_backend: false,
            mip_copy_backend: false,
            array_copy_backend: false,
            typed_copy_capabilities: GraphicsTypedCopyCapabilities::default(),
            resource_roles: GraphicsResourceRoleCapabilities::default(),
            attachment_transactions: false,
            writable_views: HashMap::new(),
            storage_views: HashMap::new(),
            retired_storage_views: HashMap::new(),
            retired_writable_views: HashMap::new(),
            unaligned_buffer_region_backend: false,
            graph_backend: false,
            geometry_backend: false,
            texture_views: HashMap::new(),
            array_range_views: HashMap::new(),
            samplers: HashMap::new(),
            retired_views: HashMap::new(),
            retired_array_ranges: HashMap::new(),
            queued_backend: None,
            queued: HashMap::new(),
            queue_values: HashMap::new(),
            queued_generation: 1,
            queued_failed: false,
            queued_serial: 0,
            published_serial: 0,
            root_publications: HashMap::new(),
            submission: None,
            retired_resources: HashMap::new(),
            retired_depth: HashMap::new(),
            retired_pipelines: HashMap::new(),
        }
    }

    /// Install bounded two-queue support. The backend must own native generations
    /// and retain produced versions until their terminal ticket is dropped.
    pub fn set_metal_queued_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.texture_capabilities = GraphicsTextureCapabilities::default();
        self.uniform_backend = false;
        self.produced_color_backend = false;
        self.produced_depth_backend = false;
        self.copy_backend = false;
        self.buffer_region_backend = false;
        self.image_copy_backend = false;
        self.mip_copy_backend = false;
        self.array_copy_backend = false;
        self.typed_copy_capabilities = GraphicsTypedCopyCapabilities::default();
        self.resource_roles = GraphicsResourceRoleCapabilities::default();
        self.attachment_transactions = false;
        self.unaligned_buffer_region_backend = false;
        self.graph_backend = false;
        self.geometry_backend = false;
        self.view_backend = false;
        self.array_view_backend = false;
        self.shader_array_views = false;
        self.shader_storage_views = false;
        self.backend = None;
        self.pass_backend = None;
        self.async_backend = None;
        self.multislot_backend = true;
        self.queued_backend = Some(Arc::new(move |work| {
            let GraphicsQueuedWorkRequest::Render(request) = work else {
                return Err(GraphicsError::NativeMetalRequired);
            };
            renderer(request).map(|ticket| {
                Box::new(LegacyQueuedTicket(ticket)) as Box<dyn GraphicsQueuedPendingWork>
            })
        }));
        self.backend_id = GraphicsBackendId::MetalNative;
    }

    pub fn submission_point(
        &self,
        handle: GraphicsHandle,
    ) -> Result<GraphicsQueuePoint, GraphicsError> {
        self.queued
            .get(&handle)
            .filter(|node| !node.released)
            .map(|node| node.point)
            .ok_or(GraphicsError::InvalidResourceHandle)
    }

    /// Submit with retained existing dependencies. Legacy backends admit two nodes
    /// and one pending node per queue; graph backends admit sixteen retained nodes.
    pub fn submit_command_list_after(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        waits: &[GraphicsQueuePoint],
        memory: &GuestMemory,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.submit_command_list_after_internal(queue, list, waits, None, memory)
    }

    fn submit_command_list_after_internal(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        waits: &[GraphicsQueuePoint],
        composition_waits: Option<&[GraphicsQueuePoint]>,
        memory: &GuestMemory,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let capacity = self.queued_submission_capacity();
        if waits
            .len()
            .checked_add(composition_waits.map_or(0, |w| w.len()))
            .is_none_or(|n| n > capacity)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if self.queued_backend.is_none() {
            if waits.is_empty() {
                return self.submit_command_list(queue, list, memory);
            }
            return Err(GraphicsError::NativeMetalRequired);
        }
        if self.submission.is_some()
            || self.queued.len() >= capacity
            || self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS
            || (!self.graph_backend
                && self.queued.values().any(|node| {
                    node.point.queue == queue
                        && node.receipt.status == GraphicsSubmissionStatus::Pending
                }))
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if self.queued_failed && !self.queued.is_empty() {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let generation = if self.queued_failed {
            self.queued_generation
                .checked_add(1)
                .ok_or(GraphicsError::InvalidDescriptor)?
        } else {
            self.queued_generation
        };
        let mut dependencies = Vec::new();
        for point in waits {
            let (&handle, node) = self
                .queued
                .iter()
                .find(|(_, node)| node.point == *point)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            if dependencies.contains(&handle)
                || node.receipt.status == GraphicsSubmissionStatus::Failed
                || node.generation != generation
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            dependencies.push(handle);
        }
        let explicit_semantic = dependencies.clone();
        let mut composition_dependencies = Vec::new();
        for point in composition_waits.unwrap_or(&[]) {
            let (&handle, node) = self
                .queued
                .iter()
                .find(|(_, n)| n.point == *point)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            if node.generation != generation
                || node.receipt.status == GraphicsSubmissionStatus::Failed
                || dependencies.contains(&handle)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            composition_dependencies.push(handle);
            dependencies.push(handle);
        }
        if let Some((&handle, node)) = self
            .queued
            .iter()
            .filter(|(_, node)| node.point.queue == queue)
            .max_by_key(|(_, node)| node.point.value)
            && node.generation == generation
            && node.receipt.status != GraphicsSubmissionStatus::Failed
            && !dependencies.contains(&handle)
        {
            dependencies.push(handle);
        }
        let closure = self.queued_dependency_closure(&dependencies)?;
        let semantic_dependencies = if composition_waits.is_some() {
            explicit_semantic
        } else {
            dependencies.clone()
        };
        let semantic_closure = self.semantic_closure(&semantic_dependencies)?;
        let mut preparation = self.queued_preparation(&semantic_closure, generation)?;
        preparation.semantic_dependencies = semantic_dependencies;

        if self.command_lists.get(&list).is_some_and(|commands| {
            commands.commands.iter().any(|command| {
                matches!(
                    command,
                    GraphicsCommand::CopyResource { .. }
                        | GraphicsCommand::CopyBufferRegion { .. }
                        | GraphicsCommand::CopyImage { .. }
                )
            })
        }) {
            if composition_waits.is_some() {
                return Err(GraphicsError::InvalidDescriptor);
            }
            return self.submit_queued_copy(
                queue,
                list,
                generation,
                dependencies,
                preparation,
                memory,
            );
        }
        let mut prepared = self.prepare_submission(
            queue,
            list,
            memory,
            true,
            GraphicsBackendId::MetalNative,
            Some(&preparation),
        )?;
        let (mut reads, mut writes, sampled_handles) = self.queued_accesses(list);
        writes.extend(prepared.commit.targets.iter().copied());
        if let Some(depth) = prepared.commit.depth {
            if matches!(
                prepared.queued_depth,
                Some(GraphicsQueuedDepthInput::Clear(_))
            ) {
                writes.insert(depth);
            } else {
                reads.insert(depth);
            }
            if prepared
                .request
                .draws
                .iter()
                .any(|draw| draw.state.depth_write)
            {
                writes.insert(depth);
            }
        }
        if let Some(plan) = &prepared.attachments {
            writes.extend(plan.roots.iter().map(|r| r.root));
        }
        preparation.ranges =
            self.range_accesses(list, prepared.attachments.as_ref(), &reads, &writes);
        if !composition_dependencies.is_empty() {
            let plan = prepared
                .attachments
                .as_ref()
                .ok_or(GraphicsError::InvalidDescriptor)?;
            preparation.destination_bases = self.composition_bases(
                plan,
                &preparation,
                &composition_dependencies,
                &preparation.ranges,
                generation,
            )?;
        }
        for root in &writes {
            if let Some(chosen) = preparation
                .destination_bases
                .get(root)
                .or_else(|| preparation.produced.get(root))
            {
                self.validate_published_base(*root, *chosen, generation)?;
            }
        }
        for (handle, node) in &self.queued {
            if composition_waits.is_some()
                && node.generation == generation
                && !semantic_closure.contains(handle)
            {
                for root in writes.iter().filter(|h| node.outputs.contains_key(h)) {
                    let base = preparation
                        .destination_bases
                        .get(root)
                        .ok_or(GraphicsError::ResourceLifetimeViolation)?;
                    let owner = self
                        .queued
                        .iter()
                        .find(|(_, n)| n.outputs.get(root) == Some(base))
                        .map(|(h, _)| *h)
                        .ok_or(GraphicsError::InvalidResourceHandle)?;
                    if !self.queued_dependency_closure(&[owner])?.contains(handle) {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                }
            }
            if node.receipt.status != GraphicsSubmissionStatus::Pending {
                continue;
            }
            let conflict = preparation.ranges.conflicts(&node.ranges)
                // Disjoint writers still need an explicit composition base; simply
                // accepting two complete-root outputs would lose one update.
                || writes.iter().any(|h| node.writes.contains(h));
            if conflict && !closure.contains(handle) {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            if prepared
                .commit
                .depth
                .is_some_and(|h| node.reads.contains(&h) || node.writes.contains(&h))
                && !closure.contains(handle)
            {
                // Read/read also needs ordering: each pass publishes a new snapshot.
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
        }
        // Dependency-covered reuse writes a fresh version, never producer storage.
        // Sampling an attachment in this same pass remains rejected by preparation.
        if !self.produced_color_backend
            && prepared
                .queued_colors
                .iter()
                .any(|input| matches!(input, GraphicsQueuedColorInput::Produced(_)))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if !self.produced_depth_backend
            && matches!(
                prepared.queued_depth,
                Some(GraphicsQueuedDepthInput::Produced(_))
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let depth_allocation_bytes = if prepared.queued_depth.is_some() {
            let bytes = prepared.commit.byte_count;
            let mut reservation = bytes
                .checked_mul(2)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            if matches!(
                prepared.queued_depth,
                Some(GraphicsQueuedDepthInput::Host(_))
            ) {
                let row_bytes = prepared
                    .commit
                    .extent
                    .0
                    .checked_mul(4)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                let padded = row_bytes
                    .div_ceil(256)
                    .checked_mul(256)
                    .and_then(|pitch| pitch.checked_mul(prepared.commit.extent.1))
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                reservation = reservation
                    .checked_add(
                        bytes
                            .checked_mul(2)
                            .ok_or(GraphicsError::InvalidDescriptor)?,
                    )
                    .and_then(|total| total.checked_add(padded))
                    .ok_or(GraphicsError::InvalidDescriptor)?;
            }
            reservation
        } else {
            0
        };
        if self
            .queued_depth_allocation_bytes()
            .checked_add(depth_allocation_bytes)
            .is_none_or(|bytes| bytes > MAX_QUEUED_DEPTH_BYTES)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let output_bytes = prepared
            .commit
            .color_bytes
            .iter()
            .try_fold(0usize, |sum, n| sum.checked_add(*n))
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let geometry_allocation_bytes = prepared
            .geometry
            .iter()
            .flatten()
            .try_fold(0usize, |sum, input| {
                sum.checked_add(input.allocation_bytes()?)
            })
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let geometry_allocation_bytes =
            geometry_allocation_bytes + usize::from(geometry_allocation_bytes != 0) * 4;
        let color_allocation_bytes = prepared.queued_colors.iter().try_fold(
            output_bytes
                .checked_mul(2)
                .and_then(|bytes| bytes.checked_add(prepared.commit.byte_count))
                .ok_or(GraphicsError::InvalidDescriptor)?,
            |total, input| {
                total
                    .checked_add(match input {
                        GraphicsQueuedColorInput::Host(bytes) => bytes.len(),
                        _ => 0,
                    })
                    .ok_or(GraphicsError::InvalidDescriptor)
            },
        )?;
        if self
            .queued_color_allocation_bytes()
            .checked_add(color_allocation_bytes)
            .is_none_or(|bytes| bytes > MAX_QUEUED_COLOR_BYTES)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let color_allocation_bytes = color_allocation_bytes
            .checked_add(geometry_allocation_bytes)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let point = GraphicsQueuePoint {
            queue,
            value: self
                .queue_values
                .get(&queue)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(GraphicsError::InvalidDescriptor)?,
        };
        let serial = self
            .queued_serial
            .checked_add(1)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let outputs: Vec<_> = (0..prepared.commit.outputs)
            .map(|slot| GraphicsProducedVersion {
                point,
                output: GraphicsProducedOutput::Color(slot as u8),
            })
            .collect();
        let depth_output = prepared.commit.depth.map(|_| GraphicsProducedVersion {
            point,
            output: GraphicsProducedOutput::Depth,
        });
        let mut output_handles: HashMap<_, _> = prepared
            .commit
            .targets
            .iter()
            .copied()
            .zip(outputs.iter().copied())
            .collect();
        if let Some((handle, version)) = prepared.commit.depth.zip(depth_output) {
            output_handles.insert(handle, version);
        }
        let mut state_definitions = HashMap::new();
        for handle in writes.iter().chain(output_handles.keys()) {
            let state = prepared
                .commit
                .states
                .get(handle)
                .copied()
                .or_else(|| preparation.states.get(handle).copied())
                .or_else(|| self.resources.get(handle).map(|r| r.state))
                .or_else(|| self.depth_resources.get(handle).map(|r| r.state));
            if let Some(state) = state {
                state_definitions.insert(*handle, state);
            }
        }
        prepared
            .commit
            .states
            .retain(|handle, _| state_definitions.contains_key(handle));
        let mut draws = Vec::new();
        for ((draw, handles), geometry) in prepared
            .request
            .draws
            .into_iter()
            .zip(sampled_handles)
            .zip(prepared.geometry)
        {
            let mut textures = [None, None];
            for (slot, snapshot) in draw.sampled_bindings.textures.into_iter().enumerate() {
                if let Some(snapshot) = snapshot {
                    textures[slot] = Some(
                        if let Some(version) =
                            handles[slot].and_then(|h| preparation.produced.get(&h))
                        {
                            let desc = GraphicsTypedTextureDesc {
                                format: snapshot.format,
                                extent: (snapshot.width, snapshot.height),
                                mip_levels: snapshot.mip_levels,
                                array_layers: snapshot.array_layers,
                            };
                            if preparation
                                .layouts
                                .get(&handles[slot].expect("produced handle"))
                                != Some(&desc.produced())
                                || !matches!(
                                    version.output,
                                    GraphicsProducedOutput::Color(_)
                                        | GraphicsProducedOutput::AttachmentRoot(_)
                                        | GraphicsProducedOutput::Copy(0)
                                        | GraphicsProducedOutput::Depth
                                )
                                || (matches!(version.output, GraphicsProducedOutput::Copy(_))
                                    && !self.copy_backend)
                                || (snapshot.mip_levels != 1 && !self.mip_copy_backend)
                            {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            if snapshot.format != TextureFormat::Rgba8Unorm
                                || matches!(
                                    version.output,
                                    GraphicsProducedOutput::AttachmentRoot(_)
                                )
                            {
                                GraphicsTextureInput::ProducedTyped {
                                    version: *version,
                                    desc,
                                }
                            } else if let Some(array_layers) = snapshot.array_layers {
                                GraphicsTextureInput::ProducedArray {
                                    version: *version,
                                    width: snapshot.width,
                                    height: snapshot.height,
                                    mip_levels: snapshot.mip_levels,
                                    array_layers,
                                }
                            } else {
                                GraphicsTextureInput::Produced {
                                    version: *version,
                                    width: snapshot.width,
                                    height: snapshot.height,
                                    mip_levels: snapshot.mip_levels,
                                }
                            }
                        } else {
                            GraphicsTextureInput::Host(snapshot)
                        },
                    );
                }
            }
            draws.push(GraphicsQueuedDraw {
                geometry,
                uniforms: draw.uniforms,
                array_views: draw.sampled_bindings.array_views,
                sampled_ranges: draw.sampled_bindings.sampled_ranges,
                views: draw.sampled_bindings.views,
                vertices: draw.vertices,
                colors: draw.colors,
                pipeline: draw.pipeline,
                textures,
                samplers: draw.sampled_bindings.samplers,
                viewport: draw.viewport,
                state: draw.state,
            });
        }
        if let Some(plan) = prepared.attachments.take() {
            return self.admit_attachment_transaction(
                plan,
                prepared.commit,
                draws,
                generation,
                point,
                serial,
                dependencies,
                closure,
                preparation,
                reads,
                writes,
                state_definitions,
                color_allocation_bytes + depth_allocation_bytes,
            );
        }
        let request = GraphicsQueuedSubmissionRequest {
            color_types: prepared.request.colors.iter().map(|c| c.typed).collect(),
            generation,
            point,
            waits: closure.iter().map(|h| self.queued[h].point).collect(),
            draws,
            extent: prepared.request.extent,
            colors: prepared.queued_colors,
            clear_depth: prepared.request.clear_depth,
            depth: prepared.queued_depth,
            outputs,
            depth_output,
        };
        let renderer = self.queued_backend.clone().expect("queued backend");
        self.validate_queued_work_budget(color_allocation_bytes + depth_allocation_bytes)?;
        let ticket = renderer(GraphicsQueuedWorkRequest::Render(request))?;
        let references = self.recorded_references(list);
        let handle = self.allocate_handle();
        prepared.commit.serial = Some(serial);
        self.queued.insert(
            handle,
            QueuedReceipt {
                depth_allocation_bytes,
                color_allocation_bytes,
                copy_allocation_bytes: 0,
                publication_bases: self.publication_bases(output_handles.keys().copied()),
                layouts: output_handles
                    .iter()
                    .map(|(handle, version)| {
                        (
                            *handle,
                            match version.output {
                                GraphicsProducedOutput::Depth => {
                                    GraphicsProducedLayout::Depth32Float {
                                        extent: prepared.commit.extent,
                                    }
                                }
                                GraphicsProducedOutput::Color(slot) => GraphicsTypedTextureDesc {
                                    format: prepared.commit.color_formats[slot as usize],
                                    extent: prepared.commit.extent,
                                    mip_levels: 1,
                                    array_layers: None,
                                }
                                .produced(),
                                _ => unreachable!("render output"),
                            },
                        )
                    })
                    .collect(),
                commit: Some(QueuedCommit::Render(prepared.commit)),
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
                outputs: output_handles,
                released: false,
                rejected_dependencies: HashSet::new(),
                receipt: SubmissionReceipt {
                    handle,
                    pending: None,
                    commit: None,
                    references,
                    status: GraphicsSubmissionStatus::Pending,
                    error: None,
                    outcome: None,
                },
            },
        );
        self.queue_values.insert(queue, point.value);
        self.queued_serial = serial;
        self.queued_generation = generation;
        self.queued_failed = false;
        Ok(handle)
    }

    fn copy_layout(&self, handle: GraphicsHandle) -> Result<GraphicsCopyLayout, GraphicsError> {
        self.copy_layout_with_guest_source(handle, false)
    }

    /// Whole-buffer and buffer-region source snapshots may resolve guest backing;
    /// destinations and image-copy endpoints remain owned-only.
    fn copy_layout_with_guest_source(
        &self,
        handle: GraphicsHandle,
        allow_guest_source: bool,
    ) -> Result<GraphicsCopyLayout, GraphicsError> {
        self.copy_layout_for_transfer(handle, allow_guest_source, false, false)
    }

    fn copy_layout_for_transfer(
        &self,
        handle: GraphicsHandle,
        allow_guest_source: bool,
        allow_mips: bool,
        allow_arrays: bool,
    ) -> Result<GraphicsCopyLayout, GraphicsError> {
        let resource = self
            .resources
            .get(&handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let desc = &resource.desc;
        if resource.is_texture_array {
            if !allow_arrays
                || resource.guest_backing_addr.is_some()
                || resource.texture_format != Some(TextureFormat::Rgba8Unorm)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let layout = GraphicsCopyLayout::Rgba8Array {
                extent: (desc.width as usize, desc.height as usize),
                mip_levels: u32::from(desc.mip_levels),
                array_layers: u32::from(desc.depth_or_array_size),
            };
            if layout.byte_len()? != resource.data.len() {
                return Err(GraphicsError::InvalidDescriptor);
            }
            return Ok(layout);
        }
        if (resource.guest_backing_addr.is_some()
            && (!allow_guest_source || desc.dimension != ResourceDimension::Buffer))
            || desc.depth_or_array_size != 1
            || (!allow_mips && desc.mip_levels != 1)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let layout = match desc.dimension {
            ResourceDimension::Buffer
                if desc.height == 1 && !desc.format_rgba8 && resource.texture_format.is_none() =>
            {
                let byte_len =
                    usize::try_from(desc.width).map_err(|_| GraphicsError::InvalidDescriptor)?;
                if byte_len == 0 || byte_len > MAX_RESOURCE_BYTES {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                GraphicsCopyLayout::Buffer { byte_len }
            }
            ResourceDimension::Texture2D
                if desc.format_rgba8
                    && resource.texture_format == Some(TextureFormat::Rgba8Unorm) =>
            {
                let width =
                    usize::try_from(desc.width).map_err(|_| GraphicsError::InvalidDescriptor)?;
                let height = desc.height as usize;
                if width == 0
                    || height == 0
                    || width > MAX_RENDER_DIMENSION
                    || height > MAX_RENDER_DIMENSION
                    || width
                        .checked_mul(height)
                        .is_none_or(|pixels| pixels > MAX_NATIVE_RENDER_PIXELS)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                if desc.mip_levels == 1 {
                    GraphicsCopyLayout::Rgba8 {
                        extent: (width, height),
                    }
                } else {
                    GraphicsCopyLayout::Rgba8MipChain {
                        extent: (width, height),
                        mip_levels: desc.mip_levels as u32,
                    }
                }
            }
            _ => return Err(GraphicsError::InvalidDescriptor),
        };
        if resource.data.len() != layout.byte_len()? {
            return Err(GraphicsError::InvalidDescriptor);
        }
        Ok(layout)
    }

    fn submit_queued_copy(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        generation: u64,
        dependencies: Vec<GraphicsHandle>,
        preparation: QueuedPreparation,
        memory: &GuestMemory,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let closure = self.queued_dependency_closure(&dependencies)?;
        if !self.copy_backend {
            return Err(GraphicsError::NativeMetalRequired);
        }
        if !self.queues.contains_key(&queue) {
            return Err(GraphicsError::InvalidQueueHandle);
        }
        let commands = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        let mut states = preparation.states.clone();
        let mut changed_states = HashMap::new();
        let mut reads = HashSet::new();
        let mut writes = HashSet::new();
        let mut copy = None;
        let mut region = None;
        let mut image = None;
        let mut mip_copy = false;
        let mut typed_copy = false;
        for command in &commands.commands {
            match *command {
                GraphicsCommand::ResourceBarrier {
                    resource,
                    before,
                    after,
                } => {
                    let initial = self.copy_resource_state(resource)?;
                    if states.get(&resource).copied().unwrap_or(initial) != before {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                    states.insert(resource, after);
                    changed_states.insert(resource, after);
                    writes.insert(resource);
                }
                GraphicsCommand::CopyResource {
                    destination,
                    source,
                }
                | GraphicsCommand::CopyBufferRegion {
                    destination,
                    source,
                    ..
                }
                | GraphicsCommand::CopyImage {
                    destination,
                    source,
                    ..
                } => {
                    if let GraphicsCommand::CopyImage {
                        source_location,
                        destination_location,
                        extent,
                        ..
                    } = *command
                    {
                        if !self.image_copy_backend {
                            return Err(GraphicsError::NativeMetalRequired);
                        }
                        typed_copy =
                            matches!(source_location, GraphicsImageCopyLocation::Texture { .. })
                                || matches!(
                                    destination_location,
                                    GraphicsImageCopyLocation::Texture { .. }
                                );
                        mip_copy =
                            matches!(source_location, GraphicsImageCopyLocation::Rgba8Mip { .. })
                                || matches!(
                                    destination_location,
                                    GraphicsImageCopyLocation::Rgba8Mip { .. }
                                );
                        if mip_copy && !self.mip_copy_backend {
                            return Err(GraphicsError::NativeMetalRequired);
                        }
                        image = Some((source_location, destination_location, extent));
                    }
                    if let GraphicsCommand::CopyBufferRegion {
                        source_offset,
                        destination_offset,
                        byte_len,
                        ..
                    } = *command
                    {
                        if !self.buffer_region_backend {
                            return Err(GraphicsError::NativeMetalRequired);
                        }
                        region = Some((source_offset, destination_offset, byte_len));
                    }
                    if copy.is_some() || destination == source {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let source_layout =
                        self.copy_submission_layout(source, image.is_none(), mip_copy, typed_copy)?;
                    let layout =
                        self.copy_submission_layout(destination, false, mip_copy, typed_copy)?;
                    if (matches!(source_layout, GraphicsCopyLayout::Rgba8Array { .. })
                        || matches!(layout, GraphicsCopyLayout::Rgba8Array { .. }))
                        && !self.array_copy_backend
                    {
                        return Err(GraphicsError::NativeMetalRequired);
                    }
                    if let Some((source_location, destination_location, extent)) = image {
                        if matches!(
                            (source_location, destination_location),
                            (
                                GraphicsImageCopyLocation::Buffer { .. },
                                GraphicsImageCopyLocation::Buffer { .. }
                            )
                        ) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        if typed_copy {
                            self.validate_typed_pair(
                                source_layout,
                                layout,
                                source_location,
                                destination_location,
                                extent,
                                true,
                            )?;
                        } else {
                            source_location.validate(source_layout, extent)?;
                            destination_location.validate(layout, extent)?;
                        }
                        reads.insert(destination);
                    } else if let Some((source_offset, destination_offset, byte_len)) = region {
                        let (
                            GraphicsCopyLayout::Buffer {
                                byte_len: source_len,
                            },
                            GraphicsCopyLayout::Buffer {
                                byte_len: destination_len,
                            },
                        ) = (source_layout, layout)
                        else {
                            return Err(GraphicsError::InvalidDescriptor);
                        };
                        if byte_len == 0
                            || source_offset
                                .checked_add(byte_len)
                                .is_none_or(|end| end > source_len)
                            || destination_offset
                                .checked_add(byte_len)
                                .is_none_or(|end| end > destination_len)
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        if (!byte_len.is_multiple_of(4)
                            || !source_offset.is_multiple_of(4)
                            || !destination_offset.is_multiple_of(4))
                            && !self.unaligned_buffer_region_backend
                        {
                            return Err(GraphicsError::NativeMetalRequired);
                        }
                        reads.insert(destination);
                    } else if source_layout != layout {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if states
                        .get(&source)
                        .copied()
                        .unwrap_or(self.copy_resource_state(source)?)
                        != ResourceStates::COPY_SOURCE
                        || states
                            .get(&destination)
                            .copied()
                            .unwrap_or(self.copy_resource_state(destination)?)
                            != ResourceStates::COPY_DEST
                    {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                    reads.insert(source);
                    writes.insert(destination);
                    copy = Some((destination, source, layout));
                }
                _ => return Err(GraphicsError::InvalidDescriptor),
            }
        }
        let (destination, source, layout) = copy.ok_or(GraphicsError::EmptyCommandList)?;
        for (handle, node) in &self.queued {
            if node.receipt.status == GraphicsSubmissionStatus::Pending
                && !closure.contains(handle)
                && (writes
                    .iter()
                    .any(|h| node.reads.contains(h) || node.writes.contains(h))
                    || reads.iter().any(|h| node.writes.contains(h)))
            {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
        }
        let source_layout =
            self.copy_submission_layout(source, image.is_none(), mip_copy, typed_copy)?;
        let produced = preparation.produced.get(&source).copied();
        if produced.is_some()
            && self
                .resources
                .get(&source)
                .is_some_and(|r| r.guest_backing_addr.is_some())
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let prior = if region.is_some() || image.is_some() {
            preparation.produced.get(&destination).copied()
        } else {
            None
        };
        if let Some(version) = prior
            && (preparation.layouts.get(&destination) != Some(&layout.produced())
                || !match layout {
                    GraphicsCopyLayout::Buffer { .. } => {
                        version.output == GraphicsProducedOutput::Copy(0)
                    }
                    GraphicsCopyLayout::TypedTexture(desc) => {
                        matches!(
                            version.output,
                            GraphicsProducedOutput::Copy(0)
                                | GraphicsProducedOutput::AttachmentRoot(_)
                        ) || (desc.format == TextureFormat::Depth32Float
                            && version.output == GraphicsProducedOutput::Depth)
                            || (desc.format == TextureFormat::Rgba8Unorm
                                && matches!(version.output, GraphicsProducedOutput::Color(_)))
                    }
                    GraphicsCopyLayout::Rgba8 { .. }
                    | GraphicsCopyLayout::Rgba8MipChain { .. }
                    | GraphicsCopyLayout::Rgba8Array { .. } => {
                        matches!(
                            version.output,
                            GraphicsProducedOutput::Color(_)
                                | GraphicsProducedOutput::Copy(0)
                                | GraphicsProducedOutput::AttachmentRoot(_)
                        )
                    }
                })
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if let Some(version) = produced
            && (preparation.layouts.get(&source) != Some(&source_layout.produced())
                || !match source_layout {
                    GraphicsCopyLayout::Buffer { .. } => {
                        version.output == GraphicsProducedOutput::Copy(0)
                    }
                    GraphicsCopyLayout::TypedTexture(desc) => {
                        matches!(
                            version.output,
                            GraphicsProducedOutput::Copy(0)
                                | GraphicsProducedOutput::AttachmentRoot(_)
                        ) || (desc.format == TextureFormat::Depth32Float
                            && version.output == GraphicsProducedOutput::Depth)
                            || (desc.format == TextureFormat::Rgba8Unorm
                                && matches!(version.output, GraphicsProducedOutput::Color(_)))
                    }
                    GraphicsCopyLayout::Rgba8 { .. }
                    | GraphicsCopyLayout::Rgba8MipChain { .. }
                    | GraphicsCopyLayout::Rgba8Array { .. } => {
                        matches!(
                            version.output,
                            GraphicsProducedOutput::Color(_)
                                | GraphicsProducedOutput::Copy(0)
                                | GraphicsProducedOutput::AttachmentRoot(_)
                        )
                    }
                })
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if let Some(version) = prior {
            self.validate_published_base(destination, version, generation)?;
        }
        let bytes = layout.byte_len()?;
        let padded = match layout {
            GraphicsCopyLayout::Buffer { .. } => bytes
                .checked_add(3)
                .map(|n| n & !3)
                .ok_or(GraphicsError::InvalidDescriptor)?,
            GraphicsCopyLayout::TypedTexture(_)
            | GraphicsCopyLayout::Rgba8 { .. }
            | GraphicsCopyLayout::Rgba8MipChain { .. }
            | GraphicsCopyLayout::Rgba8Array { .. } => bytes,
        };
        let factor = match (layout, produced.is_some()) {
            (_, true) => 2,
            (GraphicsCopyLayout::Buffer { .. }, false) => 4,
            (
                GraphicsCopyLayout::TypedTexture(_)
                | GraphicsCopyLayout::Rgba8 { .. }
                | GraphicsCopyLayout::Rgba8MipChain { .. }
                | GraphicsCopyLayout::Rgba8Array { .. },
                false,
            ) => 3,
        };
        let copy_allocation_bytes = if region.is_some() || image.is_some() {
            let source_padded = source_layout
                .byte_len()?
                .checked_add(3)
                .map(|n| n & !3)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            padded
                .checked_mul(2)
                .and_then(|n| {
                    n.checked_add(if produced.is_none() {
                        source_padded
                            * if matches!(source_layout, GraphicsCopyLayout::Buffer { .. }) {
                                2
                            } else {
                                1
                            }
                    } else {
                        0
                    })
                })
                .and_then(|n| {
                    n.checked_add(if prior.is_none() {
                        padded
                            * if matches!(layout, GraphicsCopyLayout::Buffer { .. }) {
                                2
                            } else {
                                1
                            }
                    } else {
                        0
                    })
                })
                .ok_or(GraphicsError::InvalidDescriptor)?
        } else {
            padded
                .checked_mul(factor)
                .ok_or(GraphicsError::InvalidDescriptor)?
        };
        let copy_allocation_bytes = if typed_copy {
            typed_copy::copy_reservation(
                source_layout,
                layout,
                produced.is_none(),
                prior.is_none(),
            )?
        } else {
            copy_allocation_bytes
        };
        // This ledger bounds native payload/scratch, not just logical publication.
        // The byte-patch kernel copies four u32 parameters into command storage.
        let copy_allocation_bytes = copy_allocation_bytes
            .checked_add(
                if region.is_some_and(|(source, destination, len)| {
                    !source.is_multiple_of(4)
                        || !destination.is_multiple_of(4)
                        || !len.is_multiple_of(4)
                }) {
                    16
                } else {
                    0
                },
            )
            .ok_or(GraphicsError::InvalidDescriptor)?;
        self.validate_queued_work_budget(copy_allocation_bytes)?;
        let point = GraphicsQueuePoint {
            queue,
            value: self
                .queue_values
                .get(&queue)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or(GraphicsError::InvalidDescriptor)?,
        };
        // Admission order only; copy commits never assign a presentation serial.
        let serial = self
            .queued_serial
            .checked_add(1)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let output = GraphicsProducedVersion {
            point,
            output: GraphicsProducedOutput::Copy(0),
        };
        let input = |handle, version| -> Result<GraphicsCopyInput, GraphicsError> {
            if let Some(version) = version {
                return Ok(GraphicsCopyInput::Produced(version));
            }
            if let Some(depth) = self.depth_resources.get(&handle) {
                let snapshot = depth
                    .snapshot
                    .as_ref()
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                if let Some(packed) = snapshot.downcast_ref::<GraphicsTypedDepthSnapshot>() {
                    if packed.desc != depth.typed_desc
                        || packed.bytes.len() != depth.typed_desc.byte_len()?
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    return Ok(GraphicsCopyInput::Host(packed.bytes.clone()));
                }
                return Ok(GraphicsCopyInput::HostDepth(snapshot.clone()));
            }
            let resource = &self.resources[&handle];
            let bytes = if resource.guest_backing_addr.is_some() {
                // Payload budget is validated above, before allocating the snapshot.
                // Recording never reads memory; only this admission-time read does.
                let mut bytes = Vec::new();
                bytes
                    .try_reserve_exact(resource.data.len())
                    .map_err(|_| MemoryError::AllocationFailed)?;
                bytes.resize(resource.data.len(), 0);
                Self::read_resource(resource, memory, 0, &mut bytes)?;
                Arc::from(bytes)
            } else {
                Arc::from(resource.data.as_slice())
            };
            Ok(GraphicsCopyInput::Host(bytes))
        };
        let waits = closure
            .iter()
            .map(|handle| self.queued[handle].point)
            .collect();
        let request = if let Some((source_location, destination_location, extent)) = image {
            GraphicsQueuedWorkRequest::ImageCopy(GraphicsQueuedImageCopyRequest {
                generation,
                point,
                waits,
                source_layout,
                destination_layout: layout,
                source: input(source, produced)?,
                destination: input(destination, prior)?,
                source_location,
                destination_location,
                extent,
                output,
            })
        } else if let Some((source_offset, destination_offset, byte_len)) = region {
            GraphicsQueuedWorkRequest::BufferRegion(GraphicsQueuedBufferRegionRequest {
                generation,
                point,
                waits,
                source_byte_len: source_layout.byte_len()?,
                destination_byte_len: layout.byte_len()?,
                source: input(source, produced)?,
                destination: input(destination, prior)?,
                source_offset,
                destination_offset,
                byte_len,
                output,
            })
        } else {
            GraphicsQueuedWorkRequest::Copy(GraphicsQueuedCopyRequest {
                generation,
                point,
                waits,
                layout,
                source: input(source, produced)?,
                output,
            })
        };
        let mut state_definitions = HashMap::new();
        for handle in &writes {
            state_definitions.insert(
                *handle,
                states
                    .get(handle)
                    .copied()
                    .unwrap_or(self.copy_resource_state(*handle)?),
            );
        }
        let ticket = self
            .queued_backend
            .as_ref()
            .expect("copy backend installed")(request)?;
        let references = self.recorded_references(list);
        let handle = self.allocate_handle();
        self.queued.insert(
            handle,
            QueuedReceipt {
                publication_bases: self.publication_bases([destination]),
                color_allocation_bytes: 0,
                depth_allocation_bytes: 0,
                copy_allocation_bytes,
                layouts: HashMap::from([(destination, layout.produced())]),
                commit: Some(QueuedCommit::Copy(CopyCommit {
                    queue,
                    list,
                    destination,
                    layout,
                    states: changed_states,
                })),
                copy_outcome: None,
                point,
                generation,
                serial,
                semantic_dependencies: preparation.semantic_dependencies.clone(),
                ranges: RangeAccesses::whole(&reads, &writes),
                dependencies,
                ticket,
                reads,
                writes,
                state_definitions,
                outputs: HashMap::from([(destination, output)]),
                released: false,
                rejected_dependencies: HashSet::new(),
                receipt: SubmissionReceipt {
                    handle,
                    pending: None,
                    commit: None,
                    references,
                    status: GraphicsSubmissionStatus::Pending,
                    error: None,
                    outcome: None,
                },
            },
        );
        self.queue_values.insert(queue, point.value);
        self.queued_serial = serial;
        self.queued_generation = generation;
        self.queued_failed = false;
        Ok(handle)
    }

    fn commit_copy(
        &mut self,
        commit: CopyCommit,
        layout: GraphicsCopyLayout,
        bytes: Vec<u8>,
    ) -> Result<GraphicsCopyOutcome, GraphicsError> {
        if layout != commit.layout || bytes.len() != commit.layout.byte_len()? {
            return Err(GraphicsError::BackendError(
                "copy output layout or length mismatch".into(),
            ));
        }
        // The admitted layout, not the current backend capability, selects the
        // validation contract. Legacy receipts still require a single mip.
        let allow_mips = matches!(commit.layout, GraphicsCopyLayout::Rgba8MipChain { .. });
        if matches!(layout, GraphicsCopyLayout::TypedTexture(_)) {
            return self.commit_typed_copy(commit, layout, bytes);
        }
        if self.resources.contains_key(&commit.destination)
            && self.copy_layout_for_transfer(
                commit.destination,
                false,
                allow_mips,
                matches!(commit.layout, GraphicsCopyLayout::Rgba8Array { .. }),
            )? != layout
        {
            return Err(GraphicsError::BackendError(
                "copy destination layout changed".into(),
            ));
        }
        // No presentation lock, frame update, or presentation serial. All fallible
        // validation precedes host bytes and planned-state publication.
        if let Some(destination) = self.resources.get_mut(&commit.destination) {
            destination.data.copy_from_slice(&bytes);
        }
        for (handle, state) in commit.states {
            if let Some(resource) = self.resources.get_mut(&handle) {
                resource.state = state;
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

    fn queued_accesses(&self, list: GraphicsHandle) -> QueuedAccesses {
        let mut reads = HashSet::new();
        let mut writes = HashSet::new();
        let mut sampled = [None; 2];
        let mut vertex = None;
        let mut colors = None;
        let mut index = None;
        let mut draws = Vec::new();
        for command in &self.command_lists[&list].commands {
            let indexed = match command {
                GraphicsCommand::SetStorageView(view) => {
                    if let Some(handle) = view {
                        let root = self.storage_views[handle].root;
                        reads.insert(root);
                        writes.insert(root);
                    }
                    continue;
                }
                GraphicsCommand::SetSampledArrayRanges { ranges, .. } => {
                    sampled =
                        ranges.map(|handle| handle.map(|h| self.array_range_views[&h].backing));
                    continue;
                }
                GraphicsCommand::SetSampledViews { views, .. } => {
                    sampled = views.map(|handle| handle.map(|h| self.texture_views[&h].backing));
                    continue;
                }
                GraphicsCommand::SetSampledBindings { textures, .. } => {
                    sampled = *textures;
                    continue;
                }
                GraphicsCommand::SetSampledTexture { texture, .. } => {
                    sampled[0] = *texture;
                    continue;
                }
                GraphicsCommand::SetVertexBuffer { resource, .. } => {
                    vertex = Some(*resource);
                    continue;
                }
                GraphicsCommand::SetVertexColors { resource, .. } => {
                    colors = Some(*resource);
                    continue;
                }
                GraphicsCommand::SetIndexBuffer { resource, .. } => {
                    index = Some(*resource);
                    continue;
                }
                GraphicsCommand::ResourceBarrier { resource, .. } => {
                    writes.insert(*resource);
                    continue;
                }
                GraphicsCommand::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    ..
                } if *vertex_count_per_instance != 0 && *instance_count != 0 => false,
                GraphicsCommand::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    ..
                } if *index_count_per_instance != 0 && *instance_count != 0 => true,
                _ => continue,
            };
            reads.extend(vertex);
            reads.extend(colors);
            if indexed {
                reads.extend(index);
            }
            reads.extend(sampled.iter().flatten().copied());
            draws.push(sampled);
        }
        (reads, writes, draws)
    }

    fn finish_queued_submission(
        &mut self,
        handle: GraphicsHandle,
        wait: bool,
    ) -> Result<GraphicsSubmissionStatus, GraphicsError> {
        if self.queued.get(&handle).is_none_or(|node| node.released) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        self.collect_queued_node(handle, wait)
    }

    fn collect_queued_node(
        &mut self,
        handle: GraphicsHandle,
        wait: bool,
    ) -> Result<GraphicsSubmissionStatus, GraphicsError> {
        if self.queued[&handle].receipt.status != GraphicsSubmissionStatus::Pending {
            return Ok(self.queued[&handle].receipt.status);
        }
        let dependencies = self.queued[&handle].dependencies.clone();
        let mut failed = !self.queued[&handle].rejected_dependencies.is_empty();
        for dependency in dependencies {
            match self.collect_queued_node(dependency, wait)? {
                GraphicsSubmissionStatus::Pending => return Ok(GraphicsSubmissionStatus::Pending),
                GraphicsSubmissionStatus::Failed => {
                    let point = self.queued[&dependency].point;
                    let node = self.queued.get_mut(&handle).unwrap();
                    if node.rejected_dependencies.insert(point) {
                        node.ticket.reject_dependency(point);
                    }
                    failed = true;
                }
                GraphicsSubmissionStatus::Complete => {}
            }
        }
        let node = self.queued.get_mut(&handle).unwrap();
        let result = if wait {
            Some(node.ticket.wait())
        } else {
            node.ticket.poll()
        };
        let Some(result) = result else {
            return Ok(GraphicsSubmissionStatus::Pending);
        };
        let commit = node.commit.take().expect("pending queued commit");
        let publication_valid = self.validate_publication_commit(handle);
        let result = if failed {
            Err(GraphicsError::BackendError(
                "submission dependency failed".into(),
            ))
        } else {
            publication_valid
                .and(result)
                .and_then(|output| match (commit, output) {
                    (
                        QueuedCommit::Attachment(commit),
                        GraphicsQueuedWorkOutput::AttachmentTransaction(output),
                    ) => self
                        .commit_attachment_transaction(commit, output)
                        .map(QueuedOutcome::Render),
                    (QueuedCommit::Render(commit), GraphicsQueuedWorkOutput::Render(output)) => {
                        self.commit_submission(commit, output)
                            .map(QueuedOutcome::Render)
                    }
                    (
                        QueuedCommit::Copy(commit),
                        GraphicsQueuedWorkOutput::Copy { layout, bytes },
                    ) => self
                        .commit_copy(commit, layout, bytes)
                        .map(QueuedOutcome::Copy),
                    _ => Err(GraphicsError::BackendError(
                        "submission output kind mismatch".into(),
                    )),
                })
        };
        if result.is_ok() {
            let node = &self.queued[&handle];
            for (root, version) in &node.outputs {
                if self.resources.contains_key(root) || self.depth_resources.contains_key(root) {
                    let publication = self.root_publications.entry(*root).or_default();
                    publication.version = Some((node.generation, *version));
                }
            }
        }
        let node = self.queued.get_mut(&handle).unwrap();
        node.receipt
            .references
            .retain(|handle| node.outputs.contains_key(handle));
        match result {
            Ok(outcome) => {
                node.receipt.status = GraphicsSubmissionStatus::Complete;
                match outcome {
                    QueuedOutcome::Render(outcome) => node.receipt.outcome = Some(outcome),
                    QueuedOutcome::Copy(outcome) => node.copy_outcome = Some(outcome),
                }
            }
            Err(error) => {
                node.receipt.status = GraphicsSubmissionStatus::Failed;
                node.receipt.error = Some(error);
                self.queued_failed = true;
            }
        }
        let status = node.receipt.status;
        let point = node.point;
        let generation = node.generation;
        if status == GraphicsSubmissionStatus::Failed {
            for dependent in self
                .queued
                .values_mut()
                .filter(|other| other.generation == generation)
            {
                if dependent.receipt.status == GraphicsSubmissionStatus::Pending
                    && dependent.rejected_dependencies.insert(point)
                {
                    dependent.ticket.reject_dependency(point);
                }
            }
        }
        self.collect_retired();
        Ok(status)
    }

    fn collect_retired(&mut self) {
        let pinned: HashSet<_> = self
            .queued
            .values()
            .flat_map(|node| node.receipt.references.iter().copied())
            .chain(
                self.submission
                    .iter()
                    .filter(|receipt| receipt.status == GraphicsSubmissionStatus::Pending)
                    .flat_map(|receipt| receipt.references.iter().copied()),
            )
            .collect();
        self.retired_resources
            .retain(|handle, _| pinned.contains(handle));
        self.retired_depth
            .retain(|handle, _| pinned.contains(handle));
        self.retired_pipelines
            .retain(|handle, _| pinned.contains(handle));
        self.retired_views
            .retain(|handle, _| pinned.contains(handle));
        self.retired_array_ranges
            .retain(|handle, _| pinned.contains(handle));
        self.retired_writable_views
            .retain(|handle, _| pinned.contains(handle));
        self.retired_storage_views
            .retain(|handle, _| pinned.contains(handle));
    }

    fn prune_queued(&mut self) {
        loop {
            let removable = self.queued.iter().find_map(|(handle, node)| {
                (node.released
                    && !self
                        .queued
                        .values()
                        .any(|other| other.dependencies.contains(handle)))
                .then_some(*handle)
            });
            let Some(handle) = removable else {
                break;
            };
            self.queued.remove(&handle); // Terminal native ticket Drop releases versions.
        }
        self.collect_retired();
    }

    /// Install a queued backend that loads immutable producer colors into fresh
    /// native destinations after GPU dependency waits. Includes uniform/view/typed
    /// input support; no host wait/readback may substitute for a Produced input.
    pub fn set_metal_produced_color_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_uniform_queued_backend(capabilities, renderer);
        self.produced_color_backend = true;
    }

    /// Enable produced color and depth loads into fresh native destinations.
    /// Older installers revoke produced-depth support.
    pub fn set_metal_produced_attachment_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_produced_color_queued_backend(capabilities, renderer);
        self.produced_depth_backend = true;
    }

    /// Install typed render/copy work with all produced-attachment capabilities.
    /// Native produced copies must not be reconstructed through host readback.
    pub fn set_metal_copy_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_produced_attachment_queued_backend(capabilities, |_| {
            Err(GraphicsError::NativeMetalRequired)
        });
        self.queued_backend = Some(Arc::new(renderer));
        self.copy_backend = true;
    }

    /// Install typed work supporting aligned buffer regions as well as whole copies.
    pub fn set_metal_buffer_region_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_copy_queued_backend(capabilities, renderer);
        self.buffer_region_backend = true;
    }

    /// Install typed image transfers in addition to buffer regions and whole copies.
    pub fn set_metal_image_copy_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_buffer_region_queued_backend(capabilities, renderer);
        self.image_copy_backend = true;
    }

    /// Install arbitrary-byte buffer regions plus image and whole-copy support.
    /// Older installers retain their aligned-region contract and revoke this capability.
    pub fn set_metal_unaligned_buffer_region_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_image_copy_queued_backend(capabilities, renderer);
        self.unaligned_buffer_region_backend = true;
    }

    /// Install bounded dependency graphs, including multiple pending nodes per queue.
    pub fn set_metal_graph_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_unaligned_buffer_region_queued_backend(capabilities, renderer);
        self.graph_backend = true;
    }

    /// Opt in to transfer-only arrays, preserving full backing and explicit array kind.
    pub fn set_metal_array_view_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_array_copy_queued_backend(capabilities, renderer);
        self.array_view_backend = true;
    }

    pub fn set_metal_array_copy_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_mip_copy_queued_backend(capabilities, renderer);
        self.array_copy_backend = true;
    }

    /// Opt in to full-chain RGBA8 copies with explicit mip endpoints.
    pub fn set_metal_mip_copy_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_geometry_graph_backend(capabilities, renderer);
        self.mip_copy_backend = true;
    }

    /// Opt in to exact produced vertex/color/index inputs gathered on the GPU.
    pub fn set_metal_geometry_graph_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedWorkRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_graph_queued_backend(capabilities, renderer);
        self.geometry_backend = true;
    }

    /// Retained-node and explicit-wait limit of the installed backend contract.
    pub fn queued_submission_capacity(&self) -> usize {
        if self.graph_backend {
            MAX_QUEUED_GRAPH_NODES
        } else {
            2
        }
    }

    fn queued_dependency_closure(
        &self,
        roots: &[GraphicsHandle],
    ) -> Result<Vec<GraphicsHandle>, GraphicsError> {
        let mut visited = HashSet::new();
        let mut pending = roots.to_vec();
        while let Some(handle) = pending.pop() {
            if !visited.insert(handle) {
                continue;
            }
            if visited.len() > MAX_QUEUED_GRAPH_NODES {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let node = self
                .queued
                .get(&handle)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            for dependency in &node.dependencies {
                let parent = self
                    .queued
                    .get(dependency)
                    .ok_or(GraphicsError::InvalidResourceHandle)?;
                if parent.serial >= node.serial {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                pending.push(*dependency);
            }
        }
        let mut ordered: Vec<_> = visited.into_iter().collect();
        ordered.sort_by_key(|handle| self.queued[handle].serial);
        Ok(ordered)
    }

    fn queued_preparation(
        &self,
        closure: &[GraphicsHandle],
        generation: u64,
    ) -> Result<QueuedPreparation, GraphicsError> {
        self.queued_preparation_from(closure, generation, true)
    }

    fn queued_preparation_from(
        &self,
        closure: &[GraphicsHandle],
        generation: u64,
        semantic: bool,
    ) -> Result<QueuedPreparation, GraphicsError> {
        let mut preparation = QueuedPreparation::default();
        let mut ancestors = HashMap::new();
        let mut state_owners = HashMap::<GraphicsHandle, Vec<GraphicsHandle>>::new();
        let mut output_owners = HashMap::<GraphicsHandle, Vec<GraphicsHandle>>::new();
        for handle in closure {
            let node = &self.queued[handle];
            if node.generation != generation
                || node.receipt.status == GraphicsSubmissionStatus::Failed
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            ancestors.insert(
                *handle,
                if semantic {
                    self.semantic_closure(&node.semantic_dependencies)?
                } else {
                    self.queued_dependency_closure(&node.dependencies)?
                },
            );
            // Retired roots remain in dependency/failure ordering, but their
            // handles can no longer contribute public state or produced inputs.
            for resource in node.state_definitions.keys() {
                if self.resources.contains_key(resource)
                    || self.depth_resources.contains_key(resource)
                {
                    state_owners.entry(*resource).or_default().push(*handle);
                }
            }
            for resource in node.outputs.keys() {
                if self.resources.contains_key(resource)
                    || self.depth_resources.contains_key(resource)
                {
                    output_owners.entry(*resource).or_default().push(*handle);
                }
            }
        }
        let maximal = |owners: &[GraphicsHandle]| -> Result<GraphicsHandle, GraphicsError> {
            let mut maxima = owners.iter().copied().filter(|owner| {
                !owners
                    .iter()
                    .any(|other| other != owner && ancestors[other].contains(owner))
            });
            let selected = maxima.next().ok_or(GraphicsError::InvalidDescriptor)?;
            if maxima.next().is_some() {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            Ok(selected)
        };
        for (resource, owners) in state_owners {
            let owner = maximal(&owners)?;
            preparation
                .states
                .insert(resource, self.queued[&owner].state_definitions[&resource]);
        }
        for (resource, owners) in output_owners {
            let owner = maximal(&owners)?;
            let node = &self.queued[&owner];
            if node
                .publication_bases
                .get(&resource)
                .is_none_or(|p| p.host_epoch != self.publication(resource).host_epoch)
            {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            let layout = node
                .layouts
                .get(&resource)
                .ok_or(GraphicsError::InvalidDescriptor)?;
            preparation
                .produced
                .insert(resource, node.outputs[&resource]);
            preparation.layouts.insert(resource, *layout);
        }
        Ok(preparation)
    }

    /// Copy completion is distinct from rendering and never fabricates a frame.
    pub fn submission_copy_outcome(
        &self,
        handle: GraphicsHandle,
    ) -> Result<Option<&GraphicsCopyOutcome>, GraphicsError> {
        self.queued
            .get(&handle)
            .filter(|node| !node.released)
            .map(|node| node.copy_outcome.as_ref())
            .ok_or(GraphicsError::InvalidResourceHandle)
    }

    pub fn queued_copy_allocation_bytes(&self) -> usize {
        self.queued
            .values()
            .map(|node| node.copy_allocation_bytes)
            .sum()
    }

    fn validate_queued_work_budget(&self, additional: usize) -> Result<(), GraphicsError> {
        // Shared conservative payload ceiling, separate from logical backing budget.
        let current = self.queued.values().try_fold(0usize, |sum, node| {
            sum.checked_add(node.color_allocation_bytes)?
                .checked_add(node.depth_allocation_bytes)?
                .checked_add(node.copy_allocation_bytes)
        });
        if current
            .and_then(|sum| sum.checked_add(additional))
            .is_none_or(|sum| sum > MAX_CONTEXT_RESOURCE_BYTES)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        Ok(())
    }

    /// Depth reservation retained until node pruning; native external clones
    /// retain independent surface leases on the worker.
    pub fn queued_depth_allocation_bytes(&self) -> usize {
        self.queued
            .values()
            .map(|node| node.depth_allocation_bytes)
            .sum()
    }

    /// Reserved queued color bytes, including terminal ancestor versions retained
    /// by descendants. Refunded only when the owning node/ticket is pruned.
    /// This excludes logical backings, depth, sampled uploads and vertex storage.
    pub fn queued_color_allocation_bytes(&self) -> usize {
        self.queued
            .values()
            .map(|node| node.color_allocation_bytes)
            .sum()
    }

    pub fn set_metal_array_view_attachment_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_uniform_attachment_backend(capabilities, renderer);
        self.array_view_backend = true;
    }

    pub fn set_metal_uniform_attachment_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_typed_attachment_backend(capabilities, renderer);
        self.uniform_backend = true;
    }

    pub fn set_metal_array_view_async_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_uniform_async_backend(capabilities, renderer);
        self.array_view_backend = true;
    }

    pub fn set_metal_uniform_async_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_typed_async_backend(capabilities, renderer);
        self.uniform_backend = true;
    }

    pub fn set_metal_uniform_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_typed_queued_backend(capabilities, renderer);
        self.uniform_backend = true;
    }

    pub fn set_metal_typed_attachment_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_view_attachment_backend(renderer);
        self.texture_capabilities = capabilities;
    }

    pub fn set_metal_typed_async_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_view_async_backend(renderer);
        self.texture_capabilities = capabilities;
    }

    pub fn set_metal_typed_queued_backend<F>(
        &mut self,
        capabilities: GraphicsTextureCapabilities,
        renderer: F,
    ) where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_view_queued_backend(renderer);
        self.texture_capabilities = capabilities;
    }

    pub fn set_metal_view_attachment_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_multislot_attachment_backend(renderer);
        self.view_backend = true;
    }

    pub fn set_metal_view_async_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_multislot_async_backend(renderer);
        self.view_backend = true;
    }

    pub fn set_metal_view_queued_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                GraphicsQueuedSubmissionRequest,
            ) -> Result<Box<dyn GraphicsQueuedPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_queued_backend(renderer);
        self.view_backend = true;
    }

    /// Install a submitter that acknowledges native commit, never completion.
    /// An error must mean no native work still references the rejected request.
    pub fn set_metal_async_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.queued_backend = None;
        self.texture_capabilities = GraphicsTextureCapabilities::default();
        self.uniform_backend = false;
        self.produced_color_backend = false;
        self.produced_depth_backend = false;
        self.copy_backend = false;
        self.buffer_region_backend = false;
        self.image_copy_backend = false;
        self.mip_copy_backend = false;
        self.array_copy_backend = false;
        self.typed_copy_capabilities = GraphicsTypedCopyCapabilities::default();
        self.resource_roles = GraphicsResourceRoleCapabilities::default();
        self.attachment_transactions = false;
        self.unaligned_buffer_region_backend = false;
        self.graph_backend = false;
        self.geometry_backend = false;
        self.view_backend = false;
        self.array_view_backend = false;
        self.shader_array_views = false;
        self.shader_storage_views = false;
        self.multislot_backend = false;
        self.backend = None;
        self.pass_backend = None;
        self.async_backend = Some(Arc::new(renderer));
        self.backend_id = GraphicsBackendId::MetalNative;
    }

    /// Install a submitter supporting both independent texture and sampler slots.
    pub fn set_metal_multislot_async_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                GraphicsSubmissionRequest,
            ) -> Result<Box<dyn GraphicsPendingSubmission>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_async_backend(renderer);
        self.multislot_backend = true;
    }

    /// Install an attachment renderer supporting both sampled slots.
    pub fn set_metal_multislot_attachment_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_attachment_backend(renderer);
        self.multislot_backend = true;
    }

    fn has_pending_submission(&self) -> bool {
        self.queued
            .values()
            .any(|node| node.receipt.status == GraphicsSubmissionStatus::Pending)
            || self
                .submission
                .as_ref()
                .is_some_and(|receipt| receipt.status == GraphicsSubmissionStatus::Pending)
    }

    fn is_pinned(&self, handle: GraphicsHandle) -> bool {
        self.queued
            .values()
            .any(|node| node.receipt.references.contains(&handle))
            || self.submission.as_ref().is_some_and(|receipt| {
                receipt.status == GraphicsSubmissionStatus::Pending
                    && receipt.references.contains(&handle)
            })
    }

    fn retired_bytes(&self) -> usize {
        self.retired_resources
            .values()
            .map(|resource| resource.data.len())
            .sum::<usize>()
            + self
                .retired_depth
                .values()
                .map(|depth| depth.typed_desc.byte_len().expect("validated depth layout"))
                .sum::<usize>()
    }

    fn recorded_references(&self, list: GraphicsHandle) -> HashSet<GraphicsHandle> {
        let mut references = HashSet::new();
        for command in &self.command_lists[&list].commands {
            match command {
                GraphicsCommand::CopyResource {
                    destination,
                    source,
                }
                | GraphicsCommand::CopyBufferRegion {
                    destination,
                    source,
                    ..
                }
                | GraphicsCommand::CopyImage {
                    destination,
                    source,
                    ..
                } => {
                    references.insert(*destination);
                    references.insert(*source);
                }
                GraphicsCommand::SetVertexBuffer { resource, .. }
                | GraphicsCommand::SetVertexColors { resource, .. }
                | GraphicsCommand::SetIndexBuffer { resource, .. }
                | GraphicsCommand::ResourceBarrier { resource, .. } => {
                    references.insert(*resource);
                }
                GraphicsCommand::SetGraphicsPipeline(handle)
                | GraphicsCommand::SetSampledTexture {
                    texture: handle, ..
                }
                | GraphicsCommand::SetDepthTarget { target: handle, .. }
                | GraphicsCommand::SetRenderTargets {
                    color_target: handle,
                    ..
                } => {
                    references.extend(handle.iter().copied());
                }
                GraphicsCommand::SetStorageView(Some(handle)) => {
                    references.insert(*handle);
                    references.insert(self.storage_views[handle].root);
                }
                GraphicsCommand::SetWritableColorTargets { views, .. } => {
                    for handle in views {
                        references.insert(*handle);
                        references.insert(self.writable_views[handle].root);
                    }
                }
                GraphicsCommand::SetWritableDepthTarget {
                    view: Some(handle), ..
                } => {
                    references.insert(*handle);
                    references.insert(self.writable_views[handle].root);
                }
                GraphicsCommand::SetSampledArrayRanges { ranges, .. } => {
                    for handle in ranges.iter().flatten() {
                        references.insert(*handle);
                        references.insert(self.array_range_views[handle].backing);
                    }
                }
                GraphicsCommand::SetSampledViews { views, .. } => {
                    for handle in views.iter().flatten() {
                        references.insert(*handle);
                        references.insert(self.texture_views[handle].backing);
                    }
                }
                GraphicsCommand::SetSampledBindings { textures, .. } => {
                    references.extend(textures.iter().flatten().copied());
                }
                GraphicsCommand::SetColorTargets { targets, .. } => {
                    references.extend(targets.iter().copied());
                }
                _ => {}
            }
        }
        references
    }

    /// Validate and snapshot now; one unreleased receipt is allowed per context.
    pub fn submit_command_list(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        memory: &GuestMemory,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if self.queued_backend.is_some() {
            return self.submit_command_list_after(queue, list, &[], memory);
        }
        if !self.queued.is_empty()
            || self.submission.is_some()
            || self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let renderer = self
            .async_backend
            .clone()
            .ok_or(GraphicsError::NativeMetalRequired)?;
        let prepared = self.prepare_submission(
            queue,
            list,
            memory,
            true,
            GraphicsBackendId::MetalNative,
            None,
        )?;
        let references = self.recorded_references(list);
        let pending = renderer(prepared.request)?;
        let handle = self.allocate_handle();
        self.submission = Some(SubmissionReceipt {
            handle,
            pending: Some(pending),
            commit: Some(prepared.commit),
            references,
            status: GraphicsSubmissionStatus::Pending,
            error: None,
            outcome: None,
        });
        Ok(handle)
    }

    pub fn poll_submission(
        &mut self,
        handle: GraphicsHandle,
    ) -> Result<GraphicsSubmissionStatus, GraphicsError> {
        if self.queued.contains_key(&handle) {
            return self.finish_queued_submission(handle, false);
        }
        self.finish_submission(handle, false)
    }

    pub fn wait_submission(
        &mut self,
        handle: GraphicsHandle,
    ) -> Result<GraphicsSubmissionStatus, GraphicsError> {
        if self.queued.contains_key(&handle) {
            return self.finish_queued_submission(handle, true);
        }
        self.finish_submission(handle, true)
    }

    pub fn submission_error(
        &self,
        handle: GraphicsHandle,
    ) -> Result<Option<&GraphicsError>, GraphicsError> {
        if let Some(node) = self.queued.get(&handle).filter(|node| !node.released) {
            return Ok(node.receipt.error.as_ref());
        }
        let receipt = self
            .submission
            .as_ref()
            .filter(|receipt| receipt.handle == handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        Ok(receipt.error.as_ref())
    }

    pub fn submission_outcome(
        &self,
        handle: GraphicsHandle,
    ) -> Result<Option<&ExecutionOutcome>, GraphicsError> {
        if let Some(node) = self.queued.get(&handle).filter(|node| !node.released) {
            return Ok(node.receipt.outcome.as_ref());
        }
        let receipt = self
            .submission
            .as_ref()
            .filter(|receipt| receipt.handle == handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        Ok(receipt.outcome.as_ref())
    }

    fn finish_submission(
        &mut self,
        handle: GraphicsHandle,
        wait: bool,
    ) -> Result<GraphicsSubmissionStatus, GraphicsError> {
        let receipt = self
            .submission
            .as_mut()
            .filter(|receipt| receipt.handle == handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if receipt.status != GraphicsSubmissionStatus::Pending {
            return Ok(receipt.status);
        }
        let pending = receipt.pending.as_mut().expect("pending receipt");
        let result = if wait {
            Some(pending.wait())
        } else {
            pending.poll()
        };
        let Some(result) = result else {
            return Ok(GraphicsSubmissionStatus::Pending);
        };
        // Terminal means drained. Keep the receipt/pins installed through commit.
        let commit = receipt.commit.take().expect("pending commit");
        let result = result.and_then(|output| self.commit_submission(commit, output));
        let receipt = self.submission.as_mut().expect("receipt retained");
        receipt.pending = None;
        receipt.references.clear();
        receipt.status = if result.is_ok() {
            GraphicsSubmissionStatus::Complete
        } else {
            GraphicsSubmissionStatus::Failed
        };
        match result {
            Ok(outcome) => receipt.outcome = Some(outcome),
            Err(error) => receipt.error = Some(error),
        }
        self.retired_resources.clear();
        self.retired_depth.clear();
        self.retired_pipelines.clear();
        self.retired_views.clear();
        self.retired_array_ranges.clear();
        Ok(receipt.status)
    }

    pub fn release_submission(&mut self, handle: GraphicsHandle) -> Result<(), GraphicsError> {
        if let Some(node) = self.queued.get_mut(&handle) {
            if node.released || node.receipt.status == GraphicsSubmissionStatus::Pending {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            node.released = true;
            self.prune_queued();
            return Ok(());
        }
        let receipt = self
            .submission
            .as_ref()
            .filter(|receipt| receipt.handle == handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if receipt.status == GraphicsSubmissionStatus::Pending {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        self.submission = None;
        Ok(())
    }

    /// Install a native Metal backend adapter.
    pub fn set_metal_backend<F>(&mut self, renderer: F)
    where
        F: Fn(&[[f32; 4]], (usize, usize), [u8; 4]) -> Result<Vec<u8>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.queued_backend = None;
        self.texture_capabilities = GraphicsTextureCapabilities::default();
        self.uniform_backend = false;
        self.produced_color_backend = false;
        self.produced_depth_backend = false;
        self.copy_backend = false;
        self.buffer_region_backend = false;
        self.image_copy_backend = false;
        self.mip_copy_backend = false;
        self.array_copy_backend = false;
        self.typed_copy_capabilities = GraphicsTypedCopyCapabilities::default();
        self.resource_roles = GraphicsResourceRoleCapabilities::default();
        self.attachment_transactions = false;
        self.unaligned_buffer_region_backend = false;
        self.graph_backend = false;
        self.geometry_backend = false;
        self.view_backend = false;
        self.array_view_backend = false;
        self.shader_array_views = false;
        self.shader_storage_views = false;
        self.multislot_backend = false;
        self.async_backend = None;
        self.pass_backend = None;
        self.backend = Some(Arc::new(renderer));
        self.backend_id = GraphicsBackendId::MetalNative;
    }

    /// Install an ordered native pass adapter, replacing any legacy adapter.
    pub fn set_metal_pass_backend<F>(&mut self, renderer: F)
    where
        F: Fn(&[GraphicsDraw], (usize, usize), [u8; 4]) -> Result<Vec<u8>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_pass_with_depth_backend(move |draws, extent, clear, depth_clear| {
            if depth_clear != 1.0 {
                return Err(GraphicsError::InvalidDescriptor);
            }
            renderer(draws, extent, clear)
        });
    }

    /// Install an ordered adapter receiving the pass depth clear (finite 0..=1).
    pub fn set_metal_pass_with_depth_backend<F>(&mut self, renderer: F)
    where
        F: Fn(&[GraphicsDraw], (usize, usize), [u8; 4], f32) -> Result<Vec<u8>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_targets_backend(move |draws, extent, clears, depth| {
            if clears.len() != 1 {
                return Err(GraphicsError::InvalidDescriptor);
            }
            renderer(draws, extent, clears[0], depth).map(|output| vec![output])
        });
    }

    /// Install an ordered adapter supporting one through eight color outputs.
    pub fn set_metal_targets_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[[u8; 4]],
                f32,
            ) -> Result<Vec<Vec<u8>>, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_stateful_backend(move |draws, extent, clears, clear_depth, depth| {
            if depth.is_some()
                || draws
                    .iter()
                    .any(|draw| draw.pipeline.is_some() || draw.sampled.is_some())
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            renderer(draws, extent, clears, clear_depth).map(|colors| GraphicsPassOutput {
                color_layouts: None,
                colors,
                depth: None,
            })
        });
    }

    /// Install a renderer that stages depth snapshots alongside all color outputs.
    pub fn set_metal_stateful_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[[u8; 4]],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.set_metal_attachment_backend(move |draws, extent, bindings, clear_depth, depth| {
            if bindings.iter().any(|binding| binding.previous.is_some()) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let clears: Vec<_> = bindings.iter().map(|binding| binding.clear).collect();
            renderer(draws, extent, &clears, clear_depth, depth)
        });
    }

    /// Install a renderer supporting immutable color and depth load snapshots.
    pub fn set_metal_attachment_backend<F>(&mut self, renderer: F)
    where
        F: Fn(
                &[GraphicsDraw],
                (usize, usize),
                &[GraphicsColorBinding],
                f32,
                Option<GraphicsDepthBinding>,
            ) -> Result<GraphicsPassOutput, GraphicsError>
            + Send
            + Sync
            + 'static,
    {
        self.queued_backend = None;
        self.texture_capabilities = GraphicsTextureCapabilities::default();
        self.uniform_backend = false;
        self.produced_color_backend = false;
        self.produced_depth_backend = false;
        self.copy_backend = false;
        self.buffer_region_backend = false;
        self.image_copy_backend = false;
        self.mip_copy_backend = false;
        self.array_copy_backend = false;
        self.typed_copy_capabilities = GraphicsTypedCopyCapabilities::default();
        self.resource_roles = GraphicsResourceRoleCapabilities::default();
        self.attachment_transactions = false;
        self.unaligned_buffer_region_backend = false;
        self.graph_backend = false;
        self.geometry_backend = false;
        self.view_backend = false;
        self.array_view_backend = false;
        self.shader_array_views = false;
        self.shader_storage_views = false;
        self.multislot_backend = false;
        self.backend = None;
        self.async_backend = None;
        self.pass_backend = Some(Arc::new(renderer));
        self.backend_id = GraphicsBackendId::MetalNative;
    }

    /// Explicitly configure context to require native Metal without an attached backend.
    /// Useful for verifying that missing/unsupported Metal devices correctly fail execution.
    pub fn set_require_metal_without_backend(&mut self) {
        self.queued_backend = None;
        self.texture_capabilities = GraphicsTextureCapabilities::default();
        self.uniform_backend = false;
        self.produced_color_backend = false;
        self.produced_depth_backend = false;
        self.copy_backend = false;
        self.buffer_region_backend = false;
        self.image_copy_backend = false;
        self.mip_copy_backend = false;
        self.array_copy_backend = false;
        self.typed_copy_capabilities = GraphicsTypedCopyCapabilities::default();
        self.resource_roles = GraphicsResourceRoleCapabilities::default();
        self.attachment_transactions = false;
        self.unaligned_buffer_region_backend = false;
        self.graph_backend = false;
        self.geometry_backend = false;
        self.view_backend = false;
        self.array_view_backend = false;
        self.shader_array_views = false;
        self.shader_storage_views = false;
        self.multislot_backend = false;
        self.backend = None;
        self.async_backend = None;
        self.pass_backend = None;
        self.backend_id = GraphicsBackendId::MetalNative;
    }

    /// Retrieve the currently configured backend ID.
    pub fn current_backend_id(&self) -> GraphicsBackendId {
        self.backend_id
    }

    /// Release only unpublished objects created by a failed legacy submission.
    /// The caller must hold exclusive context access and supply only fresh handles;
    /// existing command lists must never reference these objects.
    pub(crate) fn discard_unpublished_guest_objects(&mut self, handles: &[GraphicsHandle]) {
        for handle in handles {
            self.command_lists.remove(handle);
        }
        for handle in handles {
            self.queues.remove(handle);
            self.allocators.remove(handle);
            self.resources.remove(handle);
            self.devices.remove(handle);
        }
    }

    pub(crate) fn guest_object_count(&self) -> usize {
        self.devices.len()
            + self.queues.len()
            + self.allocators.len()
            + self.command_lists.len()
            + self.resources.len()
            + self.depth_resources.len()
            + self.pipelines.len()
            + self.retired_resources.len()
            + self.retired_depth.len()
            + self.retired_pipelines.len()
            + usize::from(self.submission.is_some())
            + self.queued.len()
            + self.samplers.len()
            + self.texture_views.len()
            + self.array_range_views.len()
            + self.writable_views.len()
            + self.retired_writable_views.len()
            + self.storage_views.len()
            + self.retired_storage_views.len()
            + self.retired_views.len()
            + self.retired_array_ranges.len()
    }

    pub(crate) fn guest_command_count(&self, list: GraphicsHandle) -> Option<usize> {
        self.command_lists
            .get(&list)
            .map(|commands| commands.commands.len())
    }

    fn allocate_handle(&mut self) -> GraphicsHandle {
        let id = self.next_id;
        self.next_id += 1;
        GraphicsHandle { id, generation: 1 }
    }

    /// Create guest graphics device (`D3D12CreateDevice` / test device).
    pub fn create_device(&mut self) -> Result<GraphicsHandle, GraphicsError> {
        let handle = self.allocate_handle();
        self.devices.insert(handle, true);
        Ok(handle)
    }

    /// Create command queue (`ID3D12Device::CreateCommandQueue`).
    pub fn create_command_queue(
        &mut self,
        device: GraphicsHandle,
        list_type: CommandListType,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        let handle = self.allocate_handle();
        self.queues.insert(
            handle,
            GraphicsCommandQueue {
                handle,
                list_type,
                submitted_lists: Vec::new(),
            },
        );
        Ok(handle)
    }

    /// Create command allocator (`ID3D12Device::CreateCommandAllocator`).
    pub fn create_command_allocator(
        &mut self,
        device: GraphicsHandle,
        list_type: CommandListType,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        let handle = self.allocate_handle();
        self.allocators.insert(
            handle,
            GraphicsCommandAllocator {
                handle,
                list_type,
                is_reset: true,
            },
        );
        Ok(handle)
    }

    /// Create command list (`ID3D12Device::CreateCommandList`).
    pub fn create_command_list(
        &mut self,
        device: GraphicsHandle,
        allocator: GraphicsHandle,
        list_type: CommandListType,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        let alloc = self
            .allocators
            .get(&allocator)
            .ok_or(GraphicsError::InvalidAllocatorHandle)?;
        if alloc.list_type != list_type {
            return Err(GraphicsError::InvalidAllocatorHandle);
        }

        let handle = self.allocate_handle();
        self.command_lists.insert(
            handle,
            GraphicsCommandList {
                handle,
                list_type,
                is_recording: true,
                commands: Vec::new(),
            },
        );
        Ok(handle)
    }

    /// Create committed resource (Buffer or Texture).
    pub fn create_committed_resource(
        &mut self,
        device: GraphicsHandle,
        desc: ResourceDesc,
        initial_state: ResourceStates,
        guest_addr: Option<u64>,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let texture_format = (desc.dimension == ResourceDimension::Texture2D && desc.format_rgba8)
            .then_some(TextureFormat::Rgba8Unorm);
        self.create_resource_storage(device, desc, initial_state, guest_addr, texture_format)
    }

    pub fn create_texture_resource(
        &mut self,
        device: GraphicsHandle,
        texture: GraphicsTextureDesc,
        initial_state: ResourceStates,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if texture.width > MAX_RENDER_DIMENSION
            || texture.height > MAX_RENDER_DIMENSION
            || (texture.format != TextureFormat::Rgba8Unorm
                && ![
                    ResourceStates::COMMON,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                    ResourceStates::PIXEL_SHADER_RESOURCE,
                ]
                .contains(&initial_state))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: texture
                .width
                .try_into()
                .map_err(|_| GraphicsError::InvalidDescriptor)?,
            height: texture
                .height
                .try_into()
                .map_err(|_| GraphicsError::InvalidDescriptor)?,
            depth_or_array_size: 1,
            mip_levels: texture
                .mip_levels
                .try_into()
                .map_err(|_| GraphicsError::InvalidDescriptor)?,
            format_rgba8: texture.format == TextureFormat::Rgba8Unorm,
        };
        self.create_resource_storage(device, desc, initial_state, None, Some(texture.format))
    }

    fn create_resource_storage(
        &mut self,
        device: GraphicsHandle,
        desc: ResourceDesc,
        initial_state: ResourceStates,
        guest_addr: Option<u64>,
        texture_format: Option<TextureFormat>,
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.create_resource_storage_typed(
            device,
            desc,
            initial_state,
            guest_addr,
            texture_format,
            false,
        )
    }

    /// Create transfer-only RGBA8 array storage, including explicit one-layer arrays.
    pub fn create_texture_array_resource(
        &mut self,
        device: GraphicsHandle,
        texture: GraphicsTextureArrayDesc,
        initial_state: ResourceStates,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if texture.width > MAX_RENDER_DIMENSION
            || texture.height > MAX_RENDER_DIMENSION
            || texture.array_layers == 0
            || texture.array_layers > 256
            || texture
                .mip_levels
                .checked_mul(texture.array_layers)
                .is_none_or(|n| n > 4096)
            || ![
                ResourceStates::COMMON,
                ResourceStates::COPY_SOURCE,
                ResourceStates::COPY_DEST,
            ]
            .contains(&initial_state)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: texture.width as u64,
            height: texture.height as u32,
            depth_or_array_size: texture.array_layers as u16,
            mip_levels: texture
                .mip_levels
                .try_into()
                .map_err(|_| GraphicsError::InvalidDescriptor)?,
            format_rgba8: true,
        };
        self.create_resource_storage_typed(
            device,
            desc,
            initial_state,
            None,
            Some(TextureFormat::Rgba8Unorm),
            true,
        )
    }

    #[allow(clippy::too_many_arguments)] // Explicit array identity is separate from legacy resource descriptors.
    fn create_resource_storage_typed(
        &mut self,
        device: GraphicsHandle,
        desc: ResourceDesc,
        initial_state: ResourceStates,
        guest_addr: Option<u64>,
        texture_format: Option<TextureFormat>,
        is_texture_array: bool,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if desc.width == 0
            || desc.height == 0
            || (!is_texture_array && desc.depth_or_array_size != 1)
            || desc.mip_levels == 0
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let width = usize::try_from(desc.width).map_err(|_| GraphicsError::InvalidDescriptor)?;
        let size = match desc.dimension {
            ResourceDimension::Buffer
                if desc.height == 1 && !desc.format_rgba8 && desc.mip_levels == 1 =>
            {
                width
            }
            ResourceDimension::Texture2D if is_texture_array => PackedTextureArrayLayout::new(
                texture_format.ok_or(GraphicsError::InvalidDescriptor)?,
                width,
                desc.height as usize,
                u32::from(desc.mip_levels),
                u32::from(desc.depth_or_array_size),
            )
            .map_err(|_| GraphicsError::InvalidDescriptor)?
            .byte_len(),
            ResourceDimension::Texture2D => PackedTextureLayout::new(
                texture_format.ok_or(GraphicsError::InvalidDescriptor)?,
                width,
                desc.height as usize,
                u32::from(desc.mip_levels),
            )
            .map_err(|_| GraphicsError::InvalidDescriptor)?
            .byte_len(),
            _ => return Err(GraphicsError::InvalidDescriptor),
        };
        if size > MAX_RESOURCE_BYTES
            || guest_addr.is_some_and(|address| address.checked_add(size as u64).is_none())
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let live_bytes: usize = self
            .resources
            .values()
            .map(|resource| resource.data.len())
            .sum::<usize>()
            + self
                .depth_resources
                .values()
                .map(|depth| depth.typed_desc.byte_len().expect("validated depth layout"))
                .sum::<usize>();
        if size > MAX_CONTEXT_RESOURCE_BYTES.saturating_sub(live_bytes + self.retired_bytes()) {
            return Err(MemoryError::AllocationFailed.into());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| MemoryError::AllocationFailed)?;
        data.resize(size, 0);
        let handle = self.allocate_handle();
        self.resources.insert(
            handle,
            GraphicsResource {
                is_texture_array,
                texture_format,
                handle,
                desc,
                state: initial_state,
                guest_backing_addr: guest_addr,
                data,
            },
        );
        Ok(handle)
    }

    /// Compile validated original gfx1030 stages into a bounded pipeline object.
    pub fn create_graphics_pipeline(
        &mut self,
        device: GraphicsHandle,
        vertex: &[u8],
        pixel: &[u8],
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.create_typed_graphics_pipeline(device, vertex, pixel, Default::default())
    }

    pub fn create_typed_graphics_pipeline(
        &mut self,
        device: GraphicsHandle,
        vertex: &[u8],
        pixel: &[u8],
        types: xenora_shader::GraphicsPipelineTypes,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.pipelines.len() + self.retired_pipelines.len() >= 256 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let compiled = xenora_shader::compile_graphics_pipeline_typed(
            xenora_shader::Architecture::Gfx1030,
            vertex,
            pixel,
            types,
        )
        .map_err(|error| GraphicsError::BackendError(format!("shader translation: {error}")))?;
        if compiled.shader.msl.len() > 65536 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut required_uniform_mask = compiled.required_uniform_mask;
        for layer in compiled.array_layers.iter().flatten() {
            if let xenora_shader::GraphicsArrayLayer::Uniform(index) = layer {
                if *index >= 8 {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                required_uniform_mask |= 1 << index;
            }
        }
        let pipeline = Arc::new(GraphicsPipeline {
            storage: compiled.storage,
            array_layers: compiled.array_layers,
            types: compiled.types,
            msl: compiled.shader.msl,
            required_uniform_mask,
            color_targets: compiled.color_targets,
            uses_texture: compiled.uses_texture,
            sampled_pairs: compiled.sampled_pairs,
        });
        let handle = self.allocate_handle();
        self.pipelines.insert(handle, pipeline);
        Ok(handle)
    }

    pub fn destroy_graphics_pipeline(
        &mut self,
        handle: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        if !self.pipelines.contains_key(&handle) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if self.command_lists.values().any(|list| list.commands.iter().any(|cmd| matches!(cmd,GraphicsCommand::SetGraphicsPipeline(Some(bound)) if *bound == handle))) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let pipeline = self.pipelines.remove(&handle).expect("validated pipeline");
        if self.is_pinned(handle) {
            self.retired_pipelines.insert(handle, pipeline);
        }
        Ok(())
    }

    pub fn cmd_set_graphics_pipeline(
        &mut self,
        list: GraphicsHandle,
        pipeline: Option<GraphicsHandle>,
    ) -> Result<(), GraphicsError> {
        if pipeline.is_some_and(|p| !self.pipelines.contains_key(&p)) {
            return Err(GraphicsError::InvalidResourceHandle);
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
            .push(GraphicsCommand::SetGraphicsPipeline(pipeline));
        Ok(())
    }

    /// Copy eight scalar words into recording state; None explicitly unbinds them.
    pub fn cmd_set_shader_uniforms(
        &mut self,
        list: GraphicsHandle,
        values: Option<[u32; 8]>,
    ) -> Result<(), GraphicsError> {
        let commands = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        commands
            .commands
            .push(GraphicsCommand::SetShaderUniforms(values));
        Ok(())
    }

    /// Create an immutable shader-read view; bytes remain owned by the backing resource.
    pub fn create_texture_view(
        &mut self,
        device: GraphicsHandle,
        backing: GraphicsHandle,
        desc: GraphicsTextureViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let texture = self.sampled_resource_desc(backing)?;
        if texture.array_layers.is_some()
            || texture.format != desc.format.storage_format()
            || desc.mip_count == 0
            || desc
                .base_mip
                .checked_add(desc.mip_count)
                .is_none_or(|end| end > texture.mip_levels)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let handle = self.allocate_handle();
        self.texture_views.insert(
            handle,
            GraphicsTextureView {
                backing,
                desc,
                slice: None,
            },
        );
        Ok(handle)
    }

    /// Create a descriptor-only range over an explicit array backing. Range handles
    /// cannot bind as texture2D; a selected slice child must be created first.
    pub fn create_array_range_view(
        &mut self,
        device: GraphicsHandle,
        backing: GraphicsHandle,
        desc: GraphicsArrayRangeViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let texture = self.sampled_resource_desc(backing)?;
        let layers = texture
            .array_layers
            .ok_or(GraphicsError::InvalidDescriptor)?;
        if texture.format != desc.format.storage_format()
            || desc.slice_count == 0
            || desc.mip_count == 0
            || desc
                .base_slice
                .checked_add(desc.slice_count)
                .is_none_or(|end| end > layers)
            || desc
                .base_mip
                .checked_add(desc.mip_count)
                .is_none_or(|end| end > texture.mip_levels)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let handle = self.allocate_handle();
        self.array_range_views
            .insert(handle, GraphicsArrayRangeView { backing, desc });
        Ok(handle)
    }

    /// Normalize a parent-relative range directly to its root. The child owns an
    /// independent root reference and remains valid after parent destruction.
    pub fn create_child_array_range_view(
        &mut self,
        device: GraphicsHandle,
        parent: GraphicsHandle,
        relative_desc: GraphicsArrayRangeViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let parent = self
            .array_range_views
            .get(&parent)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if relative_desc.slice_count == 0
            || relative_desc.mip_count == 0
            || relative_desc
                .base_slice
                .checked_add(relative_desc.slice_count)
                .is_none_or(|end| end > parent.desc.slice_count)
            || relative_desc
                .base_mip
                .checked_add(relative_desc.mip_count)
                .is_none_or(|end| end > parent.desc.mip_count)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let desc = GraphicsArrayRangeViewDesc {
            base_slice: parent
                .desc
                .base_slice
                .checked_add(relative_desc.base_slice)
                .ok_or(GraphicsError::InvalidDescriptor)?,
            base_mip: parent
                .desc
                .base_mip
                .checked_add(relative_desc.base_mip)
                .ok_or(GraphicsError::InvalidDescriptor)?,
            ..relative_desc
        };
        self.create_array_range_view(device, parent.backing, desc)
    }

    /// Select one parent-relative slice and mip range as an ordinary bindable 2D
    /// view, retaining the full root backing without copying any subresources.
    pub fn create_array_range_slice_view(
        &mut self,
        device: GraphicsHandle,
        parent: GraphicsHandle,
        relative_desc: GraphicsArraySliceViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let parent = self
            .array_range_views
            .get(&parent)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if relative_desc.slice >= parent.desc.slice_count
            || relative_desc.mip_count == 0
            || relative_desc
                .base_mip
                .checked_add(relative_desc.mip_count)
                .is_none_or(|end| end > parent.desc.mip_count)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let desc = GraphicsArraySliceViewDesc {
            slice: parent
                .desc
                .base_slice
                .checked_add(relative_desc.slice)
                .ok_or(GraphicsError::InvalidDescriptor)?,
            base_mip: parent
                .desc
                .base_mip
                .checked_add(relative_desc.base_mip)
                .ok_or(GraphicsError::InvalidDescriptor)?,
            ..relative_desc
        };
        self.create_array_slice_view(device, parent.backing, desc)
    }

    /// Children independently retain the root. Recorded range bindings retain the
    /// descriptor until reset; submitted bindings retain it through retirement.
    pub fn destroy_array_range_view(
        &mut self,
        handle: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        if !self.array_range_views.contains_key(&handle) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if self.command_lists.values().any(|list| list.commands.iter().any(|cmd|
            matches!(cmd, GraphicsCommand::SetSampledArrayRanges { ranges, .. } if ranges.contains(&Some(handle))))) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let range = self
            .array_range_views
            .remove(&handle)
            .expect("validated range");
        if self.is_pinned(handle) {
            self.retired_array_ranges.insert(handle, range);
        }
        Ok(())
    }

    /// Opt in after installing a queued backend with actual shader array support.
    pub fn enable_shader_array_views(&mut self) -> Result<(), GraphicsError> {
        if self.queued_backend.is_none() || !self.array_view_backend {
            return Err(GraphicsError::NativeMetalRequired);
        }
        self.shader_array_views = true;
        Ok(())
    }

    /// Atomically replace independent texture and sampler slots with array ranges.
    pub fn cmd_set_sampled_array_ranges(
        &mut self,
        list: GraphicsHandle,
        ranges: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    ) -> Result<(), GraphicsError> {
        for handle in ranges.iter().flatten() {
            if !self.array_range_views.contains_key(handle) {
                return Err(GraphicsError::InvalidResourceHandle);
            }
        }
        for sampler in samplers.iter().flatten() {
            sampler
                .validate()
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
        }
        let list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        list.commands
            .push(GraphicsCommand::SetSampledArrayRanges { ranges, samplers });
        Ok(())
    }

    /// Select one explicit array slice without copying any backing bytes.
    pub fn create_array_slice_view(
        &mut self,
        device: GraphicsHandle,
        backing: GraphicsHandle,
        desc: GraphicsArraySliceViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let texture = self.sampled_resource_desc(backing)?;
        if texture
            .array_layers
            .is_none_or(|layers| desc.slice >= layers)
            || texture.format != desc.format.storage_format()
            || desc.mip_count == 0
            || desc
                .base_mip
                .checked_add(desc.mip_count)
                .is_none_or(|end| end > texture.mip_levels)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let handle = self.allocate_handle();
        self.texture_views.insert(
            handle,
            GraphicsTextureView {
                backing,
                slice: Some(desc.slice),
                desc: GraphicsTextureViewDesc {
                    base_mip: desc.base_mip,
                    mip_count: desc.mip_count,
                    format: desc.format,
                },
            },
        );
        Ok(handle)
    }

    /// Create an independent read-only view relative to an existing view's mip range.
    /// The child stores only the root backing and normalized absolute range; its
    /// parent handle may be destroyed without invalidating the child.
    pub fn create_child_texture_view(
        &mut self,
        device: GraphicsHandle,
        parent_view: GraphicsHandle,
        desc: GraphicsTextureViewDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        let parent = self
            .texture_views
            .get(&parent_view)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if desc.mip_count == 0
            || desc
                .base_mip
                .checked_add(desc.mip_count)
                .is_none_or(|end| end > parent.desc.mip_count)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let base_mip = parent
            .desc
            .base_mip
            .checked_add(desc.base_mip)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        if let Some(slice) = parent.slice {
            return self.create_array_slice_view(
                device,
                parent.backing,
                GraphicsArraySliceViewDesc {
                    slice,
                    base_mip,
                    mip_count: desc.mip_count,
                    format: desc.format,
                },
            );
        }
        // Reuse root storage-family, device, budget and complete-chain validation.
        // Allocation occurs only after every validation succeeds.
        self.create_texture_view(
            device,
            parent.backing,
            GraphicsTextureViewDesc { base_mip, ..desc },
        )
    }

    pub fn destroy_texture_view(&mut self, view: GraphicsHandle) -> Result<(), GraphicsError> {
        if !self.texture_views.contains_key(&view) {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if self.command_lists.values().any(|list| list.commands.iter().any(|cmd|
            matches!(cmd, GraphicsCommand::SetSampledViews { views, .. } if views.contains(&Some(view))))) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let value = self.texture_views.remove(&view).expect("validated view");
        if self.is_pinned(view) {
            self.retired_views.insert(view, value);
        }
        Ok(())
    }

    /// Create an immutable sampler descriptor. Texture-relative LOD validation
    /// remains per used sampling pair at draw preparation.
    pub fn create_sampler(
        &mut self,
        device: GraphicsHandle,
        desc: xenora_texture::SamplerDesc,
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        desc.validate()
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
        if self.guest_object_count() >= crate::graphics_service::MAX_GUEST_OBJECTS {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let handle = self.allocate_handle();
        self.samplers.insert(handle, desc);
        Ok(handle)
    }

    /// Recorded and submitted work owns descriptor copies, not sampler handles.
    pub fn destroy_sampler(&mut self, sampler: GraphicsHandle) -> Result<(), GraphicsError> {
        self.samplers
            .remove(&sampler)
            .map(|_| ())
            .ok_or(GraphicsError::InvalidResourceHandle)
    }

    /// Resolve all sampler handles before atomically recording ordinary view bindings.
    /// Texture and sampler slots are independent; per-pair occupancy is checked at draw.
    pub fn cmd_set_sampled_view_handles(
        &mut self,
        list: GraphicsHandle,
        views: [Option<GraphicsHandle>; 2],
        sampler_handles: [Option<GraphicsHandle>; 2],
    ) -> Result<(), GraphicsError> {
        let mut samplers = [None; 2];
        for (slot, handle) in sampler_handles.into_iter().enumerate() {
            if let Some(handle) = handle {
                samplers[slot] = Some(
                    *self
                        .samplers
                        .get(&handle)
                        .ok_or(GraphicsError::InvalidResourceHandle)?,
                );
            }
        }
        self.cmd_set_sampled_views(list, views, samplers)
    }

    /// Atomically replace both texture slots with view handles and independent samplers.
    pub fn cmd_set_sampled_views(
        &mut self,
        list: GraphicsHandle,
        views: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    ) -> Result<(), GraphicsError> {
        for view in views.iter().flatten() {
            if !self.texture_views.contains_key(view) {
                return Err(GraphicsError::InvalidResourceHandle);
            }
        }
        for sampler in samplers.iter().flatten() {
            sampler
                .validate()
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
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
            .push(GraphicsCommand::SetSampledViews { views, samplers });
        Ok(())
    }

    pub fn cmd_set_sampled_texture(
        &mut self,
        list: GraphicsHandle,
        texture: Option<GraphicsHandle>,
        sampler: xenora_texture::SamplerDesc,
    ) -> Result<(), GraphicsError> {
        sampler
            .validate()
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
        if let Some(handle) = texture {
            let texture = self.sampled_resource_desc(handle)?;
            if texture.array_layers.is_some() {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let metadata = xenora_texture::TextureMetadata::new(
                texture.extent.0,
                texture.extent.1,
                texture.mip_levels,
            )
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
            sampler
                .validate_against_texture(&metadata)
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
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
            .push(GraphicsCommand::SetSampledTexture { texture, sampler });
        Ok(())
    }

    /// Atomically replace both independent sampled binding tables.
    pub fn cmd_set_sampled_bindings(
        &mut self,
        list: GraphicsHandle,
        textures: [Option<GraphicsHandle>; 2],
        samplers: [Option<xenora_texture::SamplerDesc>; 2],
    ) -> Result<(), GraphicsError> {
        for handle in textures.iter().flatten() {
            if self.sampled_resource_desc(*handle)?.array_layers.is_some() {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        for sampler in samplers.iter().flatten() {
            sampler
                .validate()
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
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
            .push(GraphicsCommand::SetSampledBindings { textures, samplers });
        Ok(())
    }

    pub fn cmd_set_vertex_colors(
        &mut self,
        list: GraphicsHandle,
        resource: GraphicsHandle,
        offset: usize,
        stride: usize,
    ) -> Result<(), GraphicsError> {
        let buffer = self
            .resources
            .get(&resource)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if buffer.desc.dimension != ResourceDimension::Buffer
            || stride < 16
            || offset
                .checked_add(16)
                .is_none_or(|end| end > buffer.data.len())
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
        commands.commands.push(GraphicsCommand::SetVertexColors {
            resource,
            offset,
            stride,
        });
        Ok(())
    }

    /// Allocate bounded uninitialized depth in DEPTH_WRITE state.
    pub fn create_depth_resource(
        &mut self,
        device: GraphicsHandle,
        extent: (usize, usize),
    ) -> Result<GraphicsHandle, GraphicsError> {
        if !self.devices.contains_key(&device) {
            return Err(GraphicsError::InvalidDeviceHandle);
        }
        if extent.0 > MAX_RENDER_DIMENSION || extent.1 > MAX_RENDER_DIMENSION {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let bytes = extent
            .0
            .checked_mul(extent.1)
            .filter(|n| (1..=MAX_NATIVE_RENDER_PIXELS).contains(n))
            .ok_or(GraphicsError::InvalidDescriptor)?
            * 4;
        let live: usize = self.resources.values().map(|r| r.data.len()).sum::<usize>()
            + self
                .depth_resources
                .values()
                .map(|d| d.typed_desc.byte_len().expect("validated depth layout"))
                .sum::<usize>();
        if bytes > MAX_CONTEXT_RESOURCE_BYTES.saturating_sub(live + self.retired_bytes()) {
            return Err(MemoryError::AllocationFailed.into());
        }
        let handle = self.allocate_handle();
        self.depth_resources.insert(
            handle,
            DepthResource {
                typed_desc: GraphicsTypedTextureDesc {
                    format: TextureFormat::Depth32Float,
                    extent,
                    mip_levels: 1,
                    array_layers: None,
                },
                extent,
                state: ResourceStates::DEPTH_WRITE,
                snapshot: None,
            },
        );
        Ok(handle)
    }

    /// Bind retained depth; None unbinds it. Binding changes after drawing are rejected.
    pub fn cmd_set_depth_target(
        &mut self,
        list: GraphicsHandle,
        target: Option<GraphicsHandle>,
        load: GraphicsDepthLoad,
    ) -> Result<(), GraphicsError> {
        if let GraphicsDepthLoad::Clear(value) = load
            && (!value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if target.is_none() && load == GraphicsDepthLoad::Load {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if target.is_some_and(|h| {
            self.depth_resources.get(&h).is_some_and(|d| {
                d.typed_desc.mip_levels != 1 || d.typed_desc.array_layers.is_some()
            })
        }) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if target.is_some_and(|h| !self.depth_resources.contains_key(&h)) {
            return Err(GraphicsError::InvalidResourceHandle);
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
            .push(GraphicsCommand::SetDepthTarget { target, load });
        Ok(())
    }

    /// Destroy a color, buffer or depth resource after all recorded references are released.
    pub fn destroy_resource(&mut self, resource: GraphicsHandle) -> Result<(), GraphicsError> {
        if self
            .writable_views
            .values()
            .chain(self.storage_views.values())
            .any(|v| v.root == resource)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if self
            .texture_views
            .values()
            .any(|view| view.backing == resource)
            || self
                .array_range_views
                .values()
                .any(|view| view.backing == resource)
        {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if !self.resources.contains_key(&resource) && !self.depth_resources.contains_key(&resource)
        {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        let referenced = self.command_lists.values().any(|list| {
            list.commands.iter().any(|command| match command {
                GraphicsCommand::CopyResource {
                    destination,
                    source,
                }
                | GraphicsCommand::CopyBufferRegion {
                    destination,
                    source,
                    ..
                }
                | GraphicsCommand::CopyImage {
                    destination,
                    source,
                    ..
                } => *destination == resource || *source == resource,
                GraphicsCommand::SetVertexColors {
                    resource: handle, ..
                }
                | GraphicsCommand::ResourceBarrier {
                    resource: handle, ..
                }
                | GraphicsCommand::SetVertexBuffer {
                    resource: handle, ..
                }
                | GraphicsCommand::SetIndexBuffer {
                    resource: handle, ..
                } => *handle == resource,
                GraphicsCommand::SetSampledArrayRanges { ranges, .. } => ranges
                    .iter()
                    .flatten()
                    .any(|handle| self.array_range_views[handle].backing == resource),
                GraphicsCommand::SetSampledViews { views, .. } => views
                    .iter()
                    .flatten()
                    .any(|handle| self.texture_views[handle].backing == resource),
                GraphicsCommand::SetSampledTexture { texture, .. } => *texture == Some(resource),
                GraphicsCommand::SetSampledBindings { textures, .. } => {
                    textures.contains(&Some(resource))
                }
                GraphicsCommand::SetGraphicsPipeline(_) | GraphicsCommand::SetShaderUniforms(_) => {
                    false
                }
                GraphicsCommand::SetStorageView(view) => {
                    view.is_some_and(|h| self.storage_views[&h].root == resource)
                }
                GraphicsCommand::SetWritableColorTargets { views, .. } => views
                    .iter()
                    .any(|v| self.writable_views[v].root == resource),
                GraphicsCommand::SetWritableDepthTarget { view, .. } => {
                    view.is_some_and(|v| self.writable_views[&v].root == resource)
                }
                GraphicsCommand::SetDepthTarget { target, .. } => *target == Some(resource),
                GraphicsCommand::SetColorTargets { targets, .. } => targets.contains(&resource),
                GraphicsCommand::SetRenderTargets { color_target, .. } => {
                    *color_target == Some(resource)
                }
                GraphicsCommand::SetDepthClear(_)
                | GraphicsCommand::SetDrawState(_)
                | GraphicsCommand::SetViewport { .. }
                | GraphicsCommand::DrawInstanced { .. }
                | GraphicsCommand::DrawIndexedInstanced { .. } => false,
            })
        });
        if referenced {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        // Recorded lists retain references even after submission. Once reset,
        // in-flight resources retire privately until terminal completion.
        let pinned = self.is_pinned(resource);
        if let Some(value) = self.resources.remove(&resource)
            && pinned
        {
            self.retired_resources.insert(resource, value);
        }
        if let Some(value) = self.depth_resources.remove(&resource)
            && pinned
        {
            self.retired_depth.insert(resource, value);
        }
        self.root_publications.remove(&resource);
        Ok(())
    }

    /// Synchronously upload guest bytes into a host-backed COPY_DEST resource.
    /// All source permissions and bounds are checked before changing the resource.
    pub fn copy_guest_to_resource(
        &mut self,
        resource: GraphicsHandle,
        offset: usize,
        address: u64,
        size: usize,
        memory: &GuestMemory,
    ) -> Result<(), GraphicsError> {
        let range = self.transfer_range(resource, offset, size, ResourceStates::COPY_DEST)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| MemoryError::AllocationFailed)?;
        bytes.resize(size, 0);
        memory.read_bytes(address, &mut bytes)?;
        let host_epoch = self
            .publication(resource)
            .host_epoch
            .checked_add(1)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        if let Some(depth) = self.depth_resources.get_mut(&resource) {
            let len = depth.typed_desc.byte_len()?;
            let mut full = Vec::new();
            full.try_reserve_exact(len)
                .map_err(|_| MemoryError::AllocationFailed)?;
            if range.start == 0 && range.end == len {
                full.extend_from_slice(&bytes);
            } else {
                let packed = depth
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.downcast_ref::<GraphicsTypedDepthSnapshot>())
                    .filter(|s| s.desc == depth.typed_desc && s.bytes.len() == len)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                full.extend_from_slice(&packed.bytes);
                full[range].copy_from_slice(&bytes);
            }
            depth.snapshot = Some(Arc::new(GraphicsTypedDepthSnapshot {
                desc: depth.typed_desc,
                bytes: Arc::from(full),
            }));
        } else {
            self.resources.get_mut(&resource).unwrap().data[range].copy_from_slice(&bytes);
        }
        self.root_publications.insert(
            resource,
            RootPublication {
                host_epoch,
                version: None,
            },
        );
        Ok(())
    }

    /// Synchronously read a host-backed COPY_SOURCE resource into guest memory.
    /// GuestMemory validates the whole destination before writing any bytes.
    pub fn copy_resource_to_guest(
        &self,
        resource: GraphicsHandle,
        offset: usize,
        address: u64,
        size: usize,
        memory: &mut GuestMemory,
    ) -> Result<(), GraphicsError> {
        let range = self.transfer_range(resource, offset, size, ResourceStates::COPY_SOURCE)?;
        if let Some(depth) = self.depth_resources.get(&resource) {
            let packed = depth
                .snapshot
                .as_ref()
                .and_then(|s| s.downcast_ref::<GraphicsTypedDepthSnapshot>())
                .filter(|s| {
                    s.desc == depth.typed_desc
                        && s.bytes.len() == depth.typed_desc.byte_len().unwrap_or(0)
                })
                .ok_or(GraphicsError::InvalidDescriptor)?;
            memory.write_bytes(address, &packed.bytes[range])?;
        } else {
            memory.write_bytes(address, &self.resources[&resource].data[range])?;
        }
        Ok(())
    }

    fn transfer_range(
        &self,
        resource: GraphicsHandle,
        offset: usize,
        size: usize,
        state: ResourceStates,
    ) -> Result<std::ops::Range<usize>, GraphicsError> {
        if self.is_pinned(resource) {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        if let Some(depth) = self.depth_resources.get(&resource) {
            if depth.state != state {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            let end = offset
                .checked_add(size)
                .ok_or(GraphicsError::IndexOutOfRange)?;
            if end > depth.typed_desc.byte_len()? {
                return Err(GraphicsError::IndexOutOfRange);
            }
            return Ok(offset..end);
        }
        let resource = self
            .resources
            .get(&resource)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if resource.guest_backing_addr.is_some() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if resource.state != state {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let end = offset
            .checked_add(size)
            .ok_or(GraphicsError::IndexOutOfRange)?;
        if end > resource.data.len() {
            return Err(GraphicsError::IndexOutOfRange);
        }
        Ok(offset..end)
    }

    /// Set the pass depth clear before drawing. Reset defaults to 1.0.
    pub fn cmd_set_depth_clear(
        &mut self,
        list: GraphicsHandle,
        value: f32,
    ) -> Result<(), GraphicsError> {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        list.commands.push(GraphicsCommand::SetDepthClear(value));
        Ok(())
    }

    /// Record an exact whole-resource copy. Execution requires the copy-capable
    /// queued backend; ordinary render backends must never treat it as a no-op.
    pub fn cmd_copy_resource(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        source: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        let commands = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if destination == source {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let destination_resource = self
            .resources
            .get(&destination)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let source_resource = self
            .resources
            .get(&source)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        // Guest memory may supply an immutable whole-buffer source snapshot.
        // Destination publication remains owned-storage-only.
        if destination_resource.is_texture_array
            || source_resource.is_texture_array
            || destination_resource.guest_backing_addr.is_some()
            || (source_resource.guest_backing_addr.is_some()
                && source_resource.desc.dimension != ResourceDimension::Buffer)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let a = &destination_resource.desc;
        let b = &source_resource.desc;
        if a.dimension != b.dimension
            || a.width != b.width
            || a.height != b.height
            || a.depth_or_array_size != b.depth_or_array_size
            || a.mip_levels != b.mip_levels
            || a.format_rgba8 != b.format_rgba8
            || destination_resource.texture_format != source_resource.texture_format
            || destination_resource.data.len() != source_resource.data.len()
            || destination_resource.data.is_empty()
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        match a.dimension {
            ResourceDimension::Buffer
                if a.height == 1
                    && a.depth_or_array_size == 1
                    && a.mip_levels == 1
                    && !a.format_rgba8 => {}
            ResourceDimension::Texture2D
                if a.depth_or_array_size == 1
                    && a.mip_levels == 1
                    && a.format_rgba8
                    && destination_resource.texture_format == Some(TextureFormat::Rgba8Unorm) => {}
            _ => return Err(GraphicsError::InvalidDescriptor),
        }
        self.command_lists
            .get_mut(&list)
            .expect("validated list")
            .commands
            .push(GraphicsCommand::CopyResource {
                destination,
                source,
            });
        Ok(())
    }

    /// Record a byte region copy into a distinct owned buffer.
    /// Guest-backed sources snapshot their entire logical buffer at submission,
    /// after budget validation. Recording does not read guest memory; unreadable
    /// bytes outside the copied region still reject the full source snapshot.
    pub fn cmd_copy_buffer_region(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_offset: usize,
        source: GraphicsHandle,
        source_offset: usize,
        byte_len: usize,
    ) -> Result<(), GraphicsError> {
        let commands = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if destination == source || byte_len == 0 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let GraphicsCopyLayout::Buffer {
            byte_len: source_len,
        } = self.copy_layout_with_guest_source(source, true)?
        else {
            return Err(GraphicsError::InvalidDescriptor);
        };
        let GraphicsCopyLayout::Buffer {
            byte_len: destination_len,
        } = self.copy_layout(destination)?
        else {
            return Err(GraphicsError::InvalidDescriptor);
        };
        if source_offset
            .checked_add(byte_len)
            .is_none_or(|end| end > source_len)
            || destination_offset
                .checked_add(byte_len)
                .is_none_or(|end| end > destination_len)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        self.command_lists
            .get_mut(&list)
            .expect("validated list")
            .commands
            .push(GraphicsCommand::CopyBufferRegion {
                destination,
                destination_offset,
                source,
                source_offset,
                byte_len,
            });
        Ok(())
    }

    /// Record a texture rectangle or pitched buffer/texture transfer.
    pub fn cmd_copy_image(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
    ) -> Result<(), GraphicsError> {
        if matches!(source_location, GraphicsImageCopyLocation::Rgba8Mip { .. })
            || matches!(
                destination_location,
                GraphicsImageCopyLocation::Rgba8Mip { .. }
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        self.record_image_copy(
            list,
            destination,
            destination_location,
            source,
            source_location,
            extent,
            false,
            false,
        )
    }

    /// Record a full-backing copy preserving all untouched mip levels.
    pub fn cmd_copy_image_mips(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
    ) -> Result<(), GraphicsError> {
        if matches!(source_location, GraphicsImageCopyLocation::Rgba8 { .. })
            || matches!(
                destination_location,
                GraphicsImageCopyLocation::Rgba8 { .. }
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        self.record_image_copy(
            list,
            destination,
            destination_location,
            source,
            source_location,
            extent,
            true,
            false,
        )
    }

    pub fn cmd_copy_image_subresources(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
    ) -> Result<(), GraphicsError> {
        if matches!(source_location, GraphicsImageCopyLocation::Rgba8 { .. })
            || matches!(
                destination_location,
                GraphicsImageCopyLocation::Rgba8 { .. }
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        self.record_image_copy(
            list,
            destination,
            destination_location,
            source,
            source_location,
            extent,
            true,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)] // Shared validator preserves the two public recording contracts.
    fn record_image_copy(
        &mut self,
        list: GraphicsHandle,
        destination: GraphicsHandle,
        destination_location: GraphicsImageCopyLocation,
        source: GraphicsHandle,
        source_location: GraphicsImageCopyLocation,
        extent: (usize, usize),
        allow_mips: bool,
        allow_arrays: bool,
    ) -> Result<(), GraphicsError> {
        if matches!(source_location, GraphicsImageCopyLocation::Texture { .. })
            || matches!(
                destination_location,
                GraphicsImageCopyLocation::Texture { .. }
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let commands = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if source == destination
            || matches!(
                (source_location, destination_location),
                (
                    GraphicsImageCopyLocation::Buffer { .. },
                    GraphicsImageCopyLocation::Buffer { .. }
                )
            )
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        source_location.validate(
            self.copy_layout_for_transfer(source, false, allow_mips, allow_arrays)?,
            extent,
        )?;
        destination_location.validate(
            self.copy_layout_for_transfer(destination, false, allow_mips, allow_arrays)?,
            extent,
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

    /// Record resource barrier in command list.
    pub fn cmd_resource_barrier(
        &mut self,
        list: GraphicsHandle,
        resource: GraphicsHandle,
        before: ResourceStates,
        after: ResourceStates,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if !self.resources.contains_key(&resource) && !self.depth_resources.contains_key(&resource)
        {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        cmd_list.commands.push(GraphicsCommand::ResourceBarrier {
            resource,
            before,
            after,
        });
        Ok(())
    }

    /// Record set render targets.
    pub fn cmd_set_render_targets(
        &mut self,
        list: GraphicsHandle,
        color_target: Option<GraphicsHandle>,
        clear_color: [u8; 4],
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if let Some(target) = color_target
            && !self.resources.contains_key(&target)
        {
            return Err(GraphicsError::InvalidResourceHandle);
        }
        if color_target.is_some_and(|h| self.resources[&h].is_texture_array) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        cmd_list.commands.push(GraphicsCommand::SetRenderTargets {
            color_target,
            clear_color,
        });
        Ok(())
    }

    /// Bind distinct, host-backed RGBA8 textures with one common extent.
    pub fn cmd_set_color_targets(
        &mut self,
        list: GraphicsHandle,
        targets: &[GraphicsHandle],
        clears: &[[u8; 4]],
    ) -> Result<(), GraphicsError> {
        let loads: Vec<_> = clears
            .iter()
            .copied()
            .map(GraphicsColorLoad::Clear)
            .collect();
        self.cmd_set_color_targets_with_loads(list, targets, &loads)
    }

    /// Bind color attachments with explicit clear or committed-content load actions.
    pub fn cmd_set_color_targets_with_loads(
        &mut self,
        list: GraphicsHandle,
        targets: &[GraphicsHandle],
        loads: &[GraphicsColorLoad],
    ) -> Result<(), GraphicsError> {
        self.color_target_extent(targets, loads)?;
        let commands = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        commands.commands.push(GraphicsCommand::SetColorTargets {
            targets: targets.to_vec(),
            loads: loads.to_vec(),
        });
        Ok(())
    }

    fn color_target_extent(
        &self,
        targets: &[GraphicsHandle],
        loads: &[GraphicsColorLoad],
    ) -> Result<(usize, usize), GraphicsError> {
        if !(1..=8).contains(&targets.len()) || targets.len() != loads.len() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut extent = None;
        for (slot, handle) in targets.iter().enumerate() {
            let resource = self
                .resources
                .get(handle)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            let current = (resource.desc.width as usize, resource.desc.height as usize);
            if targets[..slot].contains(handle)
                || resource.desc.dimension != ResourceDimension::Texture2D
                || resource.is_texture_array
                || resource.texture_format
                    != Some(match loads[slot] {
                        GraphicsColorLoad::Typed { attachment, .. } => {
                            attachment.validate()?;
                            if !self.resource_roles.color(attachment.format)
                                || resource.desc.mip_levels != 1
                            {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            attachment.format
                        }
                        _ => TextureFormat::Rgba8Unorm,
                    })
                || resource.guest_backing_addr.is_some()
                || extent.is_some_and(|value| value != current)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            extent = Some(current);
        }
        Ok(extent.expect("nonempty validated target list"))
    }

    /// Record bounded per-draw depth, blend and color-mask state.
    /// Nondefault state requires an ordered pass adapter at execution.
    pub fn cmd_set_draw_state(
        &mut self,
        list: GraphicsHandle,
        state: GraphicsDrawState,
    ) -> Result<(), GraphicsError> {
        let commands = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !commands.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if state.color_write_mask > 15 || (state.depth_write && state.depth_compare.is_none()) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        commands.commands.push(GraphicsCommand::SetDrawState(state));
        Ok(())
    }

    /// Record set viewport.
    pub fn cmd_set_viewport(
        &mut self,
        list: GraphicsHandle,
        width: usize,
        height: usize,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if width == 0
            || height == 0
            || width > MAX_RENDER_DIMENSION
            || height > MAX_RENDER_DIMENSION
        {
            return Err(GraphicsError::InvalidViewport);
        }
        cmd_list
            .commands
            .push(GraphicsCommand::SetViewport { width, height });
        Ok(())
    }

    /// Record set vertex buffer.
    pub fn cmd_set_vertex_buffer(
        &mut self,
        list: GraphicsHandle,
        resource: GraphicsHandle,
        offset: usize,
        stride: usize,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        let res = self
            .resources
            .get(&resource)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let end = offset
            .checked_add(stride)
            .ok_or(GraphicsError::InvalidDescriptor)?;
        if end > res.data.len() && res.guest_backing_addr.is_none() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        cmd_list.commands.push(GraphicsCommand::SetVertexBuffer {
            resource,
            offset,
            stride,
        });
        Ok(())
    }

    /// Record draw instanced.
    pub fn cmd_draw_instanced(
        &mut self,
        list: GraphicsHandle,
        vertex_count_per_instance: u32,
        instance_count: u32,
        start_vertex_location: u32,
        start_instance_location: u32,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        cmd_list.commands.push(GraphicsCommand::DrawInstanced {
            vertex_count_per_instance,
            instance_count,
            start_vertex_location,
            start_instance_location,
        });
        Ok(())
    }

    /// Record set index buffer. `format` is a DXGI format code; only
    /// `R16_UINT` (57) and `R32_UINT` (42) are index formats.
    pub fn cmd_set_index_buffer(
        &mut self,
        list: GraphicsHandle,
        resource: GraphicsHandle,
        offset: usize,
        format: u32,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        let format =
            IndexFormat::from_dxgi(format).ok_or(GraphicsError::InvalidIndexFormat(format))?;
        let res = self
            .resources
            .get(&resource)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        if !offset.is_multiple_of(format.size()) || offset > res.data.len() {
            return Err(GraphicsError::InvalidDescriptor);
        }
        cmd_list.commands.push(GraphicsCommand::SetIndexBuffer {
            resource,
            offset,
            format,
        });
        Ok(())
    }

    /// Record draw indexed instanced. Each draw renders a bounded triangle list: at
    /// execution the index count must be a multiple of 3 within the selected renderer limit, the instance count 0 or 1 and
    /// the start instance 0, or the draw is rejected; a zero index or instance
    /// count draws nothing. Index values are ordinary triangle-list indices
    /// (no primitive restart), range checked after adding the base vertex.
    pub fn cmd_draw_indexed_instanced(
        &mut self,
        list: GraphicsHandle,
        index_count_per_instance: u32,
        instance_count: u32,
        start_index_location: u32,
        base_vertex_location: i32,
        start_instance_location: u32,
    ) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        cmd_list
            .commands
            .push(GraphicsCommand::DrawIndexedInstanced {
                index_count_per_instance,
                instance_count,
                start_index_location,
                base_vertex_location,
                start_instance_location,
            });
        Ok(())
    }

    /// Close command list.
    pub fn close_command_list(&mut self, list: GraphicsHandle) -> Result<(), GraphicsError> {
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        if !cmd_list.is_recording {
            return Err(GraphicsError::CommandListAlreadyClosed);
        }
        cmd_list.is_recording = false;
        Ok(())
    }

    /// Reset command list with an allocator.
    pub fn reset_command_list(
        &mut self,
        list: GraphicsHandle,
        allocator: GraphicsHandle,
    ) -> Result<(), GraphicsError> {
        if !self.allocators.contains_key(&allocator) {
            return Err(GraphicsError::InvalidAllocatorHandle);
        }
        let cmd_list = self
            .command_lists
            .get_mut(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;
        cmd_list.commands.clear();
        cmd_list.is_recording = true;
        Ok(())
    }

    /// Execute command list on command queue using the configured backend.
    pub fn execute_command_list(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        memory: &GuestMemory,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        if self.command_lists.get(&list).is_some_and(|commands| {
            commands.commands.iter().any(|command| {
                matches!(
                    command,
                    GraphicsCommand::CopyResource { .. }
                        | GraphicsCommand::CopyBufferRegion { .. }
                        | GraphicsCommand::CopyImage { .. }
                )
            })
        }) {
            return Err(GraphicsError::NativeMetalRequired);
        }
        if self.queued_backend.is_some() {
            let handle = self.submit_command_list(queue, list, memory)?;
            self.wait_submission(handle)?;
            let node = self
                .queued
                .get_mut(&handle)
                .expect("internal queued receipt");
            let result = node
                .receipt
                .outcome
                .take()
                .ok_or_else(|| node.receipt.error.take().expect("terminal error"));
            self.release_submission(handle)?;
            return result;
        }
        if self.async_backend.is_some() {
            let handle = self.submit_command_list(queue, list, memory)?;
            // wait always processes a terminal result, including backend/commit failure.
            let status = self.wait_submission(handle)?;
            let receipt = self.submission.as_mut().expect("internal receipt");
            let result = match status {
                GraphicsSubmissionStatus::Complete => {
                    Ok(receipt.outcome.take().expect("completed outcome"))
                }
                GraphicsSubmissionStatus::Failed => {
                    Err(receipt.error.take().expect("failed outcome"))
                }
                GraphicsSubmissionStatus::Pending => unreachable!("wait is terminal"),
            };
            self.release_submission(handle)?;
            return result;
        }
        let backend_closure = self.backend.clone();
        let backend_id = self.backend_id;
        let renderer_ref: Option<&BackendRenderer<'_>> = match &backend_closure {
            Some(arc) => {
                let r: &BackendRenderer<'_> = arc.as_ref();
                Some(r)
            }
            None => None,
        };
        let pass = self.pass_backend.clone();
        self.execute_command_list_internal(
            queue,
            list,
            memory,
            renderer_ref,
            pass.as_deref(),
            backend_id,
        )
    }

    /// Execute command list strictly using native Metal. Fails with `GraphicsError::NativeMetalRequired`
    /// if a native Metal backend is not attached.
    pub fn execute_command_list_native_metal(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        memory: &GuestMemory,
        metal_renderer: &BackendRenderer<'_>,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        self.execute_command_list_internal(
            queue,
            list,
            memory,
            Some(metal_renderer),
            None,
            GraphicsBackendId::MetalNative,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_geometry(
        &self,
        memory: &GuestMemory,
        snapshot_budget: &std::cell::Cell<usize>,
        first_gather: bool,
        queued: Option<&QueuedPreparation>,
        states: &HashMap<GraphicsHandle, ResourceStates>,
        positions: (Option<GraphicsHandle>, usize, usize),
        colors: Option<(GraphicsHandle, usize, usize)>,
        indices: Option<(GraphicsHandle, usize, IndexFormat)>,
        start: u32,
        count: u32,
        base_vertex: i32,
    ) -> Result<Option<GraphicsGeometryInput>, GraphicsError> {
        let Some(q) = queued else {
            return Ok(None);
        };
        if !positions
            .0
            .into_iter()
            .chain(colors.map(|a| a.0))
            .chain(indices.map(|a| a.0))
            .any(|h| q.produced.contains_key(&h))
        {
            return Ok(None);
        }
        if !self.geometry_backend {
            return Err(GraphicsError::NativeMetalRequired);
        }
        let next = snapshot_budget
            .get()
            .checked_add(count as usize * 32 + 128 + if first_gather { 4 } else { 0 })
            .ok_or(GraphicsError::InvalidDescriptor)?;
        self.validate_queued_work_budget(next)?;
        snapshot_budget.set(next);
        let buffer = |handle: GraphicsHandle,
                      required: ResourceStates|
         -> Result<GraphicsGeometryBuffer, GraphicsError> {
            let resource = self
                .resources
                .get(&handle)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            if resource.desc.dimension != ResourceDimension::Buffer
                || resource.data.is_empty()
                || resource.data.len() > 64 * 1024 * 1024
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            if states.get(&handle).copied().unwrap_or(resource.state) != required {
                return Err(GraphicsError::ResourceLifetimeViolation);
            }
            let byte_len = resource.data.len();
            let source = if let Some(version) = q.produced.get(&handle) {
                if version.output != GraphicsProducedOutput::Copy(0)
                    || q.layouts.get(&handle) != Some(&GraphicsProducedLayout::Buffer { byte_len })
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                GraphicsCopyInput::Produced(*version)
            } else {
                let next = snapshot_budget
                    .get()
                    .checked_add(byte_len.div_ceil(4) * 12)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                self.validate_queued_work_budget(next)?;
                snapshot_budget.set(next);
                let mut bytes = vec![0; byte_len];
                Self::read_resource(resource, memory, 0, &mut bytes)?;
                GraphicsCopyInput::Host(bytes.into())
            };
            Ok(GraphicsGeometryBuffer { byte_len, source })
        };
        let attribute = |(handle, offset, stride): (GraphicsHandle, usize, usize)| -> Result<GraphicsAttributeInput, GraphicsError> {
            let buffer = buffer(handle, ResourceStates::VERTEX_AND_CONSTANT_BUFFER)?;
            if stride < 16 || stride > u32::MAX as usize || offset.checked_add(16).is_none_or(|end| end > buffer.byte_len) {
                return Err(GraphicsError::IndexOutOfRange);
            }
            Ok(GraphicsAttributeInput { buffer, offset, stride })
        };
        let positions = attribute((
            positions.0.ok_or(GraphicsError::InvalidResourceHandle)?,
            positions.1,
            positions.2,
        ))?;
        let colors = colors.map(attribute).transpose()?;
        let indices = indices
            .map(|(handle, offset, format)| -> Result<_, GraphicsError> {
                let buffer = buffer(handle, ResourceStates::INDEX_BUFFER)?;
                (start as usize)
                    .checked_add(count as usize)
                    .and_then(|n| n.checked_mul(format.size()))
                    .and_then(|n| offset.checked_add(n))
                    .filter(|end| *end <= buffer.byte_len)
                    .ok_or(GraphicsError::IndexOutOfRange)?;
                Ok(GraphicsIndexInput {
                    buffer,
                    offset,
                    format,
                })
            })
            .transpose()?;
        // Known host indices (or linear vertices) permit all address checks now;
        // produced indices defer only content-dependent checks to guarded GPU loads.
        for row in 0..count as usize {
            let vertex = match &indices {
                None => Some(i64::from(start) + row as i64),
                Some(index) => match &index.buffer.source {
                    GraphicsCopyInput::HostDepth(_) => {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    GraphicsCopyInput::Produced(_) => None,
                    GraphicsCopyInput::Host(bytes) => {
                        let at = index.offset + (start as usize + row) * index.format.size();
                        let value = if index.format.size() == 2 {
                            u32::from(u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()))
                        } else {
                            u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
                        };
                        Some(i64::from(value) + i64::from(base_vertex))
                    }
                },
            };
            if let Some(vertex) = vertex {
                let vertex = usize::try_from(vertex).map_err(|_| GraphicsError::IndexOutOfRange)?;
                for attr in std::iter::once(&positions).chain(colors.iter()) {
                    let at = vertex
                        .checked_mul(attr.stride)
                        .and_then(|n| n.checked_add(attr.offset))
                        .filter(|at| {
                            at.checked_add(16)
                                .is_some_and(|end| end <= attr.buffer.byte_len)
                        })
                        .ok_or(GraphicsError::IndexOutOfRange)?;
                    if let GraphicsCopyInput::Host(bytes) = &attr.buffer.source
                        && bytes[at..at + 16]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|b| u32::from_le_bytes(*b) & 0x7f800000 == 0x7f800000)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                }
            }
        }
        Ok(Some(GraphicsGeometryInput {
            positions,
            colors,
            indices,
            start,
            count,
            base_vertex,
        }))
    }

    fn prepare_submission(
        &self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        memory: &GuestMemory,
        ordered: bool,
        backend_id: GraphicsBackendId,
        queued: Option<&QueuedPreparation>,
    ) -> Result<PreparedSubmission, GraphicsError> {
        if !self.queues.contains_key(&queue) {
            return Err(GraphicsError::InvalidQueueHandle);
        }
        let cmd_list = self
            .command_lists
            .get(&list)
            .ok_or(GraphicsError::InvalidCommandListHandle)?;

        if cmd_list.is_recording {
            return Err(GraphicsError::CommandListNotRecording);
        }
        if cmd_list.commands.is_empty() {
            return Err(GraphicsError::EmptyCommandList);
        }

        let vertex_limit = if backend_id == GraphicsBackendId::MetalNative {
            MAX_NATIVE_RENDER_VERTICES
        } else {
            MAX_SOFTWARE_RENDER_VERTICES
        };
        let pixel_limit = if backend_id == GraphicsBackendId::MetalNative {
            MAX_NATIVE_RENDER_PIXELS
        } else {
            MAX_SOFTWARE_RENDER_PIXELS
        };

        let attachments = self.attachment_plan(&cmd_list.commands)?;
        if attachments.is_some() && queued.is_none() {
            return Err(GraphicsError::NativeMetalRequired);
        }
        if let Some(plan) = &attachments {
            self.attachment_budget(plan, queued.expect("queued transaction"))?;
        }
        let commands = attachments
            .as_ref()
            .map_or(cmd_list.commands.as_slice(), |p| p.commands.as_slice());
        // State machine parsing recorded commands
        let mut viewport = (4usize, 3usize);
        let mut clear_color = [0u8, 0u8, 0u8, 255u8];
        let mut target_handle: Option<GraphicsHandle> = None;
        let mut color_targets = Vec::new();
        let mut strict_targets = false;
        let mut color_clears = vec![clear_color];
        let mut color_loads = vec![GraphicsColorLoad::Clear(clear_color)];
        let mut vertex_resource: Option<GraphicsHandle> = None;
        let mut vertex_offset = 0usize;
        let mut vertex_stride = 0usize;
        let mut index_buffer: Option<(GraphicsHandle, usize, IndexFormat)> = None;
        let mut pass_vertices: Vec<[f32; 4]> = Vec::new();
        let mut pass_draws = Vec::new();
        let mut geometry = Vec::new();
        let geometry_snapshot_budget = std::cell::Cell::new(0usize);
        let mut total_vertices = 0usize;
        let mut pass_extent = None;
        let mut draw_count = 0usize;
        let mut draw_state = GraphicsDrawState::default();
        let mut depth_clear = 1.0;
        let mut explicit_depth_clear = false;
        let mut depth_target = None;
        let mut depth_load = GraphicsDepthLoad::Clear(1.0);
        let mut pending_states = queued.map_or_else(HashMap::new, |q| q.states.clone());
        let mut pipeline_handle = None;
        let mut uniforms = None;
        let mut storage_view = None;
        let mut sampled_textures = [None; 2];
        let mut sampled_views = [None; 2];
        let mut sampled_array_views = [None; 2];
        let mut sampled_ranges = [None; 2];
        let mut sampled_samplers = [None; 2];
        let mut vertex_colors = None;
        let mut texture_snapshots = HashMap::new();
        if let Some(queued) = queued {
            for handle in queued.produced.keys() {
                texture_snapshots.insert(*handle, Arc::<[u8]>::from([]));
            }
        }

        for cmd in commands {
            match cmd {
                GraphicsCommand::SetWritableColorTargets { .. }
                | GraphicsCommand::SetWritableDepthTarget { .. } => {
                    unreachable!("normalized attachment commands")
                }
                GraphicsCommand::CopyResource { .. }
                | GraphicsCommand::CopyBufferRegion { .. }
                | GraphicsCommand::CopyImage { .. } => {
                    return Err(GraphicsError::NativeMetalRequired);
                }
                GraphicsCommand::SetGraphicsPipeline(handle) => {
                    pipeline_handle = *handle;
                }
                GraphicsCommand::SetStorageView(view) => {
                    if queued.is_none() || !self.shader_storage_views {
                        return Err(GraphicsError::NativeMetalRequired);
                    }
                    storage_view = *view;
                }
                GraphicsCommand::SetShaderUniforms(values) => {
                    uniforms = *values;
                }
                GraphicsCommand::SetSampledArrayRanges { ranges, samplers } => {
                    if queued.is_none() || !self.shader_array_views {
                        return Err(GraphicsError::NativeMetalRequired);
                    }
                    sampled_textures =
                        ranges.map(|h| h.map(|h| self.array_range_views[&h].backing));
                    sampled_ranges = ranges.map(|h| h.map(|h| self.array_range_views[&h].desc));
                    sampled_views = [None; 2];
                    sampled_array_views = [None; 2];
                    sampled_samplers = *samplers;
                }
                GraphicsCommand::SetSampledViews { views, samplers } => {
                    sampled_ranges = [None; 2];
                    sampled_textures =
                        views.map(|handle| handle.map(|h| self.texture_views[&h].backing));
                    sampled_views = views.map(|handle| {
                        handle.and_then(|h| {
                            let v = &self.texture_views[&h];
                            v.slice.is_none().then_some(v.desc)
                        })
                    });
                    sampled_array_views = views.map(|handle| {
                        handle.and_then(|h| {
                            let v = &self.texture_views[&h];
                            v.slice.map(|slice| GraphicsArraySliceViewDesc {
                                slice,
                                base_mip: v.desc.base_mip,
                                mip_count: v.desc.mip_count,
                                format: v.desc.format,
                            })
                        })
                    });
                    sampled_samplers = *samplers;
                }
                GraphicsCommand::SetSampledTexture { texture, sampler } => {
                    sampled_ranges[0] = None;
                    sampled_views[0] = None;
                    sampled_array_views[0] = None;
                    sampled_textures[0] = *texture;
                    sampled_samplers[0] = texture.map(|_| *sampler);
                }
                GraphicsCommand::SetSampledBindings { textures, samplers } => {
                    sampled_ranges = [None; 2];
                    sampled_views = [None; 2];
                    sampled_array_views = [None; 2];
                    sampled_textures = *textures;
                    sampled_samplers = *samplers;
                }
                GraphicsCommand::SetVertexColors {
                    resource,
                    offset,
                    stride,
                } => vertex_colors = Some((*resource, *offset, *stride)),
                GraphicsCommand::SetDepthTarget { target, load } => {
                    if draw_count != 0 || (target.is_some() && explicit_depth_clear) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if let Some(handle) = target {
                        let depth = self
                            .depth_resources
                            .get(handle)
                            .ok_or(GraphicsError::InvalidResourceHandle)?;
                        if attachments.is_none()
                            && (depth.typed_desc.mip_levels != 1
                                || depth.typed_desc.array_layers.is_some())
                        {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        let produced = queued.and_then(|q| q.produced.get(handle));
                        if produced.is_some_and(|version| {
                            !matches!(
                                version.output,
                                GraphicsProducedOutput::Depth
                                    | GraphicsProducedOutput::Copy(0)
                                    | GraphicsProducedOutput::AttachmentRoot(_)
                            )
                        }) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        if *load == GraphicsDepthLoad::Load
                            && depth.snapshot.is_none()
                            && produced.is_none()
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                        let state = pending_states.get(handle).copied().unwrap_or(depth.state);
                        if state != ResourceStates::DEPTH_WRITE
                            && (state != ResourceStates::DEPTH_READ
                                || !matches!(load, GraphicsDepthLoad::Load))
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                    }
                    depth_target = *target;
                    depth_load = *load;
                }
                GraphicsCommand::SetColorTargets { targets, loads } => {
                    if draw_count != 0 && (color_targets != *targets || color_loads != *loads) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    pass_extent = Some(if let Some(plan) = &attachments {
                        plan.extent
                    } else {
                        self.color_target_extent(targets, loads)?
                    });
                    // Validate at binding time, before later barriers transition to copy-source.
                    for handle in targets {
                        let resource = &self.resources[handle];
                        if pending_states
                            .get(handle)
                            .copied()
                            .unwrap_or(resource.state)
                            != ResourceStates::RENDER_TARGET
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                    }
                    strict_targets = true;
                    target_handle = Some(targets[0]);
                    color_loads = loads.clone();
                    color_clears = loads
                        .iter()
                        .map(|load| match load {
                            GraphicsColorLoad::Clear(clear) => *clear,
                            GraphicsColorLoad::Load | GraphicsColorLoad::Typed { .. } => [0; 4],
                        })
                        .collect();
                    clear_color = color_clears[0];
                    color_targets = targets.clone();
                }
                GraphicsCommand::SetDepthClear(value) => {
                    explicit_depth_clear = true;
                    if depth_target.is_some() {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if (draw_count != 0 && depth_clear != *value) || (!ordered && *value != 1.0) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    depth_clear = *value;
                }
                GraphicsCommand::SetDrawState(state) => {
                    if !ordered && *state != GraphicsDrawState::default() {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    draw_state = *state;
                }
                GraphicsCommand::ResourceBarrier {
                    resource,
                    before,
                    after,
                } => {
                    let initial = self
                        .resources
                        .get(resource)
                        .map(|r| r.state)
                        .or_else(|| self.depth_resources.get(resource).map(|r| r.state))
                        .ok_or(GraphicsError::InvalidResourceHandle)?;
                    if self.depth_resources.contains_key(resource)
                        && ![
                            ResourceStates::DEPTH_READ,
                            ResourceStates::DEPTH_WRITE,
                            ResourceStates::COMMON,
                            ResourceStates::COPY_SOURCE,
                            ResourceStates::COPY_DEST,
                            ResourceStates::PIXEL_SHADER_RESOURCE,
                        ]
                        .contains(after)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let current = pending_states.get(resource).copied().unwrap_or(initial);
                    if current != *before {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                    pending_states.insert(*resource, *after);
                }
                GraphicsCommand::SetRenderTargets {
                    color_target,
                    clear_color: cc,
                } => {
                    if color_target
                        .and_then(|handle| self.resources.get(&handle))
                        .and_then(|resource| resource.texture_format)
                        .is_some_and(|format| format != TextureFormat::Rgba8Unorm)
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }

                    if draw_count != 0
                        && (target_handle != *color_target
                            || clear_color != *cc
                            || color_loads != [GraphicsColorLoad::Clear(*cc)])
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if draw_count == 0 {
                        pass_extent = color_target
                            .and_then(|handle| self.resources.get(&handle))
                            .filter(|resource| {
                                resource.desc.dimension == ResourceDimension::Texture2D
                            })
                            .map(|resource| {
                                (resource.desc.width as usize, resource.desc.height as usize)
                            });
                    }
                    if draw_count == 0 {
                        strict_targets = false;
                    }
                    target_handle = *color_target;
                    clear_color = *cc;
                    color_targets = color_target.iter().copied().collect();
                    color_clears = vec![*cc];
                    color_loads = vec![GraphicsColorLoad::Clear(*cc)];
                }
                GraphicsCommand::SetViewport { width, height } => {
                    if !ordered && draw_count != 0 && viewport != (*width, *height) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    viewport = (*width, *height);
                }
                GraphicsCommand::SetVertexBuffer {
                    resource,
                    offset,
                    stride,
                } => {
                    vertex_resource = Some(*resource);
                    vertex_offset = *offset;
                    vertex_stride = *stride;
                }
                GraphicsCommand::SetIndexBuffer {
                    resource,
                    offset,
                    format,
                } => index_buffer = Some((*resource, *offset, *format)),
                GraphicsCommand::DrawIndexedInstanced {
                    index_count_per_instance,
                    instance_count,
                    start_index_location,
                    base_vertex_location,
                    start_instance_location,
                } => {
                    if *index_count_per_instance as usize > vertex_limit
                        || !index_count_per_instance.is_multiple_of(3)
                        || *instance_count > 1
                        || *start_instance_location != 0
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if *index_count_per_instance == 0 || *instance_count == 0 {
                        continue;
                    }
                    if draw_count >= MAX_NATIVE_RENDER_DRAWS {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    self.validate_storage_draw(StorageDrawValidation {
                        plan: attachments.as_ref(),
                        view: storage_view,
                        pipeline: pipeline_handle,
                        count: *index_count_per_instance,
                        previous_draws: draw_count,
                        viewport,
                        state: draw_state,
                        depth_active: depth_target.is_some() || explicit_depth_clear,
                        sampled: &sampled_textures,
                        states: &pending_states,
                        colors: &color_targets,
                    })?;
                    total_vertices = total_vertices
                        .checked_add(*index_count_per_instance as usize)
                        .filter(|count| *count <= vertex_limit)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    if !ordered
                        && (pipeline_handle.is_some()
                            || (sampled_textures.iter().any(Option::is_some)
                                || sampled_samplers.iter().any(Option::is_some))
                            || vertex_colors.is_some())
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if sampled_textures.iter().flatten().any(|handle| {
                        color_targets.contains(handle) || depth_target == Some(*handle)
                    }) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if pipeline_handle.is_some()
                        && color_targets.iter().any(|handle| {
                            pending_states
                                .get(handle)
                                .copied()
                                .unwrap_or(self.resources[handle].state)
                                != ResourceStates::RENDER_TARGET
                        })
                    {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }

                    let gather = self.prepare_geometry(
                        memory,
                        &geometry_snapshot_budget,
                        !geometry.iter().any(Option::is_some),
                        queued,
                        &pending_states,
                        (vertex_resource, vertex_offset, vertex_stride),
                        vertex_colors,
                        index_buffer,
                        *start_index_location,
                        *index_count_per_instance,
                        *base_vertex_location,
                    )?;
                    let vertices = if gather.is_some() {
                        Vec::new()
                    } else {
                        self.fetch_indexed_triangles(
                            memory,
                            index_buffer,
                            (vertex_resource, vertex_offset, vertex_stride),
                            (*start_index_location, *index_count_per_instance),
                            *base_vertex_location,
                        )?
                    };
                    if strict_targets
                        && color_targets.iter().any(|handle| {
                            pending_states
                                .get(handle)
                                .copied()
                                .unwrap_or(self.resources[handle].state)
                                != ResourceStates::RENDER_TARGET
                        })
                    {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                    if let Some(handle) = depth_target {
                        let depth = &self.depth_resources[&handle];
                        let state = pending_states.get(&handle).copied().unwrap_or(depth.state);
                        if state != ResourceStates::DEPTH_WRITE
                            && (state != ResourceStates::DEPTH_READ || draw_state.depth_write)
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                        if pass_extent.is_some_and(|extent| {
                            extent != attachments.as_ref().map_or(depth.extent, |p| p.extent)
                        }) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        pass_extent = Some(attachments.as_ref().map_or(depth.extent, |p| p.extent));
                    }
                    pass_extent = Some(Self::draw_extent(viewport, draw_state, pass_extent)?);
                    if ordered {
                        if let Some((handle, _, _)) = vertex_colors {
                            if pipeline_handle.is_none() {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            if pending_states
                                .get(&handle)
                                .copied()
                                .unwrap_or(self.resources[&handle].state)
                                != ResourceStates::VERTEX_AND_CONSTANT_BUFFER
                            {
                                return Err(GraphicsError::ResourceLifetimeViolation);
                            }
                        }
                        let (pipeline, sampled, sampled_bindings) = self.draw_bindings(
                            pipeline_handle,
                            (
                                sampled_textures,
                                sampled_samplers,
                                sampled_views,
                                sampled_array_views,
                                sampled_ranges,
                                uniforms,
                            ),
                            &pending_states,
                            &mut texture_snapshots,
                            &color_loads,
                            queued,
                        )?;
                        let colors = if gather.is_some() {
                            Vec::new()
                        } else if let Some((resource, offset, stride)) = vertex_colors {
                            self.fetch_indexed_triangles(
                                memory,
                                index_buffer,
                                (Some(resource), offset, stride),
                                (*start_index_location, *index_count_per_instance),
                                *base_vertex_location,
                            )?
                        } else {
                            vec![[0.0; 4]; vertices.len()]
                        };
                        pass_draws.push(GraphicsDraw {
                            uniforms,
                            colors,
                            pipeline,
                            sampled,
                            sampled_bindings,
                            vertices: vertices.clone(),
                            viewport,
                            state: draw_state,
                        });
                    }
                    geometry.push(gather);
                    pass_vertices.extend_from_slice(&vertices);
                    draw_count += 1;
                }
                GraphicsCommand::DrawInstanced {
                    vertex_count_per_instance,
                    instance_count,
                    start_vertex_location,
                    start_instance_location,
                } => {
                    // Bounded triangle lists only; per-instance attributes are unsupported.
                    if *vertex_count_per_instance as usize > vertex_limit
                        || !vertex_count_per_instance.is_multiple_of(3)
                        || *instance_count > 1
                        || *start_instance_location != 0
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if *vertex_count_per_instance == 0 || *instance_count == 0 {
                        continue;
                    }
                    if draw_count >= MAX_NATIVE_RENDER_DRAWS {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    self.validate_storage_draw(StorageDrawValidation {
                        plan: attachments.as_ref(),
                        view: storage_view,
                        pipeline: pipeline_handle,
                        count: *vertex_count_per_instance,
                        previous_draws: draw_count,
                        viewport,
                        state: draw_state,
                        depth_active: depth_target.is_some() || explicit_depth_clear,
                        sampled: &sampled_textures,
                        states: &pending_states,
                        colors: &color_targets,
                    })?;
                    total_vertices = total_vertices
                        .checked_add(*vertex_count_per_instance as usize)
                        .filter(|count| *count <= vertex_limit)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    if !ordered
                        && (pipeline_handle.is_some()
                            || (sampled_textures.iter().any(Option::is_some)
                                || sampled_samplers.iter().any(Option::is_some))
                            || vertex_colors.is_some())
                    {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if sampled_textures.iter().flatten().any(|handle| {
                        color_targets.contains(handle) || depth_target == Some(*handle)
                    }) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    if pipeline_handle.is_some()
                        && color_targets.iter().any(|handle| {
                            pending_states
                                .get(handle)
                                .copied()
                                .unwrap_or(self.resources[handle].state)
                                != ResourceStates::RENDER_TARGET
                        })
                    {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }

                    let gather = self.prepare_geometry(
                        memory,
                        &geometry_snapshot_budget,
                        !geometry.iter().any(Option::is_some),
                        queued,
                        &pending_states,
                        (vertex_resource, vertex_offset, vertex_stride),
                        vertex_colors,
                        None,
                        *start_vertex_location,
                        *vertex_count_per_instance,
                        0,
                    )?;
                    let vertices = if gather.is_some() {
                        Vec::new()
                    } else {
                        let v_res_handle =
                            vertex_resource.ok_or(GraphicsError::InvalidResourceHandle)?;
                        let v_res = self
                            .resources
                            .get(&v_res_handle)
                            .ok_or(GraphicsError::InvalidResourceHandle)?;
                        if vertex_stride < 16 {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        (*start_vertex_location as usize)
                            .checked_add(*vertex_count_per_instance as usize - 1)
                            .and_then(|last| last.checked_mul(vertex_stride))
                            .and_then(|bytes| bytes.checked_add(vertex_offset))
                            .and_then(|bytes| bytes.checked_add(16))
                            .filter(|end| *end <= v_res.data.len())
                            .ok_or(GraphicsError::IndexOutOfRange)?;
                        let mut vertices = vec![[0.0f32; 4]; *vertex_count_per_instance as usize];
                        for (i, row) in vertices.iter_mut().enumerate() {
                            let at = (*start_vertex_location as usize)
                                .checked_add(i)
                                .and_then(|vertex| vertex.checked_mul(vertex_stride))
                                .and_then(|bytes| bytes.checked_add(vertex_offset))
                                .ok_or(GraphicsError::IndexOutOfRange)?;
                            let mut raw = [0u8; 16];
                            Self::read_resource(v_res, memory, at, &mut raw)?;
                            for (value, bytes) in row.iter_mut().zip(raw.as_chunks::<4>().0) {
                                *value = f32::from_le_bytes(*bytes);
                            }
                        }
                        vertices
                    };
                    if strict_targets
                        && color_targets.iter().any(|handle| {
                            pending_states
                                .get(handle)
                                .copied()
                                .unwrap_or(self.resources[handle].state)
                                != ResourceStates::RENDER_TARGET
                        })
                    {
                        return Err(GraphicsError::ResourceLifetimeViolation);
                    }
                    if let Some(handle) = depth_target {
                        let depth = &self.depth_resources[&handle];
                        let state = pending_states.get(&handle).copied().unwrap_or(depth.state);
                        if state != ResourceStates::DEPTH_WRITE
                            && (state != ResourceStates::DEPTH_READ || draw_state.depth_write)
                        {
                            return Err(GraphicsError::ResourceLifetimeViolation);
                        }
                        if pass_extent.is_some_and(|extent| {
                            extent != attachments.as_ref().map_or(depth.extent, |p| p.extent)
                        }) {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                        pass_extent = Some(attachments.as_ref().map_or(depth.extent, |p| p.extent));
                    }
                    pass_extent = Some(Self::draw_extent(viewport, draw_state, pass_extent)?);
                    if ordered {
                        if let Some((handle, _, _)) = vertex_colors {
                            if pipeline_handle.is_none() {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            if pending_states
                                .get(&handle)
                                .copied()
                                .unwrap_or(self.resources[&handle].state)
                                != ResourceStates::VERTEX_AND_CONSTANT_BUFFER
                            {
                                return Err(GraphicsError::ResourceLifetimeViolation);
                            }
                        }
                        let (pipeline, sampled, sampled_bindings) = self.draw_bindings(
                            pipeline_handle,
                            (
                                sampled_textures,
                                sampled_samplers,
                                sampled_views,
                                sampled_array_views,
                                sampled_ranges,
                                uniforms,
                            ),
                            &pending_states,
                            &mut texture_snapshots,
                            &color_loads,
                            queued,
                        )?;
                        let colors = if gather.is_some() {
                            Vec::new()
                        } else {
                            self.fetch_colors(
                                memory,
                                vertex_colors,
                                *start_vertex_location as usize,
                                vertices.len(),
                            )?
                        };
                        pass_draws.push(GraphicsDraw {
                            uniforms,
                            colors,
                            pipeline,
                            sampled,
                            sampled_bindings,
                            vertices: vertices.clone(),
                            viewport,
                            state: draw_state,
                        });
                    }
                    geometry.push(gather);
                    pass_vertices.extend_from_slice(&vertices);
                    draw_count += 1;
                }
            }
        }

        if attachments.as_ref().is_some_and(|p| p.storage.is_some())
            && (draw_count != 1 || explicit_depth_clear || depth_target.is_some())
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if !ordered
            && (pipeline_handle.is_some()
                || (sampled_textures.iter().any(Option::is_some)
                    || sampled_samplers.iter().any(Option::is_some))
                || vertex_colors.is_some())
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        // Each draw snapshots its pipeline; the attachment set stays fixed per submission.
        // Validate/fetch all draws before issuing one triangle-list render.
        // Legacy callbacks use one viewport for both geometry and output extent.
        // Reject layouts they cannot preserve rather than packing narrow rows into
        // a wider texture. Ordered adapters retain both dimensions independently.
        if !ordered && draw_count != 0 && pass_extent.is_some_and(|extent| extent != viewport) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if let Some(handle) = depth_target {
            let extent = attachments
                .as_ref()
                .map_or(self.depth_resources[&handle].extent, |p| p.extent);
            if pass_extent.is_some_and(|current| current != extent) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            pass_extent = Some(extent);
        }
        let viewport = pass_extent.unwrap_or(viewport);
        if viewport.0 > MAX_RENDER_DIMENSION || viewport.1 > MAX_RENDER_DIMENSION {
            return Err(GraphicsError::InvalidViewport);
        }
        let pixel_count = viewport
            .0
            .checked_mul(viewport.1)
            .filter(|&count| (1..=pixel_limit).contains(&count))
            .ok_or(GraphicsError::InvalidViewport)?;
        let byte_count = pixel_count * 4;
        let color_formats: Vec<_> = color_loads
            .iter()
            .map(|load| match load {
                GraphicsColorLoad::Typed { attachment, .. } => attachment.format,
                _ => TextureFormat::Rgba8Unorm,
            })
            .collect();
        let color_bytes: Vec<_> = color_formats
            .iter()
            .map(|format| pixel_count * format.block_bytes())
            .collect();
        for (slot, target) in color_targets.iter().enumerate() {
            let resource = self
                .resources
                .get(target)
                .ok_or(GraphicsError::InvalidResourceHandle)?;
            if resource.data.len() < color_bytes[slot] {
                return Err(GraphicsError::IndexOutOfRange);
            }
        }
        let mut color_bindings = Vec::with_capacity(color_loads.len());
        let mut queued_colors = Vec::with_capacity(color_loads.len());
        for (slot, action) in color_loads.iter().enumerate() {
            let (typed, load, clear) = match action {
                GraphicsColorLoad::Clear(clear) => (None, false, *clear),
                GraphicsColorLoad::Load => (None, true, [0; 4]),
                GraphicsColorLoad::Typed { attachment, load } => (Some(*attachment), *load, [0; 4]),
            };
            let target = color_targets.get(slot).copied();
            let (previous, input) = if !load || attachments.is_some() {
                (None, GraphicsQueuedColorInput::Clear(clear))
            } else if let Some(version) =
                target.and_then(|t| queued.and_then(|q| q.produced.get(&t)))
            {
                let target = target.expect("produced target");
                let desc = GraphicsTypedTextureDesc {
                    format: color_formats[slot],
                    extent: viewport,
                    mip_levels: 1,
                    array_layers: None,
                };
                if !matches!(
                    version.output,
                    GraphicsProducedOutput::Color(_)
                        | GraphicsProducedOutput::Copy(0)
                        | GraphicsProducedOutput::AttachmentRoot(_)
                ) || (matches!(version.output, GraphicsProducedOutput::Copy(_))
                    && !self.copy_backend)
                    || self.resources[&target].desc.mip_levels != 1
                    || queued.expect("produced queue").layouts.get(&target)
                        != Some(&desc.produced())
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                (None, GraphicsQueuedColorInput::Produced(*version))
            } else {
                let target = target.ok_or(GraphicsError::InvalidDescriptor)?;
                let bytes: Arc<[u8]> =
                    Arc::from(&self.resources[&target].data[..color_bytes[slot]]);
                (Some(bytes.clone()), GraphicsQueuedColorInput::Host(bytes))
            };
            color_bindings.push(GraphicsColorBinding {
                typed,
                clear,
                previous,
            });
            queued_colors.push(input);
        }
        if let Some(queued) = queued {
            for handle in queued.states.keys() {
                if !cmd_list.commands.iter().any(|cmd| {
                    matches!(cmd,
                    GraphicsCommand::ResourceBarrier { resource, .. } if resource == handle)
                }) {
                    pending_states.remove(handle);
                }
            }
        }
        let queued_depth = if attachments.is_some() {
            depth_target.map(|_| GraphicsQueuedDepthInput::Clear(1.0))
        } else if queued.is_some() {
            depth_target
                .map(|handle| -> Result<_, GraphicsError> {
                    Ok(match depth_load {
                        GraphicsDepthLoad::Clear(value) => GraphicsQueuedDepthInput::Clear(value),
                        GraphicsDepthLoad::Load => {
                            if let Some(version) = queued.and_then(|q| q.produced.get(&handle)) {
                                if !matches!(
                                    version.output,
                                    GraphicsProducedOutput::Depth
                                        | GraphicsProducedOutput::Copy(0)
                                        | GraphicsProducedOutput::AttachmentRoot(_)
                                ) || queued.unwrap().layouts.get(&handle)
                                    != Some(&GraphicsProducedLayout::Depth32Float {
                                        extent: viewport,
                                    })
                                {
                                    return Err(GraphicsError::InvalidDescriptor);
                                }
                                GraphicsQueuedDepthInput::Produced(*version)
                            } else {
                                GraphicsQueuedDepthInput::Host(
                                    self.depth_resources[&handle]
                                        .snapshot
                                        .clone()
                                        .ok_or(GraphicsError::ResourceLifetimeViolation)?,
                                )
                            }
                        }
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(PreparedSubmission {
            attachments,
            geometry,
            queued_depth,
            queued_colors,
            request: GraphicsSubmissionRequest {
                draws: pass_draws,
                extent: viewport,
                colors: color_bindings,
                clear_depth: depth_clear,
                depth: depth_target.map(|handle| GraphicsDepthBinding {
                    load: depth_load,
                    previous: self.depth_resources[&handle].snapshot.clone(),
                }),
            },
            vertices: pass_vertices,
            commit: SubmissionCommit {
                serial: None,
                queue,
                list,
                targets: color_targets,
                depth: depth_target,
                states: pending_states,
                extent: viewport,
                outputs: color_clears.len(),
                byte_count,
                color_bytes,
                color_formats,
                typed_colors: color_loads
                    .iter()
                    .any(|l| matches!(l, GraphicsColorLoad::Typed { .. })),
                draw_count,
                backend: backend_id,
            },
        })
    }

    fn execute_command_list_internal(
        &mut self,
        queue: GraphicsHandle,
        list: GraphicsHandle,
        memory: &GuestMemory,
        backend_renderer: Option<&BackendRenderer<'_>>,
        pass_renderer: Option<&AttachmentRenderer>,
        backend_id: GraphicsBackendId,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        if self.has_pending_submission() {
            return Err(GraphicsError::ResourceLifetimeViolation);
        }
        let prepared = self.prepare_submission(
            queue,
            list,
            memory,
            pass_renderer.is_some(),
            backend_id,
            None,
        )?;
        let request = prepared.request;
        let output = if prepared.vertices.is_empty()
            && request.depth.is_none()
            && request.colors.iter().all(|c| c.typed.is_none())
        {
            GraphicsPassOutput {
                color_layouts: None,
                colors: request
                    .colors
                    .iter()
                    .map(|binding| match &binding.previous {
                        Some(bytes) => bytes.to_vec(),
                        None => binding.clear.repeat(prepared.commit.byte_count / 4),
                    })
                    .collect(),
                depth: None,
            }
        } else if let Some(renderer) = pass_renderer {
            renderer(
                &request.draws,
                request.extent,
                &request.colors,
                request.clear_depth,
                request.depth,
            )?
        } else {
            if request.colors.len() != 1
                || request.depth.is_some()
                || request
                    .colors
                    .iter()
                    .any(|binding| binding.previous.is_some())
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            let clear = request.colors[0].clear;
            let pixels = if let Some(renderer) = backend_renderer {
                renderer(&prepared.vertices, request.extent, clear)?
            } else if backend_id == GraphicsBackendId::MetalNative {
                return Err(GraphicsError::NativeMetalRequired);
            } else {
                Self::execute_software_reference_triangles(
                    &prepared.vertices,
                    request.extent,
                    clear,
                )?
            };
            GraphicsPassOutput {
                color_layouts: None,
                colors: vec![pixels],
                depth: None,
            }
        };
        self.commit_submission(prepared.commit, output)
    }

    fn commit_submission(
        &mut self,
        commit: SubmissionCommit,
        output: GraphicsPassOutput,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        let expected_layouts: Vec<_> = commit
            .color_formats
            .iter()
            .map(|format| GraphicsTypedTextureDesc {
                format: *format,
                extent: commit.extent,
                mip_levels: 1,
                array_layers: None,
            })
            .collect();
        if (commit.typed_colors && output.color_layouts.is_none())
            || output
                .color_layouts
                .as_ref()
                .is_some_and(|actual| *actual != expected_layouts)
            || output.colors.len() != commit.outputs
            || output
                .colors
                .iter()
                .zip(&commit.color_bytes)
                .any(|(bytes, expected)| bytes.len() != *expected)
            || output.depth.is_some() != commit.depth.is_some()
        {
            return Err(GraphicsError::BackendError(
                "invalid submission outputs".into(),
            ));
        }
        if let Some(depth) = output
            .depth
            .as_ref()
            .and_then(|d| d.downcast_ref::<GraphicsTypedDepthSnapshot>())
        {
            let expected = GraphicsTypedTextureDesc {
                format: TextureFormat::Depth32Float,
                extent: commit.extent,
                mip_levels: 1,
                array_layers: None,
            };
            if depth.desc != expected || depth.bytes.len() != expected.byte_len()? {
                return Err(GraphicsError::BackendError(
                    "invalid depth output layout".into(),
                ));
            }
        }
        let host_publications = if commit.serial.is_none() {
            commit
                .targets
                .iter()
                .copied()
                .chain(commit.depth)
                .map(|root| {
                    self.publication(root)
                        .host_epoch
                        .checked_add(1)
                        .map(|host_epoch| {
                            (
                                root,
                                RootPublication {
                                    host_epoch,
                                    version: None,
                                },
                            )
                        })
                        .ok_or(GraphicsError::InvalidDescriptor)
                })
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        // All destinations are pinned or synchronously validated. Acquire the only
        // fallible commit lock before touching colors, depth, barriers or history.
        let mut presentation = self
            .last_presentation
            .lock()
            .map_err(|_| GraphicsError::BackendError("presentation lock poisoned".into()))?;
        for (target, bytes) in commit.targets.iter().zip(&output.colors) {
            // Destruction invalidates the guest destination permanently. Native
            // produced versions remain owned by tickets for admitted consumers.
            if let Some(resource) = self.resources.get_mut(target) {
                resource.data[..bytes.len()].copy_from_slice(bytes);
            }
        }
        if let Some(handle) = commit.depth
            && let Some(depth) = self.depth_resources.get_mut(&handle)
        {
            depth.snapshot = output.depth;
        }
        for (handle, state) in commit.states {
            if let Some(resource) = self.resources.get_mut(&handle) {
                resource.state = state;
            } else if let Some(depth) = self.depth_resources.get_mut(&handle) {
                depth.state = state;
            }
        }
        for (root, publication) in host_publications {
            self.root_publications.insert(root, publication);
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
        let pixels = output.colors[0].clone();
        if commit.color_formats[0] == TextureFormat::Rgba8Unorm
            && commit
                .serial
                .is_none_or(|serial| serial >= self.published_serial)
        {
            if let Some(serial) = commit.serial {
                self.published_serial = serial;
            }
            presentation.width = commit.extent.0;
            presentation.height = commit.extent.1;
            presentation.pixels = pixels.clone();
            presentation.frame_index += 1;
            presentation.backend = Some(commit.backend);
        }
        Ok(ExecutionOutcome {
            backend: commit.backend,
            pixels,
            color_outputs: output.colors,
            width: commit.extent.0,
            height: commit.extent.1,
            draw_count: commit.draw_count,
        })
    }

    fn fetch_colors(
        &self,
        memory: &GuestMemory,
        binding: Option<(GraphicsHandle, usize, usize)>,
        start: usize,
        count: usize,
    ) -> Result<Vec<[f32; 4]>, GraphicsError> {
        let Some((handle, offset, stride)) = binding else {
            return Ok(vec![[0.0; 4]; count]);
        };
        let resource = self
            .resources
            .get(&handle)
            .ok_or(GraphicsError::InvalidResourceHandle)?;
        let mut colors = Vec::with_capacity(count);
        for index in 0..count {
            let at = start
                .checked_add(index)
                .and_then(|i| i.checked_mul(stride))
                .and_then(|n| n.checked_add(offset))
                .ok_or(GraphicsError::IndexOutOfRange)?;
            let mut bytes = [0; 16];
            Self::read_resource(resource, memory, at, &mut bytes)?;
            colors.push(std::array::from_fn(|channel| {
                f32::from_le_bytes(bytes[channel * 4..channel * 4 + 4].try_into().unwrap())
            }));
        }
        Ok(colors)
    }

    fn draw_bindings(
        &self,
        pipeline: Option<GraphicsHandle>,
        sampled: RecordedSampledBindings,
        states: &HashMap<GraphicsHandle, ResourceStates>,
        snapshots: &mut HashMap<GraphicsHandle, Arc<[u8]>>,
        color_loads: &[GraphicsColorLoad],
        queued: Option<&QueuedPreparation>,
    ) -> Result<DrawBindings, GraphicsError> {
        let outputs = color_loads.len();
        let (textures, samplers, views, array_views, sampled_ranges, uniforms) = sampled;
        if sampled_ranges.iter().any(Option::is_some)
            && (queued.is_none() || !self.shader_array_views)
        {
            return Err(GraphicsError::NativeMetalRequired);
        }
        for slot in 0..2 {
            if usize::from(views[slot].is_some())
                + usize::from(array_views[slot].is_some())
                + usize::from(sampled_ranges[slot].is_some())
                > 1
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        if array_views.iter().any(Option::is_some) && !self.array_view_backend {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if views.iter().any(Option::is_some) && !self.view_backend {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let pipeline = pipeline
            .map(|h| {
                self.pipelines
                    .get(&h)
                    .cloned()
                    .ok_or(GraphicsError::InvalidResourceHandle)
            })
            .transpose()?;
        if pipeline
            .as_ref()
            .is_some_and(|p| p.required_uniform_mask != 0)
            && (!self.uniform_backend || uniforms.is_none())
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        for (slot, load) in color_loads.iter().enumerate() {
            let uint = matches!(load, GraphicsColorLoad::Typed { attachment, .. } if attachment.format == TextureFormat::R32Uint);
            let export = pipeline
                .as_ref()
                .map_or(xenora_shader::GraphicsExportType::Float, |p| {
                    p.types.colors[slot]
                });
            if uint != (export == xenora_shader::GraphicsExportType::Uint) {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        let pairs = pipeline.as_ref().map_or(0, |p| p.sampled_pairs);
        if pairs & !15 != 0
            || (!self.multislot_backend && pairs & !1 != 0)
            || pipeline
                .as_ref()
                .is_some_and(|p| p.color_targets != outputs)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        for slot in 0..2 {
            let needs_texture = pairs & (3 << (slot * 2)) != 0;
            let needs_sampler = pairs & (5 << slot) != 0;
            if textures[slot].is_some() != needs_texture
                || samplers[slot].is_some() != needs_sampler
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        let mut bindings = GraphicsSampledBindings {
            sampled_ranges,
            array_views,
            views,
            samplers,
            ..Default::default()
        };
        for (slot, handle) in textures.into_iter().enumerate() {
            if let Some(handle) = handle {
                let texture = self.sampled_resource_desc(handle)?;
                let format = texture.format;
                if !self.texture_capabilities.supports(format)
                    && !self.resource_roles.sampled(format)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let expected = match format {
                    TextureFormat::R32Uint if sampled_ranges[slot].is_some() => {
                        xenora_shader::GraphicsSampleType::UintArray
                    }
                    TextureFormat::R32Uint => xenora_shader::GraphicsSampleType::Uint,
                    TextureFormat::Depth32Float => xenora_shader::GraphicsSampleType::Depth,
                    _ if sampled_ranges[slot].is_some() => {
                        xenora_shader::GraphicsSampleType::FloatArray
                    }
                    _ => xenora_shader::GraphicsSampleType::Float,
                };
                if pipeline
                    .as_ref()
                    .is_none_or(|p| p.types.textures[slot] != expected)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let state = self
                    .resources
                    .get(&handle)
                    .map(|r| r.state)
                    .or_else(|| self.depth_resources.get(&handle).map(|r| r.state))
                    .ok_or(GraphicsError::InvalidResourceHandle)?;
                if states.get(&handle).copied().unwrap_or(state)
                    != ResourceStates::PIXEL_SHADER_RESOURCE
                {
                    return Err(GraphicsError::ResourceLifetimeViolation);
                }
                if texture.array_layers.is_some()
                    != (array_views[slot].is_some() || sampled_ranges[slot].is_some())
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                if let Some(range) = sampled_ranges[slot] {
                    if format == TextureFormat::Depth32Float {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let pipeline = pipeline.as_ref().ok_or(GraphicsError::InvalidDescriptor)?;
                    for layer in &pipeline.array_layers[slot] {
                        let valid = match *layer {
                            xenora_shader::GraphicsArrayLayer::Constant(value) => {
                                value < range.slice_count
                            }
                            xenora_shader::GraphicsArrayLayer::Uniform(index) => {
                                let bits = uniforms
                                    .and_then(|values| values.get(usize::from(index)).copied())
                                    .ok_or(GraphicsError::InvalidDescriptor)?;
                                let value = f32::from_bits(bits);
                                value.is_finite()
                                    && value >= 0.0
                                    && value.fract() == 0.0
                                    && f64::from(value) < f64::from(range.slice_count)
                            }
                        };
                        if !valid {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                    }
                }
                let effective_view = views[slot]
                    .or(sampled_ranges[slot].map(|v| GraphicsTextureViewDesc {
                        base_mip: v.base_mip,
                        mip_count: v.mip_count,
                        format: v.format,
                    }))
                    .or(array_views[slot].map(|v| GraphicsTextureViewDesc {
                        base_mip: v.base_mip,
                        mip_count: v.mip_count,
                        format: v.format,
                    }));
                let (width, height, mips) = if let Some(view) = effective_view {
                    (
                        (texture.extent.0 >> view.base_mip).max(1),
                        (texture.extent.1 >> view.base_mip).max(1),
                        view.mip_count,
                    )
                } else {
                    (texture.extent.0, texture.extent.1, texture.mip_levels)
                };
                let metadata = xenora_texture::TextureMetadata::new(width, height, mips)
                    .map_err(|_| GraphicsError::InvalidDescriptor)?;
                for (sampler_slot, sampler) in samplers.iter().enumerate() {
                    if pairs & (1 << (slot * 2 + sampler_slot)) != 0 {
                        sampler
                            .as_ref()
                            .ok_or(GraphicsError::InvalidDescriptor)?
                            .validate_against_texture(&metadata)
                            .map_err(|_| GraphicsError::InvalidDescriptor)?;
                    }
                }
                let pixels = if queued.is_some_and(|q| q.produced.contains_key(&handle)) {
                    Arc::from([])
                } else if let Some(bytes) = snapshots.get(&handle) {
                    bytes.clone()
                } else {
                    let bytes = self.sampled_resource_bytes(handle)?;
                    snapshots.insert(handle, bytes.clone());
                    bytes
                };
                bindings.textures[slot] = Some(GraphicsTextureSnapshot {
                    array_layers: texture.array_layers,
                    format,
                    width: texture.extent.0,
                    height: texture.extent.1,
                    pixels,
                    mip_levels: texture.mip_levels,
                });
            }
        }
        let sampled = if pairs == 1
            && views[0].is_none()
            && array_views[0].is_none()
            && sampled_ranges[0].is_none()
            && bindings.textures[0]
                .as_ref()
                .is_some_and(|t| t.format == TextureFormat::Rgba8Unorm)
        {
            let texture = bindings.textures[0]
                .as_ref()
                .ok_or(GraphicsError::InvalidDescriptor)?;
            Some(GraphicsSampledTexture {
                width: texture.width,
                height: texture.height,
                pixels: texture.pixels.clone(),
                mip_levels: texture.mip_levels,
                sampler: samplers[0].ok_or(GraphicsError::InvalidDescriptor)?,
            })
        } else {
            None
        };
        Ok((pipeline, sampled, bindings))
    }

    fn draw_extent(
        viewport: (usize, usize),
        state: GraphicsDrawState,
        fixed: Option<(usize, usize)>,
    ) -> Result<(usize, usize), GraphicsError> {
        let right = state.viewport_origin[0]
            .checked_add(viewport.0)
            .ok_or(GraphicsError::InvalidViewport)?;
        let bottom = state.viewport_origin[1]
            .checked_add(viewport.1)
            .ok_or(GraphicsError::InvalidViewport)?;
        let extent = fixed.unwrap_or((right, bottom));
        if extent.0 > MAX_RENDER_DIMENSION
            || extent.1 > MAX_RENDER_DIMENSION
            || right > extent.0
            || bottom > extent.1
            || state.scissor.is_some_and(|[x, y, w, h]| {
                x.checked_add(w).is_none_or(|right| right > extent.0)
                    || y.checked_add(h).is_none_or(|bottom| bottom > extent.1)
            })
        {
            return Err(GraphicsError::InvalidViewport);
        }
        Ok(extent)
    }

    /// Fetch float4 positions for the validated bounded index range: index `k` is
    /// read at `offset + (start + k) * size` of the index buffer, and vertex
    /// `index + base_vertex` at `offset + vertex * stride` of the vertex buffer.
    fn fetch_indexed_triangles(
        &self,
        memory: &GuestMemory,
        index_buffer: Option<(GraphicsHandle, usize, IndexFormat)>,
        (vertex_resource, vertex_offset, vertex_stride): (Option<GraphicsHandle>, usize, usize),
        (start, count): (u32, u32),
        base_vertex: i32,
    ) -> Result<Vec<[f32; 4]>, GraphicsError> {
        let resource = |handle: Option<GraphicsHandle>| {
            handle
                .and_then(|handle| self.resources.get(&handle))
                .ok_or(GraphicsError::InvalidResourceHandle)
        };
        let (index_handle, index_offset, format) =
            index_buffer.ok_or(GraphicsError::InvalidResourceHandle)?;
        let index_res = resource(Some(index_handle))?;
        let vertex_res = resource(vertex_resource)?;
        if vertex_stride < 16 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let size = format.size();
        let byte_count = (count as usize)
            .checked_mul(size)
            .ok_or(GraphicsError::IndexOutOfRange)?;
        let first = (start as usize)
            .checked_mul(size)
            .and_then(|bytes| bytes.checked_add(index_offset))
            .ok_or(GraphicsError::IndexOutOfRange)?;
        first
            .checked_add(byte_count)
            .filter(|end| *end <= index_res.data.len())
            .ok_or(GraphicsError::IndexOutOfRange)?;
        let mut raw_indices = vec![0u8; byte_count];
        Self::read_resource(index_res, memory, first, &mut raw_indices)?;
        let mut vertices = vec![[0.0f32; 4]; count as usize];
        for (row, index) in vertices.iter_mut().zip(raw_indices.chunks_exact(size)) {
            let index = if size == 2 {
                u32::from(u16::from_le_bytes([index[0], index[1]]))
            } else {
                u32::from_le_bytes(index.try_into().unwrap())
            };
            let vertex = usize::try_from(i64::from(index) + i64::from(base_vertex))
                .map_err(|_| GraphicsError::IndexOutOfRange)?;
            let at = vertex
                .checked_mul(vertex_stride)
                .and_then(|bytes| bytes.checked_add(vertex_offset))
                .ok_or(GraphicsError::IndexOutOfRange)?;
            let mut raw = [0u8; 16];
            Self::read_resource(vertex_res, memory, at, &mut raw)?;
            for (value, bytes) in row.iter_mut().zip(raw.as_chunks::<4>().0) {
                *value = f32::from_le_bytes(*bytes);
            }
        }
        Ok(vertices)
    }

    /// Read `out.len()` bytes at `offset` of a resource, from its guest backing
    /// when it has one. The range must lie inside the resource's size.
    fn read_resource(
        res: &GraphicsResource,
        memory: &GuestMemory,
        offset: usize,
        out: &mut [u8],
    ) -> Result<(), GraphicsError> {
        let end = offset
            .checked_add(out.len())
            .filter(|&end| end <= res.data.len())
            .ok_or(GraphicsError::IndexOutOfRange)?;
        match res.guest_backing_addr {
            Some(guest_addr) => {
                let address = guest_addr
                    .checked_add(offset as u64)
                    .filter(|address| address.checked_add(out.len() as u64).is_some())
                    .ok_or(GraphicsError::IndexOutOfRange)?;
                memory.read_bytes(address, out)?;
            }
            None => out.copy_from_slice(&res.data[offset..end]),
        }
        Ok(())
    }

    /// Software reference rasterizer for deterministic test verification when GPU tests are off.
    pub fn execute_software_reference_draw(
        vertices: &[[f32; 4]; 3],
        viewport: (usize, usize),
        clear_color: [u8; 4],
    ) -> Result<Vec<u8>, GraphicsError> {
        Self::execute_software_reference_triangles(vertices, viewport, clear_color)
    }

    /// Bounded triangle-list reference; clears once and rasterizes all triangles.
    pub fn execute_software_reference_triangles(
        vertices: &[[f32; 4]],
        viewport: (usize, usize),
        clear_color: [u8; 4],
    ) -> Result<Vec<u8>, GraphicsError> {
        if vertices.is_empty()
            || vertices.len() > MAX_SOFTWARE_RENDER_VERTICES
            || !vertices.len().is_multiple_of(3)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let (w, h) = viewport;
        if w == 0 || h == 0 || w > MAX_RENDER_DIMENSION || h > MAX_RENDER_DIMENSION {
            return Err(GraphicsError::InvalidViewport);
        }
        let pixels_count = w
            .checked_mul(h)
            .filter(|count| (1..=MAX_SOFTWARE_RENDER_PIXELS).contains(count))
            .ok_or(GraphicsError::InvalidViewport)?;
        let mut pixels = clear_color.repeat(pixels_count);

        for vertices in vertices.as_chunks::<3>().0 {
            // Convert clip-space coordinates to window pixels [0, w-1] x [0, h-1]
            let p0 = [
                (vertices[0][0] + 1.0) * 0.5 * w as f32,
                (1.0 - vertices[0][1]) * 0.5 * h as f32,
            ];
            let p1 = [
                (vertices[1][0] + 1.0) * 0.5 * w as f32,
                (1.0 - vertices[1][1]) * 0.5 * h as f32,
            ];
            let p2 = [
                (vertices[2][0] + 1.0) * 0.5 * w as f32,
                (1.0 - vertices[2][1]) * 0.5 * h as f32,
            ];

            // 2D barycentric edge functions
            let edge = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> f32 {
                (c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])
            };

            let area = edge(p0, p1, p2);
            if area.abs() < 1e-5 {
                continue;
            }

            for y in 0..h {
                for x in 0..w {
                    let pt = [x as f32 + 0.5, y as f32 + 0.5];
                    let w0 = edge(p1, p2, pt);
                    let w1 = edge(p2, p0, pt);
                    let w2 = edge(p0, p1, pt);

                    let inside = if area > 0.0 {
                        w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
                    } else {
                        w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
                    };

                    if inside {
                        let idx = (y * w + x) * 4;
                        pixels[idx..idx + 4].copy_from_slice(&[255, 0, 0, 255]);
                    }
                }
            }
        }
        Ok(pixels)
    }
}

#[cfg(test)]
mod submission_history_tests {
    use super::*;

    #[test]
    fn guest_copy_budget_rejection_precedes_guest_memory_read() {
        struct Pending;
        impl GraphicsQueuedPendingWork for Pending {
            fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
                None
            }
            fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
                Err(GraphicsError::InvalidDescriptor)
            }
            fn reject_dependency(&mut self, _: GraphicsQueuePoint) {}
        }
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let captured = calls.clone();
        ctx.set_metal_copy_queued_backend(Default::default(), move |_| {
            captured.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::new(Pending))
        });
        let queues: [GraphicsHandle; 2] = std::array::from_fn(|_| {
            ctx.create_command_queue(device, CommandListType::Direct)
                .unwrap()
        });
        let lists: [GraphicsHandle; 2] = std::array::from_fn(|_| {
            let allocator = ctx
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap();
            ctx.create_command_list(device, allocator, CommandListType::Direct)
                .unwrap()
        });
        let desc = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 5,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        for (slot, address) in [None, Some(0xdead0000)].into_iter().enumerate() {
            let source = ctx
                .create_committed_resource(
                    device,
                    desc.clone(),
                    ResourceStates::COPY_SOURCE,
                    address,
                )
                .unwrap();
            let destination = ctx
                .create_committed_resource(device, desc.clone(), ResourceStates::COPY_DEST, None)
                .unwrap();
            ctx.cmd_copy_resource(lists[slot], destination, source)
                .unwrap();
            ctx.close_command_list(lists[slot]).unwrap();
        }
        let memory = GuestMemory::new();
        let receipt = ctx
            .submit_command_list(queues[0], lists[0], &memory)
            .unwrap();
        let charge = ctx.queued[&receipt].copy_allocation_bytes;
        // Inject ledger saturation to test ordering without allocating 256MiB.
        ctx.queued.get_mut(&receipt).unwrap().copy_allocation_bytes = MAX_CONTEXT_RESOURCE_BYTES;
        let next_id = ctx.next_id;
        let states: HashMap<_, _> = ctx.resources.iter().map(|(h, r)| (*h, r.state)).collect();
        assert_eq!(
            ctx.submit_command_list(queues[1], lists[1], &memory),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        assert_eq!(ctx.next_id, next_id);
        assert!(!ctx.queue_values.contains_key(&queues[1]));
        assert_eq!(ctx.queued.len(), 1);
        assert_eq!(
            ctx.queued_copy_allocation_bytes(),
            MAX_CONTEXT_RESOURCE_BYTES
        );
        ctx.queued.get_mut(&receipt).unwrap().copy_allocation_bytes = charge;
        assert!(matches!(
            ctx.submit_command_list(queues[1], lists[1], &memory),
            Err(GraphicsError::Memory(_))
        ));
        assert_eq!(ctx.next_id, next_id);
        assert_eq!(ctx.queued_copy_allocation_bytes(), charge);
        assert_eq!(
            ctx.resources
                .iter()
                .map(|(h, r)| (*h, r.state))
                .collect::<HashMap<_, _>>(),
            states
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        ctx.wait_submission(receipt).unwrap();
        ctx.release_submission(receipt).unwrap();
    }

    #[test]
    fn geometry_snapshot_budget_is_cumulative_before_next_host_allocation() {
        let mut ctx = GraphicsContext::new();
        ctx.set_metal_geometry_graph_backend(GraphicsTextureCapabilities::default(), |_| {
            Err(GraphicsError::InvalidDescriptor)
        });
        let device = ctx.create_device().unwrap();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 48,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let positions = ctx
            .create_committed_resource(
                device,
                desc.clone(),
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                None,
            )
            .unwrap();
        let colors = ctx
            .create_committed_resource(
                device,
                desc,
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                None,
            )
            .unwrap();
        let mut q = QueuedPreparation::default();
        q.produced.insert(
            positions,
            GraphicsProducedVersion {
                point: GraphicsQueuePoint {
                    queue: device,
                    value: 1,
                },
                output: GraphicsProducedOutput::Copy(0),
            },
        );
        q.layouts
            .insert(positions, GraphicsProducedLayout::Buffer { byte_len: 48 });
        let budget = std::cell::Cell::new(MAX_CONTEXT_RESOURCE_BYTES - 740);
        let memory = GuestMemory::new();
        let states = HashMap::new();
        let first = ctx
            .prepare_geometry(
                &memory,
                &budget,
                true,
                Some(&q),
                &states,
                (Some(positions), 0, 16),
                Some((colors, 0, 16)),
                None,
                0,
                3,
                0,
            )
            .unwrap()
            .unwrap();
        // 96 packed bytes + 128 parameter copies + 4 status + 144 host staging.
        assert_eq!(budget.get(), MAX_CONTEXT_RESOURCE_BYTES - 368);
        assert!(matches!(
            first.colors.unwrap().buffer.source,
            GraphicsCopyInput::Host(_)
        ));
        ctx.prepare_geometry(
            &memory,
            &budget,
            false,
            Some(&q),
            &states,
            (Some(positions), 0, 16),
            Some((colors, 0, 16)),
            None,
            0,
            3,
            0,
        )
        .unwrap();
        assert_eq!(budget.get(), MAX_CONTEXT_RESOURCE_BYTES);
        assert!(
            ctx.prepare_geometry(
                &memory,
                &budget,
                false,
                Some(&q),
                &states,
                (Some(positions), 0, 16),
                Some((colors, 0, 16)),
                None,
                0,
                3,
                0
            )
            .is_err()
        );
        assert_eq!(budget.get(), MAX_CONTEXT_RESOURCE_BYTES);
    }

    #[test]
    fn every_older_backend_registration_revokes_geometry() {
        let mut ctx = GraphicsContext::new();
        macro_rules! check {
            ($call:expr) => {{
                ctx.set_metal_geometry_graph_backend(
                    GraphicsTextureCapabilities::default(),
                    |_| Err(GraphicsError::InvalidDescriptor),
                );
                assert!(ctx.geometry_backend);
                $call;
                assert!(!ctx.geometry_backend);
            }};
        }
        check!(ctx.set_metal_queued_backend(|_| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_async_backend(|_| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_multislot_async_backend(|_| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_view_async_backend(|_| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_view_queued_backend(|_| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_backend(|_, _, _| Err(GraphicsError::InvalidDescriptor)));
        check!(ctx.set_metal_pass_backend(|_, _, _| Err(GraphicsError::InvalidDescriptor)));
        check!(
            ctx.set_metal_pass_with_depth_backend(|_, _, _, _| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(ctx.set_metal_targets_backend(|_, _, _, _| Err(GraphicsError::InvalidDescriptor)));
        check!(
            ctx.set_metal_stateful_backend(|_, _, _, _, _| Err(GraphicsError::InvalidDescriptor))
        );
        check!(
            ctx.set_metal_attachment_backend(|_, _, _, _, _| Err(GraphicsError::InvalidDescriptor))
        );
        check!(
            ctx.set_metal_multislot_attachment_backend(|_, _, _, _, _| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(ctx.set_metal_view_attachment_backend(|_, _, _, _, _| Err(
            GraphicsError::InvalidDescriptor
        )));
        check!(ctx.set_metal_uniform_attachment_backend(
            GraphicsTextureCapabilities::default(),
            |_, _, _, _, _| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(ctx.set_metal_typed_attachment_backend(
            GraphicsTextureCapabilities::default(),
            |_, _, _, _, _| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(
            ctx.set_metal_uniform_async_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(
            ctx.set_metal_uniform_queued_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(
            ctx.set_metal_typed_async_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(
            ctx.set_metal_typed_queued_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(ctx.set_metal_produced_color_queued_backend(
            GraphicsTextureCapabilities::default(),
            |_| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(ctx.set_metal_produced_attachment_queued_backend(
            GraphicsTextureCapabilities::default(),
            |_| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(
            ctx.set_metal_copy_queued_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(ctx.set_metal_buffer_region_queued_backend(
            GraphicsTextureCapabilities::default(),
            |_| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(
            ctx.set_metal_image_copy_queued_backend(GraphicsTextureCapabilities::default(), |_| {
                Err(GraphicsError::InvalidDescriptor)
            })
        );
        check!(ctx.set_metal_unaligned_buffer_region_queued_backend(
            GraphicsTextureCapabilities::default(),
            |_| Err(GraphicsError::InvalidDescriptor)
        ));
        check!(
            ctx.set_metal_graph_queued_backend(GraphicsTextureCapabilities::default(), |_| Err(
                GraphicsError::InvalidDescriptor
            ))
        );
        check!(ctx.set_require_metal_without_backend());
    }

    #[test]
    fn mip_chains_use_checked_packed_storage_and_existing_limits() {
        let mut context = GraphicsContext::new();
        let device = context.create_device().unwrap();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: 3,
            height: 5,
            depth_or_array_size: 1,
            mip_levels: 3,
            format_rgba8: true,
        };
        let resource = context
            .create_committed_resource(device, desc.clone(), ResourceStates::COPY_DEST, None)
            .unwrap();
        assert_eq!(context.resources[&resource].data, vec![0; 72]); // 3x5, 1x2, 1x1
        assert_eq!(
            context.create_committed_resource(
                device,
                ResourceDesc {
                    mip_levels: 4,
                    ..desc.clone()
                },
                ResourceStates::COPY_DEST,
                None
            ),
            Err(GraphicsError::InvalidDescriptor)
        );
        assert_eq!(
            context.create_committed_resource(
                device,
                ResourceDesc {
                    width: 4096,
                    height: 4096,
                    mip_levels: 2,
                    ..desc
                },
                ResourceStates::COPY_DEST,
                None
            ),
            Err(GraphicsError::InvalidDescriptor)
        ); // base alone is 64MiB
    }

    #[test]
    fn retained_loader_depth_stages_snapshot_and_barriers_with_colors() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let depth = ctx.create_depth_resource(device, (4, 3)).unwrap();
        let prior: GraphicsDepthSnapshot = Arc::new(17u32);
        ctx.depth_resources.get_mut(&depth).unwrap().snapshot = Some(prior.clone());
        let mode = Arc::new(AtomicUsize::new(0));
        let callback_mode = mode.clone();
        ctx.set_metal_stateful_backend(move |draws, extent, clears, _, binding| {
            assert!(draws.is_empty());
            assert_eq!(extent, (4, 3));
            let binding = binding.unwrap();
            assert_eq!(
                *binding.previous.unwrap().downcast_ref::<u32>().unwrap(),
                17
            );
            let mut colors = vec![clears[0].repeat(12)];
            let mode = callback_mode.load(Ordering::SeqCst);
            if mode == 0 {
                colors[0].pop();
            }
            Ok(GraphicsPassOutput {
                color_layouts: None,
                colors,
                depth: if mode == 1 {
                    None
                } else {
                    Some(Arc::new(23u32))
                },
            })
        });
        ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Load)
            .unwrap();
        ctx.cmd_resource_barrier(
            list,
            depth,
            ResourceStates::DEPTH_WRITE,
            ResourceStates::DEPTH_READ,
        )
        .unwrap();
        ctx.close_command_list(list).unwrap();
        for failure in 0..2 {
            mode.store(failure, Ordering::SeqCst);
            assert!(matches!(
                ctx.execute_command_list(queue, list, &GuestMemory::new()),
                Err(GraphicsError::BackendError(_))
            ));
            assert!(Arc::ptr_eq(
                ctx.depth_resources[&depth].snapshot.as_ref().unwrap(),
                &prior
            ));
            assert_eq!(
                ctx.depth_resources[&depth].state,
                ResourceStates::DEPTH_WRITE
            );
            assert!(ctx.queues[&queue].submitted_lists.is_empty());
            assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 0);
        }
        mode.store(2, Ordering::SeqCst);
        ctx.execute_command_list(queue, list, &GuestMemory::new())
            .unwrap();
        assert_eq!(
            *ctx.depth_resources[&depth]
                .snapshot
                .as_ref()
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap(),
            23
        );
        assert_eq!(
            ctx.depth_resources[&depth].state,
            ResourceStates::DEPTH_READ
        );
    }

    #[test]
    fn retained_loader_depth_binding_lifetime_and_validation() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        for extent in [(0, 3), (4, 0), (4097, 1), (usize::MAX, 2)] {
            assert_eq!(
                ctx.create_depth_resource(device, extent),
                Err(GraphicsError::InvalidDescriptor)
            );
        }
        let depth = ctx.create_depth_resource(device, (4, 3)).unwrap();
        ctx.set_metal_stateful_backend(|_, _, _, _, _| panic!("invalid depth reached backend"));
        ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Load)
            .unwrap();
        ctx.close_command_list(list).unwrap();
        assert_eq!(
            ctx.execute_command_list(queue, list, &GuestMemory::new()),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        ctx.reset_command_list(list, allocator).unwrap();
        for value in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert_eq!(
                ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Clear(value)),
                Err(GraphicsError::InvalidDescriptor)
            );
        }
        ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Clear(1.0))
            .unwrap();
        ctx.cmd_set_depth_target(list, None, GraphicsDepthLoad::Clear(1.0))
            .unwrap();
        for phase in 0..2 {
            if phase == 1 {
                ctx.close_command_list(list).unwrap();
            }
            assert_eq!(
                ctx.destroy_resource(depth),
                Err(GraphicsError::ResourceLifetimeViolation)
            );
        }
        ctx.reset_command_list(list, allocator).unwrap();
        ctx.destroy_resource(depth).unwrap();
        assert_eq!(
            ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Clear(1.0)),
            Err(GraphicsError::InvalidResourceHandle)
        );
    }

    #[test]
    fn retained_loader_depth_read_state_rejects_writes_and_clear_conflicts() {
        for mode in 0..4 {
            let mut ctx = GraphicsContext::new();
            let device = ctx.create_device().unwrap();
            let queue = ctx
                .create_command_queue(device, CommandListType::Direct)
                .unwrap();
            let allocator = ctx
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap();
            let list = ctx
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap();
            let depth = ctx.create_depth_resource(device, (4, 3)).unwrap();
            ctx.depth_resources.get_mut(&depth).unwrap().snapshot = Some(Arc::new(1u32));
            let buffer = ctx
                .create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Buffer,
                        width: 48,
                        height: 1,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: false,
                    },
                    ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                    None,
                )
                .unwrap();
            ctx.set_metal_stateful_backend(|_, _, _, _, _| panic!("invalid depth reached backend"));
            if mode == 2 {
                ctx.cmd_set_depth_clear(list, 1.0).unwrap();
            }
            ctx.cmd_set_depth_target(list, Some(depth), GraphicsDepthLoad::Load)
                .unwrap();
            if mode == 3 {
                ctx.cmd_set_depth_clear(list, 1.0).unwrap();
            }
            ctx.cmd_resource_barrier(
                list,
                depth,
                ResourceStates::DEPTH_WRITE,
                ResourceStates::DEPTH_READ,
            )
            .unwrap();
            ctx.cmd_set_draw_state(
                list,
                GraphicsDrawState {
                    depth_compare: Some(GraphicsDepthCompare::Less),
                    depth_write: true,
                    ..Default::default()
                },
            )
            .unwrap();
            ctx.cmd_set_vertex_buffer(list, buffer, 0, 16).unwrap();
            if mode == 1 {
                ctx.cmd_set_index_buffer(list, buffer, 0, 42).unwrap();
                ctx.cmd_draw_indexed_instanced(list, 3, 1, 0, 0, 0).unwrap();
            } else {
                ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
            }
            ctx.close_command_list(list).unwrap();
            let expected = if mode < 2 {
                GraphicsError::ResourceLifetimeViolation
            } else {
                GraphicsError::InvalidDescriptor
            };
            assert_eq!(
                ctx.execute_command_list(queue, list, &GuestMemory::new()),
                Err(expected)
            );
            assert_eq!(
                ctx.depth_resources[&depth].state,
                ResourceStates::DEPTH_WRITE
            );
        }
    }

    #[test]
    fn strict_target_state_survives_legacy_rebinding() {
        for indexed in [false, true] {
            let mut ctx = GraphicsContext::new();
            let device = ctx.create_device().unwrap();
            let queue = ctx
                .create_command_queue(device, CommandListType::Direct)
                .unwrap();
            let allocator = ctx
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap();
            let list = ctx
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap();
            let target = ctx
                .create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Texture2D,
                        width: 4,
                        height: 3,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: true,
                    },
                    ResourceStates::RENDER_TARGET,
                    None,
                )
                .unwrap();
            let buffer = ctx
                .create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Buffer,
                        width: 48,
                        height: 1,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: false,
                    },
                    ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                    None,
                )
                .unwrap();
            ctx.resources.get_mut(&target).unwrap().data.fill(73);
            ctx.set_metal_targets_backend(|_, _, _, _| panic!("invalid pass reached renderer"));
            ctx.cmd_set_color_targets(list, &[target], &[[0; 4]])
                .unwrap();
            ctx.cmd_set_vertex_buffer(list, buffer, 0, 16).unwrap();
            ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
            ctx.cmd_resource_barrier(
                list,
                target,
                ResourceStates::RENDER_TARGET,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
            ctx.cmd_set_render_targets(list, Some(target), [0; 4])
                .unwrap();
            if indexed {
                ctx.cmd_set_index_buffer(list, buffer, 0, 42).unwrap();
                ctx.cmd_draw_indexed_instanced(list, 3, 1, 0, 0, 0).unwrap();
            } else {
                ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
            }
            ctx.close_command_list(list).unwrap();
            assert_eq!(
                ctx.execute_command_list(queue, list, &GuestMemory::new()),
                Err(GraphicsError::ResourceLifetimeViolation)
            );
            assert_eq!(ctx.resources[&target].data, vec![73; 48]);
            assert_eq!(ctx.resources[&target].state, ResourceStates::RENDER_TARGET);
            assert!(ctx.queues[&queue].submitted_lists.is_empty());
            assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 0);
        }
    }

    #[test]
    fn multiple_target_bindings_alone_retain_superseded_resources() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let targets: Vec<_> = (0..8)
            .map(|_| {
                ctx.create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Texture2D,
                        width: 4,
                        height: 3,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: true,
                    },
                    ResourceStates::RENDER_TARGET,
                    None,
                )
                .unwrap()
            })
            .collect();
        ctx.cmd_set_color_targets(list, &targets, &[[0; 4]; 8])
            .unwrap();
        // Superseded bindings still belong to the replayable command list.
        ctx.cmd_set_render_targets(list, None, [1; 4]).unwrap();
        for phase in 0..3 {
            if phase == 1 {
                ctx.close_command_list(list).unwrap();
            }
            if phase == 2 {
                ctx.execute_command_list(queue, list, &GuestMemory::new())
                    .unwrap();
            }
            for target in &targets {
                assert_eq!(
                    ctx.destroy_resource(*target),
                    Err(GraphicsError::ResourceLifetimeViolation)
                );
            }
        }
        ctx.reset_command_list(list, allocator).unwrap();
        for target in targets {
            ctx.destroy_resource(target).unwrap();
        }
    }

    #[test]
    fn multiple_targets_commit_atomically_and_retain_references() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: 4,
            height: 3,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: true,
        };
        let targets: Vec<_> = (0..8)
            .map(|_| {
                ctx.create_committed_resource(device, desc.clone(), ResourceStates::COMMON, None)
                    .unwrap()
            })
            .collect();
        let vertex = ctx
            .create_committed_resource(
                device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: 48,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                None,
            )
            .unwrap();
        let mode = Arc::new(AtomicUsize::new(0));
        let callback_mode = mode.clone();
        ctx.set_metal_targets_backend(move |draws, extent, clears, depth| {
            assert_eq!(draws.len(), 1);
            assert_eq!(extent, (4, 3));
            assert_eq!(depth, 1.0);
            assert_eq!(clears, &[[9, 8, 7, 6]; 8]);
            let mut outputs: Vec<_> = (0..8).map(|slot| [slot, 13, 17, 255].repeat(12)).collect();
            match callback_mode.load(Ordering::SeqCst) {
                0 => {
                    outputs[7].pop();
                }
                1 => {
                    outputs.pop();
                }
                2 => {
                    outputs.push(vec![0; 48]);
                }
                _ => {}
            }
            Ok(outputs)
        });
        for target in &targets {
            ctx.resources.get_mut(target).unwrap().data.fill(42);
            ctx.cmd_resource_barrier(
                list,
                *target,
                ResourceStates::COMMON,
                ResourceStates::RENDER_TARGET,
            )
            .unwrap();
        }
        ctx.cmd_set_color_targets(list, &targets, &[[9, 8, 7, 6]; 8])
            .unwrap();
        ctx.cmd_set_vertex_buffer(list, vertex, 0, 16).unwrap();
        ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
        for target in &targets {
            ctx.cmd_resource_barrier(
                list,
                *target,
                ResourceStates::RENDER_TARGET,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
        }
        ctx.close_command_list(list).unwrap();
        for failure in 0..3 {
            mode.store(failure, Ordering::SeqCst);
            assert!(matches!(
                ctx.execute_command_list(queue, list, &GuestMemory::new()),
                Err(GraphicsError::BackendError(_))
            ));
            for target in &targets {
                assert_eq!(ctx.resources[target].data, vec![42; 48]);
                assert_eq!(ctx.resources[target].state, ResourceStates::COMMON);
                assert_eq!(
                    ctx.destroy_resource(*target),
                    Err(GraphicsError::ResourceLifetimeViolation)
                );
            }
            assert!(ctx.queues[&queue].submitted_lists.is_empty());
            assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 0);
        }
        mode.store(3, Ordering::SeqCst);
        let out = ctx
            .execute_command_list(queue, list, &GuestMemory::new())
            .unwrap();
        assert_eq!(out.color_outputs.len(), 8);
        for (slot, target) in targets.iter().enumerate() {
            let expected = [slot as u8, 13, 17, 255].repeat(12);
            assert_eq!(ctx.resources[target].data, expected);
            assert_eq!(out.color_outputs[slot], expected);
            assert_eq!(ctx.resources[target].state, ResourceStates::COPY_SOURCE);
        }
        assert_eq!(out.pixels, [0, 13, 17, 255].repeat(12));
        assert_eq!(ctx.last_presentation.lock().unwrap().pixels, out.pixels);
        assert_eq!(ctx.queues[&queue].submitted_lists, vec![list]);
        ctx.reset_command_list(list, allocator).unwrap();
        for target in targets {
            ctx.destroy_resource(target).unwrap();
        }
    }

    #[test]
    fn multiple_targets_validate_binding_and_clear_every_slot() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Texture2D,
            width: 4,
            height: 3,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: true,
        };
        let a = ctx
            .create_committed_resource(device, desc.clone(), ResourceStates::RENDER_TARGET, None)
            .unwrap();
        let b = ctx
            .create_committed_resource(device, desc.clone(), ResourceStates::RENDER_TARGET, None)
            .unwrap();
        let guest = ctx
            .create_committed_resource(
                device,
                desc.clone(),
                ResourceStates::RENDER_TARGET,
                Some(0x1000),
            )
            .unwrap();
        let small = ctx
            .create_committed_resource(
                device,
                ResourceDesc { width: 2, ..desc },
                ResourceStates::RENDER_TARGET,
                None,
            )
            .unwrap();
        for (targets, clears) in [
            (vec![], vec![]),
            (vec![a; 9], vec![[0; 4]; 9]),
            (vec![a, a], vec![[0; 4]; 2]),
            (vec![a, small], vec![[0; 4]; 2]),
            (vec![a, guest], vec![[0; 4]; 2]),
            (vec![a, b], vec![[0; 4]]),
        ] {
            assert_eq!(
                ctx.cmd_set_color_targets(list, &targets, &clears),
                Err(GraphicsError::InvalidDescriptor)
            );
            assert!(ctx.command_lists[&list].commands.is_empty());
        }
        ctx.cmd_set_color_targets(list, &[a, b], &[[1, 2, 3, 4], [5, 6, 7, 8]])
            .unwrap();
        ctx.close_command_list(list).unwrap();
        let out = ctx
            .execute_command_list(queue, list, &GuestMemory::new())
            .unwrap();
        assert_eq!(
            out.color_outputs,
            vec![[1, 2, 3, 4].repeat(12), [5, 6, 7, 8].repeat(12)]
        );
        assert_eq!(ctx.resources[&a].data, out.color_outputs[0]);
        assert_eq!(ctx.resources[&b].data, out.color_outputs[1]);
    }

    #[test]
    fn completed_submission_history_is_bounded_and_excludes_failures() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        ctx.cmd_set_viewport(list, 1, 1).unwrap();
        ctx.close_command_list(list).unwrap();
        for _ in 0..1024 {
            ctx.execute_command_list(queue, list, &GuestMemory::new())
                .unwrap();
        }
        assert_eq!(ctx.queues[&queue].submitted_lists.len(), 256);
        let failed = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        ctx.cmd_draw_instanced(failed, 3, 1, 0, 0).unwrap();
        ctx.close_command_list(failed).unwrap();
        assert!(
            ctx.execute_command_list(queue, failed, &GuestMemory::new())
                .is_err()
        );
        assert!(
            ctx.queues[&queue]
                .submitted_lists
                .iter()
                .all(|&handle| handle == list)
        );
        assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 1024);
    }
    #[test]
    fn failed_submission_does_not_commit_barrier_states() {
        for invalid_barrier in [false, true] {
            let mut ctx = GraphicsContext::new();
            let device = ctx.create_device().unwrap();
            let queue = ctx
                .create_command_queue(device, CommandListType::Direct)
                .unwrap();
            let allocator = ctx
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap();
            let list = ctx
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap();
            let initial = ResourceStates::VERTEX_AND_CONSTANT_BUFFER;
            let resource = ctx
                .create_committed_resource(
                    device,
                    ResourceDesc {
                        dimension: ResourceDimension::Buffer,
                        width: 48,
                        height: 1,
                        depth_or_array_size: 1,
                        mip_levels: 1,
                        format_rgba8: false,
                    },
                    initial,
                    None,
                )
                .unwrap();
            ctx.cmd_resource_barrier(list, resource, initial, ResourceStates::COPY_DEST)
                .unwrap();
            if invalid_barrier {
                ctx.cmd_resource_barrier(list, resource, initial, ResourceStates::COPY_SOURCE)
                    .unwrap();
            }
            ctx.cmd_set_vertex_buffer(list, resource, 0, 16).unwrap();
            ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
            ctx.close_command_list(list).unwrap();
            ctx.set_metal_backend(|_, _, _| Err(GraphicsError::BackendError("injected".into())));
            let error = ctx
                .execute_command_list(queue, list, &GuestMemory::new())
                .unwrap_err();
            if invalid_barrier {
                assert_eq!(error, GraphicsError::ResourceLifetimeViolation);
            } else {
                assert!(matches!(error, GraphicsError::BackendError(_)));
            }
            assert_eq!(ctx.resources[&resource].state, initial);
            assert!(ctx.queues[&queue].submitted_lists.is_empty());
            assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 0);
            if !invalid_barrier {
                ctx.set_metal_backend(GraphicsContext::execute_software_reference_triangles);
                ctx.execute_command_list(queue, list, &GuestMemory::new())
                    .unwrap();
                assert_eq!(ctx.resources[&resource].state, ResourceStates::COPY_DEST);
            }
        }
    }
    #[test]
    fn resource_transfers_are_checked_and_atomic() {
        use crate::memory::MemoryPermissions;
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let resource = ctx
            .create_committed_resource(
                device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: 8,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::COPY_DEST,
                None,
            )
            .unwrap();
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 4, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
            .map_region(0x1004, 4, MemoryPermissions::READ_ONLY)
            .unwrap();
        memory
            .map_region(0x2000, 8, MemoryPermissions::NO_ACCESS)
            .unwrap();
        memory.write_bytes(0x1000, &[1, 2, 3, 4]).unwrap();
        ctx.copy_guest_to_resource(resource, 2, 0x1000, 4, &memory)
            .unwrap();
        let expected = vec![0, 0, 1, 2, 3, 4, 0, 0];
        assert_eq!(ctx.resources[&resource].data, expected);
        for (offset, address, size) in [
            (0, 0x1006, 4),
            (0, 0x2000, 4),
            (0, u64::MAX, 4),
            (usize::MAX, 0x1000, 4),
            (7, 0x1000, 2),
        ] {
            assert!(
                ctx.copy_guest_to_resource(resource, offset, address, size, &memory)
                    .is_err()
            );
            assert_eq!(ctx.resources[&resource].data, expected);
        }
        assert_eq!(
            ctx.copy_resource_to_guest(resource, 0, 0x1000, 4, &mut memory),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        ctx.resources.get_mut(&resource).unwrap().state = ResourceStates::COPY_SOURCE;
        assert_eq!(
            ctx.copy_guest_to_resource(resource, 0, 0x1000, 4, &memory),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        // A writable prefix followed by a read-only region must remain untouched.
        assert!(
            ctx.copy_resource_to_guest(resource, 0, 0x1000, 8, &mut memory)
                .is_err()
        );
        let mut actual = [0; 4];
        memory.read_bytes(0x1000, &mut actual).unwrap();
        assert_eq!(actual, [1, 2, 3, 4]);
        ctx.copy_resource_to_guest(resource, 1, 0x1000, 4, &mut memory)
            .unwrap();
        memory.read_bytes(0x1000, &mut actual).unwrap();
        assert_eq!(actual, [0, 1, 2, 3]);
        assert_eq!(
            ctx.copy_resource_to_guest(GraphicsHandle::NULL, 0, 0x1000, 1, &mut memory),
            Err(GraphicsError::InvalidResourceHandle)
        );
        ctx.resources.get_mut(&resource).unwrap().guest_backing_addr = Some(0x1000);
        assert_eq!(
            ctx.copy_resource_to_guest(resource, 0, 0x1000, 1, &mut memory),
            Err(GraphicsError::InvalidDescriptor)
        );
    }
    #[test]
    fn resource_destruction_requires_all_command_lists_to_release_references() {
        for kind in 0..4 {
            let mut ctx = GraphicsContext::new();
            let device = ctx.create_device().unwrap();
            let allocator = ctx
                .create_command_allocator(device, CommandListType::Direct)
                .unwrap();
            let first = ctx
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap();
            let second = ctx
                .create_command_list(device, allocator, CommandListType::Direct)
                .unwrap();
            let desc = ResourceDesc {
                dimension: ResourceDimension::Buffer,
                width: 48,
                height: 1,
                depth_or_array_size: 1,
                mip_levels: 1,
                format_rgba8: false,
            };
            let resource = ctx
                .create_committed_resource(device, desc.clone(), ResourceStates::COMMON, None)
                .unwrap();
            for list in [first, second] {
                match kind {
                    0 => ctx.cmd_set_vertex_buffer(list, resource, 0, 16).unwrap(),
                    1 => ctx.cmd_set_index_buffer(list, resource, 0, 57).unwrap(),
                    2 => ctx
                        .cmd_set_render_targets(list, Some(resource), [0; 4])
                        .unwrap(),
                    _ => ctx
                        .cmd_resource_barrier(
                            list,
                            resource,
                            ResourceStates::COMMON,
                            ResourceStates::COPY_SOURCE,
                        )
                        .unwrap(),
                }
            }
            ctx.close_command_list(first).unwrap();
            assert_eq!(
                ctx.destroy_resource(resource),
                Err(GraphicsError::ResourceLifetimeViolation)
            );
            assert!(ctx.resources.contains_key(&resource));
            ctx.reset_command_list(first, allocator).unwrap();
            assert_eq!(
                ctx.destroy_resource(resource),
                Err(GraphicsError::ResourceLifetimeViolation)
            );
            ctx.reset_command_list(second, allocator).unwrap();
            ctx.destroy_resource(resource).unwrap();
            assert!(!ctx.resources.contains_key(&resource));
            assert_eq!(
                ctx.destroy_resource(resource),
                Err(GraphicsError::InvalidResourceHandle)
            );
            let replacement = ctx
                .create_committed_resource(device, desc, ResourceStates::COPY_DEST, None)
                .unwrap();
            assert_ne!(resource, replacement);
            assert_eq!(
                ctx.cmd_set_vertex_buffer(first, resource, 0, 16),
                Err(GraphicsError::InvalidResourceHandle)
            );
            assert_eq!(
                ctx.copy_guest_to_resource(resource, 0, 0, 0, &GuestMemory::new()),
                Err(GraphicsError::InvalidResourceHandle)
            );
            ctx.destroy_resource(replacement).unwrap();
        }
    }
    #[test]
    fn resource_allocation_rejects_invalid_layouts_without_consuming_handles() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let valid = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 16,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let mut invalid = Vec::new();
        for dimension in [
            ResourceDimension::Unknown,
            ResourceDimension::Texture1D,
            ResourceDimension::Texture3D,
        ] {
            invalid.push(ResourceDesc {
                dimension,
                ..valid.clone()
            });
        }
        invalid.extend([
            ResourceDesc {
                width: 0,
                ..valid.clone()
            },
            ResourceDesc {
                height: 0,
                ..valid.clone()
            },
            ResourceDesc {
                height: 2,
                ..valid.clone()
            },
            ResourceDesc {
                depth_or_array_size: 0,
                ..valid.clone()
            },
            ResourceDesc {
                depth_or_array_size: 2,
                ..valid.clone()
            },
            ResourceDesc {
                mip_levels: 0,
                ..valid.clone()
            },
            ResourceDesc {
                mip_levels: 2,
                ..valid.clone()
            },
            ResourceDesc {
                format_rgba8: true,
                ..valid.clone()
            },
            ResourceDesc {
                dimension: ResourceDimension::Texture2D,
                ..valid.clone()
            },
            ResourceDesc {
                dimension: ResourceDimension::Texture2D,
                format_rgba8: true,
                width: u64::MAX,
                height: u32::MAX,
                ..valid.clone()
            },
            ResourceDesc {
                width: 64 * 1024 * 1024 + 1,
                ..valid.clone()
            },
        ]);
        let next = ctx.next_id;
        for desc in invalid {
            assert_eq!(
                ctx.create_committed_resource(device, desc, ResourceStates::COMMON, None),
                Err(GraphicsError::InvalidDescriptor)
            );
            assert!(ctx.resources.is_empty());
            assert_eq!(ctx.next_id, next);
        }
        assert_eq!(
            ctx.create_committed_resource(
                device,
                valid.clone(),
                ResourceStates::COMMON,
                Some(u64::MAX - 7)
            ),
            Err(GraphicsError::InvalidDescriptor)
        );
        let resource = ctx
            .create_committed_resource(device, valid, ResourceStates::COMMON, None)
            .unwrap();
        assert_eq!(resource.id, next);
        assert_eq!(ctx.resources[&resource].data, [0; 16]);
        let texture = ctx
            .create_committed_resource(
                device,
                ResourceDesc {
                    dimension: ResourceDimension::Texture2D,
                    width: 3,
                    height: 2,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: true,
                },
                ResourceStates::RENDER_TARGET,
                None,
            )
            .unwrap();
        assert_eq!(ctx.resources[&texture].data, [0; 24]);
    }

    #[test]
    fn resource_allocation_budget_is_reclaimed_on_destruction() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 64 * 1024 * 1024,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let mut handles = Vec::new();
        for _ in 0..4 {
            handles.push(
                ctx.create_committed_resource(device, desc.clone(), ResourceStates::COMMON, None)
                    .unwrap(),
            );
        }
        let next = ctx.next_id;
        assert_eq!(
            ctx.create_committed_resource(device, desc.clone(), ResourceStates::COMMON, None),
            Err(GraphicsError::Memory(MemoryError::AllocationFailed))
        );
        assert_eq!(ctx.next_id, next);
        assert_eq!(ctx.resources.len(), 4);
        ctx.destroy_resource(handles[0]).unwrap();
        let replacement = ctx
            .create_committed_resource(device, desc, ResourceStates::COMMON, None)
            .unwrap();
        assert_ne!(replacement, handles[0]);
        assert_eq!(ctx.resources.len(), 4);
        assert!(
            ctx.resources[&replacement]
                .data
                .iter()
                .all(|&byte| byte == 0)
        );
    }
    #[test]
    fn common_barriers_validate_staged_state_and_replay() {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let resource = ctx
            .create_committed_resource(
                device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: 4,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::COMMON,
                None,
            )
            .unwrap();
        ctx.cmd_resource_barrier(
            list,
            resource,
            ResourceStates::COMMON,
            ResourceStates::COPY_DEST,
        )
        .unwrap();
        ctx.cmd_resource_barrier(
            list,
            resource,
            ResourceStates::COMMON,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
        ctx.close_command_list(list).unwrap();
        assert_eq!(
            ctx.execute_command_list(queue, list, &GuestMemory::new()),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        assert_eq!(ctx.resources[&resource].state, ResourceStates::COMMON);
        assert!(ctx.queues[&queue].submitted_lists.is_empty());
        assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 0);
        ctx.reset_command_list(list, allocator).unwrap();
        ctx.cmd_resource_barrier(
            list,
            resource,
            ResourceStates::COMMON,
            ResourceStates::COPY_DEST,
        )
        .unwrap();
        ctx.close_command_list(list).unwrap();
        ctx.execute_command_list(queue, list, &GuestMemory::new())
            .unwrap();
        assert_eq!(
            ctx.execute_command_list(queue, list, &GuestMemory::new()),
            Err(GraphicsError::ResourceLifetimeViolation)
        );
        assert_eq!(ctx.resources[&resource].state, ResourceStates::COPY_DEST);
        assert_eq!(ctx.queues[&queue].submitted_lists.len(), 1);
        assert_eq!(ctx.last_presentation.lock().unwrap().frame_index, 1);
    }
}
