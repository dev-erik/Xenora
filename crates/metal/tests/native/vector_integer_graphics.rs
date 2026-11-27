// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal R32Uint graphics answers; no production/reference evaluator.
//! Public AMD RDNA 2 ISA guide 12.7 and 13.3: signed/unsigned 24-bit products,
//! integer min/max, bitwise operations, NC add/sub and reverse shifts.
//! https://docs.amd.com/v/u/en-US/rdna2-shader-instruction-set-architecture
//! Reverse shifts use S0[4:0] as count; SUBREV subtracts S0 from S1.
use xenora_metal::*;
use xenora_shader::{
    Architecture, GraphicsExportType, GraphicsPipelineTypes, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT,
    compile_graphics_pipeline_typed,
};
use xenora_texture::TextureFormat;

const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const COLORS: [[f32; 4]; 3] = [[0.; 4]; 3];

#[derive(Clone, Copy, Debug)]
enum Source {
    Vector,
    Scalar,
    Literal,
    Inline(u32),
}

// Tuple: VOP2 opcode, S0, S1, exact expected word. No calculated arithmetic oracle.
const CASES: &[(u32, u32, u32, u32)] = &[
    // Signed low/high 24-bit products; bit 23 sign-extends, bits 31:24 do not.
    (9, 0, 0xffffffff, 0),
    (9, 0x00800000, 0x00800000, 0),
    (9, 0x00ffffff, 2, 0xfffffffe),
    (9, 0x00800000, 0x007fffff, 0x00800000),
    (9, 0xff000003, 0x01000005, 15),
    (9, 0x01000000, 0xffffffff, 0),
    (10, 0, 0xffffffff, 0),
    (10, 0x00800000, 0x00800000, 0x00004000),
    (10, 0x00ffffff, 2, 0xffffffff),
    (10, 0x00800000, 0x007fffff, 0xffffc000),
    (10, 0x00ffffff, 0x00ffffff, 0),
    (10, 0xff000003, 0x01000005, 0),
    // Unsigned high product keeps bits 63:32, not bits 47:16.
    (11, 0, 0xffffffff, 0),
    (11, 0x00ffffff, 0x00ffffff, 0xfe000001),
    (11, 0xff000003, 0x01000005, 15),
    (11, 0x00800000, 2, 0x01000000),
    (11, 0x01000000, 0xffffffff, 0),
    (12, 0, 0xffffffff, 0),
    (12, 0x00ffffff, 0x00ffffff, 0x0000ffff),
    (12, 0x00800000, 0x00800000, 0x00004000),
    (12, 0xff000003, 0x01000005, 0),
    // Signed/unsigned extrema, both orders, ties, and NaN-shaped payloads.
    (17, 0x80000000, 0x7fffffff, 0x80000000),
    (17, 0x7fffffff, 0x80000000, 0x80000000),
    (17, 0xffffffff, 0, 0xffffffff),
    (17, 0x7fa12345, 0x7fffffff, 0x7fa12345),
    (17, 0x80000000, 0x80000000, 0x80000000),
    (18, 0x80000000, 0x7fffffff, 0x7fffffff),
    (18, 0x7fffffff, 0x80000000, 0x7fffffff),
    (18, 0xffffffff, 0, 0),
    (18, 0x7fa12345, 0, 0x7fa12345),
    (18, 0xffffffff, 0xffffffff, 0xffffffff),
    (19, 0x80000000, 0x7fffffff, 0x7fffffff),
    (19, 0x7fffffff, 0x80000000, 0x7fffffff),
    (19, 0xffffffff, 0, 0),
    (19, 0x7fa12345, 0xffffffff, 0x7fa12345),
    (20, 0x80000000, 0x7fffffff, 0x80000000),
    (20, 0x7fffffff, 0x80000000, 0x80000000),
    (20, 0xffffffff, 0, 0xffffffff),
    (20, 0x7fa12345, 0, 0x7fa12345),
    // All three reverse shifts: count is first, data second; mask to five bits.
    (22, 0, 0x80000001, 0x80000001),
    (22, 31, 0x80000001, 1),
    (22, 32, 0x80000001, 0x80000001),
    (22, 33, 0x80000001, 0x40000000),
    (22, 1, 0xffffffff, 0x7fffffff),
    (22, 63, 0x80000001, 1),
    (22, 0xffffffff, 0x80000001, 1),
    (24, 0, 0x80000001, 0x80000001),
    (24, 31, 0x80000001, 0xffffffff),
    (24, 32, 0x80000001, 0x80000001),
    (24, 33, 0x80000001, 0xc0000000),
    (24, 31, 0x7fffffff, 0),
    (24, 63, 0x80000001, 0xffffffff),
    (24, 0xffffffff, 0x80000001, 0xffffffff),
    (26, 0, 0x7fa12345, 0x7fa12345),
    (26, 31, 1, 0x80000000),
    (26, 32, 0x7fa12345, 0x7fa12345),
    (26, 33, 0x80000001, 2),
    (26, 1, 0xffffffff, 0xfffffffe),
    (26, 63, 1, 0x80000000),
    (26, 0xffffffff, 1, 0x80000000),
    // AND/OR/XOR and registry XNOR, including signaling-NaN-shaped words.
    (27, 0, 0xffffffff, 0),
    (27, 0xffffffff, 0x7fa12345, 0x7fa12345),
    (27, 0xaaaaaaaa, 0x55555555, 0),
    (28, 0, 0x7fa12345, 0x7fa12345),
    (28, 0x80000000, 0x7fffffff, 0xffffffff),
    (28, 0, 0, 0),
    (29, 0, 0x7fa12345, 0x7fa12345),
    (29, 0xffffffff, 0x80000000, 0x7fffffff),
    (29, 0xffffffff, 0xffffffff, 0),
    (30, 0, 0, 0xffffffff),
    (30, 0xffffffff, 0, 0),
    (30, 0x805edcba, 0, 0x7fa12345),
    (30, 0x0f0f0f0f, 0x00ff00ff, 0xf00ff00f),
    // Wrapping arithmetic, opposite operand orders and sign boundaries.
    (37, 0, 0, 0),
    (37, 0x00000001, 0, 0x00000001),
    (37, 0x80000000, 0, 0x80000000),
    (37, 0x7f800000, 0, 0x7f800000),
    (37, 0x7fc12345, 0, 0x7fc12345),
    (37, 0x7fa12345, 0, 0x7fa12345),
    (37, 0xffffffff, 0, 0xffffffff),
    (37, 0xffffffff, 1, 0),
    (37, 0x7fffffff, 1, 0x80000000),
    (37, 0x80000000, 0x80000000, 0),
    (37, 0x7fa12344, 1, 0x7fa12345),
    (38, 5, 3, 2),
    (38, 3, 5, 0xfffffffe),
    (38, 0, 0xffffffff, 1),
    (38, 0x80000000, 1, 0x7fffffff),
    (38, 0x7fa12346, 1, 0x7fa12345),
    (39, 5, 3, 0xfffffffe),
    (39, 3, 5, 2),
    (39, 0xffffffff, 0, 1),
    (39, 1, 0x80000000, 0x7fffffff),
    (39, 1, 0x7fa12346, 0x7fa12345),
];

fn pixel(opcode: u32, dst: u32, source: Source, a: u32, e64: bool) -> Vec<u8> {
    // Materialize raw uint uniforms through the integer scalar carrier. Direct
    // V_MOV s24/s25 creates UniformFloat array-layer metadata and must not be
    // laundered into integer data. Both reads remain reachable for every mode.
    let mut words = vec![0x7e180280, 0x4a101818, 0x4a121819];
    // v12=0; ADD_NC v8,s24,v12; ADD_NC v9,s25,v12.
    if e64 {
        assert!(matches!(source, Source::Vector));
        assert_eq!(opcode, 37);
        // VOP3 ADD_NC: no modifiers, VGPR sources, unused source is inline zero.
        words.extend([0xd5250000 | dst, 128 << 18 | 265 << 9 | 264]);
    } else {
        let src0 = match source {
            Source::Vector => 264,
            Source::Scalar => 24,
            Source::Literal => 255,
            Source::Inline(selector) => selector,
        };
        words.push(opcode << 25 | dst << 17 | 9 << 9 | src0);
        if matches!(source, Source::Literal) {
            words.push(a);
        }
    }
    // Repeat the destination in all four export components; only R is stored.
    words.extend([0xf800180f, dst * 0x01010101, 0xbf810000]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn render(gpu: &NativeCompute, ps: &[u8], a: u32, b: u32) -> Vec<Vec<u8>> {
    let mut types = GraphicsPipelineTypes::default();
    types.colors[0] = GraphicsExportType::Uint;
    let pipeline = compile_graphics_pipeline_typed(Architecture::Gfx1030, VS, ps, types).unwrap();
    assert_eq!(pipeline.required_uniform_mask, 3);
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

#[test]
fn translated_integer_vectors_export_literal_uint_results_with_both_aliases() {
    let gpu = NativeCompute::new().unwrap();
    for &(opcode, a, b, expected) in CASES {
        for dst in [8, 9, 10] {
            let sources = &[Source::Vector, Source::Scalar, Source::Literal];
            for &source in sources {
                let ps = pixel(opcode, dst, source, a, false);
                assert_eq!(
                    render(&gpu, &ps, a, b),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={opcode} dst={dst} source={source:?} a={a:#010x} b={b:#010x}"
                );
            }
            // The same independently specified add answers cover the e64 form.
            if opcode == 37 {
                let ps = pixel(opcode, dst, Source::Vector, a, true);
                assert_eq!(
                    render(&gpu, &ps, a, b),
                    vec![expected.to_le_bytes().to_vec()],
                    "VOP3 add dst={dst} a={a:#010x} b={b:#010x}"
                );
            }
        }
    }
}

#[test]
fn translated_integer_vectors_accept_inline_raw_words_without_float_conversion() {
    let gpu = NativeCompute::new().unwrap();
    // Opcode, selector, S1, literal result. A is deliberately unrelated so a
    // decoder/lifter incorrectly using v8 instead of the inline source fails.
    let cases: &[(u32, u32, u32, u32)] = &[
        (22, 161, 0x80000001, 0x40000000),
        (24, 193, 0x80000001, 0xffffffff),
        (26, 161, 0x80000001, 2),
        (22, 248, 0x80000001, 0x10000000),
        (24, 248, 0x80000001, 0xf0000000),
        (26, 248, 0x80000001, 8),
        (27, 242, 0xffffffff, 0x3f800000),
        (28, 241, 0x007fa123, 0xbf7fa123),
        (29, 248, 0x3e22f983, 0),
        (37, 128, 0x7fa12345, 0x7fa12345),
        (37, 129, 0xffffffff, 0),
        (38, 193, 1, 0xfffffffe),
        (39, 129, 3, 2),
        (39, 248, 0x3e22f984, 1),
        (37, 241, 0, 0xbf000000),
        (30, 242, 0, 0xc07fffff),
        (9, 193, 2, 0xfffffffe),
        (10, 193, 2, 0xffffffff),
        (11, 193, 2, 0x01fffffe),
        (12, 193, 0x00ffffff, 0x0000ffff),
        (17, 241, 0, 0xbf000000),
        (18, 241, 0, 0),
        (19, 241, 0, 0),
        (20, 241, 0, 0xbf000000),
    ];
    for &(opcode, selector, b, expected) in cases {
        for dst in [8, 9, 10] {
            let ps = pixel(opcode, dst, Source::Inline(selector), 0x12345678, false);
            assert_eq!(
                render(&gpu, &ps, 0x12345678, b),
                vec![expected.to_le_bytes().to_vec()],
                "inline op={opcode} selector={selector} dst={dst} b={b:#010x}"
            );
        }
    }
}

#[test]
fn bounded_countdown_preserves_integer_vgpr_writes_and_nan_payloads() {
    let gpu = NativeCompute::new().unwrap();
    // s24=count, s25=initial word. The guest clamps count to four, then each
    // backedge carries v8 += 1. Structural projection must see that v8 write.
    // Only v12 is initialized with a finite V_MOV; arbitrary bits use ADD_NC.
    let ps: Vec<u8> = [
        0x83a08418u32, // s_min_u32 s32,s24,4
        0x7e180280,    // v_mov_b32 v12,0
        0x4a101819,    // v_add_nc_u32 v8,s25,v12
        0xbf078020,    // header: s_cmp_lg_u32 s32,0
        0xbf840003,    // s_cbranch_scc0 exit (word 8)
        0x4a101081,    // v_add_nc_u32 v8,1,v8
        0x80a08120,    // s_sub_u32 s32,s32,1
        0xbf82fffb,    // s_branch header (word 3)
        0xf800180f,    // exit: exp mrt0,v8,v8,v8,v8 done vm
        0x08080808,
        0xbf810000,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();
    // Literal outcomes include zero-trip, all admitted trip counts, and clamping
    // a huge unsigned count. Payload bits must survive every phi/edge copy.
    for (seed, answers) in [
        (
            0x7fa12340u32,
            [
                0x7fa12340u32,
                0x7fa12341,
                0x7fa12342,
                0x7fa12343,
                0x7fa12344,
            ],
        ),
        (0xfffffffe, [0xfffffffe, 0xffffffff, 0, 1, 2]),
    ] {
        for (count, index) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 4),
            (0xffffffff, 4),
        ] {
            assert_eq!(
                render(&gpu, &ps, count, seed),
                vec![answers[index].to_le_bytes().to_vec()],
                "integer countdown count={count} seed={seed:#010x}"
            );
        }
    }
}

#[test]
fn runtime_branch_join_preserves_integer_payload_before_further_arithmetic() {
    let gpu = NativeCompute::new().unwrap();
    // s24 chooses a branch; s25 supplies a raw word. Both sides overwrite v8,
    // and a post-join add into distinct v10 forces the selected phi to be used.
    let ps: Vec<u8> = [
        0x7e180280u32, // v12=0
        0x4a101819,    // v8=s25+v12
        0xbf078018,    // s_cmp_lg_u32 s24,0
        0xbf840002,    // s_cbranch_scc0 else (word 6)
        0x4a101081,    // v8=1+v8
        0xbf820001,    // s_branch join (word 7)
        0x4e101081,    // else: v_subrev_nc_u32 v8,1,v8 -> v8-1
        0x4a141082,    // join: v10=2+v8
        0xf800180f,
        0x0a0a0a0a,
        0xbf810000,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();
    for (seed, choice, expected) in [
        (0x7fa12345u32, 0, 0x7fa12346u32),
        (0x7fa12345, 1, 0x7fa12348),
        (0x7fa12345, 0xffffffff, 0x7fa12348),
        (0xffffffff, 0, 0),
        (0xffffffff, 1, 2),
        (0x80000000, 0, 0x80000001),
        (0x80000000, 1, 0x80000003),
    ] {
        assert_eq!(
            render(&gpu, &ps, choice, seed),
            vec![expected.to_le_bytes().to_vec()],
            "integer join choice={choice} seed={seed:#010x}"
        );
    }
}
