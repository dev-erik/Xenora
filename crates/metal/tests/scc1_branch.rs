// SPDX-License-Identifier: GPL-3.0-or-later
//! Original AMD RDNA2 SCC branch programs; fixed readback answers.
//! No generated MSL text is used as an oracle. Run only in the coordinated GPU lane.
#![cfg(all(target_arch = "aarch64", target_os = "macos", feature = "gpu-tests"))]

use xenora_metal::*;
use xenora_shader::{
    Architecture::Gfx1030, GraphicsExportType, GraphicsPipelineTypes, PIXEL_ENTRY_POINT,
    VERTEX_ENTRY_POINT, compile, compile_graphics_pipeline_typed,
};
use xenora_texture::TextureFormat;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn validated_gpu() -> NativeCompute {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    NativeCompute::new().unwrap()
}

#[test]
fn compute_scc1_taken_untaken_and_exec_zero_readback() {
    let gpu = validated_gpu();
    // Twelve dispatches: complementary opcodes x SCC states x three EXEC masks.
    for (comparison, scc1_answer, scc0_answer) in
        [(0xbf068080, 7, 0x12345678), (0xbf068081, 0x12345678, 7)]
    {
        for exec in [0xbefe0380, 0xbefe0381, 0xbefe03c1] {
            for (branch, answer) in [(0xbf850002, scc1_answer), (0xbf840002, scc0_answer)] {
                let code = bytes(&[
                    0xbe880387, comparison, exec, branch, 0xbe8803ff,
                    0x12345678, // one eight-byte move, skipped by +2
                    0xbefe03c1, 0x7e020208, 0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000,
                ]);
                assert_eq!(xenora_ref::execute(&code, &[0; 32]).unwrap(), [answer; 32]);
                let shader = compile(Gfx1030, &code).unwrap();
                assert_eq!(
                    gpu.run_words(&shader.msl, xenora_shader::ENTRY_POINT, &[0; 32])
                        .unwrap(),
                    [answer; 32],
                    "branch={branch:08x} comparison={comparison:08x} exec={exec:08x}"
                );
            }
        }
    }
}

#[test]
fn graphics_runtime_scc1_and_complement_preserve_joined_scalar_bits() {
    let gpu = validated_gpu();
    let vertex = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let positions = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
    let colors = [[0.; 4]; 3];
    let mut types = GraphicsPipelineTypes::default();
    types.colors[0] = GraphicsExportType::Uint;
    // Eight draws. Runtime uniform compare: equality comes from raw uint words,
    // including NaN-shaped bits; s32 joins across the conditional write.
    for (a, b, scc1_answer, scc0_answer) in [
        (0, 0, 7u32, 9u32),
        (0, 1, 9, 7),
        (0x7fa12345, 0x7fa12345, 7, 9),
        (0x80000000, 0, 9, 7),
    ] {
        for (branch, expected) in [(0xbf850001, scc1_answer), (0xbf840001, scc0_answer)] {
            let pixel = bytes(&[
                0xbea00387, // s_mov_b32 s32,7
                0xbf061918, // s_cmp_eq_u32 s24,s25 (runtime uniforms)
                branch, 0xbea00389, // conditionally skip s32=9
                0x7e180280, // v12=0, preserving SCC
                0x4a101820, // v_or_b32 v8,s32,v12: integer provenance
                0xf800180f, 0x08080808, 0xbf810000,
            ]);
            let pipeline = compile_graphics_pipeline_typed(Gfx1030, vertex, &pixel, types).unwrap();
            assert_eq!(pipeline.required_uniform_mask, 3);
            let result = gpu
                .submit_typed_triangle_targets(
                    &[TriangleShader {
                        source: &pipeline.shader.msl,
                        entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                    }],
                    &TypedTriangleTargetsPass {
                        draws: &[TriangleDraw {
                            vertices: &positions,
                            colors: &colors,
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
                .unwrap();
            assert_eq!(
                result,
                vec![expected.to_le_bytes().to_vec()],
                "branch={branch:08x} a={a:08x} b={b:08x}"
            );
        }
    }
}
