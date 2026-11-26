// SPDX-License-Identifier: GPL-3.0-or-later

//! Native adapter for bounded loader pipelines and the original constant-red fixture.
//! This host interface does not define a general guest shader ABI.

mod queued;
pub use queued::install_queued_pipeline_backend;

use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use xenora_loader::graphics::{
    CommandListType, ExecutionOutcome, GraphicsArraySliceViewDesc, GraphicsBlendMode,
    GraphicsColorBinding, GraphicsContext, GraphicsDepthBinding, GraphicsDepthCompare,
    GraphicsDepthLoad, GraphicsDepthSnapshot, GraphicsDraw, GraphicsDrawState, GraphicsError,
    GraphicsHandle, GraphicsPassOutput, GraphicsPendingSubmission, GraphicsSubmissionRequest,
    GraphicsTextureCapabilities, GraphicsTextureSnapshot, GraphicsTextureViewDesc,
    GraphicsTextureViewFormat, ResourceDesc, ResourceDimension, ResourceStates,
};
use xenora_loader::memory::{GuestMemory, MemoryPermissions};
use xenora_metal::{
    ArraySliceViewDesc, BcMipUpload, BcTextureDesc, BindingSlot, BlendFactor as F,
    BlendOperation as O, BlendState, BoundSampledTexture, ColorWriteMask, DepthCompare, DepthLoad,
    DepthSnapshot, DepthState, MetalBcTexture, MetalR8Texture, MetalRgba8ArrayTexture,
    MetalRgba8Texture, MetalRgba8TextureDesc, MetalSampler, MetalTextureView, NativeCompute,
    R8MipUpload, R8TextureDesc, Rgba8ArrayMipUpload, Rgba8ArrayTextureDesc, Rgba8MipLevel,
    SampledBinding, TextureViewDesc, TextureViewFormat, TriangleDraw, TriangleShader,
    TriangleState,
};
use xenora_shader::{PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT};
use xenora_texture::{PackedTextureArrayLayout, PackedTextureLayout, TextureFormat};

const VERTICES: u64 = 0x10000;
const MAX_VERTICES: usize = 768;

struct UploadedTexture {
    array_layers: Option<u32>,
    pixels: Arc<[u8]>,
    width: usize,
    height: usize,
    mips: u32,
    format: TextureFormat,
    texture: UploadedStorage,
}

struct UploadedView {
    array_desc: Option<GraphicsArraySliceViewDesc>,
    array_layers: Option<u32>,
    pixels: Arc<[u8]>,
    backing: (TextureFormat, usize, usize, u32),
    desc: GraphicsTextureViewDesc,
    view: MetalTextureView,
}

#[derive(Clone)]
enum UploadedStorage {
    Typed(xenora_metal::MetalTypedTexture),
    Array(MetalRgba8ArrayTexture),
    Rgba8(MetalRgba8Texture),
    Bc(MetalBcTexture),
    R8(MetalR8Texture),
}
impl UploadedStorage {
    fn view(&self, desc: TextureViewDesc) -> Result<MetalTextureView, GraphicsError> {
        match self {
            Self::Typed(t) => t.view(desc).map_err(metal_error),
            Self::Array(_) => Err(GraphicsError::InvalidDescriptor),
            Self::Rgba8(t) => t.view(desc).map_err(metal_error),
            Self::Bc(t) => t.view(desc).map_err(metal_error),
            Self::R8(t) => t.view(desc).map_err(metal_error),
        }
    }
    fn rgba8(&self) -> Result<MetalRgba8Texture, GraphicsError> {
        match self {
            Self::Rgba8(t) => Ok(t.clone()),
            Self::Typed(_) | Self::Array(_) | Self::Bc(_) | Self::R8(_) => {
                Err(GraphicsError::InvalidDescriptor)
            }
        }
    }
}

fn texture_capabilities(gpu: &NativeCompute) -> GraphicsTextureCapabilities {
    let supported = gpu.supports_bc_textures();
    GraphicsTextureCapabilities {
        bc1: supported,
        bc3: supported,
        // R8Unorm is supported across Apple GPU families in Apple's feature tables:
        // https://developer.apple.com/metal/Metal-Feature-Set-Tables.pdf
        r8: true,
    }
}

// Non-RGBA8 whole resources use an immutable full-range native view.
fn sampled_view_desc(
    snapshot: &GraphicsTextureSnapshot,
    explicit: Option<GraphicsTextureViewDesc>,
) -> Option<GraphicsTextureViewDesc> {
    explicit.or_else(|| match snapshot.format {
        TextureFormat::Rgba8Unorm => None,
        format => Some(GraphicsTextureViewDesc {
            base_mip: 0,
            mip_count: snapshot.mip_levels,
            format: match format {
                TextureFormat::R8Unorm => GraphicsTextureViewFormat::R8Unorm,
                TextureFormat::Bc1RgbaUnorm => GraphicsTextureViewFormat::Bc1RgbaUnorm,
                TextureFormat::Bc3RgbaUnorm => GraphicsTextureViewFormat::Bc3RgbaUnorm,
                TextureFormat::Rgba8Unorm => unreachable!(),
                TextureFormat::Rg8Unorm => GraphicsTextureViewFormat::Rg8Unorm,
                TextureFormat::Bgra8Unorm => GraphicsTextureViewFormat::Bgra8Unorm,
                TextureFormat::Rgba16Float => GraphicsTextureViewFormat::Rgba16Float,
                TextureFormat::R32Float => GraphicsTextureViewFormat::R32Float,
                TextureFormat::R32Uint => GraphicsTextureViewFormat::R32Uint,
                TextureFormat::Depth32Float => GraphicsTextureViewFormat::Depth32Float,
            },
        }),
    })
}

fn native_view_desc(desc: GraphicsTextureViewDesc) -> TextureViewDesc {
    TextureViewDesc {
        base_mip: desc.base_mip,
        mip_count: desc.mip_count,
        format: match desc.format {
            GraphicsTextureViewFormat::Rg8Unorm => TextureViewFormat::Rg8Unorm,
            GraphicsTextureViewFormat::Bgra8Unorm => TextureViewFormat::Bgra8Unorm,
            GraphicsTextureViewFormat::Bgra8UnormSrgb => TextureViewFormat::Bgra8UnormSrgb,
            GraphicsTextureViewFormat::Rgba16Float => TextureViewFormat::Rgba16Float,
            GraphicsTextureViewFormat::R32Float => TextureViewFormat::R32Float,
            GraphicsTextureViewFormat::R32Uint => TextureViewFormat::R32Uint,
            GraphicsTextureViewFormat::Depth32Float => TextureViewFormat::Depth32Float,
            GraphicsTextureViewFormat::R8Unorm => TextureViewFormat::R8Unorm,
            GraphicsTextureViewFormat::Rgba8Unorm => TextureViewFormat::Rgba8Unorm,
            GraphicsTextureViewFormat::Rgba8UnormSrgb => TextureViewFormat::Rgba8UnormSrgb,
            GraphicsTextureViewFormat::Bc1RgbaUnorm => TextureViewFormat::Bc1RgbaUnorm,
            GraphicsTextureViewFormat::Bc1RgbaUnormSrgb => TextureViewFormat::Bc1RgbaUnormSrgb,
            GraphicsTextureViewFormat::Bc3RgbaUnorm => TextureViewFormat::Bc3RgbaUnorm,
            GraphicsTextureViewFormat::Bc3RgbaUnormSrgb => TextureViewFormat::Bc3RgbaUnormSrgb,
        },
    }
}

fn upload_view(
    gpu: &NativeCompute,
    snapshot: &GraphicsTextureSnapshot,
    desc: GraphicsTextureViewDesc,
    textures: &mut Vec<UploadedTexture>,
    views: &mut Vec<UploadedView>,
) -> Result<MetalTextureView, GraphicsError> {
    if snapshot.array_layers.is_some() {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let backing = (
        snapshot.format,
        snapshot.width,
        snapshot.height,
        snapshot.mip_levels,
    );
    if let Some(cached) = views.iter().find(|view| {
        view.array_desc.is_none()
            && view.array_layers.is_none()
            && Arc::ptr_eq(&view.pixels, &snapshot.pixels)
            && view.backing == backing
            && view.desc == desc
    }) {
        return Ok(cached.view.clone());
    }
    let texture = upload_texture(gpu, snapshot, textures)?;
    let view = texture.view(native_view_desc(desc))?;
    views.push(UploadedView {
        array_desc: None,
        array_layers: None,
        pixels: snapshot.pixels.clone(),
        backing,
        desc,
        view: view.clone(),
    });
    Ok(view)
}

fn native_array_view_desc(desc: GraphicsArraySliceViewDesc) -> ArraySliceViewDesc {
    ArraySliceViewDesc {
        slice: desc.slice,
        base_mip: desc.base_mip,
        mip_count: desc.mip_count,
        format: native_view_desc(GraphicsTextureViewDesc {
            base_mip: desc.base_mip,
            mip_count: desc.mip_count,
            format: desc.format,
        })
        .format,
    }
}
fn upload_array_view(
    gpu: &NativeCompute,
    snapshot: &GraphicsTextureSnapshot,
    desc: GraphicsArraySliceViewDesc,
    textures: &mut Vec<UploadedTexture>,
    views: &mut Vec<UploadedView>,
) -> Result<MetalTextureView, GraphicsError> {
    if snapshot.array_layers.is_none() {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let backing = (
        snapshot.format,
        snapshot.width,
        snapshot.height,
        snapshot.mip_levels,
    );
    if let Some(cached) = views.iter().find(|v| {
        Arc::ptr_eq(&v.pixels, &snapshot.pixels)
            && v.backing == backing
            && v.array_layers == snapshot.array_layers
            && v.array_desc == Some(desc)
    }) {
        return Ok(cached.view.clone());
    }
    let view = match upload_texture(gpu, snapshot, textures)? {
        UploadedStorage::Array(owner) => owner.array_slice_view(native_array_view_desc(desc)),
        UploadedStorage::Typed(owner) => owner.array_slice_view(native_array_view_desc(desc)),
        _ => return Err(GraphicsError::InvalidDescriptor),
    }
    .map_err(metal_error)?;
    views.push(UploadedView {
        pixels: snapshot.pixels.clone(),
        backing,
        array_layers: snapshot.array_layers,
        array_desc: Some(desc),
        desc: GraphicsTextureViewDesc {
            base_mip: desc.base_mip,
            mip_count: desc.mip_count,
            format: desc.format,
        },
        view: view.clone(),
    });
    Ok(view)
}
fn upload_selected_view(
    gpu: &NativeCompute,
    snapshot: &GraphicsTextureSnapshot,
    desc: Option<GraphicsTextureViewDesc>,
    array_desc: Option<GraphicsArraySliceViewDesc>,
    textures: &mut Vec<UploadedTexture>,
    views: &mut Vec<UploadedView>,
) -> Result<Option<MetalTextureView>, GraphicsError> {
    match (snapshot.array_layers, desc, array_desc) {
        (Some(_), None, Some(desc)) => {
            upload_array_view(gpu, snapshot, desc, textures, views).map(Some)
        }
        (None, desc, None) => sampled_view_desc(snapshot, desc)
            .map(|desc| upload_view(gpu, snapshot, desc, textures, views))
            .transpose(),
        _ => Err(GraphicsError::InvalidDescriptor),
    }
}

fn upload_texture(
    gpu: &NativeCompute,
    sampled: &GraphicsTextureSnapshot,
    textures: &mut Vec<UploadedTexture>,
) -> Result<UploadedStorage, GraphicsError> {
    let layout = PackedTextureLayout::new(
        sampled.format,
        sampled.width,
        sampled.height,
        sampled.mip_levels,
    )
    .map_err(|_| GraphicsError::InvalidDescriptor)?;
    let array_layout = sampled
        .array_layers
        .map(|layers| {
            PackedTextureArrayLayout::new(
                sampled.format,
                sampled.width,
                sampled.height,
                sampled.mip_levels,
                layers,
            )
            .map_err(|_| GraphicsError::InvalidDescriptor)
        })
        .transpose()?;
    if sampled.pixels.len()
        != array_layout
            .as_ref()
            .map_or(layout.byte_len(), |l| l.byte_len())
    {
        return Err(GraphicsError::InvalidDescriptor);
    }
    if let Some(uploaded) = textures.iter().find(|uploaded| {
        Arc::ptr_eq(&uploaded.pixels, &sampled.pixels)
            && uploaded.width == sampled.width
            && uploaded.height == sampled.height
            && uploaded.mips == sampled.mip_levels
            && uploaded.array_layers == sampled.array_layers
            && uploaded.format == sampled.format
    }) {
        return Ok(uploaded.texture.clone());
    }
    let typed = !matches!(
        sampled.format,
        TextureFormat::Rgba8Unorm
            | TextureFormat::R8Unorm
            | TextureFormat::Bc1RgbaUnorm
            | TextureFormat::Bc3RgbaUnorm
    ) || (sampled.array_layers.is_some()
        && sampled.format != TextureFormat::Rgba8Unorm);
    let texture = if typed {
        UploadedStorage::Typed(
            xenora_metal::MetalTypedTexture::upload(
                gpu,
                xenora_metal::TypedTextureDesc {
                    format: sampled.format,
                    extent: [sampled.width, sampled.height],
                    mip_levels: sampled.mip_levels,
                    array_layers: sampled.array_layers,
                },
                &sampled.pixels,
            )
            .map_err(metal_error)?,
        )
    } else if let Some(array) = array_layout {
        let mut uploads = Vec::new();
        for slice in 0..array.layers() {
            for level in 0..sampled.mip_levels {
                let mip = array
                    .footprint(slice, level)
                    .map_err(|_| GraphicsError::InvalidDescriptor)?;
                uploads.push(Rgba8ArrayMipUpload {
                    slice,
                    level,
                    row_pitch: mip.row_bytes,
                    bytes: &sampled.pixels[mip.offset..mip.offset + mip.byte_len],
                });
            }
        }
        UploadedStorage::Array(
            MetalRgba8ArrayTexture::upload(
                gpu.device(),
                Rgba8ArrayTextureDesc {
                    width: sampled.width,
                    height: sampled.height,
                    mips: sampled.mip_levels,
                    array_layers: array.layers(),
                },
                &uploads,
            )
            .map_err(metal_error)?,
        )
    } else {
        match sampled.format {
            TextureFormat::Rgba8Unorm => {
                let desc =
                    MetalRgba8TextureDesc::new(sampled.width, sampled.height, sampled.mip_levels)
                        .map_err(metal_error)?;
                let texture =
                    MetalRgba8Texture::allocate(gpu.device(), desc).map_err(metal_error)?;
                for (level, mip) in layout.levels().iter().enumerate() {
                    texture
                        .upload_mip(&Rgba8MipLevel::new_packed(
                            level as u32,
                            mip.width,
                            mip.height,
                            &sampled.pixels[mip.offset..mip.offset + mip.byte_len],
                        ))
                        .map_err(metal_error)?;
                }
                UploadedStorage::Rgba8(texture)
            }
            TextureFormat::R8Unorm => {
                let uploads: Vec<_> = layout
                    .levels()
                    .iter()
                    .enumerate()
                    .map(|(level, mip)| R8MipUpload {
                        level: level as u32,
                        row_pitch: mip.row_bytes,
                        bytes: &sampled.pixels[mip.offset..mip.offset + mip.byte_len],
                    })
                    .collect();
                UploadedStorage::R8(
                    MetalR8Texture::upload(
                        gpu.device(),
                        R8TextureDesc {
                            width: sampled.width,
                            height: sampled.height,
                            mips: sampled.mip_levels,
                        },
                        &uploads,
                    )
                    .map_err(metal_error)?,
                )
            }
            format @ (TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc3RgbaUnorm) => {
                let uploads: Vec<_> = layout
                    .levels()
                    .iter()
                    .enumerate()
                    .map(|(level, mip)| BcMipUpload {
                        level: level as u32,
                        row_pitch: mip.row_bytes,
                        bytes: &sampled.pixels[mip.offset..mip.offset + mip.byte_len],
                    })
                    .collect();
                UploadedStorage::Bc(
                    MetalBcTexture::upload(
                        gpu.device(),
                        BcTextureDesc {
                            width: sampled.width,
                            height: sampled.height,
                            mips: sampled.mip_levels,
                            format,
                        },
                        &uploads,
                    )
                    .map_err(metal_error)?,
                )
            }
            _ => return Err(GraphicsError::InvalidDescriptor),
        }
    };
    textures.push(UploadedTexture {
        array_layers: sampled.array_layers,
        pixels: sampled.pixels.clone(),
        width: sampled.width,
        height: sampled.height,
        mips: sampled.mip_levels,
        format: sampled.format,
        texture: texture.clone(),
    });
    Ok(texture)
}

/// Install native rendering for loader-owned pipeline and sampled-resource snapshots.
/// Every nonempty draw requires an explicit compiled pipeline. Empty passes need no shader.
/// The adapter returns fresh owned depth bytes; the loader decides when to commit them.
pub fn install_pipeline_backend(context: &mut GraphicsContext) -> Result<(), GraphicsError> {
    let gpu = NativeCompute::new().map_err(metal_error)?;
    let capabilities = texture_capabilities(&gpu);
    let roles = resource_role_capabilities(&gpu);
    context.set_metal_array_view_attachment_backend(
        capabilities,
        move |draws, extent, attachments, transient_clear, binding| {
            render_pipeline_request(
                &gpu,
                draws,
                extent,
                attachments,
                transient_clear,
                binding,
                &mut None,
            )
        },
    );
    context.enable_resource_roles(roles)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_pipeline_request(
    gpu: &NativeCompute,
    draws: &[GraphicsDraw],
    extent: (usize, usize),
    attachments: &[GraphicsColorBinding],
    transient_clear: f32,
    binding: Option<GraphicsDepthBinding>,
    committed: &mut Option<mpsc::SyncSender<Result<(), GraphicsError>>>,
) -> Result<GraphicsPassOutput, GraphicsError> {
    let clears: Vec<_> = attachments
        .iter()
        .map(|attachment| attachment.clear)
        .collect();
    let color_loads: Vec<_> = attachments
        .iter()
        .map(|attachment| attachment.previous.as_deref())
        .collect();
    if !(1..=8).contains(&clears.len()) {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let shaders = draws
        .iter()
        .map(|draw| {
            let pipeline = draw
                .pipeline
                .as_ref()
                .ok_or(GraphicsError::InvalidDescriptor)?;
            let color_formats: Vec<_> = attachments
                .iter()
                .map(|c| c.typed.map_or(TextureFormat::Rgba8Unorm, |t| t.format))
                .collect();
            if draw
                .sampled_bindings
                .sampled_ranges
                .iter()
                .any(Option::is_some)
            {
                return Err(GraphicsError::NativeMetalRequired);
            }
            validate_pipeline_formats(
                pipeline,
                &color_formats,
                std::array::from_fn(|slot| {
                    draw.sampled_bindings.textures[slot]
                        .as_ref()
                        .map(|t| t.format)
                }),
                [false; 2],
            )?;
            if pipeline.color_targets != clears.len()
                || draw.colors.len() != draw.vertices.len()
                || pipeline.sampled_pairs & !15 != 0
                || pipeline.uses_texture != (pipeline.sampled_pairs != 0)
                || (0..2).any(|slot| {
                    draw.sampled_bindings.textures[slot].is_some()
                        != (pipeline.sampled_pairs & (3 << (slot * 2)) != 0)
                        || draw.sampled_bindings.samplers[slot].is_some()
                            != (pipeline.sampled_pairs & (5 << slot) != 0)
                })
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            Ok(TriangleShader {
                source: pipeline.msl.as_str(),
                entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
            })
        })
        .collect::<Result<Vec<_>, GraphicsError>>()?;
    let states: Vec<_> = draws.iter().map(|draw| metal_state(draw.state)).collect();
    let native_draws: Vec<_> = draws
        .iter()
        .map(|draw| TriangleDraw {
            vertices: &draw.vertices,
            colors: &draw.colors,
            viewport: [draw.viewport.0, draw.viewport.1],
        })
        .collect();
    // Upload each immutable pixel snapshot once, including reuse across slots.
    let mut texture_cache: Vec<UploadedTexture> = Vec::new();
    let mut view_cache = Vec::new();
    let sampled = draws
        .iter()
        .map(|draw| {
            let native_textures = draw
                .sampled_bindings
                .textures
                .iter()
                .map(|texture| {
                    texture
                        .as_ref()
                        .map(|texture| upload_texture(gpu, texture, &mut texture_cache))
                        .transpose()
                })
                .collect::<Result<Vec<_>, GraphicsError>>()?;
            let native_views = draw
                .sampled_bindings
                .views
                .iter()
                .enumerate()
                .map(
                    |(slot, desc)| match draw.sampled_bindings.textures[slot].as_ref() {
                        Some(snapshot) => upload_selected_view(
                            gpu,
                            snapshot,
                            *desc,
                            draw.sampled_bindings.array_views[slot],
                            &mut texture_cache,
                            &mut view_cache,
                        ),
                        None if desc.is_none()
                            && draw.sampled_bindings.array_views[slot].is_none() =>
                        {
                            Ok(None)
                        }
                        None => Err(GraphicsError::InvalidDescriptor),
                    },
                )
                .collect::<Result<Vec<_>, GraphicsError>>()?;
            // One object per sampler slot per draw. Cross-pairs must share this object
            // so the native slot table cannot accidentally depend on iteration order.
            let native_samplers = draw
                .sampled_bindings
                .samplers
                .iter()
                .map(|sampler| {
                    sampler
                        .map(|desc| MetalSampler::new(gpu.device(), desc).map_err(metal_error))
                        .transpose()
                })
                .collect::<Result<Vec<_>, GraphicsError>>()?;
            let pairs = draw
                .pipeline
                .as_ref()
                .ok_or(GraphicsError::InvalidDescriptor)?
                .sampled_pairs;
            let mut bindings = Vec::new();
            for (texture_slot, texture) in native_textures.iter().enumerate() {
                for (sampler_slot, sampler) in native_samplers.iter().enumerate() {
                    if pairs & (1 << (texture_slot * 2 + sampler_slot)) != 0 {
                        let sampler = sampler
                            .as_ref()
                            .ok_or(GraphicsError::InvalidDescriptor)?
                            .clone();
                        let native_texture_slot = BindingSlot::new(texture_slot as u32);
                        let native_sampler_slot = BindingSlot::new(sampler_slot as u32);
                        bindings.push(if let Some(view) = &native_views[texture_slot] {
                            SampledBinding::View {
                                view: view.clone(),
                                sampler,
                                texture_slot: native_texture_slot,
                                sampler_slot: native_sampler_slot,
                            }
                        } else {
                            SampledBinding::Whole(
                                BoundSampledTexture::bind(
                                    texture
                                        .as_ref()
                                        .ok_or(GraphicsError::InvalidDescriptor)?
                                        .rgba8()?,
                                    sampler,
                                    native_texture_slot,
                                    native_sampler_slot,
                                )
                                .map_err(metal_error)?,
                            )
                        });
                    }
                }
            }
            Ok(bindings)
        })
        .collect::<Result<Vec<_>, GraphicsError>>()?;
    let mut retained = match binding.as_ref() {
        Some(binding) => Some(match binding.load {
            GraphicsDepthLoad::Clear(_) => gpu
                .create_depth_target([extent.0, extent.1])
                .map_err(metal_error)?,
            GraphicsDepthLoad::Load => {
                let snapshot = native_depth_snapshot(
                    binding
                        .previous
                        .as_ref()
                        .ok_or(GraphicsError::InvalidDescriptor)?,
                )?;
                gpu.restore_depth(&snapshot).map_err(metal_error)?
            }
        }),
        None => None,
    };
    let load = binding.as_ref().map(|binding| match binding.load {
        GraphicsDepthLoad::Clear(value) => DepthLoad::Clear(value),
        GraphicsDepthLoad::Load => DepthLoad::Load,
    });
    let typed_colors: Vec<_> = attachments
        .iter()
        .map(|c| native_color_attachment(c.typed, c.clear))
        .collect();
    let pass = xenora_metal::TypedTriangleTargetsPass {
        draws: &native_draws,
        states: &states,
        extent: [extent.0, extent.1],
        colors: &typed_colors,
        clear_depth: (binding.is_none() && draws.iter().any(|d| d.state.depth_compare.is_some()))
            .then_some(transient_clear),
    };
    let uniforms: Vec<_> = draws
        .iter()
        .map(|draw| xenora_metal::DrawUniforms {
            required_mask: draw
                .pipeline
                .as_ref()
                .map_or(0, |p| p.required_uniform_mask),
            values: draw.uniforms,
        })
        .collect();
    let pending = gpu
        .submit_typed_triangle_targets(
            &shaders,
            &pass,
            &sampled,
            retained.as_mut().zip(load),
            &color_loads,
            &uniforms,
        )
        .map_err(metal_error)?;
    if let Some(acknowledgement) = committed.take() {
        acknowledgement
            .send(Ok(()))
            .map_err(|_| GraphicsError::BackendError("submission caller disconnected".into()))?;
    }
    let color_layouts = Some(
        pending
            .color_layouts()
            .into_iter()
            .map(|desc| xenora_loader::graphics::GraphicsTypedTextureDesc {
                format: desc.format,
                extent: (desc.extent[0], desc.extent[1]),
                mip_levels: desc.mip_levels,
                array_layers: desc.array_layers,
            })
            .collect(),
    );
    let colors = pending.wait().map_err(metal_error)?;
    let depth = retained
        .as_ref()
        .map(|depth| {
            gpu.snapshot_depth(depth)
                .map(|snapshot| packed_depth_snapshot(snapshot, extent))
                .map_err(metal_error)
        })
        .transpose()?;
    Ok(GraphicsPassOutput {
        colors,
        color_layouts,
        depth,
    })
}

type Completion = Result<GraphicsPassOutput, GraphicsError>;
struct WorkerRequest {
    request: GraphicsSubmissionRequest,
    committed: mpsc::SyncSender<Result<(), GraphicsError>>,
    completed: mpsc::Sender<Completion>,
}

struct RenderWorker {
    requests: Option<mpsc::Sender<WorkerRequest>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Ok(thread) = self.thread.get_mut()
            && let Some(thread) = thread.take()
        {
            let _ = thread.join();
        }
    }
}
struct NativeSubmission {
    completed: mpsc::Receiver<Completion>,
    // Keep the worker alive until this ticket has drained or been dropped.
    _worker: Arc<RenderWorker>,
    terminal: bool,
}
impl GraphicsPendingSubmission for NativeSubmission {
    fn poll(&mut self) -> Option<Completion> {
        if self.terminal {
            return Some(Err(GraphicsError::BackendError(
                "submission already collected".into(),
            )));
        }
        let result = match self.completed.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err(GraphicsError::BackendError(
                "native worker disconnected".into(),
            )),
        };
        self.terminal = true;
        Some(result)
    }
    fn wait(&mut self) -> Completion {
        if self.terminal {
            return Err(GraphicsError::BackendError(
                "submission already collected".into(),
            ));
        }
        let result = self.completed.recv().unwrap_or_else(|_| {
            Err(GraphicsError::BackendError(
                "native worker disconnected".into(),
            ))
        });
        self.terminal = true;
        result
    }
}
impl Drop for NativeSubmission {
    fn drop(&mut self) {
        if !self.terminal {
            let _ = self.wait();
        }
    }
}

/// Install a native worker that acknowledges submission only after Metal commit.
/// The worker owns all native wrappers; only owned host snapshots cross channels.
pub fn install_async_pipeline_backend(context: &mut GraphicsContext) -> Result<(), GraphicsError> {
    install_async_pipeline_backend_inner(context, |_| Ok(()))
}

fn install_async_pipeline_backend_inner<F>(
    context: &mut GraphicsContext,
    initialize: F,
) -> Result<(), GraphicsError>
where
    F: FnOnce(&NativeCompute) -> Result<(), GraphicsError> + Send + 'static,
{
    let (requests, incoming) = mpsc::channel::<WorkerRequest>();
    let (initialized, ready) = mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("xenora-native-render".into())
        .spawn(move || {
            let gpu = match NativeCompute::new().map_err(metal_error) {
                Ok(gpu) => gpu,
                Err(error) => {
                    let _ = initialized.send(Err(error));
                    return;
                }
            };
            if let Err(error) = initialize(&gpu) {
                let _ = initialized.send(Err(error));
                return;
            }
            if initialized
                .send(Ok((
                    texture_capabilities(&gpu),
                    resource_role_capabilities(&gpu),
                )))
                .is_err()
            {
                return;
            }
            while let Ok(message) = incoming.recv() {
                let mut committed = Some(message.committed);
                let request = message.request;
                let result = render_pipeline_request(
                    &gpu,
                    &request.draws,
                    request.extent,
                    &request.colors,
                    request.clear_depth,
                    request.depth,
                    &mut committed,
                );
                // Setup failure precedes native commit; no GPU work remains in this path.
                if let Some(acknowledgement) = committed {
                    let error = result
                        .as_ref()
                        .err()
                        .cloned()
                        .unwrap_or(GraphicsError::InvalidDescriptor);
                    let _ = acknowledgement.send(Err(error));
                }
                // render_pipeline_request has drained the native ticket before sending.
                let _ = message.completed.send(result);
            }
        })
        .map_err(|error| GraphicsError::BackendError(format!("native worker: {error}")))?;
    let worker = Arc::new(RenderWorker {
        requests: Some(requests),
        thread: Mutex::new(Some(thread)),
    });
    let (capabilities, roles) = ready.recv().map_err(|_| {
        GraphicsError::BackendError("native worker initialization disconnected".into())
    })??;
    context.set_metal_array_view_async_backend(capabilities, move |request| {
        let (committed, acknowledgement) = mpsc::sync_channel(1);
        let (completed, completion) = mpsc::channel();
        worker
            .requests
            .as_ref()
            .expect("live worker")
            .send(WorkerRequest {
                request,
                committed,
                completed,
            })
            .map_err(|_| GraphicsError::BackendError("native worker stopped".into()))?;
        acknowledgement.recv().map_err(|_| {
            GraphicsError::BackendError("native worker commit disconnected".into())
        })??;
        Ok(Box::new(NativeSubmission {
            completed: completion,
            _worker: worker.clone(),
            terminal: false,
        }))
    });
    context.enable_resource_roles(roles)?;
    Ok(())
}

fn metal_error(error: xenora_metal::MetalError) -> GraphicsError {
    GraphicsError::BackendError(error.to_string())
}

/// One reusable loader context and vertex buffer backed by native Metal.
/// All draws use the original fixture shader, which writes opaque red.
pub struct FixtureRenderer {
    context: GraphicsContext,
    memory: GuestMemory,
    queue: GraphicsHandle,
    allocator: GraphicsHandle,
    list: GraphicsHandle,
    buffer: GraphicsHandle,
    pipeline: GraphicsHandle,
}

impl FixtureRenderer {
    /// Compile the trusted fixture and initialize native rendering resources.
    pub fn new() -> Result<Self, String> {
        let mut context = GraphicsContext::new();
        install_pipeline_backend(&mut context).map_err(|e| format!("graphics setup: {e}"))?;
        let setup = |e: GraphicsError| format!("graphics setup: {e}");
        let device = context.create_device().map_err(setup)?;
        let pipeline = context
            .create_graphics_pipeline(
                device,
                include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
                include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
            )
            .map_err(setup)?;
        let queue = context
            .create_command_queue(device, CommandListType::Direct)
            .map_err(setup)?;
        let allocator = context
            .create_command_allocator(device, CommandListType::Direct)
            .map_err(setup)?;
        let list = context
            .create_command_list(device, allocator, CommandListType::Direct)
            .map_err(setup)?;
        let buffer = context
            .create_committed_resource(
                device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: (MAX_VERTICES * 16) as u64,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
                Some(VERTICES),
            )
            .map_err(setup)?;
        let mut memory = GuestMemory::new();
        memory
            .map_region(VERTICES, 16384, MemoryPermissions::READ_WRITE)
            .map_err(|e| format!("graphics memory: {e:?}"))?;
        Ok(Self {
            context,
            memory,
            queue,
            allocator,
            list,
            buffer,
            pipeline,
        })
    }

    /// Record and submit ordered draws with the default depth clear of 1.0.
    pub fn render(
        &mut self,
        draws: &[GraphicsDraw],
        clear: [u8; 4],
    ) -> Result<ExecutionOutcome, GraphicsError> {
        self.render_with_depth_clear(draws, clear, 1.0)
    }

    /// Render with an explicit finite depth clear in 0..=1.
    /// Invalid inputs can be retried; the context and buffer are reused.
    pub fn render_with_depth_clear(
        &mut self,
        draws: &[GraphicsDraw],
        clear: [u8; 4],
        depth_clear: f32,
    ) -> Result<ExecutionOutcome, GraphicsError> {
        if !depth_clear.is_finite() || !(0.0..=1.0).contains(&depth_clear) {
            return Err(GraphicsError::InvalidDescriptor);
        }
        if draws.is_empty() || draws.len() > 256 {
            return Err(GraphicsError::InvalidDescriptor);
        }
        let mut total = 0;
        for draw in draws {
            if draw.vertices.is_empty()
                || draw.vertices.len() > MAX_VERTICES
                || !draw.vertices.len().is_multiple_of(3)
            {
                return Err(GraphicsError::InvalidDescriptor);
            }
            total += draw.vertices.len();
            if total > MAX_VERTICES {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
        let bytes: Vec<_> = draws
            .iter()
            .flat_map(|d| &d.vertices)
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.memory.write_bytes(VERTICES, &bytes)?;
        self.context.reset_command_list(self.list, self.allocator)?;
        self.context
            .cmd_set_graphics_pipeline(self.list, Some(self.pipeline))?;
        self.context.cmd_set_depth_clear(self.list, depth_clear)?;
        self.context
            .cmd_set_render_targets(self.list, None, clear)?;
        self.context
            .cmd_set_vertex_buffer(self.list, self.buffer, 0, 16)?;
        let mut first = 0;
        for draw in draws {
            self.context
                .cmd_set_viewport(self.list, draw.viewport.0, draw.viewport.1)?;
            self.context.cmd_set_draw_state(self.list, draw.state)?;
            self.context
                .cmd_draw_instanced(self.list, draw.vertices.len() as u32, 1, first, 0)?;
            first += draw.vertices.len() as u32;
        }
        self.context.close_command_list(self.list)?;
        self.context
            .execute_command_list(self.queue, self.list, &self.memory)
    }
}

fn metal_state(state: GraphicsDrawState) -> TriangleState {
    use GraphicsDepthCompare as D;
    let compare = match state.depth_compare {
        None | Some(D::Always) => DepthCompare::Always,
        Some(D::Never) => DepthCompare::Never,
        Some(D::Less) => DepthCompare::Less,
        Some(D::Equal) => DepthCompare::Equal,
        Some(D::LessEqual) => DepthCompare::LessEqual,
        Some(D::Greater) => DepthCompare::Greater,
        Some(D::NotEqual) => DepthCompare::NotEqual,
        Some(D::GreaterEqual) => DepthCompare::GreaterEqual,
    };
    let blend = match state.blend {
        GraphicsBlendMode::Replace => None,
        GraphicsBlendMode::Alpha => Some(BlendState {
            source_rgb: F::SourceAlpha,
            destination_rgb: F::OneMinusSourceAlpha,
            rgb_operation: O::Add,
            source_alpha: F::One,
            destination_alpha: F::OneMinusSourceAlpha,
            alpha_operation: O::Add,
        }),
        GraphicsBlendMode::Additive => Some(BlendState {
            source_rgb: F::One,
            destination_rgb: F::One,
            rgb_operation: O::Add,
            source_alpha: F::One,
            destination_alpha: F::One,
            alpha_operation: O::Add,
        }),
    };
    TriangleState {
        viewport_origin: state.viewport_origin,
        scissor: state.scissor,
        depth: DepthState {
            compare,
            write: state.depth_write,
        },
        blend,
        write_mask: ColorWriteMask {
            red: state.color_write_mask & 1 != 0,
            green: state.color_write_mask & 2 != 0,
            blue: state.color_write_mask & 4 != 0,
            alpha: state.color_write_mask & 8 != 0,
        },
    }
}

#[cfg(all(test, feature = "gpu-tests"))]
mod async_tests {
    use super::*;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLEvent, MTLSharedEvent};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use xenora_loader::graphics::GraphicsSubmissionStatus;

    #[test]
    fn worker_acknowledges_commit_before_gpu_completion_and_retires_resources() {
        let event_owner = NativeCompute::new().unwrap();
        let event = event_owner.device().newSharedEvent().unwrap();
        let worker_event = event.clone();
        let rescue_event = event.clone();
        let timeout = Arc::new(AtomicBool::new(false));
        let rescue_timeout = timeout.clone();
        let (done, finished) = mpsc::channel();
        let rescue = thread::spawn(move || {
            if finished.recv_timeout(Duration::from_secs(2)).is_err() {
                rescue_timeout.store(true, Ordering::SeqCst);
                rescue_event.setSignaledValue(1);
            }
        });
        let mut context = GraphicsContext::new();
        install_async_pipeline_backend_inner(&mut context, move |gpu| {
            let gate = gpu.queue().commandBuffer().unwrap();
            let event: &ProtocolObject<dyn MTLEvent> = ProtocolObject::from_ref(&*worker_event);
            gate.encodeWaitForEvent_value(event, 1);
            gate.commit();
            Ok(())
        })
        .unwrap();
        drop(event_owner);
        let device = context.create_device().unwrap();
        let queue = context
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = context
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = context
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let vertices = context
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
                Some(0x1000),
            )
            .unwrap();
        let target = context
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
        let pipeline = context
            .create_graphics_pipeline(
                device,
                include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin"),
                include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin"),
            )
            .unwrap();
        let mut memory = GuestMemory::new();
        let positions: [f32; 12] = [-1., -1., 0.25, 1., 3., -1., 0.25, 1., -1., 3., 0.25, 1.];
        memory
            .map_region_with_data(
                0x1000,
                48,
                MemoryPermissions::READ_WRITE,
                &positions
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        context
            .cmd_set_color_targets(list, &[target], &[[0; 4]])
            .unwrap();
        context
            .cmd_set_graphics_pipeline(list, Some(pipeline))
            .unwrap();
        context
            .cmd_set_vertex_buffer(list, vertices, 0, 16)
            .unwrap();
        context.cmd_set_viewport(list, 4, 3).unwrap();
        context.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
        context.close_command_list(list).unwrap();
        let receipt = context.submit_command_list(queue, list, &memory).unwrap();
        let early = !timeout.load(Ordering::SeqCst);
        let pending = context.poll_submission(receipt).unwrap();
        let prior_frame = context.last_presentation.lock().unwrap().frame_index;
        context.reset_command_list(list, allocator).unwrap();
        context.destroy_resource(vertices).unwrap();
        context.destroy_resource(target).unwrap();
        context.destroy_graphics_pipeline(pipeline).unwrap();
        drop(memory);
        event.setSignaledValue(1);
        assert!(early, "worker acknowledged only after completion");
        assert_eq!(pending, GraphicsSubmissionStatus::Pending);
        assert_eq!(prior_frame, 0);
        assert_eq!(
            context.wait_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        assert_eq!(
            context.last_presentation.lock().unwrap().pixels,
            [255, 0, 0, 255].repeat(12)
        );
        assert!(context.destroy_resource(target).is_err());
        assert_eq!(
            context.poll_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        assert_eq!(context.last_presentation.lock().unwrap().frame_index, 1);
        context.release_submission(receipt).unwrap();
        drop(context);
        done.send(()).unwrap();
        rescue.join().unwrap();
        assert!(!timeout.load(Ordering::SeqCst));
    }
}

fn native_depth_snapshot(snapshot: &GraphicsDepthSnapshot) -> Result<DepthSnapshot, GraphicsError> {
    if let Some(native) = snapshot.downcast_ref::<DepthSnapshot>() {
        return Ok(native.clone());
    }
    let typed = snapshot
        .downcast_ref::<xenora_loader::graphics::GraphicsTypedDepthSnapshot>()
        .ok_or(GraphicsError::InvalidDescriptor)?;
    if typed.desc.format != TextureFormat::Depth32Float
        || typed.desc.mip_levels != 1
        || typed.desc.array_layers.is_some()
    {
        return Err(GraphicsError::InvalidDescriptor);
    }
    DepthSnapshot::from_packed_bytes([typed.desc.extent.0, typed.desc.extent.1], &typed.bytes)
        .map_err(metal_error)
}

fn resource_role_capabilities(
    gpu: &NativeCompute,
) -> xenora_loader::graphics::GraphicsResourceRoleCapabilities {
    xenora_loader::graphics::GraphicsResourceRoleCapabilities {
        sampled_formats: gpu.typed_copy_capabilities(),
        color_formats: 0x1f9,
    }
}
fn native_color_attachment(
    typed: Option<xenora_loader::graphics::GraphicsTypedColorAttachment>,
    clear: [u8; 4],
) -> xenora_metal::TypedColorAttachment {
    if let Some(t) = typed {
        xenora_metal::TypedColorAttachment {
            format: t.format,
            clear: match t.clear {
                xenora_loader::graphics::GraphicsTypedColorClear::Float(v) => {
                    xenora_metal::TypedColorClear::Float(v)
                }
                xenora_loader::graphics::GraphicsTypedColorClear::Uint(v) => {
                    xenora_metal::TypedColorClear::Uint(v)
                }
            },
            blend_enabled: t.blend_enabled,
        }
    } else {
        xenora_metal::TypedColorAttachment {
            format: TextureFormat::Rgba8Unorm,
            clear: xenora_metal::TypedColorClear::Float(clear.map(|v| f64::from(v) / 255.0)),
            blend_enabled: true,
        }
    }
}
fn packed_depth_snapshot(snapshot: DepthSnapshot, extent: (usize, usize)) -> GraphicsDepthSnapshot {
    Arc::new(xenora_loader::graphics::GraphicsTypedDepthSnapshot {
        desc: xenora_loader::graphics::GraphicsTypedTextureDesc {
            format: TextureFormat::Depth32Float,
            extent,
            mip_levels: 1,
            array_layers: None,
        },
        bytes: Arc::from(snapshot.to_packed_bytes()),
    })
}

fn validate_pipeline_formats(
    pipeline: &xenora_loader::graphics::GraphicsPipeline,
    colors: &[TextureFormat],
    textures: [Option<TextureFormat>; 2],
    arrays: [bool; 2],
) -> Result<(), GraphicsError> {
    use xenora_shader::{GraphicsExportType, GraphicsSampleType};
    if colors.len() != pipeline.color_targets
        || colors.iter().enumerate().any(|(i, f)| {
            pipeline.types.colors[i]
                != if *f == TextureFormat::R32Uint {
                    GraphicsExportType::Uint
                } else {
                    GraphicsExportType::Float
                }
        })
    {
        return Err(GraphicsError::InvalidDescriptor);
    }
    for (slot, format) in textures.into_iter().enumerate() {
        if let Some(f) = format {
            let expected = match (f, arrays[slot]) {
                (TextureFormat::R32Uint, true) => GraphicsSampleType::UintArray,
                (TextureFormat::Depth32Float, true) => {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                (_, true) => GraphicsSampleType::FloatArray,
                (TextureFormat::R32Uint, false) => GraphicsSampleType::Uint,
                (TextureFormat::Depth32Float, false) => GraphicsSampleType::Depth,
                (_, false) => GraphicsSampleType::Float,
            };
            if pipeline.types.textures[slot] != expected {
                return Err(GraphicsError::InvalidDescriptor);
            }
        }
    }
    Ok(())
}
