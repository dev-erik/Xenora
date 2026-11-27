// SPDX-License-Identifier: GPL-3.0-or-later
//! Original counted-loop fixtures; native literal pixels are separate evidence.
use xenora_shader::{Architecture::Gfx1030, compile_graphics_pipeline};

const CONSTANT_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const LOOP_VGPR_PS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-loop-vgpr-ps.bin");

#[test]
fn original_counted_uniform_loop_compiles_with_carried_vgpr_and_real_iteration() {
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, LOOP_VGPR_PS)
        .expect("original MIN-bounded countdown with a carried VGPR must compile");
    assert_eq!(compiled.vertex_uniform_mask, 0);
    assert_eq!(compiled.pixel_uniform_mask, 1);
    assert_eq!(compiled.required_uniform_mask, 1);
    assert_eq!(compiled.color_targets, 1);
    assert_eq!(compiled.sampled_pairs, 0);
    assert!(!compiled.uses_texture);
    // Require an actual iterative construct, not four copies of the body.
    // Pixel correctness and termination are checked by independent later cases.
    let tokens: Vec<_> = compiled
        .shader
        .msl
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .collect();
    assert!(
        tokens.contains(&"while"),
        "counted loop must emit real iteration"
    );
}

const CONSTANT_PS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
const VARYING_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");

#[test]
fn original_counted_loop_vertex_geometry_preserves_stage_uniform_contract() {
    let vertex = include_bytes!("../../../fixtures/rdna2/x9-loop-geometry-vs.bin");
    let compiled = compile_graphics_pipeline(Gfx1030, vertex, CONSTANT_PS).unwrap();
    assert_eq!(compiled.vertex_uniform_mask, 1);
    assert_eq!(compiled.pixel_uniform_mask, 0);
    assert_eq!(compiled.required_uniform_mask, 1);
}

#[test]
fn original_counted_loop_scalar_swap_and_diamond_acceptance() {
    // Runtime oracles to be checked independently by native tests:
    // scalar n=0,1,2,3,4: 3,8,13,18,23;
    // swap even=(7,11), odd=(11,7);
    // diamond n=0,1,2,3,4: 0,.125,.375,.5,.625.
    let cases: &[(&[u8], u8)] = &[
        (
            include_bytes!("../../../fixtures/rdna2/x9-loop-scalar-ps.bin"),
            3,
        ),
        (
            include_bytes!("../../../fixtures/rdna2/x9-loop-scalar-reversed-min-ps.bin"),
            3,
        ),
        (
            include_bytes!("../../../fixtures/rdna2/x9-loop-swap-ps.bin"),
            7,
        ),
        (
            include_bytes!("../../../fixtures/rdna2/x9-loop-diamond-ps.bin"),
            1,
        ),
    ];
    for (pixel, mask) in cases {
        let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, pixel).unwrap();
        assert_eq!(compiled.vertex_uniform_mask, 0);
        assert_eq!(compiled.pixel_uniform_mask, *mask);
        assert_eq!(compiled.required_uniform_mask, *mask);
        assert_eq!(compiled.sampled_pairs, 0);
    }
}

#[test]
fn original_counted_loop_runtime_resource_union_and_constant_zero_pruning() {
    let live = include_bytes!("../../../fixtures/rdna2/x9-loop-sampled-ps.bin");
    let compiled = compile_graphics_pipeline(Gfx1030, VARYING_VS, live).unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
    assert_eq!(compiled.sampled_pairs, 0x09);
    assert!(compiled.uses_texture);
    let dead = include_bytes!("../../../fixtures/rdna2/x9-loop-zero-sampled-ps.bin");
    let compiled = compile_graphics_pipeline(Gfx1030, VARYING_VS, dead).unwrap();
    assert_eq!(compiled.required_uniform_mask, 0);
    assert_eq!(compiled.sampled_pairs, 0);
    assert!(!compiled.uses_texture);
    let dead_read = include_bytes!("../../../fixtures/rdna2/x9-loop-zero-dead-ps.bin");
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, dead_read).unwrap();
    assert_eq!(
        compiled.required_uniform_mask, 0,
        "dead s31 read must not require uniforms"
    );
}

macro_rules! reject_loop_fixture {
    ($test:ident, $fixture:literal) => {
        #[test]
        fn $test() {
            let pixel = include_bytes!(concat!("../../../fixtures/rdna2/", $fixture, ".bin"));
            let error = compile_graphics_pipeline(Gfx1030, VARYING_VS, pixel)
                .expect_err("unsupported or unsafe countdown must fail admission");
            match error {
                xenora_shader::CompileError::GraphicsStage {
                    stage: "pixel",
                    error,
                } => {
                    assert!(
                        matches!(*error, xenora_shader::CompileError::Verify { .. }),
                        "fixture must reach graphics verification, not fail decoding: {error:?}"
                    );
                }
                other => panic!("expected pixel verification failure: {other:?}"),
            }
        }
    };
}

reject_loop_fixture!(
    counted_loop_rejects_header_counter_reset,
    "x9-loop-reject-header-reset-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_decrement_from_stale_initial_counter,
    "x9-loop-reject-stale-decrement-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_compare_of_stale_alias,
    "x9-loop-reject-stale-compare-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_backedge_only_initialization,
    "x9-loop-reject-missing-entry-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_body_only_definition_after_zero_trip,
    "x9-loop-reject-missing-zero-trip-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_unbounded_uniform_count,
    "x9-loop-reject-unbounded-ps"
);
#[test]
fn counted_loop_accepts_signed_positive_guard_with_unsigned_min_bound() {
    // Historical rejection fixture is now inside the explicit signed-positive contract.
    let compiled = compile_graphics_pipeline(
        Gfx1030,
        CONSTANT_VS,
        include_bytes!("../../../fixtures/rdna2/x9-loop-reject-signed-ps.bin"),
    )
    .unwrap();
    assert_eq!(compiled.required_uniform_mask, 1);
}
reject_loop_fixture!(
    counted_loop_rejects_decrement_that_can_wrap,
    "x9-loop-reject-wrap-step-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_extra_counter_write,
    "x9-loop-reject-counter-overwrite-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_export_inside_cycle,
    "x9-loop-reject-export-body-ps"
);
reject_loop_fixture!(
    counted_loop_rejects_entry_into_second_interpolation_phase,
    "x9-loop-reject-interpolation-entry-ps"
);

#[test]
fn constant_zero_loop_still_requires_full_stream_decoding() {
    let mut pixel = include_bytes!("../../../fixtures/rdna2/x9-loop-zero-dead-ps.bin").to_vec();
    // Original source has four initialization words, CMP, then SCC0 exit.
    // Replace the first (unreachable) body MOV at byte 24 without shifting targets.
    pixel[24..28].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
    let error = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &pixel).unwrap_err();
    match error {
        xenora_shader::CompileError::GraphicsStage {
            stage: "pixel",
            error,
        } => {
            assert!(
                matches!(*error, xenora_shader::CompileError::Decode(_)),
                "{error:?}"
            );
        }
        other => panic!("expected pixel full-stream decode failure: {other:?}"),
    }
}

#[test]
fn scc1_countdown_requires_complementary_header_comparison() {
    let mut words: Vec<_> = LOOP_VGPR_PS
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b))
        .collect();
    let branch = words
        .iter()
        .position(|w| w & 0xffff_0000 == 0xbf84_0000)
        .unwrap();
    words[branch] ^= 0x0001_0000;
    let encode = |words: &[u32]| {
        words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>()
    };
    assert!(compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &encode(&words)).is_err());
    // unsigned != zero / SCC0 becomes unsigned == zero / SCC1.
    assert_eq!(words[branch - 1] & 0xffff_0000, 0xbf07_0000);
    words[branch - 1] ^= 0x0001_0000;
    let compiled = compile_graphics_pipeline(Gfx1030, CONSTANT_VS, &encode(&words)).unwrap();
    assert_eq!(compiled.pixel_uniform_mask, 1);
    assert!(compiled.shader.msl.contains("while"));
}
