// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_shader::{
    Architecture::Gfx1030, CompileError, compile, compile_graphics, compile_graphics_pipeline,
};

const CONSTANT_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const CONSTANT_PS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
const VARYING_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
const VARYING_PS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
const TRANSFORM_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-transform-vs.bin");
const END: &[u8] = &[0x00, 0x00, 0x81, 0xbf];

#[test]
fn compiles_image_sample_lz_in_pixel_shader_and_emits_msl_texture_sampling() {
    // VS: simple passthrough that exports pos0 from v[0:3] and param0 from v[4:7]
    let vs_bytes = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");

    // PS that samples a texture at (u, v) and exports color to mrt0:
    // v_interp_p1_f32_e32 v2, v0, attr0.x
    // v_interp_p2_f32_e32 v2, v1, attr0.x
    // v_interp_p1_f32_e32 v3, v0, attr0.y
    // v_interp_p2_f32_e32 v3, v1, attr0.y
    // image_sample_lz v[4:7], v[2:3], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D
    // exp mrt0, v4, v5, v6, v7 done vm
    // s_endpgm
    let mut ps_bytes = Vec::new();
    // attr0.x interp p1 & p2
    ps_bytes.extend_from_slice(&[0x00, 0x00, 0x08, 0xc8]); // v_interp_p1_f32_e32 v2, v0, attr0.x
    ps_bytes.extend_from_slice(&[0x01, 0x00, 0x09, 0xc8]); // v_interp_p2_f32_e32 v2, v1, attr0.x
    // attr0.y interp p1 & p2
    ps_bytes.extend_from_slice(&[0x00, 0x01, 0x0c, 0xc8]); // v_interp_p1_f32_e32 v3, v0, attr0.y
    ps_bytes.extend_from_slice(&[0x01, 0x01, 0x0d, 0xc8]); // v_interp_p2_f32_e32 v3, v1, attr0.y
    // image_sample_lz v[4:7], v[2:3], s[0:7], s[8:11] dmask:0xf dim:SQ_RSRC_IMG_2D
    ps_bytes.extend_from_slice(&[0x08, 0x0f, 0x9c, 0xf0, 0x02, 0x04, 0x40, 0x00]);
    // exp mrt0, v4, v5, v6, v7 done vm
    ps_bytes.extend_from_slice(&[0x0f, 0x18, 0x00, 0xf8, 0x04, 0x05, 0x06, 0x07]);
    // s_endpgm
    ps_bytes.extend_from_slice(&[0x00, 0x00, 0x81, 0xbf]);

    let compiled = compile_graphics(Gfx1030, vs_bytes, &ps_bytes)
        .expect("compile graphics with texture sampling");

    assert!(compiled.ir.contains("sample_lz.2d.f32 tex0, samp0"));
    assert!(
        compiled
            .msl
            .contains("texture2d<float, access::sample> tex0 [[texture(0)]]")
    );
    assert!(compiled.msl.contains("sampler samp0 [[sampler(0)]]"));
    assert!(compiled.msl.contains("tex0.sample(samp0, float2("));
}

#[test]
fn links_constant_and_varying_graphics_fixtures() {
    let constant = compile_graphics(Gfx1030, CONSTANT_VS, CONSTANT_PS).unwrap();
    assert!(
        constant
            .disassembly
            .contains("exp mrt0, v8, v9, v9, v8 done vm")
    );
    assert!(constant.ir.contains("export.f32x4 mrt0, t0, t1, t1, t0"));
    assert!(constant.msl.contains("fragment PixelOutput xenora_pixel()"));

    let varying = compile_graphics(Gfx1030, VARYING_VS, VARYING_PS).unwrap();
    assert!(varying.ir.contains("export.f32x4 param0, t4, t5, t6, t7"));
    assert!(varying.ir.contains("export.f32x4 mrt0, t0, t1, t2, t3"));
    assert!(
        varying
            .msl
            .contains("float4 parameter [[user(locn0), center_perspective]]")
    );
    for component in 0..4 {
        assert!(varying.ir.contains(&format!(
            "interpolate.center_perspective.f32 attr0[{component}]"
        )));
    }
}

#[test]
fn preserves_transform_operands_and_keeps_vertex_arithmetic_unfused() {
    let shader = compile_graphics(Gfx1030, TRANSFORM_VS, CONSTANT_PS).unwrap();
    assert!(shader.ir.contains("0010: t10 = mul.f32 t0, t8"));
    assert!(shader.ir.contains("0014: t11 = mul.f32 t1, t8"));
    assert!(shader.ir.contains("0018: t12 = add.f32 t10, t9"));
    assert!(
        shader
            .ir
            .contains("001c: export.f32x4 pos0, t12, t11, t2, t3")
    );
    assert!(
        shader
            .msl
            .contains("    #pragma clang fp contract(off)\n    #pragma clang fp reassociate(off)")
    );
}

#[test]
fn refuses_unlinked_or_missing_exports() {
    for (vertex, pixel, expected) in [
        (VARYING_VS, CONSTANT_PS, "PARAM0"),
        (CONSTANT_VS, VARYING_PS, "PARAM0"),
        (END, CONSTANT_PS, "POS0"),
        (CONSTANT_VS, END, "MRT0"),
    ] {
        let error = compile_graphics(Gfx1030, vertex, pixel).unwrap_err();
        assert!(matches!(error, CompileError::GraphicsLink { .. }));
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn refuses_invalid_export_sources_stage_and_placement() {
    let mut uninitialized = CONSTANT_VS.to_vec();
    uninitialized[4] = 8;
    let mut duplicate = CONSTANT_VS[..8].to_vec();
    duplicate.extend_from_slice(CONSTANT_VS);
    let mut trailing = CONSTANT_VS.to_vec();
    trailing.extend_from_slice(END);
    let mut misplaced_move = CONSTANT_VS[..8].to_vec();
    misplaced_move.extend_from_slice(&CONSTANT_PS[..4]);
    misplaced_move.extend_from_slice(END);
    for (vertex, expected_offset, message) in [
        (
            &uninitialized[..],
            0,
            "source VGPR is uninitialized or reserved",
        ),
        (&duplicate[..], 8, "export target is written more than once"),
        (&trailing[..], 12, "instruction follows program termination"),
        (&misplaced_move[..], 8, "instruction is unsupported"),
        (&CONSTANT_VS[..8], 8, "missing program termination"),
        (CONSTANT_PS, 8, "export target is invalid for this stage"),
    ] {
        let error = compile_graphics(Gfx1030, vertex, CONSTANT_PS).unwrap_err();
        assert_stage_error(&error, "vertex", expected_offset, message);
    }
}

#[test]
fn refuses_unsupported_interpolation_and_graphics_resources() {
    for (index, replacement, expected_offset, expected) in [
        (0, 1, 0, "instruction is unsupported"),
        (1, 4, 0, "instruction is unsupported"),
        (4, 2, 4, "matching second interpolation phase"),
        (5, 1, 4, "matching second interpolation phase"),
        (6, 0x25, 4, "matching second interpolation phase"),
    ] {
        let mut pixel = VARYING_PS.to_vec();
        pixel[index] = replacement;
        let error = compile_graphics(Gfx1030, VARYING_VS, &pixel).unwrap_err();
        assert_stage_error(&error, "pixel", expected_offset, expected);
    }
    let resource_instruction = &include_bytes!("../../../fixtures/rdna2/add.bin")[..8];
    let error = compile_graphics(Gfx1030, resource_instruction, CONSTANT_PS).unwrap_err();
    assert_stage_error(&error, "vertex", 0, "instruction is unsupported");
    let error = compile(Gfx1030, CONSTANT_VS).unwrap_err();
    assert!(matches!(error, CompileError::Verify { offset: 0, .. }));
}

fn assert_stage_error(error: &CompileError, stage: &str, offset: usize, message: &str) {
    let CompileError::GraphicsStage {
        stage: actual_stage,
        error: cause,
    } = error
    else {
        panic!("expected stage-specific error, found {error}");
    };
    assert_eq!(*actual_stage, stage);
    assert!(
        matches!(cause.as_ref(), CompileError::Verify { offset: actual, reason } if *actual == offset && reason.contains(message)),
        "{error}"
    );
}

#[test]
fn pixel_arithmetic_lowers_after_interpolation_without_contraction() {
    let pixel = include_bytes!("../../../fixtures/rdna2/x9-pixel-arithmetic-ps.bin");
    let compiled = compile_graphics(Gfx1030, VARYING_VS, pixel).unwrap();
    let pixel_ir = compiled.ir.split("pixel:\n").nth(1).unwrap();
    assert_eq!(pixel_ir.matches("mul.f32").count(), 2);
    assert_eq!(pixel_ir.matches("add.f32").count(), 2);
    let pixel_msl = compiled.msl.split("fragment PixelOutput").nth(1).unwrap();
    assert!(pixel_msl.contains("#pragma clang fp contract(off)"));
    assert!(pixel_msl.contains("#pragma clang fp reassociate(off)"));
    assert!(
        pixel_msl.find("#pragma clang fp").unwrap()
            < pixel_msl.find("PixelOutput output;").unwrap()
    );
}

#[test]
fn pixel_arithmetic_rejects_reserved_and_uninitialized_registers() {
    let original = include_bytes!("../../../fixtures/rdna2/x9-pixel-arithmetic-ps.bin");
    // LLVM-assembled VOP2 starts at byte 48. Destination occupies bits 17..24;
    // src1 occupies bits 9..16, src0 occupies bits 0..8.
    let word = u32::from_le_bytes(original[48..52].try_into().unwrap());
    for dst in [0, 1] {
        let mut pixel = original.to_vec();
        let changed = (word & !(0xff << 17)) | (dst << 17);
        pixel[48..52].copy_from_slice(&changed.to_le_bytes());
        let error = compile_graphics(Gfx1030, VARYING_VS, &pixel).unwrap_err();
        assert_stage_error(&error, "pixel", 48, "reserved for interpolation");
    }
    for src in [0, 1, 255] {
        let mut pixel = original.to_vec();
        let changed = (word & !(0xff << 9)) | (src << 9);
        pixel[48..52].copy_from_slice(&changed.to_le_bytes());
        let error = compile_graphics(Gfx1030, VARYING_VS, &pixel).unwrap_err();
        assert_stage_error(&error, "pixel", 48, "uninitialized or reserved");
    }
    let mut pixel = original[..72].to_vec(); // include the MRT export
    pixel.extend_from_slice(&original[48..52]);
    pixel.extend_from_slice(END);
    let error = compile_graphics(Gfx1030, VARYING_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 72, "instruction is unsupported");
}

#[test]
fn multiple_color_exports_preserve_all_attachment_indices() {
    let pixel = include_bytes!("../../../fixtures/rdna2/x9-mrt-ps.bin");
    let compiled = compile_graphics(Gfx1030, CONSTANT_VS, pixel).unwrap();
    for slot in 0..8 {
        assert!(compiled.msl.contains(&format!("[[color({slot})]]")));
        assert!(compiled.ir.contains(&format!("export.f32x4 mrt{slot},")));
    }
    assert!(compiled.disassembly.contains("exp mrt0, v3, v3, v3, v2 vm"));
    assert!(
        compiled
            .disassembly
            .contains("exp mrt7, v2, v2, v2, v2 done vm")
    );
}

#[test]
fn multiple_color_exports_reject_incomplete_duplicate_and_post_done_sequences() {
    let original = include_bytes!("../../../fixtures/rdna2/x9-mrt-ps.bin");
    let mut early_done = original.to_vec();
    early_done[9] |= 8;
    let mut missing_done = original.to_vec();
    missing_done[65] &= !8;
    let mut duplicate = original.to_vec();
    duplicate[16] = 0x0f; // Second export repeats MRT0, still without DONE.
    for (pixel, offset, message) in [
        (early_done, 16, "pixel export follows DONE"),
        (missing_done, 72, "final pixel export requires DONE"),
        (duplicate, 16, "export target is written more than once"),
    ] {
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
        assert_stage_error(&error, "pixel", offset, message);
    }
    let mut gap = original.to_vec();
    gap.drain(40..48); // Remove MRT4 without renumbering later targets.
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &gap).unwrap_err();
    assert!(matches!(error,CompileError::GraphicsLink { reason } if reason.contains("contiguous")));
    // Export order need not equal slot order; the final export must still be DONE.
    let mut reordered = original.to_vec();
    reordered[8..16].copy_from_slice(&original[16..24]);
    reordered[16..24].copy_from_slice(&original[8..16]);
    let compiled = compile_graphics(Gfx1030, CONSTANT_VS, &reordered).unwrap();
    assert!(compiled.ir.contains("export.f32x4 mrt0, t1, t1, t1, t0"));
    assert!(compiled.ir.contains("export.f32x4 mrt1, t0, t1, t1, t0"));
    let mut interleaved = original.to_vec();
    // v_add_f32_e32 v4, v2, v2 after the first non-final export.
    interleaved.splice(16..16, [0x02, 0x05, 0x08, 0x06]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &interleaved).unwrap_err();
    assert_stage_error(&error, "pixel", 16, "instruction is unsupported");
}

#[test]
fn graphics_pipeline_metadata_and_legacy_shader_outputs_match_fixtures() {
    let mrt: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-mrt-ps.bin");
    let sampled: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin");
    // Resource requirements are literal expectations from the original fixtures.
    for (name, vertex, pixel, color_targets, uses_texture) in [
        ("constant", CONSTANT_VS, CONSTANT_PS, 1, false),
        ("mrt", CONSTANT_VS, mrt, 8, false),
        ("sampled arithmetic", VARYING_VS, sampled, 1, true),
    ] {
        let pipeline = compile_graphics_pipeline(Gfx1030, vertex, pixel).unwrap();
        assert_eq!(pipeline.color_targets, color_targets, "{name}");
        assert_eq!(pipeline.uses_texture, uses_texture, "{name}");

        let legacy = compile_graphics(Gfx1030, vertex, pixel).unwrap();
        assert_eq!(pipeline.shader.msl, legacy.msl, "{name}: MSL");
        assert_eq!(pipeline.shader.ir, legacy.ir, "{name}: IR");
        assert_eq!(
            pipeline.shader.disassembly, legacy.disassembly,
            "{name}: disassembly"
        );
    }
}

// AMD RDNA 2 ISA section 12.5, S_BRANCH (SOPP opcode 2):
// target = instruction byte offset + 4 + sign_extend(SIMM16) * 4.
// https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf
// These literal words and expected offsets do not use the production decoder
// or assembler to calculate expected branch destinations.
fn graphics_words(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

#[test]
fn graphics_forward_branch_skips_eight_byte_poison_in_each_stage() {
    let vertex = graphics_words(&[
        0xbf820002, // byte 0 -> byte 12, skipping an invalid POS0 export
        0xf80008cf, 0x03020108, // POS0 reads uninitialized v8
        0xf80008cf, 0x03020100, // byte 12: POS0 = input v0..v3
        0xbf810000,
    ]);
    let pixel = graphics_words(&[
        0xbf820002, // byte 0 -> byte 12, skipping a literal-word instruction
        0x7e1002ff, 0x7fc00000, // v_mov_b32 v8, NaN: invalid if reached
        0x7e1002f2, // byte 12: v8 = 1.0
        0x7e120280, // byte 16: v9 = 0.0
        0xf800180f, 0x08090908, // byte 20: MRT0 = (1,0,0,1), DONE
        0xbf810000,
    ]);
    let compiled = compile_graphics(Gfx1030, &vertex, &pixel).unwrap();
    assert!(
        compiled
            .ir
            .contains("000c: export.f32x4 pos0, t0, t1, t2, t3")
    );
    assert_eq!(
        compiled.ir.split("pixel:\n").nth(1).unwrap(),
        "000c: t0 = const.f32 0x3f800000\n0010: t1 = const.f32 0x00000000\n0014: export.f32x4 mrt0, t0, t1, t1, t0\n"
    );
    assert!(
        compiled
            .msl
            .contains("output.color0 = float4(t0, t1, t1, t0);")
    );
    assert!(!compiled.msl.contains("7fc00000"));
}

#[test]
fn graphics_forward_branch_supports_fallthrough_chains_and_preserves_export_state() {
    let pixel = graphics_words(&[
        0xbf820000, // byte 0 -> byte 4: SIMM16 zero advances one instruction
        0xbf820002, // byte 4 -> byte 16
        0x7e1002ff, 0x7fc00000, 0x7e1002f2, 0x7e120280, 0xf800180f,
        0x08090908, // byte 24: MRT0 DONE
        0xbf820001, // byte 32 -> byte 40, skipping illegal post-export arithmetic
        0x7e1002f2, 0xbf810000,
    ]);
    let compiled = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(
        compiled.ir.split("pixel:\n").nth(1).unwrap(),
        "0010: t0 = const.f32 0x3f800000\n0014: t1 = const.f32 0x00000000\n0018: export.f32x4 mrt0, t0, t1, t1, t0\n"
    );
    // A branch must not reset DONE/export tracking on the selected path.
    let duplicate = graphics_words(&[
        0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, 0xbf820000, 0xf800180f, 0x08090908,
        0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &duplicate).unwrap_err();
    assert_stage_error(&error, "pixel", 20, "pixel export follows DONE");
}

#[test]
fn graphics_forward_branch_does_not_initialize_skipped_registers_or_hide_reached_poison() {
    let uninitialized = graphics_words(&[
        0xbf820001, // byte 0 -> byte 8, skipping v8 initialization
        0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &uninitialized).unwrap_err();
    assert_stage_error(&error, "pixel", 12, "uninitialized or reserved");
    let reached = graphics_words(&[
        0xbf820000, 0x7e1002ff, 0x7fc00000, 0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908,
        0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &reached).unwrap_err();
    assert_stage_error(&error, "pixel", 4, "finite FP32 bits");
}

#[test]
fn graphics_forward_branch_rejects_bad_targets_and_loops() {
    use xenora_decoder::DecodeErrorKind;
    for (prefix, target) in [
        (vec![0xbf820001, 0x7e1002ff, 0x3f800000], 8), // literal payload
        (vec![0xbf820001, 0xf800180f, 0x08090908], 8), // export second word
        (vec![0xbf820005], 24),                        // one-past-end byte offset
        (vec![0xbf827fff], 131072),                    // far outside the input
    ] {
        let mut pixel = graphics_words(&prefix);
        pixel.extend_from_slice(CONSTANT_PS);
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
        let CompileError::GraphicsStage {
            stage: "pixel",
            error,
        } = error
        else {
            panic!("expected pixel-stage decode error");
        };
        assert!(matches!(*error, CompileError::Decode(ref error)
            if error.offset == 0 && error.kind == (DecodeErrorKind::InvalidBranchTarget { target })));
    }
    for (words, offset) in [
        (vec![0xbf82ffff, 0xbf810000], 0), // -1: self-loop to byte 0
        (vec![0x7e1002f2, 0xbf82fffe, 0xbf810000], 4), // -2: back to byte 0
    ] {
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &graphics_words(&words)).unwrap_err();
        let CompileError::GraphicsStage {
            stage: "pixel",
            error,
        } = error
        else {
            panic!("expected pixel-stage decode error");
        };
        assert!(matches!(*error, CompileError::Verify { offset: at, reason }
            if at == offset && reason == "counted loop requires a preheader and nonempty body"));
    }
}

#[test]
fn graphics_forward_branch_requires_exports_on_the_chosen_path() {
    let vertex = graphics_words(&[0xbf820002, 0xf80008cf, 0x03020100, 0xbf810000]); // byte 0 -> END at byte 12, bypassing POS0
    let error = compile_graphics(Gfx1030, &vertex, CONSTANT_PS).unwrap_err();
    assert!(matches!(error, CompileError::GraphicsLink { reason }
        if reason.contains("POS0")));
    let pixel = graphics_words(&[
        0xbf820004, 0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]); // byte 0 -> END at byte 20, bypassing MRT0
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert!(matches!(error, CompileError::GraphicsLink { reason }
        if reason.contains("MRT0")));
}

#[test]
fn graphics_forward_branch_preserves_interpolation_pair_checks() {
    let mut pixel = graphics_words(&[0xbf820001]); // target byte 8 is attr0.x P2
    pixel.extend_from_slice(VARYING_PS);
    let error = compile_graphics(Gfx1030, VARYING_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 0, "second interpolation phase");
    let mut interrupted = VARYING_PS[..4].to_vec();
    interrupted.extend_from_slice(&graphics_words(&[0xbf820000]));
    interrupted.extend_from_slice(&VARYING_PS[4..]);
    let error = compile_graphics(Gfx1030, VARYING_VS, &interrupted).unwrap_err();
    assert_stage_error(
        &error,
        "pixel",
        4,
        "branch target enters the second interpolation phase",
    );
    // SCC must be initialized even when the branch targets the next instruction.
    let mut conditional = graphics_words(&[0xbf840000]);
    conditional.extend_from_slice(CONSTANT_PS);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &conditional).unwrap_err();
    assert_stage_error(&error, "pixel", 0, "SCC is uninitialized");
}

#[test]
fn graphics_forward_branch_still_decodes_unreachable_words_and_targets() {
    use xenora_decoder::DecodeErrorKind;
    for (skipped, expected) in [
        (
            0xffffffff,
            DecodeErrorKind::UnsupportedOpcode { word: 0xffffffff },
        ),
        (
            0xbf827fff,
            DecodeErrorKind::InvalidBranchTarget { target: 131076 },
        ),
    ] {
        // Byte 0 -> byte 8 bypasses byte 4, but full-stream decoding must
        // reject its unknown opcode or its out-of-range branch destination.
        let mut pixel = graphics_words(&[0xbf820001, skipped]);
        pixel.extend_from_slice(CONSTANT_PS);
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
        let CompileError::GraphicsStage {
            stage: "pixel",
            error,
        } = error
        else {
            panic!("expected pixel-stage decode error");
        };
        assert!(matches!(*error, CompileError::Decode(ref error)
            if error.offset == 4 && error.kind == expected));
    }
}

#[test]
fn graphics_forward_branch_can_bypass_but_not_reach_early_termination() {
    // Both streams contain an early END at byte 4 and a final END at byte 24.
    // Only the first branch bypasses the early END and reaches the exports.
    let mut pixel = graphics_words(&[0xbf820001, 0xbf810000]);
    pixel.extend_from_slice(CONSTANT_PS);
    let compiled = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(
        compiled.ir.split("pixel:\n").nth(1).unwrap(),
        "0008: t0 = const.f32 0x3f800000\n000c: t1 = const.f32 0x00000000\n0010: export.f32x4 mrt0, t0, t1, t1, t0\n"
    );
    pixel[..4].copy_from_slice(&0xbf820000_u32.to_le_bytes());
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(
        &error,
        "pixel",
        8,
        "instruction follows program termination",
    );
}

#[test]
fn graphics_forward_branch_cannot_skip_the_only_done_export() {
    let pixel = graphics_words(&[
        0x7e1002f2, 0x7e120280, 0xf800100f, 0x08090908, // byte 8: MRT0 without DONE
        0xbf820002, // byte 16 -> END at byte 28
        0xf800181f, 0x08090908, // byte 20: skipped MRT1, the only DONE
        0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 28, "final pixel export requires DONE");
    // Reaching that same DONE export completes the contiguous MRT0/MRT1 pair.
    let mut reached = pixel;
    reached[16..20].copy_from_slice(&0xbf820000_u32.to_le_bytes());
    let compiled = compile_graphics(Gfx1030, CONSTANT_VS, &reached).unwrap();
    assert!(
        compiled
            .ir
            .contains("0014: export.f32x4 mrt1, t0, t1, t1, t0")
    );
}

#[test]
fn graphics_branch_metadata_ignores_unreached_sampling_and_extra_targets() {
    for skipped in [
        [0xf09c0f08, 0x00400402], // image_sample_lz, decodable but unused
        [0xf800187f, 0x08090908], // MRT7 DONE, bypassed before MRT0
    ] {
        let mut pixel = graphics_words(&[0xbf820002, skipped[0], skipped[1]]);
        pixel.extend_from_slice(CONSTANT_PS);
        let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
        assert!(!compiled.uses_texture);
        assert_eq!(compiled.color_targets, 1);
        assert!(!compiled.shader.msl.contains("texture2d"));
    }
}

#[test]
fn graphics_scalar_compare_all_twelve_relations_have_independent_literal_expectations() {
    // AMD RDNA2 §12.4: SOPC opcodes 0..5 signed EQ,LG,GT,GE,LT,LE;
    // 6..11 are the corresponding unsigned relations. No production semantic
    // helper or decoder is used to compute these expected Boolean results.
    let cases = [
        (
            0u32,
            0x80u32,
            [
                true, false, false, true, false, true, true, false, false, true, false, true,
            ],
        ),
        (
            0xffff_ffff,
            0x80,
            [
                false, true, false, false, true, true, false, true, true, true, false, false,
            ],
        ),
        (
            0x8000_0000,
            0x81,
            [
                false, true, false, false, true, true, false, true, true, true, false, false,
            ],
        ),
        (
            0x7fff_ffff,
            0xc1,
            [
                false, true, true, true, false, false, false, true, false, false, true, true,
            ],
        ),
        (
            0,
            0x81,
            [
                false, true, false, false, true, true, false, true, false, false, true, true,
            ],
        ),
        // These are integer bits, legal even though their FP32 interpretations are Inf/NaN.
        (
            0x7f80_0000,
            0x80,
            [
                false, true, true, true, false, false, false, true, true, true, false, false,
            ],
        ),
        (
            0x7fc0_0000,
            0x80,
            [
                false, true, true, true, false, false, false, true, true, true, false, false,
            ],
        ),
    ];
    for (lhs, rhs_encoding, expected) in cases {
        for (opcode, scc) in expected.into_iter().enumerate() {
            for vertex_stage in [false, true] {
                let code = graphics_words(&[
                    0x7e100280, // byte 0: v8=0
                    0xbf0000ff | ((opcode as u32) << 16) | (rhs_encoding << 8),
                    lhs,
                    0xbf840001, // byte 12: SCC0 -> byte 20
                    0x7e1002f2, // byte 16: v8=1 only when SCC=true
                    0x7e120280, // byte 20: v9=0
                    if vertex_stage { 0xf80008cf } else { 0xf800180f },
                    0x08090908,
                    0xbf810000,
                ]);
                let compiled = if vertex_stage {
                    compile_graphics(Gfx1030, &code, CONSTANT_PS)
                } else {
                    compile_graphics(Gfx1030, CONSTANT_VS, &code)
                }
                .unwrap();
                let expected_export = match (vertex_stage, scc) {
                    (false, true) => "0018: export.f32x4 mrt0, t1, t2, t2, t1",
                    (false, false) => "0018: export.f32x4 mrt0, t0, t1, t1, t0",
                    (true, true) => "0018: export.f32x4 pos0, t9, t10, t10, t9",
                    (true, false) => "0018: export.f32x4 pos0, t8, t9, t9, t8",
                };
                assert!(
                    compiled.ir.contains(expected_export),
                    "opcode={opcode} lhs={lhs:#x} vertex={vertex_stage}: {}",
                    compiled.ir
                );
            }
        }
    }
}

#[test]
fn graphics_scc_survives_vector_operations_and_repeated_compare_overwrites() {
    for comparisons in [vec![0xbf008180], vec![0xbf008080, 0xbf008180]] {
        // EQ 0,1 sets false; the second case must overwrite earlier EQ 0,0=true.
        let mut words = comparisons;
        words.extend_from_slice(&[
            0x7e1002f2, 0x7e120280, // vector moves must preserve SCC=false
            0xbf840002, // skip the eight-byte NaN move
            0x7e1402ff, 0x7fc00000, 0xf800180f, 0x08090908, 0xbf810000,
        ]);
        compile_graphics(Gfx1030, CONSTANT_VS, &graphics_words(&words)).unwrap();
    }
    // Literal source1, inline source0: signed 0 < INT_MIN is false.
    let pixel = graphics_words(&[
        0xbf04ff80, 0x80000000, 0xbf840002, 0x7e1402ff, 0x7fc00000, 0x7e1002f2, 0x7e120280,
        0xf800180f, 0x08090908, 0xbf810000,
    ]);
    compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
}

#[test]
fn graphics_scc_rejects_register_sources_and_uninitialized_selected_paths() {
    for compare in [0xbf008000, 0xbf000080] {
        let mut code = graphics_words(&[compare]); // s0 in either source position
        code.extend_from_slice(CONSTANT_PS);
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
        assert_stage_error(&error, "pixel", 0, "SGPRs are unsupported");
    }
    let mut code = graphics_words(&[0xbf820001, 0xbf008080, 0xbf840000]);
    code.extend_from_slice(CONSTANT_PS);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
    assert_stage_error(&error, "pixel", 8, "SCC is uninitialized");
}

#[test]
fn graphics_scc_validates_untaken_targets_and_full_stream() {
    for compare in [0xbf008080, 0xbf008180] {
        // true and false
        for branch in [0xbf84ffff, 0xbf84fffe] {
            // self and backwards
            let mut code = graphics_words(&[compare, branch]);
            code.extend_from_slice(CONSTANT_PS);
            let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
            let CompileError::GraphicsStage {
                stage: "pixel",
                error,
            } = error
            else {
                panic!("expected pixel-stage error");
            };
            assert!(matches!(*error, CompileError::Verify { offset: 4, reason }
                if reason == "counted loop requires an unconditional latch"));
        }
        let mut code = graphics_words(&[compare, 0xbf840001, 0x7e1402ff, 0x3f000000]);
        code.extend_from_slice(CONSTANT_PS);
        let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
        let CompileError::GraphicsStage {
            stage: "pixel",
            error,
        } = error
        else {
            panic!("expected pixel-stage error");
        };
        assert!(matches!(*error, CompileError::Decode(ref error)
            if error.offset == 4 && error.kind == xenora_decoder::DecodeErrorKind::InvalidBranchTarget { target: 12 }));
    }
    // SCC=false would skip this word, but full-stream decoding must still reject it.
    let mut code = graphics_words(&[0xbf008180, 0xbf840001, 0xffffffff]);
    code.extend_from_slice(CONSTANT_PS);
    assert!(compile_graphics(Gfx1030, CONSTANT_VS, &code).is_err());
    // An untaken SCC=true branch still cannot target outside the stage.
    let mut code = graphics_words(&[0xbf008080, 0xbf847fff]);
    code.extend_from_slice(CONSTANT_PS);
    assert!(compile_graphics(Gfx1030, CONSTANT_VS, &code).is_err());
}

#[test]
fn graphics_scc_branches_preserve_done_and_selected_export_requirements() {
    let code = graphics_words(&[
        0xbf008180, 0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, // byte12: DONE export
        0xbf840000, // byte20: taken to next export
        0xf800181f, 0x08090908, 0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
    assert_stage_error(&error, "pixel", 24, "follows DONE");
    let code = graphics_words(&[
        0xbf008180, 0x7e1002f2, 0x7e120280, 0xf800100f, 0x08090908, // MRT0 without DONE
        0xbf840002, // skip the only DONE export
        0xf800181f, 0x08090908, 0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
    assert_stage_error(&error, "pixel", 32, "requires DONE");
}

#[test]
fn graphics_scc_is_stage_local_and_branches_do_not_consume_it() {
    let mut vertex = graphics_words(&[0xbf008080]);
    vertex.extend_from_slice(CONSTANT_VS);
    let mut pixel = graphics_words(&[0xbf840000]);
    pixel.extend_from_slice(CONSTANT_PS);
    let error = compile_graphics(Gfx1030, &vertex, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 0, "SCC is uninitialized");

    let pixel = graphics_words(&[
        0xbf008180, // false
        0xbf840000, // zero displacement: next instruction, SCC stays false
        0x7e1002f2, 0x7e120280, 0x06101108, // v_add_f32 v8,v8,v8: SCC unchanged
        0xbf840002, // still false: skip eight-byte NaN vector move
        0x7e1402ff, 0x7fc00000, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    // True SCC also survives consecutive untaken zero-displacement branches.
    let mut pixel = graphics_words(&[0xbf008080, 0xbf840000, 0xbf840000]);
    pixel.extend_from_slice(CONSTANT_PS);
    compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
}

#[test]
fn graphics_scc_selected_metadata_excludes_skipped_sampling_and_mrt() {
    for skipped in [[0xf09c0f08, 0x00400402], [0xf800187f, 0x08090908]] {
        let mut pixel = graphics_words(&[0xbf008180, 0xbf840002, skipped[0], skipped[1]]);
        pixel.extend_from_slice(CONSTANT_PS);
        let compiled =
            xenora_shader::compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
        assert_eq!(compiled.color_targets, 1);
        assert!(!compiled.uses_texture);
    }
    // An untaken branch reaches poison; no speculative path merge may hide it.
    let mut pixel = graphics_words(&[0xbf008080, 0xbf840002, 0x7e1402ff, 0x7fc00000]);
    pixel.extend_from_slice(CONSTANT_PS);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 8, "finite FP32");
}

#[test]
fn graphics_scc_rejects_malformed_literals_even_on_skipped_paths() {
    for words in [vec![0xbf00ffff, 0], vec![0xbf0080ff]] {
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &graphics_words(&words)).is_err());
    }
    // The first branch skips malformed compare encoding, but decoding is whole-stream.
    let mut pixel = graphics_words(&[0xbf820002, 0xbf00ffff, 0]);
    pixel.extend_from_slice(CONSTANT_PS);
    assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
}

#[test]
fn graphics_scc_selected_interpolation_controls_stage_linking() {
    for (compare, taken) in [(0xbf008180, true), (0xbf008080, false)] {
        let mut pixel = graphics_words(&[compare, 0xbf840002]);
        pixel.extend_from_slice(&VARYING_PS[..8]); // one complete attr0.x pair
        pixel.extend_from_slice(CONSTANT_PS); // independent constant export
        let (matching, mismatching) = if taken {
            (CONSTANT_VS, VARYING_VS)
        } else {
            (VARYING_VS, CONSTANT_VS)
        };
        compile_graphics(Gfx1030, matching, &pixel).unwrap();
        assert!(matches!(compile_graphics(Gfx1030, mismatching, &pixel),
            Err(CompileError::GraphicsLink { reason }) if reason.contains("PARAM0")));
    }
}

#[test]
fn graphics_scc_skipped_writes_do_not_change_selected_state() {
    let mut pixel = graphics_words(&[
        0xbf008180, // SCC=false
        0xbf840001, // skip compare that would overwrite SCC=true
        0xbf008080, 0xbf840002, // still false: skip invalid vector move
        0x7e1402ff, 0x7fc00000,
    ]);
    pixel.extend_from_slice(CONSTANT_PS);
    compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();

    let pixel = graphics_words(&[
        0xbf008180, 0xbf840001, 0x7e1002f2, // v8 initialization skipped
        0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 16, "uninitialized");
}

#[test]
fn graphics_scc_selected_exports_require_pos0_mrt0_and_contiguous_targets() {
    let vertex = graphics_words(&[0xbf008180, 0xbf840002, 0xf80008cf, 0x03020100, 0xbf810000]);
    assert!(matches!(compile_graphics(Gfx1030, &vertex, CONSTANT_PS),
        Err(CompileError::GraphicsLink { reason }) if reason.contains("POS0")));
    let pixel = graphics_words(&[
        0xbf008180, 0xbf840004, 0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    assert!(matches!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel),
        Err(CompileError::GraphicsLink { reason }) if reason.contains("MRT0")));
    let pixel = graphics_words(&[
        0xbf008180, 0x7e1002f2, 0x7e120280, 0xf800100f, 0x08090908, // MRT0, not DONE
        0xbf840002, // skip MRT1
        0xf800101f, 0x08090908, 0xf800182f, 0x08090908, // MRT2 DONE
        0xbf810000,
    ]);
    assert!(matches!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel),
        Err(CompileError::GraphicsLink { reason }) if reason.contains("contiguous")));
}

#[test]
fn graphics_multislot_emits_all_four_independent_sampling_pairs() {
    let compiled = compile_graphics(
        Gfx1030,
        VARYING_VS,
        include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin"),
    )
    .unwrap();
    for texture in 0..2 {
        for sampler in 0..2 {
            assert!(
                compiled
                    .ir
                    .contains(&format!("tex{texture}, samp{sampler}"))
            );
            assert!(
                compiled
                    .msl
                    .contains(&format!("tex{texture}.sample(samp{sampler},"))
            );
        }
        assert_eq!(
            compiled
                .msl
                .matches(&format!("[[texture({texture})]]"))
                .count(),
            1
        );
        assert_eq!(
            compiled
                .msl
                .matches(&format!("[[sampler({texture})]]"))
                .count(),
            1
        );
    }
}

#[test]
fn graphics_multislot_metadata_is_exact_for_each_pair_and_selected_path() {
    let all = include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin");
    assert_eq!(
        compile_graphics_pipeline(Gfx1030, VARYING_VS, all)
            .unwrap()
            .sampled_pairs,
        15
    );
    // Original four interpolation instructions, one sample, one MRT0 export.
    for (resource, texture) in [(0u32, 0), (12, 1)] {
        for (sampler, slot) in [(8u32, 0), (20, 1)] {
            let mut pixel = all[..16].to_vec();
            pixel.extend_from_slice(&graphics_words(&[
                0xf09c0f08,
                0x00000402 | ((resource / 4) << 16) | ((sampler / 4) << 21),
                0xf800180f,
                0x07060504,
                0xbf810000,
            ]));
            let compiled = compile_graphics_pipeline(Gfx1030, VARYING_VS, &pixel).unwrap();
            assert_eq!(compiled.sampled_pairs, 1 << (texture * 2 + slot));
            assert!(compiled.uses_texture);
            assert_eq!(compiled.shader.msl.matches("[[texture(").count(), 1);
            assert_eq!(compiled.shader.msl.matches("[[sampler(").count(), 1);
            assert!(
                compiled
                    .shader
                    .msl
                    .contains(&format!("tex{texture}.sample(samp{slot},"))
            );
        }
    }
    // A skipped slot1 sample must not acquire bindings or interpolation linkage.
    let mut pixel = graphics_words(&[0xbf820002, 0xf09c0f08, 0x00a30402]);
    pixel.extend_from_slice(CONSTANT_PS);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.sampled_pairs, 0);
    assert!(!compiled.uses_texture);
}

#[test]
fn graphics_multislot_rejects_unmapped_descriptor_bases() {
    let fixture = include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin");
    for (resource, sampler) in [(4u32, 8u32), (8, 8), (16, 8), (0, 0), (0, 12), (0, 24)] {
        let mut pixel = fixture.to_vec();
        let sample = 0x00000402 | ((resource / 4) << 16) | ((sampler / 4) << 21);
        pixel[20..24].copy_from_slice(&sample.to_le_bytes());
        let error = compile_graphics_pipeline(Gfx1030, VARYING_VS, &pixel).unwrap_err();
        assert_stage_error(&error, "pixel", 16, "outside synthetic");
    }
}

#[test]
fn runtime_uniform_diamonds_preserve_both_paths_for_one_compiled_pipeline() {
    let compiled = compile_graphics_pipeline(
        Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-ps.bin"),
    )
    .expect("runtime scalar inputs must compile without specialization");
    // Both stages require runtime data; the condition cannot be folded away.
    assert_eq!(compiled.shader.msl.matches("[[buffer(2)]]").count(), 2);
    assert!(compiled.shader.msl.contains("if ("));
    // Both signs of the vertex translation remain represented in generated code.
    assert!(compiled.shader.msl.contains("0xbf000000u"));
    assert!(compiled.shader.msl.contains("0x3f000000u"));
}

#[test]
fn runtime_uniform_masks_are_stage_local_and_constant_dead_reads_disappear() {
    let compiled = compile_graphics_pipeline(
        Gfx1030,
        include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-vs.bin"),
        include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-ps.bin"),
    )
    .unwrap();
    assert_eq!(compiled.vertex_uniform_mask, 1);
    assert_eq!(compiled.pixel_uniform_mask, 2);
    assert_eq!(compiled.required_uniform_mask, 3);
    let mut pixel = graphics_words(&[0xbf820001, 0xbf00801f]); // skip s31 compare
    pixel.extend_from_slice(CONSTANT_PS);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.required_uniform_mask, 0);
    assert!(!compiled.shader.msl.contains("[[buffer(2)]]"));
}

#[test]
fn runtime_uniform_join_requires_every_incoming_definition() {
    // Only the fallthrough initializes v8; both edges reach the same export.
    let pixel = graphics_words(&[
        0xbf008018, 0xbf840001, 0x7e1002f2, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 16, "uninitialized");
}

#[test]
fn runtime_uniform_compare_emission_covers_all_integer_relations() {
    // Literal operator expectations are independent of ScalarCompareOp::test.
    for (opcode, signed, relation) in [
        (0, true, "=="),
        (1, true, "!="),
        (2, true, ">"),
        (3, true, ">="),
        (4, true, "<"),
        (5, true, "<="),
        (6, false, "=="),
        (7, false, "!="),
        (8, false, ">"),
        (9, false, ">="),
        (10, false, "<"),
        (11, false, "<="),
    ] {
        for literal in [0x80000000, 0x7fffffff, 0xffffffff, 0x7fc00000, 0x7f800000] {
            for reverse in [false, true] {
                let sources = if reverse { 0x18ff } else { 0xff18 };
                let mut pixel = graphics_words(&[0xbf000000 | (opcode << 16) | sources, literal]);
                pixel.extend_from_slice(CONSTANT_PS);
                let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
                assert_eq!(compiled.pixel_uniform_mask, 1);
                let uniform = if signed {
                    "as_type<int>(uniforms[0])".to_string()
                } else {
                    "uniforms[0]".to_string()
                };
                let bits = format!("0x{literal:08x}u");
                let bits = if signed {
                    format!("as_type<int>({bits})")
                } else {
                    bits
                };
                let expression = if reverse {
                    format!("{bits} {relation} {uniform}")
                } else {
                    format!("{uniform} {relation} {bits}")
                };
                assert!(compiled.shader.msl.contains(&expression), "{expression}");
            }
        }
    }
}

#[test]
fn runtime_uniform_join_preserves_inherited_values_and_merges_scc() {
    // v8 is inherited on the false edge and overwritten on the true edge.
    // SCC is replaced by true/false on the respective arms, then read at join.
    let pixel = graphics_words(&[
        0x7e100280, 0x7e1202f2, 0xbf008018, 0xbf840003, 0x7e1002f2, 0xbf008080, 0xbf820001,
        0xbf008180, 0xbf840001, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
    assert!(compiled.shader.ir.contains("<- true"));
    assert!(compiled.shader.ir.contains("<- false"));
    assert_eq!(compiled.shader.ir.matches("branch ").count(), 2);
    assert!(compiled.shader.ir.contains("export.f32x4 mrt0"));
}

#[test]
fn runtime_uniform_paths_must_share_the_actual_export_suffix() {
    // Enter MRT1 without MRT0; bypass all exports; terminate before the suffix;
    // or export MRT0 separately on both arms. None is a common export epilogue.
    let cases = [
        vec![
            0x7e1002f2, 0xbf008018, 0xbf840002, 0xf800100f, 0x08080808, 0xf800181f, 0x08080808,
            0xbf810000,
        ],
        vec![
            0x7e1002f2, 0xbf008018, 0xbf840002, 0xf800180f, 0x08080808, 0xbf810000,
        ],
        vec![
            0x7e1002f2, 0xbf008018, 0xbf840001, 0xbf810000, 0xf800180f, 0x08080808, 0xbf810000,
        ],
        vec![
            0x7e1002f2, 0xbf008018, 0xbf840003, 0xf800180f, 0x08080808, 0xbf820002, 0xf800180f,
            0x08080808, 0xbf810000,
        ],
    ];
    for words in cases {
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &graphics_words(&words)).is_err());
    }
}

#[test]
fn runtime_uniform_sampling_union_keeps_samples_inside_guarded_blocks() {
    let multislot = include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin");
    let mut pixel = multislot[..16].to_vec(); // initialize UV with complete pairs
    pixel.extend_from_slice(&graphics_words(&[
        0xbf008018, 0xbf840003, // s24 condition, false -> second sample
        0xf09c0f08, 0x00400402, // tex0/samp0 -> v4..7
        0xbf820002, 0xf09c0f08, 0x00a30402, // tex1/samp1 -> v4..7
        0xf800180f, 0x07060504, 0xbf810000,
    ]));
    let compiled = compile_graphics_pipeline(Gfx1030, VARYING_VS, &pixel).unwrap();
    assert_eq!(compiled.sampled_pairs, 0b1001);
    assert_eq!(compiled.pixel_uniform_mask, 1);
    let fragment = compiled
        .shader
        .msl
        .split("fragment PixelOutput")
        .nth(1)
        .unwrap();
    for sample in ["tex0.sample", "tex1.sample"] {
        let at = fragment.find(sample).unwrap();
        let guard = fragment[..at].rfind("case ").unwrap();
        assert!(
            !fragment[guard..at].contains('}'),
            "sample escaped its block"
        );
    }
    // With the identical CFG but a constant-true condition only pair0 remains.
    pixel[16..20].copy_from_slice(&0xbf008080u32.to_le_bytes());
    let compiled = compile_graphics_pipeline(Gfx1030, VARYING_VS, &pixel).unwrap();
    assert_eq!(compiled.sampled_pairs, 1);
    assert_eq!(compiled.required_uniform_mask, 0);
}

#[test]
fn runtime_uniform_cfg_rejects_phase_two_entry_backedges_and_undefined_scc() {
    let mut pixel = graphics_words(&[0xbf008018, 0xbf840001]);
    pixel.extend_from_slice(VARYING_PS); // target is the second interpolation phase
    assert!(compile_graphics(Gfx1030, VARYING_VS, &pixel).is_err());
    for prefix in [
        vec![0xbf008018, 0xbf84fffe],             // runtime backedge to compare
        vec![0xbf840000], // zero displacement still requires initialized SCC
        vec![0xbf820001, 0xbf008018, 0xbf840000], // skipped compare
    ] {
        let mut pixel = graphics_words(&prefix);
        pixel.extend_from_slice(CONSTANT_PS);
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
    }
    // Zero displacement is valid when SCC is initialized, including runtime SCC.
    let mut pixel = graphics_words(&[0xbf008018, 0xbf840000]);
    pixel.extend_from_slice(CONSTANT_PS);
    compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap();
}

#[test]
fn runtime_uniform_chain_of_diamonds_has_linear_emission() {
    let mut words = vec![0x7e100280, 0x7e1202f2];
    for _ in 0..32 {
        words.extend_from_slice(&[0xbf008018, 0xbf840002, 0x7e1002f2, 0xbf820001, 0x7e100280]);
    }
    words.extend_from_slice(&[0xf800180f, 0x08090908, 0xbf810000]);
    let compiled =
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &graphics_words(&words)).unwrap();
    assert_eq!(compiled.shader.ir.matches("branch ").count(), 32);
    assert!(compiled.shader.msl.len() < 100_000);
    assert!(compiled.shader.msl.matches("if (block ==").count() < 200);
}

#[test]
fn runtime_uniform_nested_joins_union_inputs_and_preserve_wrapper() {
    // Outer condition s24, nested condition s31, both with inherited v8.
    let pixel = graphics_words(&[
        0x7e100280, 0x7e1202f2, 0xbf008018, 0xbf840004, 0xbf00801f, 0xbf840001, 0x7e1002f2,
        0xbf820001, 0x7e120280, 0xf800180f, 0x08090908, 0xbf810000,
    ]);
    let pipeline = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(pipeline.vertex_uniform_mask, 0);
    assert_eq!(pipeline.pixel_uniform_mask, 0x81);
    assert_eq!(pipeline.required_uniform_mask, 0x81);
    assert_eq!(
        compile_graphics(Gfx1030, CONSTANT_VS, &pixel).unwrap().msl,
        pipeline.shader.msl
    );
    assert_eq!(pipeline.shader.msl.matches("[[buffer(2)]]").count(), 1);
}

#[test]
fn runtime_scalar_local_move_add_uniform_and_branch_preserve_input_mask() {
    // Original gfx1030 instructions. AMD RDNA2 sections 12.1/12.3/12.4:
    // s32 = 1; s33 = s32 + s24 (wrapping u32); SCC = (s33 == 2).
    // Uniform 1 selects red; uniform 0 selects black. Both paths define v8
    // and reach the same MRT0 export; no scalar-to-vector transfer is needed.
    let pixel = graphics_words(&[
        0xbea00381, // s_mov_b32 s32, 1
        0x80211820, // s_add_u32 s33, s32, s24
        0xbf068221, // s_cmp_eq_u32 s33, 2
        0x7e120280, // v_mov_b32 v9, 0 (preserves SCC)
        0xbf840002, // s_cbranch_scc0 to false arm
        0x7e1002f2, // true: v_mov_b32 v8, 1.0
        0xbf820001, // branch to common export
        0x7e100280, // false: v_mov_b32 v8, 0
        0xf800180f, 0x08090908, // exp mrt0 v8,v9,v9,v8 done vm
        0xbf810000, // s_endpgm
    ]);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel)
        .expect("initialized scalar locals must feed runtime arithmetic and control flow");
    assert_eq!(compiled.vertex_uniform_mask, 0);
    assert_eq!(compiled.pixel_uniform_mask, 1);
    assert_eq!(compiled.required_uniform_mask, 1);
    assert_eq!(compiled.color_targets, 1);
    assert_eq!(compiled.sampled_pairs, 0);
    assert!(compiled.shader.msl.contains("if ("));
}

// Raw SOP1/SOP2 encodings use the public gfx1030 fields. Expected results below
// are literal hand-worked words/flags, never ScalarAluOp::evaluate outputs.
fn scalar_local_mov(dst: u32, bits: u32) -> [u32; 2] {
    [0xbe8003ff | (dst << 16), bits]
}

fn scalar_local_alu(op: u32, dst: u32, left: u32, right: u32) -> u32 {
    0x80000000 | (op << 23) | (dst << 16) | (right << 8) | left
}

fn scalar_require_flag(words: &mut Vec<u32>, flag: bool) {
    // The wrong edge reaches a valid decoded read of undefined local s104.
    if flag {
        words.extend([0xbf840001, 0xbf820001, 0xbf008068]);
    } else {
        words.extend([0xbf840001, 0xbf008068]);
    }
}

#[test]
fn scalar_locals_constant_results_and_scc_have_literal_boundary_expectations() {
    // opcode, a, b, incoming SCC, result bits, outgoing SCC
    let cases = [
        (0, 0xffffffff, 1, false, 0, true),
        (0, 0x7fffffff, 1, false, 0x80000000, false),
        (1, 0, 1, false, 0xffffffff, true),
        (1, 0x80000000, 1, false, 0x7fffffff, false),
        (2, 0x7fffffff, 1, false, 0x80000000, true),
        (2, 0xffffffff, 1, false, 0, false),
        (2, 0x80000000, 0xffffffff, false, 0x7fffffff, true),
        (3, 0x80000000, 1, false, 0x7fffffff, true),
        (3, 0, 1, false, 0xffffffff, false),
        (3, 0x7fffffff, 0xffffffff, false, 0x80000000, true),
        (4, 0xffffffff, 0, true, 0, true),
        (4, 0xffffffff, 1, false, 0, true),
        (4, 0xffffffff, 1, true, 1, true),
        (4, 7, 8, false, 15, false),
        (5, 0xffffffff, 0xffffffff, true, 0xffffffff, true),
        (5, 0, 0xffffffff, true, 0, true),
        (5, 1, 0, true, 0, false),
        (5, 7, 7, false, 0, false),
        (6, 0x80000000, 0x7fffffff, false, 0x80000000, true),
        (6, 0x7fffffff, 0x80000000, false, 0x80000000, false),
        (7, 0xffffffff, 1, false, 1, false),
        (7, 1, 0xffffffff, false, 1, true),
        (8, 0xffffffff, 0, false, 0, false),
        (8, 0, 0xffffffff, false, 0, true),
        (9, 0xffffffff, 1, false, 0xffffffff, true),
        (9, 1, 0xffffffff, false, 0xffffffff, false),
        (6, 7, 7, true, 7, false),
        (7, 7, 7, true, 7, false),
        (8, 7, 7, true, 7, false),
        (9, 7, 7, true, 7, false),
        (10, 0x7fc00000, 0x7f800000, true, 0x7fc00000, true),
        (10, 0x7fc00000, 0x7f800000, false, 0x7f800000, false),
    ];
    for (op, a, b, incoming, result, flag) in cases {
        for dst in [32, 33, 105] {
            // alias left, alias right, highest ordinary SGPR
            let mut words = Vec::new();
            words.extend(scalar_local_mov(32, a));
            words.extend(scalar_local_mov(33, b));
            words.push(if incoming { 0xbf008080 } else { 0xbf008180 });
            words.push(scalar_local_alu(op, dst, 32, 33));
            // MOV must preserve the ALU's SCC while copying the result.
            words.push(0xbea20300 | dst); // s_mov_b32 s34, s[dst]
            scalar_require_flag(&mut words, flag);
            words.extend([0xbf06ff22, result]); // s_cmp_eq_u32 s34, literal
            scalar_require_flag(&mut words, true);
            let mut pixel = graphics_words(&words);
            pixel.extend_from_slice(CONSTANT_PS);
            let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel)
                .unwrap_or_else(|error| panic!("opcode {op}, dst {dst}: {error}"));
            assert_eq!(compiled.required_uniform_mask, 0);
            assert!(!compiled.shader.msl.contains("if ("));
        }
    }
}

#[test]
fn scalar_locals_runtime_all_operations_keep_uint_results_and_uniform_reads() {
    for op in 0..=10 {
        for dst in [32, 33, 105] {
            let words = [
                0xbea00318, // s32 = uniform0
                0xbea10319, // s33 = uniform1
                0xbf001a80, // SCC = 0 == uniform2
                scalar_local_alu(op, dst, 32, 33),
                0xbf068000 | dst, // compare local with zero
            ];
            let mut pixel = graphics_words(&words);
            pixel.extend_from_slice(CONSTANT_PS);
            let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
            assert_eq!(compiled.pixel_uniform_mask, 7);
            assert!(compiled.shader.msl.contains("uint t"));
            assert!(!compiled.shader.msl.contains("float t0 = uniforms"));
        }
    }
}

#[test]
fn scalar_locals_validate_both_select_sources_and_unknown_scc() {
    for op in [4, 5, 10] {
        let mut pixel = graphics_words(&[0xbea00381, scalar_local_alu(op, 32, 32, 0x81)]);
        pixel.extend_from_slice(CONSTANT_PS);
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
    }
    for (condition, left, right) in [(0xbf008080, 0x81, 32), (0xbf008180, 32, 0x81)] {
        let mut pixel = graphics_words(&[condition, scalar_local_alu(10, 33, left, right)]);
        pixel.extend_from_slice(CONSTANT_PS);
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
    }
    // Both reachable operands count, even the constant-unselected uniform and a
    // uniform MOV overwritten immediately afterward.
    let mut pixel = graphics_words(&[
        0xbf008080,
        scalar_local_alu(10, 32, 0x81, 31),
        0xbea1031e,
        0xbea10380,
    ]);
    pixel.extend_from_slice(CONSTANT_PS);
    assert_eq!(
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel)
            .unwrap()
            .required_uniform_mask,
        0xc0
    );
}

#[test]
fn scalar_locals_protect_abi_and_do_not_initialize_other_stage() {
    for dst in 0..32 {
        for words in [
            vec![0xbe800381 | (dst << 16)],
            vec![scalar_local_alu(0, dst, 0x81, 0x81)],
        ] {
            let mut pixel = graphics_words(&words);
            pixel.extend_from_slice(CONSTANT_PS);
            assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
        }
    }
    let mut vertex = graphics_words(&[0xbea00381]);
    vertex.extend_from_slice(CONSTANT_VS);
    let mut pixel = graphics_words(&[0xbf068120]);
    pixel.extend_from_slice(CONSTANT_PS);
    assert!(compile_graphics(Gfx1030, &vertex, &pixel).is_err());
}

#[test]
fn scalar_local_constant_paths_prune_uniform_reads_and_preserve_uniform_aliases() {
    // 1+1==2: branch over a reachable-encoding but dead uniform read.
    let mut pixel = graphics_words(&[
        0xbea00381,
        scalar_local_alu(0, 32, 32, 0x81),
        0xbf068220,
        0xbf840001,
        0xbf820001,
        0xbea1031f,
    ]);
    pixel.extend_from_slice(CONSTANT_PS);
    assert_eq!(
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel)
            .unwrap()
            .required_uniform_mask,
        0
    );
    // Input survives two MOV aliases, even if the original local is overwritten.
    let mut pixel = graphics_words(&[0xbea0031b, 0xbea10320, 0xbea00380, 0xbf068021]);
    pixel.extend_from_slice(CONSTANT_PS);
    assert_eq!(
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel)
            .unwrap()
            .required_uniform_mask,
        8
    );
}

#[test]
fn scalar_local_runtime_join_is_uint_and_requires_both_predecessors() {
    // Runtime fork chooses raw uint words with sign/NaN-shaped bits, then compares
    // the merged local. Both arms inherit s33 without manufacturing another phi.
    let prefix = [
        0xbea10381,
        0xbf008018,
        0xbf840003,
        0xbea003ff,
        0x80000000,
        0xbf820002,
        0xbea003ff,
        0x7fc00000,
        scalar_local_alu(0, 32, 32, 33),
        0xbf068020,
    ];
    let mut pixel = graphics_words(&prefix);
    pixel.extend_from_slice(CONSTANT_PS);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
    assert!(compiled.shader.msl.contains("uint t"));
    assert!(compiled.shader.ir.contains("<- 0x80000000u"));
    assert!(compiled.shader.ir.contains("<- 0x7fc00000u"));
    let mut missing = graphics_words(&[0xbf008018, 0xbf840001, 0xbea00381, 0xbf068120]);
    missing.extend_from_slice(CONSTANT_PS);
    assert!(compile_graphics(Gfx1030, CONSTANT_VS, &missing).is_err());
}

#[test]
fn scalar_local_join_merges_scc_before_carry_and_preserves_it_through_select() {
    // Both arms initialize s32 and replace SCC with different constants. At join,
    // ADDC consumes both uint/bool phis; CSELECT and MOV preserve its runtime SCC.
    let pixel = graphics_words(&[
        0xbf008018,
        0xbf840003,
        0xbea003c1,
        0xbf008080,
        0xbf820002, // s32=-1, SCC=true
        0xbea00380,
        0xbf008180, // s32=0, SCC=false
        scalar_local_alu(4, 32, 32, 0x80),
        scalar_local_alu(10, 33, 0x81, 0x80),
        0xbea20321,
        0xbf840002,
        0x7e1002f2,
        0xbf820001,
        0x7e100280,
        0x7e120280,
        0xf800180f,
        0x08090908,
        0xbf810000,
    ]);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
    assert!(compiled.shader.ir.contains("<- true"));
    assert!(compiled.shader.ir.contains("<- false"));
    assert!(compiled.shader.ir.contains("<- 0xffffffffu"));
    assert_eq!(compiled.shader.ir.matches("branch ").count(), 2);
}

#[test]
fn scalar_local_diamond_chain_stays_linear_and_unsupported_operations_reject() {
    let mut words = vec![0xbea00380];
    for _ in 0..24 {
        words.extend([
            0xbf008018,
            0xbf840002,
            scalar_local_alu(0, 32, 32, 0x81),
            0xbf820001,
            scalar_local_alu(1, 32, 32, 0x81),
        ]);
    }
    words.push(0xbf068020);
    let mut pixel = graphics_words(&words);
    pixel.extend_from_slice(CONSTANT_PS);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
    assert!(compiled.shader.msl.len() < 50000);
    for prefix in [
        vec![scalar_local_alu(15, 32, 0x81, 0x81)], // unsupported 64-bit AND
        vec![0xbea00381, 0x7e100220],               // unsupported SGPR-to-VGPR move
        vec![0xbf820001, 0xbea00381, 0xbf068120],
    ] {
        // skipped definition
        let mut pixel = graphics_words(&prefix);
        pixel.extend_from_slice(CONSTANT_PS);
        assert!(compile_graphics(Gfx1030, CONSTANT_VS, &pixel).is_err());
    }
}

#[test]
fn signed_decode_does_not_enable_unreachable_graphics_backedges() {
    let mut code = graphics_words(&[0xbf820001, 0xbf82fffe]);
    code.extend_from_slice(CONSTANT_PS);
    let error = compile_graphics(Gfx1030, CONSTANT_VS, &code).unwrap_err();
    assert_stage_error(
        &error,
        "pixel",
        4,
        "counted loop requires a preheader and nonempty body",
    );
}

#[test]
fn scalar_high_multiplication_supports_runtime_operands_and_preserves_unset_scc() {
    for opcode in [53u32, 54] {
        let mut ps = include_bytes!("../../../fixtures/rdna2/x9-scalar-mul-ps.bin").to_vec();
        ps[12..16]
            .copy_from_slice(&(0x80000000 | opcode << 23 | 32 << 16 | 33 << 8 | 32).to_le_bytes());
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap();
        // No previous SCC: later consumer must reject rather than silently initialize it.
        ps[8..12].copy_from_slice(&0xbea20380u32.to_le_bytes());
        assert!(compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).is_err());
    }
}

#[test]
fn scalar_bit_operations_initialize_scc_except_flagless_mask() {
    for opcode in [
        14u32, 16, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 39, 40, 44, 46, 47, 48, 49,
    ] {
        let mut ps = include_bytes!("../../../fixtures/rdna2/x9-scalar-mul-ps.bin").to_vec();
        ps[12..16]
            .copy_from_slice(&(0x80000000 | opcode << 23 | 32 << 16 | 33 << 8 | 32).to_le_bytes());
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap();
        ps[8..12].copy_from_slice(&0xbea20380u32.to_le_bytes());
        let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps);
        assert_eq!(
            compiled.is_ok(),
            opcode != 36,
            "opcode {opcode}: {compiled:?}"
        );
        // Restore SCC so the invalid operand is the sole rejection reason.
        let original = include_bytes!("../../../fixtures/rdna2/x9-scalar-mul-ps.bin");
        ps[8..12].copy_from_slice(&original[8..12]);
        for source_byte in [12, 13] {
            let saved = ps[source_byte];
            ps[source_byte] = 35;
            let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap_err();
            assert_stage_error(&error, "pixel", 12, "scalar local is uninitialized");
            ps[source_byte] = saved;
        }
    }
}

#[test]
fn scalar_unary_graphics_tracks_defined_flags_and_rejects_invalid_operands() {
    for opcode in [7u32, 11, 13, 15, 17, 19, 21, 23, 25, 26, 52] {
        let mut ps = include_bytes!("../../../fixtures/rdna2/x9-scalar-mul-ps.bin").to_vec();
        ps[12..16].copy_from_slice(&(0xbe800000 | 32 << 16 | opcode << 8 | 32).to_le_bytes());
        compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap();
        ps[8..12].copy_from_slice(&0xbea20380u32.to_le_bytes());
        let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps);
        assert_eq!(
            compiled.is_ok(),
            matches!(opcode, 7 | 13 | 15 | 52),
            "opcode {opcode}: {compiled:?}"
        );
        let original = include_bytes!("../../../fixtures/rdna2/x9-scalar-mul-ps.bin");
        ps[8..12].copy_from_slice(&original[8..12]);
        ps[12] = 35;
        let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap_err();
        assert_stage_error(&error, "pixel", 12, "scalar local is uninitialized");
        ps[12] = 32;
        // Input uniforms cannot be overwritten as scalar locals.
        ps[14] = 0x80 | 24;
        let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap_err();
        assert_stage_error(&error, "pixel", 12, "destination");
    }
}

#[test]
fn unary_loop_carried_local_accepts_but_counter_rewrite_rejects() {
    let mut ps = include_bytes!("../../../fixtures/rdna2/x9-loop-scalar-ps.bin").to_vec();
    // Replace the body ADD with NOT s33,s33; original decrement remains intact.
    ps[40..44].copy_from_slice(&0xbea10721u32.to_le_bytes());
    compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).unwrap();
    // Rewriting the bounded induction variable invalidates the termination proof.
    ps[40..44].copy_from_slice(&0xbea00720u32.to_le_bytes());
    assert!(compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &ps).is_err());
}

#[test]
fn scc1_graphics_constant_selection_unknown_and_runtime_complement() {
    // A poisoned read is skipped exactly when SCC1 is taken.
    for (compare, accepted) in [(0xbf06_8080, true), (0xbf06_8180, false)] {
        let pixel: Vec<_> = graphics_words(&[compare, 0xbf85_0001, 0xbf00_8068])
            .into_iter()
            .chain(CONSTANT_PS.iter().copied())
            .collect();
        assert_eq!(
            compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).is_ok(),
            accepted
        );
    }
    let unknown: Vec<_> = graphics_words(&[0xbf85_0000])
        .into_iter()
        .chain(CONSTANT_PS.iter().copied())
        .collect();
    let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &unknown).unwrap_err();
    assert_stage_error(
        &error,
        "pixel",
        0,
        "SCC is uninitialized on the selected graphics path",
    );
    let pixel = graphics_words(&[
        0xbf06_8018, // Compare runtime s24 with zero.
        0x7e12_0280,
        0xbf85_0002,
        0x7e10_02f2,
        0xbf82_0001,
        0x7e10_0280,
        0xf800_180f,
        0x0809_0908,
        0xbf81_0000,
    ]);
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap();
    assert_eq!(compiled.pixel_uniform_mask, 1);
    assert!(compiled.shader.ir.contains("not.bool"));
    assert!(compiled.shader.msl.contains("if ("));
}

#[test]
fn vmcnt_has_no_graphics_admission() {
    let pixel: Vec<_> = graphics_words(&[0xbc7d0000])
        .into_iter()
        .chain(CONSTANT_PS.iter().copied())
        .collect();
    let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    assert_stage_error(&error, "pixel", 0, "instruction is unsupported");
}
