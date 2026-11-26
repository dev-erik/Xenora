// SPDX-License-Identifier: GPL-3.0-or-later

//! Native Metal texture storage, uploads and read-only sampled views.
//!
//! Legacy RGBA8 owners retain their contract; immutable typed owners preserve
//! format, mip and array identity across uploads and selected views.

use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSRange;
use objc2_metal::{
    MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion, MTLResource, MTLResourceOptions, MTLSize,
    MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureType, MTLTextureUsage,
};
use xenora_texture::{
    PackedTextureArrayLayout, PackedTextureLayout, SamplerError, TextureFormat, TextureMetadata,
};

use crate::MetalError;
use crate::sampler::MetalSampler;

/// Bytes per RGBA8 texel.
pub const RGBA8_TEXEL_BYTES: usize = 4;

/// Upper limit on total pixels per texture to prevent bounded resource overflow.
pub const MAX_TEXTURE_PIXELS: usize = 16384 * 16384;
/// Per-axis bound for this native texture interface.
pub const MAX_TEXTURE_DIMENSION: usize = 16384;

/// Subresource mip level data descriptor for RGBA8 upload.
#[derive(Debug, Clone, Copy)]
pub struct Rgba8MipLevel<'a> {
    pub level: u32,
    pub width: usize,
    pub height: usize,
    pub row_pitch: usize,
    pub bytes: &'a [u8],
}

impl<'a> Rgba8MipLevel<'a> {
    /// Create a tightly packed mip level descriptor.
    pub fn new_packed(level: u32, width: usize, height: usize, bytes: &'a [u8]) -> Self {
        // Keep this descriptor builder infallible without wrapping malformed input.
        // Upload validates dimensions and pitch before touching native storage.
        let row_pitch = width.saturating_mul(RGBA8_TEXEL_BYTES);
        Self {
            level,
            width,
            height,
            row_pitch,
            bytes,
        }
    }
}

/// Description of an RGBA8 2D texture to allocate on Metal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8TextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
}

impl Rgba8TextureDesc {
    pub fn new(width: usize, height: usize, mips: u32) -> Result<Self, MetalError> {
        if width > MAX_TEXTURE_DIMENSION || height > MAX_TEXTURE_DIMENSION {
            return Err(MetalError::new(
                "texture bounds",
                "texture dimension exceeds interface limit",
            ));
        }
        let meta = TextureMetadata::new(width, height, mips).map_err(|err: SamplerError| {
            MetalError::new("texture metadata validation", err.to_string())
        })?;

        let total_pixels = width
            .checked_mul(height)
            .ok_or_else(|| MetalError::new("texture dimensions", "pixel count overflow"))?;

        if total_pixels > MAX_TEXTURE_PIXELS {
            return Err(MetalError::new(
                "texture bounds",
                "texture exceeds maximum allowable pixels",
            ));
        }

        Ok(Self {
            width: meta.width,
            height: meta.height,
            mips: meta.mips,
        })
    }

    /// Compute expected dimensions for a given mip level.
    pub fn mip_extent(&self, level: u32) -> Result<(usize, usize), MetalError> {
        Self::new(self.width, self.height, self.mips)?;
        if level >= self.mips {
            return Err(MetalError::new(
                "mip level bounds",
                "mip level index is out of bounds",
            ));
        }
        let mip_w = (self.width >> level).max(1);
        let mip_h = (self.height >> level).max(1);
        Ok((mip_w, mip_h))
    }
}

/// Read-only views preserve their storage family. Optional sRGB interpretation
/// is available only for RGBA8, BGRA8, BC1 and BC3; cross-family casts reject.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureViewFormat {
    R8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bc1RgbaUnorm,
    Bc1RgbaUnormSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaUnormSrgb,
    Rg8Unorm,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    Rgba16Float,
    R32Float,
    R32Uint,
    Depth32Float,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureViewDesc {
    pub base_mip: u32,
    pub mip_count: u32,
    pub format: TextureViewFormat,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ArraySliceViewDesc {
    pub slice: u32,
    pub base_mip: u32,
    pub mip_count: u32,
    pub format: TextureViewFormat,
}

/// Immutable native shared-storage view. No upload, raw texture or public
/// conversion to the upload-capable whole-texture binding is available.
/// The owner of the original uploaded texture must still serialize its uploads.
///
/// ```compile_fail
/// fn cannot_upload(v: xenora_metal::MetalTextureView, mip: &xenora_metal::Rgba8MipLevel<'_>) {
///     v.upload_mip(mip);
/// }
/// ```
/// ```compile_fail
/// fn cannot_extract(v: xenora_metal::MetalTextureView) { v.raw(); }
/// ```
/// ```compile_fail
/// use xenora_metal::{MetalTextureView, MetalSampler, BoundSampledTexture, BindingSlot};
/// fn cannot_bind(v: MetalTextureView, s: MetalSampler) {
///     BoundSampledTexture::bind(v, s, BindingSlot::new(0), BindingSlot::new(0));
/// }
/// ```
#[derive(Clone, Debug)]
pub struct MetalTextureView {
    slice: Option<u32>,
    texture: NativeSampledTexture,
    desc: TextureViewDesc,
}
impl MetalTextureView {
    pub fn slice(&self) -> Option<u32> {
        self.slice
    }
    pub fn desc(&self) -> TextureViewDesc {
        self.desc
    }
    pub fn extent(&self) -> [usize; 2] {
        [self.texture.width, self.texture.height]
    }
    pub(crate) fn retained_texture(&self) -> Retained<ProtocolObject<dyn MTLTexture>> {
        self.texture.texture.clone()
    }
    pub(crate) fn bind(
        &self,
        sampler: MetalSampler,
        texture_slot: BindingSlot,
        sampler_slot: BindingSlot,
    ) -> Result<NativeSampledBinding, MetalError> {
        NativeSampledBinding::bind(self.texture.clone(), sampler, texture_slot, sampler_slot)
    }
}
/// Additive immutable-view binding interface; normalization is crate-private.
#[derive(Clone, Debug)]
pub enum SampledBinding {
    Whole(BoundSampledTexture),
    View {
        view: MetalTextureView,
        sampler: MetalSampler,
        texture_slot: BindingSlot,
        sampler_slot: BindingSlot,
    },
}
impl SampledBinding {
    pub(crate) fn normalize(&self) -> Result<NativeSampledBinding, MetalError> {
        match self {
            Self::Whole(pair) => Ok(NativeSampledBinding::from_whole(pair)),
            Self::View {
                view,
                sampler,
                texture_slot,
                sampler_slot,
            } => view.bind(sampler.clone(), *texture_slot, *sampler_slot),
        }
    }
}

/// Private format-aware sampled storage. Compressed data never enters the RGBA8 owner.
#[derive(Clone, Debug)]
pub(crate) struct NativeSampledTexture {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    _parent: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    width: usize,
    height: usize,
    mips: u32,
    format: TextureFormat,
}
impl NativeSampledTexture {
    pub(crate) fn from_rgba(texture: &MetalRgba8Texture) -> Self {
        Self {
            texture: texture.texture.clone(),
            _parent: texture._parent.clone(),
            width: texture.desc.width,
            height: texture.desc.height,
            mips: texture.desc.mips,
            format: TextureFormat::Rgba8Unorm,
        }
    }
    pub(crate) fn raw(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }
    pub(crate) fn same_object(&self, other: &Self) -> bool {
        std::ptr::eq(self.raw(), other.raw())
    }
    fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, MetalError> {
        if self.texture.textureType() == MTLTextureType::Type2DArray {
            return Err(MetalError::new(
                "texture view",
                "array requires selected slice",
            ));
        }
        self.view_internal(desc, None)
    }
    fn array_slice_view(&self, desc: ArraySliceViewDesc) -> Result<MetalTextureView, MetalError> {
        if self.texture.textureType() != MTLTextureType::Type2DArray
            || desc.slice as usize >= self.texture.arrayLength()
        {
            return Err(MetalError::new(
                "array view",
                "invalid array slice or format",
            ));
        }
        self.view_internal(
            TextureViewDesc {
                base_mip: desc.base_mip,
                mip_count: desc.mip_count,
                format: desc.format,
            },
            Some(desc.slice),
        )
    }
    fn view_internal(
        &self,
        desc: TextureViewDesc,
        slice: Option<u32>,
    ) -> Result<MetalTextureView, MetalError> {
        let end = desc
            .base_mip
            .checked_add(desc.mip_count)
            .ok_or_else(|| MetalError::new("texture view", "mip range overflow"))?;
        if desc.mip_count == 0 || end > self.mips {
            return Err(MetalError::new(
                "texture view",
                "empty or out-of-bounds mip range",
            ));
        }
        let pixel_format = match (self.format, desc.format) {
            (TextureFormat::R8Unorm, TextureViewFormat::R8Unorm) => MTLPixelFormat::R8Unorm,
            (TextureFormat::Rg8Unorm, TextureViewFormat::Rg8Unorm) => MTLPixelFormat::RG8Unorm,
            (TextureFormat::Bgra8Unorm, TextureViewFormat::Bgra8Unorm) => {
                MTLPixelFormat::BGRA8Unorm
            }
            (TextureFormat::Bgra8Unorm, TextureViewFormat::Bgra8UnormSrgb) => {
                MTLPixelFormat::BGRA8Unorm_sRGB
            }
            (TextureFormat::Rgba16Float, TextureViewFormat::Rgba16Float) => {
                MTLPixelFormat::RGBA16Float
            }
            (TextureFormat::R32Float, TextureViewFormat::R32Float) => MTLPixelFormat::R32Float,
            (TextureFormat::R32Uint, TextureViewFormat::R32Uint) => MTLPixelFormat::R32Uint,
            (TextureFormat::Depth32Float, TextureViewFormat::Depth32Float) => {
                MTLPixelFormat::Depth32Float
            }
            (TextureFormat::Rgba8Unorm, TextureViewFormat::Rgba8Unorm) => {
                MTLPixelFormat::RGBA8Unorm
            }
            (TextureFormat::Rgba8Unorm, TextureViewFormat::Rgba8UnormSrgb) => {
                MTLPixelFormat::RGBA8Unorm_sRGB
            }
            (TextureFormat::Bc1RgbaUnorm, TextureViewFormat::Bc1RgbaUnorm) => {
                MTLPixelFormat::BC1_RGBA
            }
            (TextureFormat::Bc1RgbaUnorm, TextureViewFormat::Bc1RgbaUnormSrgb) => {
                MTLPixelFormat::BC1_RGBA_sRGB
            }
            (TextureFormat::Bc3RgbaUnorm, TextureViewFormat::Bc3RgbaUnorm) => {
                MTLPixelFormat::BC3_RGBA
            }
            (TextureFormat::Bc3RgbaUnorm, TextureViewFormat::Bc3RgbaUnormSrgb) => {
                MTLPixelFormat::BC3_RGBA_sRGB
            }
            _ => {
                return Err(MetalError::new(
                    "texture view",
                    "incompatible storage/view format",
                ));
            }
        };
        // SAFETY: Checked nonempty mip range and selected root slice; only same-family
        // linear/sRGB casts are accepted. These do not require PixelFormatView.
        let texture = unsafe {
            self.texture
                .newTextureViewWithPixelFormat_textureType_levels_slices(
                    pixel_format,
                    MTLTextureType::Type2D,
                    NSRange::new(desc.base_mip as usize, desc.mip_count as usize),
                    NSRange::new(slice.unwrap_or(0) as usize, 1),
                )
        }
        .ok_or_else(|| MetalError::new("texture view", "native view creation failed"))?;
        Ok(MetalTextureView {
            slice,
            texture: Self {
                texture,
                _parent: Some(self.texture.clone()),
                width: (self.width >> desc.base_mip).max(1),
                height: (self.height >> desc.base_mip).max(1),
                mips: desc.mip_count,
                format: self.format,
            },
            desc,
        })
    }
}
#[derive(Clone)]
pub(crate) struct NativeSampledBinding {
    texture: NativeSampledTexture,
    sampler: MetalSampler,
    texture_slot: BindingSlot,
    sampler_slot: BindingSlot,
}
impl NativeSampledBinding {
    pub(crate) fn from_whole(pair: &BoundSampledTexture) -> Self {
        Self {
            texture: NativeSampledTexture::from_rgba(&pair.texture),
            sampler: pair.sampler.clone(),
            texture_slot: pair.texture_slot,
            sampler_slot: pair.sampler_slot,
        }
    }
    fn bind(
        texture: NativeSampledTexture,
        sampler: MetalSampler,
        texture_slot: BindingSlot,
        sampler_slot: BindingSlot,
    ) -> Result<Self, MetalError> {
        let pair = Self {
            texture,
            sampler,
            texture_slot,
            sampler_slot,
        };
        pair.validate_for_device(&pair.texture.texture.device())?;
        Ok(pair)
    }
    pub(crate) fn validate_for_device(
        &self,
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Result<(), MetalError> {
        if self.texture.texture.device().registryID() != self.sampler.device_id()
            || self.texture.texture.device().registryID() != device.registryID()
        {
            return Err(MetalError::new(
                "render bindings",
                "texture/sampler/render device mismatch",
            ));
        }
        let metadata =
            TextureMetadata::new(self.texture.width, self.texture.height, self.texture.mips)
                .map_err(|e| MetalError::new("texture metadata", e.to_string()))?;
        self.sampler
            .desc()
            .validate_against_texture(&metadata)
            .map_err(|e| MetalError::new("sampler texture compatibility", e.to_string()))
    }
    pub(crate) fn texture(&self) -> &NativeSampledTexture {
        &self.texture
    }
    pub(crate) fn sampler(&self) -> &MetalSampler {
        &self.sampler
    }
    pub(crate) fn texture_slot(&self) -> BindingSlot {
        self.texture_slot
    }
    pub(crate) fn sampler_slot(&self) -> BindingSlot {
        self.sampler_slot
    }
}

/// Bounded, immutable BC storage; all mip data must be supplied before publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BcTextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
    pub format: TextureFormat,
}
#[derive(Clone, Copy, Debug)]
pub struct BcMipUpload<'a> {
    pub level: u32,
    pub row_pitch: usize,
    pub bytes: &'a [u8],
}
pub const MAX_BC_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
/// Fully initialized persistent BC1/BC3 owner. Changed bytes require a new owner.
/// ```compile_fail
/// fn cannot_extract(t: xenora_metal::MetalBcTexture) { t.raw(); }
/// ```
/// ```compile_fail
/// fn cannot_upload(t: xenora_metal::MetalBcTexture, mip: &xenora_metal::BcMipUpload<'_>) { t.upload_mip(mip); }
/// ```
#[derive(Clone, Debug)]
pub struct MetalBcTexture {
    texture: NativeSampledTexture,
    desc: BcTextureDesc,
}
fn validate_bc_upload(
    desc: BcTextureDesc,
    uploads: &[BcMipUpload<'_>],
    supported: bool,
) -> Result<PackedTextureLayout, MetalError> {
    let block_bytes = match desc.format {
        TextureFormat::Bc1RgbaUnorm => 8,
        TextureFormat::Bc3RgbaUnorm => 16,
        _ => return Err(MetalError::new("BC texture", "expected BC1 or BC3 storage")),
    };
    if desc.width > MAX_TEXTURE_DIMENSION || desc.height > MAX_TEXTURE_DIMENSION {
        return Err(MetalError::new(
            "BC texture",
            "dimension exceeds native policy",
        ));
    }
    let layout = PackedTextureLayout::new(desc.format, desc.width, desc.height, desc.mips)
        .map_err(|e| MetalError::new("BC layout", format!("{e:?}")))?;
    if layout.byte_len() > MAX_BC_TEXTURE_BYTES || uploads.len() != layout.levels().len() {
        return Err(MetalError::new(
            "BC upload",
            "byte budget or mip count mismatch",
        ));
    }
    let mut seen = vec![false; layout.levels().len()];
    let mut total = 0usize;
    for upload in uploads {
        let level = upload.level as usize;
        let footprint = layout
            .levels()
            .get(level)
            .ok_or_else(|| MetalError::new("BC upload", "mip out of range"))?;
        if seen[level]
            || upload.row_pitch < footprint.row_bytes
            || !upload.row_pitch.is_multiple_of(block_bytes)
            || upload.row_pitch.checked_mul(footprint.rows) != Some(upload.bytes.len())
        {
            return Err(MetalError::new(
                "BC upload",
                "duplicate mip or invalid pitch/span",
            ));
        }
        seen[level] = true;
        total = total
            .checked_add(upload.bytes.len())
            .filter(|n| *n <= MAX_BC_TEXTURE_BYTES)
            .ok_or_else(|| MetalError::new("BC upload", "padded upload byte budget exceeded"))?;
    }
    if !supported {
        return Err(MetalError::new(
            MetalError::BC_UNAVAILABLE,
            "BC texture compression unavailable",
        ));
    }
    Ok(layout)
}
impl MetalBcTexture {
    pub fn desc(&self) -> BcTextureDesc {
        self.desc
    }
    pub fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, MetalError> {
        self.texture.view(desc)
    }
    pub fn upload(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: BcTextureDesc,
        uploads: &[BcMipUpload<'_>],
    ) -> Result<Self, MetalError> {
        let layout = validate_bc_upload(desc, uploads, device.supportsBCTextureCompression())?;
        let format = match desc.format {
            TextureFormat::Bc1RgbaUnorm => MTLPixelFormat::BC1_RGBA,
            TextureFormat::Bc3RgbaUnorm => MTLPixelFormat::BC3_RGBA,
            _ => unreachable!("validated BC format"),
        };
        autoreleasepool(|_| {
            // SAFETY: Canonical layout and dimensions were checked before allocation.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    format,
                    desc.width,
                    desc.height,
                    desc.mips > 1,
                )
            };
            // SAFETY: PackedTextureLayout validated a nonempty count within the full mip chain.
            unsafe { descriptor.setMipmapLevelCount(desc.mips as usize) };
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .ok_or_else(|| {
                    MetalError::new("newTextureWithDescriptor", "BC allocation failed")
                })?;
            for upload in uploads {
                let level = &layout.levels()[upload.level as usize];
                let region = MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: level.width,
                        height: level.height,
                        depth: 1,
                    },
                };
                // SAFETY: Entire fresh mip, complete edge blocks and exact checked
                // padded span. Single block rows use Apple's required zero stride.
                unsafe {
                    texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        upload.level as usize,
                        NonNull::from(upload.bytes).cast(),
                        if level.rows == 1 { 0 } else { upload.row_pitch },
                    );
                }
            }
            Ok(Self {
                texture: NativeSampledTexture {
                    texture,
                    _parent: None,
                    width: desc.width,
                    height: desc.height,
                    mips: desc.mips,
                    format: desc.format,
                },
                desc,
            })
        })
    }
}

/// Bounded, immutable R8 storage; all mip data must be supplied before publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct R8TextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct R8MipUpload<'a> {
    pub level: u32,
    pub row_pitch: usize,
    pub bytes: &'a [u8],
}
pub const MAX_R8_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
/// Fully initialized persistent R8 owner. Changed bytes require a new owner.
/// ```compile_fail
/// fn cannot_extract(t: xenora_metal::MetalR8Texture) { t.raw(); }
/// ```
/// ```compile_fail
/// fn cannot_upload(t: xenora_metal::MetalR8Texture, mip: &xenora_metal::R8MipUpload<'_>) { t.upload_mip(mip); }
/// ```
#[derive(Clone, Debug)]
pub struct MetalR8Texture {
    texture: NativeSampledTexture,
    desc: R8TextureDesc,
}
fn validate_r8_upload(
    desc: R8TextureDesc,
    uploads: &[R8MipUpload<'_>],
) -> Result<PackedTextureLayout, MetalError> {
    if desc.width > MAX_TEXTURE_DIMENSION || desc.height > MAX_TEXTURE_DIMENSION {
        return Err(MetalError::new(
            "R8 texture",
            "dimension exceeds native policy",
        ));
    }
    let layout =
        PackedTextureLayout::new(TextureFormat::R8Unorm, desc.width, desc.height, desc.mips)
            .map_err(|e| MetalError::new("R8 layout", format!("{e:?}")))?;
    if layout.byte_len() > MAX_R8_TEXTURE_BYTES || uploads.len() != layout.levels().len() {
        return Err(MetalError::new(
            "R8 upload",
            "byte budget or mip count mismatch",
        ));
    }
    let mut seen = vec![false; layout.levels().len()];
    let mut total = 0usize;
    for upload in uploads {
        let level = upload.level as usize;
        let footprint = layout
            .levels()
            .get(level)
            .ok_or_else(|| MetalError::new("R8 upload", "mip out of range"))?;
        if seen[level]
            || upload.row_pitch < footprint.row_bytes
            || upload.row_pitch >= i32::MAX as usize
            || upload.row_pitch.checked_mul(footprint.rows) != Some(upload.bytes.len())
        {
            return Err(MetalError::new(
                "R8 upload",
                "duplicate mip or invalid pitch/span",
            ));
        }
        seen[level] = true;
        total = total
            .checked_add(upload.bytes.len())
            .filter(|n| *n <= MAX_R8_TEXTURE_BYTES)
            .ok_or_else(|| MetalError::new("R8 upload", "padded upload byte budget exceeded"))?;
    }
    Ok(layout)
}
impl MetalR8Texture {
    pub fn desc(&self) -> R8TextureDesc {
        self.desc
    }
    pub fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, MetalError> {
        self.texture.view(desc)
    }
    pub fn upload(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: R8TextureDesc,
        uploads: &[R8MipUpload<'_>],
    ) -> Result<Self, MetalError> {
        let layout = validate_r8_upload(desc, uploads)?;
        let format = MTLPixelFormat::R8Unorm;
        autoreleasepool(|_| {
            // SAFETY: Canonical layout and dimensions were checked before allocation.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    format,
                    desc.width,
                    desc.height,
                    desc.mips > 1,
                )
            };
            // SAFETY: PackedTextureLayout validated a nonempty count within the full mip chain.
            unsafe { descriptor.setMipmapLevelCount(desc.mips as usize) };
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .ok_or_else(|| {
                    MetalError::new("newTextureWithDescriptor", "R8 allocation failed")
                })?;
            for upload in uploads {
                let level = &layout.levels()[upload.level as usize];
                let region = MTLRegion {
                    origin: MTLOrigin { x: 0, y: 0, z: 0 },
                    size: MTLSize {
                        width: level.width,
                        height: level.height,
                        depth: 1,
                    },
                };
                // SAFETY: Entire fresh mip and exact checked
                // padded span. Single rows use Apple's required zero stride.
                unsafe {
                    texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        upload.level as usize,
                        NonNull::from(upload.bytes).cast(),
                        if level.rows == 1 { 0 } else { upload.row_pitch },
                    );
                }
            }
            Ok(Self {
                texture: NativeSampledTexture {
                    texture,
                    _parent: None,
                    width: desc.width,
                    height: desc.height,
                    mips: desc.mips,
                    format: TextureFormat::R8Unorm,
                },
                desc,
            })
        })
    }
}

/// Fully initialized immutable typed storage. Upload completes before publication;
/// GPU-produced versions use the separate provenance-retaining cohort interface.
#[derive(Clone, Debug)]
pub struct MetalTypedTexture {
    texture: NativeSampledTexture,
    desc: crate::TypedTextureDesc,
}
impl MetalTypedTexture {
    pub fn upload(
        gpu: &crate::NativeCompute,
        desc: crate::TypedTextureDesc,
        bytes: &[u8],
    ) -> Result<Self, MetalError> {
        let texture = gpu.upload_typed_texture_storage(desc, bytes)?;
        Ok(Self {
            texture: typed_sampled_storage(texture, desc)?,
            desc,
        })
    }
    pub fn desc(&self) -> crate::TypedTextureDesc {
        self.desc
    }
    pub fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, MetalError> {
        self.texture.view(desc)
    }
    pub fn array_slice_view(
        &self,
        desc: ArraySliceViewDesc,
    ) -> Result<MetalTextureView, MetalError> {
        self.texture.array_slice_view(desc)
    }
}

fn typed_sampled_storage(
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    desc: crate::TypedTextureDesc,
) -> Result<NativeSampledTexture, MetalError> {
    let kind = if desc.array_layers.is_some() {
        MTLTextureType::Type2DArray
    } else {
        MTLTextureType::Type2D
    };
    if texture.textureType() != kind
        || texture.pixelFormat() != storage_pixel_format(desc.format)
        || texture.width() != desc.extent[0]
        || texture.height() != desc.extent[1]
        || texture.mipmapLevelCount() != desc.mip_levels as usize
        || texture.arrayLength() != desc.array_layers.unwrap_or(1) as usize
    {
        return Err(MetalError::new(
            "typed texture",
            "native storage does not match authoritative descriptor",
        ));
    }
    Ok(NativeSampledTexture {
        texture,
        _parent: None,
        width: desc.extent[0],
        height: desc.extent[1],
        mips: desc.mip_levels,
        format: desc.format,
    })
}

/// Internal only: callers retain the exact Produced owner alongside this view.
pub(crate) fn typed_texture_view(
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    desc: crate::TypedTextureDesc,
    view: TextureViewDesc,
    slice: Option<u32>,
) -> Result<MetalTextureView, MetalError> {
    let storage = typed_sampled_storage(texture, desc)?;
    match slice {
        None => storage.view(view),
        Some(slice) => storage.array_slice_view(ArraySliceViewDesc {
            slice,
            base_mip: view.base_mip,
            mip_count: view.mip_count,
            format: view.format,
        }),
    }
}

pub(crate) fn storage_pixel_format(format: TextureFormat) -> MTLPixelFormat {
    match format {
        TextureFormat::R8Unorm => MTLPixelFormat::R8Unorm,
        TextureFormat::Rg8Unorm => MTLPixelFormat::RG8Unorm,
        TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        TextureFormat::R32Float => MTLPixelFormat::R32Float,
        TextureFormat::R32Uint => MTLPixelFormat::R32Uint,
        TextureFormat::Depth32Float => MTLPixelFormat::Depth32Float,
        TextureFormat::Bc1RgbaUnorm => MTLPixelFormat::BC1_RGBA,
        TextureFormat::Bc3RgbaUnorm => MTLPixelFormat::BC3_RGBA,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba8ArrayTextureDesc {
    pub width: usize,
    pub height: usize,
    pub mips: u32,
    pub array_layers: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Rgba8ArrayMipUpload<'a> {
    pub slice: u32,
    pub level: u32,
    pub row_pitch: usize,
    pub bytes: &'a [u8],
}
/// Fully initialized immutable array. Changed bytes require a new owner.
#[derive(Clone, Debug)]
pub struct MetalRgba8ArrayTexture {
    texture: NativeSampledTexture,
    desc: Rgba8ArrayTextureDesc,
}
impl MetalRgba8ArrayTexture {
    pub fn desc(&self) -> Rgba8ArrayTextureDesc {
        self.desc
    }
    pub fn array_slice_view(
        &self,
        desc: ArraySliceViewDesc,
    ) -> Result<MetalTextureView, MetalError> {
        self.texture.array_slice_view(desc)
    }
    pub(crate) fn from_render_output(
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        desc: Rgba8ArrayTextureDesc,
    ) -> Result<Self, MetalError> {
        if texture.textureType() != MTLTextureType::Type2DArray
            || texture.pixelFormat() != MTLPixelFormat::RGBA8Unorm
            || texture.width() != desc.width
            || texture.height() != desc.height
            || texture.mipmapLevelCount() != desc.mips as usize
            || texture.arrayLength() != desc.array_layers as usize
        {
            return Err(MetalError::new("array view", "native backing mismatch"));
        }
        Ok(Self {
            texture: NativeSampledTexture {
                texture,
                _parent: None,
                width: desc.width,
                height: desc.height,
                mips: desc.mips,
                format: TextureFormat::Rgba8Unorm,
            },
            desc,
        })
    }
    pub fn upload(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: Rgba8ArrayTextureDesc,
        uploads: &[Rgba8ArrayMipUpload<'_>],
    ) -> Result<Self, MetalError> {
        if desc.width > MAX_TEXTURE_DIMENSION
            || desc.height > MAX_TEXTURE_DIMENSION
            || desc.array_layers == 0
            || desc.array_layers > 256
            || desc
                .mips
                .checked_mul(desc.array_layers)
                .is_none_or(|n| n > 4096)
        {
            return Err(MetalError::new(
                "array upload",
                "dimension/subresource limit",
            ));
        }
        let layout = PackedTextureArrayLayout::new(
            TextureFormat::Rgba8Unorm,
            desc.width,
            desc.height,
            desc.mips,
            desc.array_layers,
        )
        .map_err(|_| MetalError::new("array upload", "invalid layout"))?;
        let count = desc.mips as usize * desc.array_layers as usize;
        if layout.byte_len() > 64 * 1024 * 1024 || uploads.len() != count {
            return Err(MetalError::new(
                "array upload",
                "byte or subresource count mismatch",
            ));
        }
        let mut seen = vec![false; count];
        let mut total = 0usize;
        for upload in uploads {
            let mip = layout
                .footprint(upload.slice, upload.level)
                .map_err(|_| MetalError::new("array upload", "invalid subresource"))?;
            let index = upload.slice as usize * desc.mips as usize + upload.level as usize;
            if seen[index]
                || upload.row_pitch < mip.row_bytes
                || !upload.row_pitch.is_multiple_of(4)
                || upload.row_pitch >= i32::MAX as usize * 4
                || upload.row_pitch.checked_mul(mip.height) != Some(upload.bytes.len())
            {
                return Err(MetalError::new(
                    "array upload",
                    "duplicate or invalid pitch/span",
                ));
            }
            seen[index] = true;
            total = total
                .checked_add(upload.bytes.len())
                .filter(|n| *n <= 64 * 1024 * 1024)
                .ok_or_else(|| MetalError::new("array upload", "padded upload budget"))?;
        }
        autoreleasepool(|_| {
            // SAFETY: Full dimensions and mip/layer counts validated before allocation.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::RGBA8Unorm,
                    desc.width,
                    desc.height,
                    desc.mips > 1,
                )
            };
            descriptor.setTextureType(MTLTextureType::Type2DArray);
            unsafe {
                descriptor.setMipmapLevelCount(desc.mips as usize);
                descriptor.setArrayLength(desc.array_layers as usize);
            }
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);
            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .ok_or_else(|| MetalError::new("array upload", "allocation failed"))?;
            for upload in uploads {
                let mip = layout
                    .footprint(upload.slice, upload.level)
                    .expect("prevalidated subresource");
                // SAFETY: Every initialized row span validated; fresh texture has no GPU users.
                unsafe {
                    texture.replaceRegion_mipmapLevel_slice_withBytes_bytesPerRow_bytesPerImage(
                        MTLRegion {
                            origin: MTLOrigin { x: 0, y: 0, z: 0 },
                            size: MTLSize {
                                width: mip.width,
                                height: mip.height,
                                depth: 1,
                            },
                        },
                        upload.level as usize,
                        upload.slice as usize,
                        NonNull::from(upload.bytes).cast(),
                        if mip.height == 1 { 0 } else { upload.row_pitch },
                        0,
                    );
                }
            }
            Self::from_render_output(texture, desc)
        })
    }
}

/// Safe wrapper owning a native Metal RGBA8 2D texture.
#[derive(Debug, Clone)]
pub struct MetalRgba8Texture {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    // Normalized private view bindings retain their backing allocation explicitly.
    _parent: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    desc: Rgba8TextureDesc,
}

impl MetalRgba8Texture {
    /// Select a real shared-storage mip range. View-relative level zero is the
    /// selected parent base mip. Linear/sRGB conversion needs no PixelFormatView.
    pub fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, MetalError> {
        NativeSampledTexture::from_rgba(self).view(desc)
    }

    // Only cohort normalization may wrap a produced render texture. Never expose
    // this upload-capable wrapper through the public produced-output handle.
    pub(crate) fn from_render_output(
        texture: Retained<ProtocolObject<dyn MTLTexture>>,
        width: usize,
        height: usize,
    ) -> Result<Self, MetalError> {
        let mips = u32::try_from(texture.mipmapLevelCount())
            .map_err(|_| MetalError::new("produced texture", "invalid mip count"))?;
        Ok(Self {
            texture,
            desc: Rgba8TextureDesc::new(width, height, mips)?,
            _parent: None,
        })
    }

    /// Allocate an empty native Metal RGBA8 2D texture with storage mode Shared and ShaderRead usage.
    pub fn allocate(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: Rgba8TextureDesc,
    ) -> Result<Self, MetalError> {
        // Public descriptor fields may bypass the constructor. Validate again at
        // the native boundary before making any Objective-C allocation calls.
        let desc = Rgba8TextureDesc::new(desc.width, desc.height, desc.mips)?;
        autoreleasepool(|_| {
            // SAFETY: Validated non-zero dimensions within MAX_TEXTURE_PIXELS and RGBA8Unorm format.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::RGBA8Unorm,
                    desc.width,
                    desc.height,
                    desc.mips > 1,
                )
            };
            if desc.mips > 1 {
                // SAFETY: desc.mips is validated by TextureMetadata to not exceed max_mip_levels.
                unsafe {
                    descriptor.setMipmapLevelCount(desc.mips as usize);
                }
            }
            descriptor.setStorageMode(MTLStorageMode::Shared);
            descriptor.setResourceOptions(MTLResourceOptions::StorageModeShared);
            descriptor.setUsage(MTLTextureUsage::ShaderRead);

            let texture = device
                .newTextureWithDescriptor(&descriptor)
                .ok_or_else(|| {
                    MetalError::new(
                        "newTextureWithDescriptor",
                        "failed to allocate RGBA8 Metal texture",
                    )
                })?;

            Ok(Self {
                texture,
                desc,
                _parent: None,
            })
        })
    }

    /// Upload a single validated mip level into this texture.
    pub fn upload_mip(&self, mip: &Rgba8MipLevel<'_>) -> Result<(), MetalError> {
        let (expected_w, expected_h) = self.desc.mip_extent(mip.level)?;
        if mip.width != expected_w || mip.height != expected_h {
            return Err(MetalError::new(
                "mip dimensions",
                format!(
                    "mip level {} expected {}x{}, got {}x{}",
                    mip.level, expected_w, expected_h, mip.width, mip.height
                ),
            ));
        }

        let tight_pitch = mip
            .width
            .checked_mul(RGBA8_TEXEL_BYTES)
            .ok_or_else(|| MetalError::new("row pitch", "row pitch overflow"))?;
        if mip.row_pitch < tight_pitch {
            return Err(MetalError::new(
                "row pitch",
                format!(
                    "row pitch {} smaller than tight pitch {}",
                    mip.row_pitch, tight_pitch
                ),
            ));
        }

        let min_bytes = mip
            .row_pitch
            .checked_mul(mip.height)
            .ok_or_else(|| MetalError::new("mip bytes", "byte length calculation overflow"))?;
        if mip.bytes.len() < min_bytes {
            return Err(MetalError::new(
                "mip buffer bounds",
                format!(
                    "buffer length {} is less than required {}",
                    mip.bytes.len(),
                    min_bytes
                ),
            ));
        }

        autoreleasepool(|_| {
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: mip.width,
                    height: mip.height,
                    depth: 1,
                },
            };

            let ptr = NonNull::new(mip.bytes.as_ptr() as *mut c_void).ok_or_else(|| {
                MetalError::new("pointer", "null data pointer for texture upload")
            })?;

            // SAFETY: The bounds check verifies that mip.bytes has at least
            // `mip.row_pitch * mip.height` bytes, the region size matches mip dimensions,
            // the mip level is within the texture's allocated mip count, and the pointer is valid.
            unsafe {
                self.texture
                    .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        mip.level as usize,
                        ptr,
                        mip.row_pitch,
                    );
            }

            Ok(())
        })
    }

    /// Allocate and populate an RGBA8 2D texture from a set of mip levels.
    pub fn upload(
        device: &ProtocolObject<dyn MTLDevice>,
        desc: Rgba8TextureDesc,
        mips: &[Rgba8MipLevel<'_>],
    ) -> Result<Self, MetalError> {
        let texture = Self::allocate(device, desc)?;
        for mip in mips {
            texture.upload_mip(mip)?;
        }
        Ok(texture)
    }

    /// Access the underlying native Metal texture protocol object.
    #[inline]
    pub fn raw(&self) -> &ProtocolObject<dyn MTLTexture> {
        &self.texture
    }

    pub(crate) fn device_id(&self) -> u64 {
        self.texture.device().registryID()
    }

    /// Access the texture descriptor.
    #[inline]
    pub fn desc(&self) -> &Rgba8TextureDesc {
        &self.desc
    }
}

/// Resource binding indices and lifetime conventions for sampling pipelines.
///
/// In the Xenora fixture ABI and translated shader conventions:
/// - Metal textures are bound to `[[texture(slot)]]`.
/// - Metal samplers are bound to `[[sampler(slot)]]`.
///
/// Under future RDNA2-to-Metal shader translation:
/// - An image resource descriptor (`T#`) maps to a `texture2d` binding.
/// - A sampler descriptor (`S#`) maps to a `sampler` binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingSlot {
    pub index: u32,
}

impl BindingSlot {
    pub const fn new(index: u32) -> Self {
        Self { index }
    }
}

/// A validated texture-sampler pair ready for pipeline binding.
///
/// This struct holds strong references to the native Metal texture and sampler,
/// verifying that LOD ranges and explicit mip levels are consistent before
/// submitting draw or compute work.
#[derive(Debug, Clone)]
pub struct BoundSampledTexture {
    texture: MetalRgba8Texture,
    sampler: MetalSampler,
    texture_slot: BindingSlot,
    sampler_slot: BindingSlot,
}

impl BoundSampledTexture {
    /// Bind a texture and sampler together to specified shader slots.
    ///
    /// # Errors
    ///
    /// Returns [`MetalError`] if the sampler's explicit mip or LOD constraints
    /// are out of bounds for the texture's mip chain, or devices differ.
    pub fn bind(
        texture: MetalRgba8Texture,
        sampler: MetalSampler,
        texture_slot: BindingSlot,
        sampler_slot: BindingSlot,
    ) -> Result<Self, MetalError> {
        Self::validate_pair(&texture, &sampler)?;

        Ok(Self {
            texture,
            sampler,
            texture_slot,
            sampler_slot,
        })
    }

    fn validate_pair(
        texture: &MetalRgba8Texture,
        sampler: &MetalSampler,
    ) -> Result<(), MetalError> {
        if texture.device_id() != sampler.device_id() {
            return Err(MetalError::new(
                "render bindings",
                "texture/sampler device mismatch",
            ));
        }
        let meta = TextureMetadata::new(
            texture.desc().width,
            texture.desc().height,
            texture.desc().mips,
        )
        .map_err(|err: SamplerError| MetalError::new("texture metadata", err.to_string()))?;

        sampler
            .desc()
            .validate_against_texture(&meta)
            .map_err(|err: SamplerError| {
                MetalError::new("sampler texture compatibility", err.to_string())
            })?;

        Ok(())
    }

    pub(crate) fn validate_for_device(
        &self,
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Result<(), MetalError> {
        Self::validate_pair(&self.texture, &self.sampler)?;
        if self.texture.device_id() != device.registryID() {
            return Err(MetalError::new(
                "render bindings",
                "binding/render device mismatch",
            ));
        }
        Ok(())
    }

    #[inline]
    pub fn texture(&self) -> &MetalRgba8Texture {
        &self.texture
    }

    #[inline]
    pub fn sampler(&self) -> &MetalSampler {
        &self.sampler
    }

    #[inline]
    pub fn texture_slot(&self) -> BindingSlot {
        self.texture_slot
    }

    #[inline]
    pub fn sampler_slot(&self) -> BindingSlot {
        self.sampler_slot
    }
}

#[cfg(test)]
mod descriptor_tests {
    use super::*;

    #[test]
    fn malformed_public_descriptors_cannot_bypass_extent_validation() {
        for desc in [
            Rgba8TextureDesc {
                width: 0,
                height: 1,
                mips: 1,
            },
            Rgba8TextureDesc {
                width: 1,
                height: 0,
                mips: 1,
            },
            Rgba8TextureDesc {
                width: 1,
                height: 1,
                mips: 65,
            },
            Rgba8TextureDesc {
                width: 16385,
                height: 1,
                mips: 1,
            },
            Rgba8TextureDesc {
                width: usize::MAX,
                height: 2,
                mips: 1,
            },
        ] {
            assert!(desc.mip_extent(0).is_err(), "descriptor {desc:?}");
        }
        assert!(Rgba8TextureDesc::new(16385, 1, 1).is_err());
        let valid = Rgba8TextureDesc::new(4, 2, 3).unwrap();
        assert_eq!(valid.mip_extent(0).unwrap(), (4, 2));
        assert_eq!(valid.mip_extent(1).unwrap(), (2, 1));
        assert_eq!(valid.mip_extent(2).unwrap(), (1, 1));
        assert!(valid.mip_extent(3).is_err());
    }

    #[test]
    fn malformed_packed_upload_descriptor_does_not_overflow() {
        let mip = Rgba8MipLevel::new_packed(0, usize::MAX, 1, &[]);
        assert_eq!(mip.row_pitch, usize::MAX);
    }
}

#[cfg(test)]
mod view_identity_tests {
    use super::*;
    #[test]
    #[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
    fn native_views_share_the_exact_parent_allocation() {
        let gpu = crate::NativeCompute::new().unwrap();
        let parent =
            MetalRgba8Texture::allocate(gpu.device(), Rgba8TextureDesc::new(4, 4, 3).unwrap())
                .unwrap();
        for format in [
            TextureViewFormat::Rgba8Unorm,
            TextureViewFormat::Rgba8UnormSrgb,
        ] {
            let view = parent
                .view(TextureViewDesc {
                    base_mip: 1,
                    mip_count: 2,
                    format,
                })
                .unwrap();
            let actual_parent = view
                .texture
                .texture
                .parentTexture()
                .expect("real Metal view parent");
            assert!(std::ptr::eq(&*actual_parent, parent.raw()));
            assert_eq!(view.texture.texture.parentRelativeLevel(), 1);
            assert_eq!(view.texture.texture.mipmapLevelCount(), 2);
            assert_eq!(view.texture.texture.width(), 2);
            let sampler = crate::MetalSampler::new(
                gpu.device(),
                xenora_texture::SamplerDesc::nearest_clamp(),
            )
            .unwrap();
            let normalized = view
                .bind(sampler, BindingSlot::new(0), BindingSlot::new(0))
                .unwrap();
            drop(view);
            let retained_parent = normalized
                .texture
                ._parent
                .as_ref()
                .expect("normalized binding must own parent");
            assert!(std::ptr::eq(&**retained_parent, parent.raw()));
            let pending_clone = normalized.clone();
            drop(normalized);
            assert!(std::ptr::eq(
                &**pending_clone.texture._parent.as_ref().unwrap(),
                parent.raw()
            ));
        }
    }
}

#[cfg(test)]
mod bc_upload_tests {
    use super::*;
    #[test]
    fn bc_upload_preflight_checks_all_levels_and_capability_without_gpu() {
        let bytes = [0u8; 32];
        let desc = BcTextureDesc {
            width: 5,
            height: 5,
            mips: 2,
            format: TextureFormat::Bc1RgbaUnorm,
        };
        let valid = [
            BcMipUpload {
                level: 0,
                row_pitch: 16,
                bytes: &bytes,
            },
            BcMipUpload {
                level: 1,
                row_pitch: 8,
                bytes: &bytes[..8],
            },
        ];
        assert_eq!(
            validate_bc_upload(desc, &valid, true).unwrap().byte_len(),
            40
        );
        assert_eq!(
            validate_bc_upload(desc, &valid, false)
                .unwrap_err()
                .operation,
            MetalError::BC_UNAVAILABLE
        );
        let mut invalid = valid;
        invalid[1].level = 0;
        assert!(validate_bc_upload(desc, &invalid, true).is_err());
        invalid = valid;
        invalid[1].row_pitch = 9;
        assert!(validate_bc_upload(desc, &invalid, true).is_err());
        invalid = valid;
        invalid[1].bytes = &bytes[..7];
        assert!(validate_bc_upload(desc, &invalid, true).is_err());
        invalid = valid;
        invalid[1].row_pitch = usize::MAX;
        assert!(validate_bc_upload(desc, &invalid, true).is_err());
        assert!(validate_bc_upload(desc, &valid[..1], true).is_err());
        assert!(
            validate_bc_upload(
                BcTextureDesc {
                    format: TextureFormat::Rgba8Unorm,
                    ..desc
                },
                &valid,
                true
            )
            .is_err()
        );
        assert!(
            validate_bc_upload(
                BcTextureDesc {
                    width: usize::MAX,
                    ..desc
                },
                &valid,
                true
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod r8_validation_tests {
    use super::*;
    #[test]
    fn r8_upload_preflight_checks_all_mips_pitches_and_spans() {
        let desc = R8TextureDesc {
            width: 5,
            height: 3,
            mips: 3,
        };
        let uploads = [
            R8MipUpload {
                level: 0,
                row_pitch: 7,
                bytes: &[99; 21],
            },
            R8MipUpload {
                level: 1,
                row_pitch: 3,
                bytes: &[88; 3],
            },
            R8MipUpload {
                level: 2,
                row_pitch: 1,
                bytes: &[77],
            },
        ];
        assert_eq!(validate_r8_upload(desc, &uploads).unwrap().byte_len(), 18);
        assert!(validate_r8_upload(desc, &uploads[..2]).is_err());
        for bad in [
            R8MipUpload {
                level: 0,
                row_pitch: 4,
                bytes: &[0; 12],
            },
            R8MipUpload {
                level: 0,
                row_pitch: 7,
                bytes: &[0; 20],
            },
            R8MipUpload {
                level: 1,
                row_pitch: 7,
                bytes: &[0; 21],
            },
            R8MipUpload {
                level: 3,
                row_pitch: 7,
                bytes: &[0; 21],
            },
            R8MipUpload {
                level: 0,
                row_pitch: i32::MAX as usize,
                bytes: &[],
            },
            R8MipUpload {
                level: 0,
                row_pitch: usize::MAX,
                bytes: &[],
            },
        ] {
            let mut invalid = uploads;
            invalid[0] = bad;
            assert!(validate_r8_upload(desc, &invalid).is_err());
        }
        assert!(validate_r8_upload(R8TextureDesc { width: 0, ..desc }, &uploads).is_err());
    }
    #[test]
    fn r8_padded_budget_is_checked_before_native_allocation() {
        let bytes = vec![0; MAX_R8_TEXTURE_BYTES + 1];
        assert!(
            validate_r8_upload(
                R8TextureDesc {
                    width: 1,
                    height: 1,
                    mips: 1
                },
                &[R8MipUpload {
                    level: 0,
                    row_pitch: bytes.len(),
                    bytes: &bytes
                }]
            )
            .is_err()
        );
    }
}
