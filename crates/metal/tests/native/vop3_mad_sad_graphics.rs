// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 12.12 MAD24 and sum-of-absolute-difference arithmetic.
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

// Fixed mathematical words: wrapped then saturated, independent of runtime evaluators.
const OPS: [u32; 6] = [322, 323, 346, 347, 348, 349];
const CASES: &[(u32, u32, u32, [u32; 6], [u32; 6])] = &[
    (
        0x00000000,
        0x00000000,
        0x00000000,
        [
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
        ],
        [
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
        ],
    ),
    (
        0xffffffff,
        0x00000000,
        0xffffffff,
        [
            0xffffffff, 0xffffffff, 0x000003fb, 0x03fbffff, 0x0001fffd, 0xfffffffe,
        ],
        [
            0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        0x00112233,
        0x33221100,
        0x00000007,
        [
            0xae0b6307, 0xae0b6307, 0x0000008f, 0x00880007, 0x0000444b, 0x3310eed4,
        ],
        [
            0x7fffffff, 0xffffffff, 0x0000008f, 0x00880007, 0x0000444b, 0x3310eed4,
        ],
    ),
    (
        0x00ffffff,
        0x00000002,
        0x00000003,
        [
            0x00000001, 0x02000001, 0x000002fe, 0x02fb0003, 0x000100ff, 0x01000000,
        ],
        [
            0x00000001, 0x02000001, 0x000002fe, 0x02fb0003, 0x000100ff, 0x01000000,
        ],
    ),
    (
        0x00800000,
        0x00000002,
        0x00000000,
        [
            0xff000000, 0x01000000, 0x00000082, 0x00820000, 0x00000082, 0x007ffffe,
        ],
        [
            0xff000000, 0x01000000, 0x00000082, 0x00820000, 0x00000082, 0x007ffffe,
        ],
    ),
    (
        0x007fffff,
        0x007fffff,
        0xffffffff,
        [
            0xff000000, 0xff000000, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
        [
            0x7fffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        0x00008000,
        0x00010000,
        0xffffffff,
        [
            0x7fffffff, 0x7fffffff, 0x00000080, 0x0080ffff, 0x00008000, 0x00007fff,
        ],
        [
            0x7fffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        0x00ff7fff,
        0x00010000,
        0x00010000,
        [
            0x80000000, 0x80000000, 0x0001027c, 0x027d0000, 0x000180fd, 0x00ff7fff,
        ],
        [
            0x80000000, 0xffffffff, 0x0001027c, 0x027d0000, 0x000180fd, 0x00ff7fff,
        ],
    ),
    (
        0x00800000,
        0x00800000,
        0x80000000,
        [
            0x80000000, 0x80000000, 0x80000000, 0x80000000, 0x80000000, 0x80000000,
        ],
        [
            0x7fffffff, 0xffffffff, 0x80000000, 0x80000000, 0x80000000, 0x80000000,
        ],
    ),
    (
        0xabffffff,
        0xcd000002,
        0x00000003,
        [
            0x00000001, 0x02000001, 0x00000320, 0x031d0003, 0x00012101, 0x21000006,
        ],
        [
            0x00000001, 0x02000001, 0x00000320, 0x031d0003, 0x00012101, 0x21000006,
        ],
    ),
    (
        0x12345678,
        0x12345678,
        0x7fa12345,
        [
            0xbd95fb85, 0xbd95fb85, 0x7fa12345, 0x7fa12345, 0x7fa12345, 0x7fa12345,
        ],
        [
            0x7fffffff, 0xffffffff, 0x7fa12345, 0x7fa12345, 0x7fa12345, 0x7fa12345,
        ],
    ),
    (
        0x80000000,
        0x00000000,
        0x00000000,
        [
            0x00000000, 0x00000000, 0x00000080, 0x00800000, 0x00008000, 0x80000000,
        ],
        [
            0x00000000, 0x00000000, 0x00000080, 0x00800000, 0x00008000, 0x80000000,
        ],
    ),
    (
        0x00010000,
        0x0000ffff,
        0x01020304,
        [
            0x01010304, 0x01010304, 0x01020503, 0x03010304, 0x01030304, 0x01020305,
        ],
        [
            0x7fffffff, 0xffffffff, 0x01020503, 0x03010304, 0x01030304, 0x01020305,
        ],
    ),
    (
        0x01000000,
        0x00000000,
        0xff000000,
        [
            0xff000000, 0xff000000, 0xff000001, 0xff010000, 0xff000100, 0x00000000,
        ],
        [
            0xff000000, 0xff000000, 0xff000001, 0xff010000, 0xff000100, 0xffffffff,
        ],
    ),
    (
        0x00000000,
        0xffffffff,
        0x00000001,
        [
            0x00000001, 0x00000001, 0x000003fd, 0x03fc0001, 0x0001ffff, 0x00000000,
        ],
        [
            0x00000001, 0x00000001, 0x000003fd, 0x03fc0001, 0x0001ffff, 0xffffffff,
        ],
    ),
    (
        0x7fa12345,
        0x805edcbb,
        0x13579bdf,
        [
            0x3bb1ab46, 0xf6b1ab46, 0x13579d52, 0x14ca9bdf, 0x13585612, 0x14155555,
        ],
        [
            0x80000000, 0xffffffff, 0x13579d52, 0x14ca9bdf, 0x13585612, 0x14155555,
        ],
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
fn mad_sad_wrap_saturation_and_signed_cancellation_are_exact() {
    let gpu = NativeCompute::new().unwrap();
    for &(a, b, c, wrapped, saturated) in CASES {
        for (i, op) in OPS.into_iter().enumerate() {
            for (clamp, expected) in [(false, wrapped[i]), (true, saturated[i])] {
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(op, 11, [264, 265, 266], None, clamp),
                        [a, b, c, 0, 0, 0, 0, 0]
                    ),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={op} clamp={clamp} a={a:x} b={b:x} c={c:x}"
                );
            }
        }
    }
}
#[test]
fn mad_sad_every_source_alias_scalar_literal_and_shared_literal() {
    let gpu = NativeCompute::new().unwrap();
    let (a, b, c, wrapped, saturated) = CASES[15];
    let inputs = [a, b, c];
    for (i, op) in OPS.into_iter().enumerate() {
        for (clamp, expected) in [(false, wrapped[i]), (true, saturated[i])] {
            for position in 0..3 {
                for selector in [264 + position as u32, 24 + position as u32, 255] {
                    let mut source = [264, 265, 266];
                    source[position] = selector;
                    assert_eq!(
                        render(
                            &gpu,
                            &pixel(
                                op,
                                8 + position as u32,
                                source,
                                (selector == 255).then_some(inputs[position]),
                                clamp
                            ),
                            [a, b, c, 0, 0, 0, 0, 0]
                        ),
                        vec![expected.to_le_bytes().to_vec()],
                        "op={op} clamp={clamp} position={position} selector={selector}"
                    );
                }
            }
        }
        // All three literal selectors share one word. Equal inputs make SAD zero.
        for clamp in [false, true] {
            let expected = if i < 2 { 2u32 } else { 1u32 };
            assert_eq!(
                render(
                    &gpu,
                    &pixel(op, 11, [255, 255, 255], Some(1), clamp),
                    [0; 8]
                ),
                vec![expected.to_le_bytes().to_vec()]
            );
        }
    }
}
