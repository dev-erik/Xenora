// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal fixtures based on AMD RDNA2 Tables 41, 43 and 100/101.
//! Native ARM64 llvm-mc checked instruction encodings; these are compiler tests,
//! not GPU or independent acceptance evidence. Runtime binding validation is separate.
use xenora_shader::{
    Architecture::Gfx1030, GraphicsArrayLayer, GraphicsExportType, GraphicsPipelineTypes,
    GraphicsSampleType, compile_graphics_pipeline_typed,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
const UV: &[u8] = &[
    0x00, 0x00, 0x08, 0xc8, 0x01, 0x00, 0x09, 0xc8, 0x00, 0x01, 0x0c, 0xc8, 0x01, 0x01, 0x0d, 0xc8,
];
const ARRAY0: &[u8] = &[0x28, 0x0f, 0x9c, 0xf0, 0x02, 0x08, 0x40, 0x00];
const ARRAY1: &[u8] = &[0x28, 0x0f, 0x9c, 0xf0, 0x02, 0x08, 0xa3, 0x00];
const END: &[u8] = &[0x0f, 0x18, 0, 0xf8, 8, 9, 10, 11, 0, 0, 0x81, 0xbf];
// Force literal FP32 2.0: inline selector 244 is outside the decoder subset.
// llvm-mc: v_mov_b32_e32 v4, lit(0x40000000).
const LAYER2: &[u8] = &[0xff, 0x02, 0x08, 0x7e, 0x00, 0x00, 0x00, 0x40];
const UNIFORM31: &[u8] = &[0x1f, 0x02, 0x08, 0x7e];
fn types() -> GraphicsPipelineTypes {
    let mut t = GraphicsPipelineTypes::default();
    t.textures[0] = GraphicsSampleType::FloatArray;
    t
}
fn program(layer: &[u8]) -> Vec<u8> {
    [UV, layer, ARRAY0, END].concat()
}

#[test]
fn constant_and_each_direct_uniform_have_exact_validation_metadata() {
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &program(LAYER2), types()).unwrap();
    assert_eq!(
        compiled.array_layers,
        [vec![GraphicsArrayLayer::Constant(2)], vec![]]
    );
    assert_eq!(compiled.required_uniform_mask, 0);
    assert_eq!(compiled.sampled_pairs, 1);
    assert!(
        compiled
            .shader
            .msl
            .contains("texture2d_array<float, access::sample>")
    );
    assert!(compiled.shader.msl.contains(", uint("));
    assert!(compiled.shader.ir.contains("sample_lz.2d_array.f32"));
    for slot in 0..8 {
        let layer = [24 + slot, 0x02, 0x08, 0x7e]; // v_mov_b32 v4, s24+slot
        let compiled =
            compile_graphics_pipeline_typed(Gfx1030, VS, &program(&layer), types()).unwrap();
        assert_eq!(
            compiled.array_layers,
            [vec![GraphicsArrayLayer::Uniform(slot)], vec![]]
        );
        assert_eq!(compiled.vertex_uniform_mask, 0);
        assert_eq!(compiled.pixel_uniform_mask, 1 << slot);
        assert_eq!(compiled.required_uniform_mask, 1 << slot);
        assert!(
            compiled
                .shader
                .msl
                .contains(&format!("as_type<float>(uniforms[{slot}])"))
        );
    }
}

#[test]
fn constant_range_is_checked_before_u32_conversion_but_view_bounds_are_deferred() {
    for bits in [
        0xbf800000u32,
        0x3f000000,
        0x7f800000,
        0xff800000,
        0x7fc00000,
        0x4f800000,
    ] {
        let mut layer = vec![0xff, 0x02, 0x08, 0x7e];
        layer.extend_from_slice(&bits.to_le_bytes());
        assert!(
            compile_graphics_pipeline_typed(Gfx1030, VS, &program(&layer), types()).is_err(),
            "{bits:08x}"
        );
    }
    for (bits, expected) in [(0x80000000u32, 0), (0x4f7fffff, 4294967040)] {
        let mut layer = vec![0xff, 0x02, 0x08, 0x7e];
        layer.extend_from_slice(&bits.to_le_bytes());
        let compiled =
            compile_graphics_pipeline_typed(Gfx1030, VS, &program(&layer), types()).unwrap();
        assert_eq!(
            compiled.array_layers[0],
            [GraphicsArrayLayer::Constant(expected)]
        );
    }
}

#[test]
fn array_uint_preserves_bits_and_dimension_and_depth_mismatches_reject() {
    let mut t = types();
    t.textures[0] = GraphicsSampleType::UintArray;
    assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &program(LAYER2), t).is_err());
    t.colors[0] = GraphicsExportType::Uint;
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &program(LAYER2), t).unwrap();
    assert!(
        compiled
            .shader
            .msl
            .contains("texture2d_array<uint, access::sample>")
    );
    assert!(compiled.shader.msl.contains("as_type<float>(tex0.sample"));
    assert!(compiled.shader.msl.contains("as_type<uint4>"));
    for kind in [
        GraphicsSampleType::Float,
        GraphicsSampleType::Uint,
        GraphicsSampleType::Depth,
    ] {
        t.textures[0] = kind;
        assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &program(LAYER2), t).is_err());
    }
    let mut pixel = program(LAYER2);
    pixel[UV.len() + LAYER2.len()] = 0x08; // DIM1 cannot bind an array type.
    assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).is_err());
}

#[test]
fn computed_interpolated_local_scalar_and_uninitialized_layers_reject() {
    for layer in [
        vec![],
        [LAYER2, &[0x04, 0x09, 0x08, 0x06]].concat(), // v_add_f32 v4,v4,v4
        vec![0x00, 0x02, 0x10, 0xc8, 0x01, 0x02, 0x11, 0xc8], // attr0.z -> v4
        vec![0x20, 0x02, 0x08, 0x7e],                 // s32 is not a direct uniform
        vec![0x17, 0x02, 0x08, 0x7e],                 // sampled descriptor register
    ] {
        assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &program(&layer), types()).is_err());
    }
}

fn diamond() -> Vec<u8> {
    [
        UV,
        &[0x1a, 0x80, 0x06, 0xbf, 0x05, 0x00, 0x84, 0xbf], // s26==0; else byte44
        LAYER2,
        ARRAY0,
        &[0x03, 0x00, 0x82, 0xbf], // join byte56
        UNIFORM31,
        ARRAY1,
        END,
    ]
    .concat()
}
#[test]
fn runtime_branches_union_layers_and_constant_dead_branch_prunes_uniform() {
    let mut t = types();
    t.textures[1] = GraphicsSampleType::FloatArray;
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &diamond(), t).unwrap();
    assert_eq!(compiled.sampled_pairs, 9);
    assert_eq!(compiled.required_uniform_mask, 0x84);
    assert_eq!(
        compiled.array_layers,
        [
            vec![GraphicsArrayLayer::Constant(2)],
            vec![GraphicsArrayLayer::Uniform(7)]
        ]
    );
    let mut dead = diamond();
    dead[16] = 0x80; // compare inline zero with zero
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &dead, types()).unwrap();
    assert_eq!(compiled.sampled_pairs, 1);
    assert_eq!(compiled.required_uniform_mask, 0);
    assert!(compiled.array_layers[1].is_empty());
}

#[test]
fn phi_layer_rejects_instead_of_guessing_a_source() {
    let pixel = [
        UV,
        &[0x1a, 0x80, 0x06, 0xbf, 0x03, 0x00, 0x84, 0xbf], // else byte36
        LAYER2,
        &[0x01, 0x00, 0x82, 0xbf], // join byte40
        UNIFORM31,
        ARRAY0,
        END,
    ]
    .concat();
    let error = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap_err();
    assert!(error.to_string().contains("phi layers"), "{error}");
}

#[test]
fn loop_samples_track_four_destinations_and_deduplicate_requirements() {
    let pixel = [
        UV,
        LAYER2,
        ARRAY0,
        &[
            0x81, 0x03, 0xa0, 0xbe, // s32=1 (byte32)
            0x20, 0x80, 0x07, 0xbf, // header byte36: s32 != 0
            0x04, 0x00, 0x84, 0xbf,
        ], // exit byte60
        ARRAY0,
        &[0x20, 0x81, 0xa0, 0x80, 0xfa, 0xff, 0x82, 0xbf],
        END,
    ]
    .concat();
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
    assert_eq!(compiled.array_layers[0], [GraphicsArrayLayer::Constant(2)]);
    assert!(compiled.shader.msl.contains("while ("));
}

#[test]
fn uniform_float_move_cannot_escape_layer_contract() {
    let mut pixel = program(UNIFORM31);
    // Export the uniform layer itself even though it also has valid sample metadata.
    let export = pixel.len() - END.len();
    pixel[export + 4] = 4;
    assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).is_err());
    // An otherwise unused direct uniform move must not expand the legacy ABI.
    let mut pixel = program(LAYER2);
    pixel.splice(UV.len()..UV.len(), [0x1f, 0x02, 0x0a, 0x7e]); // v5=s31
    assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).is_err());
}

#[test]
fn same_texture_branch_requirements_union_and_vgpr_copy_preserves_uniform_source() {
    let mut pixel = diamond();
    pixel[54] = 0x40; // second branch also samples tex0/samp0
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
    assert_eq!(compiled.sampled_pairs, 1);
    assert_eq!(
        compiled.array_layers[0],
        [
            GraphicsArrayLayer::Constant(2),
            GraphicsArrayLayer::Uniform(7)
        ]
    );
    let layer = [0x1f, 0x02, 0x0a, 0x7e, 0x05, 0x03, 0x08, 0x7e]; // v5=s31; v4=v5
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, &program(&layer), types()).unwrap();
    assert_eq!(compiled.array_layers[0], [GraphicsArrayLayer::Uniform(7)]);
    assert_eq!(compiled.required_uniform_mask, 0x80);
}

#[test]
fn legacy_two_d_has_empty_array_metadata_and_unchanged_source_contract() {
    let pixel = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
    let compiled =
        compile_graphics_pipeline_typed(Gfx1030, VS, pixel, GraphicsPipelineTypes::default())
            .unwrap();
    assert_eq!(compiled.array_layers, [vec![], vec![]]);
    assert!(
        compiled
            .shader
            .msl
            .contains("texture2d<float, access::sample>")
    );
    assert!(!compiled.shader.msl.contains("texture2d_array"));
    assert!(!compiled.shader.msl.contains("Host precondition"));
}

#[test]
fn original_branch_fixture_preserves_both_uniform_layer_requirements() {
    let pixel = include_bytes!("../../../fixtures/rdna2/x9-array-branch-ps.bin");
    let compiled = compile_graphics_pipeline_typed(Gfx1030, VS, pixel, types())
        .unwrap_or_else(|error| panic!("original array branch fixture: {error:?}\n{error}"));
    assert_eq!(compiled.sampled_pairs, 1);
    assert_eq!(compiled.vertex_uniform_mask, 0);
    assert_eq!(compiled.pixel_uniform_mask, 0x83);
    assert_eq!(compiled.required_uniform_mask, 0x83);
    assert_eq!(compiled.array_layers[0].len(), 2);
    assert!(compiled.array_layers[0].contains(&GraphicsArrayLayer::Uniform(0)));
    assert!(compiled.array_layers[0].contains(&GraphicsArrayLayer::Uniform(7)));
    assert!(compiled.array_layers[1].is_empty());
}
