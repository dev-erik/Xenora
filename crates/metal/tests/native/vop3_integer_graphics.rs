// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 12.12 full-width products and three-source integer operations.
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

// a,b,c; literal LO, HI unsigned, HI signed, BFI, ADD3, AND_OR words.
const CASES: &[(u32, u32, u32, [u32; 6])] = &[
    (
        0x00000000,
        0x00000000,
        0x00000000,
        [
            0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
        ],
    ),
    (
        0xffffffff,
        0xffffffff,
        0xffffffff,
        [
            0x00000001, 0xfffffffe, 0x00000000, 0xffffffff, 0xfffffffd, 0xffffffff,
        ],
    ),
    (
        0x80000000,
        0x00000002,
        0x00000007,
        [
            0x00000000, 0x00000001, 0xffffffff, 0x00000007, 0x80000009, 0x00000007,
        ],
    ),
    (
        0x80000000,
        0x80000000,
        0x00000007,
        [
            0x00000000, 0x40000000, 0x40000000, 0x80000007, 0x00000007, 0x80000007,
        ],
    ),
    (
        0x01000000,
        0x00000100,
        0x00000009,
        [
            0x00000000, 0x00000001, 0x00000001, 0x00000009, 0x01000109, 0x00000009,
        ],
    ),
    (
        0x0f0f0f0f,
        0xaaaaaaaa,
        0x55555555,
        [
            0xf5f5f5f6, 0x0a0a0a09, 0xfafafafa, 0x5a5a5a5a, 0x0f0f0f0e, 0x5f5f5f5f,
        ],
    ),
    (
        0x7fa12345,
        0x805edcbb,
        0x13579bdf,
        [
            0x285a0f67, 0x3fffdcd9, 0xc05eb994, 0x0056989b, 0x13579bdf, 0x13579bdf,
        ],
    ),
    (
        0xffffffff,
        0x00000001,
        0x00000007,
        [
            0xffffffff, 0x00000000, 0xffffffff, 0x00000001, 0x00000007, 0x00000007,
        ],
    ),
];
const OPS: [u32; 6] = [361, 362, 364, 330, 877, 881];
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
fn vop3_products_and_ternary_words_use_every_source_and_capture_aliases() {
    let gpu = NativeCompute::new().unwrap();
    for &(a, b, c, answers) in CASES {
        let input = [a, b, c];
        for (column, opcode) in OPS.into_iter().enumerate() {
            let count = if column < 3 { 2 } else { 3 };
            let base = [264, 265, if count == 2 { 128 } else { 266 }];
            let expected = vec![answers[column].to_le_bytes().to_vec()];
            for dst in [8, 9, 10, 11] {
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(opcode, dst, base, None),
                        [a, b, c, 0, 0, 0, 0, 0]
                    ),
                    expected,
                    "op={opcode} alias={dst} input={input:08x?}"
                );
            }
            for position in 0..count {
                for literal in [false, true] {
                    let mut source = base;
                    source[position] = if literal { 255 } else { 24 + position as u32 };
                    for dst in [8, 11] {
                        assert_eq!(
                            render(
                                &gpu,
                                &pixel(opcode, dst, source, literal.then_some(input[position])),
                                [a, b, c, 0, 0, 0, 0, 0]
                            ),
                            expected,
                            "op={opcode} position={position} literal={literal} dst={dst} input={input:08x?}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn vop3_shared_literals_and_inline_payloads_keep_identity_and_bits() {
    let gpu = NativeCompute::new().unwrap();
    // Shared literals occupy one payload even in multiple active slots.
    for (opcode, sources, literal, expected) in [
        (361, [255, 255, 128], 0xffffffff, 1u32),
        (362, [255, 255, 128], 0xffffffff, 0xfffffffe),
        (364, [255, 255, 128], 0xffffffff, 0),
        (330, [255, 255, 255], 0x7fa12345, 0x7fa12345),
        (877, [255, 255, 255], 0xffffffff, 0xfffffffd),
        (881, [255, 255, 255], 0x7fa12345, 0x7fa12345),
    ] {
        assert_eq!(
            render(&gpu, &pixel(opcode, 11, sources, Some(literal)), [0; 8]),
            vec![expected.to_le_bytes().to_vec()]
        );
    }
    // A float-named inline word is integer input, not numeric float conversion.
    for (opcode, sources, expected) in [
        (361, [248, 129, 128], 0x3e22f983u32),
        (362, [242, 193, 128], 0x3f7fffff),
        (364, [241, 129, 128], 0xffffffff),
        (330, [193, 248, 128], 0x3e22f983),
        (877, [248, 129, 193], 0x3e22f983),
        (881, [242, 193, 241], 0xbf800000),
    ] {
        assert_eq!(
            render(&gpu, &pixel(opcode, 11, sources, None), [0; 8]),
            vec![expected.to_le_bytes().to_vec()]
        );
    }
}
#[test]
fn vop3_loop_writes_keep_nan_shaped_integer_payloads() {
    let gpu = NativeCompute::new().unwrap();
    // Increment v8 by3 for min(count,4) iterations through ADD3.
    let code = [
        0x83a08418u32,
        0x7e180280,
        0x4a101819,
        0xbf078020,
        0xbf840004,
        0xd76d0008,
        264 | 129 << 9 | 130 << 18,
        0x80a08120,
        0xbf82fffa,
        0xf800180f,
        0x08080808,
        0xbf810000,
    ];
    let ps: Vec<_> = code.into_iter().flat_map(u32::to_le_bytes).collect();
    for (count, expected) in [
        (0, 0x7fa12345u32),
        (1, 0x7fa12348),
        (2, 0x7fa1234b),
        (3, 0x7fa1234e),
        (4, 0x7fa12351),
        (0xffffffff, 0x7fa12351),
    ] {
        assert_eq!(
            render(&gpu, &ps, [count, 0x7fa12345, 0, 0, 0, 0, 0, 0]),
            vec![expected.to_le_bytes().to_vec()]
        );
    }
}
