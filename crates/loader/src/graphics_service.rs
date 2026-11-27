// SPDX-License-Identifier: GPL-3.0-or-later
//! Versioned graphics calls for original synthetic guests, not a console ABI.
use crate::cpu::ExitReason;
use crate::graphics::*;
use crate::memory::{GuestMemory, MemoryError};
use xenora_texture::SamplerDesc;

pub const CALL_BYTES: usize = 128;
pub const STATUS_OK: u64 = 0;
pub const STATUS_INVALID_CALL: u64 = 1;
pub const STATUS_GRAPHICS_ERROR: u64 = 2;
pub(crate) const MAX_GUEST_OBJECTS: usize = 4096;
const MAX_GUEST_COMMANDS: usize = 4096;

struct Args([u64; 14]);
impl Args {
    fn handle(&self, index: usize) -> GraphicsHandle {
        GraphicsHandle::from_u64(self.0[index])
    }
    fn optional_handle(&self, index: usize) -> Option<GraphicsHandle> {
        (self.0[index] != 0).then(|| self.handle(index))
    }
    fn u32(&self, index: usize) -> Result<u32, GraphicsError> {
        self.0[index]
            .try_into()
            .map_err(|_| GraphicsError::InvalidDescriptor)
    }
    fn size(&self, index: usize) -> Result<usize, GraphicsError> {
        self.0[index]
            .try_into()
            .map_err(|_| GraphicsError::InvalidDescriptor)
    }
    fn mip_image_location(&self, index: usize) -> Result<GraphicsImageCopyLocation, GraphicsError> {
        let sub = self.0[index + 1];
        match self.0[index] {
            0 => Ok(GraphicsImageCopyLocation::Rgba8Mip {
                mip: sub as u32,
                slice: (sub >> 32) as u32,
                origin: (self.size(index + 2)?, self.size(index + 3)?),
            }),
            1 if sub == 0 => Ok(GraphicsImageCopyLocation::Buffer {
                offset: self.size(index + 2)?,
                bytes_per_row: self.size(index + 3)?,
            }),
            _ => Err(GraphicsError::InvalidDescriptor),
        }
    }
    fn image_location(&self, index: usize) -> Result<GraphicsImageCopyLocation, GraphicsError> {
        match self.0[index] {
            0 => Ok(GraphicsImageCopyLocation::Rgba8 {
                origin: (self.size(index + 1)?, self.size(index + 2)?),
            }),
            1 => Ok(GraphicsImageCopyLocation::Buffer {
                offset: self.size(index + 1)?,
                bytes_per_row: self.size(index + 2)?,
            }),
            _ => Err(GraphicsError::InvalidDescriptor),
        }
    }
    fn flag(&self, index: usize) -> Result<bool, GraphicsError> {
        match self.0[index] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(GraphicsError::InvalidDescriptor),
        }
    }
    fn float(&self, index: usize) -> Result<f32, GraphicsError> {
        Ok(f32::from_bits(self.u32(index)?))
    }
}

fn code(memory: &GuestMemory, address: u64, length: u64) -> Result<Vec<u8>, GraphicsError> {
    if length == 0 || length > 1024 || !length.is_multiple_of(4) {
        return Err(GraphicsError::InvalidDescriptor);
    }
    let mut bytes = vec![0; length as usize];
    memory.read_bytes(address, &mut bytes)?;
    Ok(bytes)
}

/// Execute one fixed-size descriptor. The writable result slot is validated before
/// any object creation, recording, upload, rendering or destruction can take place.
/// Memory faults stop the guest; semantic errors return a status and leave result intact.
pub fn graphics_call_v1(
    graphics: &mut GraphicsContext,
    memory: &mut GuestMemory,
    descriptor: u64,
    bytes: u64,
    result: u64,
) -> Result<u64, ExitReason> {
    if bytes != CALL_BYTES as u64 {
        return Ok(STATUS_INVALID_CALL);
    }
    let mut raw = [0; CALL_BYTES];
    memory
        .read_bytes(descriptor, &mut raw)
        .map_err(ExitReason::MemoryFault)?;
    let words: [u64; 16] =
        std::array::from_fn(|i| u64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap()));
    if words[0] != 1 {
        return Ok(STATUS_INVALID_CALL);
    }
    let opcode = words[1];
    let used = match opcode {
        1 => 0,
        2 | 3 | 12 | 26 | 27 | 31 | 32 | 33 | 36 | 38 | 47 | 60 | 62 | 68 | 70 | 72 => 1,
        4 | 11 | 14 | 25 | 28 | 30 | 34 | 39 | 48 | 66 | 69 | 73 | 75 => 2,
        5 | 7 | 15 | 17 | 29 | 41 | 42 | 56 | 63 | 76 => 3,
        9 | 10 | 13 | 16 | 18 | 19 | 20 | 35 | 46 | 49 | 64 | 67 | 71 => 4,
        6 | 8 | 21 | 23 | 37 | 45 | 61 => 5,
        24 | 40 | 43 | 51 | 52 | 55 | 59 | 65 | 74 => 6,
        44 => 11,
        50 | 54 => 13,
        53 | 57 | 58 => 7,
        22 => 12,
        _ => return Ok(STATUS_INVALID_CALL),
    };
    if words[2 + used..].iter().any(|word| *word != 0) {
        return Ok(STATUS_INVALID_CALL);
    }
    memory
        .validate_write_range(result, 8)
        .map_err(ExitReason::MemoryFault)?;
    if ((1..=8).contains(&opcode)
        || opcode == 37
        || opcode == 40
        || opcode == 45
        || opcode == 46
        || opcode == 49
        || opcode == 51
        || opcode == 52
        || opcode == 53
        || opcode == 55
        || (57..=59).contains(&opcode)
        || opcode == 61
        || opcode == 67
        || opcode == 70
        || opcode == 74)
        && graphics.guest_object_count() >= MAX_GUEST_OBJECTS
    {
        return Ok(STATUS_GRAPHICS_ERROR);
    }
    let args = Args(words[2..].try_into().unwrap());
    if ((13..=24).contains(&opcode)
        || opcode == 28
        || opcode == 29
        || opcode == 34
        || opcode == 39
        || opcode == 41
        || opcode == 42
        || opcode == 43
        || opcode == 44
        || opcode == 48
        || opcode == 50
        || opcode == 54
        || opcode == 56
        || opcode == 63
        || opcode == 64
        || opcode == 66
        || opcode == 69
        || opcode == 73)
        && graphics
            .guest_command_count(args.handle(0))
            .is_some_and(|count| count >= MAX_GUEST_COMMANDS)
    {
        return Ok(STATUS_GRAPHICS_ERROR);
    }
    let operation = (|| -> Result<u64, GraphicsError> {
        let mut value = 0;
        match opcode {
            1 => value = graphics.create_device()?.to_u64(),
            2 => {
                value = graphics
                    .create_command_queue(args.handle(0), CommandListType::Direct)?
                    .to_u64()
            }
            3 => {
                value = graphics
                    .create_command_allocator(args.handle(0), CommandListType::Direct)?
                    .to_u64()
            }
            4 => {
                value = graphics
                    .create_command_list(args.handle(0), args.handle(1), CommandListType::Direct)?
                    .to_u64()
            }
            5 | 49 => {
                value = graphics
                    .create_committed_resource(
                        args.handle(0),
                        ResourceDesc {
                            dimension: ResourceDimension::Buffer,
                            width: args.0[1],
                            height: 1,
                            depth_or_array_size: 1,
                            mip_levels: 1,
                            format_rgba8: false,
                        },
                        ResourceStates(args.u32(2)?),
                        (opcode == 49).then_some(args.0[3]),
                    )?
                    .to_u64()
            }
            6 => {
                value = graphics
                    .create_committed_resource(
                        args.handle(0),
                        ResourceDesc {
                            dimension: ResourceDimension::Texture2D,
                            width: args.0[1],
                            height: args.u32(2)?,
                            depth_or_array_size: 1,
                            mip_levels: args.0[3]
                                .try_into()
                                .map_err(|_| GraphicsError::InvalidDescriptor)?,
                            format_rgba8: true,
                        },
                        ResourceStates(args.u32(4)?),
                        None,
                    )?
                    .to_u64()
            }
            7 => {
                value = graphics
                    .create_depth_resource(args.handle(0), (args.size(1)?, args.size(2)?))?
                    .to_u64()
            }
            8 => {
                let vertex = code(memory, args.0[1], args.0[2])?;
                let pixel = code(memory, args.0[3], args.0[4])?;
                value = graphics
                    .create_graphics_pipeline(args.handle(0), &vertex, &pixel)?
                    .to_u64();
            }
            9 => graphics.copy_guest_to_resource(
                args.handle(0),
                args.size(1)?,
                args.0[2],
                args.size(3)?,
                memory,
            )?,
            10 => {
                let end = args.0[2]
                    .checked_add(args.0[3])
                    .ok_or(MemoryError::Overflow)?;
                let result_end = result
                    .checked_add(8)
                    .ok_or(GraphicsError::InvalidDescriptor)?;
                if args.0[3] != 0 && result < end && args.0[2] < result_end {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                graphics.copy_resource_to_guest(
                    args.handle(0),
                    args.size(1)?,
                    args.0[2],
                    args.size(3)?,
                    memory,
                )?;
            }
            11 => graphics.reset_command_list(args.handle(0), args.handle(1))?,
            12 => graphics.close_command_list(args.handle(0))?,
            13 => graphics.cmd_resource_barrier(
                args.handle(0),
                args.handle(1),
                ResourceStates(args.u32(2)?),
                ResourceStates(args.u32(3)?),
            )?,
            14 => graphics.cmd_set_graphics_pipeline(args.handle(0), args.optional_handle(1))?,
            15 => {
                let count = args.size(1)?;
                if !(1..=8).contains(&count) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut records = vec![0u8; count * 16];
                memory.read_bytes(args.0[2], &mut records)?;
                let mut targets = Vec::with_capacity(count);
                let mut clears = Vec::with_capacity(count);
                for record in records.as_chunks::<16>().0 {
                    let target = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let clear = u64::from_le_bytes(record[8..].try_into().unwrap());
                    let clear: u32 = clear
                        .try_into()
                        .map_err(|_| GraphicsError::InvalidDescriptor)?;
                    targets.push(GraphicsHandle::from_u64(target));
                    clears.push(clear.to_le_bytes());
                }
                graphics.cmd_set_color_targets(args.handle(0), &targets, &clears)?;
            }
            46 => {
                let mip = args.u32(1)?;
                let desc = SamplerDesc {
                    explicit_mip: (mip != u32::MAX).then_some(mip),
                    min_lod: args.float(2)?,
                    max_lod: args.float(3)?,
                    ..SamplerDesc::nearest_clamp()
                };
                value = graphics.create_sampler(args.handle(0), desc)?.to_u64();
            }
            47 => graphics.destroy_sampler(args.handle(0))?,
            48 => {
                let mut bytes = [0u8; 32];
                memory.read_bytes(args.0[1], &mut bytes)?;
                let handles: [Option<GraphicsHandle>; 4] = std::array::from_fn(|i| {
                    let raw = u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
                    (raw != 0).then(|| GraphicsHandle::from_u64(raw))
                });
                graphics.cmd_set_sampled_view_handles(
                    args.handle(0),
                    [handles[0], handles[1]],
                    [handles[2], handles[3]],
                )?;
            }
            34 | 39 | 66 => {
                let mut bytes = [0u8; 80];
                memory.read_bytes(args.0[1], &mut bytes)?;
                let words: [u64; 10] = std::array::from_fn(|i| {
                    u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap())
                });
                let textures = std::array::from_fn(|i| {
                    (words[i] != 0).then(|| GraphicsHandle::from_u64(words[i]))
                });
                let mut samplers = [None; 2];
                for (slot, sampler) in samplers.iter_mut().enumerate() {
                    let start = 2 + slot * 4;
                    match words[start] {
                        0 if words[start + 1..start + 4].iter().all(|word| *word == 0) => {}
                        1 => {
                            let mip = u32::try_from(words[start + 1])
                                .map_err(|_| GraphicsError::InvalidDescriptor)?;
                            let min = u32::try_from(words[start + 2])
                                .map_err(|_| GraphicsError::InvalidDescriptor)?;
                            let max = u32::try_from(words[start + 3])
                                .map_err(|_| GraphicsError::InvalidDescriptor)?;
                            *sampler = Some(SamplerDesc {
                                explicit_mip: (mip != u32::MAX).then_some(mip),
                                min_lod: f32::from_bits(min),
                                max_lod: f32::from_bits(max),
                                ..SamplerDesc::nearest_clamp()
                            });
                        }
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    }
                }
                if opcode == 66 {
                    graphics.cmd_set_sampled_array_ranges(args.handle(0), textures, samplers)?;
                } else if opcode == 39 {
                    graphics.cmd_set_sampled_views(args.handle(0), textures, samplers)?;
                } else {
                    graphics.cmd_set_sampled_bindings(args.handle(0), textures, samplers)?;
                }
            }
            55 => {
                use xenora_shader::{
                    GraphicsExportType, GraphicsPipelineTypes, GraphicsSampleType,
                };
                let bits = args.0[5];
                if bits & !0xff7f != 0 {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut types = GraphicsPipelineTypes::default();
                for (slot, sample_type) in types.textures.iter_mut().enumerate() {
                    *sample_type = match (bits >> (slot * 2)) & 3 {
                        0 => GraphicsSampleType::Float,
                        1 => GraphicsSampleType::Uint,
                        2 => GraphicsSampleType::Depth,
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    };
                    if bits & (1 << (4 + slot)) != 0 {
                        *sample_type = match *sample_type {
                            GraphicsSampleType::Float => GraphicsSampleType::FloatArray,
                            GraphicsSampleType::Uint => GraphicsSampleType::UintArray,
                            _ => return Err(GraphicsError::InvalidDescriptor),
                        };
                    }
                }
                if bits & (1 << 6) != 0 {
                    types.textures[0] = match types.textures[0] {
                        GraphicsSampleType::Float => GraphicsSampleType::StorageFloat,
                        GraphicsSampleType::Uint => GraphicsSampleType::StorageUint,
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    };
                }
                for (slot, export_type) in types.colors.iter_mut().enumerate() {
                    if bits & (1 << (8 + slot)) != 0 {
                        *export_type = GraphicsExportType::Uint;
                    }
                }
                let vertex = code(memory, args.0[1], args.0[2])?;
                let pixel = code(memory, args.0[3], args.0[4])?;
                value = graphics
                    .create_typed_graphics_pipeline(args.handle(0), &vertex, &pixel, types)?
                    .to_u64();
            }
            56 | 63 => {
                let count = args.size(1)?;
                if !(1..=8).contains(&count) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut bytes = [0u8; 512];
                memory.read_bytes(args.0[2], &mut bytes[..count * 64])?;
                let mut targets = Vec::with_capacity(count);
                let mut loads = Vec::with_capacity(count);
                for record in bytes[..count * 64].as_chunks::<64>().0 {
                    let words: [u64; 8] = std::array::from_fn(|i| {
                        u64::from_le_bytes(record[i * 8..i * 8 + 8].try_into().unwrap())
                    });
                    let target = GraphicsHandle::from_u64(words[0]);
                    if words[1] > 1 || words[2] > 1 {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    let clear = match words[3] {
                        0 => GraphicsTypedColorClear::Float(std::array::from_fn(|i| {
                            f64::from_bits(words[i + 4])
                        })),
                        1 => {
                            if words[4..].iter().any(|v| *v > u64::from(u32::MAX)) {
                                return Err(GraphicsError::InvalidDescriptor);
                            }
                            GraphicsTypedColorClear::Uint(std::array::from_fn(|i| {
                                words[i + 4] as u32
                            }))
                        }
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    };
                    let format = if opcode == 63 {
                        graphics.writable_selection(target)?.desc.format
                    } else {
                        graphics.sampled_resource_desc(target)?.format
                    };
                    targets.push(target);
                    loads.push(GraphicsColorLoad::Typed {
                        attachment: GraphicsTypedColorAttachment {
                            format,
                            clear,
                            blend_enabled: words[2] != 0,
                        },
                        load: words[1] != 0,
                    });
                }
                if opcode == 63 {
                    graphics.cmd_set_writable_color_targets(args.handle(0), &targets, &loads)?;
                } else {
                    graphics.cmd_set_color_targets_with_loads(args.handle(0), &targets, &loads)?;
                }
            }
            53 => {
                let format = match args.u32(5)? {
                    0 => xenora_texture::TextureFormat::Rgba8Unorm,
                    1 => xenora_texture::TextureFormat::Bc1RgbaUnorm,
                    2 => xenora_texture::TextureFormat::Bc3RgbaUnorm,
                    3 => xenora_texture::TextureFormat::R8Unorm,
                    4 => xenora_texture::TextureFormat::Rg8Unorm,
                    5 => xenora_texture::TextureFormat::Bgra8Unorm,
                    6 => xenora_texture::TextureFormat::Rgba16Float,
                    7 => xenora_texture::TextureFormat::R32Float,
                    8 => xenora_texture::TextureFormat::R32Uint,
                    9 => xenora_texture::TextureFormat::Depth32Float,
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                let layers = args.u32(4)?;
                value = graphics
                    .create_typed_texture_resource(
                        args.handle(0),
                        crate::graphics::GraphicsTypedTextureDesc {
                            format,
                            extent: (args.size(1)?, args.size(2)?),
                            mip_levels: args.u32(3)?,
                            array_layers: (layers != 0).then_some(layers),
                        },
                        ResourceStates(args.u32(6)?),
                    )?
                    .to_u64();
            }
            54 => {
                let location = |index| -> Result<GraphicsImageCopyLocation, GraphicsError> {
                    match args.mip_image_location(index)? {
                        GraphicsImageCopyLocation::Rgba8Mip { mip, slice, origin } => {
                            Ok(GraphicsImageCopyLocation::Texture { mip, slice, origin })
                        }
                        value @ GraphicsImageCopyLocation::Buffer { .. } => Ok(value),
                        _ => Err(GraphicsError::InvalidDescriptor),
                    }
                };
                graphics.cmd_copy_typed_image(
                    args.handle(0),
                    args.handle(1),
                    location(2)?,
                    args.handle(6),
                    location(7)?,
                    (args.size(11)?, args.size(12)?),
                )?;
            }
            61 => {
                let role = args.u32(4)?;
                if role > 1 {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                value = graphics
                    .create_writable_attachment_view(
                        args.handle(0),
                        args.handle(1),
                        args.u32(2)?,
                        args.u32(3)?,
                        role == 1,
                    )?
                    .to_u64();
            }
            62 => graphics.destroy_writable_attachment_view(args.handle(0))?,
            64 => {
                let load = match args.0[2] {
                    0 => GraphicsDepthLoad::Clear(f32::from_bits(args.u32(3)?)),
                    1 if args.0[3] == 0 => GraphicsDepthLoad::Load,
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                graphics.cmd_set_writable_depth_target(
                    args.handle(0),
                    (args.0[1] != 0).then(|| args.handle(1)),
                    load,
                )?;
            }
            57 | 58 => {
                let desc = crate::graphics::GraphicsArrayRangeViewDesc {
                    base_slice: args.u32(2)?,
                    slice_count: args.u32(3)?,
                    base_mip: args.u32(4)?,
                    mip_count: args.u32(5)?,
                    format: view_format(args.0[6])?,
                };
                value = if opcode == 57 {
                    graphics.create_array_range_view(args.handle(0), args.handle(1), desc)?
                } else {
                    graphics.create_child_array_range_view(args.handle(0), args.handle(1), desc)?
                }
                .to_u64();
            }
            59 => {
                value = graphics
                    .create_array_range_slice_view(
                        args.handle(0),
                        args.handle(1),
                        GraphicsArraySliceViewDesc {
                            slice: args.u32(2)?,
                            base_mip: args.u32(3)?,
                            mip_count: args.u32(4)?,
                            format: view_format(args.0[5])?,
                        },
                    )?
                    .to_u64();
            }
            60 => graphics.destroy_array_range_view(args.handle(0))?,
            52 => {
                let format = view_format(args.0[5])?;
                value = graphics
                    .create_array_slice_view(
                        args.handle(0),
                        args.handle(1),
                        GraphicsArraySliceViewDesc {
                            slice: args.u32(2)?,
                            base_mip: args.u32(3)?,
                            mip_count: args.u32(4)?,
                            format,
                        },
                    )?
                    .to_u64();
            }
            51 => {
                value = graphics
                    .create_texture_array_resource(
                        args.handle(0),
                        GraphicsTextureArrayDesc {
                            width: args.size(1)?,
                            height: args.u32(2)? as usize,
                            mip_levels: args.u32(3)?,
                            array_layers: args.u32(4)?,
                        },
                        ResourceStates(args.u32(5)?),
                    )?
                    .to_u64();
            }
            50 => {
                graphics.cmd_copy_image_subresources(
                    args.handle(0),
                    args.handle(1),
                    args.mip_image_location(2)?,
                    args.handle(6),
                    args.mip_image_location(7)?,
                    (args.size(11)?, args.size(12)?),
                )?;
            }
            44 => {
                graphics.cmd_copy_image(
                    args.handle(0),
                    args.handle(1),
                    args.image_location(2)?,
                    args.handle(5),
                    args.image_location(6)?,
                    (args.size(9)?, args.size(10)?),
                )?;
            }
            43 => {
                graphics.cmd_copy_buffer_region(
                    args.handle(0),
                    args.handle(1),
                    args.size(2)?,
                    args.handle(3),
                    args.size(4)?,
                    args.size(5)?,
                )?;
            }
            42 => {
                graphics.cmd_copy_resource(args.handle(0), args.handle(1), args.handle(2))?;
            }
            41 => {
                let present = args.flag(2)?;
                let values = if present {
                    let mut bytes = [0u8; 32];
                    memory.read_bytes(args.0[1], &mut bytes)?;
                    Some(std::array::from_fn(|i| {
                        u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
                    }))
                } else {
                    if args.0[1] != 0 {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    None
                };
                graphics.cmd_set_shader_uniforms(args.handle(0), values)?;
            }
            40 => {
                let format = match args.0[4] {
                    0 => xenora_texture::TextureFormat::Rgba8Unorm,
                    1 => xenora_texture::TextureFormat::Bc1RgbaUnorm,
                    2 => xenora_texture::TextureFormat::Bc3RgbaUnorm,
                    3 => xenora_texture::TextureFormat::R8Unorm,
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                value = graphics
                    .create_texture_resource(
                        args.handle(0),
                        GraphicsTextureDesc {
                            width: args.size(1)?,
                            height: args.u32(2)? as usize,
                            mip_levels: args.u32(3)?,
                            format,
                        },
                        ResourceStates(args.u32(5)?),
                    )?
                    .to_u64();
            }
            37 | 45 => {
                let format = view_format(args.0[4])?;
                let desc = GraphicsTextureViewDesc {
                    base_mip: args.u32(2)?,
                    mip_count: args.u32(3)?,
                    format,
                };
                value = if opcode == 37 {
                    graphics.create_texture_view(args.handle(0), args.handle(1), desc)?
                } else {
                    graphics.create_child_texture_view(args.handle(0), args.handle(1), desc)?
                }
                .to_u64();
            }
            38 => graphics.destroy_texture_view(args.handle(0))?,
            70 => value = graphics.create_descriptor_table(args.handle(0))?.to_u64(),
            71 => {
                let first = args.size(1)?;
                let count = args.size(2)?;
                if count == 0 || first.checked_add(count).is_none_or(|n| n > 8) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut bytes = [0u8; 128];
                memory.read_bytes(args.0[3], &mut bytes[..count * 16])?;
                let mut entries = Vec::with_capacity(count);
                for record in bytes[..count * 16].as_chunks::<16>().0 {
                    let kind = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let raw = u64::from_le_bytes(record[8..].try_into().unwrap());
                    let h = GraphicsHandle::from_u64(raw);
                    entries.push(match kind {
                        0 if raw == 0 => GraphicsDescriptorEntry::Null,
                        1 if raw != 0 => GraphicsDescriptorEntry::TextureView(h),
                        2 if raw != 0 => GraphicsDescriptorEntry::ArrayRange(h),
                        3 if raw != 0 => GraphicsDescriptorEntry::Sampler(h),
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    });
                }
                value = graphics.update_descriptor_table(args.handle(0), first, &entries)?;
            }
            72 => graphics.destroy_descriptor_table(args.handle(0))?,
            73 => graphics.cmd_set_descriptor_table(
                args.handle(0),
                (args.0[1] != 0).then(|| args.handle(1)),
            )?,
            74 => {
                let mut bytes = [0u8; 96];
                memory.read_bytes(args.0[5], &mut bytes)?;
                let mut types = xenora_shader::GraphicsPipelineTypes4::default();
                for (i, b) in bytes.as_chunks::<8>().0.iter().enumerate() {
                    let kind = u64::from_le_bytes(*b);
                    if i < 4 {
                        use xenora_shader::GraphicsSampleType as T;
                        types.textures[i] = match kind {
                            0 => T::Float,
                            1 => T::Uint,
                            2 => T::Depth,
                            3 => T::FloatArray,
                            4 => T::UintArray,
                            5 if i == 0 => T::StorageFloat,
                            6 if i == 0 => T::StorageUint,
                            _ => return Err(GraphicsError::InvalidDescriptor),
                        };
                    } else {
                        types.colors[i - 4] = match kind {
                            0 => xenora_shader::GraphicsExportType::Float,
                            1 => xenora_shader::GraphicsExportType::Uint,
                            _ => return Err(GraphicsError::InvalidDescriptor),
                        };
                    }
                }
                let vertex = code(memory, args.0[1], args.0[2])?;
                let pixel = code(memory, args.0[3], args.0[4])?;
                value = graphics
                    .create_typed_graphics_pipeline4(args.handle(0), &vertex, &pixel, types)?
                    .to_u64();
            }
            75 => graphics.bind_guest_memory(args.handle(0), args.0[1], memory)?,
            76 => {
                let count = args.size(1)?;
                if !(1..=16).contains(&count) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let import = args.flag(2)?;
                let mut raw = vec![0; count * 8];
                memory.read_bytes(args.0[0], &mut raw)?;
                let roots: Vec<_> = raw
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|b| GraphicsHandle::from_u64(u64::from_le_bytes(*b)))
                    .collect();
                if !import {
                    let result_end = result
                        .checked_add(8)
                        .ok_or(GraphicsError::InvalidDescriptor)?;
                    for (address, length) in graphics.guest_sync_ranges(&roots)? {
                        if address < result_end && result < address + length as u64 {
                            return Err(GraphicsError::InvalidDescriptor);
                        }
                    }
                }
                graphics.synchronize_guest_memory(&roots, import, memory)?;
            }
            67 => {
                value = graphics
                    .create_storage_view(
                        args.handle(0),
                        args.handle(1),
                        args.u32(2)?,
                        args.u32(3)?,
                    )?
                    .to_u64();
            }
            68 => graphics.destroy_storage_view(args.handle(0))?,
            69 => graphics
                .cmd_set_storage_view(args.handle(0), (args.0[1] != 0).then(|| args.handle(1)))?,
            65 => {
                let semantic_count = args.size(3)?;
                let composition_count = args.size(5)?;
                if semantic_count
                    .checked_add(composition_count)
                    .is_none_or(|n| n > MAX_QUEUED_GRAPH_NODES)
                    || (semantic_count == 0 && args.0[2] != 0)
                    || (composition_count == 0 && args.0[4] != 0)
                {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let read_waits =
                    |address, count| -> Result<Vec<GraphicsQueuePoint>, GraphicsError> {
                        let mut bytes = [0u8; MAX_QUEUED_GRAPH_NODES * 16];
                        if count != 0 {
                            memory.read_bytes(address, &mut bytes[..count * 16])?;
                        }
                        Ok(bytes[..count * 16]
                            .as_chunks::<16>()
                            .0
                            .iter()
                            .map(|record| GraphicsQueuePoint {
                                queue: GraphicsHandle::from_u64(u64::from_le_bytes(
                                    record[..8].try_into().unwrap(),
                                )),
                                value: u64::from_le_bytes(record[8..].try_into().unwrap()),
                            })
                            .collect())
                    };
                let semantic = read_waits(args.0[2], semantic_count)?;
                let composition = read_waits(args.0[4], composition_count)?;
                value = graphics
                    .submit_command_list_after_composed(
                        args.handle(0),
                        args.handle(1),
                        &semantic,
                        &composition,
                        memory,
                    )?
                    .to_u64();
            }
            35 => {
                let count = args.size(3)?;
                if count > MAX_QUEUED_GRAPH_NODES || (count == 0 && args.0[2] != 0) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut records = [0u8; MAX_QUEUED_GRAPH_NODES * 16];
                if count != 0 {
                    memory.read_bytes(args.0[2], &mut records[..count * 16])?;
                }
                let waits: Vec<_> = records[..count * 16]
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .map(|record| GraphicsQueuePoint {
                        queue: GraphicsHandle::from_u64(u64::from_le_bytes(
                            record[..8].try_into().unwrap(),
                        )),
                        value: u64::from_le_bytes(record[8..].try_into().unwrap()),
                    })
                    .collect();
                value = graphics
                    .submit_command_list_after(args.handle(0), args.handle(1), &waits, memory)?
                    .to_u64();
            }
            36 => value = graphics.submission_point(args.handle(0))?.value,
            30 => {
                value = graphics
                    .submit_command_list(args.handle(0), args.handle(1), memory)?
                    .to_u64()
            }
            31 | 32 => {
                let status = if opcode == 31 {
                    graphics.poll_submission(args.handle(0))?
                } else {
                    graphics.wait_submission(args.handle(0))?
                };
                value = match status {
                    GraphicsSubmissionStatus::Pending => 0,
                    GraphicsSubmissionStatus::Complete => 1,
                    GraphicsSubmissionStatus::Failed => 2,
                };
            }
            33 => graphics.release_submission(args.handle(0))?,
            29 => {
                let count = args.size(1)?;
                if !(1..=8).contains(&count) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let mut records = vec![0u8; count * 24];
                memory.read_bytes(args.0[2], &mut records)?;
                let mut targets = Vec::with_capacity(count);
                let mut loads = Vec::with_capacity(count);
                for record in records.as_chunks::<24>().0 {
                    let target = u64::from_le_bytes(record[..8].try_into().unwrap());
                    let mode = u64::from_le_bytes(record[8..16].try_into().unwrap());
                    let clear = u64::from_le_bytes(record[16..].try_into().unwrap());
                    let load = match mode {
                        0 => GraphicsColorLoad::Clear(
                            u32::try_from(clear)
                                .map_err(|_| GraphicsError::InvalidDescriptor)?
                                .to_le_bytes(),
                        ),
                        1 if clear == 0 => GraphicsColorLoad::Load,
                        _ => return Err(GraphicsError::InvalidDescriptor),
                    };
                    targets.push(GraphicsHandle::from_u64(target));
                    loads.push(load);
                }
                graphics.cmd_set_color_targets_with_loads(args.handle(0), &targets, &loads)?;
            }
            16 => {
                let load = match args.0[2] {
                    0 => GraphicsDepthLoad::Clear(args.float(3)?),
                    1 if args.0[3] == 0 => GraphicsDepthLoad::Load,
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                graphics.cmd_set_depth_target(args.handle(0), args.optional_handle(1), load)?;
            }
            17 => graphics.cmd_set_viewport(args.handle(0), args.size(1)?, args.size(2)?)?,
            18 => graphics.cmd_set_vertex_buffer(
                args.handle(0),
                args.handle(1),
                args.size(2)?,
                args.size(3)?,
            )?,
            19 => graphics.cmd_set_vertex_colors(
                args.handle(0),
                args.handle(1),
                args.size(2)?,
                args.size(3)?,
            )?,
            20 => graphics.cmd_set_index_buffer(
                args.handle(0),
                args.handle(1),
                args.size(2)?,
                args.u32(3)?,
            )?,
            21 => {
                let mip = args.u32(2)?;
                let sampler = SamplerDesc {
                    explicit_mip: (mip != u32::MAX).then_some(mip),
                    min_lod: args.float(3)?,
                    max_lod: args.float(4)?,
                    ..SamplerDesc::nearest_clamp()
                };
                graphics.cmd_set_sampled_texture(
                    args.handle(0),
                    args.optional_handle(1),
                    sampler,
                )?;
            }
            22 => {
                let compare = match args.0[1] {
                    0 => None,
                    1 => Some(GraphicsDepthCompare::Never),
                    2 => Some(GraphicsDepthCompare::Less),
                    3 => Some(GraphicsDepthCompare::Equal),
                    4 => Some(GraphicsDepthCompare::LessEqual),
                    5 => Some(GraphicsDepthCompare::Greater),
                    6 => Some(GraphicsDepthCompare::NotEqual),
                    7 => Some(GraphicsDepthCompare::GreaterEqual),
                    8 => Some(GraphicsDepthCompare::Always),
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                let blend = match args.0[3] {
                    0 => GraphicsBlendMode::Replace,
                    1 => GraphicsBlendMode::Alpha,
                    2 => GraphicsBlendMode::Additive,
                    _ => return Err(GraphicsError::InvalidDescriptor),
                };
                let scissor = if args.flag(7)? {
                    Some([args.size(8)?, args.size(9)?, args.size(10)?, args.size(11)?])
                } else {
                    if args.0[8..12].iter().any(|word| *word != 0) {
                        return Err(GraphicsError::InvalidDescriptor);
                    }
                    None
                };
                graphics.cmd_set_draw_state(
                    args.handle(0),
                    GraphicsDrawState {
                        depth_compare: compare,
                        depth_write: args.flag(2)?,
                        blend,
                        color_write_mask: args.0[4]
                            .try_into()
                            .map_err(|_| GraphicsError::InvalidDescriptor)?,
                        viewport_origin: [args.size(5)?, args.size(6)?],
                        scissor,
                    },
                )?;
            }
            23 => graphics.cmd_draw_instanced(
                args.handle(0),
                args.u32(1)?,
                args.u32(2)?,
                args.u32(3)?,
                args.u32(4)?,
            )?,
            24 => graphics.cmd_draw_indexed_instanced(
                args.handle(0),
                args.u32(1)?,
                args.u32(2)?,
                args.u32(3)?,
                (args.0[4] as i64)
                    .try_into()
                    .map_err(|_| GraphicsError::InvalidDescriptor)?,
                args.u32(5)?,
            )?,
            25 => {
                value = graphics
                    .execute_command_list(args.handle(0), args.handle(1), memory)?
                    .draw_count as u64
            }
            26 => graphics.destroy_resource(args.handle(0))?,
            27 => graphics.destroy_graphics_pipeline(args.handle(0))?,
            28 => graphics.cmd_set_depth_clear(args.handle(0), args.float(1)?)?,
            _ => unreachable!("validated opcode"),
        }
        Ok(value)
    })();
    match operation {
        Ok(value) => {
            memory
                .write_u64(result, value)
                .map_err(ExitReason::MemoryFault)?;
            Ok(STATUS_OK)
        }
        Err(GraphicsError::Memory(error)) if error != MemoryError::AllocationFailed => {
            Err(ExitReason::MemoryFault(error))
        }
        Err(_) => Ok(STATUS_GRAPHICS_ERROR),
    }
}

fn view_format(code: u64) -> Result<GraphicsTextureViewFormat, GraphicsError> {
    use GraphicsTextureViewFormat::*;
    Ok(match code {
        0 => Rgba8Unorm,
        1 => Rgba8UnormSrgb,
        2 => Bc1RgbaUnorm,
        3 => Bc1RgbaUnormSrgb,
        4 => Bc3RgbaUnorm,
        5 => Bc3RgbaUnormSrgb,
        6 => R8Unorm,
        7 => Rg8Unorm,
        8 => Bgra8Unorm,
        9 => Bgra8UnormSrgb,
        10 => Rgba16Float,
        11 => R32Float,
        12 => R32Uint,
        13 => Depth32Float,
        _ => return Err(GraphicsError::InvalidDescriptor),
    })
}
