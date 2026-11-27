// SPDX-License-Identifier: GPL-3.0-or-later
//! Original assembled scalar programs, with hand-derived result words and flags.
//! No ISA evaluator or translator output supplies expected arithmetic results.
use xenora_metal::*;
use xenora_shader::{
    Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics_pipeline,
};
const ENTRIES: [&str; 2] = [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT];
const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const COLORS: [[f32; 4]; 3] = [[0.; 4]; 3];

#[test]
fn translated_inline_scalar_sources_preserve_exact_words_and_borrow() {
    let gpu = NativeCompute::new().unwrap();
    // Selector, literal bit pattern, bits-A, A-bits (A=0x11223344).
    // All nine positive/negative FP bit words are unsigned-greater than A.
    let cases: &[(u32, u32, u32, u32)] = &[
        (240, 0x3f000000, 0x2dddccbc, 0xd2223344),
        (241, 0xbf000000, 0xadddccbc, 0x52223344),
        (242, 0x3f800000, 0x2e5dccbc, 0xd1a23344),
        (243, 0xbf800000, 0xae5dccbc, 0x51a23344),
        (244, 0x40000000, 0x2eddccbc, 0xd1223344),
        (245, 0xc0000000, 0xaeddccbc, 0x51223344),
        (246, 0x40800000, 0x2f5dccbc, 0xd0a23344),
        (247, 0xc0800000, 0xaf5dccbc, 0x50a23344),
        (248, 0x3e22f983, 0x2d00c63f, 0xd2ff39c1),
    ];
    for &(selector, bits, result_left, result_right) in cases {
        let a = 0x11223344;
        let b = bits;
        for inline_left in [false, true] {
            let result = if inline_left {
                result_left
            } else {
                result_right
            };
            let changed = Some(!inline_left);
            for separate_destination in [false, true] {
                for constants in [false, true] {
                    let mut ps =
                        include_bytes!("../../../../fixtures/rdna2/x9-scalar-mul-ps.bin").to_vec();
                    let dst = if separate_destination { 33 } else { 32 };
                    ps[12..16].copy_from_slice(
                        &(0x80800000
                            | dst << 16
                            | if inline_left {
                                32 << 8 | selector
                            } else {
                                selector << 8 | 32
                            })
                        .to_le_bytes(),
                    );
                    ps[16] = dst as u8;
                    if constants {
                        let mut prefix: Vec<u8> = [0xbea003ffu32, a, 0xbea103ff, b]
                            .into_iter()
                            .flat_map(u32::to_le_bytes)
                            .collect();
                        prefix.extend_from_slice(&ps[8..]);
                        ps = prefix;
                    }
                    let pipeline =
                        compile_graphics_pipeline(Architecture::Gfx1030, VS, &ps).unwrap();
                    for flag in [false, true] {
                        for matches in [true, false] {
                            let outgoing = changed.unwrap_or(flag);
                            let draws = [TriangleDraw {
                                vertices: &POS,
                                colors: &COLORS,
                                viewport: [1, 1],
                            }];
                            let states = [TriangleState::default()];
                            let shaders = [TriangleShader {
                                source: &pipeline.shader.msl,
                                entries: ENTRIES,
                            }];
                            let uniforms = [DrawUniforms {
                                required_mask: pipeline.required_uniform_mask,
                                values: Some([
                                    a,
                                    b,
                                    u32::from(flag),
                                    result ^ u32::from(!matches),
                                    0,
                                    0,
                                    0,
                                    0,
                                ]),
                            }];
                            let pass = TriangleTargetsPass {
                                draws: &draws,
                                states: &states,
                                extent: [1, 1],
                                clears: &[[7, 11, 13, 255]],
                                clear_depth: None,
                            };
                            let actual = gpu
                                .submit_view_binding_triangle_targets_with_uniforms_rgba8(
                                    &shaders,
                                    &pass,
                                    &[vec![]],
                                    None,
                                    &[None],
                                    &uniforms,
                                )
                                .unwrap()
                                .wait()
                                .unwrap();
                            assert_eq!(
                                actual,
                                vec![vec![
                                    if matches { 255 } else { 0 },
                                    if outgoing { 255 } else { 0 },
                                    0,
                                    255
                                ]]
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn translated_inline_vector_moves_export_exact_uint_bits() {
    use xenora_shader::{
        GraphicsExportType, GraphicsPipelineTypes, compile_graphics_pipeline_typed,
    };
    use xenora_texture::TextureFormat;
    let gpu = NativeCompute::new().unwrap();
    let words = [
        0x3f000000u32,
        0xbf000000,
        0x3f800000,
        0xbf800000,
        0x40000000,
        0xc0000000,
        0x40800000,
        0xc0800000,
        0x3e22f983,
    ];
    for (index, expected) in words.into_iter().enumerate() {
        let ps: Vec<_> = [
            0x7e100200 | (240 + index as u32),
            0x7e120280,
            0xf800180f,
            0x09090908,
            0xbf810000,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        let mut types = GraphicsPipelineTypes::default();
        types.colors[0] = GraphicsExportType::Uint;
        let pipeline =
            compile_graphics_pipeline_typed(Architecture::Gfx1030, VS, &ps, types).unwrap();
        assert_eq!(pipeline.required_uniform_mask, 0);
        let actual = gpu
            .submit_typed_triangle_targets(
                &[TriangleShader {
                    source: &pipeline.shader.msl,
                    entries: ENTRIES,
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
                        clear: TypedColorClear::Uint([0; 4]),
                        blend_enabled: false,
                    }],
                    clear_depth: None,
                },
                &[vec![]],
                None,
                &[None],
                &[DrawUniforms::default()],
            )
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(actual, vec![expected.to_le_bytes().to_vec()]);
    }
}

#[test]
fn translated_inline_compute_sources_match_literal_words() {
    let gpu = NativeCompute::new().unwrap();
    let cases = [
        (240, 0x3f000000u32, 0xc0ffffff),
        (241, 0xbf000000, 0x40ffffff),
        (242, 0x3f800000, 0xc07fffff),
        (243, 0xbf800000, 0x407fffff),
        (244, 0x40000000, 0xbfffffff),
        (245, 0xc0000000, 0x3fffffff),
        (246, 0x40800000, 0xbf7fffff),
        (247, 0xc0800000, 0x3f7fffff),
        (248, 0x3e22f983, 0xc1dd067c),
    ];
    for (selector, bits, inverted) in cases {
        for (body, expected) in [
            (vec![0x7e040200 | selector], bits),
            (vec![0xbe880300 | selector, 0x7e040208], bits),
            (vec![0xbe880700 | selector, 0x7e040208], inverted),
            (vec![0x4a040200 | selector], bits),
            (vec![0x7e0602ff, bits, 0x7d840600 | selector, 0x7e04026a], 1),
            (
                vec![0x7e0602ff, inverted, 0x7d840600 | selector, 0x7e04026a],
                0,
            ),
        ] {
            let mut code = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
            code.extend(body);
            code.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
            let code: Vec<_> = code.into_iter().flat_map(u32::to_le_bytes).collect();
            let compiled = xenora_shader::compile(Architecture::Gfx1030, &code).unwrap();
            let actual = gpu
                .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[0])
                .unwrap();
            assert_eq!(actual, [expected]);
        }
    }
}
