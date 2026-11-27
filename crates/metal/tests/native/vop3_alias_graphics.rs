// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 12.12 canonical integer e64 aliases.
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

// Fixed words derived independently using unbounded integer arithmetic.
const OPS: [u32; 23] = [
    265, 266, 267, 268, 273, 274, 275, 276, 286, 293, 294, 295, 282, 278, 280, 283, 284, 285, 439,
    440, 441, 442, 443,
];
const CASES: &[(u32, u32, [u32; 23])] = &[
    (
        0x00000000,
        0x00000000,
        [
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
            0x00000000, 0xffffffff, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0xffffffff, 0x00000000, 0xffffffff,
            0xffffffff, 0xffffffff,
        ],
    ),
    (
        0xffffffff,
        0xffffffff,
        [
            0x00000001, 0x00000000, 0xfe000001, 0x0000ffff, 0xffffffff, 0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff, 0xfffffffe, 0x00000000, 0x00000000, 0x80000000, 0x00000001,
            0xffffffff, 0xffffffff, 0xffffffff, 0x00000000, 0x00000000, 0xffffffff, 0x00000000,
            0x00000000, 0xffffffff,
        ],
    ),
    (
        0x00800000,
        0x00000002,
        [
            0xff000000, 0xffffffff, 0x01000000, 0x00000000, 0x00000002, 0x00800000, 0x00000002,
            0x00800000, 0xff7ffffd, 0x00800002, 0x007ffffe, 0xff800002, 0x00000002, 0x00000002,
            0x00000002, 0x00000000, 0x00800002, 0x00800002, 0xff7fffff, 0x00000100, 0x00000008,
            0x00000017, 0x00000008,
        ],
    ),
    (
        0x00800001,
        0x00ffffff,
        [
            0x007fffff, 0x00000000, 0x007fffff, 0x00008000, 0x00800001, 0x00ffffff, 0x00800001,
            0x00ffffff, 0xff800001, 0x01800000, 0xff800002, 0x007ffffe, 0x01fffffe, 0x007fffff,
            0x007fffff, 0x00800001, 0x00ffffff, 0x007ffffe, 0xff7ffffe, 0x80000100, 0x00000008,
            0x00000000, 0x00000008,
        ],
    ),
    (
        0x80000000,
        0x0000001f,
        [
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x80000000, 0x0000001f, 0x0000001f,
            0x80000000, 0x7fffffe0, 0x8000001f, 0x7fffffe1, 0x8000001f, 0x0000001f, 0x0000001f,
            0x0000001f, 0x00000000, 0x8000001f, 0x8000001f, 0x7fffffff, 0x00000001, 0x00000000,
            0x0000001f, 0x00000001,
        ],
    ),
    (
        0x0000001f,
        0x80000001,
        [
            0x0000001f, 0x00000000, 0x0000001f, 0x00000000, 0x80000001, 0x0000001f, 0x0000001f,
            0x80000001, 0x7fffffe1, 0x80000020, 0x8000001e, 0x7fffffe2, 0x80000000, 0x00000001,
            0xffffffff, 0x00000001, 0x8000001f, 0x8000001e, 0xffffffe0, 0xf8000000, 0x0000001b,
            0x00000000, 0x0000001b,
        ],
    ),
    (
        0x00000020,
        0x00000001,
        [
            0x00000020, 0x00000000, 0x00000020, 0x00000000, 0x00000001, 0x00000020, 0x00000001,
            0x00000020, 0xffffffde, 0x00000021, 0x0000001f, 0xffffffe1, 0x00000001, 0x00000001,
            0x00000001, 0x00000000, 0x00000021, 0x00000021, 0xffffffdf, 0x04000000, 0x0000001a,
            0x00000005, 0x0000001a,
        ],
    ),
    (
        0x00000021,
        0xffffffff,
        [
            0xffffffdf, 0xffffffff, 0x20ffffdf, 0x00000000, 0xffffffff, 0x00000021, 0x00000021,
            0xffffffff, 0x00000021, 0x00000020, 0x00000022, 0xffffffde, 0xfffffffe, 0x7fffffff,
            0xffffffff, 0x00000021, 0xffffffff, 0xffffffde, 0xffffffde, 0x84000000, 0x0000001a,
            0x00000000, 0x0000001a,
        ],
    ),
    (
        0x7fa12345,
        0x805edcbb,
        [
            0x285a0f67, 0xffffdcd9, 0xe35a0f67, 0x00003bb5, 0x805edcbb, 0x7fa12345, 0x7fa12345,
            0x805edcbb, 0x00000001, 0x00000000, 0xff42468a, 0x00bdb976, 0x0bdb9760, 0x0402f6e5,
            0xfc02f6e5, 0x00000001, 0xffffffff, 0xfffffffe, 0x805edcba, 0xa2c485fe, 0x00000001,
            0x00000000, 0x00000001,
        ],
    ),
];
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
fn arity(op: u32) -> usize {
    if op >= 439 { 1 } else { 2 }
}
fn sources(op: u32) -> [u32; 3] {
    [264, if arity(op) == 1 { 128 } else { 265 }, 128]
}
#[test]
fn aliases_match_literal_arithmetic_words() {
    let gpu = NativeCompute::new().unwrap();
    for &(a, b, answers) in CASES {
        for (i, op) in OPS.into_iter().enumerate() {
            assert_eq!(
                render(
                    &gpu,
                    &pixel(op, 11, sources(op), None),
                    [a, b, 0, 0, 0, 0, 0, 0]
                ),
                vec![answers[i].to_le_bytes().to_vec()],
                "op={op} a={a:x} b={b:x}"
            );
        }
    }
}
#[test]
fn alias_operand_positions_and_shared_literals_preserve_words() {
    let gpu = NativeCompute::new().unwrap();
    let (a, b, answers) = CASES[8];
    let inputs = [a, b];
    for (i, op) in OPS.into_iter().enumerate() {
        for position in 0..arity(op) {
            for selector in [264 + position as u32, 24 + position as u32, 255] {
                let mut source = sources(op);
                source[position] = selector;
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(
                            op,
                            8 + position as u32,
                            source,
                            (selector == 255).then_some(inputs[position])
                        ),
                        [a, b, 0, 0, 0, 0, 0, 0]
                    ),
                    vec![answers[i].to_le_bytes().to_vec()],
                    "op={op} position={position} selector={selector}"
                );
            }
        }
        let mut source = [128; 3];
        source[..arity(op)].fill(255);
        assert_eq!(
            render(&gpu, &pixel(op, 11, source, Some(0xffffffff)), [0; 8]),
            vec![CASES[1].2[i].to_le_bytes().to_vec()],
            "shared literal op={op}"
        );
    }
}
