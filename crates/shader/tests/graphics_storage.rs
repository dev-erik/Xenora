// SPDX-License-Identifier: GPL-3.0-or-later
//! Original bounded-store compiler fixtures, not GPU or independent acceptance.
//! Instructions checked with native ARM64 LLVM; expected behavior derives from
//! AMD RDNA2 plain IMAGE_STORE and the synthetic unique-pixel-address contract.
use xenora_shader::{
    Architecture::Gfx1030, GraphicsPipelineTypes, GraphicsSampleType, GraphicsStorageType, compile,
    compile_graphics_pipeline_typed,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const VARYING_VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
const COLORS: &[u8] = &[
    0xff, 0x02, 0x10, 0x7e, 0x00, 0x00, 0x80, 0x3e, // v8=.25, literal
    0xff, 0x02, 0x12, 0x7e, 0x00, 0x00, 0x00, 0x3f, // v9=.5, literal
    0xff, 0x02, 0x14, 0x7e, 0x00, 0x00, 0x40, 0x3f, // v10=.75, literal
    0xf2, 0x02, 0x16, 0x7e, // v11=1, supported inline selector
];
const RGBA: &[u8] = &[0x08, 0x1f, 0x20, 0xf0, 0x20, 0x08, 0, 0];
const UINT: &[u8] = &[0x08, 0x11, 0x20, 0xf0, 0x20, 0x08, 0, 0];
const EXPORT: &[u8] = &[0x0f, 0x18, 0, 0xf8, 8, 9, 10, 11, 0, 0, 0x81, 0xbf];
const WHITE: &[u8] = &[
    0xf2, 0x02, 0x18, 0x7e, 0x0f, 0x18, 0, 0xf8, 12, 12, 12, 12, 0, 0, 0x81, 0xbf,
];
const UV: &[u8] = &[0, 0, 8, 0xc8, 1, 0, 9, 0xc8, 0, 1, 12, 0xc8, 1, 1, 13, 0xc8];
const SAMPLE1: &[u8] = &[0x08, 0x0f, 0x9c, 0xf0, 2, 8, 0xa3, 0];
fn types(uint: bool) -> GraphicsPipelineTypes {
    let mut t = GraphicsPipelineTypes::default();
    t.textures[0] = if uint {
        GraphicsSampleType::StorageUint
    } else {
        GraphicsSampleType::StorageFloat
    };
    t
}
fn error(pixel: &[u8], t: GraphicsPipelineTypes) -> String {
    compile_graphics_pipeline_typed(Gfx1030, VS, pixel, t)
        .unwrap_err()
        .to_string()
}

#[test]
fn rgba_store_is_a_resultless_ordered_effect_with_explicit_unique_address_metadata() {
    let pixel = [COLORS, RGBA, EXPORT].concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types(false)).unwrap();
    assert_eq!(p.storage, Some(GraphicsStorageType::Float));
    assert!(!p.uses_texture);
    assert_eq!(p.sampled_pairs, 0);
    assert_eq!(p.required_uniform_mask, 0);
    assert!(p.array_layers.iter().all(Vec::is_empty));
    assert!(
        p.shader
            .msl
            .contains("texture2d<float, access::write> storage0 [[texture(0)]]")
    );
    assert!(
        p.shader
            .msl
            .contains("uint2 pixel_coordinates = uint2(pixel_position.xy)")
    );
    assert!(
        p.shader
            .msl
            .contains("as_type<float>(pixel_coordinates[0])")
    );
    assert!(p.shader.msl.contains("storage0.write(float4("));
    assert!(p.shader.msl.contains("uint2(as_type<uint>("));
    let effect = p
        .shader
        .ir
        .lines()
        .find(|line| line.contains("store.2d"))
        .unwrap();
    assert!(!effect.contains(" = "), "store must not define a Value");
    assert!(
        p.shader.msl.find("storage0.write").unwrap()
            < p.shader.msl.find("output.color0 =").unwrap()
    );
}

#[test]
fn uint_store_reinterprets_literal_bits_and_zeros_unused_components() {
    let literal = [0xff, 0x02, 0x10, 0x7e, 0x01, 0x00, 0x00, 0x01]; // v8 bits 0x01000001
    let pixel = [&literal[..], UINT, WHITE].concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types(true)).unwrap();
    assert_eq!(p.storage, Some(GraphicsStorageType::Uint));
    assert!(p.shader.msl.contains("texture2d<uint, access::write>"));
    assert!(p.shader.msl.contains("0x01000001u"));
    assert!(p.shader.msl.contains("storage0.write(uint4(as_type<uint>("));
    assert!(p.shader.msl.contains(", 0u, 0u, 0u)"));
    for bits in [0x7fc00000u32, 0x7f800000, 0xff800000] {
        let mut literal = vec![0xff, 0x02, 0x10, 0x7e];
        literal.extend_from_slice(&bits.to_le_bytes());
        assert!(error(&[&literal[..], UINT, WHITE].concat(), types(true)).contains("finite FP32"));
    }
}

#[test]
fn opaque_uint_sample_slot1_can_feed_uint_store_but_not_rgba_store() {
    let mut t = types(true);
    t.textures[1] = GraphicsSampleType::Uint;
    let pixel = [UV, SAMPLE1, UINT, WHITE].concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VARYING_VS, &pixel, t).unwrap();
    assert_eq!(p.sampled_pairs, 8);
    assert!(p.uses_texture);
    assert_eq!(p.storage, Some(GraphicsStorageType::Uint));
    assert!(p.shader.msl.contains("as_type<float>(tex1.sample"));
    assert!(p.shader.msl.contains("storage0.write(uint4(as_type<uint>("));
    assert!(p.shader.msl.contains("uint2(input.position.xy)"));
    assert!(!p.shader.msl.contains("float4 pixel_position [[position]]"));
    t.textures[0] = GraphicsSampleType::StorageFloat;
    let error = compile_graphics_pipeline_typed(
        Gfx1030,
        VARYING_VS,
        &[UV, SAMPLE1, RGBA, WHITE].concat(),
        t,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("RGBA storage cannot consume sampled integer bits")
    );
}

#[test]
fn only_direct_xy_coordinate_copies_are_accepted() {
    let copies = [0x20, 0x03, 0x50, 0x7e, 0x21, 0x03, 0x52, 0x7e]; // v40=x,v41=y
    let mut store = RGBA.to_vec();
    store[4] = 40;
    let p = compile_graphics_pipeline_typed(
        Gfx1030,
        VS,
        &[COLORS, &copies, &store, EXPORT].concat(),
        types(false),
    )
    .unwrap();
    assert_eq!(p.storage, Some(GraphicsStorageType::Float));
    for copies in [
        [0x21, 0x03, 0x50, 0x7e, 0x20, 0x03, 0x52, 0x7e], // swapped Y/X
        [0x20, 0x41, 0x50, 0x06, 0x21, 0x03, 0x52, 0x7e], // x+x is not original X
        [0x80, 0x02, 0x50, 0x7e, 0x21, 0x03, 0x52, 0x7e], // literal zero is not X
    ] {
        assert!(
            error(&[COLORS, &copies, &store, EXPORT].concat(), types(false))
                .contains("store coordinates")
        );
    }
}

#[test]
fn coordinate_escape_to_data_arithmetic_sample_or_export_rejects() {
    let data = [COLORS, &[0x20, 0x03, 0x10, 0x7e], RGBA, EXPORT].concat();
    assert!(error(&data, types(false)).contains("pixel-coordinate bits"));
    let math = [COLORS, &[0x20, 0x41, 0x50, 0x06], RGBA, EXPORT].concat();
    assert!(error(&math, types(false)).contains("pixel-coordinate bits"));
    let mut exported = [COLORS, RGBA, EXPORT].concat();
    let at = exported.len() - EXPORT.len();
    exported[at + 4] = 32;
    assert!(error(&exported, types(false)).contains("pixel-coordinate bits cannot be exported"));
    let mut t = types(false);
    t.textures[1] = GraphicsSampleType::Float;
    let pixel = [UV, COLORS, &[0x20, 0x03, 0x04, 0x7e], SAMPLE1, RGBA, WHITE].concat();
    let err = compile_graphics_pipeline_typed(Gfx1030, VARYING_VS, &pixel, t).unwrap_err();
    assert!(err.to_string().contains("pixel-coordinate bits"));
}

#[test]
fn used_coordinate_phi_rejects_but_unused_coordinate_phi_is_allowed() {
    let prefix = [
        COLORS,
        &[0x21, 0x03, 0x52, 0x7e],                         // v41=y
        &[0x18, 0x80, 0x06, 0xbf, 0x02, 0x00, 0x84, 0xbf], // runtime condition
        &[0x20, 0x03, 0x50, 0x7e, 0x01, 0x00, 0x82, 0xbf], // v40=x; join
        &[0x21, 0x03, 0x50, 0x7e],                         // v40=y
    ]
    .concat();
    let unused = [&prefix[..], RGBA, EXPORT].concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &unused, types(false)).unwrap();
    assert_eq!(p.required_uniform_mask, 1);
    let mut phi_store = RGBA.to_vec();
    phi_store[4] = 40;
    assert!(
        error(&[&prefix[..], &phi_store, EXPORT].concat(), types(false))
            .contains("used phi/arithmetic")
    );
}

#[test]
fn runtime_branch_keeps_store_inside_selected_block() {
    let pixel = [
        COLORS,
        &[0x18, 0x80, 0x06, 0xbf, 0x02, 0x00, 0x84, 0xbf],
        RGBA,
        EXPORT,
    ]
    .concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types(false)).unwrap();
    assert_eq!(p.required_uniform_mask, 1);
    assert_eq!(p.shader.msl.matches("storage0.write(").count(), 1);
    assert!(
        p.shader.msl.find("switch (block)").unwrap()
            < p.shader.msl.find("storage0.write(").unwrap()
    );
    // Inline 1 == 0 prunes the only store: storage must not claim an effect.
    let mut dead = pixel;
    dead[COLORS.len()] = 0x81;
    assert!(error(&dead, types(false)).contains("reachable store"));
}

#[test]
fn runtime_bounded_zero_trip_loop_keeps_real_store_effect_in_body() {
    let pixel = [
        COLORS,
        &[0x18, 0x03, 0xa0, 0xbe, 0x20, 0x82, 0xa0, 0x83], // s32=min(s24,2)
        &[0x20, 0x80, 0x07, 0xbf, 0x04, 0x00, 0x84, 0xbf], // pretest s32 != 0
        RGBA,
        &[0x20, 0x81, 0xa0, 0x80, 0xfa, 0xff, 0x82, 0xbf], // decrement and latch
        EXPORT,
    ]
    .concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types(false)).unwrap();
    assert_eq!(p.storage, Some(GraphicsStorageType::Float));
    assert_eq!(p.required_uniform_mask, 1);
    assert_eq!(p.shader.msl.matches("storage0.write(").count(), 1);
    assert!(p.shader.msl.contains("while (!finished)"));
    assert!(p.shader.ir.contains("branch"));
}

#[test]
fn repeated_stores_preserve_source_versions_and_order() {
    let second = [0xff, 0x02, 0x10, 0x7e, 0, 0, 0x40, 0x3f]; // v8=.75, literal
    let pixel = [COLORS, RGBA, &second, RGBA, EXPORT].concat();
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types(false)).unwrap();
    let stores: Vec<_> = p
        .shader
        .msl
        .lines()
        .filter(|l| l.contains("storage0.write("))
        .collect();
    assert_eq!(stores.len(), 2);
    assert_ne!(stores[0], stores[1]);
    let first = p.shader.ir.find("store.2d").unwrap();
    let last = p.shader.ir.rfind("store.2d").unwrap();
    assert!(p.shader.ir[first..last].contains("const.f32 0x3f400000"));
}

#[test]
fn storage_roles_masks_descriptor_and_compute_rejections_are_explicit() {
    assert!(error(&[COLORS, UINT, EXPORT].concat(), types(false)).contains("store mask"));
    assert!(error(&[COLORS, RGBA, EXPORT].concat(), types(true)).contains("store mask"));
    assert!(error(&[COLORS, EXPORT].concat(), types(false)).contains("reachable store"));
    let mut other = RGBA.to_vec();
    other[6] = 3; // base s12
    assert!(error(&[COLORS, &other, EXPORT].concat(), types(false)).contains("descriptor base s0"));
    assert!(
        error(
            &[COLORS, RGBA, EXPORT].concat(),
            GraphicsPipelineTypes::default()
        )
        .contains("unsupported")
    );
    let mut t = types(false);
    t.textures[1] = GraphicsSampleType::StorageUint;
    assert!(error(&[COLORS, RGBA, EXPORT].concat(), t).contains("restricted to texture slot0"));
    let mut sample0 = SAMPLE1.to_vec();
    sample0[6] = 0x40;
    let err = compile_graphics_pipeline_typed(
        Gfx1030,
        VARYING_VS,
        &[UV, &sample0, RGBA, WHITE].concat(),
        types(false),
    )
    .unwrap_err();
    assert!(err.to_string().contains("excludes sampling texture0"));
    let err = compile(Gfx1030, &[RGBA, &[0, 0, 0x81, 0xbf]].concat()).unwrap_err();
    assert!(
        err.to_string()
            .contains("graphics instruction is unsupported")
    );
}

#[test]
fn legacy_pipeline_has_no_storage_metadata_or_pixel_coordinate_inputs() {
    let p = compile_graphics_pipeline_typed(
        Gfx1030,
        VS,
        &[COLORS, EXPORT].concat(),
        GraphicsPipelineTypes::default(),
    )
    .unwrap();
    assert_eq!(p.storage, None);
    assert!(!p.shader.msl.contains("pixel_coordinates"));
    assert!(!p.shader.msl.contains("access::write"));
}

#[test]
fn original_storage_fp_loop_places_fp_controls_at_function_body_start() {
    let pixel = include_bytes!("../../../fixtures/rdna2/x9-storage-loop-ps.bin");
    let p = compile_graphics_pipeline_typed(Gfx1030, VS, pixel, types(false)).unwrap();
    assert_eq!(p.storage, Some(GraphicsStorageType::Float));
    assert_eq!(p.required_uniform_mask, 1);
    let fragment = p.shader.msl.split("fragment PixelOutput").nth(1).unwrap();
    let body = fragment.split_once('{').unwrap().1.trim_start();
    assert!(
        body.starts_with("#pragma clang fp contract(off)\n    #pragma clang fp reassociate(off)\n")
    );
    assert!(body.contains("uint2 pixel_coordinates ="));
    assert!(body.contains("storage0.write("));
    assert!(p.shader.ir.contains("add.f32"));
}
