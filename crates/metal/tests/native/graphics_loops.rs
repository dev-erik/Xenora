// SPDX-License-Identifier: GPL-3.0-or-later
//! Original assembled countdown shaders, with literal independent pixel oracles.
use xenora_metal::*;
use xenora_shader::{
    Architecture::Gfx1030, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics_pipeline,
};
const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const COUNTS: [u32; 8] = [0, 1, 2, 3, 4, 9, 0xffff_ffff, 0];

fn pixel_grid(ps: &[u8], words: &[[u32; 8]], expected: &[[u8; 4]], bindings: Vec<SampledBinding>) {
    let gpu = NativeCompute::new().unwrap();
    pixel_grid_on(&gpu, ps, words, expected, bindings);
}
fn pixel_grid_on(
    gpu: &NativeCompute,
    ps: &[u8],
    words: &[[u32; 8]],
    expected: &[[u8; 4]],
    bindings: Vec<SampledBinding>,
) {
    let vs = if bindings.is_empty() {
        VS
    } else {
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin")
    };
    let compiled = compile_graphics_pipeline(Gfx1030, vs, ps).unwrap();
    let uv = [[0.5, 0.5, 0., 1.]; 3];
    let draws: Vec<_> = words
        .iter()
        .map(|_| TriangleDraw {
            vertices: &POS,
            colors: &uv,
            viewport: [words.len(), 1],
        })
        .collect();
    let states: Vec<_> = (0..words.len())
        .map(|x| TriangleState {
            scissor: Some([x, 0, 1, 1]),
            ..Default::default()
        })
        .collect();
    let shaders: Vec<_> = words
        .iter()
        .map(|_| TriangleShader {
            source: &compiled.shader.msl,
            entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
        })
        .collect();
    let uniforms: Vec<_> = words
        .iter()
        .map(|values| DrawUniforms {
            required_mask: compiled.required_uniform_mask,
            values: Some(*values),
        })
        .collect();
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [words.len(), 1],
        clears: &[[17, 31, 47, 255]],
        clear_depth: None,
    };
    let output = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![bindings; words.len()],
            None,
            &[None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(output, vec![expected.as_flattened().to_vec()]);
}

#[test]
fn translated_counted_loop_vgpr_and_body_diamond_literal_pixels() {
    let words: Vec<_> = COUNTS.iter().map(|n| [*n, 0, 0, 0, 0, 0, 0, 0]).collect();
    pixel_grid(
        include_bytes!("../../../../fixtures/rdna2/x9-loop-vgpr-ps.bin"),
        &words,
        &[
            [0, 0, 0, 255],
            [64, 64, 64, 255],
            [128, 128, 128, 255],
            [191, 191, 191, 255],
            [255, 255, 255, 255],
            [255, 255, 255, 255],
            [255, 255, 255, 255],
            [0, 0, 0, 255],
        ],
        vec![],
    );
    pixel_grid(
        include_bytes!("../../../../fixtures/rdna2/x9-loop-diamond-ps.bin"),
        &words,
        &[
            [0, 0, 0, 255],
            [32, 32, 32, 255],
            [96, 96, 96, 255],
            [128, 128, 128, 255],
            [159, 159, 159, 255],
            [159, 159, 159, 255],
            [159, 159, 159, 255],
            [0, 0, 0, 255],
        ],
        vec![],
    );
}

#[test]
fn translated_counted_loop_scalar_min_order_ties_and_final_header_scc() {
    let results = [3, 8, 13, 18, 23, 23, 23, 3];
    let normal_flags = [255, 255, 255, 255, 0, 0, 0, 255];
    let reversed_flags = [0, 0, 0, 0, 0, 255, 255, 0];
    for (ps, flags) in [
        (
            include_bytes!("../../../../fixtures/rdna2/x9-loop-scalar-ps.bin").as_slice(),
            normal_flags,
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-loop-scalar-reversed-min-ps.bin")
                .as_slice(),
            reversed_flags,
        ),
    ] {
        let mut words = Vec::new();
        let mut expected = Vec::new();
        for i in 0..8 {
            for mismatch in [false, true] {
                words.push([
                    COUNTS[i],
                    results[i] + u32::from(mismatch),
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ]);
                // Blue is final header SCC: false even on the zero-trip path.
                expected.push([if mismatch { 0 } else { 255 }, flags[i], 0, 255]);
            }
        }
        pixel_grid(ps, &words, &expected, vec![]);
    }
}

#[test]
fn translated_counted_loop_parallel_scalar_swap() {
    let pairs = [
        (7, 11),
        (11, 7),
        (7, 11),
        (11, 7),
        (7, 11),
        (7, 11),
        (7, 11),
        (7, 11),
    ];
    let mut words = Vec::new();
    let mut expected = Vec::new();
    for i in 0..8 {
        for mismatch in [false, true] {
            words.push([
                COUNTS[i],
                pairs[i].0,
                pairs[i].1 + u32::from(mismatch),
                0,
                0,
                0,
                0,
                0,
            ]);
            expected.push(if mismatch {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            });
        }
    }
    pixel_grid(
        include_bytes!("../../../../fixtures/rdna2/x9-loop-swap-ps.bin"),
        &words,
        &expected,
        vec![],
    );
}

#[test]
fn translated_counted_loop_vertex_geometry_literal_columns() {
    let gpu = NativeCompute::new().unwrap();
    let compiled = compile_graphics_pipeline(
        Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-loop-geometry-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let positions = [
        [-1., -1., 0.5, 1.],
        [-0.5, -1., 0.5, 1.],
        [-1., 1., 0.5, 1.],
        [-1., 1., 0.5, 1.],
        [-0.5, -1., 0.5, 1.],
        [-0.5, 1., 0.5, 1.],
    ];
    let colors = [[0.; 4]; 6];
    for (count, first) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (9, 4), (0, 0)] {
        let draws = [TriangleDraw {
            vertices: &positions,
            colors: &colors,
            viewport: [8, 2],
        }];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &[TriangleState::default()],
            extent: [8, 2],
            clears: &[[17, 31, 47, 255]],
            clear_depth: None,
        };
        let output = gpu
            .submit_view_binding_triangle_targets_with_uniforms_rgba8(
                &[TriangleShader {
                    source: &compiled.shader.msl,
                    entries: [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT],
                }],
                &pass,
                &[vec![]],
                None,
                &[None],
                &[DrawUniforms {
                    required_mask: 1,
                    values: Some([count, 0, 0, 0, 0, 0, 0, 0]),
                }],
            )
            .unwrap()
            .wait()
            .unwrap();
        let mut row = [[17, 31, 47, 255]; 8];
        row[first] = [255, 0, 0, 255];
        row[first + 1] = [255, 0, 0, 255];
        assert_eq!(output, vec![row.as_flattened().repeat(2)], "count={count}");
    }
}

#[test]
fn translated_counted_loop_dynamic_sample_union() {
    let gpu = NativeCompute::new().unwrap();
    let mut bindings = Vec::new();
    for (slot, color) in [(0, [255, 0, 0, 255]), (1, [0, 255, 0, 255])] {
        let texture = MetalRgba8Texture::upload(
            gpu.device(),
            MetalRgba8TextureDesc::new(1, 1, 1).unwrap(),
            &[Rgba8MipLevel::new_packed(0, 1, 1, &color)],
        )
        .unwrap();
        let sampler =
            MetalSampler::new(gpu.device(), xenora_texture::SamplerDesc::nearest_clamp()).unwrap();
        bindings.push(SampledBinding::Whole(
            BoundSampledTexture::bind(
                texture,
                sampler,
                BindingSlot::new(slot),
                BindingSlot::new(slot),
            )
            .unwrap(),
        ));
    }
    let ps = include_bytes!("../../../../fixtures/rdna2/x9-loop-sampled-ps.bin");
    let compiled = compile_graphics_pipeline(
        Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        ps,
    )
    .unwrap();
    assert_eq!(compiled.sampled_pairs, 0x09);
    let words: Vec<_> = COUNTS.iter().map(|n| [*n, 0, 0, 0, 0, 0, 0, 0]).collect();
    // Every positive countdown finishes with counter=1, selecting pair1.
    // The count=2+ paths also execute pair0 before the final pair1 overwrite.
    pixel_grid_on(
        &gpu,
        ps,
        &words,
        &[
            [0, 0, 0, 0],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 0, 0],
        ],
        bindings,
    );
}

#[test]
fn translated_counted_loop_preheader_merge_then_header_recurrence() {
    // Rows are literal results for n=0,1,2,3,4,9,MAX,0; both preheader
    // choices reach the header directly, without a separate merge block.
    let results = [
        [3, 8, 13, 18, 23, 23, 23, 3],
        [7, 12, 17, 22, 27, 27, 27, 7],
    ];
    let mut words = Vec::new();
    let mut expected = Vec::new();
    for (select, row) in results.into_iter().enumerate() {
        for i in 0..8 {
            for mismatch in [false, true] {
                words.push([
                    COUNTS[i],
                    select as u32,
                    row[i] + u32::from(mismatch),
                    0,
                    0,
                    0,
                    0,
                    0,
                ]);
                expected.push(if mismatch {
                    [0, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                });
            }
        }
    }
    pixel_grid(
        include_bytes!("../../../../fixtures/rdna2/x9-loop-preheader-merge-ps.bin"),
        &words,
        &expected,
        vec![],
    );
}

#[test]
fn translated_counted_loop_body_diamond_swap_join_across_iterations() {
    // n=2: swap (7,11)->(11,7), then increment ->(12,7).
    // n=3/4 first increment one/two times, then swap, then increment.
    let pairs = [
        (7, 11),
        (8, 11),
        (12, 7),
        (12, 8),
        (12, 9),
        (12, 9),
        (12, 9),
        (7, 11),
    ];
    let mut words = Vec::new();
    let mut expected = Vec::new();
    for i in 0..8 {
        for (delta_a, delta_b, color) in [
            (0, 0, [255, 255, 0, 255]),
            (1, 0, [0, 255, 0, 255]),
            (0, 1, [255, 0, 0, 255]),
        ] {
            words.push([
                COUNTS[i],
                pairs[i].0 + delta_a,
                pairs[i].1 + delta_b,
                0,
                0,
                0,
                0,
                0,
            ]);
            expected.push(color);
        }
    }
    pixel_grid(
        include_bytes!("../../../../fixtures/rdna2/x9-loop-diamond-swap-ps.bin"),
        &words,
        &expected,
        vec![],
    );
}

#[test]
fn translated_counted_loop_sample_accumulator_observes_both_pairs_and_iterations() {
    let gpu = NativeCompute::new().unwrap();
    let mut bindings = Vec::new();
    for (slot, color) in [(0, [64, 0, 0, 255]), (1, [0, 64, 0, 255])] {
        let texture = MetalRgba8Texture::upload(
            gpu.device(),
            MetalRgba8TextureDesc::new(1, 1, 1).unwrap(),
            &[Rgba8MipLevel::new_packed(0, 1, 1, &color)],
        )
        .unwrap();
        let sampler =
            MetalSampler::new(gpu.device(), xenora_texture::SamplerDesc::nearest_clamp()).unwrap();
        bindings.push(SampledBinding::Whole(
            BoundSampledTexture::bind(
                texture,
                sampler,
                BindingSlot::new(slot),
                BindingSlot::new(slot),
            )
            .unwrap(),
        ));
    }
    let words: Vec<_> = COUNTS.iter().map(|n| [*n, 0, 0, 0, 0, 0, 0, 0]).collect();
    // n=2 includes red from pair0 AND green from pair1. Further iterations
    // add green without saturation, exposing skipped or stale sample results.
    pixel_grid_on(
        &gpu,
        include_bytes!("../../../../fixtures/rdna2/x9-loop-sampled-accum-ps.bin"),
        &words,
        &[
            [0, 0, 0, 255],
            [0, 64, 0, 255],
            [64, 64, 0, 255],
            [64, 128, 0, 255],
            [64, 192, 0, 255],
            [64, 192, 0, 255],
            [64, 192, 0, 255],
            [0, 0, 0, 255],
        ],
        bindings,
    );
}

#[test]
fn translated_unary_loop_carried_scalar_keeps_iteration_values() {
    let mut ps = include_bytes!("../../../../fixtures/rdna2/x9-loop-scalar-ps.bin").to_vec();
    ps[40..44].copy_from_slice(&0xbea10721u32.to_le_bytes()); // NOT s33,s33
    let cases = [
        (0, 3, 255),
        (1, 0xfffffffc, 255),
        (2, 3, 255),
        (3, 0xfffffffc, 255),
        (4, 3, 0),
        (0xffffffff, 3, 0),
    ];
    let mut words = Vec::new();
    let mut expected = Vec::new();
    for (count, result, initial_flag) in cases {
        for mismatch in [false, true] {
            words.push([count, result ^ u32::from(mismatch), 0, 0, 0, 0, 0, 0]);
            expected.push([if mismatch { 0 } else { 255 }, initial_flag, 0, 255]);
        }
    }
    pixel_grid(&ps, &words, &expected, vec![]);
}
