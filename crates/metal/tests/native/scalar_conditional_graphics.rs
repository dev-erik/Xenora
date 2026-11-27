// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! AMD RDNA2 scalar bit updates/tests and conditional writes (12.2-12.4).
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

fn finish(mut code: Vec<u32>, scalar: u32) -> Vec<u8> {
    code.extend([
        0x7e180280,
        0x4a101800 | scalar,
        0xf800180f,
        0x08080808,
        0xbf810000,
    ]);
    code.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn check(gpu: &NativeCompute, code: Vec<u32>, scalar: u32, input: [u32; 3], expected: u32) {
    assert_eq!(
        render(
            gpu,
            &finish(code.clone(), scalar),
            [input[0], input[1], input[2], 0, 0, 0, 0, 0]
        ),
        vec![expected.to_le_bytes().to_vec()],
        "code={code:08x?} input={input:08x?}"
    );
}
#[test]
fn scalar_bit_updates_keep_old_bits_aliases_and_both_scc_values() {
    let gpu = NativeCompute::new().unwrap();
    // old,index,clear,set; literals do not use production/reference evaluators.
    let cases = [
        (0, 0, 0, 1),
        (0xffffffff, 0, 0xfffffffe, 0xffffffff),
        (0xffffffff, 31, 0x7fffffff, 0xffffffff),
        (0, 31, 0, 0x80000000),
        (0x80000001, 32, 0x80000000, 0x80000001),
        (0x80000001, 33, 0x80000001, 0x80000003),
        (0x7fa12345, 0xffffffff, 0x7fa12345, 0xffa12345),
        (0xaaaaaaaa, 63, 0x2aaaaaaa, 0xaaaaaaaa),
    ];
    for (old, index, clear, set) in cases {
        for (opcode, expected) in [(27, clear), (29, set)] {
            for flag in [0, 1] {
                let code = vec![
                    0xbea00318,
                    0xbf0d801a,
                    0xbea00000 | opcode << 8 | 25,
                    0x85238081,
                ];
                check(&gpu, code.clone(), 32, [old, index, flag], expected);
                check(&gpu, code, 35, [old, index, flag], flag);
            }
        }
    }
    // Old destination supplies both data and masked index before the write.
    for (old, clear, set) in [
        (0, 0, 1),
        (1, 1, 3),
        (31, 31, 0x8000001f),
        (0xffffffff, 0x7fffffff, 0xffffffff),
    ] {
        for (opcode, expected) in [(27, clear), (29, set)] {
            check(
                &gpu,
                vec![0xbea00318, 0xbea00020 | opcode << 8],
                32,
                [old, 0, 0],
                expected,
            );
        }
    }
}
#[test]
fn scalar_bit_tests_replace_scc_without_changing_sources() {
    let gpu = NativeCompute::new().unwrap();
    for (word, index, bit) in [
        (0, 0, 0),
        (0xffffffff, 31, 1),
        (0x80000001, 33, 0),
        (0x80000001, 32, 1),
        (0x80000000, 0xffffffff, 1),
    ] {
        for (opcode, expected) in [(12, 1 - bit), (13, bit)] {
            let code = vec![
                0xbf068080,
                0xbf000000 | opcode << 16 | 25 << 8 | 24,
                0x85208081,
            ];
            check(&gpu, code, 32, [word, index, 0], expected);
        }
    }
}
#[test]
fn scalar_conditional_moves_select_words_and_preserve_predicates() {
    let gpu = NativeCompute::new().unwrap();
    for flag in [0, 1] {
        for (old, source) in [
            (0x7fa12345, 0xffa54321),
            (0, 0xffffffff),
            (0xffffffff, 0x80000000),
        ] {
            for source_reg in [25, 32, 33] {
                let code = vec![
                    0xbea00318,
                    0xbea10319,
                    0xbf0d801a,
                    0xbea00500 | source_reg,
                    0x85238081,
                ];
                let expected = if flag == 0 || source_reg == 32 {
                    old
                } else {
                    source
                };
                check(&gpu, code.clone(), 32, [old, source, flag], expected);
                check(&gpu, code, 35, [old, source, flag], flag);
            }
        }
        for (immediate, word) in [
            (0, 0),
            (1, 1),
            (0x7fff, 0x7fff),
            (0x8000, 0xffff8000),
            (0xffff, 0xffffffff),
        ] {
            let code = vec![0xbea00318, 0xbf0d801a, 0xb1200000 | immediate, 0x85238081];
            check(
                &gpu,
                code.clone(),
                32,
                [0x7fa12345, 0, flag],
                if flag == 0 { 0x7fa12345 } else { word },
            );
            check(&gpu, code, 35, [0x7fa12345, 0, flag], flag);
        }
    }
}
#[test]
fn scalar_bit_updates_and_conditional_moves_carry_state_through_loops() {
    let gpu = NativeCompute::new().unwrap();
    // min(count,4); accumulated bits must survive each backedge.
    for (count, set, clear) in [
        (0, 0, 0xffffffff),
        (1, 2, 0xfffffffd),
        (2, 6, 0xfffffff9),
        (3, 14, 0xfffffff1),
        (4, 30, 0xffffffe1),
        (0xffffffff, 30, 0xffffffe1),
    ] {
        for (opcode, seed, expected) in [(27, 193, clear), (29, 128, set)] {
            let code = vec![
                0x83a08418,
                0xbea10300 | seed,
                0xbf078020,
                0xbf840003,
                0xbea10020 | opcode << 8,
                0x80a08120,
                0xbf82fffb,
            ];
            check(&gpu, code, 33, [count, 0, 0], expected);
        }
        // Odd counter values replace the accumulator; the final odd value is1.
        let code = vec![
            0x83a08418, 0xbea10319, 0xbf078020, 0xbf840004, 0xbf0d8020, 0xbea10520, 0x80a08120,
            0xbf82fffa,
        ];
        check(
            &gpu,
            code,
            33,
            [count, 0x7fa12345, 0],
            if count == 0 { 0x7fa12345 } else { 1 },
        );
    }
}
