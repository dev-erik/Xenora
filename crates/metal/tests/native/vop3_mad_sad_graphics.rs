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
type ArithmeticCase = (u32, u32, u32, [u32; 6], [u32; 6]);
const CASES: &[ArithmeticCase] = &[
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

// Independently transcribed from AMD RDNA2 section12.12 opcode entries
// 837 (XAD), 333 (LERP), and 369 (MSAD). Each answer is a fixed raw word;
// neither the compiler nor a reference interpreter computes expected pixels.
// XAD wraps after XOR+add. LERP uses only S2 bits 0/8/16/24. MSAD masks
// zero bytes in S1 (not S0), then adds the remaining unsigned byte distances.
const XAD_LERP_MSAD_CASES: &[(u32, [u32; 3], u32)] = &[
    (837, [0, 0, 0], 0),
    (837, [0xffffffff, 0, 1], 0),
    (837, [0x80000000, 0, 0x80000000], 0),
    (837, [0xaaaaaaaa, 0x55555555, 2], 1),
    (837, [0x12345678, 0x12345678, 0x89abcdef], 0x89abcdef),
    (837, [0xff00ff00, 0x00ff00ff, 0xfffffffe], 0xfffffffd),
    // Four odd sums: truncation, then independently selected round-up bytes.
    (333, [0, 0x01010101, 0], 0),
    (333, [0, 0x01010101, 0x01000100], 0x01000100),
    // A 9-bit intermediate must not overflow before division by two.
    (333, [0xffffffff, 0xffffffff, 0xffffffff], 0xffffffff),
    (333, [0x00ff00ff, 0x01000100, 0], 0x007f007f),
    (333, [0x00ff00ff, 0x01000100, 0x01010101], 0x01800180),
    // Dirty upper seven bits of every rounding byte have no effect.
    (333, [0x01030507, 0x02040608, 0xfefefefe], 0x01030507),
    (333, [0x01030507, 0x02040608, 0xffffffff], 0x02040608),
    (333, [0xff00fe01, 0x0001fffe, 0x01010001], 0x8001fe80),
    (369, [0xffffffff, 0, 0x12345678], 0x12345678),
    (369, [0, 0xffffffff, 0], 0x000003fc),
    // Included distances 255+1 carry through the accumulator and wrap to zero.
    (369, [0, 0x00ff0001, 0xffffff00], 0),
    (369, [0xff00ff00, 0x000100ff, 5], 0x00000105),
    (369, [0x01020304, 0x01000300, 7], 7),
    (369, [0, 0x80017fff, 0], 0x000001ff),
    (369, [0xffffffff, 0x01010101, 0xffffffff], 0x000003f7),
    (369, [0x000000ff, 0, 0], 0),
];

#[test]
fn xad_lerp_msad_literal_wrap_byte_rounding_and_reference_mask_pixels() {
    let gpu = NativeCompute::new().unwrap();
    assert_eq!(XAD_LERP_MSAD_CASES.len(), 22);
    for &(op, [a, b, c], expected) in XAD_LERP_MSAD_CASES {
        assert_eq!(
            render(
                &gpu,
                &pixel(op, 11, [264, 265, 266], None, false),
                [a, b, c, 0, 0, 0, 0, 0],
            ),
            vec![expected.to_le_bytes().to_vec()],
            "op={op} a={a:08x} b={b:08x} c={c:08x}",
        );
    }
}

#[test]
fn xad_lerp_msad_source_snapshots_scalar_literal_and_shared_literal_pixels() {
    let gpu = NativeCompute::new().unwrap();
    // Distinct source values make destination aliasing observable in every slot.
    for (op, inputs, expected) in [
        (837, [0xff00ff00, 0x00ff00ff, 0xfffffffe], 0xfffffffdu32),
        (333, [0xff00fe01, 0x0001fffe, 0x01010001], 0x8001fe80),
        (369, [0xff00ff00, 0x000100ff, 5], 0x00000105),
    ] {
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
                            false,
                        ),
                        [inputs[0], inputs[1], inputs[2], 0, 0, 0, 0, 0],
                    ),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={op} alias position={position} selector={selector}",
                );
            }
        }
        // With all operands equal to one: XAD=0+1; LERP low byte=(1+1+1)/2;
        // MSAD has zero distance and retains the accumulator. One shared word.
        assert_eq!(
            render(&gpu, &pixel(op, 11, [255; 3], Some(1), false), [0; 8]),
            vec![1u32.to_le_bytes().to_vec()],
            "op={op} shared literal",
        );
    }
}

// AMD RDNA2 section12.12 p185: op883 uses unsigned low16 multiplicands;
// op885 sign-extends each low16 multiplicand. Both add the full32 accumulator.
// Literal answers below retain the low32 result, with no input/output modifiers.
// Columns are inputs, unsigned result, signed result. Upper multiplicand bits
// deliberately differ from bit15; they cannot participate in either product.
const MAD16_GRAPHICS_CASES: &[([u32; 3], u32, u32)] = &[
    ([0xffff0000, 0xabcdffff, 0x89abcdef], 0x89abcdef, 0x89abcdef),
    ([0x0000ffff, 0x0000ffff, 0], 0xfffe0001, 1),
    ([0x00008000, 0x0000ffff, 0], 0x7fff8000, 0x00008000),
    ([0x00007fff, 2, 0xffff0002], 0, 0),
    ([0x00008000, 2, 0x7fffffff], 0x8000ffff, 0x7ffeffff),
    ([0x00008000, 0x00008000, 0xc0000000], 0, 0),
    ([0x0000ffff, 2, 1], 0x0001ffff, 0xffffffff),
    ([0xabcdffff, 0x12340002, 0xffffffff], 0x0001fffd, 0xfffffffd),
    ([0x00007fff, 0x00007fff, 0], 0x3fff0001, 0x3fff0001),
    ([0x0000ffff, 0x0000ffff, 0x0001ffff], 0, 0x00020000),
    ([0x00008000, 1, 0], 0x00008000, 0xffff8000),
    ([0xffff0000, 0xffff0001, 0x80000000], 0x80000000, 0x80000000),
];

#[test]
fn mad16_low_half_signed_boundaries_dirty_upper_bits_and_wrap_pixels() {
    let gpu = NativeCompute::new().unwrap();
    assert_eq!(MAD16_GRAPHICS_CASES.len(), 12);
    for &(inputs, unsigned, signed) in MAD16_GRAPHICS_CASES {
        for (op, expected) in [(883, unsigned), (885, signed)] {
            assert_eq!(
                render(
                    &gpu,
                    &pixel(op, 11, [264, 265, 266], None, false),
                    [inputs[0], inputs[1], inputs[2], 0, 0, 0, 0, 0],
                ),
                vec![expected.to_le_bytes().to_vec()],
                "op={op} inputs={inputs:08x?}",
            );
        }
    }
}

#[test]
fn mad16_source_aliases_scalar_legal_literals_and_shared_literal_pixels() {
    let gpu = NativeCompute::new().unwrap();
    // (-1)*(-32768)-1 = 32767; unsigned 65535*32768-1 = 0x7fff7fff.
    // Both multiplicand literals fit16; the SRC2 literal retains all32 bits.
    let inputs = [0x0000ffff, 0x00008000, 0xffffffff];
    for (op, expected) in [(883, 0x7fff7fffu32), (885, 0x00007fff)] {
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
                            false,
                        ),
                        [inputs[0], inputs[1], inputs[2], 0, 0, 0, 0, 0],
                    ),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={op} alias position={position} selector={selector}",
                );
            }
        }
        for (literal, unsigned, signed) in [
            (1, 2u32, 2u32),
            (0x8000, 0x40008000, 0x40008000),
            (0xffff, 0xffff0000, 0x00010000),
        ] {
            let answer = if op == 883 { unsigned } else { signed };
            assert_eq!(
                render(&gpu, &pixel(op, 11, [255; 3], Some(literal), false), [0; 8],),
                vec![answer.to_le_bytes().to_vec()],
                "op={op} shared literal={literal:08x}",
            );
        }
    }
}

// Table 20 and p185: columns are unsigned/signed HALF times one,
// FP32 raw addend plus one, and unsigned/signed HALF squared plus FP32.
// These fixed answers do not call either evaluator or a production table.
const FLOAT_ANSWERS: &[(u32, u32, u32, u32, u32)] = &[
    (0x00003800, 0x00003800, 0x3f000001, 0x4b400000, 0x4b400000),
    (0x0000b800, 0xffffb800, 0xbf000001, 0x43400000, 0xd3400000),
    (0x00003c00, 0x00003c00, 0x3f800001, 0x4d900000, 0x4d900000),
    (0x0000bc00, 0xffffbc00, 0xbf800001, 0x49900000, 0xd1900000),
    (0x00004000, 0x00004000, 0x40000001, 0x50000000, 0x50000000),
    (0x0000c000, 0xffffc000, 0xc0000001, 0x50000000, 0xd0000000),
    (0x00004400, 0x00004400, 0x40800001, 0x52900000, 0x52900000),
    (0x0000c400, 0xffffc400, 0xc0800001, 0x56900000, 0xce900000),
    (0x00003118, 0x00003118, 0x3e22f984, 0x478d2bc3, 0x478d2bc3),
];
#[test]
fn mad16_named_float_sources_use_half_and_full_width_pixels() {
    let gpu = NativeCompute::new().unwrap();
    let mut count = 0;
    for (index, &(u, s, add, mixed_u, mixed_s)) in FLOAT_ANSWERS.iter().enumerate() {
        let selector = 240 + index as u32;
        for op in [883, 885] {
            for position in 0..4 {
                let mut src = [129, 129, 128];
                let expected = if position == 3 {
                    src = [selector; 3];
                    if op == 883 { mixed_u } else { mixed_s }
                } else {
                    src[position] = selector;
                    if position == 2 {
                        add
                    } else if op == 883 {
                        u
                    } else {
                        s
                    }
                };
                assert_eq!(
                    render(&gpu, &pixel(op, 11, src, None, false), [0; 8]),
                    vec![expected.to_le_bytes().to_vec()],
                    "op={op} selector={selector} position={position}",
                );
                count += 1;
            }
        }
    }
    assert_eq!(count, 72);
}

#[test]
fn sub_nc_i32_boundaries_sources_and_aliases() {
    let gpu = NativeCompute::new().unwrap();
    for clamp in [false, true] {
        for (a, b, wrapped, saturated) in [
            (0u32, 1u32, 0xffffffffu32, 0xffffffffu32),
            (0x80000000, 1, 0x7fffffff, 0x80000000),
            (0x7fffffff, 0xffffffff, 0x80000000, 0x7fffffff),
            (0xffffffff, 0x80000000, 0x7fffffff, 0x7fffffff),
            (0x80000000, 0x7fffffff, 1, 0x80000000),
            (0x7fa12345, 0x12345678, 0x6d6ccccd, 0x6d6ccccd),
            (0xffffffff, 0xffffffff, 0, 0),
        ] {
            let want = if clamp { saturated } else { wrapped };
            for dst in [8, 9, 11] {
                for (src, lit) in [
                    ([264, 265, 128], None),
                    ([24, 265, 128], None),
                    ([264, 25, 128], None),
                    ([255, 265, 128], Some(a)),
                    ([264, 255, 128], Some(b)),
                ] {
                    assert_eq!(
                        render(
                            &gpu,
                            &pixel(886, dst, src, lit, clamp),
                            [a, b, 0, 0, 0, 0, 0, 0]
                        ),
                        vec![want.to_le_bytes().to_vec()]
                    );
                }
            }
        }
        for (src, lit, want) in [
            ([255, 255, 128], Some(0x80000000), 0u32),
            ([240, 129, 128], None, 0x3effffff),
            ([129, 240, 128], None, 0xc1000001),
            ([193, 129, 128], None, 0xfffffffe),
        ] {
            assert_eq!(
                render(&gpu, &pixel(886, 11, src, lit, clamp), [0; 8]),
                vec![want.to_le_bytes().to_vec()]
            );
        }
    }
}

#[test]
fn add_nc_i32_boundaries_sources_and_aliases() {
    let gpu = NativeCompute::new().unwrap();
    for clamp in [false, true] {
        for (a, b, wrapped, saturated) in [
            (0u32, 1u32, 1u32, 1u32),
            (0x7fffffff, 1, 0x80000000, 0x7fffffff),
            (0x80000000, 0xffffffff, 0x7fffffff, 0x80000000),
            (0x80000000, 0x80000000, 0, 0x80000000),
            (0x7fffffff, 0x80000000, 0xffffffff, 0xffffffff),
            (0xffffffff, 0xffffffff, 0xfffffffe, 0xfffffffe),
            (0x7fffffff, 0x7fffffff, 0xfffffffe, 0x7fffffff),
        ] {
            let want = if clamp { saturated } else { wrapped };
            for dst in [8, 9, 11] {
                for (src, lit) in [
                    ([264, 265, 128], None),
                    ([24, 265, 128], None),
                    ([264, 25, 128], None),
                    ([255, 265, 128], Some(a)),
                    ([264, 255, 128], Some(b)),
                ] {
                    assert_eq!(
                        render(
                            &gpu,
                            &pixel(895, dst, src, lit, clamp),
                            [a, b, 0, 0, 0, 0, 0, 0]
                        ),
                        vec![want.to_le_bytes().to_vec()]
                    );
                }
            }
        }
        for (src, lit, want) in [
            (
                [255, 255, 128],
                Some(0x80000000),
                if clamp { 0x80000000u32 } else { 0u32 },
            ),
            ([240, 129, 128], None, 0x3f000001),
            ([129, 240, 128], None, 0x3f000001),
            ([193, 129, 128], None, 0),
        ] {
            assert_eq!(
                render(&gpu, &pixel(895, 11, src, lit, clamp), [0; 8]),
                vec![want.to_le_bytes().to_vec()]
            );
        }
    }
}

#[test]
fn perm_b32_selectors_sources_and_aliases() {
    let gpu = NativeCompute::new().unwrap();
    // AMD RDNA2 p181, fixed words for byte ordering, sign expansion and constants.
    for (control, want) in [
        (0x03020100u32, 0x807f81ffu32),
        (0x07060504, 0x00fe7e01),
        (0x00010407, 0xff810100),
        (0x0b0a0908, 0x0000ffff),
        (0xff800d0c, 0xffffff00),
    ] {
        for dst in [8, 9, 10, 11] {
            for (sources, literal) in [
                ([264, 265, 266], None),
                ([24, 265, 266], None),
                ([264, 25, 266], None),
                ([264, 265, 26], None),
                ([255, 265, 266], Some(0x00fe7e01)),
                ([264, 255, 266], Some(0x807f81ff)),
                ([264, 265, 255], Some(control)),
            ] {
                assert_eq!(
                    render(
                        &gpu,
                        &pixel(836, dst, sources, literal, false),
                        [0x00fe7e01, 0x807f81ff, control, 0, 0, 0, 0, 0]
                    ),
                    vec![want.to_le_bytes().to_vec()]
                );
            }
        }
    }
}

#[test]
fn perm_b32_each_sign_selector_is_independent() {
    let gpu = NativeCompute::new().unwrap();
    for (a, b, want) in [
        (0u32, 0x00008000u32, 0x000000ffu32),
        (0, 0x80000000, 0x0000ff00),
        (0x00008000, 0, 0x00ff0000),
        (0x80000000, 0, 0xff000000),
    ] {
        assert_eq!(
            render(
                &gpu,
                &pixel(836, 11, [264, 265, 266], None, false),
                [a, b, 0x0b0a0908, 0, 0, 0, 0, 0]
            ),
            vec![want.to_le_bytes().to_vec()]
        );
    }
}

// Fixed source-width expectations; unselected operands are 2,3,5.
const MAD16_WIDE_GRAPHICS_ANSWERS: &[(u32, [u32; 7], [u32; 7])] = &[
    (
        0xffff8000,
        [
            0x00018005, 0x00010005, 0x40000005, 0xffff8006, 0x00010000, 0x00008000, 0x3fff8000,
        ],
        [
            0xfffe8005, 0xffff0005, 0x40000005, 0xffff8006, 0xfffe0000, 0xfffe8000, 0x3fff8000,
        ],
    ),
    (
        0xffffffff,
        [
            0x00030002, 0x00020003, 0xfffe0006, 0x00000005, 0x0002fffc, 0x0001fffd, 0xfffe0000,
        ],
        [
            0x00000002, 0x00000003, 0x00000006, 0x00000005, 0xfffffffc, 0xfffffffd, 0x00000000,
        ],
    ),
    (
        0x12345678,
        [
            0x0001036d, 0x0000acf5, 0x1d34d845, 0x1234567e, 0x123559e0, 0x12350368, 0x2f692eb8,
        ],
        [
            0x0001036d, 0x0000acf5, 0x1d34d845, 0x1234567e, 0x123559e0, 0x12350368, 0x2f692eb8,
        ],
    ),
    (
        0x80000000,
        [
            0x00000005, 0x00000005, 0x00000005, 0x80000006, 0x80000000, 0x80000000, 0x80000000,
        ],
        [
            0x00000005, 0x00000005, 0x00000005, 0x80000006, 0x80000000, 0x80000000, 0x80000000,
        ],
    ),
    (
        0x00010000,
        [
            0x00000005, 0x00000005, 0x00000005, 0x00010006, 0x00010000, 0x00010000, 0x00010000,
        ],
        [
            0x00000005, 0x00000005, 0x00000005, 0x00010006, 0x00010000, 0x00010000, 0x00010000,
        ],
    ),
];

#[test]
fn mad16_full_word_shared_literals_and_aliases() {
    let gpu = NativeCompute::new().unwrap();
    let mut count = 0;
    for &(literal, unsigned, signed) in MAD16_WIDE_GRAPHICS_ANSWERS {
        for (op, answers) in [(883, unsigned), (885, signed)] {
            for mask in 1..8 {
                let mut sources = [264, 265, 266];
                for (i, source) in sources.iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        *source = 255;
                    }
                }
                for dst in [8, 9, 10, 11] {
                    assert_eq!(
                        render(
                            &gpu,
                            &pixel(op, dst, sources, Some(literal), false),
                            [2, 3, 5, 0, 0, 0, 0, 0]
                        ),
                        vec![answers[mask - 1].to_le_bytes().to_vec()]
                    );
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 280);
}
