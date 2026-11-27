// SPDX-License-Identifier: GPL-3.0-or-later
//! Original graphics SOP1 known-answer cases. Expected words and SCC are literals;
//! no production evaluator, reference interpreter or generated semantic table is an oracle.
//! AMD RDNA 2 guide 12.3: NOT/BCNT0/BCNT1/ABS set SCC to result-nonzero;
//! BREV/FF0/FF1/FLBIT/SEXT preserve SCC. SOP1 fields follow section 13.1.3.
use xenora_metal::*;
use xenora_shader::{
    Architecture, PIXEL_ENTRY_POINT, VERTEX_ENTRY_POINT, compile_graphics_pipeline,
};

const ENTRIES: [&str; 2] = [VERTEX_ENTRY_POINT, PIXEL_ENTRY_POINT];
const VS: &[u8] = include_bytes!("../../../../fixtures/rdna2/x9-constant-vs.bin");
const POS: [[f32; 4]; 3] = [[-1., -1., 0.5, 1.], [3., -1., 0.5, 1.], [-1., 3., 0.5, 1.]];
const COLORS: [[f32; 4]; 3] = [[0.; 4]; 3];

#[test]
fn translated_scalar_unary_matches_literal_results_flags_and_aliases() {
    let gpu = NativeCompute::new().unwrap();
    // (SOP1 opcode, input bits, result bits, replacement SCC or preserved SCC).
    // Every form covers zero, all ones, bit 31, byte/word boundaries and bit 16.
    let cases: &[(u32, u32, u32, Option<bool>)] = &[
        // NOT.
        (7, 0x00000000, 0xffffffff, Some(true)),
        (7, 0xffffffff, 0x00000000, Some(false)),
        (7, 0x80000000, 0x7fffffff, Some(true)),
        (7, 0x0000007f, 0xffffff80, Some(true)),
        (7, 0x00000080, 0xffffff7f, Some(true)),
        (7, 0x000000ff, 0xffffff00, Some(true)),
        (7, 0x00007fff, 0xffff8000, Some(true)),
        (7, 0x00008000, 0xffff7fff, Some(true)),
        (7, 0x0000ffff, 0xffff0000, Some(true)),
        (7, 0x00010000, 0xfffeffff, Some(true)),
        // BREV.
        (11, 0x00000000, 0x00000000, None),
        (11, 0xffffffff, 0xffffffff, None),
        (11, 0x80000000, 0x00000001, None),
        (11, 0x0000007f, 0xfe000000, None),
        (11, 0x00000080, 0x01000000, None),
        (11, 0x000000ff, 0xff000000, None),
        (11, 0x00007fff, 0xfffe0000, None),
        (11, 0x00008000, 0x00010000, None),
        (11, 0x0000ffff, 0xffff0000, None),
        (11, 0x00010000, 0x00008000, None),
        // BCNT0.
        (13, 0x00000000, 0x00000020, Some(true)),
        (13, 0xffffffff, 0x00000000, Some(false)),
        (13, 0x80000000, 0x0000001f, Some(true)),
        (13, 0x0000007f, 0x00000019, Some(true)),
        (13, 0x00000080, 0x0000001f, Some(true)),
        (13, 0x000000ff, 0x00000018, Some(true)),
        (13, 0x00007fff, 0x00000011, Some(true)),
        (13, 0x00008000, 0x0000001f, Some(true)),
        (13, 0x0000ffff, 0x00000010, Some(true)),
        (13, 0x00010000, 0x0000001f, Some(true)),
        // BCNT1.
        (15, 0x00000000, 0x00000000, Some(false)),
        (15, 0xffffffff, 0x00000020, Some(true)),
        (15, 0x80000000, 0x00000001, Some(true)),
        (15, 0x0000007f, 0x00000007, Some(true)),
        (15, 0x00000080, 0x00000001, Some(true)),
        (15, 0x000000ff, 0x00000008, Some(true)),
        (15, 0x00007fff, 0x0000000f, Some(true)),
        (15, 0x00008000, 0x00000001, Some(true)),
        (15, 0x0000ffff, 0x00000010, Some(true)),
        (15, 0x00010000, 0x00000001, Some(true)),
        // FF0.
        (17, 0x00000000, 0x00000000, None),
        (17, 0xffffffff, 0xffffffff, None),
        (17, 0x80000000, 0x00000000, None),
        (17, 0x0000007f, 0x00000007, None),
        (17, 0x00000080, 0x00000000, None),
        (17, 0x000000ff, 0x00000008, None),
        (17, 0x00007fff, 0x0000000f, None),
        (17, 0x00008000, 0x00000000, None),
        (17, 0x0000ffff, 0x00000010, None),
        (17, 0x00010000, 0x00000000, None),
        // FF1.
        (19, 0x00000000, 0xffffffff, None),
        (19, 0xffffffff, 0x00000000, None),
        (19, 0x80000000, 0x0000001f, None),
        (19, 0x0000007f, 0x00000000, None),
        (19, 0x00000080, 0x00000007, None),
        (19, 0x000000ff, 0x00000000, None),
        (19, 0x00007fff, 0x00000000, None),
        (19, 0x00008000, 0x0000000f, None),
        (19, 0x0000ffff, 0x00000000, None),
        (19, 0x00010000, 0x00000010, None),
        // FLBIT_B32.
        (21, 0x00000000, 0xffffffff, None),
        (21, 0xffffffff, 0x00000000, None),
        (21, 0x80000000, 0x00000000, None),
        (21, 0x0000007f, 0x00000019, None),
        (21, 0x00000080, 0x00000018, None),
        (21, 0x000000ff, 0x00000018, None),
        (21, 0x00007fff, 0x00000011, None),
        (21, 0x00008000, 0x00000010, None),
        (21, 0x0000ffff, 0x00000010, None),
        (21, 0x00010000, 0x0000000f, None),
        // FLBIT_I32.
        (23, 0x00000000, 0xffffffff, None),
        (23, 0xffffffff, 0xffffffff, None),
        (23, 0x80000000, 0x00000001, None),
        (23, 0x0000007f, 0x00000019, None),
        (23, 0x00000080, 0x00000018, None),
        (23, 0x000000ff, 0x00000018, None),
        (23, 0x00007fff, 0x00000011, None),
        (23, 0x00008000, 0x00000010, None),
        (23, 0x0000ffff, 0x00000010, None),
        (23, 0x00010000, 0x0000000f, None),
        // SEXT_I8.
        (25, 0x00000000, 0x00000000, None),
        (25, 0xffffffff, 0xffffffff, None),
        (25, 0x80000000, 0x00000000, None),
        (25, 0x0000007f, 0x0000007f, None),
        (25, 0x00000080, 0xffffff80, None),
        (25, 0x000000ff, 0xffffffff, None),
        (25, 0x00007fff, 0xffffffff, None),
        (25, 0x00008000, 0x00000000, None),
        (25, 0x0000ffff, 0xffffffff, None),
        (25, 0x00010000, 0x00000000, None),
        // SEXT_I16.
        (26, 0x00000000, 0x00000000, None),
        (26, 0xffffffff, 0xffffffff, None),
        (26, 0x80000000, 0x00000000, None),
        (26, 0x0000007f, 0x0000007f, None),
        (26, 0x00000080, 0x00000080, None),
        (26, 0x000000ff, 0x000000ff, None),
        (26, 0x00007fff, 0x00007fff, None),
        (26, 0x00008000, 0xffff8000, None),
        (26, 0x0000ffff, 0xffffffff, None),
        (26, 0x00010000, 0x00000000, None),
        // ABS.
        (52, 0x00000000, 0x00000000, Some(false)),
        (52, 0xffffffff, 0x00000001, Some(true)),
        (52, 0x80000000, 0x80000000, Some(true)),
        (52, 0x0000007f, 0x0000007f, Some(true)),
        (52, 0x00000080, 0x00000080, Some(true)),
        (52, 0x000000ff, 0x000000ff, Some(true)),
        (52, 0x00007fff, 0x00007fff, Some(true)),
        (52, 0x00008000, 0x00008000, Some(true)),
        (52, 0x0000ffff, 0x0000ffff, Some(true)),
        (52, 0x00010000, 0x00010000, Some(true)),
        // Dirty upper bits must not survive sign extension.
        (25, 0xffff007f, 0x0000007f, None),
        (25, 0x12345680, 0xffffff80, None),
        (26, 0xffff7fff, 0x00007fff, None),
        (26, 0x12348000, 0xffff8000, None),
        // Signed first-differing-bit scan and wrapping signed absolute value.
        (23, 0xfffffffe, 0x0000001f, None),
        (23, 0xffff7fff, 0x00000010, None),
        (23, 0x7fffffff, 0x00000001, None),
        (52, 0x80000001, 0x7fffffff, Some(true)),
        (52, 0xfffffffe, 0x00000002, Some(true)),
        // An asymmetric pattern distinguishes reversal from a swap or identity.
        (11, 0x12345678, 0x1e6a2c48, None),
    ];
    assert_eq!(cases.len(), 120);
    for &(opcode, input, result, changed) in cases {
        for dst in [32u32, 33] {
            for constants in [false, true] {
                let mut ps =
                    include_bytes!("../../../../fixtures/rdna2/x9-scalar-mul-ps.bin").to_vec();
                // Replace only the four-byte multiply; source is always s32.
                ps[12..16]
                    .copy_from_slice(&(0xbe800000 | dst << 16 | opcode << 8 | 32).to_le_bytes());
                // The following MOV captures the selected destination in s105.
                ps[16] = dst as u8;
                if constants {
                    // Replace the two original moves with literal s32 initialization
                    // and an unused s33 initializer. All later relative branches shift
                    // together; the result and SCC observation sequence is unchanged.
                    let mut prefix: Vec<u8> = [0xbea003ffu32, input, 0xbea10380]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect();
                    prefix.extend_from_slice(&ps[8..]);
                    ps = prefix;
                }
                let pipeline = compile_graphics_pipeline(Architecture::Gfx1030, VS, &ps)
                    .unwrap_or_else(|error| {
                        panic!("opcode={opcode} input={input:#010x} dst={dst} constants={constants}: {error:?}")
                    });
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
                                input,
                                0,
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
                            ]],
                            "opcode={opcode} input={input:#010x} result={result:#010x} dst={dst} constants={constants} incoming_scc={flag} matches={matches}"
                        );
                    }
                }
            }
        }
    }
}
