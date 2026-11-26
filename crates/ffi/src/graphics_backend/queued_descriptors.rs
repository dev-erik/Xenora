// SPDX-License-Identifier: GPL-3.0-or-later
//! Borrowed binding normalization and four-slot validation before native admission.
use super::*;
use xenora_loader::graphics::{
    GraphicsArrayRangeViewDesc, GraphicsArraySliceViewDesc, GraphicsQueuedDraw,
    GraphicsTextureViewDesc, GraphicsTextureViewFormat,
};
use xenora_shader::{GraphicsArrayLayer, GraphicsExportType, GraphicsSampleType};

pub(super) struct SampledInputs<'a> {
    pub textures: &'a [Option<GraphicsTextureInput>],
    pub samplers: &'a [Option<xenora_texture::SamplerDesc>],
    pub views: &'a [Option<GraphicsTextureViewDesc>],
    pub array_views: &'a [Option<GraphicsArraySliceViewDesc>],
    pub sampled_ranges: &'a [Option<GraphicsArrayRangeViewDesc>],
    pub types: &'a [GraphicsSampleType],
    pub layers: &'a [Vec<GraphicsArrayLayer>],
    pub mask: u16,
    pub stride: usize,
}
impl SampledInputs<'_> {
    pub fn needs_texture(&self, slot: usize) -> bool {
        self.mask & (((1u16 << self.stride) - 1) << (slot * self.stride)) != 0
    }
    fn needs_sampler(&self, slot: usize) -> bool {
        (0..self.stride).any(|t| self.mask & (1 << (t * self.stride + slot)) != 0)
    }
}
pub(super) fn sampled_inputs(
    draw: &GraphicsQueuedDraw,
) -> Result<SampledInputs<'_>, GraphicsError> {
    let p = draw
        .pipeline
        .as_ref()
        .ok_or(GraphicsError::InvalidDescriptor)?;
    match (&p.descriptor_requirements, &draw.descriptor_bindings) {
        (Some(r), Some(b)) => {
            if draw.textures.iter().any(Option::is_some)
                || draw.samplers.iter().any(Option::is_some)
                || draw.views.iter().any(Option::is_some)
                || draw.array_views.iter().any(Option::is_some)
                || draw.sampled_ranges.iter().any(Option::is_some)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            Ok(SampledInputs {
                textures: &b.textures,
                samplers: &b.samplers,
                views: &b.views,
                array_views: &b.array_views,
                sampled_ranges: &b.sampled_ranges,
                types: &r.types.textures,
                layers: &r.array_layers,
                mask: r.sampled_pairs,
                stride: 4,
            })
        }
        (None, None) => Ok(SampledInputs {
            textures: &draw.textures,
            samplers: &draw.samplers,
            views: &draw.views,
            array_views: &draw.array_views,
            sampled_ranges: &draw.sampled_ranges,
            types: &p.types.textures,
            layers: &p.array_layers,
            mask: u16::from(p.sampled_pairs),
            stride: 2,
        }),
        _ => Err(GraphicsError::InvalidDescriptor),
    }
}
pub(super) fn validate_descriptor_draw(
    draw: &GraphicsQueuedDraw,
    colors: &[TextureFormat],
) -> Result<(), GraphicsError> {
    let p = draw
        .pipeline
        .as_ref()
        .ok_or(GraphicsError::InvalidDescriptor)?;
    let Some(r) = &p.descriptor_requirements else {
        return if draw.descriptor_bindings.is_none() {
            Ok(())
        } else {
            Err(GraphicsError::InvalidDescriptor)
        };
    };
    let inputs = sampled_inputs(draw)?;
    if (p.required_uniform_mask != 0 && draw.uniforms.is_none())
        || p.color_targets != colors.len()
        || p.uses_texture != (r.sampled_pairs != 0)
        || r.descriptor_index_uniform_mask & !p.required_uniform_mask != 0
    {
        return Err(GraphicsError::InvalidDescriptor);
    }
    for i in 0..8 {
        if r.descriptor_index_uniform_mask & (1 << i) != 0
            && draw.uniforms.as_ref().is_none_or(|u| u[i] >= 4)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
    }
    for (i, f) in colors.iter().enumerate() {
        if r.types.colors[i]
            != if *f == TextureFormat::R32Uint {
                GraphicsExportType::Uint
            } else {
                GraphicsExportType::Float
            }
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
    }
    for slot in 0..4 {
        if inputs.textures[slot].is_some() != inputs.needs_texture(slot)
            || inputs.samplers[slot].is_some() != inputs.needs_sampler(slot)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if !inputs.needs_texture(slot) {
            if inputs.views[slot].is_some()
                || inputs.array_views[slot].is_some()
                || inputs.sampled_ranges[slot].is_some()
                || !inputs.layers[slot].is_empty()
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            continue;
        }
        let texture = inputs.textures[slot]
            .as_ref()
            .ok_or(GraphicsError::InvalidDescriptor)?;
        let desc = match texture {
            GraphicsTextureInput::Host(t) => xenora_loader::graphics::GraphicsTypedTextureDesc {
                format: t.format,
                extent: (t.width, t.height),
                mip_levels: t.mip_levels,
                array_layers: t.array_layers,
            },
            GraphicsTextureInput::ProducedTyped { desc, .. } => *desc,
            GraphicsTextureInput::ProducedArray {
                width,
                height,
                mip_levels,
                array_layers,
                ..
            } => xenora_loader::graphics::GraphicsTypedTextureDesc {
                format: TextureFormat::Rgba8Unorm,
                extent: (*width, *height),
                mip_levels: *mip_levels,
                array_layers: Some(*array_layers),
            },
            GraphicsTextureInput::Produced {
                width,
                height,
                mip_levels,
                ..
            } => xenora_loader::graphics::GraphicsTypedTextureDesc {
                format: TextureFormat::Rgba8Unorm,
                extent: (*width, *height),
                mip_levels: *mip_levels,
                array_layers: None,
            },
        };
        let layout = PackedTextureArrayLayout::new(
            desc.format,
            desc.extent.0,
            desc.extent.1,
            desc.mip_levels,
            desc.array_layers.unwrap_or(1),
        )
        .map_err(|_| GraphicsError::InvalidDescriptor)?;
        if let GraphicsTextureInput::Host(t) = texture
            && t.pixels.len() != layout.byte_len()
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let (base_mip, mip_count, view_format) = if let Some(v) = inputs.sampled_ranges[slot] {
            if inputs.views[slot].is_some()
                || inputs.array_views[slot].is_some()
                || desc.array_layers.is_none()
                || v.slice_count == 0
                || v.base_slice
                    .checked_add(v.slice_count)
                    .is_none_or(|end| end > desc.array_layers.unwrap_or(0))
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            (v.base_mip, v.mip_count, v.format)
        } else if let Some(v) = inputs.array_views[slot] {
            if inputs.views[slot].is_some() || desc.array_layers.is_none_or(|n| v.slice >= n) {
                return Err(GraphicsError::InvalidDescriptor);
            }
            (v.base_mip, v.mip_count, v.format)
        } else if let Some(v) = inputs.views[slot] {
            if desc.array_layers.is_some() {
                return Err(GraphicsError::InvalidDescriptor);
            }
            (v.base_mip, v.mip_count, v.format)
        } else {
            return Err(GraphicsError::InvalidDescriptor);
        };
        let storage_format =
            match view_format {
                GraphicsTextureViewFormat::Rg8Unorm => TextureFormat::Rg8Unorm,
                GraphicsTextureViewFormat::Bgra8Unorm
                | GraphicsTextureViewFormat::Bgra8UnormSrgb => TextureFormat::Bgra8Unorm,
                GraphicsTextureViewFormat::Rgba16Float => TextureFormat::Rgba16Float,
                GraphicsTextureViewFormat::R32Float => TextureFormat::R32Float,
                GraphicsTextureViewFormat::R32Uint => TextureFormat::R32Uint,
                GraphicsTextureViewFormat::Depth32Float => TextureFormat::Depth32Float,
                GraphicsTextureViewFormat::R8Unorm => TextureFormat::R8Unorm,
                GraphicsTextureViewFormat::Rgba8Unorm
                | GraphicsTextureViewFormat::Rgba8UnormSrgb => TextureFormat::Rgba8Unorm,
                GraphicsTextureViewFormat::Bc1RgbaUnorm
                | GraphicsTextureViewFormat::Bc1RgbaUnormSrgb => TextureFormat::Bc1RgbaUnorm,
                GraphicsTextureViewFormat::Bc3RgbaUnorm
                | GraphicsTextureViewFormat::Bc3RgbaUnormSrgb => TextureFormat::Bc3RgbaUnorm,
            };
        if storage_format != desc.format {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if mip_count == 0
            || base_mip
                .checked_add(mip_count)
                .is_none_or(|end| end > desc.mip_levels)
        {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let array = inputs.sampled_ranges[slot].is_some();
        let expected = match (desc.format, array) {
            (TextureFormat::R32Uint, true) => GraphicsSampleType::UintArray,
            (TextureFormat::Depth32Float, true) => return Err(GraphicsError::InvalidDescriptor),
            (_, true) => GraphicsSampleType::FloatArray,
            (TextureFormat::R32Uint, false) => GraphicsSampleType::Uint,
            (TextureFormat::Depth32Float, false) => GraphicsSampleType::Depth,
            (_, false) => GraphicsSampleType::Float,
        };
        if inputs.types[slot] != expected {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let footprint = layout
            .footprint(0, base_mip)
            .map_err(|_| GraphicsError::InvalidDescriptor)?;
        let metadata =
            xenora_texture::TextureMetadata::new(footprint.width, footprint.height, mip_count)
                .map_err(|_| GraphicsError::InvalidDescriptor)?;
        for (sampler_slot, sampler) in inputs.samplers.iter().enumerate() {
            if inputs.mask & (1 << (slot * 4 + sampler_slot)) != 0 {
                sampler
                    .as_ref()
                    .ok_or(GraphicsError::InvalidDescriptor)?
                    .validate_against_texture(&metadata)
                    .map_err(|_| GraphicsError::InvalidDescriptor)?;
            }
        }
    }
    super::validate_shader_array_layers(draw)
}

/// Resolve every descriptor producer before a transaction can admit a host seed.
pub(super) fn validate_descriptor_producers(
    draw: &GraphicsQueuedDraw,
    waits: &[GraphicsQueuePoint],
    entries: &HashMap<GraphicsQueuePoint, Entry>,
) -> Result<(), GraphicsError> {
    let inputs = sampled_inputs(draw)?;
    for texture in inputs.textures.iter().flatten() {
        match texture {
            GraphicsTextureInput::Host(_) => {}
            GraphicsTextureInput::ProducedTyped { version, desc } => {
                if !waits.contains(&version.point)
                    || produced_typed_output(entries, *version)?.desc() != native_desc(*desc)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
            }
            GraphicsTextureInput::ProducedArray {
                version,
                width,
                height,
                mip_levels,
                array_layers,
            } => {
                if !waits.contains(&version.point) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let output = produced_typed_output(entries, *version)?;
                if output.desc()
                    != (xenora_metal::TypedTextureDesc {
                        format: TextureFormat::Rgba8Unorm,
                        extent: [*width, *height],
                        mip_levels: *mip_levels,
                        array_layers: Some(*array_layers),
                    })
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
            }
            GraphicsTextureInput::Produced {
                version,
                width,
                height,
                mip_levels,
            } => {
                if !waits.contains(&version.point) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let output = produced_output(entries, *version)?;
                if output.is_array()
                    || output.extent() != [*width, *height]
                    || output.mip_levels() != *mip_levels
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
            }
        }
    }
    Ok(())
}
