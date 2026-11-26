// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_shader::{Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics};

use super::context;

const ENTRIES: [&str; 2] = [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT];
const POSITIONS: [[f32; 4]; 3] = [
    [-1.0, -1.0, 0.5, 1.0],
    [3.0, -1.0, 0.5, 1.0],
    [-1.0, 3.0, 0.5, 1.0],
];
const CLEAR: [u8; 4] = [19, 43, 93, 251];

#[test]
fn translated_constant_pixel_shader() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .expect("compile constant graphics fixture");
    let gpu = context();
    // This triangle falls entirely past the right clipping plane.
    let offscreen = POSITIONS.map(|[_, y, z, w]| [2.0, y, z, w]);
    for (positions, pixel) in [(&POSITIONS, [255, 0, 0, 255]), (&offscreen, CLEAR)] {
        let actual = gpu
            .render_rgba8(
                &compiled.msl,
                ENTRIES,
                positions,
                &[[0.0; 4]; 3],
                [4, 3],
                CLEAR,
            )
            .expect("draw translated constant pixel shader");
        assert_eq!(actual, pixel.repeat(12));
    }
}

#[test]
fn translated_varying_pixel_shader() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .expect("compile varying graphics fixture");
    // Double the second vertex's homogeneous coordinates to keep the
    // projected triangle fixed but vary its perspective interpolation weight.
    let mut positions = POSITIONS;
    positions[1] = positions[1].map(|component| 2.0 * component);
    let colors = [
        [0.0, 0.0, 0.0, 1.0],
        [128.0 / 255.0, 0.0, 64.0 / 255.0, 1.0],
        [0.0, 128.0 / 255.0, 64.0 / 255.0, 1.0],
    ];
    let gpu = context();
    let actual = gpu
        .render_rgba8(&compiled.msl, ENTRIES, &positions, &colors, [4, 4], CLEAR)
        .expect("draw translated varying pixel shader");

    // The triangle legs span eight pixels from the bottom-left viewport
    // corner. At pixel centers the second and third vertex's affine
    // weights are (2x+1)/16 and (7-2y)/16. Dividing each by its clip W
    // and normalizing gives the perspective weights below. The odd
    // denominator makes nearest-byte conversion unambiguous.
    let expected: Vec<u8> = (0..4)
        .flat_map(|y| {
            (0..4).flat_map(move |x| {
                let u = 2 * x + 1;
                let v = 2 * (7 - 2 * y);
                let sum = 32 - u;
                let byte = |value| ((value + sum / 2) / sum) as u8;
                [byte(128 * u), byte(128 * v), byte(64 * (u + v)), 255]
            })
        })
        .collect();
    assert_eq!(actual, expected);

    let swapped_colors = colors.map(|[r, g, b, a]| [g, r, b, a]);
    let swapped = gpu
        .render_rgba8(
            &compiled.msl,
            ENTRIES,
            &positions,
            &swapped_colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw the same translated shader with red and green vertex data swapped");
    let mut expected_swapped = expected.clone();
    for pixel in expected_swapped.as_chunks_mut::<4>().0 {
        pixel.swap(0, 1);
    }
    assert_ne!(expected_swapped, expected);
    assert_eq!(swapped, expected_swapped);
    println!("checked 16 perspective-interpolated RGBA8 pixels per color input");
}

#[test]
fn translated_render_rejects_out_of_policy_inputs() {
    let gpu = context();
    for extent in [[0, 1], [1024, 1025], [usize::MAX, 2]] {
        let error = gpu
            .render_rgba8("unused", ENTRIES, &POSITIONS, &[[0.0; 4]; 3], extent, CLEAR)
            .expect_err("invalid dimensions must fail before MSL compilation");
        assert_eq!(error.operation, "render bounds");
    }
    let error = gpu
        .render_rgba8(
            "unused",
            ENTRIES,
            &POSITIONS,
            &[[f32::NAN; 4]; 3],
            [1, 1],
            CLEAR,
        )
        .expect_err("non-finite colors must fail before MSL compilation");
    assert_eq!(error.operation, "render input");
}

#[test]
fn transformed_vertex_shader_draws_three_interior_pixels() {
    let constant_ps = include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin");
    let transformed = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-transform-vs.bin"),
        constant_ps,
    )
    .expect("compile position transform");
    let passthrough = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        constant_ps,
    )
    .expect("compile pass-through control");
    let positions = [
        [-1.5, -1.0, 0.5, 1.0],
        [0.5, -1.0, 0.5, 1.0],
        [-1.5, 2.0, 0.5, 1.0],
    ];

    // Scaling x and y by half and biasing x by one quarter places the
    // vertices at (1, 3), (3, 3), and (1, 0) in this 4x4 viewport.
    // A sample center is inside exactly when 1 < x < 3 and
    // 3*(x-1)/2 < y < 3. No edge crosses a sample center.
    let mut expected = [CLEAR; 16];
    for index in [5, 9, 10] {
        expected[index] = [255, 0, 0, 255];
    }
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let transformed_pixels = gpu
        .render_rgba8(
            &transformed.msl,
            ENTRIES,
            &positions,
            &colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw translated transform");
    assert_eq!(transformed_pixels, expected.as_flattened());

    let passthrough_pixels = gpu
        .render_rgba8(
            &passthrough.msl,
            ENTRIES,
            &positions,
            &colors,
            [4, 4],
            CLEAR,
        )
        .expect("draw unchanged positions through pass-through control");
    // Without the transform, the diagonal extends from (-1, -2) to (3, 4)
    // and the viewport clips both the left and upper edges of the triangle.
    let mut expected_passthrough = [CLEAR; 16];
    for index in [0, 4, 8, 9, 12, 13, 14] {
        expected_passthrough[index] = [255, 0, 0, 255];
    }
    assert_eq!(passthrough_pixels, expected_passthrough.as_flattened());
    println!("checked 3 covered pixels and 13 clear pixels from translated vertex arithmetic");
}

#[test]
fn translated_image_sample_lz_pixel_shader_matches_cpu_reference() {
    let gpu = context();

    // 1. Prepare 2x2 RGBA8 texture with four distinct colors:
    // (0, 0): Red   [255,   0,   0, 255]
    // (1, 0): Green [  0, 255,   0, 255]
    // (0, 1): Blue  [  0,   0, 255, 255]
    // (1, 1): White [255, 255, 255, 255]
    let texels_level0: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let tex_desc = xenora_metal::texture_pipeline::Rgba8TextureDesc::new(2, 2, 1)
        .expect("valid 2x2 texture desc");
    let mip0 = xenora_metal::texture_pipeline::Rgba8MipLevel::new_packed(0, 2, 2, &texels_level0);
    let texture =
        xenora_metal::texture_pipeline::MetalRgba8Texture::upload(gpu.device(), tex_desc, &[mip0])
            .expect("upload RGBA8 2x2 texture to Metal");

    // 2. Create native Metal sampler (nearest, clamp-to-edge)
    let sampler_desc = xenora_texture::SamplerDesc::nearest_clamp();
    let sampler = xenora_metal::sampler::MetalSampler::new(gpu.device(), sampler_desc)
        .expect("create native Metal sampler");

    // 3. VS: x9-varying-vs.bin exports position from buffer(0) and UV from buffer(1) via param0
    let vs_bytes = include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin");

    // 4. PS: genuine gfx1030 machine-code instructions:
    // v_interp_p1_f32_e32 v2, v0, attr0.x
    // v_interp_p2_f32_e32 v2, v1, attr0.x
    // v_interp_p1_f32_e32 v3, v0, attr0.y
    // v_interp_p2_f32_e32 v3, v1, attr0.y
    // image_sample_lz v[4:7], v[2:3], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D
    // exp mrt0, v4, v5, v6, v7 done vm
    // s_endpgm
    let mut ps_bytes = Vec::new();
    ps_bytes.extend_from_slice(&[0x00, 0x00, 0x08, 0xc8]); // v_interp_p1_f32_e32 v2, v0, attr0.x
    ps_bytes.extend_from_slice(&[0x01, 0x00, 0x09, 0xc8]); // v_interp_p2_f32_e32 v2, v1, attr0.x
    ps_bytes.extend_from_slice(&[0x00, 0x01, 0x0c, 0xc8]); // v_interp_p1_f32_e32 v3, v0, attr0.y
    ps_bytes.extend_from_slice(&[0x01, 0x01, 0x0d, 0xc8]); // v_interp_p2_f32_e32 v3, v1, attr0.y
    ps_bytes.extend_from_slice(&[0x08, 0x0f, 0x9c, 0xf0, 0x02, 0x04, 0x40, 0x00]); // image_sample_lz
    ps_bytes.extend_from_slice(&[0x0f, 0x18, 0x00, 0xf8, 0x04, 0x05, 0x06, 0x07]); // exp mrt0
    ps_bytes.extend_from_slice(&[0x00, 0x00, 0x81, 0xbf]); // s_endpgm

    // 5. Compile through Xenora shader compiler pipeline:
    // gfx1030 binary -> xenora-decoder -> xenora-shader IR -> MSL
    let compiled = compile_graphics(Architecture::Gfx1030, vs_bytes, &ps_bytes)
        .expect("compile translated image_sample_lz graphics shader");

    assert!(compiled.ir.contains("sample_lz.2d.f32 tex0, samp0"));
    assert!(
        compiled
            .msl
            .contains("texture2d<float, access::sample> tex0 [[texture(0)]]")
    );
    assert!(compiled.msl.contains("sampler samp0 [[sampler(0)]]"));

    // 6. Set up a fullscreen triangle covering a 2x2 render viewport:
    // In Metal framebuffer layout, row 0 is at the top (NDC y = +1.0) and row 1 is at the bottom (NDC y = -1.0).
    // Mapping UV coordinates so top row samples V in [0.0..0.5] and bottom row samples V in [0.5..1.0]:
    // Vertex 0 (NDC [-1, -1], bottom-left)  -> UV (0.0, 1.0)
    // Vertex 1 (NDC [ 3, -1], bottom-right) -> UV (2.0, 1.0)
    // Vertex 2 (NDC [-1,  3], top-left)     -> UV (0.0, -1.0)
    // In the 2x2 render target:
    // Pixel (x=0, y=0, top-left row 0): center UV = (0.25, 0.25) -> Texel (0, 0) Red   [255,   0,   0, 255]
    // Pixel (x=1, y=0, top-right row 0): center UV = (0.75, 0.25) -> Texel (1, 0) Green [  0, 255,   0, 255]
    // Pixel (x=0, y=1, bottom-left row 1): center UV = (0.25, 0.75) -> Texel (0, 1) Blue  [  0,   0, 255, 255]
    // Pixel (x=1, y=1, bottom-right row 1): center UV = (0.75, 0.75) -> Texel (1, 1) White [255, 255, 255, 255]
    let uv_coords = [
        [0.0_f32, 1.0, 0.0, 1.0],
        [2.0, 1.0, 0.0, 1.0],
        [0.0, -1.0, 0.0, 1.0],
    ];

    let actual_pixels = gpu
        .render_rgba8_sampled(
            &compiled.msl,
            ENTRIES,
            &POSITIONS,
            &uv_coords,
            [2, 2],
            CLEAR,
            Some((&texture, &sampler)),
        )
        .expect("draw translated texture sampling pipeline on Apple GPU");

    // 7. Verify with independent CPU texture sampling reference
    let ref_desc = xenora_ref::texture_sampling::Rgba8TextureDesc::new_packed(2, 2, 1)
        .expect("valid CPU ref texture desc");
    let test_points = [
        [0.25_f32, 0.25], // pixel (0, 0)
        [0.75, 0.25],     // pixel (1, 0)
        [0.25, 0.75],     // pixel (0, 1)
        [0.75, 0.75],     // pixel (1, 1)
    ];

    let mut expected_pixels = Vec::new();
    for [u, v] in test_points {
        let cpu_sample =
            xenora_ref::texture_sampling::sample_rgba8_2d(&texels_level0, &ref_desc, 0, u, v)
                .expect("CPU reference sampling");
        expected_pixels.extend_from_slice(&cpu_sample);
    }

    assert_eq!(
        actual_pixels, expected_pixels,
        "GPU translated sampling output must bitwise match independent CPU sampling oracle"
    );

    println!(
        "device={} verified translated RDNA2 image_sample_lz against independent CPU reference on Apple Silicon GPU",
        gpu.device_name()
    );
}

#[test]
fn translated_triangle_list_covers_both_halves_and_reaches_limit() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let gpu = context();
    let square = [
        [-1.0, -1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [1.0, 1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
    ];
    for count in [6, 768] {
        // Only the LAST two triangles cover the viewport; dropping the tail
        // or submitting a fixed vertex count leaves clear pixels.
        let mut vertices = vec![[2.0, 2.0, 0.5, 1.0]; count];
        vertices[count - 6..].copy_from_slice(&square);
        let pixels = gpu
            .render_triangles_rgba8(
                &compiled.msl,
                ENTRIES,
                &vertices,
                &vec![[0.0; 4]; count],
                [4, 3],
                CLEAR,
            )
            .unwrap();
        assert_eq!(pixels, [255, 0, 0, 255].repeat(12), "vertex count {count}");
    }
}

#[test]
fn triangle_lists_reject_malformed_inputs_before_shader_compilation() {
    let gpu = context();
    for (positions, colors) in [
        (0, 0),
        (1, 1),
        (4, 4),
        (769, 769),
        (65538, 65538),
        (6, 3),
        (3, 6),
    ] {
        let error = gpu
            .render_triangles_rgba8(
                "invalid shader",
                ENTRIES,
                &vec![[0.0; 4]; positions],
                &vec![[0.0; 4]; colors],
                [4, 3],
                CLEAR,
            )
            .unwrap_err();
        assert!(error.to_string().contains("render input"), "{error}");
    }
}

#[test]
fn translated_triangle_list_uses_colors_for_each_triangle() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let vertices = [
        [-1.0, -1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [1.0, 1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
    ];
    let colors = [
        [1.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
    ];
    let pixels = context()
        .render_triangles_rgba8(&compiled.msl, ENTRIES, &vertices, &colors, [4, 4], CLEAR)
        .unwrap();
    // The shared edge maps to y=x in top-down framebuffer coordinates.
    // Edge pixels belong to either triangle under rasterization tie-breaking.
    for (i, &pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % 4, i / 4);
        if y > x {
            assert_eq!(pixel, [255, 0, 0, 255]);
        } else if y < x {
            assert_eq!(pixel, [0, 255, 0, 255]);
        } else {
            assert!(pixel == [255, 0, 0, 255] || pixel == [0, 255, 0, 255]);
        }
    }
}

#[test]
fn translated_pass_preserves_draws_and_changes_viewport() {
    use xenora_metal::TriangleDraw;
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let red = [[1.0, 0.0, 0.0, 1.0]; 3];
    let green = [[0.0, 1.0, 0.0, 1.0]; 3];
    let gpu = context();
    for reverse in [false, true] {
        let mut draws = [
            TriangleDraw {
                vertices: &POSITIONS,
                colors: &red,
                viewport: [2, 4],
            },
            TriangleDraw {
                vertices: &POSITIONS,
                colors: &green,
                viewport: [4, 2],
            },
        ];
        if reverse {
            draws.reverse();
        }
        let pixels = gpu
            .render_triangle_pass_rgba8(&compiled.msl, ENTRIES, &draws, [4, 4], CLEAR)
            .unwrap();
        for (i, &pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (i % 4, i / 4);
            let expected = if x < 2 && (reverse || y >= 2) {
                [255, 0, 0, 255]
            } else if y < 2 {
                [0, 255, 0, 255]
            } else {
                CLEAR
            };
            assert_eq!(pixel, expected, "pixel {x},{y}, reverse={reverse}");
        }
    }
}

#[test]
fn triangle_pass_rejects_invalid_later_draw_before_compilation() {
    use xenora_metal::TriangleDraw;
    let gpu = context();
    let colors = [[1.0; 4]; 3];
    let valid = TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 4],
    };
    let mut nonfinite = POSITIONS;
    nonfinite[2][0] = f32::NAN;
    for invalid in [
        TriangleDraw {
            vertices: &[],
            ..valid
        },
        TriangleDraw {
            vertices: &POSITIONS[..2],
            colors: &colors[..2],
            ..valid
        },
        TriangleDraw {
            colors: &colors[..2],
            ..valid
        },
        TriangleDraw {
            vertices: &nonfinite,
            ..valid
        },
        TriangleDraw {
            viewport: [0, 4],
            ..valid
        },
        TriangleDraw {
            viewport: [4, 5],
            ..valid
        },
    ] {
        let err = gpu
            .render_triangle_pass_rgba8("invalid shader", ENTRIES, &[valid, invalid], [4, 4], CLEAR)
            .unwrap_err();
        assert!(
            err.to_string().contains("render input") || err.to_string().contains("render bounds"),
            "{err}"
        );
    }
    let many_vertices = vec![POSITIONS[0]; 65535];
    let many_colors = vec![[1.0; 4]; 65535];
    for draws in [
        vec![],
        vec![valid; 257],
        vec![
            TriangleDraw {
                vertices: &many_vertices,
                colors: &many_colors,
                ..valid
            },
            valid,
        ],
    ] {
        let err = gpu
            .render_triangle_pass_rgba8("invalid shader", ENTRIES, &draws, [4, 4], CLEAR)
            .unwrap_err();
        assert!(err.to_string().contains("render input"), "{err}");
    }
}

#[test]
fn translated_pass_reaches_last_draw_with_separate_buffer_views() {
    use xenora_metal::TriangleDraw;
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let clipped = [[2.0, 2.0, 0.5, 1.0]; 3];
    let red = [[1.0, 0.0, 0.0, 1.0]; 3];
    let green = [[0.0, 1.0, 0.0, 1.0]; 3];
    let mut draws = vec![
        TriangleDraw {
            vertices: &clipped,
            colors: &red,
            viewport: [4, 4]
        };
        256
    ];
    draws[255] = TriangleDraw {
        vertices: &POSITIONS,
        colors: &green,
        viewport: [4, 4],
    };
    let pixels = context()
        .render_triangle_pass_rgba8(&compiled.msl, ENTRIES, &draws, [4, 4], CLEAR)
        .unwrap();
    assert_eq!(pixels, [0, 255, 0, 255].repeat(16));
}

#[test]
fn translated_arithmetic_survives_ordered_pass_buffer_changes() {
    use xenora_metal::TriangleDraw;
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-transform-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let clipped = [[8.0, 8.0, 0.5, 1.0]; 3];
    let positions = [
        [-1.5, -1.0, 0.5, 1.0],
        [0.5, -1.0, 0.5, 1.0],
        [-1.5, 2.0, 0.5, 1.0],
    ];
    let colors = [[0.0; 4]; 3];
    let draws = [
        TriangleDraw {
            vertices: &clipped,
            colors: &colors,
            viewport: [2, 2],
        },
        TriangleDraw {
            vertices: &positions,
            colors: &colors,
            viewport: [4, 4],
        },
    ];
    // x'=x/2+1/4, y'=y/2 gives screen vertices (1,3), (3,3), (1,0).
    // Exactly sample centers 5, 9 and 10 lie inside; none lies on an edge.
    let mut expected = [CLEAR; 16];
    for i in [5, 9, 10] {
        expected[i] = [255, 0, 0, 255];
    }
    let actual = context()
        .render_triangle_pass_rgba8(&compiled.msl, ENTRIES, &draws, [4, 4], CLEAR)
        .unwrap();
    assert_eq!(actual, expected.as_flattened());
}

#[test]
fn translated_pass_combines_depth_writes_and_alpha_blending() {
    use xenora_metal::{
        BlendFactor, BlendOperation, BlendState, DepthCompare, DepthState, TriangleDraw,
        TrianglePass, TriangleState,
    };
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let far = POSITIONS.map(|[x, y, _, w]| [x, y, 0.75, w]);
    let near = POSITIONS.map(|[x, y, _, w]| [x, y, 0.25, w]);
    let middle = POSITIONS.map(|[x, y, _, w]| [x, y, 0.5, w]);
    let blue = [[0.0, 0.0, 1.0, 1.0]; 3];
    let red = [[1.0, 0.0, 0.0, 0.5]; 3];
    let green = [[0.0, 1.0, 0.0, 1.0]; 3];
    let draws = [
        TriangleDraw {
            vertices: &far,
            colors: &blue,
            viewport: [4, 4],
        },
        TriangleDraw {
            vertices: &near,
            colors: &red,
            viewport: [4, 4],
        },
        TriangleDraw {
            vertices: &middle,
            colors: &green,
            viewport: [4, 4],
        },
    ];
    let depth = DepthState {
        compare: DepthCompare::Less,
        write: true,
    };
    let alpha = BlendState {
        source_rgb: BlendFactor::SourceAlpha,
        destination_rgb: BlendFactor::OneMinusSourceAlpha,
        rgb_operation: BlendOperation::Add,
        source_alpha: BlendFactor::One,
        destination_alpha: BlendFactor::Zero,
        alpha_operation: BlendOperation::Add,
    };
    let gpu = context();
    for write in [false, true] {
        let states = [
            TriangleState {
                depth,
                ..TriangleState::default()
            },
            TriangleState {
                depth: DepthState { write, ..depth },
                blend: Some(alpha),
                ..TriangleState::default()
            },
            TriangleState {
                depth,
                ..TriangleState::default()
            },
        ];
        let out = gpu
            .render_triangle_pass_with_state_rgba8(
                &compiled.msl,
                ENTRIES,
                &TrianglePass {
                    draws: &draws,
                    states: &states,
                    extent: [4, 4],
                    clear: CLEAR,
                    clear_depth: Some(1.0),
                },
            )
            .unwrap();
        for pixel in out.as_chunks::<4>().0 {
            if write {
                // Near half-alpha red over opaque blue; middle depth then fails.
                for (&got, want) in pixel.iter().zip([128u8, 0, 128, 128]) {
                    assert!(got.abs_diff(want) <= 1);
                }
            } else {
                assert_eq!(*pixel, [0, 255, 0, 255]);
            }
        }
    }
}

#[test]
fn stateful_pass_rejects_invalid_attachment_state_before_compilation() {
    use xenora_metal::{DepthCompare, DepthState, TriangleDraw, TrianglePass, TriangleState};
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 4],
    }];
    let disabled = [TriangleState::default()];
    let enabled = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write: true,
        },
        ..TriangleState::default()
    }];
    let gpu = context();
    for (states, clear_depth) in [
        (&[][..], None),
        (&enabled[..], None),
        (&disabled[..], Some(f32::NAN)),
        (&disabled[..], Some(-0.1)),
        (&disabled[..], Some(1.1)),
    ] {
        let err = gpu
            .render_triangle_pass_with_state_rgba8(
                "invalid shader",
                ENTRIES,
                &TrianglePass {
                    draws: &draws,
                    states,
                    extent: [4, 4],
                    clear: CLEAR,
                    clear_depth,
                },
            )
            .unwrap_err();
        assert_eq!(err.operation, "render input");
    }
}

#[test]
fn translated_pass_offsets_viewports_and_resets_scissors() {
    use xenora_metal::{TriangleDraw, TrianglePass, TriangleState};
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let red = [[1.0, 0.0, 0.0, 1.0]; 3];
    let green = [[0.0, 1.0, 0.0, 1.0]; 3];
    let blue = [[0.0, 0.0, 1.0, 1.0]; 3];
    let mut draws = [
        TriangleDraw {
            vertices: &POSITIONS,
            colors: &red,
            viewport: [2, 4],
        },
        TriangleDraw {
            vertices: &POSITIONS,
            colors: &green,
            viewport: [3, 2],
        },
        TriangleDraw {
            vertices: &POSITIONS,
            colors: &blue,
            viewport: [1, 1],
        },
        TriangleDraw {
            vertices: &POSITIONS,
            colors: &red,
            viewport: [6, 4],
        },
    ];
    let mut states = [
        TriangleState::default(),
        TriangleState {
            viewport_origin: [3, 1],
            scissor: Some([4, 0, 1, 4]),
            ..TriangleState::default()
        },
        TriangleState {
            viewport_origin: [5, 3],
            ..TriangleState::default()
        },
        TriangleState {
            scissor: Some([0, 0, 0, 4]),
            ..TriangleState::default()
        },
    ];
    // An empty draw before the blue draw must still advance input-buffer offsets.
    draws.swap(2, 3);
    states.swap(2, 3);
    let actual = context()
        .render_triangle_pass_with_state_rgba8(
            &compiled.msl,
            ENTRIES,
            &TrianglePass {
                draws: &draws,
                states: &states,
                extent: [6, 4],
                clear: CLEAR,
                clear_depth: None,
            },
        )
        .unwrap();
    for (i, &pixel) in actual.as_chunks::<4>().0.iter().enumerate() {
        let (x, y) = (i % 6, i / 6);
        let expected = if x < 2 {
            [255, 0, 0, 255]
        } else if x == 4 && (1..3).contains(&y) {
            [0, 255, 0, 255]
        } else if x == 5 && y == 3 {
            [0, 0, 255, 255]
        } else {
            CLEAR
        };
        assert_eq!(pixel, expected, "pixel {x},{y}");
    }
}

#[test]
fn stateful_pass_rejects_viewport_and_scissor_overflow_before_compilation() {
    use xenora_metal::{TriangleDraw, TrianglePass, TriangleState};
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 4],
    }];
    let gpu = context();
    for state in [
        TriangleState {
            viewport_origin: [1, 0],
            ..TriangleState::default()
        },
        TriangleState {
            viewport_origin: [usize::MAX, 0],
            ..TriangleState::default()
        },
        TriangleState {
            viewport_origin: [0, usize::MAX],
            ..TriangleState::default()
        },
        TriangleState {
            scissor: Some([3, 0, 2, 1]),
            ..TriangleState::default()
        },
        TriangleState {
            scissor: Some([usize::MAX, 0, 2, 1]),
            ..TriangleState::default()
        },
        TriangleState {
            scissor: Some([0, usize::MAX, 1, 2]),
            ..TriangleState::default()
        },
    ] {
        let err = gpu
            .render_triangle_pass_with_state_rgba8(
                "invalid shader",
                ENTRIES,
                &TrianglePass {
                    draws: &draws,
                    states: &[state],
                    extent: [4, 4],
                    clear: CLEAR,
                    clear_depth: None,
                },
            )
            .unwrap_err();
        assert_eq!(err.operation, "render bounds");
    }
}

#[test]
fn translated_pixel_arithmetic_matches_literal_colors() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-pixel-arithmetic-ps.bin"),
    )
    .unwrap();
    let gpu = context();
    // Original program computes (r/2+1/4, g/2, b+1/4, a). All arithmetic
    // below is exact binary FP32; literal bytes include RGBA8 conversion.
    for (color, expected) in [
        ([0.5, 1.0, 0.25, 1.0], [128, 128, 128, 255]),
        ([0.0, 0.0, 0.75, 1.0], [64, 0, 255, 255]),
        ([1.0, 0.5, 0.0, 0.0], [191, 64, 64, 0]),
    ] {
        let output = gpu
            .render_rgba8(
                &compiled.msl,
                ENTRIES,
                &POSITIONS,
                &[color; 3],
                [4, 3],
                CLEAR,
            )
            .unwrap();
        assert_eq!(output, expected.repeat(12));
    }
}

#[test]
fn sampled_pass_rejects_binding_count_before_shader_compilation() {
    use xenora_metal::{TriangleDraw, TrianglePass, TriangleState};
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let states = [TriangleState::default()];
    let pass = TrianglePass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clear: CLEAR,
        clear_depth: None,
    };
    let error = gpu
        .render_sampled_triangle_pass_rgba8("invalid shader", ENTRIES, &pass, &[])
        .unwrap_err();
    assert_eq!(error.operation, "render bindings");
}

#[test]
fn sampled_pass_combines_uploads_arithmetic_depth_blending_and_clipping() {
    use xenora_metal::{
        BindingSlot, BlendFactor, BlendOperation, BlendState, BoundSampledTexture, DepthCompare,
        DepthState, MetalRgba8Texture, MetalRgba8TextureDesc, MetalSampler, Rgba8MipLevel,
        TriangleDraw, TrianglePass, TriangleState,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin"),
    )
    .unwrap();
    let sampler =
        MetalSampler::new(gpu.device(), xenora_texture::SamplerDesc::nearest_clamp()).unwrap();
    let make_binding = |rgba: &[u8]| {
        let texture = MetalRgba8Texture::upload(
            gpu.device(),
            MetalRgba8TextureDesc::new(1, 1, 1).unwrap(),
            &[Rgba8MipLevel::new_packed(0, 1, 1, rgba)],
        )
        .unwrap();
        BoundSampledTexture::bind(
            texture,
            sampler.clone(),
            BindingSlot::new(0),
            BindingSlot::new(0),
        )
        .unwrap()
    };
    let red = make_binding(&[255, 0, 0, 255]);
    let changing = make_binding(&[0, 255, 0, 255]);
    let mut bindings = [red.clone(), changing.clone(), changing.clone()];
    // Different texture AND sampler for the third draw: its base mip is blue,
    // but its sampler clamps explicit LOD zero up to mip 1, which is yellow.
    let distinct_texture = MetalRgba8Texture::upload(
        gpu.device(),
        MetalRgba8TextureDesc::new(2, 2, 2).unwrap(),
        &[
            Rgba8MipLevel::new_packed(0, 2, 2, &[0, 0, 255, 255].repeat(4)),
            Rgba8MipLevel::new_packed(1, 1, 1, &[255, 255, 0, 255]),
        ],
    )
    .unwrap();
    let distinct_sampler = MetalSampler::new(
        gpu.device(),
        xenora_texture::SamplerDesc::nearest_clamp().with_lod_range(1.0, 1.0),
    )
    .unwrap();
    let distinct = BoundSampledTexture::bind(
        distinct_texture,
        distinct_sampler,
        BindingSlot::new(0),
        BindingSlot::new(0),
    )
    .unwrap();
    assert!(
        bindings[0]
            .texture()
            .upload_mip(&Rgba8MipLevel::new_packed(0, usize::MAX, 1, &[]))
            .is_err()
    );
    assert!(
        bindings[0]
            .texture()
            .upload_mip(&Rgba8MipLevel {
                level: 0,
                width: 1,
                height: 1,
                row_pitch: usize::MAX,
                bytes: &[0; 4]
            })
            .is_err()
    );
    // Binding clones retain resources after the original owners leave scope.
    drop(red);
    drop(changing);
    drop(sampler);
    let near = POSITIONS.map(|[x, y, _, w]| [x, y, 0.25, w]);
    let far = POSITIONS.map(|[x, y, _, w]| [x, y, 0.75, w]);
    let front = POSITIONS.map(|[x, y, _, w]| [x, y, 0.125, w]);
    let uv = [[0.5, 0.5, 0.0, 1.0]; 3];
    let draws = [&near, &far, &front].map(|vertices| TriangleDraw {
        vertices,
        colors: &uv,
        viewport: [4, 4],
    });
    let depth = DepthState {
        compare: DepthCompare::Less,
        write: true,
    };
    let additive = BlendState {
        source_rgb: BlendFactor::One,
        destination_rgb: BlendFactor::One,
        rgb_operation: BlendOperation::Add,
        source_alpha: BlendFactor::One,
        destination_alpha: BlendFactor::One,
        alpha_operation: BlendOperation::Add,
    };
    let states = [
        TriangleState {
            depth,
            scissor: Some([0, 0, 2, 4]),
            ..Default::default()
        },
        TriangleState {
            depth,
            ..Default::default()
        },
        TriangleState {
            depth,
            blend: Some(additive),
            scissor: Some([0, 0, 1, 1]),
            ..Default::default()
        },
    ];
    let pass = TrianglePass {
        draws: &draws,
        states: &states,
        extent: [4, 4],
        clear: CLEAR,
        clear_depth: Some(1.0),
    };
    for frame in 0..32 {
        if frame == 16 {
            bindings[2] = distinct.clone();
        }
        let texel = if frame % 2 == 0 {
            [0, 255, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
        bindings[1]
            .texture()
            .upload_mip(&Rgba8MipLevel::new_packed(0, 1, 1, &texel))
            .unwrap();
        let result = gpu
            .render_sampled_triangle_pass_rgba8(&shader.msl, ENTRIES, &pass, &bindings)
            .unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let expected = if x == 0 && y == 0 {
                    if frame < 16 {
                        [128, texel[1], texel[2], 255]
                    } else {
                        [255, 255, 0, 255]
                    }
                } else if x < 2 {
                    [128, 0, 0, 255]
                } else {
                    texel
                };
                assert_eq!(
                    &result[(y * 4 + x) * 4..(y * 4 + x + 1) * 4],
                    &expected,
                    "frame {frame}, pixel {x},{y}"
                );
            }
        }
    }
    let invalid = BoundSampledTexture::bind(
        bindings[0].texture().clone(),
        bindings[0].sampler().clone(),
        BindingSlot::new(1),
        BindingSlot::new(0),
    )
    .unwrap();
    let error = gpu
        .render_sampled_triangle_pass_rgba8(
            "invalid shader",
            ENTRIES,
            &pass,
            &[invalid, bindings[1].clone(), bindings[2].clone()],
        )
        .unwrap_err();
    assert_eq!(error.operation, "render bindings");
}

#[test]
fn multiple_targets_preserve_slot_order_clears_depth_and_clipping() {
    use xenora_metal::{
        DepthCompare, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let source = r#"#include <metal_stdlib>
using namespace metal;
struct Vertex { float4 position [[position]]; float4 color; };
vertex Vertex multi_vertex(uint id [[vertex_id]], device const float4* p [[buffer(0)]], device const float4* c [[buffer(1)]]) {
    Vertex v; v.position=p[id]; v.color=c[id]; return v;
}
struct Pixel { float4 first [[color(0)]]; float4 second [[color(1)]]; };
fragment Pixel multi_pixel(Vertex v [[stage_in]]) {
    Pixel p; p.first=v.color; p.second=v.color.brga; return p;
}
"#;
    let near = POSITIONS.map(|[x, y, _, w]| [x, y, 0.25, w]);
    let far = POSITIONS.map(|[x, y, _, w]| [x, y, 0.75, w]);
    let red = [[1.0, 0.0, 0.0, 1.0]; 3];
    let blue = [[0.0, 0.0, 1.0, 1.0]; 3];
    let draws = [
        TriangleDraw {
            vertices: &near,
            colors: &red,
            viewport: [4, 3],
        },
        TriangleDraw {
            vertices: &far,
            colors: &blue,
            viewport: [4, 3],
        },
    ];
    let depth = DepthState {
        compare: DepthCompare::Less,
        write: true,
    };
    let states = [
        TriangleState {
            depth,
            scissor: Some([0, 0, 2, 2]),
            ..Default::default()
        },
        TriangleState {
            depth,
            scissor: Some([0, 0, 4, 2]),
            ..Default::default()
        },
    ];
    let clears = [[1, 2, 3, 4], [5, 6, 7, 8]];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: Some(1.0),
    };
    let outputs = gpu
        .render_triangle_targets_rgba8(source, ["multi_vertex", "multi_pixel"], &pass)
        .unwrap();
    assert_eq!(outputs.len(), 2);
    for (slot, output) in outputs.iter().enumerate() {
        assert_eq!(output.len(), 48);
        for y in 0..3 {
            for x in 0..4 {
                let expected = if y == 2 {
                    clears[slot]
                } else if slot == 0 {
                    if x < 2 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }
                } else if x < 2 {
                    [0, 255, 0, 255]
                } else {
                    [255, 0, 0, 255]
                };
                assert_eq!(
                    &output[(y * 4 + x) * 4..(y * 4 + x + 1) * 4],
                    &expected,
                    "slot {slot} pixel {x},{y}"
                );
            }
        }
    }
}

#[test]
fn multiple_targets_reaches_eight_and_rejects_invalid_counts_before_compile() {
    use xenora_metal::{TriangleDraw, TriangleState, TriangleTargetsPass};
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [2, 2],
    }];
    let states = [TriangleState::default()];
    let clears = [[17, 31, 47, 255]; 8];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [2, 2],
        clears: &clears,
        clear_depth: None,
    };
    let expected = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    let mut source = String::from(
        "#include <metal_stdlib>\nusing namespace metal;\nvertex float4 targets_vertex(uint id [[vertex_id]], device const float4* p [[buffer(0)]]) { return p[id]; }\nstruct Pixel {\n",
    );
    for slot in 0..8 {
        source.push_str(&format!("float4 c{slot} [[color({slot})]];\n"));
    }
    source.push_str("};\nfragment Pixel targets_pixel() { Pixel p;\n");
    for (slot, pixel) in expected.iter().enumerate() {
        source.push_str(&format!(
            "p.c{slot}=float4({},{},{},1);\n",
            pixel[0] / 255,
            pixel[1] / 255,
            pixel[2] / 255
        ));
    }
    source.push_str("return p; }\n");
    let output = gpu
        .render_triangle_targets_rgba8(&source, ["targets_vertex", "targets_pixel"], &pass)
        .unwrap();
    assert_eq!(output.len(), 8);
    for (actual, expected) in output.iter().zip(expected) {
        assert_eq!(actual, &expected.repeat(4));
    }
    for count in [0, 9] {
        let invalid = vec![[0; 4]; count];
        let invalid_pass = TriangleTargetsPass {
            clears: &invalid,
            ..pass
        };
        let error = gpu
            .render_triangle_targets_rgba8(
                "invalid shader",
                ["targets_vertex", "targets_pixel"],
                &invalid_pass,
            )
            .unwrap_err();
        assert_eq!(error.operation, "render targets");
    }
}

#[test]
fn translated_mrt_exports_reach_all_eight_native_attachments() {
    use xenora_metal::{TriangleDraw, TriangleState, TriangleTargetsPass};
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-mrt-ps.bin"),
    )
    .unwrap();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let states = [TriangleState::default()];
    let clears = [CLEAR; 8];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let output = gpu
        .render_triangle_targets_rgba8(&shader.msl, ENTRIES, &pass)
        .unwrap();
    let expected = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    assert_eq!(output.len(), 8);
    for (slot, (actual, expected)) in output.iter().zip(expected).enumerate() {
        assert_eq!(actual, &expected.repeat(12), "MRT{slot}");
    }
    // Blend and mask must be configured on every attachment, not just MRT0.
    use xenora_metal::{BlendFactor, BlendOperation, BlendState, ColorWriteMask};
    let masked = [
        TriangleState {
            scissor: Some([0, 0, 2, 3]),
            blend: Some(BlendState {
                source_rgb: BlendFactor::One,
                destination_rgb: BlendFactor::One,
                rgb_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
            }),
            write_mask: ColorWriteMask {
                red: true,
                green: false,
                blue: true,
                alpha: false,
            },
            ..Default::default()
        },
        TriangleState {
            scissor: Some([2, 0, 2, 3]),
            ..Default::default()
        },
    ];
    let clears: [[u8; 4]; 8] =
        std::array::from_fn(|slot| [17, 31 + slot as u8, 47, 53 + slot as u8]);
    let masked_draws = [draws[0], draws[0]];
    let masked_pass = TriangleTargetsPass {
        draws: &masked_draws,
        states: &masked,
        clears: &clears,
        ..pass
    };
    let output = gpu
        .render_triangle_targets_rgba8(&shader.msl, ENTRIES, &masked_pass)
        .unwrap();
    assert_eq!(output.len(), 8);
    for (slot, (actual, color)) in output.iter().zip(expected).enumerate() {
        let pixel = [
            if color[0] == 0 { 17 } else { 255 },
            clears[slot][1],
            if color[2] == 0 { 47 } else { 255 },
            clears[slot][3],
        ];
        for y in 0..3 {
            for x in 0..4 {
                let expected = if x < 2 { pixel } else { color };
                assert_eq!(
                    &actual[(y * 4 + x) * 4..(y * 4 + x + 1) * 4],
                    &expected,
                    "masked/restored MRT{slot} pixel {x},{y}"
                );
            }
        }
    }
}

#[test]
fn one_target_wrapper_matches_mrt_with_state_and_invalid_inputs() {
    use xenora_metal::{
        BlendFactor, BlendOperation, BlendState, ColorWriteMask, DepthCompare, DepthState,
        TriangleDraw, TrianglePass, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }; 2];
    let states = [
        TriangleState {
            depth: DepthState {
                compare: DepthCompare::Less,
                write: true,
            },
            blend: Some(BlendState {
                source_rgb: BlendFactor::One,
                destination_rgb: BlendFactor::One,
                rgb_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
            }),
            write_mask: ColorWriteMask {
                red: true,
                green: false,
                blue: false,
                alpha: false,
            },
            scissor: Some([0, 0, 2, 2]),
            ..Default::default()
        },
        TriangleState {
            scissor: Some([2, 0, 2, 2]),
            ..Default::default()
        },
    ];
    let clear = [0, 0, 255, 255];
    let clears = [clear];
    let mut single = TrianglePass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clear,
        clear_depth: Some(1.0),
    };
    let mut multi = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: Some(1.0),
    };
    let one = gpu
        .render_triangle_pass_with_state_rgba8(&shader.msl, ENTRIES, &single)
        .unwrap();
    let many = gpu
        .render_triangle_targets_rgba8(&shader.msl, ENTRIES, &multi)
        .unwrap();
    assert_eq!(many.as_slice(), std::slice::from_ref(&one));
    for y in 0..3 {
        for x in 0..4 {
            let expected = if y == 2 {
                clear
            } else if x < 2 {
                [255, 0, 255, 255]
            } else {
                [255, 0, 0, 255]
            };
            assert_eq!(&one[(y * 4 + x) * 4..(y * 4 + x + 1) * 4], &expected);
        }
    }
    single.extent = [0, 3];
    multi.extent = [0, 3];
    let a = gpu
        .render_triangle_pass_with_state_rgba8("invalid shader", ENTRIES, &single)
        .unwrap_err();
    let b = gpu
        .render_triangle_targets_rgba8("invalid shader", ENTRIES, &multi)
        .unwrap_err();
    assert_eq!((a.operation, a.detail), (b.operation, b.detail));
}

#[test]
fn retained_depth_loads_across_submissions_and_commits_only_on_success() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-mrt-ps.bin"),
    )
    .unwrap();
    let mut depth = gpu.create_depth_target([4, 3]).unwrap();
    let palette = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    let clears: [[u8; 4]; 8] = std::array::from_fn(|slot| [17 + slot as u8, 31, 47, 255]);
    let colors = [[0.0; 4]; 3];
    let mut render = |z: f32, scissor, compare, load, source: &str| {
        let positions = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
        let draws = [TriangleDraw {
            vertices: &positions,
            colors: &colors,
            viewport: [4, 3],
        }];
        let states = [TriangleState {
            depth: DepthState {
                compare,
                write: true,
            },
            scissor,
            ..Default::default()
        }];
        gpu.render_triangle_targets_with_depth_rgba8(
            source,
            ENTRIES,
            &TriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [4, 3],
                clears: &clears,
                clear_depth: None,
            },
            &mut depth,
            load,
        )
    };
    assert_eq!(
        render(0.25, None, DepthCompare::Less, DepthLoad::Load, "invalid")
            .unwrap_err()
            .operation,
        "depth target"
    );
    let first = render(
        0.25,
        Some([0, 0, 2, 3]),
        DepthCompare::Less,
        DepthLoad::Clear(1.0),
        &shader.msl,
    )
    .unwrap();
    let second = render(0.75, None, DepthCompare::Less, DepthLoad::Load, &shader.msl).unwrap();
    assert_eq!(first.len(), 8);
    assert_eq!(second.len(), 8);
    for slot in 0..8 {
        for pixel in 0..12 {
            assert_eq!(
                &first[slot][pixel * 4..pixel * 4 + 4],
                if pixel % 4 < 2 {
                    &palette[slot]
                } else {
                    &clears[slot]
                }
            );
            assert_eq!(
                &second[slot][pixel * 4..pixel * 4 + 4],
                if pixel % 4 < 2 {
                    &clears[slot]
                } else {
                    &palette[slot]
                }
            );
        }
    }
    // A valid closer draw updates retained depth everywhere, not just its first pass.
    let closer = render(
        0.125,
        None,
        DepthCompare::Less,
        DepthLoad::Load,
        &shader.msl,
    )
    .unwrap();
    assert_eq!(closer.len(), 8);
    for slot in 0..8 {
        assert_eq!(closer[slot], palette[slot].repeat(12));
    }
    // Failed clear/compilation submissions must not erase the previous occluder.
    assert!(
        render(
            0.0,
            None,
            DepthCompare::Less,
            DepthLoad::Clear(1.0),
            "invalid shader"
        )
        .is_err()
    );
    for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            render(
                0.0,
                None,
                DepthCompare::Less,
                DepthLoad::Clear(invalid),
                &shader.msl
            )
            .is_err()
        );
    }
    let occluded = render(0.25, None, DepthCompare::Less, DepthLoad::Load, &shader.msl).unwrap();
    assert_eq!(occluded.len(), 8);
    for slot in 0..8 {
        assert_eq!(occluded[slot], clears[slot].repeat(12));
    }
    // An empty Load pass still preserves every copied depth value.
    let empty = render(
        0.0,
        Some([0, 0, 0, 3]),
        DepthCompare::Less,
        DepthLoad::Load,
        &shader.msl,
    )
    .unwrap();
    assert_eq!(empty.len(), 8);
    for slot in 0..8 {
        assert_eq!(empty[slot], clears[slot].repeat(12));
    }
    let still_occluded =
        render(0.25, None, DepthCompare::Less, DepthLoad::Load, &shader.msl).unwrap();
    assert_eq!(still_occluded.len(), 8);
    for slot in 0..8 {
        assert_eq!(still_occluded[slot], clears[slot].repeat(12));
    }
    // Attachment clears occur even when all draws have an empty scissor.
    let empty_clear = render(
        0.0,
        Some([0, 0, 0, 3]),
        DepthCompare::Less,
        DepthLoad::Clear(1.0),
        &shader.msl,
    )
    .unwrap();
    assert_eq!(empty_clear.len(), 8);
    for slot in 0..8 {
        assert_eq!(empty_clear[slot], clears[slot].repeat(12));
    }
    let after_empty_clear =
        render(0.75, None, DepthCompare::Less, DepthLoad::Load, &shader.msl).unwrap();
    assert_eq!(after_empty_clear.len(), 8);
    for slot in 0..8 {
        assert_eq!(after_empty_clear[slot], palette[slot].repeat(12));
    }
    // Explicit clear resets retained contents and supports reversed depth.
    let reset = render(
        0.75,
        None,
        DepthCompare::Greater,
        DepthLoad::Clear(0.0),
        &shader.msl,
    )
    .unwrap();
    let reversed = render(
        0.5,
        None,
        DepthCompare::Greater,
        DepthLoad::Load,
        &shader.msl,
    )
    .unwrap();
    assert_eq!(reset.len(), 8);
    assert_eq!(reversed.len(), 8);
    for slot in 0..8 {
        assert_eq!(reset[slot], palette[slot].repeat(12));
        assert_eq!(reversed[slot], clears[slot].repeat(12));
    }
}

#[test]
fn retained_depth_rejects_extent_and_ambiguous_clear_before_compilation() {
    use xenora_metal::{DepthLoad, TriangleDraw, TriangleState, TriangleTargetsPass};
    let gpu = context();
    for extent in [[0, 1], [1, 0], [4097, 1], [usize::MAX, 2]] {
        assert!(gpu.create_depth_target(extent).is_err());
    }
    let mut depth = gpu.create_depth_target([4, 3]).unwrap();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let states = [TriangleState::default()];
    let mut pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 4],
        clears: &[CLEAR],
        clear_depth: None,
    };
    let error = gpu
        .render_triangle_targets_with_depth_rgba8(
            "invalid shader",
            ENTRIES,
            &pass,
            &mut depth,
            DepthLoad::Clear(1.0),
        )
        .unwrap_err();
    assert_eq!(error.operation, "depth target");
    pass.extent = [4, 3];
    pass.clear_depth = Some(1.0);
    let error = gpu
        .render_triangle_targets_with_depth_rgba8(
            "invalid shader",
            ENTRIES,
            &pass,
            &mut depth,
            DepthLoad::Clear(1.0),
        )
        .unwrap_err();
    assert_eq!(error.operation, "depth target");
    pass.clear_depth = None;
    // Neither failed attempt initialized the surface.
    let error = gpu
        .render_triangle_targets_with_depth_rgba8(
            "invalid shader",
            ENTRIES,
            &pass,
            &mut depth,
            DepthLoad::Load,
        )
        .unwrap_err();
    assert_eq!(error.operation, "depth target");
}

#[test]
fn retained_depth_read_only_draws_preserve_prior_values_across_queues() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let first_gpu = context();
    let second_gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let mut depth = first_gpu.create_depth_target([4, 3]).unwrap();
    let colors = [[0.0; 4]; 3];
    // First pass clears depth but has no depth writes. Then persist .75, execute
    // a nearer read-only draw, and verify .5 still passes against .75, not .25.
    for (index, (z, write, load, expected)) in [
        (0.0, false, DepthLoad::Clear(1.0), [255, 0, 0, 255]),
        (0.75, true, DepthLoad::Load, [255, 0, 0, 255]),
        (0.25, false, DepthLoad::Load, [255, 0, 0, 255]),
        (0.5, true, DepthLoad::Load, [255, 0, 0, 255]),
        (0.625, true, DepthLoad::Load, CLEAR),
    ]
    .into_iter()
    .enumerate()
    {
        let positions = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
        let draws = [TriangleDraw {
            vertices: &positions,
            colors: &colors,
            viewport: [4, 3],
        }];
        let states = [TriangleState {
            depth: DepthState {
                compare: DepthCompare::Less,
                write,
            },
            ..Default::default()
        }];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent: [4, 3],
            clears: &[CLEAR],
            clear_depth: None,
        };
        // Each call completes synchronously before another queue uses the surface.
        let gpu = if index % 2 == 0 {
            &first_gpu
        } else {
            &second_gpu
        };
        let output = gpu
            .render_triangle_targets_with_depth_rgba8(&shader.msl, ENTRIES, &pass, &mut depth, load)
            .unwrap();
        assert_eq!(output, vec![expected.repeat(12)], "submission {index}");
    }
}

#[test]
fn depth_snapshots_transfer_padded_rows_as_owned_thread_safe_data() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    for [width, height] in [[1, 5], [65, 3], [1, 4096]] {
        let mut depth = gpu.create_depth_target([width, height]).unwrap();
        assert!(gpu.snapshot_depth(&depth).is_err());
        let colors = [[0.0; 4]; 3];
        let positions = [0.25, 0.75, 0.125].map(|z| POSITIONS.map(|[x, y, _, w]| [x, y, z, w]));
        let draws: Vec<_> = positions
            .iter()
            .map(|vertices| TriangleDraw {
                vertices,
                colors: &colors,
                viewport: [width, height],
            })
            .collect();
        let scissors = [[0, 0, width - 1, 1], [width - 1, 1, 1, 1], [0, 2, width, 1]];
        let states = scissors.map(|scissor| TriangleState {
            depth: DepthState {
                compare: DepthCompare::Less,
                write: true,
            },
            scissor: Some(scissor),
            ..Default::default()
        });
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent: [width, height],
            clears: &[CLEAR],
            clear_depth: None,
        };
        gpu.render_triangle_targets_with_depth_rgba8(
            &shader.msl,
            ENTRIES,
            &pass,
            &mut depth,
            DepthLoad::Clear(1.0),
        )
        .unwrap();
        let snapshot = gpu.snapshot_depth(&depth).unwrap();
        assert_eq!(
            snapshot.byte_len(),
            width * height * 4,
            "padding must not remain retained"
        );
        let snapshot = std::thread::spawn(move || snapshot.clone()).join().unwrap();
        drop(depth);
        let mut restored = gpu.restore_depth(&snapshot).unwrap();
        for probe in [0.5, 0.875] {
            let far = POSITIONS.map(|[x, y, _, w]| [x, y, probe, w]);
            let draws = [TriangleDraw {
                vertices: &far,
                colors: &colors,
                viewport: [width, height],
            }];
            let states = [TriangleState {
                depth: DepthState {
                    compare: DepthCompare::Less,
                    write: false,
                },
                ..Default::default()
            }];
            let pass = TriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [width, height],
                clears: &[CLEAR],
                clear_depth: None,
            };
            let out = gpu
                .render_triangle_targets_with_depth_rgba8(
                    &shader.msl,
                    ENTRIES,
                    &pass,
                    &mut restored,
                    DepthLoad::Load,
                )
                .unwrap();
            assert_eq!(out.len(), 1);
            for (index, pixel) in out[0].as_chunks::<4>().0.iter().enumerate() {
                let (x, y) = (index % width, index / width);
                let stored = if y == 0 && x < width - 1 {
                    0.25
                } else if y == 1 && x == width - 1 {
                    0.75
                } else if y == 2 {
                    0.125
                } else {
                    1.0
                };
                assert_eq!(
                    pixel,
                    if probe < stored {
                        &[255, 0, 0, 255]
                    } else {
                        &CLEAR
                    },
                    "{width}x{height}, probe {probe}, ({x},{y})"
                );
            }
        }
    }
}

#[test]
fn translated_forward_branches_preserve_geometry_and_select_literal_color() {
    let compiled = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-forward-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-forward-ps.bin"),
    )
    .unwrap();
    let gpu = context();
    let actual = gpu
        .render_rgba8(
            &compiled.msl,
            ENTRIES,
            &POSITIONS,
            &[[0.0; 4]; 3],
            [4, 3],
            CLEAR,
        )
        .unwrap();
    assert_eq!(actual, [128, 0, 255, 255].repeat(12));
}

#[test]
fn loaded_color_targets_preserve_pixels_mix_clears_and_blend() {
    use xenora_metal::{
        BlendFactor, BlendOperation, BlendState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-mrt-ps.bin"),
    )
    .unwrap();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let clears = [[7, 11, 13, 17]; 8];
    // Each loaded slot has its own spatial pattern: reusing any other slot fails.
    let prior: [Vec<u8>; 8] = std::array::from_fn(|slot| {
        (0..12u8)
            .flat_map(|p| {
                let base = 10 * slot as u8;
                [20 + base + p, 40 + base + p, 60 + base + p, 80 + base + p]
            })
            .collect()
    });
    let saved = prior.clone();
    let loads: [Option<&[u8]>; 8] =
        std::array::from_fn(|slot| (slot % 2 == 0).then_some(prior[slot].as_slice()));
    let exported = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    for additive in [false, true] {
        let states = [TriangleState {
            scissor: Some([1, 1, 2, 1]),
            blend: additive.then_some(BlendState {
                source_rgb: BlendFactor::One,
                destination_rgb: BlendFactor::One,
                rgb_operation: BlendOperation::Add,
                source_alpha: BlendFactor::One,
                destination_alpha: BlendFactor::One,
                alpha_operation: BlendOperation::Add,
            }),
            ..Default::default()
        }];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent: [4, 3],
            clears: &clears,
            clear_depth: None,
        };
        let output = gpu
            .render_loaded_triangle_targets_rgba8(&shader.msl, ENTRIES, &pass, None, None, &loads)
            .unwrap();
        assert_eq!(output.len(), 8);
        for slot in 0..8 {
            for p in 0..12 {
                let background: [u8; 4] = if slot % 2 == 0 {
                    [
                        20 + 10 * slot as u8 + p as u8,
                        40 + 10 * slot as u8 + p as u8,
                        60 + 10 * slot as u8 + p as u8,
                        80 + 10 * slot as u8 + p as u8,
                    ]
                } else {
                    [7, 11, 13, 17]
                };
                let expected = if p == 5 || p == 6 {
                    if additive {
                        std::array::from_fn(|c| background[c].saturating_add(exported[slot][c]))
                    } else {
                        exported[slot]
                    }
                } else {
                    background
                };
                assert_eq!(
                    &output[slot][p * 4..p * 4 + 4],
                    &expected,
                    "additive={additive}, slot={slot}, pixel={p}"
                );
            }
        }
    }
    let pass = TriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let output = gpu
        .render_loaded_triangle_targets_rgba8(&shader.msl, ENTRIES, &pass, None, None, &loads)
        .unwrap();
    for (slot, bytes) in output.iter().enumerate() {
        assert_eq!(
            bytes,
            &if slot % 2 == 0 {
                saved[slot].clone()
            } else {
                [7, 11, 13, 17].repeat(12)
            }
        );
    }
    assert_eq!(prior, saved);
}

#[test]
fn loaded_color_targets_reject_count_and_byte_lengths_before_compile() {
    use xenora_metal::TriangleTargetsPass;
    let gpu = context();
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &[],
        states: &[],
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let short = [0u8; 47];
    let long = [0u8; 49];
    for loads in [
        vec![],
        vec![None, None],
        vec![Some(short.as_slice())],
        vec![Some(long.as_slice())],
    ] {
        let error = gpu
            .render_loaded_triangle_targets_rgba8(
                "invalid shader",
                ENTRIES,
                &pass,
                None,
                None,
                &loads,
            )
            .unwrap_err();
        assert_eq!(error.operation, "color loads");
    }
}

#[test]
fn loaded_color_last_slot_failure_preserves_initialized_depth() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-mrt-ps.bin"),
    )
    .unwrap();
    let mut depth = gpu.create_depth_target([4, 3]).unwrap();
    let colors = [[0.0; 4]; 3];
    let clears = [CLEAR; 8];
    let valid = [23u8; 48];
    let short = [29u8; 47];
    let invalid_loads = [
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(valid.as_slice()),
        Some(short.as_slice()),
    ];
    let exported = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    // Seed .5, then request a clear to zero and an Always-writing .25 draw.
    // Neither the clear nor the write may survive rejection of the last slot.
    for (step, z, compare, write, load) in [
        (0, 0.5, DepthCompare::Less, true, DepthLoad::Clear(1.0)),
        (1, 0.25, DepthCompare::Always, true, DepthLoad::Clear(0.0)),
        (2, 0.375, DepthCompare::Less, false, DepthLoad::Load),
        (3, 0.75, DepthCompare::Less, false, DepthLoad::Load),
    ] {
        let positions = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
        let draws = [TriangleDraw {
            vertices: &positions,
            colors: &colors,
            viewport: [4, 3],
        }];
        let states = [TriangleState {
            depth: DepthState { compare, write },
            ..Default::default()
        }];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent: [4, 3],
            clears: &clears,
            clear_depth: None,
        };
        if step == 1 {
            let error = gpu
                .render_loaded_triangle_targets_rgba8(
                    &shader.msl,
                    ENTRIES,
                    &pass,
                    None,
                    Some((&mut depth, load)),
                    &invalid_loads,
                )
                .unwrap_err();
            assert_eq!(error.operation, "color loads");
        } else {
            let output = gpu
                .render_loaded_triangle_targets_rgba8(
                    &shader.msl,
                    ENTRIES,
                    &pass,
                    None,
                    Some((&mut depth, load)),
                    &[None; 8],
                )
                .unwrap();
            assert_eq!(output.len(), 8);
            for (slot, actual) in output.iter().enumerate() {
                // .375 < original .5 passes; .75 < original .5 fails.
                // Read-only probes cannot alter the depth being checked.
                let expected = if step == 3 { CLEAR } else { exported[slot] };
                assert_eq!(actual, &expected.repeat(12), "step={step}, slot={slot}");
            }
        }
    }
    assert_eq!(valid, [23; 48]);
    assert_eq!(short, [29; 47]);
}

#[test]
fn translated_scene_limits_validate_before_compilation_and_allocation() {
    use xenora_metal::{
        MAX_RENDER_DIMENSION, MAX_RENDER_DRAWS, MAX_RENDER_PIXELS, MAX_RENDER_VERTICES,
        TriangleDraw,
    };
    assert_eq!(MAX_RENDER_VERTICES, 65535);
    assert_eq!(MAX_RENDER_PIXELS, 1024 * 1024);
    assert_eq!(MAX_RENDER_DIMENSION, 4096);
    assert_eq!(MAX_RENDER_DRAWS, 256);
    let gpu = context();
    let positions = vec![POSITIONS[0]; 65538];
    let colors = vec![[0.0; 4]; 65538];
    // Empty source is a deterministic sentinel after input validation, before
    // native compilation/uploads: accepted bounds must reach source bounds.
    for (count, operation) in [
        (771, "source bounds"),
        (65535, "source bounds"),
        (65536, "render input"),
        (65538, "render input"),
    ] {
        let error = gpu
            .render_triangles_rgba8(
                "",
                ENTRIES,
                &positions[..count],
                &colors[..count],
                [4, 3],
                CLEAR,
            )
            .unwrap_err();
        assert_eq!(error.operation, operation, "vertices={count}");
    }
    for (extent, operation) in [
        ([1024, 1024], "source bounds"),
        ([4096, 256], "source bounds"),
        ([256, 4096], "source bounds"),
        ([1024, 1025], "render bounds"),
        ([4097, 1], "render bounds"),
        ([1, 4097], "render bounds"),
        ([usize::MAX, 2], "render bounds"),
    ] {
        let error = gpu
            .render_triangles_rgba8("", ENTRIES, &POSITIONS, &colors[..3], extent, CLEAR)
            .unwrap_err();
        assert_eq!(error.operation, operation, "extent={extent:?}");
    }
    let draw = TriangleDraw {
        vertices: &positions[..3],
        colors: &colors[..3],
        viewport: [4, 3],
    };
    for (count, operation) in [(256, "source bounds"), (257, "render input")] {
        let error = gpu
            .render_triangle_pass_rgba8("", ENTRIES, &vec![draw; count], [4, 3], CLEAR)
            .unwrap_err();
        assert_eq!(error.operation, operation, "draws={count}");
    }
    for (count, operation) in [(65532, "source bounds"), (65535, "render input")] {
        let draws = [
            TriangleDraw {
                vertices: &positions[..count],
                colors: &colors[..count],
                ..draw
            },
            draw,
        ];
        let error = gpu
            .render_triangle_pass_rgba8("", ENTRIES, &draws, [4, 3], CLEAR)
            .unwrap_err();
        assert_eq!(error.operation, operation, "aggregate={}", count + 3);
    }
    for extent in [[4097, 1], [1, 4097], [1024, 1025], [usize::MAX, 2]] {
        assert!(
            gpu.create_depth_target(extent).is_err(),
            "depth extent={extent:?}"
        );
    }
}

#[test]
fn translated_scene_native_boundaries_preserve_large_depth_snapshots() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleState, TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let colors = vec![[0.0; 4]; 65535];
    // All but the final triangle are degenerate and beyond the right clip plane.
    // This reaches the vertex boundary without thousands of fullscreen overdraws.
    let mut vertices = vec![[2.0, 0.0, 0.25, 1.0]; 65532];
    vertices.extend(POSITIONS.map(|[x, y, _, w]| [x, y, 0.25, w]));
    assert_eq!(vertices.len(), 65535);
    let states = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write: true,
        },
        ..Default::default()
    }];
    let clears = [CLEAR];
    let red = [255, 0, 0, 255].repeat(1024 * 1024);
    let untouched = CLEAR.repeat(1024 * 1024);
    for extent in [[4096, 256], [256, 4096]] {
        let mut depth = gpu.create_depth_target(extent).unwrap();
        let draws = [TriangleDraw {
            vertices: &vertices,
            colors: &colors,
            viewport: extent,
        }];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent,
            clears: &clears,
            clear_depth: None,
        };
        let output = gpu
            .render_triangle_targets_with_depth_rgba8(
                &shader.msl,
                ENTRIES,
                &pass,
                &mut depth,
                DepthLoad::Clear(1.0),
            )
            .unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0], red, "boundary draw extent={extent:?}");
        drop(output);
        let snapshot = gpu.snapshot_depth(&depth).unwrap();
        assert_eq!(snapshot.byte_len(), 4 * 1024 * 1024);
        drop(depth);
        let mut restored = gpu.restore_depth(&snapshot).unwrap();
        drop(snapshot);
        // Literal Less comparisons against .25, then the newly written .125:
        // .5 fails, .125 passes and writes, .2 subsequently fails.
        for (z, passes) in [(0.5, false), (0.125, true), (0.2, false)] {
            let probe = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
            let draws = [TriangleDraw {
                vertices: &probe,
                colors: &colors[..3],
                viewport: extent,
            }];
            let pass = TriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent,
                clears: &clears,
                clear_depth: None,
            };
            let output = gpu
                .render_triangle_targets_with_depth_rgba8(
                    &shader.msl,
                    ENTRIES,
                    &pass,
                    &mut restored,
                    DepthLoad::Load,
                )
                .unwrap();
            assert_eq!(output.len(), 1);
            assert_eq!(
                &output[0],
                if passes { &red } else { &untouched },
                "depth probe z={z}, extent={extent:?}"
            );
        }
    }
}

#[test]
fn translated_multi_pipeline_switches_and_restores_shader_selection() {
    use xenora_metal::{TriangleDraw, TriangleShader, TriangleState, TriangleTargetsPass};
    let gpu = context();
    let red = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let varying = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-varying-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-varying-ps.bin"),
    )
    .unwrap();
    let green = [[0.0, 1.0, 0.0, 1.0]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &green,
        viewport: [4, 3],
    }; 3];
    let states = [
        TriangleState::default(),
        TriangleState {
            scissor: Some([1, 0, 3, 3]),
            ..Default::default()
        },
        TriangleState {
            scissor: Some([3, 0, 1, 3]),
            ..Default::default()
        },
    ];
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let shaders = [
        TriangleShader {
            source: &red.msl,
            entries: ENTRIES,
        },
        TriangleShader {
            source: &varying.msl,
            entries: ENTRIES,
        },
        TriangleShader {
            source: &red.msl,
            entries: ENTRIES,
        },
    ];
    let pending = gpu
        .submit_multi_pipeline_triangle_targets_rgba8(
            &shaders,
            &pass,
            &[None, None, None],
            None,
            &[None],
        )
        .unwrap();
    // The pending ticket must not borrow the shader source or context.
    drop(red);
    drop(varying);
    drop(gpu);
    let output = pending.wait().unwrap();
    assert_eq!(
        output,
        vec![
            [
                255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255
            ]
            .repeat(3)
        ]
    );
}

#[test]
fn translated_multi_pipeline_late_shader_failure_preserves_depth() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleShader, TriangleState,
        TriangleTargetsPass,
    };
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin"),
    )
    .unwrap();
    let mut depth = gpu.create_depth_target([4, 3]).unwrap();
    let colors = [[0.0; 4]; 3];
    let clears = [CLEAR];
    for (step, z, load, write) in [
        (0, 0.5, DepthLoad::Clear(1.0), true),
        (1, 0.25, DepthLoad::Clear(0.0), true),
        (2, 0.375, DepthLoad::Load, false),
        (3, 0.75, DepthLoad::Load, false),
    ] {
        let vertices = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
        let draws = [TriangleDraw {
            vertices: &vertices,
            colors: &colors,
            viewport: [4, 3],
        }; 2];
        let states = [TriangleState {
            depth: DepthState {
                compare: if step == 1 {
                    DepthCompare::Always
                } else {
                    DepthCompare::Less
                },
                write,
            },
            ..Default::default()
        }; 2];
        let shaders = [
            TriangleShader {
                source: &shader.msl,
                entries: ENTRIES,
            },
            TriangleShader {
                source: "invalid shader",
                entries: ENTRIES,
            },
        ];
        let count = if step == 1 { 2 } else { 1 };
        let pass = TriangleTargetsPass {
            draws: &draws[..count],
            states: &states[..count],
            extent: [4, 3],
            clears: &clears,
            clear_depth: None,
        };
        let bindings = [None, None];
        let result = gpu.submit_multi_pipeline_triangle_targets_rgba8(
            &shaders[..count],
            &pass,
            &bindings[..count],
            Some((&mut depth, load)),
            &[None],
        );
        if step == 1 {
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("malformed second shader accepted"),
            };
            assert!(error.operation.contains("Library"), "{error}");
        } else {
            let output = result.unwrap().wait().unwrap();
            let expected = if step == 3 { CLEAR } else { [255, 0, 0, 255] };
            assert_eq!(output, vec![expected.repeat(12)], "step={step}");
        }
    }
}

#[test]
fn translated_multi_pipeline_validates_cardinality_and_allows_empty_pass() {
    use xenora_metal::{TriangleDraw, TriangleShader, TriangleState, TriangleTargetsPass};
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let states = [TriangleState::default()];
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let shaders = [TriangleShader {
        source: "invalid",
        entries: ENTRIES,
    }];
    for (selected, bindings) in [(&shaders[..], &[][..]), (&[][..], &[None][..])] {
        let result = gpu.submit_multi_pipeline_triangle_targets_rgba8(
            selected,
            &pass,
            bindings,
            None,
            &[None],
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("invalid count accepted"),
        };
        assert_eq!(error.operation, "render shaders");
    }
    let empty = TriangleTargetsPass {
        draws: &[],
        states: &[],
        ..pass
    };
    let output = gpu
        .submit_multi_pipeline_triangle_targets_rgba8(&[], &empty, &[], None, &[None])
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(output, vec![CLEAR.repeat(12)]);
    assert!(
        gpu.submit_loaded_triangle_targets_rgba8("", ENTRIES, &empty, None, None, &[None])
            .is_err()
    );
}

#[test]
fn translated_multi_pipeline_same_source_distinguishes_both_entry_names() {
    use xenora_metal::{TriangleDraw, TriangleShader, TriangleState, TriangleTargetsPass};
    // Original trusted MSL: exercise the native API's entry-name contract directly.
    // Both vertex and fragment names must independently participate in cache keys.
    const SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;
struct EntryVertex {
    float4 position [[position]];
    float4 color;
};
vertex EntryVertex entry_red_vertex(uint id [[vertex_id]],
                                   const device float4 *positions [[buffer(0)]]) {
    return {positions[id], float4(1, 0, 0, 1)};
}
vertex EntryVertex entry_green_vertex(uint id [[vertex_id]],
                                     const device float4 *positions [[buffer(0)]]) {
    return {positions[id], float4(0, 1, 0, 1)};
}
fragment float4 entry_red_fragment(EntryVertex input [[stage_in]]) {
    return float4(1, 0, 0, 1);
}
fragment float4 entry_green_fragment(EntryVertex input [[stage_in]]) {
    return float4(0, 1, 0, 1);
}
fragment float4 entry_varying_fragment(EntryVertex input [[stage_in]]) {
    return input.color;
}
";
    let gpu = context();
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }; 3];
    let states = [
        TriangleState::default(),
        TriangleState {
            scissor: Some([1, 0, 3, 3]),
            ..Default::default()
        },
        TriangleState {
            scissor: Some([3, 0, 1, 3]),
            ..Default::default()
        },
    ];
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    for (red, green) in [
        (
            ["entry_red_vertex", "entry_red_fragment"],
            ["entry_red_vertex", "entry_green_fragment"],
        ),
        (
            ["entry_red_vertex", "entry_varying_fragment"],
            ["entry_green_vertex", "entry_varying_fragment"],
        ),
    ] {
        let shaders = [
            TriangleShader {
                source: SOURCE,
                entries: red,
            },
            TriangleShader {
                source: SOURCE,
                entries: green,
            },
            TriangleShader {
                source: SOURCE,
                entries: red,
            },
        ];
        let output = gpu
            .submit_multi_pipeline_triangle_targets_rgba8(
                &shaders,
                &pass,
                &[None, None, None],
                None,
                &[None],
            )
            .unwrap()
            .wait()
            .unwrap();
        assert_eq!(
            output,
            vec![
                [
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255
                ]
                .repeat(3)
            ],
            "red entries={red:?}, green entries={green:?}"
        );
    }
}

#[test]
fn translated_conditional_vertex_paths_select_literal_half_screen_geometry() {
    let gpu = context();
    // Two triangles cover the input clip square exactly. Scaling x by .5 and
    // adding -.5 or +.5 covers exactly four columns of an 8x4 target.
    let vertices = [
        [-1.0, -1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
        [1.0, -1.0, 0.5, 1.0],
        [1.0, 1.0, 0.5, 1.0],
        [-1.0, 1.0, 0.5, 1.0],
    ];
    let colors = [[0.0; 4]; 6];
    for (vertex, right) in [
        (
            include_bytes!("../../../../fixtures/rdna2/x9-conditional-vs.bin").as_slice(),
            false,
        ),
        (
            include_bytes!("../../../../fixtures/rdna2/x9-conditional-right-vs.bin").as_slice(),
            true,
        ),
    ] {
        // Constant PS isolates geometry; conditional PS also checks composition.
        for (pixel, drawn) in [
            (
                include_bytes!("../../../../fixtures/rdna2/x9-constant-ps.bin").as_slice(),
                [255, 0, 0, 255],
            ),
            (
                include_bytes!("../../../../fixtures/rdna2/x9-conditional-ps.bin").as_slice(),
                [255, 0, 128, 255],
            ),
        ] {
            let shader = compile_graphics(Architecture::Gfx1030, vertex, pixel).unwrap();
            let output = gpu
                .render_triangles_rgba8(&shader.msl, ENTRIES, &vertices, &colors, [8, 4], CLEAR)
                .unwrap();
            assert_eq!(output.len(), 8 * 4 * 4);
            for (index, actual) in output.as_chunks::<4>().0.iter().enumerate() {
                let covered = (index % 8 >= 4) == right;
                assert_eq!(
                    *actual,
                    if covered { drawn } else { CLEAR },
                    "right={right}, drawn={drawn:?}, pixel={index}"
                );
            }
        }
    }
}

#[test]
fn translated_conditional_pixel_paths_select_literal_color() {
    let gpu = context();
    let shader = compile_graphics(
        Architecture::Gfx1030,
        include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin"),
        include_bytes!("../../../../fixtures/rdna2/x9-conditional-ps.bin"),
    )
    .unwrap();
    // Isolate PS behavior from conditional geometry. Signed/unsigned decisions
    // and the final literal comparison must give red=1, green=0, blue=.5.
    let output = gpu
        .render_rgba8(
            &shader.msl,
            ENTRIES,
            &POSITIONS,
            &[[0.0; 4]; 3],
            [8, 4],
            CLEAR,
        )
        .unwrap();
    assert_eq!(output, [255, 0, 128, 255].repeat(32));
}

// Original trusted MSL for independent texture/sampler-slot and clearing checks.
const TWO_SLOT_SOURCE: &str = r"
#include <metal_stdlib>
using namespace metal;
vertex float4 two_slot_vertex(uint id [[vertex_id]],
                              const device float4 *positions [[buffer(0)]]) {
    return positions[id];
}
fragment float4 two_slot_cross(texture2d<float> a [[texture(0)]],
                              texture2d<float> b [[texture(1)]],
                              sampler zero [[sampler(0)]], sampler one [[sampler(1)]]) {
    return float4(a.sample(one, float2(.5), level(0.0)).r,
                  b.sample(zero, float2(.5), level(0.0)).g,
                  a.sample(zero, float2(.5), level(0.0)).b, 1.0);
}
fragment float4 two_slot_nulls(texture2d<float> a [[texture(0)]],
                              texture2d<float> b [[texture(1)]]) {
    return float4(is_null_texture(a) ? 1.0 : 0.0,
                  is_null_texture(b) ? 1.0 : 0.0, 0.0, 1.0);
}
fragment float4 two_slot_red() { return float4(1, 0, 0, 1); }
";

fn two_slot_resources(
    gpu: &xenora_metal::NativeCompute,
) -> (
    [xenora_metal::MetalRgba8Texture; 2],
    [xenora_metal::MetalSampler; 2],
) {
    use xenora_metal::{MetalRgba8Texture, MetalRgba8TextureDesc, MetalSampler, Rgba8MipLevel};
    let textures = [
        ([17, 31, 47, 255], [71, 83, 97, 255]),
        ([109, 127, 139, 255], [151, 167, 181, 255]),
    ]
    .map(|(base, mip)| {
        MetalRgba8Texture::upload(
            gpu.device(),
            MetalRgba8TextureDesc::new(2, 2, 2).unwrap(),
            &[
                Rgba8MipLevel::new_packed(0, 2, 2, &base.repeat(4)),
                Rgba8MipLevel::new_packed(1, 1, 1, &mip),
            ],
        )
        .unwrap()
    });
    let samplers = [0, 1].map(|mip| {
        MetalSampler::new(
            gpu.device(),
            xenora_texture::SamplerDesc::nearest_clamp().with_explicit_mip(mip),
        )
        .unwrap()
    });
    (textures, samplers)
}

fn two_slot_pair(
    texture: &xenora_metal::MetalRgba8Texture,
    sampler: &xenora_metal::MetalSampler,
    t: u32,
    s: u32,
) -> xenora_metal::BoundSampledTexture {
    xenora_metal::BoundSampledTexture::bind(
        texture.clone(),
        sampler.clone(),
        xenora_metal::BindingSlot::new(t),
        xenora_metal::BindingSlot::new(s),
    )
    .unwrap()
}

#[test]
fn translated_two_slots_cross_pairs_rebind_lods_clear_and_retain_owners() {
    use xenora_metal::{TriangleDraw, TriangleShader, TriangleState, TriangleTargetsPass};
    let gpu = context();
    let (textures, samplers) = two_slot_resources(&gpu);
    let pairs = |swap: bool| {
        let (s0, s1) = if swap {
            (&samplers[1], &samplers[0])
        } else {
            (&samplers[0], &samplers[1])
        };
        // Repeated slots use clones of the same underlying object, including both
        // cross-pairs. Reordering the pairs must not change the slot tables.
        vec![
            two_slot_pair(&textures[1], s1, 1, 1),
            two_slot_pair(&textures[0], s1, 0, 1),
            two_slot_pair(&textures[1], s0, 1, 0),
            two_slot_pair(&textures[0], s0, 0, 0),
        ]
    };
    let bindings = [
        pairs(false),
        pairs(true),
        vec![],
        vec![two_slot_pair(&textures[1], &samplers[0], 1, 0)],
        pairs(false),
    ];
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [8, 3],
    }; 5];
    let states = [0, 2, 4, 6, 7].map(|x| TriangleState {
        scissor: Some([x, 0, 8 - x, 3]),
        ..Default::default()
    });
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [8, 3],
        clears: &clears,
        clear_depth: None,
    };
    let shaders = [
        "two_slot_cross",
        "two_slot_cross",
        "two_slot_nulls",
        "two_slot_nulls",
        "two_slot_cross",
    ]
    .map(|fragment| TriangleShader {
        source: TWO_SLOT_SOURCE,
        entries: ["two_slot_vertex", fragment],
    });
    let pending = gpu
        .submit_multi_binding_triangle_targets_rgba8(&shaders, &pass, &bindings, None, &[None])
        .unwrap();
    drop(bindings);
    drop(textures);
    drop(samplers);
    drop(gpu);
    let output = pending.wait().unwrap();
    let row = [
        [71, 127, 47, 255],
        [71, 127, 47, 255],
        [17, 167, 97, 255],
        [17, 167, 97, 255],
        [255, 255, 0, 255],
        [255, 255, 0, 255],
        [255, 0, 0, 255],
        [71, 127, 47, 255],
    ];
    assert_eq!(
        output,
        vec![row.repeat(3).into_iter().flatten().collect::<Vec<_>>()]
    );
}

#[test]
fn translated_two_slots_reject_conflicts_before_compile_and_preserve_depth() {
    use xenora_metal::{
        DepthCompare, DepthLoad, DepthState, TriangleDraw, TriangleShader, TriangleState,
        TriangleTargetsPass,
    };
    let gpu = context();
    let (textures, samplers) = two_slot_resources(&gpu);
    let valid = two_slot_pair(&textures[0], &samplers[0], 0, 0);
    let colors = [[0.0; 4]; 3];
    let draws = [TriangleDraw {
        vertices: &POSITIONS,
        colors: &colors,
        viewport: [4, 3],
    }];
    let states = [TriangleState {
        depth: DepthState {
            compare: DepthCompare::Less,
            write: true,
        },
        ..Default::default()
    }];
    let clears = [CLEAR];
    let pass = TriangleTargetsPass {
        draws: &draws,
        states: &states,
        extent: [4, 3],
        clears: &clears,
        clear_depth: None,
    };
    let red = TriangleShader {
        source: TWO_SLOT_SOURCE,
        entries: ["two_slot_vertex", "two_slot_red"],
    };
    let mut depth = gpu.create_depth_target([4, 3]).unwrap();
    let initial = gpu
        .submit_multi_binding_triangle_targets_rgba8(
            &[red],
            &pass,
            &[vec![]],
            Some((&mut depth, DepthLoad::Clear(1.0))),
            &[None],
        )
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(initial, vec![[255, 0, 0, 255].repeat(12)]); // retained depth is .5

    let conflicts = [
        vec![
            valid.clone(),
            two_slot_pair(&textures[1], &samplers[0], 0, 1),
        ], // texture conflict
        vec![
            valid.clone(),
            two_slot_pair(&textures[0], &samplers[1], 1, 0),
        ], // sampler conflict
        vec![two_slot_pair(&textures[0], &samplers[0], 4, 0)],
        vec![two_slot_pair(&textures[0], &samplers[0], 0, 4)],
        vec![valid.clone(); 17],
    ];
    for conflict in conflicts {
        let near = POSITIONS.map(|[x, y, _, w]| [x, y, 0.25, w]);
        let draws = [TriangleDraw {
            vertices: &near,
            colors: &colors,
            viewport: [4, 3],
        }; 2];
        let states = [TriangleState {
            depth: DepthState {
                compare: DepthCompare::Always,
                write: true,
            },
            ..Default::default()
        }; 2];
        let pass = TriangleTargetsPass {
            draws: &draws,
            states: &states,
            extent: [4, 3],
            clears: &clears,
            clear_depth: None,
        };
        let invalid = TriangleShader {
            source: "",
            entries: ["", ""],
        };
        let error = gpu
            .submit_multi_binding_triangle_targets_rgba8(
                &[red, invalid],
                &pass,
                &[vec![valid.clone()], conflict],
                Some((&mut depth, DepthLoad::Clear(0.0))),
                &[None],
            )
            .err()
            .expect("invalid binding accepted");
        assert_eq!(error.operation, "render bindings");
        // Read-only probes distinguish original .5 from either the requested
        // zero clear / .25 write, and from a spurious clear to one.
        for (z, expected) in [(0.375, [255, 0, 0, 255]), (0.75, CLEAR)] {
            let probe = POSITIONS.map(|[x, y, _, w]| [x, y, z, w]);
            let draws = [TriangleDraw {
                vertices: &probe,
                colors: &colors,
                viewport: [4, 3],
            }];
            let states = [TriangleState {
                depth: DepthState {
                    compare: DepthCompare::Less,
                    write: false,
                },
                ..Default::default()
            }];
            let pass = TriangleTargetsPass {
                draws: &draws,
                states: &states,
                extent: [4, 3],
                clears: &clears,
                clear_depth: None,
            };
            let output = gpu
                .submit_multi_binding_triangle_targets_rgba8(
                    &[red],
                    &pass,
                    &[vec![]],
                    Some((&mut depth, DepthLoad::Load)),
                    &[None],
                )
                .unwrap()
                .wait()
                .unwrap();
            assert_eq!(output, vec![expected.repeat(12)], "z={z}");
        }
    }
    // Existing Option-binding entry point must continue to reject nonzero slots.
    let bad_slot = two_slot_pair(&textures[0], &samplers[0], 1, 1);
    let result = gpu.submit_multi_pipeline_triangle_targets_rgba8(
        &[red],
        &pass,
        &[Some(bad_slot)],
        Some((&mut depth, DepthLoad::Load)),
        &[None],
    );
    assert!(result.is_err());
    drop(result);
    let one_mip = xenora_metal::MetalRgba8Texture::allocate(
        gpu.device(),
        xenora_metal::MetalRgba8TextureDesc::new(1, 1, 1).unwrap(),
    )
    .unwrap();
    assert!(
        xenora_metal::BoundSampledTexture::bind(
            one_mip,
            samplers[1].clone(),
            xenora_metal::BindingSlot::new(1),
            xenora_metal::BindingSlot::new(0)
        )
        .is_err()
    );
}
