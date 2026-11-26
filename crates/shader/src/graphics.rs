// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt::Write;

mod cfg;

use xenora_decoder::{
    DecodedInstruction, ExportTarget, Instruction, InterpolationPhase, ScalarAluOp,
    ScalarCompareOp, ScalarSource, VectorSource,
};

use super::{Architecture, Binary, CompileError, CompiledShader, Value, decode_fixture};

/// Entry point of the translated vertex stage.
pub const VERTEX_ENTRY_POINT: &str = "xenora_vertex";
/// Entry point of the translated pixel stage.
pub const PIXEL_ENTRY_POINT: &str = "xenora_pixel";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Vertex,
    Pixel,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Pixel => "pixel",
        }
    }
}

#[derive(Clone, Copy)]
enum Float4Input {
    Position,
    Color,
}

#[derive(Clone, Copy)]
enum Op {
    VertexInput(Float4Input, u8),
    Compare(ScalarCompareOp, ScalarValue, ScalarValue),
    ScalarResult(ScalarAluOp, ScalarValue, ScalarValue, Condition),
    ScalarFlag(ScalarAluOp, ScalarValue, ScalarValue, Condition, Value),
    Phi(ValueType),
    Constant(u32),
    UniformFloat(u8),
    PixelCoordinate(u8),
    // Ordered effect: occupies an IR stream position but defines no Value.
    Store2D {
        coordinates: [Value; 2],
        data: [Option<Value>; 4],
        dmask: u8,
    },
    Interpolate(u8),
    Binary(Binary, Value, Value),
    SampleLodZero {
        u: Value,
        v: Value,
        layer: Option<(Value, GraphicsArrayLayer)>,
        component: u8,
        texture_index: u8,
        sampler_index: u8,
    },
}

struct Export {
    offset: usize,
    values: [Value; 4],
}

#[derive(Clone, Copy)]
enum ValueType {
    U32,
    Bool,
    F32,
}

#[derive(Clone, Copy)]
enum ScalarValue {
    Constant(u32),
    Uniform(u8),
    Runtime(Value),
}
impl ScalarValue {
    fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Constant(a), Self::Constant(b)) => a == b,
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (Self::Runtime(a), Self::Runtime(b)) => a.0 == b.0,
            _ => false,
        }
    }
    fn expression(self) -> String {
        match self {
            Self::Constant(bits) => format!("0x{bits:08x}u"),
            Self::Uniform(slot) => format!("uniforms[{slot}]"),
            Self::Runtime(value) => value.to_string(),
        }
    }
}

#[derive(Clone, Copy)]
enum EdgeValue {
    Float(Value),
    Uint(ScalarValue),
    Bool(Condition),
}
impl EdgeValue {
    fn expression(self) -> String {
        match self {
            Self::Float(value) => value.to_string(),
            Self::Uint(value) => value.expression(),
            Self::Bool(value) => value.expression(),
        }
    }
}

#[derive(Clone, Copy)]
enum Condition {
    Constant(bool),
    Runtime(Value),
}

impl Condition {
    fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Constant(a), Self::Constant(b)) => a == b,
            (Self::Runtime(a), Self::Runtime(b)) => a.0 == b.0,
            _ => false,
        }
    }

    fn expression(self) -> String {
        match self {
            Self::Constant(value) => value.to_string(),
            Self::Runtime(value) => value.to_string(),
        }
    }
}

#[derive(Clone)]
struct RegisterState {
    registers: [Option<Value>; 256],
    scalars: [Option<ScalarValue>; 106],
    scc: Option<Condition>,
    // Exact last export instruction, not a union of target bits. Equality at
    // every join inductively requires the same export suffix on every path.
    last_export: Option<usize>,
    pixel_done: bool,
}

struct Block {
    values: std::ops::Range<usize>,
    // SCC=true takes fallthrough, SCC=false takes the target.
    successors: Vec<usize>,
    condition: Option<Value>,
    edge_copies: Vec<(usize, Vec<(Value, EdgeValue)>)>,
}

#[derive(Default)]
struct StageIr {
    values: Vec<(Option<usize>, Op)>,
    scalar_upper_bounds: Vec<(Value, u32)>,
    signed_scalar_upper_bounds: Vec<(Value, i32)>,
    blocks: Vec<Option<Block>>,
    initial_values: usize,
    uniform_mask: u8,
    runtime_control: bool,
    position: Option<Export>,
    parameter: Option<Export>,
    colors: [Option<Export>; 8],
}

impl StageIr {
    fn push(&mut self, offset: Option<usize>, op: Op) -> Value {
        let value = Value(self.values.len());
        self.values.push((offset, op));
        value
    }
}

/// Element interpretation required by a synthetic sampled texture binding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphicsSampleType {
    #[default]
    Float,
    Uint,
    Depth,
    FloatArray,
    UintArray,
    StorageFloat,
    StorageUint,
}

/// Required slot0 storage format and unique-writer address contract.
/// Some storage metadata certifies that every reachable store uses the original
/// synthetic pixel X/Y uint bits (v32/v33), possibly through direct VGPR copies.
/// The loader and trusted native callers must independently enforce one draw of
/// one three-vertex, non-instanced triangle, single sampling, no depth, viewport
/// origin zero and extent equal to the selected 2D storage/color extent. Sampled
/// inputs must not alias the storage root. This is not a general storage ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsStorageType {
    Float,
    Uint,
}

fn storage_type(types: GraphicsPipelineTypes) -> Option<GraphicsStorageType> {
    match types.textures[0] {
        GraphicsSampleType::StorageFloat => Some(GraphicsStorageType::Float),
        GraphicsSampleType::StorageUint => Some(GraphicsStorageType::Uint),
        _ => None,
    }
}

/// A view-relative array layer requirement, validated before every draw.
/// Uniform indices name FP32 bit patterns in uniform words 0..7 (s24..s31).
/// The loader, or a trusted caller using generated MSL directly, must validate
/// every requirement against the actual bound array view before dispatch. Uniform
/// values must be finite, integral, nonnegative, representable as u32 and smaller
/// than the view layer count. Constants must also be smaller than that count.
/// Compilation alone does not validate runtime bindings; the shader's uint cast
/// relies on this precondition and must not be used to truncate arbitrary floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphicsArrayLayer {
    Constant(u32),
    Uniform(u8),
}

/// Fragment export representation; Uint preserves the source VGPR bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GraphicsExportType {
    #[default]
    Float,
    Uint,
}

/// Explicit synthetic resource types; ordinary pipelines retain float bindings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GraphicsPipelineTypes {
    pub textures: [GraphicsSampleType; 2],
    pub colors: [GraphicsExportType; 8],
}

/// Compiled graphics stages and resource requirements derived from their lifted IR.
#[derive(Debug)]
pub struct CompiledGraphics {
    pub types: GraphicsPipelineTypes,
    /// Slot0 storage requirement, including proven unique pixel-X/Y addresses.
    /// See [`GraphicsStorageType`] for mandatory host geometry/binding constraints.
    pub storage: Option<GraphicsStorageType>,
    /// Generated shader source, disassembly and intermediate representation.
    pub shader: CompiledShader,
    /// Number of contiguous color attachments starting at MRT0 (1..=8).
    pub color_targets: usize,
    /// Whether the pixel stage requires any synthetic texture/sampler binding.
    pub uses_texture: bool,
    /// Required pairs: bit (texture_slot * 2 + sampler_slot), for slots 0 and 1.
    pub sampled_pairs: u8,
    /// Deduplicated layer requirements per texture across all reachable samples.
    /// See [`GraphicsArrayLayer`] for mandatory per-draw binding validation.
    pub array_layers: [Vec<GraphicsArrayLayer>; 2],
    /// Uniform words read by the vertex stage; bit zero denotes s24.
    pub vertex_uniform_mask: u8,
    /// Uniform words read by the pixel stage; bit zero denotes s24.
    pub pixel_uniform_mask: u8,
    /// Union of vertex and pixel uniform requirements.
    pub required_uniform_mask: u8,
}

/// Compiles graphics stages, returning only the shader for existing callers.
/// See [`compile_graphics_pipeline`] for the synthetic ABI and resource metadata.
pub fn compile_graphics(
    architecture: Architecture,
    vertex: &[u8],
    pixel: &[u8],
) -> Result<CompiledShader, CompileError> {
    compile_graphics_pipeline(architecture, vertex, pixel).map(|compiled| compiled.shader)
}

/// Compiles a linked synthetic gfx1030 pipeline with 1..=8 contiguous RGBA8 outputs.
///
/// Vertex VGPRs 0–3 receive one clip-space position from buffer 0 and VGPRs 4–7
/// receive one color from buffer 1. Each buffer contains one float4 per
/// vertex. Pixel VGPRs 0 and 1 are reserved for the I and J barycentric
/// coordinates. Adjacent interpolation pairs for PARAM0 lower to Metal's
/// center-perspective interpolation under this ABI. Vertex and pixel FP32 addition and
/// multiplication use separate operations and require the backend's safe and
/// precise Metal math modes. Uniform u32 words at buffer 2 initialize s24..s31
/// independently in each stage. SCC0 branches use verified control flow and typed
/// joins. Bounded unsigned and signed-positive countdowns, including nesting and exits
/// to the innermost continuation, execute real iterations.
/// Constant-dead edges are pruned; runtime edges contribute resource requirements.
/// The full stream must decode and paths must share an export epilogue. General
/// loop shapes and wave-mask divergence remain unsupported.
pub fn compile_graphics_pipeline(
    architecture: Architecture,
    vertex: &[u8],
    pixel: &[u8],
) -> Result<CompiledGraphics, CompileError> {
    compile_graphics_pipeline_typed(
        architecture,
        vertex,
        pixel,
        GraphicsPipelineTypes::default(),
    )
}

/// Compile explicit unsigned-integer, raw-depth and floating-point resource roles.
/// Integer sampling uses nearest samplers supplied by the backend. Values travel
/// as VGPR bit patterns; floating arithmetic on sampled integer data is rejected.
/// Raw depth maps to (depth, 0, 0, 1) in the synthetic descriptor contract.
/// Array sampling requires matching FloatArray/UintArray types and emits binding
/// requirements in [`CompiledGraphics::array_layers`]. The loader and trusted
/// direct native callers must validate these before every draw, including all
/// reachable branch arms, before the generated shader converts FP32 layers to uint.
/// Depth arrays and unproven computed/interpolated layers are unsupported.
/// StorageFloat/StorageUint reserve texture0 for ordered 2D stores and initialize
/// v32/v33 with pixel-coordinate uint bits. See [`GraphicsStorageType`]; compilation
/// proves coordinate provenance but cannot validate the host draw or aliases.
pub fn compile_graphics_pipeline_typed(
    architecture: Architecture,
    vertex: &[u8],
    pixel: &[u8],
    types: GraphicsPipelineTypes,
) -> Result<CompiledGraphics, CompileError> {
    if matches!(
        types.textures[1],
        GraphicsSampleType::StorageFloat | GraphicsSampleType::StorageUint
    ) {
        return Err(CompileError::GraphicsLink {
            reason: "storage is restricted to texture slot0",
        });
    }
    let storage = storage_type(types);
    let compile_stage = |stage: Stage, bytes: &[u8]| {
        let result = decode_fixture(architecture, bytes).and_then(|decoded| {
            let lifted = lift(stage, &decoded, bytes.len(), storage.is_some())?;
            Ok((decoded, lifted))
        });
        result.map_err(|error| CompileError::GraphicsStage {
            stage: stage.name(),
            error: Box::new(error),
        })
    };
    let (vertex_decoded, vertex) = compile_stage(Stage::Vertex, vertex)?;
    let (pixel_decoded, pixel) = compile_stage(Stage::Pixel, pixel)?;
    let position = vertex.position.as_ref().ok_or(CompileError::GraphicsLink {
        reason: "vertex stage requires a POS0 export",
    })?;
    if pixel.colors[0].is_none() {
        return Err(CompileError::GraphicsLink {
            reason: "pixel stage requires an MRT0 export",
        });
    }
    let color_count = pixel.colors.iter().rposition(Option::is_some).unwrap() + 1;
    if pixel.colors[..color_count].iter().any(Option::is_none) {
        return Err(CompileError::GraphicsLink {
            reason: "pixel MRT exports must be contiguous from MRT0",
        });
    }
    let parameter = vertex.parameter.as_ref();
    let interpolates = pixel
        .values
        .iter()
        .any(|(_, op)| matches!(op, Op::Interpolate(_)));
    if parameter.is_some() != interpolates {
        return Err(CompileError::GraphicsLink {
            reason: "vertex PARAM0 export and pixel attr0 interpolation must both be present or absent",
        });
    }

    let sampled_pairs = pixel.values.iter().fold(0u8, |mask, (_, op)| {
        if let Op::SampleLodZero {
            texture_index,
            sampler_index,
            ..
        } = op
        {
            mask | (1 << (texture_index * 2 + sampler_index))
        } else {
            mask
        }
    });
    validate_resource_types(&pixel, types, sampled_pairs, color_count)?;
    let mut array_layers: [Vec<GraphicsArrayLayer>; 2] = Default::default();
    for (_, op) in &pixel.values {
        if let Op::SampleLodZero {
            texture_index,
            layer: Some((_, layer)),
            ..
        } = op
        {
            let requirements = &mut array_layers[usize::from(*texture_index)];
            if !requirements.contains(layer) {
                requirements.push(*layer);
            }
        }
    }
    let has_sampling = sampled_pairs != 0;

    let mut disassembly = String::new();
    for (name, instructions) in [("vertex", vertex_decoded), ("pixel", pixel_decoded)] {
        let _ = writeln!(disassembly, "{name}:");
        for instruction in instructions {
            let _ = writeln!(disassembly, "{instruction}");
        }
    }

    let mut ir = String::from("vertex:\n");
    let mut msl = String::from(
        "#include <metal_stdlib>\nusing namespace metal;\n\n\
         struct VertexOutput {\n    float4 position [[position]];\n",
    );
    if array_layers.iter().any(|layers| !layers.is_empty()) {
        msl.insert_str(0, "// Host precondition: validate CompiledGraphics.array_layers for every draw.\n// Uniform layer bits must be finite nonnegative integers within the bound array view.\n");
    }
    if parameter.is_some() {
        msl.push_str("    float4 parameter [[user(locn0), center_perspective]];\n");
    }
    let vertex_uniforms = if vertex.uniform_mask != 0 {
        ",\n    constant uint* uniforms [[buffer(2)]]"
    } else {
        ""
    };
    let _ = write!(
        msl,
        "}};\n\nvertex VertexOutput {VERTEX_ENTRY_POINT}(\n    uint vertex_id [[vertex_id]],\n    device const float4* positions [[buffer(0)]],\n    device const float4* colors [[buffer(1)]]{vertex_uniforms}) {{\n"
    );
    emit_fp_controls(&vertex, &mut msl);
    emit_values(&vertex, GraphicsPipelineTypes::default(), &mut ir, &mut msl);
    msl.push_str("    VertexOutput output;\n");
    emit_export(position, "pos0", "output.position", &mut ir, &mut msl);
    if let Some(parameter) = parameter {
        emit_export(parameter, "param0", "output.parameter", &mut ir, &mut msl);
    }
    msl.push_str("    return output;\n}\n\nstruct PixelOutput {\n");
    for (slot, export_type) in types.colors.iter().enumerate().take(color_count) {
        let kind = if *export_type == GraphicsExportType::Uint {
            "uint4"
        } else {
            "float4"
        };
        let _ = writeln!(msl, "    {kind} color{slot} [[color({slot})]];");
    }
    msl.push_str("};\n\n");
    let mut params = Vec::new();
    if let Some(storage) = storage {
        let kind = match storage {
            GraphicsStorageType::Float => "float",
            GraphicsStorageType::Uint => "uint",
        };
        params.push(format!(
            "texture2d<{kind}, access::write> storage0 [[texture(0)]]"
        ));
        if !interpolates {
            params.push("float4 pixel_position [[position]]".to_string());
        }
    }
    if pixel.uniform_mask != 0 {
        params.push("constant uint* uniforms [[buffer(2)]]".to_string());
    }
    if interpolates {
        params.push("VertexOutput input [[stage_in]]".to_string());
    }
    for (slot, sample_type) in types.textures.iter().enumerate() {
        if sampled_pairs & (0b11 << (slot * 2)) != 0 {
            let kind = match sample_type {
                GraphicsSampleType::Float => "texture2d<float, access::sample>",
                GraphicsSampleType::Uint => "texture2d<uint, access::sample>",
                GraphicsSampleType::Depth => "depth2d<float, access::sample>",
                GraphicsSampleType::FloatArray => "texture2d_array<float, access::sample>",
                GraphicsSampleType::UintArray => "texture2d_array<uint, access::sample>",
                GraphicsSampleType::StorageFloat | GraphicsSampleType::StorageUint => {
                    unreachable!("validated storage/sample role exclusion")
                }
            };
            params.push(format!("{kind} tex{slot} [[texture({slot})]]"));
        }
    }
    for slot in 0..2 {
        if sampled_pairs & (0b0101 << slot) != 0 {
            params.push(format!("sampler samp{slot} [[sampler({slot})]]"));
        }
    }
    let param_str = params.join(",\n    ");
    let param_decl = if params.is_empty() {
        String::new()
    } else if params.len() == 1 {
        param_str
    } else {
        format!("\n    {param_str}")
    };
    let _ = writeln!(
        msl,
        "fragment PixelOutput {PIXEL_ENTRY_POINT}({param_decl}) {{"
    );
    emit_fp_controls(&pixel, &mut msl);
    if storage.is_some() {
        msl.push_str(
            "    // Host must enforce the GraphicsStorageType unique-writer draw contract.\n",
        );
        let position = if interpolates {
            "input.position"
        } else {
            "pixel_position"
        };
        let _ = writeln!(msl, "    uint2 pixel_coordinates = uint2({position}.xy);");
    }
    ir.push_str("pixel:\n");
    emit_values(&pixel, types, &mut ir, &mut msl);
    msl.push_str("    PixelOutput output;\n");
    for (slot, color) in pixel.colors[..color_count].iter().enumerate() {
        if types.colors[slot] == GraphicsExportType::Uint {
            let export = color.as_ref().unwrap();
            let [a, b, c, d] = export.values;
            let _ = writeln!(
                ir,
                "{:04x}: export.bits.u32x4 mrt{slot}, {a}, {b}, {c}, {d}",
                export.offset
            );
            let _ = writeln!(
                msl,
                "    output.color{slot} = as_type<uint4>(float4({a}, {b}, {c}, {d}));"
            );
            continue;
        }
        emit_export(
            color.as_ref().unwrap(),
            &format!("mrt{slot}"),
            &format!("output.color{slot}"),
            &mut ir,
            &mut msl,
        );
    }
    msl.push_str("    return output;\n}\n");
    Ok(CompiledGraphics {
        types,
        storage,
        shader: CompiledShader {
            disassembly,
            ir,
            msl,
        },
        color_targets: color_count,
        uses_texture: has_sampling,
        sampled_pairs,
        array_layers,
        vertex_uniform_mask: vertex.uniform_mask,
        pixel_uniform_mask: pixel.uniform_mask,
        required_uniform_mask: vertex.uniform_mask | pixel.uniform_mask,
    })
}

fn lift(
    stage: Stage,
    decoded: &[DecodedInstruction],
    length: usize,
    storage: bool,
) -> Result<StageIr, CompileError> {
    let loops = cfg::analyze(decoded)?;
    let mut loop_headers: Vec<Option<RegisterState>> = vec![None; loops.len()];
    let mut loop_bounds = vec![None; loops.len()];
    let mut ir = StageIr::default();
    let mut registers = [None; 256];
    if stage == Stage::Vertex {
        let (positions, remaining) = registers.split_at_mut(4);
        for (slots, input) in [
            (positions, Float4Input::Position),
            (&mut remaining[..4], Float4Input::Color),
        ] {
            for (component, register) in slots.iter_mut().enumerate() {
                *register = Some(ir.push(None, Op::VertexInput(input, component as u8)));
            }
        }
    }

    if stage == Stage::Pixel && storage {
        registers[32] = Some(ir.push(None, Op::PixelCoordinate(0)));
        registers[33] = Some(ir.push(None, Op::PixelCoordinate(1)));
    }

    ir.initial_values = ir.values.len();
    ir.blocks = (0..decoded.len()).map(|_| None).collect();
    let mut incoming: Vec<Vec<(usize, RegisterState)>> = vec![Vec::new(); decoded.len() + 1];
    incoming[0].push((
        usize::MAX,
        RegisterState {
            registers,
            scalars: [None; 106],
            scc: None,
            last_export: None,
            pixel_done: false,
        },
    ));
    let mut terminated = false;
    // Removing proven latch edges leaves a forward DAG. Allocate header phis
    // before its body, then attach each backedge after its definitions are known.
    // Entry facts never specialize a value written by a live loop body.
    for index in 0..decoded.len() {
        if incoming[index].is_empty() {
            continue;
        }
        let DecodedInstruction {
            instruction,
            offset,
            ..
        } = decoded[index];
        let value_start = ir.values.len();
        let mut state = merge_state(&mut ir, &incoming[index], index, offset)?;
        let mut zero_trip_header = false;
        if let Some((loop_index, info)) = loops
            .iter()
            .enumerate()
            .find(|(_, info)| info.header == index)
        {
            let counter = state.scalars[usize::from(info.counter)].ok_or(CompileError::Verify {
                offset,
                reason: "loop counter is uninitialized at entry",
            })?;
            let bound = cfg::trip_bound(info, counter, &ir)
                .filter(|bound| *bound <= 256)
                .ok_or(CompileError::Verify {
                    offset,
                    reason: "loop trip count lacks a proven bound at most 256",
                })?;
            loop_bounds[loop_index] = Some(bound);
            if bound == 0 {
                // The guard is proven false, but the counter remains its actual
                // bit pattern (including runtime negative signed values).
                zero_trip_header = true;
            } else {
                let header = cfg::header_state(&mut ir, &state, info, offset);
                for (predecessor, _) in &incoming[index] {
                    let copies = cfg::copies(&header, &state, info);
                    ir.blocks[*predecessor]
                        .as_mut()
                        .expect("forward preheader")
                        .edge_copies
                        .push((index, copies));
                }
                state = header.clone();
                loop_headers[loop_index] = Some(header);
                ir.runtime_control = true;
            }
        }
        let mut registers = state.registers;
        let mut scalars = state.scalars;
        let mut scc = state.scc;
        let mut last_export = state.last_export;
        let exporting = last_export.is_some();
        let mut pixel_done = state.pixel_done;
        let mut next = index + 1;
        let mut branch = None;
        let mut ended = false;
        let error = |reason| CompileError::Verify { offset, reason };
        let read = |register: u8| {
            registers[usize::from(register)]
                .ok_or_else(|| error("source VGPR is uninitialized or reserved"))
        };
        match instruction {
            Instruction::ScalarMoveB32 { dst, src } if !exporting => {
                scalar_destination(dst, offset)?;
                let value = read_scalar(src, &scalars, &mut ir.uniform_mask, offset)?;
                scalars[usize::from(dst)] = Some(value);
                // MOV neither initializes nor overwrites SCC.
            }
            Instruction::ScalarAlu {
                op,
                dst,
                src0,
                src1,
            } if !exporting => {
                use ScalarAluOp::*;
                if !matches!(
                    op,
                    AddU32
                        | SubU32
                        | AddI32
                        | SubI32
                        | AddcU32
                        | SubbU32
                        | MinI32
                        | MinU32
                        | MaxI32
                        | MaxU32
                        | CselectB32
                ) {
                    return Err(error("graphics scalar ALU operation is unsupported"));
                }
                scalar_destination(dst, offset)?;
                // Capture both operands and incoming SCC before any aliased write.
                let left = read_scalar(src0, &scalars, &mut ir.uniform_mask, offset)?;
                let right = read_scalar(src1, &scalars, &mut ir.uniform_mask, offset)?;
                let carry = if matches!(op, AddcU32 | SubbU32 | CselectB32) {
                    scc.ok_or_else(|| error("SCC is uninitialized on the selected graphics path"))?
                } else {
                    Condition::Constant(false)
                };
                let (value, flag) = if op == CselectB32 {
                    let value = match carry {
                        Condition::Constant(true) => left,
                        Condition::Constant(false) => right,
                        _ if left.same(right) => left,
                        _ => ScalarValue::Runtime(
                            ir.push(Some(offset), Op::ScalarResult(op, left, right, carry)),
                        ),
                    };
                    (value, scc)
                } else if let (
                    ScalarValue::Constant(a),
                    ScalarValue::Constant(b),
                    Condition::Constant(c),
                ) = (left, right, carry)
                {
                    let (bits, flag) = scalar_constant(op, a, b, c);
                    (ScalarValue::Constant(bits), Some(Condition::Constant(flag)))
                } else {
                    let result = ir.push(Some(offset), Op::ScalarResult(op, left, right, carry));
                    let flag =
                        ir.push(Some(offset), Op::ScalarFlag(op, left, right, carry, result));
                    (ScalarValue::Runtime(result), Some(Condition::Runtime(flag)))
                };
                scalars[usize::from(dst)] = Some(value);
                scc = flag;
            }
            Instruction::ScalarCompare { op, src0, src1 } => {
                let left = read_scalar(src0, &scalars, &mut ir.uniform_mask, offset)?;
                let right = read_scalar(src1, &scalars, &mut ir.uniform_mask, offset)?;
                scc = Some(if zero_trip_header {
                    Condition::Constant(false)
                } else {
                    match (left, right) {
                        (ScalarValue::Constant(a), ScalarValue::Constant(b)) => {
                            Condition::Constant(op.test(a, b))
                        }
                        _ => {
                            Condition::Runtime(ir.push(Some(offset), Op::Compare(op, left, right)))
                        }
                    }
                });
            }
            Instruction::Branch { target } | Instruction::BranchSccZero { target } => {
                if target >= length {
                    return Err(error("expected a forward branch inside the graphics stage"));
                }
                let destination = decoded
                    .binary_search_by_key(&target, |instruction| instruction.offset)
                    .map_err(|_| error("branch target is not an instruction boundary"))?;
                if matches!(
                    decoded[destination].instruction,
                    Instruction::Interpolate {
                        phase: InterpolationPhase::Second,
                        ..
                    }
                ) {
                    return Err(error("branch target enters the second interpolation phase"));
                }
                let condition = match instruction {
                    Instruction::BranchSccZero { .. } => scc.ok_or_else(|| {
                        error("SCC is uninitialized on the selected graphics path")
                    })?,
                    _ => Condition::Constant(false),
                };
                match condition {
                    Condition::Constant(false) => next = destination,
                    Condition::Constant(true) => {}
                    Condition::Runtime(value) => {
                        if exporting {
                            return Err(error("runtime branch requires a common export epilogue"));
                        }
                        ir.runtime_control = true;
                        branch = Some((value, destination));
                    }
                }
            }
            Instruction::MoveB32 { dst, src } if !exporting => {
                if stage == Stage::Pixel && dst < 2 {
                    return Err(error("pixel VGPRs 0 and 1 are reserved for interpolation"));
                }
                let value = match src {
                    VectorSource::Register(register) => read(register)?,
                    VectorSource::Immediate(bits) if f32::from_bits(bits).is_finite() => {
                        ir.push(Some(offset), Op::Constant(bits))
                    }
                    VectorSource::Immediate(_) => {
                        return Err(error("graphics move requires finite FP32 bits"));
                    }
                    VectorSource::Scalar(register @ 24..=31) if stage == Stage::Pixel => {
                        let slot = register - 24;
                        ir.uniform_mask |= 1 << slot;
                        ir.push(Some(offset), Op::UniformFloat(slot))
                    }
                    VectorSource::Scalar(_) => {
                        return Err(error(
                            "graphics move requires a direct pixel uniform s24..s31",
                        ));
                    }
                };
                registers[usize::from(dst)] = Some(value);
            }
            Instruction::VectorAddF32 { dst, src0, src1 }
            | Instruction::VectorMulF32 { dst, src0, src1 }
                if !exporting =>
            {
                if stage == Stage::Pixel && dst < 2 {
                    return Err(error("pixel VGPRs 0 and 1 are reserved for interpolation"));
                }
                let operator = match instruction {
                    Instruction::VectorAddF32 { .. } => Binary::FloatAdd,
                    _ => Binary::FloatMul,
                };
                let value = ir.push(Some(offset), Op::Binary(operator, read(src0)?, read(src1)?));
                registers[usize::from(dst)] = Some(value);
            }
            Instruction::Interpolate {
                phase: InterpolationPhase::First,
                dst,
                coordinate: 0,
                attribute: 0,
                component,
            } if stage == Stage::Pixel && dst >= 2 && !exporting => {
                // Accept only an adjacent, matching pair. The intermediate
                // interpolation result is never observable independently.
                let expected = Instruction::Interpolate {
                    phase: InterpolationPhase::Second,
                    dst,
                    coordinate: 1,
                    attribute: 0,
                    component,
                };
                let second = decoded
                    .get(next)
                    .ok_or_else(|| error("missing matching second interpolation phase"))?;
                next += 1;
                if second.instruction != expected {
                    return Err(CompileError::Verify {
                        offset: second.offset,
                        reason: "expected matching second interpolation phase",
                    });
                }
                registers[usize::from(dst)] =
                    Some(ir.push(Some(offset), Op::Interpolate(component)));
            }
            Instruction::ImageStore2D {
                data,
                coord,
                resource,
                dmask,
            } if stage == Stage::Pixel && storage && !exporting => {
                if resource != 0 {
                    return Err(error("storage requires descriptor base s0"));
                }
                let coordinates = [read(coord)?, read(coord + 1)?];
                for (component, value) in coordinates.iter().enumerate() {
                    if !matches!(ir.values[value.0].1, Op::PixelCoordinate(c) if usize::from(c) == component)
                    {
                        return Err(error(
                            "store coordinates require original pixel X/Y or direct copies; used phi/arithmetic addresses are unsupported",
                        ));
                    }
                }
                let mut values = [None; 4];
                for (component, value) in
                    values
                        .iter_mut()
                        .enumerate()
                        .take(if dmask == 15 { 4 } else { 1 })
                {
                    *value = Some(read(data + component as u8)?);
                }
                ir.push(
                    Some(offset),
                    Op::Store2D {
                        coordinates,
                        data: values,
                        dmask,
                    },
                );
            }
            Instruction::ImageSampleLz {
                dst,
                coord,
                resource,
                sampler,
            }
            | Instruction::ImageSampleLzArray {
                dst,
                coord,
                resource,
                sampler,
            } if stage == Stage::Pixel && dst >= 2 && !exporting => {
                let array = matches!(instruction, Instruction::ImageSampleLzArray { .. });
                if dst > 252 || coord > if array { 253 } else { 254 } {
                    return Err(error("sample coordinate/data span exceeds v255"));
                }
                // Original synthetic ABI, not guest descriptor decoding. AMD RDNA2
                // Table41 defines four-SGPR alignment for both base operands.
                // Texture ranges0..7/12..19 and sampler ranges8..11/20..23 do not overlap.
                let texture_index = match resource {
                    0 => 0,
                    12 => 1,
                    _ => return Err(error("sampling outside synthetic texture bases 0 and 12")),
                };
                let sampler_index = match sampler {
                    8 => 0,
                    20 => 1,
                    _ => return Err(error("sampling outside synthetic sampler bases 8 and 20")),
                };
                let u = read(coord)?;
                let v = read(coord + 1)?;
                let layer = if array {
                    let value = read(coord + 2)?;
                    let requirement = match ir.values[value.0].1 {
                        Op::Constant(bits) => {
                            let f = f32::from_bits(bits);
                            if !f.is_finite()
                                || f < 0.0
                                || f.fract() != 0.0
                                || f64::from(f) >= 4294967296.0
                            {
                                return Err(error(
                                    "array layer constant must be a finite nonnegative u32 integer",
                                ));
                            }
                            GraphicsArrayLayer::Constant(f as u32)
                        }
                        Op::UniformFloat(slot) => GraphicsArrayLayer::Uniform(slot),
                        _ => {
                            return Err(error(
                                "array layer requires constant or direct uniform provenance; computed, interpolated and phi layers are unsupported",
                            ));
                        }
                    };
                    Some((value, requirement))
                } else {
                    None
                };
                for component in 0..4 {
                    let val = ir.push(
                        Some(offset),
                        Op::SampleLodZero {
                            u,
                            v,
                            layer,
                            component,
                            texture_index,
                            sampler_index,
                        },
                    );
                    registers[usize::from(dst + component)] = Some(val);
                }
            }
            Instruction::Export {
                target,
                sources: [a, b, c, d],
                done,
            } => {
                if stage == Stage::Pixel && pixel_done {
                    return Err(error("pixel export follows DONE"));
                }
                let slot = match (stage, target) {
                    (Stage::Vertex, ExportTarget::Position0) => &mut ir.position,
                    (Stage::Vertex, ExportTarget::Parameter0) => &mut ir.parameter,
                    (Stage::Pixel, ExportTarget::RenderTarget(index)) => {
                        &mut ir.colors[usize::from(index)]
                    }
                    _ => return Err(error("export target is invalid for this stage")),
                };
                if slot.is_some() {
                    return Err(error("export target is written more than once"));
                }
                *slot = Some(Export {
                    offset,
                    values: [read(a)?, read(b)?, read(c)?, read(d)?],
                });
                last_export = Some(offset);
                if stage == Stage::Pixel {
                    pixel_done = done;
                }
            }
            Instruction::EndProgram => {
                if stage == Stage::Pixel && exporting && !pixel_done {
                    return Err(error("final pixel export requires DONE"));
                }
                if let Some(next) = decoded.get(next) {
                    return Err(CompileError::Verify {
                        offset: next.offset,
                        reason: "instruction follows program termination",
                    });
                }
                terminated = true;
                ended = true;
            }
            _ => {
                return Err(error(
                    "instruction is unsupported at this point in the graphics stage",
                ));
            }
        }
        let mut successors = if ended { Vec::new() } else { vec![next] };
        if let Some((_, destination)) = branch
            && destination != next
        {
            successors.push(destination);
        }
        let outgoing = RegisterState {
            registers,
            scalars,
            scc,
            last_export,
            pixel_done,
        };
        let mut edge_copies = Vec::new();
        for &successor in &successors {
            if successor <= index {
                let (loop_index, info) = loops
                    .iter()
                    .enumerate()
                    .find(|(_, info)| info.latch == index && info.header == successor)
                    .expect("structurally verified backedge");
                let header = loop_headers[loop_index].as_ref().expect("live loop header");
                edge_copies.push((successor, cfg::copies(header, &outgoing, info)));
            } else {
                incoming[successor].push((index, outgoing.clone()));
            }
        }
        ir.blocks[index] = Some(Block {
            values: value_start..ir.values.len(),
            successors,
            condition: branch.map(|(condition, _)| condition),
            edge_copies,
        });
    }
    // Structural validation forbids interior entry. After forward propagation,
    // an interval with no emitted block is semantically unreachable, including
    // children skipped by a constant break. No artificial bound is assigned.
    let reachable_loops: Vec<bool> = loops
        .iter()
        .map(|info| {
            ir.blocks[info.header..=info.latch]
                .iter()
                .any(Option::is_some)
        })
        .collect();
    cfg::validate_work(decoded.len(), &loops, &loop_bounds, &reachable_loops)?;
    if !incoming[decoded.len()].is_empty() || !terminated {
        return Err(CompileError::Verify {
            offset: length,
            reason: "missing program termination",
        });
    }
    Ok(ir)
}

fn merge_state(
    ir: &mut StageIr,
    incoming: &[(usize, RegisterState)],
    destination: usize,
    offset: usize,
) -> Result<RegisterState, CompileError> {
    let mut groups: Vec<Vec<(Value, EdgeValue)>> = vec![Vec::new(); incoming.len()];
    let mut merged = incoming[0].1.clone();
    if incoming.iter().any(|(_, state)| {
        state.last_export != merged.last_export || state.pixel_done != merged.pixel_done
    }) {
        return Err(CompileError::Verify {
            offset,
            reason: "runtime paths require a common export epilogue",
        });
    }
    for register in 0..256 {
        if incoming
            .iter()
            .any(|(_, state)| state.registers[register].is_none())
        {
            merged.registers[register] = None;
        } else {
            let first = merged.registers[register].unwrap();
            if incoming
                .iter()
                .any(|(_, state)| state.registers[register].unwrap().0 != first.0)
            {
                let phi = ir.push(Some(offset), Op::Phi(ValueType::F32));
                for (group, (_, state)) in groups.iter_mut().zip(incoming) {
                    group.push((phi, EdgeValue::Float(state.registers[register].unwrap())));
                }
                merged.registers[register] = Some(phi);
            }
        }
    }
    for register in 32..106 {
        if incoming
            .iter()
            .any(|(_, state)| state.scalars[register].is_none())
        {
            merged.scalars[register] = None;
        } else {
            let first = merged.scalars[register].unwrap();
            if incoming
                .iter()
                .any(|(_, state)| !state.scalars[register].unwrap().same(first))
            {
                let phi = ir.push(Some(offset), Op::Phi(ValueType::U32));
                for (group, (_, state)) in groups.iter_mut().zip(incoming) {
                    group.push((phi, EdgeValue::Uint(state.scalars[register].unwrap())));
                }
                merged.scalars[register] = Some(ScalarValue::Runtime(phi));
            }
        }
    }
    if incoming.iter().any(|(_, state)| state.scc.is_none()) {
        merged.scc = None;
    } else {
        let first = merged.scc.unwrap();
        if incoming
            .iter()
            .any(|(_, state)| !state.scc.unwrap().same(first))
        {
            let phi = ir.push(Some(offset), Op::Phi(ValueType::Bool));
            for (group, (_, state)) in groups.iter_mut().zip(incoming) {
                group.push((phi, EdgeValue::Bool(state.scc.unwrap())));
            }
            merged.scc = Some(Condition::Runtime(phi));
        }
    }
    // One parallel group per incoming edge. Header initialization, if needed,
    // is appended later and must observe the values established by this group.
    for ((predecessor, _), group) in incoming.iter().zip(groups) {
        if !group.is_empty() {
            ir.blocks[*predecessor]
                .as_mut()
                .unwrap()
                .edge_copies
                .push((destination, group));
        }
    }
    Ok(merged)
}

fn scalar_destination(register: u8, offset: usize) -> Result<(), CompileError> {
    if !(32..=105).contains(&register) {
        return Err(CompileError::Verify {
            offset,
            reason: "graphics scalar destination must be a local s32..s105",
        });
    }
    Ok(())
}

fn read_scalar(
    source: ScalarSource,
    locals: &[Option<ScalarValue>; 106],
    mask: &mut u8,
    offset: usize,
) -> Result<ScalarValue, CompileError> {
    match source {
        ScalarSource::Immediate(bits) => Ok(ScalarValue::Constant(bits)),
        ScalarSource::Register(register @ 24..=31) => {
            *mask |= 1 << (register - 24);
            Ok(ScalarValue::Uniform(register - 24))
        }
        ScalarSource::Register(register @ 32..=105) => {
            locals[usize::from(register)].ok_or(CompileError::Verify {
                offset,
                reason: "graphics scalar local is uninitialized",
            })
        }
        _ => Err(CompileError::Verify {
            offset,
            reason: "graphics scalar compare: other SGPRs are unsupported",
        }),
    }
}

// AMD RDNA2 section 12.1. All arithmetic is unsigned bit arithmetic; signed
// add/sub overflow is independent of carry/borrow. No native signed overflow.
fn scalar_constant(op: ScalarAluOp, a: u32, b: u32, carry: bool) -> (u32, bool) {
    use ScalarAluOp::*;
    let c = u32::from(carry);
    match op {
        AddU32 | AddI32 | AddcU32 => {
            let t = a.wrapping_add(b);
            let r = t.wrapping_add(if op == AddcU32 { c } else { 0 });
            let flag = if op == AddI32 {
                (!(a ^ b) & (a ^ r) & 0x80000000) != 0
            } else {
                t < a || r < t
            };
            (r, flag)
        }
        SubU32 | SubI32 | SubbU32 => {
            let r = a
                .wrapping_sub(b)
                .wrapping_sub(if op == SubbU32 { c } else { 0 });
            let flag = if op == SubI32 {
                ((a ^ b) & (a ^ r) & 0x80000000) != 0
            } else {
                a < b || (op == SubbU32 && carry && a == b)
            };
            (r, flag)
        }
        MinI32 | MinU32 | MaxI32 | MaxU32 => {
            let select = match op {
                MinI32 => (a as i32) < (b as i32),
                MinU32 => a < b,
                MaxI32 => (a as i32) > (b as i32),
                _ => a > b,
            };
            (if select { a } else { b }, select)
        }
        _ => unreachable!("validated scalar operation"),
    }
}

fn scalar_result_expression(
    op: ScalarAluOp,
    left: ScalarValue,
    right: ScalarValue,
    carry: Condition,
) -> String {
    use ScalarAluOp::*;
    let a = left.expression();
    let b = right.expression();
    let c = carry.expression();
    match op {
        AddU32 | AddI32 => format!("{a} + {b}"),
        SubU32 | SubI32 => format!("{a} - {b}"),
        AddcU32 => format!("({a} + {b}) + uint({c})"),
        SubbU32 => format!("({a} - {b}) - uint({c})"),
        CselectB32 => format!("{c} ? {a} : {b}"),
        MinI32 | MinU32 | MaxI32 | MaxU32 => {
            let relation = if matches!(op, MinI32 | MinU32) {
                "<"
            } else {
                ">"
            };
            let condition = if matches!(op, MinI32 | MaxI32) {
                format!("as_type<int>({a}) {relation} as_type<int>({b})")
            } else {
                format!("{a} {relation} {b}")
            };
            format!("({condition}) ? {a} : {b}")
        }
        _ => unreachable!("validated scalar operation"),
    }
}

fn scalar_flag_expression(
    op: ScalarAluOp,
    left: ScalarValue,
    right: ScalarValue,
    carry: Condition,
    result: Value,
) -> String {
    use ScalarAluOp::*;
    let a = left.expression();
    let b = right.expression();
    let c = carry.expression();
    match op {
        AddU32 => format!("{result} < {a}"),
        AddcU32 => format!("(({a} + {b}) < {a}) || ({result} < ({a} + {b}))"),
        SubU32 => format!("{a} < {b}"),
        SubbU32 => format!("({a} < {b}) || ({c} && ({a} == {b}))"),
        AddI32 => format!("((~({a} ^ {b}) & ({a} ^ {result})) & 0x80000000u) != 0u"),
        SubI32 => format!("((({a} ^ {b}) & ({a} ^ {result})) & 0x80000000u) != 0u"),
        MinI32 => format!("as_type<int>({a}) < as_type<int>({b})"),
        MaxI32 => format!("as_type<int>({a}) > as_type<int>({b})"),
        MinU32 => format!("{a} < {b}"),
        MaxU32 => format!("{a} > {b}"),
        _ => unreachable!("validated flag-producing operation"),
    }
}

fn compare_expression(op: ScalarCompareOp, left: ScalarValue, right: ScalarValue) -> String {
    use ScalarCompareOp::*;
    let (signed, relation) = match op {
        CmpEqI32 => (true, "=="),
        CmpLgI32 => (true, "!="),
        CmpGtI32 => (true, ">"),
        CmpGeI32 => (true, ">="),
        CmpLtI32 => (true, "<"),
        CmpLeI32 => (true, "<="),
        CmpEqU32 => (false, "=="),
        CmpLgU32 => (false, "!="),
        CmpGtU32 => (false, ">"),
        CmpGeU32 => (false, ">="),
        CmpLtU32 => (false, "<"),
        CmpLeU32 => (false, "<="),
    };
    let source = |operand: ScalarValue| {
        let bits = operand.expression();
        if signed {
            format!("as_type<int>({bits})")
        } else {
            bits
        }
    };
    format!("{} {relation} {}", source(left), source(right))
}

fn validate_resource_types(
    pixel: &StageIr,
    types: GraphicsPipelineTypes,
    pairs: u8,
    colors: usize,
) -> Result<(), CompileError> {
    let storage = storage_type(types);
    let has_store = pixel
        .values
        .iter()
        .any(|(_, op)| matches!(op, Op::Store2D { .. }));
    if storage.is_some() != has_store || (storage.is_some() && pairs & 3 != 0) {
        return Err(CompileError::GraphicsLink {
            reason: "storage requires a reachable store and excludes sampling texture0",
        });
    }
    if (0..2).any(|slot| {
        pairs & (3 << (slot * 2)) == 0
            && !(slot == 0 && storage.is_some())
            && types.textures[slot] != GraphicsSampleType::Float
    }) || types.colors[colors..]
        .iter()
        .any(|t| *t != GraphicsExportType::Float)
    {
        return Err(CompileError::GraphicsLink {
            reason: "nondefault type for an unused resource slot",
        });
    }
    // Layer uniforms originate only from direct s24..s31 moves, so all indices
    // fit this eight-bit set. Scan reachable sample requirements once.
    let layer_uniform_mask = pixel.values.iter().fold(0u8, |mask, (_, op)| {
        if let Op::SampleLodZero {
            layer: Some((_, GraphicsArrayLayer::Uniform(slot))),
            ..
        } = op
        {
            mask | (1u8 << slot)
        } else {
            mask
        }
    });
    for (offset, op) in &pixel.values {
        if let Op::SampleLodZero {
            texture_index,
            layer,
            ..
        } = op
        {
            let array_type = matches!(
                types.textures[usize::from(*texture_index)],
                GraphicsSampleType::FloatArray | GraphicsSampleType::UintArray
            );
            if array_type != layer.is_some() {
                return Err(CompileError::Verify {
                    offset: offset.unwrap_or(0),
                    reason: "sample dimension does not match texture type; depth arrays are unsupported",
                });
            }
        }
        if let Op::UniformFloat(slot) = op
            && layer_uniform_mask & (1u8 << slot) == 0
        {
            return Err(CompileError::Verify {
                offset: offset.unwrap_or(0),
                reason: "uniform float move requires matching array layer validation metadata",
            });
        }
    }
    let mut integer_bits: Vec<bool> = pixel.values.iter().map(|(_, op)| matches!(op,
        Op::SampleLodZero {texture_index, ..} if matches!(types.textures[*texture_index as usize],
            GraphicsSampleType::Uint | GraphicsSampleType::UintArray)
    )).collect();
    let mut dependents = vec![Vec::new(); pixel.values.len()];
    for block in pixel.blocks.iter().flatten() {
        for (_, copies) in &block.edge_copies {
            for (destination, source) in copies {
                if let EdgeValue::Float(source) = source {
                    dependents[source.0].push(destination.0);
                }
            }
        }
    }
    let mut work: Vec<usize> = integer_bits
        .iter()
        .enumerate()
        .filter_map(|(index, tainted)| tainted.then_some(index))
        .collect();
    while let Some(source) = work.pop() {
        for &destination in &dependents[source] {
            if !integer_bits[destination] {
                integer_bits[destination] = true;
                work.push(destination);
            }
        }
    }
    let mut pixel_coordinates: Vec<bool> = pixel
        .values
        .iter()
        .map(|(_, op)| matches!(op, Op::PixelCoordinate(_)))
        .collect();
    let mut work: Vec<_> = pixel_coordinates
        .iter()
        .enumerate()
        .filter_map(|(i, used)| used.then_some(i))
        .collect();
    while let Some(source) = work.pop() {
        for &destination in &dependents[source] {
            if !pixel_coordinates[destination] {
                pixel_coordinates[destination] = true;
                work.push(destination);
            }
        }
    }
    for (offset, op) in &pixel.values {
        let escaped = match op {
            Op::Binary(_, a, b) => pixel_coordinates[a.0] || pixel_coordinates[b.0],
            Op::SampleLodZero { u, v, layer, .. } => {
                pixel_coordinates[u.0]
                    || pixel_coordinates[v.0]
                    || layer.is_some_and(|(v, _)| pixel_coordinates[v.0])
            }
            Op::Store2D { data, .. } => data.iter().flatten().any(|v| pixel_coordinates[v.0]),
            _ => false,
        };
        if escaped {
            return Err(CompileError::Verify {
                offset: offset.unwrap_or(0),
                reason: "pixel-coordinate bits may only be copied or used as store addresses",
            });
        }
        if let Op::Store2D { data, dmask, .. } = op {
            let expected = match storage {
                Some(GraphicsStorageType::Float) => 15,
                Some(GraphicsStorageType::Uint) => 1,
                None => unreachable!("validated store role"),
            };
            if *dmask != expected {
                return Err(CompileError::Verify {
                    offset: offset.unwrap_or(0),
                    reason: "store mask must match storage Float RGBA15 or Uint R1",
                });
            }
            if storage == Some(GraphicsStorageType::Float)
                && data.iter().flatten().any(|v| integer_bits[v.0])
            {
                return Err(CompileError::Verify {
                    offset: offset.unwrap_or(0),
                    reason: "RGBA storage cannot consume sampled integer bits",
                });
            }
        }
    }
    // The new SGPR->VGPR bit move is deliberately limited to validated layer
    // transport. Propagate through copies/phis to reject other observable uses.
    let mut uniform_floats: Vec<bool> = pixel
        .values
        .iter()
        .map(|(_, op)| matches!(op, Op::UniformFloat(_)))
        .collect();
    let mut work: Vec<usize> = uniform_floats
        .iter()
        .enumerate()
        .filter_map(|(index, used)| used.then_some(index))
        .collect();
    while let Some(source) = work.pop() {
        for &destination in &dependents[source] {
            if !uniform_floats[destination] {
                uniform_floats[destination] = true;
                work.push(destination);
            }
        }
    }
    for (offset, op) in &pixel.values {
        let invalid_use = match op {
            Op::SampleLodZero { u, v, .. } => uniform_floats[u.0] || uniform_floats[v.0],
            Op::Binary(_, a, b) => uniform_floats[a.0] || uniform_floats[b.0],
            Op::Store2D { data, .. } => data.iter().flatten().any(|v| uniform_floats[v.0]),
            _ => false,
        };
        if invalid_use {
            return Err(CompileError::Verify {
                offset: offset.unwrap_or(0),
                reason: "uniform float moves are supported only as validated array layers",
            });
        }
    }
    for (offset, op) in &pixel.values {
        if let Op::SampleLodZero { u, v, .. } = op
            && (integer_bits[u.0] || integer_bits[v.0])
        {
            return Err(CompileError::Verify {
                offset: offset.unwrap_or(0),
                reason: "sampled integer bits cannot be floating texture coordinates",
            });
        }
        if let Op::Binary(_, a, b) = op
            && (integer_bits[a.0] || integer_bits[b.0])
        {
            return Err(CompileError::Verify {
                offset: offset.unwrap_or(0),
                reason: "floating arithmetic on sampled integer bits requires explicit conversion",
            });
        }
    }
    for (slot, export) in pixel.colors[..colors].iter().enumerate() {
        let export = export.as_ref().expect("validated contiguous exports");
        if export.values.iter().any(|v| pixel_coordinates[v.0]) {
            return Err(CompileError::Verify {
                offset: export.offset,
                reason: "pixel-coordinate bits cannot be exported",
            });
        }
        if export.values.iter().any(|v| uniform_floats[v.0]) {
            return Err(CompileError::Verify {
                offset: export.offset,
                reason: "uniform float moves are supported only as validated array layers",
            });
        }
        if types.colors[slot] == GraphicsExportType::Float
            && export.values.iter().any(|v| integer_bits[v.0])
        {
            return Err(CompileError::Verify {
                offset: export.offset,
                reason: "sampled integer bits require an unsigned integer export",
            });
        }
    }
    Ok(())
}

fn value_type(op: Op) -> &'static str {
    match op {
        Op::Compare(..) | Op::ScalarFlag(..) | Op::Phi(ValueType::Bool) => "bool",
        Op::ScalarResult(..) | Op::Phi(ValueType::U32) => "uint",
        _ => "float",
    }
}

// Clang requires these controls before declarations or statements in the body.
fn emit_fp_controls(stage: &StageIr, msl: &mut String) {
    if stage
        .values
        .iter()
        .any(|(_, op)| matches!(op, Op::Binary(..)))
    {
        msl.push_str("    #pragma clang fp contract(off)\n    #pragma clang fp reassociate(off)\n");
    }
}

fn emit_values(stage: &StageIr, types: GraphicsPipelineTypes, ir: &mut String, msl: &mut String) {
    if !stage.runtime_control {
        emit_range(stage, types, 0..stage.values.len(), false, ir, msl);
        return;
    }
    for (index, &(_, op)) in stage.values.iter().enumerate() {
        if matches!(op, Op::Store2D { .. }) {
            continue;
        }
        let _ = writeln!(msl, "    {} {};", value_type(op), Value(index));
    }
    emit_range(stage, types, 0..stage.initial_values, true, ir, msl);
    msl.push_str("    uint block = 0u;\n    bool finished = false;\n    while (!finished) {\n    switch (block) {\n");
    for (index, block) in stage.blocks.iter().enumerate() {
        let Some(block) = block else {
            continue;
        };
        let _ = writeln!(ir, "block {index}:");
        let _ = writeln!(msl, "    case {index}u: {{");
        emit_range(stage, types, block.values.clone(), true, ir, msl);
        let edge = |next: usize, ir: &mut String, msl: &mut String| {
            msl.push_str("        {\n");
            for (target, copies) in &block.edge_copies {
                if *target == next {
                    msl.push_str("        {\n");
                    emit_parallel_copies(stage, copies, ir, msl);
                    msl.push_str("        }\n");
                }
            }
            let _ = writeln!(msl, "        block = {next}u;\n        }}");
        };
        match block.successors.as_slice() {
            [next] => edge(*next, ir, msl),
            [yes, no] => {
                let condition = block.condition.unwrap();
                let _ = writeln!(ir, "branch {condition}, {yes}, {no}");
                let _ = writeln!(msl, "        if ({condition}) {{");
                edge(*yes, ir, msl);
                msl.push_str("        } else {\n");
                edge(*no, ir, msl);
                msl.push_str("        }\n");
            }
            [] => msl.push_str("        finished = true;\n"),
            _ => unreachable!("at most two successors"),
        }
        msl.push_str("        break;\n    }\n");
    }
    // All transfers name emitted blocks; this is not a successful fallback path.
    msl.push_str("    default: __builtin_unreachable();\n    }\n    }\n");
}

fn emit_parallel_copies(
    stage: &StageIr,
    copies: &[(Value, EdgeValue)],
    ir: &mut String,
    msl: &mut String,
) {
    for (index, &(destination, source)) in copies.iter().enumerate() {
        let _ = writeln!(
            msl,
            "        {} edge_tmp{index} = {};",
            value_type(stage.values[destination.0].1),
            source.expression()
        );
    }
    for (index, &(destination, source)) in copies.iter().enumerate() {
        let _ = writeln!(ir, "{destination} <- {}", source.expression());
        let _ = writeln!(msl, "        {destination} = edge_tmp{index};");
    }
}

fn emit_range(
    stage: &StageIr,
    types: GraphicsPipelineTypes,
    range: std::ops::Range<usize>,
    assignment: bool,
    ir: &mut String,
    msl: &mut String,
) {
    for index in range {
        let (offset, op) = stage.values[index];

        let value = Value(index);
        let (operation, expression) = match op {
            Op::Store2D {
                coordinates: [x, y],
                data,
                dmask,
            } => {
                let values = match storage_type(types) {
                    Some(GraphicsStorageType::Float) => {
                        let [a, b, c, d] = data.map(|v| v.expect("validated RGBA mask"));
                        format!("float4({a}, {b}, {c}, {d})")
                    }
                    Some(GraphicsStorageType::Uint) => {
                        let a = data[0].expect("validated R mask");
                        format!("uint4(as_type<uint>({a}), 0u, 0u, 0u)")
                    }
                    None => unreachable!("validated storage effect"),
                };
                if let Some(offset) = offset {
                    let _ = write!(ir, "{offset:04x}: ");
                }
                let _ = writeln!(ir, "store.2d tex0, ({x}, {y}), dmask:{dmask} {values}");
                let _ = writeln!(
                    msl,
                    "    storage0.write({values}, uint2(as_type<uint>({x}), as_type<uint>({y})));"
                );
                continue;
            }
            Op::PixelCoordinate(component) => (
                format!("pixel.coordinate.u32.bits [{component}]"),
                format!("as_type<float>(pixel_coordinates[{component}])"),
            ),
            Op::Phi(_) => continue, // Assigned on all incoming edges.
            Op::Compare(operator, left, right) => {
                let expression = compare_expression(operator, left, right);
                (format!("compare.u32 {expression}"), expression)
            }
            Op::ScalarResult(operator, left, right, carry) => {
                let expression = scalar_result_expression(operator, left, right, carry);
                (
                    format!("{}.u32 {expression}", operator.mnemonic()),
                    expression,
                )
            }
            Op::ScalarFlag(operator, left, right, carry, result) => {
                let expression = scalar_flag_expression(operator, left, right, carry, result);
                (format!("scc.bool {expression}"), expression)
            }
            Op::VertexInput(input, component) => {
                let buffer = match input {
                    Float4Input::Position => "positions",
                    Float4Input::Color => "colors",
                };
                (
                    format!("load.vertex.f32 {buffer}[{component}]"),
                    format!("{buffer}[vertex_id][{component}]"),
                )
            }
            Op::UniformFloat(slot) => (
                format!("uniform.bits.f32 [{slot}]"),
                format!("as_type<float>(uniforms[{slot}])"),
            ),
            Op::Constant(bits) => (
                format!("const.f32 0x{bits:08x}"),
                format!("as_type<float>(0x{bits:08x}u)"),
            ),
            Op::Binary(operator, left, right) => {
                let (mnemonic, symbol) = operator.operator();
                (
                    format!("{mnemonic} {left}, {right}"),
                    format!("{left} {symbol} {right}"),
                )
            }
            Op::Interpolate(component) => (
                format!("interpolate.center_perspective.f32 attr0[{component}]"),
                format!("input.parameter[{component}]"),
            ),
            Op::SampleLodZero {
                u,
                v,
                layer,
                component,
                texture_index,
                sampler_index,
            } => {
                let channel = ["r", "g", "b", "a"][usize::from(component)];
                let layer_argument =
                    layer.map_or_else(String::new, |(value, _)| format!(", uint({value})"));
                let sample = format!(
                    "tex{texture_index}.sample(samp{sampler_index}, float2({u}, {v}){layer_argument}, level(0.0f))"
                );
                let (kind, expression) = match types.textures[texture_index as usize] {
                    GraphicsSampleType::Float | GraphicsSampleType::FloatArray => {
                        ("f32", format!("{sample}.{channel}"))
                    }
                    GraphicsSampleType::Uint | GraphicsSampleType::UintArray => {
                        ("u32.bits", format!("as_type<float>({sample}.{channel})"))
                    }
                    GraphicsSampleType::StorageFloat | GraphicsSampleType::StorageUint => {
                        unreachable!("validated storage/sample exclusion")
                    }
                    GraphicsSampleType::Depth => (
                        "depth",
                        match component {
                            0 => sample,
                            3 => "1.0f".into(),
                            _ => "0.0f".into(),
                        },
                    ),
                };
                let dimension = if layer.is_some() { "2d_array" } else { "2d" };
                let layer_ir = layer.map_or_else(String::new, |(value, _)| format!(", {value}"));
                (
                    format!(
                        "sample_lz.{dimension}.{kind} tex{texture_index}, samp{sampler_index}, ({u}, {v}{layer_ir})[{channel}]"
                    ),
                    expression,
                )
            }
        };
        if let Some(offset) = offset {
            let _ = write!(ir, "{offset:04x}: ");
        }
        let _ = writeln!(ir, "{value} = {operation}");
        let declaration = if assignment {
            String::new()
        } else {
            format!("{} ", value_type(op))
        };
        let _ = writeln!(msl, "    {declaration}{value} = {expression};");
    }
}

fn emit_export(export: &Export, target: &str, field: &str, ir: &mut String, msl: &mut String) {
    let [a, b, c, d] = export.values;
    let offset = export.offset;
    let _ = writeln!(
        ir,
        "{offset:04x}: export.f32x4 {target}, {a}, {b}, {c}, {d}"
    );
    let _ = writeln!(msl, "    {field} = float4({a}, {b}, {c}, {d});");
}

#[cfg(test)]
mod cfg_tests {
    use super::*;

    #[test]
    fn join_does_not_invent_scc_or_register_definitions() {
        // Test the dataflow meet directly: a missing SCC predecessor cannot arise
        // from an SCC-controlled fork today, but must never become false by default.
        let block = || {
            Some(Block {
                values: 0..0,
                successors: vec![2],
                condition: None,
                edge_copies: Vec::new(),
            })
        };
        let mut ir = StageIr {
            blocks: vec![block(), block()],
            ..StageIr::default()
        };
        let value = ir.push(None, Op::Constant(0x3f800000));
        let mut first = RegisterState {
            registers: [None; 256],
            scalars: [None; 106],
            scc: Some(Condition::Constant(true)),
            last_export: None,
            pixel_done: false,
        };
        first.registers[8] = Some(value);
        first.registers[9] = Some(value);
        let mut second = first.clone();
        second.scc = None;
        second.registers[8] = None;
        let merged = merge_state(&mut ir, &[(0, first), (1, second)], 2, 8).unwrap();
        assert!(merged.scc.is_none());
        assert!(merged.registers[8].is_none());
        assert_eq!(merged.registers[9].unwrap().0, value.0);
        assert!(
            ir.blocks
                .iter()
                .all(|block| block.as_ref().unwrap().edge_copies.is_empty())
        );
    }
}
