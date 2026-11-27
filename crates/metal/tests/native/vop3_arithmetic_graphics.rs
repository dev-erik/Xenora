// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 12.12 signed extraction and shift/arithmetic combinations.
//! https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture
use xenora_metal::*;
use xenora_shader::{
    Architecture, GraphicsExportType, GraphicsPipelineTypes, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT,
    compile_graphics_pipeline_typed,
};
use xenora_texture::TextureFormat;

const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const COLORS: [[f32; 4]; 3] = [[0.; 4]; 3];

fn render(gpu: &NativeCompute, ps: &[u8], values: [u32; 8]) -> Vec<Vec<u8>> {
    let mut types = GraphicsPipelineTypes::default();
    types.colors[0] = GraphicsExportType::Uint;
    let pipeline = compile_graphics_pipeline_typed(Architecture::Gfx1030, VS, ps, types).unwrap();

    gpu.submit_typed_triangle_targets(
        &[TriangleShader {
            source: &pipeline.shader.msl,
            entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
        }],
        &TypedTriangleTargetsPass {
            draws: &[TriangleDraw {
                vertices: &POS,
                colors: &COLORS,
                viewport: [1, 1],
            }],
            states: &[TriangleState::default()],
            extent: [1, 1],
            colors: &[TypedColorAttachment {
                format: TextureFormat::R32Uint,
                clear: TypedColorClear::Uint([0x13579bdf; 4]),
                blend_enabled: false,
            }],
            clear_depth: None,
        },
        &[vec![]],
        None,
        &[None],
        &[DrawUniforms {
            required_mask: pipeline.required_uniform_mask,
            values: Some(values),
        }],
    )
    .unwrap()
    .wait()
    .unwrap()
}

// Literal answers derived using unbounded integer arithmetic, not either runtime oracle.
const CASES: &[(u32, u32, u32, [u32; 4])] = &[
    (
        0x00000008,
        0x00000000,
        0x00000004,
        [0xfffffff8, 0x0000000c, 0x00000080, 0x0000000c],
    ),
    (
        0x00000007,
        0x00000000,
        0x00000004,
        [0x00000007, 0x0000000b, 0x00000070, 0x00000007],
    ),
    (
        0x80000000,
        0x0000001f,
        0x00000002,
        [0xffffffff, 0x00000002, 0x0000007c, 0x00000002],
    ),
    (
        0x80000000,
        0x00000000,
        0x0000001f,
        [0x00000000, 0x8000001f, 0x00000000, 0x8000001f],
    ),
    (
        0xffffffff,
        0x00000000,
        0x00000020,
        [0x00000000, 0x0000001f, 0xffffffff, 0xffffffff],
    ),
    (
        0x00000001,
        0x00000000,
        0x00000021,
        [0xffffffff, 0x00000022, 0x00000002, 0x00000021],
    ),
    (
        0x40000000,
        0x0000001e,
        0x00000002,
        [0x00000001, 0x00000002, 0x00000078, 0x00000002],
    ),
    (
        0xc0000000,
        0x0000001e,
        0x00000002,
        [0xffffffff, 0x00000002, 0x00000078, 0x00000002],
    ),
    (
        0x00000003,
        0x00000004,
        0x00000005,
        [0x00000000, 0x00000035, 0x000000e0, 0x00000035],
    ),
    (
        0x00000003,
        0x00000001,
        0x00000002,
        [0x00000001, 0x00000008, 0x00000010, 0x00000006],
    ),
    (
        0x7fa12345,
        0xfffffffe,
        0x0000001f,
        [0x00000001, 0x4000001f, 0x80000000, 0x4000001f],
    ),
    (
        0xffffffff,
        0x00000001,
        0x00000002,
        [0xffffffff, 0x00000000, 0x00000000, 0xfffffffe],
    ),
    (
        0x80000000,
        0x80000000,
        0x0000001f,
        [0x00000000, 0x8000001f, 0x00000000, 0x8000001f],
    ),
    (
        0xffffffff,
        0xffffffff,
        0xffffffff,
        [0xffffffff, 0x7fffffff, 0x00000000, 0xffffffff],
    ),
    (
        0x00000000,
        0x00000000,
        0x00000000,
        [0x00000000, 0x00000000, 0x00000000, 0x00000000],
    ),
];
const OPS: [u32; 4] = [329, 838, 839, 879];
fn pixel(op: u32, dst: u32, source: [u32; 3], literal: Option<u32>) -> Vec<u8> {
    let mut code = vec![
        0x7e180280,
        0x4a101818,
        0x4a121819,
        0x4a14181a,
        0xd4000000 | op << 16 | dst,
        source[0] | source[1] << 9 | source[2] << 18,
    ];
    if let Some(word) = literal {
        code.push(word);
    }
    code.extend([0xf800180f, dst * 0x01010101, 0xbf810000]);
    code.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn arithmetic_boundaries_are_exact_native_words() {
    let gpu = NativeCompute::new().unwrap();
    for &(a, b, c, answers) in CASES {
        for (column, op) in OPS.into_iter().enumerate() {
            let sources = [264, 265, 266];
            assert_eq!(
                render(
                    &gpu,
                    &pixel(op, 11, sources, None),
                    [a, b, c, 0, 0, 0, 0, 0]
                ),
                vec![answers[column].to_le_bytes().to_vec()],
                "op={op} inputs={a:x},{b:x},{c:x}"
            );
        }
    }
}
#[test]
fn arithmetic_alias_scalar_and_literal_sources_capture_each_position() {
    let gpu = NativeCompute::new().unwrap();
    let (a, b, c, answers) = CASES[10];
    let input = [a, b, c];
    for (column, op) in OPS.into_iter().enumerate() {
        let count = 3;
        let base = [264, 265, if count == 2 { 128 } else { 266 }];
        let expected = vec![answers[column].to_le_bytes().to_vec()];
        for position in 0..count {
            for selector in [base[position], 24 + position as u32, 255] {
                let mut sources = base;
                sources[position] = selector;
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(
                            op,
                            8 + position as u32,
                            sources,
                            (selector == 255).then_some(input[position])
                        ),
                        [a, b, c, 0, 0, 0, 0, 0]
                    ),
                    expected,
                    "op={op} position={position} selector={selector}"
                );
            }
        }
    }
}
