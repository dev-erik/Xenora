// SPDX-License-Identifier: GPL-3.0-or-later

use crate::texture_pipeline::{NativeSampledBinding, NativeSampledTexture};
use std::ptr::NonNull;

#[path = "render_cohort.rs"]
mod cohort;
pub use cohort::{
    OwnedDepthInput, OwnedRenderOutput, QueuedSampledBinding, RenderBufferOutput, RenderCohort,
    RenderColorOutput, RenderColorView, RenderPoint, RenderQueue, RenderTypedTextureOutput,
};

use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_metal::MTLBuffer;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBlitCommandEncoder, MTLClearColor, MTLColorWriteMask,
    MTLCommandBuffer, MTLCommandBufferStatus, MTLCommandEncoder, MTLCommandQueue,
    MTLCompareFunction, MTLDepthStencilDescriptor, MTLDepthStencilState, MTLDevice, MTLFunction,
    MTLIndexType, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLScissorRect, MTLShaderValidation, MTLSize, MTLStorageMode,
    MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage, MTLViewport,
};

use crate::{TriangleDraw, TrianglePass, TriangleState};
use xenora_texture::{Bc1Subresource, Bc1Upload};

use super::{
    ComputeInput, MetalError, NativeCompute, compile_options, complete_command, find_function,
    validate_source,
};

const MAX_PIXELS: usize = 4096;

/// Maximum aggregate vertex count in a translated pass (complete triangles).
pub const MAX_RENDER_VERTICES: usize = 65535;
/// Maximum pixel count per translated color/depth attachment.
pub const MAX_RENDER_PIXELS: usize = 1024 * 1024;
/// Maximum width or height of a translated attachment.
pub const MAX_RENDER_DIMENSION: usize = 4096;
/// Maximum ordered draw count in a translated pass.
pub const MAX_RENDER_DRAWS: usize = 256;

// Per pending translated pass: up to 8 * 8 * MAX_RENDER_PIXELS typed color
// texture bytes (64 MiB), plus the same amount in returned host readbacks;
// RGBA8-only passes retain the previous 32 MiB color bound.
// a working depth texture adds 4 MiB, alongside any previously retained depth.
// Two native float4 vertex/color buffers total 32 * MAX_RENDER_VERTICES bytes.
// Depth host transfers pad rows to 256 bytes: at most 4 * pixels + 252 * height
// bytes each (less than 5 MiB), alongside tightly packed 4-byte depth snapshots.
// Sampled resources, pipeline/driver overhead and concurrent passes are separate.
fn translated_pixels(
    [width, height]: [usize; 2],
    operation: &'static str,
) -> Result<usize, MetalError> {
    if !(1..=MAX_RENDER_DIMENSION).contains(&width) || !(1..=MAX_RENDER_DIMENSION).contains(&height)
    {
        return Err(MetalError::new(
            operation,
            "expected dimensions in 1..=4096",
        ));
    }
    width
        .checked_mul(height)
        .filter(|&pixels| pixels <= MAX_RENDER_PIXELS)
        .ok_or_else(|| MetalError::new(operation, "expected at most 1048576 pixels"))
}

#[path = "typed_render.rs"]
mod typed_render;
pub use typed_render::{TypedColorAttachment, TypedColorClear, TypedTriangleTargetsPass};

const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::RGBA8Unorm;

/// Retained, single-sample Depth32Float surface for synchronous translated passes.
/// Contents become initialized only after a successful clear submission.
/// Exclusive mutable access serializes submissions using this surface.
pub struct MetalDepthTarget {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    extent: [usize; 2],
    device_id: u64,
    initialized: bool,
}

/// Borrowed translated shader selection; submission owns the compiled native state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriangleShader<'s> {
    pub source: &'s str,
    pub entries: [&'s str; 2],
}

/// Trusted shader requirement metadata and an owned eight-word per-draw snapshot.
/// A nonzero requirement mask requires values; no shader source inspection occurs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawUniforms {
    pub required_mask: u8,
    pub values: Option<[u32; 8]>,
}

fn validate_uniforms(draws: usize, uniforms: Option<&[DrawUniforms]>) -> Result<(), MetalError> {
    if let Some(uniforms) = uniforms {
        if uniforms.len() != draws {
            return Err(MetalError::new(
                "render uniforms",
                "expected one uniform snapshot per draw",
            ));
        }
        if uniforms
            .iter()
            .any(|u| u.required_mask != 0 && u.values.is_none())
        {
            return Err(MetalError::new(
                "render uniforms",
                "required uniform values missing",
            ));
        }
    }
    Ok(())
}

type MultiPipelineInputs<'a> = (&'a [TriangleShader<'a>], &'a [Vec<NativeSampledBinding>]);

#[derive(Default)]
struct DrawBindings<'a> {
    textures: [Option<&'a NativeSampledTexture>; 4],
    samplers: [Option<&'a crate::MetalSampler>; 4],
}

impl<'a> DrawBindings<'a> {
    fn normalize(
        pairs: &'a [NativeSampledBinding],
        device: &ProtocolObject<dyn MTLDevice>,
    ) -> Result<Self, MetalError> {
        if pairs.len() > 16 {
            return Err(MetalError::new(
                "render bindings",
                "expected at most sixteen sampled pairs per draw",
            ));
        }
        let mut result = Self::default();
        for pair in pairs {
            pair.validate_for_device(device)?;
            let (t, s) = (
                pair.texture_slot().index as usize,
                pair.sampler_slot().index as usize,
            );
            if t >= 4 || s >= 4 {
                return Err(MetalError::new(
                    "render bindings",
                    "expected texture/sampler slots zero through three",
                ));
            }
            if result.textures[t].is_some_and(|old| !old.same_object(pair.texture()))
                || result.samplers[s].is_some_and(|old| !old.same_object(pair.sampler()))
            {
                return Err(MetalError::new(
                    "render bindings",
                    "conflicting objects for a sampled slot",
                ));
            }
            result.textures[t] = Some(pair.texture());
            result.samplers[s] = Some(pair.sampler());
        }
        Ok(result)
    }
}

/// Submitted native rendering. Only retained depth is borrowed; all GPU inputs
/// are owned until completion. Dropping drains the work without publishing depth.
#[must_use = "wait to read outputs and publish retained depth; drop drains without publishing"]
pub struct PendingRender<'a> {
    storage: Option<cohort::StorageTarget>,
    command: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    textures: Vec<Retained<ProtocolObject<dyn MTLTexture>>>,
    color_formats: Vec<xenora_texture::TextureFormat>,
    working_depth: Option<Retained<ProtocolObject<dyn MTLTexture>>>,
    retained_depth: Option<&'a mut MetalDepthTarget>,
    extent: [usize; 2],
    _buffers: [Retained<ProtocolObject<dyn MTLBuffer>>; 2],
    _geometry: Vec<Option<std::rc::Rc<cohort::geometry::Gather>>>,
    validation: Option<std::rc::Rc<cohort::geometry::Validation>>,
    _uniform_buffers: Vec<Option<Retained<ProtocolObject<dyn MTLBuffer>>>>,
    _pipelines: Vec<Retained<ProtocolObject<dyn MTLRenderPipelineState>>>,
    _depth_states: Vec<Option<Retained<ProtocolObject<dyn MTLDepthStencilState>>>>,
    _bindings: Vec<NativeSampledBinding>,
    _color_sources: Vec<Option<crate::RenderTypedTextureOutput>>,
    _color_lease: Option<std::rc::Rc<cohort::ColorLease>>,
    _temporary_lease: Option<std::rc::Rc<cohort::ColorLease>>,
    _owned_depth: Option<MetalDepthTarget>,
    _export_depth: bool,
    _depth_source_owner: Option<crate::RenderDepthOutput>,
    _depth_lease: Option<std::rc::Rc<cohort::ColorLease>>,
    _depth_upload: Option<Retained<ProtocolObject<dyn MTLBuffer>>>,
    _sampled: Option<(crate::MetalRgba8Texture, crate::MetalSampler)>,
}

impl PendingRender<'_> {
    /// Exact layouts of the allocated color attachments, available before collection.
    pub fn color_layouts(&self) -> Vec<crate::TypedTextureDesc> {
        self.color_formats
            .iter()
            .map(|format| crate::TypedTextureDesc {
                format: *format,
                extent: self.extent,
                mip_levels: 1,
                array_layers: None,
            })
            .collect()
    }

    /// Poll once without waiting. GPU errors are returned, never treated as success.
    pub fn is_complete(&self) -> Result<bool, MetalError> {
        let status = self.command.status();
        if status == MTLCommandBufferStatus::Error {
            return Err(self.completion_error("command status"));
        }
        if status == MTLCommandBufferStatus::Completed
            && self.validation.as_ref().is_some_and(|v| !v.succeeded())
        {
            return Err(MetalError::new(
                "geometry gather",
                "invalid geometry contents",
            ));
        }
        Ok(status == MTLCommandBufferStatus::Completed)
    }

    /// Wait for completion, read all colors, then publish staged retained depth.
    pub fn wait(self) -> Result<Vec<Vec<u8>>, MetalError> {
        self.wait_timed().map(|(outputs, _)| outputs)
    }

    fn completion_error(&self, operation: &'static str) -> MetalError {
        let detail = self.command.error().map_or_else(
            || format!("unexpected command status {:?}", self.command.status()),
            |error| error.localizedDescription().to_string(),
        );
        MetalError::new(operation, detail)
    }

    fn wait_timed(mut self) -> Result<(Vec<Vec<u8>>, Option<f64>), MetalError> {
        self.command.waitUntilCompleted();
        if self.command.status() != MTLCommandBufferStatus::Completed {
            return Err(self.completion_error("waitUntilCompleted"));
        }
        if self.validation.as_ref().is_some_and(|v| !v.succeeded()) {
            return Err(MetalError::new(
                "geometry gather",
                "invalid geometry contents",
            ));
        }
        let [width, height] = self.extent;
        let mut outputs = Vec::with_capacity(self.textures.len());
        for (texture, format) in self.textures.iter().zip(&self.color_formats) {
            let row_bytes = width * format.block_bytes();
            let mut output = vec![0; row_bytes * height];
            // SAFETY: GPU completion precedes reading these owned shared textures;
            // validated extents fit the initialized, tightly packed allocation.
            unsafe {
                texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    NonNull::from(output.as_mut_slice()).cast(),
                    row_bytes,
                    whole(width, height),
                    0,
                );
            }
            outputs.push(output);
        }
        let start = self.command.GPUStartTime();
        let seconds = self.command.GPUEndTime() - start;
        let timing = (start > 0.0 && seconds.is_finite() && seconds > 0.0).then_some(seconds);
        if let Some(depth) = self.retained_depth.as_deref_mut() {
            depth.texture = self
                .working_depth
                .take()
                .expect("retained depth attachment");
            depth.initialized = true;
        }
        Ok((outputs, timing))
    }
}

impl Drop for PendingRender<'_> {
    fn drop(&mut self) {
        // Keep every owned input alive through completion, including on errors.
        // Dropping intentionally leaves the borrowed depth's old surface intact.
        self.command.waitUntilCompleted();
    }
}

/// Owned, immutable depth bytes suitable for moving between loader threads.
/// Native readback or checked typed-copy publication creates packed snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepthSnapshot {
    extent: [usize; 2],
    words: Vec<u32>,
}

impl DepthSnapshot {
    /// Exact logical shape; equal byte lengths do not imply compatible depth layouts.
    pub fn extent(&self) -> [usize; 2] {
        self.extent
    }
    /// Preserve exact Depth32Float storage bits, without numeric conversion.
    pub fn from_packed_bytes(extent: [usize; 2], bytes: &[u8]) -> Result<Self, MetalError> {
        let pixels = translated_pixels(extent, "depth snapshot")?;
        if bytes.len() != pixels * 4 {
            return Err(MetalError::new(
                "depth snapshot",
                "packed byte length mismatch",
            ));
        }
        let mut words = Vec::new();
        words
            .try_reserve_exact(pixels)
            .map_err(|_| MetalError::new("depth snapshot", "allocation failed"))?;
        words.extend(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_le_bytes(*b)),
        );
        Ok(Self { extent, words })
    }
    pub fn to_packed_bytes(&self) -> Vec<u8> {
        self.words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect()
    }
    /// Retained host storage, excluding temporary padded transfer buffers.
    pub fn byte_len(&self) -> usize {
        self.words.len() * 4
    }
}

/// Action applied to a retained depth target at the start of a pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthLoad {
    Clear(f32),
    Load,
}

impl NativeCompute {
    /// Draw one triangle with trusted MSL into a cleared RGBA8Unorm target.
    ///
    /// The entries are ordered vertex, then fragment. Vertex slot zero holds
    /// three clip-space float4 positions. Vertex slot one holds three float4
    /// RGBA colors. The stages must restrict accesses to these inputs and
    /// the single color output. Use only trusted MSL in a process with a wall
    /// deadline. The returned bytes are tightly packed in top-to-bottom rows.
    ///
    /// # Errors
    ///
    /// Returns an error for non-finite inputs, invalid source or extent, or
    /// Metal compilation, setup or completion failure.
    pub fn render_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        vertices: &[[f32; 4]; 3],
        colors: &[[f32; 4]; 3],
        [width, height]: [usize; 2],
        clear: [u8; 4],
    ) -> Result<Vec<u8>, MetalError> {
        self.render_rgba8_sampled(
            source,
            entries,
            vertices,
            colors,
            [width, height],
            clear,
            None,
        )
    }

    /// Draw up to 21845 triangles in one native Metal triangle-list submission.
    /// Positions and colors must have equal lengths, a multiple of three up to
    /// 65535 vertices. Uses the same trusted shader contract as `render_rgba8`.
    ///
    /// # Errors
    /// Returns an error for malformed or oversized lists, non-finite inputs,
    /// invalid shaders/extents, or Metal compilation/setup/completion failure.
    pub fn render_triangles_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        vertices: &[[f32; 4]],
        colors: &[[f32; 4]],
        extent: [usize; 2],
        clear: [u8; 4],
    ) -> Result<Vec<u8>, MetalError> {
        self.render_rgba8_internal(source, entries, vertices, colors, extent, clear, None)
            .map(|(pixels, _)| pixels)
    }

    /// Execute ordered draws in one cleared target with per-draw viewports.
    /// Uses the trusted shader contract of `render_rgba8`; each draw restarts
    /// vertex IDs at zero and has its own position/color buffer view.
    /// Accepts 1..=256 draws, totaling at most 65535 vertices. A viewport is
    /// nonzero, starts at (0, 0), and must fit within the fixed target extent.
    /// Pipeline, target, clear color and sampled bindings are not per-draw state.
    ///
    /// # Errors
    /// Rejects malformed inputs before encoding; propagates shader and GPU errors.
    pub fn render_triangle_pass_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        draws: &[TriangleDraw<'_>],
        extent: [usize; 2],
        clear: [u8; 4],
    ) -> Result<Vec<u8>, MetalError> {
        self.render_pass_internal(source, entries, draws, extent, clear, None, None, None)
            .map(|(pixels, _)| pixels)
    }

    /// Render translated shaders with per-draw depth, blending and write masks.
    /// Uses the same bounded input contract as `render_triangle_pass_rgba8`.
    ///
    /// # Errors
    /// Rejects mismatched states, depth state without an attachment, invalid clear
    /// depth or malformed geometry; propagates shader and GPU failures.
    pub fn render_triangle_pass_with_state_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &TrianglePass<'_>,
    ) -> Result<Vec<u8>, MetalError> {
        self.render_pass_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clear,
            None,
            Some((pass.states, pass.clear_depth)),
            None,
        )
        .map(|(pixels, _)| pixels)
    }

    /// Render a stateful pass with one retained texture/sampler binding per draw.
    pub fn render_sampled_triangle_pass_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &TrianglePass<'_>,
        bindings: &[crate::BoundSampledTexture],
    ) -> Result<Vec<u8>, MetalError> {
        self.render_pass_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clear,
            None,
            Some((pass.states, pass.clear_depth)),
            Some(bindings),
        )
        .map(|(pixels, _)| pixels)
    }

    /// Render 1..=8 color attachments and return their readbacks in slot order.
    pub fn render_triangle_targets_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &crate::TriangleTargetsPass<'_>,
    ) -> Result<Vec<Vec<u8>>, MetalError> {
        self.render_targets_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            None,
            None,
            None,
        )
        .map(|(targets, _)| targets)
    }

    /// Render bound per-draw sampled resources with multiple color targets and optional retained depth.
    /// Retained depth uses `DepthLoad` exclusively; `pass.clear_depth` must be absent.
    pub fn render_bound_triangle_targets_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        depth: Option<(&mut MetalDepthTarget, DepthLoad)>,
    ) -> Result<Vec<Vec<u8>>, MetalError> {
        if depth.is_some() && pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        self.render_targets_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            bindings,
            depth,
            None,
        )
        .map(|(targets, _)| targets)
    }

    /// Render into fresh color attachments, loading packed RGBA8 bytes per slot.
    /// `color_loads` must match the attachment count. Each Some slice must contain
    /// exactly width * height * 4 bytes; None uses the corresponding clear color.
    /// Zero draws are allowed. Caller bytes are never modified, including on failure.
    pub fn render_loaded_triangle_targets_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        depth: Option<(&mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
    ) -> Result<Vec<Vec<u8>>, MetalError> {
        if depth.is_some() && pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        self.render_targets_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            bindings,
            depth,
            Some(color_loads),
        )
        .map(|(targets, _)| targets)
    }

    /// Encode and commit loaded rendering without waiting or reading back.
    /// Input slices, source and this context may be dropped after submission;
    /// only the optional retained-depth target remains exclusively borrowed.
    pub fn submit_loaded_triangle_targets_rgba8<'a>(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
    ) -> Result<PendingRender<'a>, MetalError> {
        if depth.is_some() && pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        self.submit_targets_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            bindings,
            depth,
            Some(color_loads),
            None,
            None,
            None,
        )
    }

    /// Submit one shader and optional sampled binding per draw in a single pass.
    /// Empty passes require empty shader/binding slices and compile no shader.
    /// All selected shaders compile before command submission; only depth is borrowed.
    pub fn submit_multi_pipeline_triangle_targets_rgba8<'a>(
        &self,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Option<crate::BoundSampledTexture>],
        retained_depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
    ) -> Result<PendingRender<'a>, MetalError> {
        if retained_depth.is_some() && pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        if shaders.len() != pass.draws.len() || bindings.len() != pass.draws.len() {
            return Err(MetalError::new(
                "render shaders",
                "expected one shader and optional binding per draw",
            ));
        }
        if bindings
            .iter()
            .flatten()
            .any(|b| b.texture_slot().index != 0 || b.sampler_slot().index != 0)
        {
            return Err(MetalError::new(
                "render bindings",
                "expected slot-zero texture/sampler bindings",
            ));
        }
        if pass.draws.len() > MAX_RENDER_DRAWS {
            return Err(MetalError::new("render input", "expected 1..=256 draws"));
        }
        let pairs: Vec<Vec<_>> = bindings
            .iter()
            .map(|binding| binding.iter().cloned().collect())
            .collect();
        self.submit_multi_binding_triangle_targets_rgba8(
            shaders,
            pass,
            &pairs,
            retained_depth,
            color_loads,
        )
    }

    /// Submit independently indexed texture/sampler bindings for each draw.
    /// Each collection contains at most sixteen pairs using slots 0 through 3. Repeated
    /// slots must refer to the same native object; clones are accepted. All slots
    /// are rebound or cleared per draw, and the ticket owns every supplied object.
    pub fn submit_multi_binding_triangle_targets_rgba8<'a>(
        &self,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<crate::BoundSampledTexture>],
        depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
    ) -> Result<PendingRender<'a>, MetalError> {
        if bindings.len() != pass.draws.len()
            || bindings.len() > MAX_RENDER_DRAWS
            || bindings.iter().any(|b| b.len() > 16)
        {
            return Err(MetalError::new("render bindings", "invalid binding count"));
        }
        let normalized: Vec<Vec<_>> = bindings
            .iter()
            .map(|draw| draw.iter().map(NativeSampledBinding::from_whole).collect())
            .collect();
        self.submit_native_bindings(shaders, pass, &normalized, depth, color_loads, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_native_bindings<'a>(
        &self,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<NativeSampledBinding>],
        depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
        uniforms: Option<&[DrawUniforms]>,
    ) -> Result<PendingRender<'a>, MetalError> {
        if depth.is_some() && pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        self.submit_targets_internal(
            "",
            ["", ""],
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, pass.clear_depth)),
            None,
            depth,
            Some(color_loads),
            Some((shaders, bindings)),
            None,
            uniforms,
        )
    }

    /// Submit mixed whole textures and immutable mip-range views. Public view
    /// handles never expose the upload-capable bindings used during normalization.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_view_binding_triangle_targets_rgba8<'a>(
        &self,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<crate::SampledBinding>],
        depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
    ) -> Result<PendingRender<'a>, MetalError> {
        let uniforms = vec![DrawUniforms::default(); pass.draws.len().min(MAX_RENDER_DRAWS + 1)];
        self.submit_view_binding_triangle_targets_with_uniforms_rgba8(
            shaders,
            pass,
            bindings,
            depth,
            color_loads,
            &uniforms,
        )
    }

    /// Submit immutable per-draw uniforms at buffer 2 in both shader stages.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_view_binding_triangle_targets_with_uniforms_rgba8<'a>(
        &self,
        shaders: &[TriangleShader<'_>],
        pass: &crate::TriangleTargetsPass<'_>,
        bindings: &[Vec<crate::SampledBinding>],
        depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: &[Option<&[u8]>],
        uniforms: &[DrawUniforms],
    ) -> Result<PendingRender<'a>, MetalError> {
        validate_uniforms(pass.draws.len(), Some(uniforms))?;
        if bindings.len() != pass.draws.len()
            || bindings.len() > MAX_RENDER_DRAWS
            || bindings.iter().any(|b| b.len() > 16)
        {
            return Err(MetalError::new(
                "render bindings",
                "invalid view binding count",
            ));
        }
        let normalized = bindings
            .iter()
            .map(|draw| {
                draw.iter()
                    .map(crate::SampledBinding::normalize)
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.submit_native_bindings(
            shaders,
            pass,
            &normalized,
            depth,
            color_loads,
            Some(uniforms),
        )
    }

    /// Allocate a retained depth surface with undefined initial contents.
    pub fn create_depth_target(&self, extent: [usize; 2]) -> Result<MetalDepthTarget, MetalError> {
        let [width, height] = extent;
        translated_pixels(extent, "depth target")?;
        Ok(MetalDepthTarget {
            texture: self.new_target(
                DEPTH_FORMAT,
                width,
                height,
                MTLStorageMode::Private,
                MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
            )?,
            extent,
            device_id: self.device.registryID(),
            initialized: false,
        })
    }

    /// Copy completed depth into owned host memory for thread-safe resource snapshots.
    pub fn snapshot_depth(&self, depth: &MetalDepthTarget) -> Result<DepthSnapshot, MetalError> {
        if !depth.initialized || depth.device_id != self.device.registryID() {
            return Err(MetalError::new(
                "depth snapshot",
                "uninitialized or foreign device depth",
            ));
        }
        translated_pixels(depth.extent, "depth snapshot")?;
        let [width, height] = depth.extent;
        let pitch = (width * 4).div_ceil(256) * 256;
        let mut words = vec![0u32; pitch / 4 * height];
        let buffer = self.new_buffer(&words)?;
        let command = self
            .queue
            .commandBuffer()
            .ok_or_else(|| MetalError::new("commandBuffer", "depth readback command failed"))?;
        let blit = command.blitCommandEncoder().ok_or_else(|| {
            MetalError::new("blitCommandEncoder", "depth readback encoder failed")
        })?;
        // SAFETY: The initialized private source and shared destination belong to
        // this device, are retained through synchronous completion, and the padded
        // buffer spans every row of the bounded single-layer/mip depth image.
        unsafe {
            blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
                &depth.texture,0,0,MTLOrigin{x:0,y:0,z:0},MTLSize{width,height,depth:1},
                &buffer,0,pitch,pitch*height);
        }
        blit.endEncoding();
        complete_command(&command)?;
        // SAFETY: Completed shared buffer owns exactly words.len()*4 readable
        // bytes, and words is a separate initialized allocation of that size.
        unsafe {
            std::ptr::copy_nonoverlapping(
                buffer.contents().cast::<u32>().as_ptr(),
                words.as_mut_ptr(),
                words.len(),
            );
        }
        let packed = words
            .chunks_exact(pitch / 4)
            .flat_map(|row| row[..width].iter().copied())
            .collect();
        Ok(DepthSnapshot {
            extent: depth.extent,
            words: packed,
        })
    }

    /// Restore an immutable host snapshot into a newly owned native depth surface.
    pub fn restore_depth(&self, snapshot: &DepthSnapshot) -> Result<MetalDepthTarget, MetalError> {
        let pixels = translated_pixels(snapshot.extent, "depth snapshot")?;
        let [width, height] = snapshot.extent;
        let pitch = (width * 4).div_ceil(256) * 256;
        if snapshot.words.len() != pixels {
            return Err(MetalError::new("depth snapshot", "invalid row storage"));
        }
        let mut depth = self.create_depth_target(snapshot.extent)?;
        let mut padded = vec![0; pitch / 4 * height];
        for (row, source) in padded
            .chunks_exact_mut(pitch / 4)
            .zip(snapshot.words.chunks_exact(width))
        {
            row[..width].copy_from_slice(source);
        }
        let buffer = self.new_buffer(&padded)?;
        let command = self
            .queue
            .commandBuffer()
            .ok_or_else(|| MetalError::new("commandBuffer", "depth upload command failed"))?;
        let blit = command
            .blitCommandEncoder()
            .ok_or_else(|| MetalError::new("blitCommandEncoder", "depth upload encoder failed"))?;
        // SAFETY: Snapshot layout is private and checked; padded shared buffer and
        // destination texture span the same bounded extent and remain alive until
        // synchronous completion. No published depth texture is modified.
        unsafe {
            blit.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                &buffer,0,pitch,pitch*height,MTLSize{width,height,depth:1},&depth.texture,0,0,MTLOrigin{x:0,y:0,z:0});
        }
        blit.endEncoding();
        complete_command(&command)?;
        depth.initialized = true;
        Ok(depth)
    }

    /// Render using retained depth. Load requires a previous successful clear.
    /// All failures preserve the previous depth contents; successful submissions
    /// atomically replace the owned surface after native command completion.
    /// pass.clear_depth must be None; load is the sole depth load/clear control.
    /// Zero draws are allowed for clear-only or load/store-only submissions.
    pub fn render_triangle_targets_with_depth_rgba8(
        &self,
        source: &str,
        entries: [&str; 2],
        pass: &crate::TriangleTargetsPass<'_>,
        depth: &mut MetalDepthTarget,
        load: DepthLoad,
    ) -> Result<Vec<Vec<u8>>, MetalError> {
        if pass.clear_depth.is_some() {
            return Err(MetalError::new("depth target", "ambiguous depth clear"));
        }
        self.render_targets_internal(
            source,
            entries,
            pass.draws,
            pass.extent,
            pass.clears,
            None,
            Some((pass.states, None)),
            None,
            Some((depth, load)),
            None,
        )
        .map(|(targets, _)| targets)
    }

    /// Draw one offscreen triangle and return readback and GPU command time in seconds.
    /// Timing is absent if Metal supplies no usable command timestamps.
    pub fn render_rgba8_profiled(
        &self,
        source: &str,
        entries: [&str; 2],
        vertices: &[[f32; 4]; 3],
        colors: &[[f32; 4]; 3],
        extent: [usize; 2],
        clear: [u8; 4],
    ) -> Result<(Vec<u8>, Option<f64>), MetalError> {
        self.render_rgba8_internal(source, entries, vertices, colors, extent, clear, None)
    }

    /// Draw one triangle with trusted MSL and optional bound sampled texture into a cleared RGBA8Unorm target.
    ///
    /// When `sampled` is provided, the texture is bound to fragment texture slot 0 and
    /// the sampler to fragment sampler slot 0.
    ///
    /// # Errors
    ///
    /// Returns an error for non-finite inputs, invalid source or extent, or
    /// Metal compilation, setup or completion failure.
    #[allow(clippy::too_many_arguments)]
    pub fn render_rgba8_sampled(
        &self,
        source: &str,
        entries: [&str; 2],
        vertices: &[[f32; 4]; 3],
        colors: &[[f32; 4]; 3],
        [width, height]: [usize; 2],
        clear: [u8; 4],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
    ) -> Result<Vec<u8>, MetalError> {
        self.render_rgba8_internal(
            source,
            entries,
            vertices,
            colors,
            [width, height],
            clear,
            sampled,
        )
        .map(|(pixels, _)| pixels)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_rgba8_internal(
        &self,
        source: &str,
        entries: [&str; 2],
        vertices: &[[f32; 4]],
        colors: &[[f32; 4]],
        [width, height]: [usize; 2],
        clear: [u8; 4],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
    ) -> Result<(Vec<u8>, Option<f64>), MetalError> {
        self.render_pass_internal(
            source,
            entries,
            &[TriangleDraw {
                vertices,
                colors,
                viewport: [width, height],
            }],
            [width, height],
            clear,
            sampled,
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_pass_internal(
        &self,
        source: &str,
        entries: [&str; 2],
        draws: &[TriangleDraw<'_>],
        [width, height]: [usize; 2],
        clear: [u8; 4],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
        pass_state: Option<(&[TriangleState], Option<f32>)>,
        bindings: Option<&[crate::BoundSampledTexture]>,
    ) -> Result<(Vec<u8>, Option<f64>), MetalError> {
        self.render_targets_internal(
            source,
            entries,
            draws,
            [width, height],
            &[clear],
            sampled,
            pass_state,
            bindings,
            None,
            None,
        )
        .map(|(mut targets, time)| (targets.remove(0), time))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_targets_internal(
        &self,
        source: &str,
        entries: [&str; 2],
        draws: &[TriangleDraw<'_>],
        [width, height]: [usize; 2],
        clears: &[[u8; 4]],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
        pass_state: Option<(&[TriangleState], Option<f32>)>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        retained_depth: Option<(&mut MetalDepthTarget, DepthLoad)>,
        color_loads: Option<&[Option<&[u8]>]>,
    ) -> Result<(Vec<Vec<u8>>, Option<f64>), MetalError> {
        self.submit_targets_internal(
            source,
            entries,
            draws,
            [width, height],
            clears,
            sampled,
            pass_state,
            bindings,
            retained_depth,
            color_loads,
            None,
            None,
            None,
        )?
        .wait_timed()
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_targets_internal<'a>(
        &self,
        source: &str,
        entries: [&str; 2],
        draws: &[TriangleDraw<'_>],
        [width, height]: [usize; 2],
        clears: &[[u8; 4]],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
        pass_state: Option<(&[TriangleState], Option<f32>)>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        retained_depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: Option<&[Option<&[u8]>]>,
        multi: Option<MultiPipelineInputs<'_>>,
        queued: Option<cohort::SubmissionSync>,
        uniforms: Option<&[DrawUniforms]>,
    ) -> Result<PendingRender<'a>, MetalError> {
        self.submit_targets_typed_internal(
            source,
            entries,
            draws,
            [width, height],
            clears,
            sampled,
            pass_state,
            bindings,
            retained_depth,
            color_loads,
            multi,
            queued,
            uniforms,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_targets_typed_internal<'a>(
        &self,
        source: &str,
        entries: [&str; 2],
        draws: &[TriangleDraw<'_>],
        [width, height]: [usize; 2],
        clears: &[[u8; 4]],
        sampled: Option<(
            &crate::texture_pipeline::MetalRgba8Texture,
            &crate::sampler::MetalSampler,
        )>,
        pass_state: Option<(&[TriangleState], Option<f32>)>,
        bindings: Option<&[crate::BoundSampledTexture]>,
        retained_depth: Option<(&'a mut MetalDepthTarget, DepthLoad)>,
        color_loads: Option<&[Option<&[u8]>]>,
        multi: Option<MultiPipelineInputs<'_>>,
        queued: Option<cohort::SubmissionSync>,
        uniforms: Option<&[DrawUniforms]>,
        typed_colors: Option<&[TypedColorAttachment]>,
    ) -> Result<PendingRender<'a>, MetalError> {
        validate_uniforms(draws.len(), uniforms)?;
        let legacy_colors: Vec<_> = clears
            .iter()
            .map(|clear| TypedColorAttachment {
                format: xenora_texture::TextureFormat::Rgba8Unorm,
                clear: TypedColorClear::Float(clear.map(|v| f64::from(v) / 255.0)),
                blend_enabled: true,
            })
            .collect();
        let color_descs = typed_colors.unwrap_or(&legacy_colors);
        if color_descs.len() != clears.len() {
            return Err(MetalError::new(
                "typed targets",
                "attachment count mismatch",
            ));
        }
        for color in color_descs {
            color.validate()?;
        }

        let gathered_geometry = queued
            .as_ref()
            .map(|q| q.geometry.clone())
            .unwrap_or_default();
        let gathered = |index: usize| gathered_geometry.get(index).and_then(Option::as_ref);
        let vertex_count =
            |index: usize| gathered(index).map_or(draws[index].vertices.len(), |g| g.count());
        let depth_source = retained_depth
            .as_ref()
            .map(|(d, l)| (&**d, *l))
            .or_else(|| {
                queued
                    .as_ref()
                    .and_then(|q| q.depth.as_ref().map(|(d, l)| (d, *l)))
            });
        if let Some((shaders, bindings)) = multi
            && (shaders.len() != draws.len() || bindings.len() != draws.len())
        {
            return Err(MetalError::new(
                "render shaders",
                "expected one shader and binding collection per draw",
            ));
        }
        if !(1..=8).contains(&clears.len()) {
            return Err(MetalError::new(
                "render targets",
                "expected 1..=8 color attachments",
            ));
        }
        if (draws.is_empty() && depth_source.is_none() && color_loads.is_none())
            || draws.len() > MAX_RENDER_DRAWS
        {
            return Err(MetalError::new("render input", "expected 1..=256 draws"));
        }
        if bindings.is_some_and(|bindings| {
            bindings.len() != draws.len()
                || bindings
                    .iter()
                    .any(|b| b.texture_slot().index != 0 || b.sampler_slot().index != 0)
        }) {
            return Err(MetalError::new(
                "render bindings",
                "expected one slot-zero texture/sampler binding per draw",
            ));
        }
        let normalized_legacy = bindings.map(|pairs| {
            pairs
                .iter()
                .map(NativeSampledBinding::from_whole)
                .collect::<Vec<_>>()
        });
        let sampled_texture = sampled.map(|(texture, _)| NativeSampledTexture::from_rgba(texture));
        let mut draw_bindings = Vec::with_capacity(draws.len());
        if let Some((texture, sampler)) = sampled {
            crate::BoundSampledTexture::bind(
                texture.clone(),
                sampler.clone(),
                crate::BindingSlot::new(0),
                crate::BindingSlot::new(0),
            )?
            .validate_for_device(&self.device)?;
        }
        for index in 0..draws.len() {
            let table = if let Some((_, bindings)) = multi {
                DrawBindings::normalize(&bindings[index], &self.device)?
            } else if let Some(bindings) = &normalized_legacy {
                DrawBindings::normalize(std::slice::from_ref(&bindings[index]), &self.device)?
            } else {
                let mut table = DrawBindings::default();
                if let Some((_, sampler)) = sampled {
                    table.textures[0] = sampled_texture.as_ref();
                    table.samplers[0] = Some(sampler);
                }
                table
            };
            draw_bindings.push(table);
        }
        let defaults = vec![TriangleState::default(); draws.len()];
        let (states, mut clear_depth) = pass_state.unwrap_or((&defaults, None));
        if let Some((depth, load)) = &depth_source {
            if depth.extent != [width, height]
                || depth.device_id != self.device.registryID()
                || (*load == DepthLoad::Load && !depth.initialized)
            {
                return Err(MetalError::new(
                    "depth target",
                    "extent/device mismatch or uninitialized load",
                ));
            }
            clear_depth = match load {
                DepthLoad::Clear(value) => Some(*value),
                DepthLoad::Load => None,
            };
        }
        if let Some(value) = queued.as_ref().and_then(|q| q.depth_clear) {
            clear_depth = Some(value);
        }
        let has_depth = depth_source.is_some() || clear_depth.is_some();
        if states.len() != draws.len()
            || clear_depth.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            || (!has_depth && states.iter().any(|s| s.depth != DepthState::DISABLED))
        {
            return Err(MetalError::new(
                "render input",
                "invalid per-draw or depth attachment state",
            ));
        }
        let mut total_vertices = 0usize;
        for (index, (draw, state)) in draws.iter().zip(states).enumerate() {
            if vertex_count(index) == 0
                || vertex_count(index) > MAX_RENDER_VERTICES
                || !vertex_count(index).is_multiple_of(3)
                || (gathered(index).is_none() && draw.colors.len() != draw.vertices.len())
            {
                return Err(MetalError::new(
                    "render input",
                    "expected complete triangles and matching colors",
                ));
            }
            total_vertices = total_vertices
                .checked_add(vertex_count(index))
                .filter(|&count| count <= MAX_RENDER_VERTICES)
                .ok_or_else(|| {
                    MetalError::new("render input", "expected at most 65535 vertices per pass")
                })?;
            if draw.viewport[0] == 0
                || draw.viewport[1] == 0
                || state.viewport_origin[0]
                    .checked_add(draw.viewport[0])
                    .is_none_or(|right| right > width)
                || state.viewport_origin[1]
                    .checked_add(draw.viewport[1])
                    .is_none_or(|bottom| bottom > height)
                || state.scissor.is_some_and(|[x, y, w, h]| {
                    x.checked_add(w).is_none_or(|right| right > width)
                        || y.checked_add(h).is_none_or(|bottom| bottom > height)
                })
            {
                return Err(MetalError::new(
                    "render bounds",
                    "viewport and scissor must fit the target",
                ));
            }
            if draw
                .vertices
                .iter()
                .chain(draw.colors)
                .flatten()
                .any(|v| !v.is_finite())
            {
                return Err(MetalError::new(
                    "render input",
                    "expected finite vertex data",
                ));
            }
        }
        let pixels = translated_pixels([width, height], "render bounds")?;
        if color_loads.is_some_and(|loads| {
            loads.len() != clears.len()
                || loads.iter().zip(color_descs).any(|(load, color)| {
                    load.is_some_and(|bytes| bytes.len() != pixels * color.format.block_bytes())
                })
        }) {
            return Err(MetalError::new(
                "color loads",
                "expected one load per attachment with exact typed packed bytes",
            ));
        }
        if let Some((shaders, _)) = multi {
            for shader in shaders {
                validate_source(shader.source, &shader.entries)?;
            }
        } else {
            validate_source(source, &entries)?;
        }

        autoreleasepool(|_| {
            let depth_source = retained_depth
                .as_ref()
                .map(|(d, l)| (&**d, *l))
                .or_else(|| {
                    queued
                        .as_ref()
                        .and_then(|q| q.depth.as_ref().map(|(d, l)| (d, *l)))
                });
            let options = compile_options()?;
            let fallback = TriangleShader { source, entries };
            let selected = multi.map_or(std::slice::from_ref(&fallback), |(shaders, _)| shaders);
            let mut shader_keys = Vec::new();
            let mut functions = Vec::new();
            let mut shader_indices = Vec::new();
            for shader in selected {
                let index = if let Some(index) = shader_keys.iter().position(|key| key == shader) {
                    index
                } else {
                    let library = self.compile_library(shader.source, &options)?;
                    let vertex = find_function(&library, shader.entries[0])?;
                    let fragment = find_function(&library, shader.entries[1])?;
                    shader_keys.push(*shader);
                    functions.push((vertex, fragment));
                    functions.len() - 1
                };
                shader_indices.push(index);
            }
            // Resolve all pipeline/depth state before starting the encoder.
            let mut pipelines = Vec::new();
            let mut pipeline_keys = Vec::new();
            let mut draw_pipelines = Vec::new();
            for (draw_index, state) in states.iter().enumerate() {
                let shader_index = shader_indices[if multi.is_some() { draw_index } else { 0 }];
                let (vertex, fragment) = &functions[shader_index];
                let key = (shader_index, state.blend, state.write_mask);
                let index = if let Some(index) = pipeline_keys.iter().position(|k| *k == key) {
                    index
                } else {
                    let descriptor = MTLRenderPipelineDescriptor::new();
                    descriptor.setVertexFunction(Some(vertex));
                    descriptor.setFragmentFunction(Some(fragment));
                    descriptor.setDepthAttachmentPixelFormat(if has_depth {
                        DEPTH_FORMAT
                    } else {
                        MTLPixelFormat::Invalid
                    });
                    for (slot, color) in color_descs.iter().enumerate() {
                        // SAFETY: Attachment count is validated to 1..=8.
                        let attachment =
                            unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(slot) };
                        attachment.setPixelFormat(color.pixel_format()?);
                        attachment.setWriteMask(state.write_mask.metal());
                        let blend = state.blend.filter(|_| color.blend_enabled);
                        attachment.setBlendingEnabled(blend.is_some());
                        if let Some(blend) = blend {
                            attachment.setSourceRGBBlendFactor(blend.source_rgb.metal());
                            attachment.setDestinationRGBBlendFactor(blend.destination_rgb.metal());
                            attachment.setRgbBlendOperation(blend.rgb_operation.metal());
                            attachment.setSourceAlphaBlendFactor(blend.source_alpha.metal());
                            attachment
                                .setDestinationAlphaBlendFactor(blend.destination_alpha.metal());
                            attachment.setAlphaBlendOperation(blend.alpha_operation.metal());
                        }
                    }
                    let pipeline = self
                        .device
                        .newRenderPipelineStateWithDescriptor_error(&descriptor)
                        .map_err(|error| {
                            MetalError::new(
                                "newRenderPipelineStateWithDescriptor",
                                error.localizedDescription().to_string(),
                            )
                        })?;
                    verify_render_validation(&pipeline)?;
                    pipeline_keys.push(key);
                    pipelines.push(pipeline);
                    pipelines.len() - 1
                };
                draw_pipelines.push(index);
            }
            let depth_states = states
                .iter()
                .map(|state| has_depth.then(|| self.depth_state(state.depth)).transpose())
                .collect::<Result<Vec<_>, _>>()?;

            let pass = MTLRenderPassDescriptor::renderPassDescriptor();
            let mut textures = Vec::with_capacity(color_descs.len());
            for (slot, color) in color_descs.iter().enumerate() {
                // SAFETY: Bounded extent and explicitly validated color format.
                let texture_descriptor = unsafe {
                    MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                        color.pixel_format()?,
                        width,
                        height,
                        false,
                    )
                };
                texture_descriptor.setStorageMode(MTLStorageMode::Shared);
                texture_descriptor
                    .setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
                let texture = self
                    .device
                    .newTextureWithDescriptor(&texture_descriptor)
                    .ok_or_else(|| {
                        MetalError::new(
                            "newTextureWithDescriptor",
                            "render target allocation failed",
                        )
                    })?;
                // SAFETY: The attachment count is validated to 1..=8.
                let attachment = unsafe { pass.colorAttachments().objectAtIndexedSubscript(slot) };
                attachment.setTexture(Some(&texture));
                let loaded = color_loads.and_then(|loads| loads[slot]);
                if let Some(bytes) = loaded {
                    // SAFETY: The exact typed packed length and bounded extent
                    // were validated before compilation. This fresh shared texture
                    // has no GPU users; replaceRegion copies the borrowed bytes.
                    unsafe {
                        texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                            whole(width, height),
                            0,
                            NonNull::from(bytes).cast(),
                            if height == 1 {
                                0
                            } else {
                                width * color.format.block_bytes()
                            },
                        );
                    }
                }
                attachment.setLoadAction(
                    if loaded.is_some()
                        || queued
                            .as_ref()
                            .is_some_and(|q| q.color_sources[slot].is_some())
                    {
                        MTLLoadAction::Load
                    } else {
                        MTLLoadAction::Clear
                    },
                );
                let [red, green, blue, alpha] = color.clear.components();
                attachment.setClearColor(MTLClearColor {
                    red,
                    green,
                    blue,
                    alpha,
                });
                attachment.setStoreAction(MTLStoreAction::Store);
                textures.push(texture);
            }

            // Render into a fresh surface so even a failed GPU command cannot
            // corrupt the previously committed retained depth contents.
            let working_depth = if has_depth {
                let texture = self.new_target(
                    DEPTH_FORMAT,
                    width,
                    height,
                    MTLStorageMode::Private,
                    MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
                )?;
                let attachment = pass.depthAttachment();
                attachment.setTexture(Some(&texture));
                attachment.setLoadAction(if clear_depth.is_some() {
                    MTLLoadAction::Clear
                } else {
                    MTLLoadAction::Load
                });
                if let Some(clear) = clear_depth {
                    attachment.setClearDepth(f64::from(clear));
                }
                attachment.setStoreAction(
                    if depth_source.is_some() || queued.as_ref().is_some_and(|q| q.export_depth) {
                        MTLStoreAction::Store
                    } else {
                        MTLStoreAction::DontCare
                    },
                );
                Some(texture)
            } else {
                None
            };

            let uniform_buffers = (0..draws.len())
                .map(|i| {
                    uniforms
                        .and_then(|u| u[i].values.as_ref())
                        .map(|v| self.new_buffer(v))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let position_words: Vec<_> = draws
                .iter()
                .flat_map(|d| d.vertices)
                .flatten()
                .map(|v| v.to_bits())
                .collect();
            let color_words: Vec<_> = draws
                .iter()
                .flat_map(|d| d.colors)
                .flatten()
                .map(|v| v.to_bits())
                .collect();
            let positions = self.new_buffer(if position_words.is_empty() {
                &[0; 4]
            } else {
                &position_words
            })?;
            let colors = self.new_buffer(if color_words.is_empty() {
                &[0; 4]
            } else {
                &color_words
            })?;
            let command = self.queue.commandBuffer().ok_or_else(|| {
                MetalError::new("commandBuffer", "render command buffer creation failed")
            })?;
            if let Some(sync) = &queued {
                sync.encode_waits(&command);
                for gather in sync.geometry.iter().flatten() {
                    gather.encode(&command)?;
                }
                sync.encode_storage_load(&command)?;
                sync.encode_color_loads(&command, &textures)?;
                sync.encode_depth_upload(&command)?;
            }
            if let Some((depth, DepthLoad::Load)) = &depth_source {
                let blit = command.blitCommandEncoder().ok_or_else(|| {
                    MetalError::new("blitCommandEncoder", "depth copy encoder creation failed")
                })?;
                // SAFETY: Both retained textures are single-layer/mip Depth32Float,
                // have the validated same extent/device, and remain alive until the
                // command completes. Encoders on this queue are ordered.
                unsafe {
                    blit.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                        &depth.texture, 0, 0, MTLOrigin { x: 0, y: 0, z: 0 },
                        MTLSize { width, height, depth: 1 }, working_depth.as_ref().expect("depth attachment"),
                        0, 0, MTLOrigin { x: 0, y: 0, z: 0 });
                }
                blit.endEncoding();
            }
            let encoder = command
                .renderCommandEncoderWithDescriptor(&pass)
                .ok_or_else(|| {
                    MetalError::new(
                        "renderCommandEncoderWithDescriptor",
                        "render encoder creation failed",
                    )
                })?;
            let mut offset = 0;
            for (index, draw) in draws.iter().enumerate() {
                let draw_offset = offset;
                offset += draw.vertices.len() * 16;
                let [x, y, scissor_width, scissor_height] =
                    states[index].scissor.unwrap_or([0, 0, width, height]);
                if scissor_width == 0 || scissor_height == 0 {
                    continue;
                }
                encoder.setScissorRect(MTLScissorRect {
                    x,
                    y,
                    width: scissor_width,
                    height: scissor_height,
                });
                encoder.setRenderPipelineState(&pipelines[draw_pipelines[index]]);
                if let Some(state) = &depth_states[index] {
                    encoder.setDepthStencilState(Some(state));
                }
                encoder.setViewport(MTLViewport {
                    originX: states[index].viewport_origin[0] as f64,
                    originY: states[index].viewport_origin[1] as f64,
                    width: draw.viewport[0] as f64,
                    height: draw.viewport[1] as f64,
                    znear: 0.0,
                    zfar: 1.0,
                });
                // SAFETY: Validated draws occupy consecutive float4 ranges in
                // these device buffers. Vertex IDs restart at zero within each
                // view; trusted shaders read only the current draw's records.
                unsafe {
                    if let Some(gather) = gathered(index) {
                        encoder.setVertexBuffer_offset_atIndex(Some(&gather.positions), 0, 0);
                        encoder.setVertexBuffer_offset_atIndex(Some(&gather.colors), 0, 1);
                    } else {
                        encoder.setVertexBuffer_offset_atIndex(Some(&positions), draw_offset, 0);
                        encoder.setVertexBuffer_offset_atIndex(Some(&colors), draw_offset, 1);
                    }
                    encoder.setVertexBuffer_offset_atIndex(uniform_buffers[index].as_deref(), 0, 2);
                    encoder.setFragmentBuffer_offset_atIndex(
                        uniform_buffers[index].as_deref(),
                        0,
                        2,
                    );
                    for slot in 0..4 {
                        encoder.setFragmentTexture_atIndex(
                            draw_bindings[index].textures[slot].map(|texture| texture.raw()),
                            slot,
                        );
                        encoder.setFragmentSamplerState_atIndex(
                            draw_bindings[index].samplers[slot].map(|sampler| sampler.raw()),
                            slot,
                        );
                    }
                    // Sampled-slot normalization above clears unused slots. Storage
                    // owns slot0 and must be installed after that clearing.
                    if let Some(storage) = queued.as_ref().and_then(|q| q.storage.as_ref()) {
                        encoder.setFragmentTexture_atIndex(Some(&storage.texture), 0);
                    }
                    encoder.drawPrimitives_vertexStart_vertexCount(
                        MTLPrimitiveType::Triangle,
                        0,
                        vertex_count(index),
                    );
                }
            }
            encoder.endEncoding();
            // Ordinary commandBuffer() retains its encoded resources (Apple's
            // retainedReferences contract). Keep explicit owners as well so no
            // caller-owned texture, sampler or buffer lifetime escapes submission.
            if let Some(sync) = &queued {
                sync.encode_signal(&command);
            }
            let pending = PendingRender {
                storage: queued.as_ref().and_then(|q| q.storage.clone()),
                command,
                textures,
                color_formats: color_descs.iter().map(|c| c.format).collect(),
                working_depth,
                retained_depth: retained_depth.map(|(depth, _)| depth),
                extent: [width, height],
                _buffers: [positions, colors],
                _geometry: queued
                    .as_ref()
                    .map(|q| q.geometry.clone())
                    .unwrap_or_default(),
                validation: queued.as_ref().and_then(|q| q.validation.clone()),
                _uniform_buffers: uniform_buffers,
                _pipelines: pipelines,
                _depth_states: depth_states,
                _bindings: if let Some((_, bindings)) = multi {
                    bindings.iter().flatten().cloned().collect()
                } else {
                    normalized_legacy.clone().unwrap_or_default()
                },
                _color_sources: queued
                    .as_ref()
                    .map(|q| q.color_sources.clone())
                    .unwrap_or_default(),
                _color_lease: queued.as_ref().map(|q| q.color_lease.clone()),
                _temporary_lease: queued.as_ref().map(|q| q.temporary_lease.clone()),
                _depth_upload: queued.as_ref().and_then(|q| q.upload.clone()),
                _export_depth: queued.as_ref().is_some_and(|q| q.export_depth),
                _depth_source_owner: queued.as_ref().and_then(|q| q.depth_source_owner.clone()),
                _depth_lease: queued.as_ref().and_then(|q| q.depth_lease.clone()),
                _owned_depth: queued.and_then(|q| q.depth.map(|(d, _)| d)),
                _sampled: sampled.map(|(texture, sampler)| (texture.clone(), sampler.clone())),
            };
            pending.command.commit();
            Ok(pending)
        })
    }
}

fn verify_render_validation(
    pipeline: &ProtocolObject<dyn MTLRenderPipelineState>,
) -> Result<(), MetalError> {
    if std::env::var("MTL_SHADER_VALIDATION").as_deref() != Ok("1") {
        return Ok(());
    }
    if !pipeline.respondsToSelector(sel!(shaderValidation)) {
        return Err(MetalError::new(
            "shaderValidation",
            "the runtime lacks the render pipeline state query",
        ));
    }
    // SAFETY: The selector exists and the newly created pipeline is private
    // to this thread. Its non-atomic property is read before encoding.
    if unsafe { pipeline.shaderValidation() } != MTLShaderValidation::Enabled {
        return Err(MetalError::new(
            "shaderValidation",
            "shader validation requested but inactive for the render pipeline",
        ));
    }
    Ok(())
}

const DEPTH_FORMAT: MTLPixelFormat = MTLPixelFormat::Depth32Float;
const MAX_DEPTH_DRAWS: usize = 16;

const DEPTH_SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;

vertex float4 depth_vertex(uint id [[vertex_id]],
                           const device float4 *positions [[buffer(0)]]) {
    return positions[id];
}

fragment float4 depth_flat(constant float4 &color [[buffer(0)]]) {
    return color;
}

fragment float4 depth_textured(float4 position [[position]],
                               texture2d<float> image [[texture(0)]],
                               constant float2 &extent [[buffer(0)]]) {
    constexpr sampler nearest(coord::normalized, filter::nearest, address::clamp_to_edge);
    return image.sample(nearest, position.xy / extent);
}
";

const DEPTH_READBACK: &str = r"
#include <metal_stdlib>
using namespace metal;
kernel void read_depth(depth2d<float, access::read> depth [[texture(0)]],
                       device uint *output [[buffer(1)]],
                       uint index [[thread_position_in_grid]]) {
    uint width = depth.get_width();
    output[index] = as_type<uint>(depth.read(uint2(index % width, index / width)));
}
";

/// Comparison of a fragment's depth against the stored depth; it passes when
/// `fragment <compare> stored` holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthCompare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

impl DepthCompare {
    fn metal(self) -> MTLCompareFunction {
        match self {
            Self::Never => MTLCompareFunction::Never,
            Self::Less => MTLCompareFunction::Less,
            Self::Equal => MTLCompareFunction::Equal,
            Self::LessEqual => MTLCompareFunction::LessEqual,
            Self::Greater => MTLCompareFunction::Greater,
            Self::NotEqual => MTLCompareFunction::NotEqual,
            Self::GreaterEqual => MTLCompareFunction::GreaterEqual,
            Self::Always => MTLCompareFunction::Always,
        }
    }
}

/// Depth test and write enable of one draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DepthState {
    pub compare: DepthCompare,
    pub write: bool,
}

impl DepthState {
    /// No depth test and no depth write: the only state allowed without a
    /// depth attachment.
    pub const DISABLED: Self = Self {
        compare: DepthCompare::Always,
        write: false,
    };
}

/// Colour source of one depth-pass draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DepthFill {
    /// A constant RGBA8 colour.
    Flat([u8; 4]),
    /// The pass's BC1 texture, sampled with nearest filtering at the pixel centre.
    Bc1Texture,
}

/// One triangle of a depth pass: three clip-space positions, colour and depth state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthDraw {
    pub positions: [[f32; 4]; 3],
    pub fill: DepthFill,
    pub depth: DepthState,
}

/// An offscreen pass of up to 16 triangles drawn in order into a cleared
/// RGBA8Unorm target with viewport depth range 0..1.
///
/// With `clear_depth`, the pass has a `Depth32Float` attachment cleared to that
/// value and stored after the pass. Without it, the pass has no depth
/// attachment and every draw must use [`DepthState::DISABLED`].
#[derive(Clone, Copy, Debug)]
pub struct DepthPass<'a> {
    pub extent: [usize; 2],
    pub clear_color: [u8; 4],
    pub clear_depth: Option<f32>,
    pub draws: &'a [DepthDraw],
    /// A validated BC1 upload whose base level of layer 0 is uploaded to a
    /// native `BC1_RGBA` texture as the original blocks, without CPU decoding.
    pub texture: Option<&'a Bc1Upload<'a>>,
}

/// Colour bytes, tightly packed top-to-bottom, and the stored depth values
/// when the pass has a depth attachment.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthOutput {
    pub color: Vec<u8>,
    pub depth: Option<Vec<f32>>,
}

impl NativeCompute {
    /// Draw a depth-tested offscreen pass and read back colour and depth.
    ///
    /// Each draw sets its own depth state before drawing, so depth compare,
    /// clear and write enable act exactly as the pass describes. The stored
    /// depth is read back by a compute kernel that returns each value as
    /// the texture read gives it, without numeric conversion.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid extent, too many draws, non-finite
    /// positions, a depth state without depth attachment, a non-finite clear
    /// depth or one outside 0..=1, a textured draw without texture, unavailable
    /// BC compression (operation `supportsBCTextureCompression`), or Metal
    /// setup or execution failure.
    pub fn render_depth(&self, pass: &DepthPass<'_>) -> Result<DepthOutput, MetalError> {
        let [width, height] = pass.extent;
        let pixels = width
            .checked_mul(height)
            .filter(|&count| (1..=MAX_PIXELS).contains(&count))
            .ok_or_else(|| MetalError::new("render bounds", "expected 1..=4096 pixels"))?;
        validate_depth_pass(pass)?;
        let bc1 = pass
            .texture
            .map(|upload| {
                let level = upload.subresource(0, 0).map_err(|error| {
                    MetalError::new("texture subresource", format!("{error:?}"))
                })?;
                self.validate_bc_extent(level.width, level.height, "BC1")?;
                self.new_bc1_texture(&level)
            })
            .transpose()?;

        autoreleasepool(|_| {
            let options = compile_options()?;
            let library = self.compile_library(DEPTH_SOURCE, &options)?;
            let vertex = find_function(&library, "depth_vertex")?;
            let depth_format = if pass.clear_depth.is_some() {
                DEPTH_FORMAT
            } else {
                MTLPixelFormat::Invalid
            };
            let flat = self.depth_pipeline(
                &vertex,
                &*find_function(&library, "depth_flat")?,
                depth_format,
            )?;
            let textured = self.depth_pipeline(
                &vertex,
                &*find_function(&library, "depth_textured")?,
                depth_format,
            )?;
            let color = self.new_target(
                PIXEL_FORMAT,
                width,
                height,
                MTLStorageMode::Shared,
                MTLTextureUsage::RenderTarget,
            )?;
            let pass_descriptor = MTLRenderPassDescriptor::renderPassDescriptor();
            // SAFETY: Slot zero is the only color attachment for this pass.
            let attachment = unsafe {
                pass_descriptor
                    .colorAttachments()
                    .objectAtIndexedSubscript(0)
            };
            attachment.setTexture(Some(&color));
            attachment.setLoadAction(MTLLoadAction::Clear);
            let [red, green, blue, alpha] = pass.clear_color.map(|value| f64::from(value) / 255.0);
            attachment.setClearColor(MTLClearColor {
                red,
                green,
                blue,
                alpha,
            });
            attachment.setStoreAction(MTLStoreAction::Store);
            let depth = pass
                .clear_depth
                .map(|clear| {
                    let texture = self.new_target(
                        DEPTH_FORMAT,
                        width,
                        height,
                        MTLStorageMode::Private,
                        MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead,
                    )?;
                    let depth_attachment = pass_descriptor.depthAttachment();
                    depth_attachment.setTexture(Some(&texture));
                    depth_attachment.setLoadAction(MTLLoadAction::Clear);
                    depth_attachment.setClearDepth(f64::from(clear));
                    depth_attachment.setStoreAction(MTLStoreAction::Store);
                    Ok::<_, MetalError>(texture)
                })
                .transpose()?;

            // A pass without draws still clears and stores both attachments.
            let words: Vec<u32> = pass
                .draws
                .iter()
                .flat_map(|draw| draw.positions.as_flattened())
                .map(|value| value.to_bits())
                .collect();
            let positions = (!words.is_empty())
                .then(|| self.new_buffer(&words))
                .transpose()?;
            // Everything fallible is resolved before encoding starts, so an
            // error never leaves an encoder without endEncoding.
            let states = pass
                .draws
                .iter()
                .map(|draw| {
                    depth
                        .as_ref()
                        .map(|_| self.depth_state(draw.depth))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let command = self.queue.commandBuffer().ok_or_else(|| {
                MetalError::new("commandBuffer", "render command buffer creation failed")
            })?;
            let encoder = command
                .renderCommandEncoderWithDescriptor(&pass_descriptor)
                .ok_or_else(|| {
                    MetalError::new(
                        "renderCommandEncoderWithDescriptor",
                        "render encoder creation failed",
                    )
                })?;
            encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: width as f64,
                height: height as f64,
                znear: 0.0,
                zfar: 1.0,
            });
            let extent = [width as f32, height as f32];
            for (index, (draw, state)) in pass.draws.iter().zip(&states).enumerate() {
                if let Some(state) = state {
                    encoder.setDepthStencilState(Some(state));
                }
                // SAFETY: The position buffer holds three float4s per draw and
                // the vertex stage reads only this draw's three. Fragment
                // constants are copied by Metal and match the declared types.
                // The texture, when bound, is a live BC1 texture of this device.
                unsafe {
                    encoder.setVertexBuffer_offset_atIndex(positions.as_deref(), index * 48, 0);
                    match draw.fill {
                        DepthFill::Flat(rgba) => {
                            encoder.setRenderPipelineState(&flat);
                            let rgba = rgba.map(|value| f32::from(value) / 255.0);
                            encoder.setFragmentBytes_length_atIndex(
                                NonNull::from(&rgba).cast(),
                                size_of_val(&rgba),
                                0,
                            );
                        }
                        DepthFill::Bc1Texture => {
                            encoder.setRenderPipelineState(&textured);
                            encoder.setFragmentTexture_atIndex(bc1.as_deref(), 0);
                            encoder.setFragmentBytes_length_atIndex(
                                NonNull::from(&extent).cast(),
                                size_of_val(&extent),
                                0,
                            );
                        }
                    }
                    encoder.drawPrimitives_vertexStart_vertexCount(
                        MTLPrimitiveType::Triangle,
                        0,
                        3,
                    );
                }
            }
            encoder.endEncoding();
            complete_command(&command)?;

            let mut output = vec![0; pixels * 4];
            // SAFETY: GPU writes have completed, and the initialized output
            // allocation fits the entire shared texture with tightly packed rows.
            unsafe {
                color.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    NonNull::from(output.as_mut_slice()).cast(),
                    width * 4,
                    whole(width, height),
                    0,
                );
            }
            let depth = depth
                .map(|texture| {
                    self.run(
                        DEPTH_READBACK,
                        "read_depth",
                        ComputeInput::Texture(&texture, pixels),
                    )
                    .map(|profile| profile.output.into_iter().map(f32::from_bits).collect())
                })
                .transpose()?;
            Ok(DepthOutput {
                color: output,
                depth,
            })
        })
    }

    fn depth_pipeline(
        &self,
        vertex: &ProtocolObject<dyn MTLFunction>,
        fragment: &ProtocolObject<dyn MTLFunction>,
        depth_format: MTLPixelFormat,
    ) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, MetalError> {
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(vertex));
        descriptor.setFragmentFunction(Some(fragment));
        // SAFETY: Slot zero is the only color attachment for this pipeline.
        unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) }
            .setPixelFormat(PIXEL_FORMAT);
        descriptor.setDepthAttachmentPixelFormat(depth_format);
        let pipeline = self
            .device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| {
                MetalError::new(
                    "newRenderPipelineStateWithDescriptor",
                    error.localizedDescription().to_string(),
                )
            })?;
        verify_render_validation(&pipeline)?;
        Ok(pipeline)
    }

    fn depth_state(
        &self,
        state: DepthState,
    ) -> Result<Retained<ProtocolObject<dyn MTLDepthStencilState>>, MetalError> {
        let descriptor = MTLDepthStencilDescriptor::new();
        descriptor.setDepthCompareFunction(state.compare.metal());
        descriptor.setDepthWriteEnabled(state.write);
        self.device
            .newDepthStencilStateWithDescriptor(&descriptor)
            .ok_or_else(|| {
                MetalError::new(
                    "newDepthStencilStateWithDescriptor",
                    "depth state creation failed",
                )
            })
    }

    fn new_target(
        &self,
        format: MTLPixelFormat,
        width: usize,
        height: usize,
        storage: MTLStorageMode,
        usage: MTLTextureUsage,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
        // SAFETY: Callers validate bounded nonzero dimensions and a supported
        // pixel format for the requested render or shader-write usage.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                format, width, height, false,
            )
        };
        descriptor.setStorageMode(storage);
        descriptor.setUsage(usage);
        self.device
            .newTextureWithDescriptor(&descriptor)
            .ok_or_else(|| {
                MetalError::new(
                    "newTextureWithDescriptor",
                    "render target allocation failed",
                )
            })
    }

    fn new_bc1_texture(
        &self,
        level: &Bc1Subresource<'_>,
    ) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, MetalError> {
        let texture = self.new_target(
            MTLPixelFormat::BC1_RGBA,
            level.width,
            level.height,
            MTLStorageMode::Shared,
            MTLTextureUsage::ShaderRead,
        )?;
        // Apple documents a zero stride for a single compressed block row.
        let bytes_per_row = if level.height <= 4 {
            0
        } else {
            level.row_pitch
        };
        // SAFETY: The region spans the whole level. The level comes only from
        // `Bc1Upload::subresource`, whose layout validation guarantees that the
        // bytes hold every block row, including partial edge blocks, at this
        // block-aligned stride.
        unsafe {
            texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                whole(level.width, level.height),
                0,
                NonNull::from(level.bytes).cast(),
                bytes_per_row,
            );
        }
        Ok(texture)
    }
}

const MAX_INDEXED_VERTICES: usize = 1024;
const MAX_INDICES: usize = 4096;

/// An index buffer of 16- or 32-bit unsigned indices.
#[derive(Clone, Copy, Debug)]
pub enum IndexBuffer<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl IndexBuffer<'_> {
    fn len(self) -> usize {
        match self {
            Self::U16(indices) => indices.len(),
            Self::U32(indices) => indices.len(),
        }
    }

    fn index(self, position: usize) -> u32 {
        match self {
            Self::U16(indices) => u32::from(indices[position]),
            Self::U32(indices) => indices[position],
        }
    }

    fn restart(self) -> u32 {
        match self {
            Self::U16(_) => u32::from(u16::MAX),
            Self::U32(_) => u32::MAX,
        }
    }

    fn size(self) -> usize {
        match self {
            Self::U16(_) => 2,
            Self::U32(_) => 4,
        }
    }

    /// The indices packed into little-endian words, the last one zero-padded.
    fn words(self) -> Vec<u32> {
        match self {
            Self::U16(indices) => indices
                .chunks(2)
                .map(|pair| {
                    u32::from(pair[0]) | pair.get(1).map_or(0, |&high| u32::from(high) << 16)
                })
                .collect(),
            Self::U32(indices) => indices.to_vec(),
        }
    }
}

/// One indexed triangle-list draw: `index_count` indices from `start_index`,
/// each fetching vertex `index + base_vertex`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IndexedDraw {
    pub start_index: usize,
    pub index_count: usize,
    pub base_vertex: i32,
    pub fill: DepthFill,
}

/// An offscreen pass of up to 16 indexed draws, in order, into a cleared
/// RGBA8Unorm target without depth attachment.
#[derive(Clone, Copy, Debug)]
pub struct IndexedPass<'a> {
    pub extent: [usize; 2],
    pub clear_color: [u8; 4],
    /// Up to 1024 clip-space float4 positions.
    pub vertices: &'a [[f32; 4]],
    /// Up to 4096 indices shared by every draw.
    pub indices: IndexBuffer<'a>,
    pub draws: &'a [IndexedDraw],
    /// As [`DepthPass::texture`], for [`DepthFill::Bc1Texture`] draws.
    pub texture: Option<&'a Bc1Upload<'a>>,
}

impl NativeCompute {
    /// Draw an indexed offscreen pass and read back its colour.
    ///
    /// Each draw is one native `drawIndexedPrimitives` call on the pass's
    /// index buffer with the draw's start offset and signed base vertex, so
    /// Metal itself fetches the vertices. Every fetched vertex is checked
    /// against the vertex buffer first: out-of-range fetches and raw
    /// primitive-restart values are rejected, never left to the GPU. Base vertex needs no capability query: the
    /// Apple family 7 that [`NativeCompute::new`] requires supports it.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid extent, too many draws, vertices or
    /// indices, non-finite positions, an index count that is zero or not a
    /// multiple of 3, an index range past the buffer, a primitive-restart
    /// index, a fetched vertex outside the vertex buffer, a textured draw without texture, unavailable BC
    /// compression (operation `supportsBCTextureCompression`), or Metal setup
    /// or execution failure.
    pub fn render_indexed(&self, pass: &IndexedPass<'_>) -> Result<Vec<u8>, MetalError> {
        let [width, height] = pass.extent;
        let pixels = width
            .checked_mul(height)
            .filter(|&count| (1..=MAX_PIXELS).contains(&count))
            .ok_or_else(|| MetalError::new("render bounds", "expected 1..=4096 pixels"))?;
        validate_indexed_pass(pass)?;
        let bc1 = pass
            .texture
            .map(|upload| {
                let level = upload.subresource(0, 0).map_err(|error| {
                    MetalError::new("texture subresource", format!("{error:?}"))
                })?;
                self.validate_bc_extent(level.width, level.height, "BC1")?;
                self.new_bc1_texture(&level)
            })
            .transpose()?;

        autoreleasepool(|_| {
            let options = compile_options()?;
            let library = self.compile_library(DEPTH_SOURCE, &options)?;
            let vertex = find_function(&library, "depth_vertex")?;
            let flat = self.depth_pipeline(
                &vertex,
                &*find_function(&library, "depth_flat")?,
                MTLPixelFormat::Invalid,
            )?;
            let textured = self.depth_pipeline(
                &vertex,
                &*find_function(&library, "depth_textured")?,
                MTLPixelFormat::Invalid,
            )?;
            let color = self.new_target(
                PIXEL_FORMAT,
                width,
                height,
                MTLStorageMode::Shared,
                MTLTextureUsage::RenderTarget,
            )?;
            let pass_descriptor = MTLRenderPassDescriptor::renderPassDescriptor();
            // SAFETY: Slot zero is the only color attachment for this pass.
            let attachment = unsafe {
                pass_descriptor
                    .colorAttachments()
                    .objectAtIndexedSubscript(0)
            };
            attachment.setTexture(Some(&color));
            attachment.setLoadAction(MTLLoadAction::Clear);
            let [red, green, blue, alpha] = pass.clear_color.map(|value| f64::from(value) / 255.0);
            attachment.setClearColor(MTLClearColor {
                red,
                green,
                blue,
                alpha,
            });
            attachment.setStoreAction(MTLStoreAction::Store);
            let positions: Vec<u32> = pass
                .vertices
                .as_flattened()
                .iter()
                .map(|value| value.to_bits())
                .collect();
            let positions = self.new_buffer(&positions)?;
            let indices = self.new_buffer(&pass.indices.words())?;
            let index_type = match pass.indices {
                IndexBuffer::U16(_) => MTLIndexType::UInt16,
                IndexBuffer::U32(_) => MTLIndexType::UInt32,
            };
            let command = self.queue.commandBuffer().ok_or_else(|| {
                MetalError::new("commandBuffer", "render command buffer creation failed")
            })?;
            let encoder = command
                .renderCommandEncoderWithDescriptor(&pass_descriptor)
                .ok_or_else(|| {
                    MetalError::new(
                        "renderCommandEncoderWithDescriptor",
                        "render encoder creation failed",
                    )
                })?;
            encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: width as f64,
                height: height as f64,
                znear: 0.0,
                zfar: 1.0,
            });
            let extent = [width as f32, height as f32];
            for draw in pass.draws {
                // SAFETY: Validation proved that every index the draw reads lies
                // inside the index buffer, that the offset is a multiple of the
                // index size, and that every `index + base_vertex` names a float4
                // of the position buffer, the only memory the vertex stage reads.
                // Fragment constants are copied by Metal and match the declared
                // types. The texture, when bound, is a live BC1 texture of this
                // device; both buffers outlive the command.
                unsafe {
                    encoder.setVertexBuffer_offset_atIndex(Some(&positions), 0, 0);
                    match draw.fill {
                        DepthFill::Flat(rgba) => {
                            encoder.setRenderPipelineState(&flat);
                            let rgba = rgba.map(|value| f32::from(value) / 255.0);
                            encoder.setFragmentBytes_length_atIndex(
                                NonNull::from(&rgba).cast(),
                                size_of_val(&rgba),
                                0,
                            );
                        }
                        DepthFill::Bc1Texture => {
                            encoder.setRenderPipelineState(&textured);
                            encoder.setFragmentTexture_atIndex(bc1.as_deref(), 0);
                            encoder.setFragmentBytes_length_atIndex(
                                NonNull::from(&extent).cast(),
                                size_of_val(&extent),
                                0,
                            );
                        }
                    }
                    encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(
                        MTLPrimitiveType::Triangle,
                        draw.index_count,
                        index_type,
                        &indices,
                        draw.start_index * pass.indices.size(),
                        1,
                        draw.base_vertex as isize,
                        0,
                    );
                }
            }
            encoder.endEncoding();
            complete_command(&command)?;

            let mut output = vec![0; pixels * 4];
            // SAFETY: GPU writes have completed, and the initialized output
            // allocation fits the entire shared texture with tightly packed rows.
            unsafe {
                color.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    NonNull::from(output.as_mut_slice()).cast(),
                    width * 4,
                    whole(width, height),
                    0,
                );
            }
            Ok(output)
        })
    }
}

fn validate_indexed_pass(pass: &IndexedPass<'_>) -> Result<(), MetalError> {
    let invalid = |reason: String| Err(MetalError::new("indexed pass", reason));
    if pass.draws.len() > MAX_DEPTH_DRAWS {
        return invalid("expected at most 16 draws".to_owned());
    }
    if !(1..=MAX_INDEXED_VERTICES).contains(&pass.vertices.len())
        || !(1..=MAX_INDICES).contains(&pass.indices.len())
    {
        return invalid("expected 1..=1024 vertices and 1..=4096 indices".to_owned());
    }
    if pass
        .vertices
        .as_flattened()
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(MetalError::new(
            "render input",
            "expected finite vertex data",
        ));
    }
    for (number, draw) in pass.draws.iter().enumerate() {
        if draw.index_count == 0 || !draw.index_count.is_multiple_of(3) {
            return invalid(format!(
                "draw {number}: index count must be a nonzero multiple of 3"
            ));
        }
        let end = draw
            .start_index
            .checked_add(draw.index_count)
            .filter(|&end| end <= pass.indices.len())
            .ok_or_else(|| {
                MetalError::new(
                    "indexed pass",
                    format!("draw {number}: index range past the index buffer"),
                )
            })?;
        for position in draw.start_index..end {
            let index = pass.indices.index(position);
            // Metal documents 0xFFFF / 0xFFFFFFFF as the primitive-restart value
            // of indexed draws without defining it for triangle lists, so the
            // raw value is rejected whatever the base vertex.
            if index == pass.indices.restart() {
                return invalid(format!(
                    "draw {number}: index {position} is the primitive-restart value"
                ));
            }
            let vertex = i64::from(index) + i64::from(draw.base_vertex);
            if !(0..pass.vertices.len() as i64).contains(&vertex) {
                return invalid(format!(
                    "draw {number}: index {position} fetches vertex {vertex} outside the vertex buffer"
                ));
            }
        }
        if draw.fill == DepthFill::Bc1Texture && pass.texture.is_none() {
            return invalid("textured draw without texture".to_owned());
        }
    }
    Ok(())
}

fn validate_depth_pass(pass: &DepthPass<'_>) -> Result<(), MetalError> {
    if pass.draws.len() > MAX_DEPTH_DRAWS {
        return Err(MetalError::new("depth pass", "expected at most 16 draws"));
    }
    if pass
        .draws
        .iter()
        .flat_map(|draw| draw.positions.as_flattened())
        .any(|value| !value.is_finite())
    {
        return Err(MetalError::new(
            "render input",
            "expected finite vertex data",
        ));
    }
    if let Some(clear) = pass.clear_depth
        && !(0.0..=1.0).contains(&clear)
    {
        return Err(MetalError::new(
            "depth pass",
            "expected a clear depth in 0..=1",
        ));
    }
    if pass.clear_depth.is_none()
        && pass
            .draws
            .iter()
            .any(|draw| draw.depth != DepthState::DISABLED)
    {
        return Err(MetalError::new(
            "depth pass",
            "a depth test or write needs a depth attachment",
        ));
    }
    if pass.texture.is_none()
        && pass
            .draws
            .iter()
            .any(|draw| draw.fill == DepthFill::Bc1Texture)
    {
        return Err(MetalError::new(
            "depth pass",
            "textured draw without texture",
        ));
    }
    Ok(())
}

fn whole(width: usize, height: usize) -> MTLRegion {
    MTLRegion {
        origin: MTLOrigin { x: 0, y: 0, z: 0 },
        size: MTLSize {
            width,
            height,
            depth: 1,
        },
    }
}

/// A blend factor, as Metal defines it for the RGB and the alpha channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendFactor {
    Zero,
    One,
    SourceColor,
    OneMinusSourceColor,
    SourceAlpha,
    OneMinusSourceAlpha,
    DestinationColor,
    OneMinusDestinationColor,
    DestinationAlpha,
    OneMinusDestinationAlpha,
}

impl BlendFactor {
    fn metal(self) -> MTLBlendFactor {
        match self {
            Self::Zero => MTLBlendFactor::Zero,
            Self::One => MTLBlendFactor::One,
            Self::SourceColor => MTLBlendFactor::SourceColor,
            Self::OneMinusSourceColor => MTLBlendFactor::OneMinusSourceColor,
            Self::SourceAlpha => MTLBlendFactor::SourceAlpha,
            Self::OneMinusSourceAlpha => MTLBlendFactor::OneMinusSourceAlpha,
            Self::DestinationColor => MTLBlendFactor::DestinationColor,
            Self::OneMinusDestinationColor => MTLBlendFactor::OneMinusDestinationColor,
            Self::DestinationAlpha => MTLBlendFactor::DestinationAlpha,
            Self::OneMinusDestinationAlpha => MTLBlendFactor::OneMinusDestinationAlpha,
        }
    }
}

/// A blend operation combining the weighted source and destination; `Min`
/// and `Max` take the unweighted values and ignore the factors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

impl BlendOperation {
    fn metal(self) -> MTLBlendOperation {
        match self {
            Self::Add => MTLBlendOperation::Add,
            Self::Subtract => MTLBlendOperation::Subtract,
            Self::ReverseSubtract => MTLBlendOperation::ReverseSubtract,
            Self::Min => MTLBlendOperation::Min,
            Self::Max => MTLBlendOperation::Max,
        }
    }
}

/// Separate RGB and alpha blend equations of one draw.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlendState {
    pub source_rgb: BlendFactor,
    pub destination_rgb: BlendFactor,
    pub rgb_operation: BlendOperation,
    pub source_alpha: BlendFactor,
    pub destination_alpha: BlendFactor,
    pub alpha_operation: BlendOperation,
}

/// The colour channels a draw writes; the others keep the stored value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ColorWriteMask {
    pub red: bool,
    pub green: bool,
    pub blue: bool,
    pub alpha: bool,
}

impl ColorWriteMask {
    pub const ALL: Self = Self {
        red: true,
        green: true,
        blue: true,
        alpha: true,
    };

    fn metal(self) -> MTLColorWriteMask {
        [
            (self.red, MTLColorWriteMask::Red),
            (self.green, MTLColorWriteMask::Green),
            (self.blue, MTLColorWriteMask::Blue),
            (self.alpha, MTLColorWriteMask::Alpha),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .fold(MTLColorWriteMask::None, |mask, (_, bit)| mask | bit)
    }
}

/// One flat-coloured triangle of a blend pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendDraw {
    pub positions: [[f32; 4]; 3],
    pub color: [u8; 4],
    /// `None` disables blending: the source replaces the masked channels.
    pub blend: Option<BlendState>,
    pub write_mask: ColorWriteMask,
}

/// An offscreen pass of up to 16 triangles drawn in order into a cleared
/// RGBA8Unorm target without depth attachment.
#[derive(Clone, Copy, Debug)]
pub struct BlendPass<'a> {
    pub extent: [usize; 2],
    pub clear_color: [u8; 4],
    pub draws: &'a [BlendDraw],
}

impl NativeCompute {
    /// Draw a blended, write-masked offscreen pass and read back its colour.
    ///
    /// Blend state and write mask are pipeline state in Metal, so each draw
    /// gets its own pipeline built from exactly the draw's settings.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid extent, too many draws, non-finite
    /// positions, or Metal setup or execution failure.
    pub fn render_blend(&self, pass: &BlendPass<'_>) -> Result<Vec<u8>, MetalError> {
        let [width, height] = pass.extent;
        let pixels = width
            .checked_mul(height)
            .filter(|&count| (1..=MAX_PIXELS).contains(&count))
            .ok_or_else(|| MetalError::new("render bounds", "expected 1..=4096 pixels"))?;
        if pass.draws.len() > MAX_DEPTH_DRAWS {
            return Err(MetalError::new("blend pass", "expected at most 16 draws"));
        }
        if pass
            .draws
            .iter()
            .flat_map(|draw| draw.positions.as_flattened())
            .any(|value| !value.is_finite())
        {
            return Err(MetalError::new(
                "render input",
                "expected finite vertex data",
            ));
        }

        autoreleasepool(|_| {
            let options = compile_options()?;
            let library = self.compile_library(DEPTH_SOURCE, &options)?;
            let vertex = find_function(&library, "depth_vertex")?;
            let fragment = find_function(&library, "depth_flat")?;
            let pipelines = pass
                .draws
                .iter()
                .map(|draw| self.blend_pipeline(&vertex, &fragment, draw))
                .collect::<Result<Vec<_>, _>>()?;
            let color = self.new_target(
                PIXEL_FORMAT,
                width,
                height,
                MTLStorageMode::Shared,
                MTLTextureUsage::RenderTarget,
            )?;
            let pass_descriptor = MTLRenderPassDescriptor::renderPassDescriptor();
            // SAFETY: Slot zero is the only color attachment for this pass.
            let attachment = unsafe {
                pass_descriptor
                    .colorAttachments()
                    .objectAtIndexedSubscript(0)
            };
            attachment.setTexture(Some(&color));
            attachment.setLoadAction(MTLLoadAction::Clear);
            let [red, green, blue, alpha] = pass.clear_color.map(|value| f64::from(value) / 255.0);
            attachment.setClearColor(MTLClearColor {
                red,
                green,
                blue,
                alpha,
            });
            attachment.setStoreAction(MTLStoreAction::Store);
            let words: Vec<u32> = pass
                .draws
                .iter()
                .flat_map(|draw| draw.positions.as_flattened())
                .map(|value| value.to_bits())
                .collect();
            let positions = (!words.is_empty())
                .then(|| self.new_buffer(&words))
                .transpose()?;
            let command = self.queue.commandBuffer().ok_or_else(|| {
                MetalError::new("commandBuffer", "render command buffer creation failed")
            })?;
            let encoder = command
                .renderCommandEncoderWithDescriptor(&pass_descriptor)
                .ok_or_else(|| {
                    MetalError::new(
                        "renderCommandEncoderWithDescriptor",
                        "render encoder creation failed",
                    )
                })?;
            encoder.setViewport(MTLViewport {
                originX: 0.0,
                originY: 0.0,
                width: width as f64,
                height: height as f64,
                znear: 0.0,
                zfar: 1.0,
            });
            for (index, (draw, pipeline)) in pass.draws.iter().zip(&pipelines).enumerate() {
                encoder.setRenderPipelineState(pipeline);
                let rgba = draw.color.map(|value| f32::from(value) / 255.0);
                // SAFETY: The position buffer holds three float4s per draw and
                // the vertex stage reads only this draw's three. The fragment
                // constant is copied by Metal and matches the declared float4.
                unsafe {
                    encoder.setVertexBuffer_offset_atIndex(positions.as_deref(), index * 48, 0);
                    encoder.setFragmentBytes_length_atIndex(
                        NonNull::from(&rgba).cast(),
                        size_of_val(&rgba),
                        0,
                    );
                    encoder.drawPrimitives_vertexStart_vertexCount(
                        MTLPrimitiveType::Triangle,
                        0,
                        3,
                    );
                }
            }
            encoder.endEncoding();
            complete_command(&command)?;

            let mut output = vec![0; pixels * 4];
            // SAFETY: GPU writes have completed, and the initialized output
            // allocation fits the entire shared texture with tightly packed rows.
            unsafe {
                color.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                    NonNull::from(output.as_mut_slice()).cast(),
                    width * 4,
                    whole(width, height),
                    0,
                );
            }
            Ok(output)
        })
    }

    fn blend_pipeline(
        &self,
        vertex: &ProtocolObject<dyn MTLFunction>,
        fragment: &ProtocolObject<dyn MTLFunction>,
        draw: &BlendDraw,
    ) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, MetalError> {
        let descriptor = MTLRenderPipelineDescriptor::new();
        descriptor.setVertexFunction(Some(vertex));
        descriptor.setFragmentFunction(Some(fragment));
        // SAFETY: Slot zero is the only color attachment for this pipeline.
        let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(PIXEL_FORMAT);
        attachment.setWriteMask(draw.write_mask.metal());
        attachment.setBlendingEnabled(draw.blend.is_some());
        if let Some(blend) = draw.blend {
            attachment.setSourceRGBBlendFactor(blend.source_rgb.metal());
            attachment.setDestinationRGBBlendFactor(blend.destination_rgb.metal());
            attachment.setRgbBlendOperation(blend.rgb_operation.metal());
            attachment.setSourceAlphaBlendFactor(blend.source_alpha.metal());
            attachment.setDestinationAlphaBlendFactor(blend.destination_alpha.metal());
            attachment.setAlphaBlendOperation(blend.alpha_operation.metal());
        }
        let pipeline = self
            .device
            .newRenderPipelineStateWithDescriptor_error(&descriptor)
            .map_err(|error| {
                MetalError::new(
                    "newRenderPipelineStateWithDescriptor",
                    error.localizedDescription().to_string(),
                )
            })?;
        verify_render_validation(&pipeline)?;
        Ok(pipeline)
    }
}
