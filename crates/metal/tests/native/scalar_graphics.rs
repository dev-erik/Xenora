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
// Each tuple is (A, B, incoming SCC, exact result bits, outgoing SCC).
type Case = (u32, u32, u32, u32, bool);
const CASES: [[Case; 6]; 12] = [
    // ADD_U32: wrapping and carry, including nonzero results without carry.
    [
        (0xffffffff, 1, 0, 0, true),
        (0xffffffff, 0, 1, 0xffffffff, false),
        (0x7fffffff, 1, 0, 0x80000000, false),
        (0x80000000, 0x80000000, 0, 0, true),
        (0, 0, 1, 0, false),
        (0x7fc00000, 1, 0, 0x7fc00001, false),
    ],
    // SUB_U32: borrow, not result-nonzero or inverted carry.
    [
        (0, 1, 0, 0xffffffff, true),
        (0xffffffff, 1, 1, 0xfffffffe, false),
        (0x80000000, 1, 0, 0x7fffffff, false),
        (0x7fffffff, 0x80000000, 0, 0xffffffff, true),
        (5, 5, 1, 0, false),
        (0x7fc00000, 0x7f800000, 0, 0x00400000, false),
    ],
    // ADD_I32: both signed overflow directions; ordinary unsigned carry is not SCC.
    [
        (0x7fffffff, 1, 0, 0x80000000, true),
        (0x80000000, 0xffffffff, 0, 0x7fffffff, true),
        (0xffffffff, 1, 1, 0, false),
        (0x80000000, 0x80000000, 0, 0, true),
        (0, 0, 1, 0, false),
        (0x7fc00000, 1, 0, 0x7fc00001, false),
    ],
    // SUB_I32.
    [
        (0x80000000, 1, 0, 0x7fffffff, true),
        (0x7fffffff, 0xffffffff, 0, 0x80000000, true),
        (0, 1, 1, 0xffffffff, false),
        (0x80000000, 0x80000000, 1, 0, false),
        (0x7fffffff, 0x80000000, 0, 0xffffffff, true),
        (0xffffffff, 0xffffffff, 1, 0, false),
    ],
    // ADDC: input SCC must be consumed before output SCC replaces it.
    [
        (0xffffffff, 0, 1, 0, true),
        (0xffffffff, 0xffffffff, 1, 0xffffffff, true),
        (0, 0, 1, 1, false),
        (0xffffffff, 0, 0, 0xffffffff, false),
        (0, 0, 0, 0, false),
        (0x7fffffff, 0, 1, 0x80000000, false),
    ],
    // SUBB: B+borrow may exceed u32; 0 - MAX - 1 is zero with borrow.
    [
        (0, 0, 1, 0xffffffff, true),
        (0xffffffff, 0xffffffff, 1, 0xffffffff, true),
        (0, 0xffffffff, 1, 0, true),
        (5, 3, 1, 1, false),
        (5, 3, 0, 2, false),
        (0, 0, 0, 0, false),
    ],
    // Signed MIN, unsigned MIN, signed MAX, unsigned MAX; strict ties clear SCC.
    [
        (0xffffffff, 1, 0, 0xffffffff, true),
        (1, 0xffffffff, 1, 0xffffffff, false),
        (0x80000000, 0x7fffffff, 0, 0x80000000, true),
        (5, 5, 1, 5, false),
        (0x80000000, 0x80000000, 1, 0x80000000, false),
        (0x7fc00000, 0x7f800000, 0, 0x7f800000, false),
    ],
    [
        (0xffffffff, 1, 1, 1, false),
        (1, 0xffffffff, 0, 1, true),
        (0x80000000, 0x7fffffff, 1, 0x7fffffff, false),
        (5, 5, 1, 5, false),
        (0x80000000, 0x80000000, 1, 0x80000000, false),
        (0x7fc00000, 0x7f800000, 0, 0x7f800000, false),
    ],
    [
        (0xffffffff, 1, 1, 1, false),
        (1, 0xffffffff, 0, 1, true),
        (0x80000000, 0x7fffffff, 1, 0x7fffffff, false),
        (5, 5, 1, 5, false),
        (0x80000000, 0x80000000, 1, 0x80000000, false),
        (0x7fc00000, 0x7f800000, 0, 0x7fc00000, true),
    ],
    [
        (0xffffffff, 1, 0, 0xffffffff, true),
        (1, 0xffffffff, 1, 0xffffffff, false),
        (0x80000000, 0x7fffffff, 0, 0x80000000, true),
        (5, 5, 1, 5, false),
        (0x80000000, 0x80000000, 1, 0x80000000, false),
        (0x7fc00000, 0x7f800000, 0, 0x7fc00000, true),
    ],
    // CSELECT preserves SCC, even when selected result's nonzero flag differs.
    [
        (0, 9, 1, 0, true),
        (9, 7, 0, 7, false),
        (0xffffffff, 1, 1, 0xffffffff, true),
        (0xffffffff, 1, 0, 1, false),
        (0x7fc00000, 0x7f800000, 1, 0x7fc00000, true),
        (0x7fc00000, 0x7f800000, 0, 0x7f800000, false),
    ],
    // MOV and the later s105 move preserve both SCC and arbitrary integer bits.
    [
        (0, 9, 1, 0, true),
        (9, 7, 0, 9, false),
        (0xffffffff, 1, 1, 0xffffffff, true),
        (0x80000000, 1, 0, 0x80000000, false),
        (0x7fc00000, 0, 1, 0x7fc00000, true),
        (0x7f800000, 0, 0, 0x7f800000, false),
    ],
];
const PIXELS: [&[u8]; 12] = [
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-add-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-sub-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-add-i32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-sub-i32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-addc-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-subb-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-min-i32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-min-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-max-i32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-max-u32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-cselect-b32-ps.bin"),
    include_bytes!("../../../../fixtures/rdna2/x9-scalar-mov-b32-ps.bin"),
];

#[test]
fn translated_scalar_results_and_scc_literal_grid() {
    let pipelines: Vec<_> = PIXELS
        .iter()
        .map(|ps| compile_graphics_pipeline(Architecture::Gfx1030, VS, ps).unwrap())
        .collect();
    let mut draws = Vec::new();
    let mut states = Vec::new();
    let mut shaders = Vec::new();
    let mut uniforms = Vec::new();
    let mut expected = Vec::new();
    for row in 0..12 {
        for (column, pipeline) in pipelines.iter().enumerate() {
            let (a, b, carry, result, flag) = CASES[column][row / 2];
            let matches = row % 2 == 0;
            draws.push(TriangleDraw {
                vertices: &POS,
                colors: &COLORS,
                viewport: [12, 12],
            });
            states.push(TriangleState {
                scissor: Some([column, row, 1, 1]),
                ..Default::default()
            });
            shaders.push(TriangleShader {
                source: &pipeline.shader.msl,
                entries: ENTRIES,
            });
            assert_eq!(pipeline.required_uniform_mask, 15);
            uniforms.push(DrawUniforms {
                required_mask: 15,
                values: Some([
                    a,
                    b,
                    carry,
                    if matches { result } else { result ^ 1 },
                    0,
                    0,
                    0,
                    0,
                ]),
            });
            expected.extend_from_slice(&[
                if matches { 255 } else { 0 },
                if flag { 255 } else { 0 },
                0,
                255,
            ]);
        }
    }
    let gpu = NativeCompute::new().unwrap();
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [12, 12],
        clears: &[[7, 11, 13, 255]],
        clear_depth: None,
    };
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 144],
            None,
            &[None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(actual, vec![expected]);
}

#[test]
fn translated_scalar_local_nested_scc_mrt_joins() {
    let pipeline = compile_graphics_pipeline(
        Architecture::Gfx1030,
        VS,
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-nested-mrt-ps.bin"),
    )
    .unwrap();
    assert_eq!(pipeline.required_uniform_mask, 31);
    // (A,outer,inner_value,inner_condition,selected result,SCC after join).
    // Last two cases retain NaN-shaped integer bits through scalar joins and
    // CSELECT with each preserved SCC value; no float conversion is valid.
    let cases = [
        (5, 0, 0xffffffff, 0, 0, true),
        (5, 0, 7, 0, 5, false),
        (5, 0, 7, 1, 5, false),
        (9, 1, 7, 0, 9, true),
        (0x7fc00000, 1, 7, 0, 0x7fc00000, true),
        (0x7fc00000, 0, 7, 1, 0x7fc00000, false),
    ];
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &COLORS,
        viewport: [12, 1],
    }; 12];
    let states = std::array::from_fn::<_, 12, _>(|x| TriangleState {
        scissor: Some([x, 0, 1, 1]),
        ..Default::default()
    });
    let shaders = [TriangleShader {
        source: &pipeline.shader.msl,
        entries: ENTRIES,
    }; 12];
    let uniforms: Vec<_> = cases
        .into_iter()
        .flat_map(|(a, b, c, d, result, _)| {
            [result, result ^ 1].map(|expected| DrawUniforms {
                required_mask: 31,
                values: Some([a, b, c, d, expected, 0, 0, 0]),
            })
        })
        .collect();
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [12, 1],
        clears: &[[7, 11, 13, 255], [17, 19, 23, 255]],
        clear_depth: None,
    };
    let gpu = NativeCompute::new().unwrap();
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 12],
            None,
            &[None, None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            vec![
                255, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255, 0,
                0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255, 255,
                0, 0, 255, 0, 0, 0, 255
            ],
            vec![
                0, 255, 255, 255, 0, 0, 255, 255, 0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0,
                0, 0, 255, 0, 255, 255, 255, 0, 0, 255, 255, 0, 255, 255, 255, 0, 0, 255, 255, 0,
                255, 0, 255, 0, 0, 0, 255
            ],
        ]
    );
}

#[test]
fn translated_scalar_add_vertex_and_pixel_share_uniform_words() {
    let pipeline = compile_graphics_pipeline(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-add-vs.bin"),
        PIXELS[0],
    )
    .unwrap();
    let square = [
        [-1., -1., 0.5, 1.],
        [1., -1., 0.5, 1.],
        [-1., 1., 0.5, 1.],
        [1., -1., 0.5, 1.],
        [1., 1., 0.5, 1.],
        [-1., 1., 0.5, 1.],
    ];
    let colors = [[0.; 4]; 6];
    let draws = [TriangleDraw {
        vertices: &square,
        colors: &colors,
        viewport: [8, 4],
    }; 4];
    let states = std::array::from_fn::<_, 4, _>(|y| TriangleState {
        scissor: Some([0, y, 8, 1]),
        ..Default::default()
    });
    let shaders = [TriangleShader {
        source: &pipeline.shader.msl,
        entries: ENTRIES,
    }; 4];
    let uniforms = [
        [0xffffffff, 1, 0, 0, 0, 0, 0, 0],
        [0xffffffff, 1, 0, 1, 0, 0, 0, 0],
        [2, 3, 0, 5, 0, 0, 0, 0],
        [2, 3, 0, 6, 0, 0, 0, 0],
    ]
    .map(|values| DrawUniforms {
        required_mask: pipeline.required_uniform_mask,
        values: Some(values),
    });
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [8, 4],
        clears: &[[7, 11, 13, 255]],
        clear_depth: None,
    };
    let gpu = NativeCompute::new().unwrap();
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 4],
            None,
            &[None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    let expected = [
        [255, 255, 0, 255],
        [0, 255, 0, 255],
        [255, 0, 0, 255],
        [0, 0, 0, 255],
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(row, color)| {
        (0..8).flat_map(move |x| {
            if (x < 4) == (row % 2 == 0) {
                color
            } else {
                [7, 11, 13, 255]
            }
        })
    })
    .collect::<Vec<_>>();
    assert_eq!(actual, vec![expected]);
}

#[test]
fn translated_scalar_mov_preserves_identical_nan_bits_with_both_scc_values() {
    let pipeline = compile_graphics_pipeline(Architecture::Gfx1030, VS, PIXELS[11]).unwrap();
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &COLORS,
        viewport: [4, 1],
    }; 4];
    let states = std::array::from_fn::<_, 4, _>(|x| TriangleState {
        scissor: Some([x, 0, 1, 1]),
        ..Default::default()
    });
    let shaders = [TriangleShader {
        source: &pipeline.shader.msl,
        entries: ENTRIES,
    }; 4];
    let uniforms = [
        (0, 0x7fc00000),
        (0, 0x7fc00001),
        (1, 0x7fc00000),
        (1, 0x7fc00001),
    ]
    .map(|(flag, expected)| DrawUniforms {
        required_mask: pipeline.required_uniform_mask,
        values: Some([0x7fc00000, 0, flag, expected, 0, 0, 0, 0]),
    });
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 1],
        clears: &[[7, 11, 13, 255]],
        clear_depth: None,
    };
    let gpu = NativeCompute::new().unwrap();
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 4],
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
            255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 255
        ]]
    );
}

#[test]
fn translated_scalar_actual_carry_borrow_chains_and_all_operand_aliases() {
    let fixtures: [&[u8]; 4] = [
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-add-chain-ps.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-add-chain-alias-ps.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-sub-chain-ps.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-sub-chain-alias-ps.bin"),
    ];
    // Literal low-A, low-B, high-A, high-B, low result, high result, final flag.
    let cases = [
        [
            (0xffffffff, 1, 0xffffffff, 0, 0, 0, true),
            (0xffffffff, 1, 0x7fffffff, 0, 0, 0x80000000, false),
            (1, 2, 0xffffffff, 0, 3, 0xffffffff, false),
            (0xffffffff, 0xffffffff, 0, 0, 0xfffffffe, 1, false),
        ],
        [
            (0xffffffff, 1, 0xffffffff, 0, 0, 0xffffffff, true),
            (0xffffffff, 1, 0, 0, 0, 1, false),
            (1, 2, 0x80000000, 0, 3, 0, true),
            (0, 0, 0x7fffffff, 0, 0, 0xfffffffe, false),
        ],
        [
            (0, 1, 0, 0xffffffff, 0xffffffff, 0, true),
            (0, 1, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, true),
            (5, 3, 5, 3, 2, 2, false),
            (0, 0, 0, 1, 0, 0xffffffff, true),
        ],
        [
            (0, 1, 0xffffffff, 0, 0xffffffff, 0xffffffff, true),
            (0, 1, 0, 0, 0xffffffff, 0xffffffff, true),
            (5, 3, 0xffffffff, 0, 2, 0, false),
            (0, 0, 0x80000000, 0, 0, 0, false),
        ],
    ];
    let pipelines: Vec<_> = fixtures
        .iter()
        .map(|ps| compile_graphics_pipeline(Architecture::Gfx1030, VS, ps).unwrap())
        .collect();
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &COLORS,
        viewport: [4, 12],
    }; 48];
    let mut states = Vec::new();
    let mut shaders = Vec::new();
    let mut uniforms = Vec::new();
    let mut expected = Vec::new();
    for row in 0..12 {
        for (column, pipeline) in pipelines.iter().enumerate() {
            let (a, b, c, d, low, high, flag) = cases[column][row / 3];
            states.push(TriangleState {
                scissor: Some([column, row, 1, 1]),
                ..Default::default()
            });
            shaders.push(TriangleShader {
                source: &pipeline.shader.msl,
                entries: ENTRIES,
            });
            uniforms.push(DrawUniforms {
                required_mask: pipeline.required_uniform_mask,
                values: Some([
                    a,
                    b,
                    c,
                    d,
                    if row % 3 == 1 { low ^ 1 } else { low },
                    if row % 3 == 2 { high ^ 1 } else { high },
                    0,
                    0,
                ]),
            });
            expected.extend_from_slice(&[
                if row % 3 == 1 { 0 } else { 255 },
                if row % 3 == 2 { 0 } else { 255 },
                if flag { 255 } else { 0 },
                255,
            ]);
        }
    }
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 12],
        clears: &[[7, 11, 13, 255]],
        clear_depth: None,
    };
    let gpu = NativeCompute::new().unwrap();
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 48],
            None,
            &[None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(actual, vec![expected]);
}

#[test]
fn translated_scalar_direct_phi_observes_false_scc_and_high_bits() {
    let pipeline = compile_graphics_pipeline(
        Architecture::Gfx1030,
        VS,
        include_bytes!("../../../../fixtures/rdna2/x9-scalar-direct-phi-mrt-ps.bin"),
    )
    .unwrap();
    // No CSELECT can hide the merged 8/7 or high-bit local on a false-SCC path.
    let cases = [
        (5, 0, 7, 0, 8),
        (5, 0, 7, 1, 7),
        (0x80000001, 1, 7, 0, 0x80000001),
        (5, 0, 0x7fc00000, 0, 0x7fc00001),
        (5, 0, 0x80000001, 1, 0x80000001),
    ];
    let draws = [TriangleDraw {
        vertices: &POS,
        colors: &COLORS,
        viewport: [10, 1],
    }; 10];
    let states = std::array::from_fn::<_, 10, _>(|x| TriangleState {
        scissor: Some([x, 0, 1, 1]),
        ..Default::default()
    });
    let shaders = [TriangleShader {
        source: &pipeline.shader.msl,
        entries: ENTRIES,
    }; 10];
    let uniforms: Vec<_> = cases
        .into_iter()
        .flat_map(|(a, b, c, d, result)| {
            [result, result ^ 1].map(|expected| DrawUniforms {
                required_mask: pipeline.required_uniform_mask,
                values: Some([a, b, c, d, expected, 0, 0, 0]),
            })
        })
        .collect();
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [10, 1],
        clears: &[[7, 11, 13, 255], [17, 19, 23, 255]],
        clear_depth: None,
    };
    let gpu = NativeCompute::new().unwrap();
    let actual = gpu
        .submit_view_binding_triangle_targets_with_uniforms_rgba8(
            &shaders,
            &pass,
            &vec![vec![]; 10],
            None,
            &[None, None],
            &uniforms,
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(
        actual,
        vec![
            vec![
                255, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 0,
                255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255
            ],
            vec![
                0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 255, 255, 0, 0,
                255, 255, 0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255
            ],
        ]
    );
}

#[test]
fn translated_scalar_packing_preserves_runtime_scc_and_literal_halves() {
    let gpu = NativeCompute::new().unwrap();
    let fixtures: [(&[u8], u32); 3] = [
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-pack-ll-ps.bin"),
            0xef01_abcd,
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-pack-lh-ps.bin"),
            0x5678_abcd,
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-pack-hh-ps.bin"),
            0x5678_1234,
        ),
    ];
    for (original, result) in fixtures {
        for alias_src1 in [false, true] {
            for constants in [false, true] {
                let mut ps = original.to_vec();
                if alias_src1 {
                    // PACK s33,s32,s33 followed by MOV s105,s33: capture both inputs first.
                    let word = u32::from_le_bytes(ps[12..16].try_into().unwrap());
                    ps[12..16].copy_from_slice(&((word & !0x007f0000) | (33 << 16)).to_le_bytes());
                    ps[16] = 33;
                }
                if constants {
                    let mut prefix: Vec<u8> = [0xbea003ffu32, 0x1234abcd, 0xbea103ff, 0x5678ef01]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect();
                    prefix.extend_from_slice(&ps[8..]);
                    ps = prefix;
                }
                let pipeline = compile_graphics_pipeline(Architecture::Gfx1030, VS, &ps).unwrap();
                for flag in [false, true] {
                    for matches in [true, false] {
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
                                0x1234_abcd,
                                0x5678_ef01,
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
                                if flag { 255 } else { 0 },
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

#[test]
fn translated_scalar_immediates_preserve_flags_and_signed_results() {
    let gpu = NativeCompute::new().unwrap();
    let fixtures: [(&[u8], &[(u32, u32, u32, Option<bool>)]); 4] = [
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-movk-ps.bin"),
            &[(0, 0, 0xffff8000, None)],
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-addk-ps.bin"),
            &[
                (0x80000000, 0, 0x7fffffff, Some(true)),
                (0, 0, 0xffffffff, Some(false)),
                (1, 0, 0, Some(false)),
            ],
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-mulk-ps.bin"),
            &[
                (0x80000000, 0, 0x80000000, None),
                (3, 0, 0xfffffffd, None),
                (0xffffffff, 0, 1, None),
            ],
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-scalar-mul-ps.bin"),
            &[
                (0x80000000, 3, 0x80000000, None),
                (0x7fffffff, 3, 0x7ffffffd, None),
                (0x12345678, 0x10000, 0x56780000, None),
            ],
        ),
    ];
    for (original, cases) in fixtures {
        for &(a, b, result, changed_flag) in cases {
            for constants in [false, true] {
                let mut ps = original.to_vec();
                if constants {
                    let mut prefix: Vec<u8> = [0xbea003ffu32, a, 0xbea103ff, b]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect();
                    prefix.extend_from_slice(&ps[8..]);
                    ps = prefix;
                }
                let pipeline = compile_graphics_pipeline(Architecture::Gfx1030, VS, &ps).unwrap();
                for flag in [false, true] {
                    let outgoing = changed_flag.unwrap_or(flag);
                    for matches in [true, false] {
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

#[test]
fn translated_immediate_comparisons_preserve_source_and_replace_scc() {
    let gpu = NativeCompute::new().unwrap();
    let original = include_bytes!("../../../../fixtures/rdna2/x9-scalar-cmpk-ps.bin");
    // Independent literal outcome masks EQ/LG/GT/GE/LT/LE, signed then unsigned.
    let rows = [
        (0u32, 0u16, 0x29u8, 0x29u8),
        (0xffffffff, 0xffff, 0x29, 0x0e),
        (0xffff8000, 0x8000, 0x29, 0x0e),
        (0x8000, 0x8000, 0x0e, 0x29),
        (0x80000000, 0, 0x32, 0x0e),
        (0, 0xffff, 0x0e, 0x32),
    ];
    for (a, imm, signed, unsigned) in rows {
        for op in 3..=14u32 {
            let mask = if op < 9 { signed } else { unsigned };
            let outgoing = mask & (1 << ((op - 3) % 6)) != 0;
            for constants in [false, true] {
                let mut ps = original.to_vec();
                ps[12..16].copy_from_slice(
                    &(0xb0000000 | op << 23 | 32 << 16 | u32::from(imm)).to_le_bytes(),
                );
                if constants {
                    let mut prefix: Vec<u8> = [0xbea003ffu32, a]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect();
                    prefix.extend_from_slice(&ps[4..]);
                    ps = prefix;
                }
                let pipeline = compile_graphics_pipeline(Architecture::Gfx1030, VS, &ps).unwrap();
                for flag in [false, true] {
                    for matches in [true, false] {
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
                                0,
                                u32::from(flag),
                                a ^ u32::from(!matches),
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
