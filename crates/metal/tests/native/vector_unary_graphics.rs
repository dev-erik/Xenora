// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! Public AMD RDNA 2 ISA guide 12.8: complement, bit reversal and first-bit scans.
//! Signed high-bit search includes the sign bit in its leading run count.
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

fn render(gpu: &NativeCompute, ps: &[u8], a: u32, b: u32) -> Vec<Vec<u8>> {
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
            values: Some([a, b, 0, 0, 0, 0, 0, 0]),
        }],
    )
    .unwrap()
    .wait()
    .unwrap()
}

// Input and five literal outputs: NOT, BFREV, FFBH_U, FFBL, FFBH_I.
const CASES: &[(u32, [u32; 5])] = &[
    (0, [0xffffffff, 0, 0xffffffff, 0xffffffff, 0xffffffff]),
    (0xffffffff, [0, 0xffffffff, 0, 0, 0xffffffff]),
    (1, [0xfffffffe, 0x80000000, 31, 0, 31]),
    (0x80000000, [0x7fffffff, 1, 0, 31, 1]),
    (0x40000000, [0xbfffffff, 2, 1, 30, 1]),
    (0xfffffffe, [1, 0x7fffffff, 0, 1, 31]),
    (0xffff0000, [0xffff, 0xffff, 0, 16, 16]),
    (0x01234567, [0xfedcba98, 0xe6a2c480, 7, 0, 7]),
    (0x7fa12345, [0x805edcba, 0xa2c485fe, 1, 0, 1]),
];

#[test]
fn vector_unary_graphics_exact_words_aliases_and_source_classes() {
    let gpu = NativeCompute::new().unwrap();
    for &(input, answers) in CASES {
        for (index, expected) in answers.into_iter().enumerate() {
            for dst in [8u32, 10] {
                for source in [264u32, 24, 255] {
                    let mut code = vec![0x7e180280, 0x4a101818];
                    code.push(0x7e000000 | dst << 17 | (55 + index as u32) << 9 | source);
                    if source == 255 {
                        code.push(input);
                    }
                    code.extend([0xf800180f, dst * 0x01010101, 0xbf810000]);
                    let ps: Vec<_> = code.into_iter().flat_map(u32::to_le_bytes).collect();
                    assert_eq!(
                        render(&gpu, &ps, input, 0),
                        vec![expected.to_le_bytes().to_vec()],
                        "op={} src={source} dst={dst} input={input:#x}",
                        55 + index
                    );
                }
            }
        }
    }
}

#[test]
fn vector_unary_graphics_loop_preserves_reversed_payloads() {
    let gpu = NativeCompute::new().unwrap();
    // Guest clamps the runtime count; BFREV v8,v8 is its loop-carried update.
    let code = [
        0x83a08418u32,
        0x7e180280,
        0x4a101819,
        0xbf078020,
        0xbf840003,
        0x7e107108,
        0x80a08120,
        0xbf82fffb,
        0xf800180f,
        0x08080808,
        0xbf810000,
    ];
    let ps: Vec<_> = code.into_iter().flat_map(u32::to_le_bytes).collect();
    for (count, expected) in [
        (0, 0x7fa12345u32),
        (1, 0xa2c485fe),
        (2, 0x7fa12345),
        (3, 0xa2c485fe),
        (4, 0x7fa12345),
        (0xffffffff, 0x7fa12345),
    ] {
        assert_eq!(
            render(&gpu, &ps, count, 0x7fa12345),
            vec![expected.to_le_bytes().to_vec()]
        );
    }
}
