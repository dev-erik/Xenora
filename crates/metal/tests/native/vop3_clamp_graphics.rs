// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 12.12 integer clamp saturation modes.
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

// Mathematical literal answers are independent of either runtime reference.
const OPS: [u32; 5] = [265, 267, 293, 294, 295];
const CASES: &[(u32, u32, [u32; 5], [u32; 5])] = &[
    (
        0x00000000,
        0x00000000,
        [0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000],
        [0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000],
    ),
    (
        0xffffffff,
        0x00000001,
        [0xffffffff, 0x00ffffff, 0xffffffff, 0xfffffffe, 0x00000000],
        [0xffffffff, 0x00ffffff, 0x00000000, 0xfffffffe, 0x00000002],
    ),
    (
        0x80000000,
        0x80000000,
        [0x00000000, 0x00000000, 0xffffffff, 0x00000000, 0x00000000],
        [0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000],
    ),
    (
        0x00000001,
        0x00000002,
        [0x00000002, 0x00000002, 0x00000003, 0x00000000, 0x00000001],
        [0x00000002, 0x00000002, 0x00000003, 0xffffffff, 0x00000001],
    ),
    (
        0x00000000,
        0x00000001,
        [0x00000000, 0x00000000, 0x00000001, 0x00000000, 0x00000001],
        [0x00000000, 0x00000000, 0x00000001, 0xffffffff, 0x00000001],
    ),
    (
        0x00000001,
        0x00000000,
        [0x00000000, 0x00000000, 0x00000001, 0x00000001, 0x00000000],
        [0x00000000, 0x00000000, 0x00000001, 0x00000001, 0xffffffff],
    ),
    (
        0xffffffff,
        0xffffffff,
        [0x00000001, 0xffffffff, 0xffffffff, 0x00000000, 0x00000000],
        [0x00000001, 0xfe000001, 0xfffffffe, 0x00000000, 0x00000000],
    ),
    (
        0x007fffff,
        0x007fffff,
        [0x7fffffff, 0xffffffff, 0x00fffffe, 0x00000000, 0x00000000],
        [0xff000001, 0xff000001, 0x00fffffe, 0x00000000, 0x00000000],
    ),
    (
        0x00800000,
        0x007fffff,
        [0x80000000, 0xffffffff, 0x00ffffff, 0x00000001, 0x00000000],
        [0x00800000, 0xff800000, 0x00ffffff, 0x00000001, 0xffffffff],
    ),
    (
        0x00800000,
        0x00800000,
        [0x7fffffff, 0xffffffff, 0x01000000, 0x00000000, 0x00000000],
        [0x00000000, 0x00000000, 0x01000000, 0x00000000, 0x00000000],
    ),
    (
        0x00ffffff,
        0x00000002,
        [0xfffffffe, 0x01fffffe, 0x01000001, 0x00fffffd, 0x00000000],
        [0xfffffffe, 0x01fffffe, 0x01000001, 0x00fffffd, 0xff000003],
    ),
    (
        0x00800000,
        0x00000100,
        [0x80000000, 0x80000000, 0x00800100, 0x007fff00, 0x00000000],
        [0x80000000, 0x80000000, 0x00800100, 0x007fff00, 0xff800100],
    ),
    (
        0x00800001,
        0x00000100,
        [0x80000100, 0x80000100, 0x00800101, 0x007fff01, 0x00000000],
        [0x80000100, 0x80000100, 0x00800101, 0x007fff01, 0xff8000ff],
    ),
    (
        0x007fffff,
        0x00000100,
        [0x7fffff00, 0x7fffff00, 0x008000ff, 0x007ffeff, 0x00000000],
        [0x7fffff00, 0x7fffff00, 0x008000ff, 0x007ffeff, 0xff800101],
    ),
    (
        0x007fffff,
        0x00000101,
        [0x7fffffff, 0x807ffeff, 0x00800100, 0x007ffefe, 0x00000000],
        [0x807ffeff, 0x807ffeff, 0x00800100, 0x007ffefe, 0xff800102],
    ),
    (
        0x0000ffff,
        0x00010000,
        [0x7fffffff, 0xffff0000, 0x0001ffff, 0x00000000, 0x00000001],
        [0xffff0000, 0xffff0000, 0x0001ffff, 0xffffffff, 0x00000001],
    ),
    (
        0x00010000,
        0x00010000,
        [0x7fffffff, 0xffffffff, 0x00020000, 0x00000000, 0x00000000],
        [0x00000000, 0x00000000, 0x00020000, 0x00000000, 0x00000000],
    ),
    (
        0xab800000,
        0xcd000100,
        [0x80000000, 0x80000000, 0xffffffff, 0x00000000, 0x21800100],
        [0x80000000, 0x80000000, 0x78800100, 0xde7fff00, 0x21800100],
    ),
    (
        0x1200ffff,
        0x34010000,
        [0x7fffffff, 0xffff0000, 0x4601ffff, 0x00000000, 0x22000001],
        [0xffff0000, 0xffff0000, 0x4601ffff, 0xddffffff, 0x22000001],
    ),
];
fn pixel(op: u32, dst: u32, source: [u32; 3], literal: Option<u32>, clamp: bool) -> Vec<u8> {
    let mut code = vec![
        0x7e180280,
        0x4a101818,
        0x4a121819,
        0x4a14181a,
        0xd4000000 | op << 16 | dst | if clamp { 0x8000 } else { 0 },
        source[0] | source[1] << 9 | source[2] << 18,
    ];
    if let Some(word) = literal {
        code.push(word);
    }
    code.extend([0xf800180f, dst * 0x01010101, 0xbf810000]);
    code.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn clamp_saturates_before_narrowing_and_clear_mode_wraps() {
    let gpu = NativeCompute::new().unwrap();
    for &(a, b, saturated, wrapped) in CASES {
        for (i, op) in OPS.into_iter().enumerate() {
            for (clamp, expected) in [(true, saturated[i]), (false, wrapped[i])] {
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(op, 11, [264, 265, 128], None, clamp),
                        [a, b, 0, 0, 0, 0, 0, 0]
                    ),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={op} clamp={clamp} a={a:x} b={b:x}"
                );
            }
        }
    }
}
#[test]
fn clamp_captures_aliases_and_both_scalar_literal_sources() {
    let gpu = NativeCompute::new().unwrap();
    let (a, b, answers, _) = CASES[8];
    let inputs = [a, b];
    for (i, op) in OPS.into_iter().enumerate() {
        for position in 0..2 {
            for selector in [264 + position as u32, 24 + position as u32, 255] {
                let mut source = [264, 265, 128];
                source[position] = selector;
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(
                            op,
                            8 + position as u32,
                            source,
                            (selector == 255).then_some(inputs[position]),
                            true
                        ),
                        [a, b, 0, 0, 0, 0, 0, 0]
                    ),
                    vec![answers[i].to_le_bytes().to_vec()],
                    "op={op} position={position} selector={selector}"
                );
            }
        }
        assert_eq!(
            render(
                &gpu,
                &pixel(op, 11, [255, 255, 128], Some(0xffffffff), true),
                [0; 8]
            ),
            vec![CASES[6].2[i].to_le_bytes().to_vec()]
        );
    }
}
