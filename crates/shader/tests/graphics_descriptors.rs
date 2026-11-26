// SPDX-License-Identifier: GPL-3.0-or-later
//! Original instruction fixtures for the explicit synthetic four-binding ABI.
//! Words use the public RDNA2 encodings, not a guest descriptor format.
use xenora_shader::{
    Architecture::Gfx1030, CompiledGraphics4, GraphicsArrayLayer, GraphicsExportType,
    GraphicsPipelineTypes, GraphicsPipelineTypes4, GraphicsSampleType,
    compile_graphics_pipeline_typed, compile_graphics_pipeline_typed4,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const UV: [u32; 2] = [0x7e0402f2, 0x7e0602f2]; // v2=v3=1.0
const EXPORT: [u32; 3] = [0xf800180f, 0x0b0a0908, 0xbf810000];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn mov(dst: u8, source: u8) -> u32 {
    0xbe800300 | (u32::from(dst) << 16) | u32::from(source)
}
fn sample(texture: u8, sampler: u8, array: bool) -> [u32; 2] {
    // IMAGE_SAMPLE_LZ, RGBA, DIM1 or DIM5, v[8:11], coords v2 onward.
    [
        if array { 0xf09c0f28 } else { 0xf09c0f08 },
        0x00000802 | (u32::from(texture / 4) << 16) | (u32::from(sampler / 4) << 21),
    ]
}
fn shader(body: &[u32]) -> Vec<u8> {
    bytes(&[&UV[..], body, &EXPORT].concat())
}
fn compile(body: &[u32], types: GraphicsPipelineTypes4) -> CompiledGraphics4 {
    compile_graphics_pipeline_typed4(Gfx1030, VS, &shader(body), types).unwrap()
}
fn rejects(body: &[u32], types: GraphicsPipelineTypes4, message: &str) {
    let e = compile_graphics_pipeline_typed4(Gfx1030, VS, &shader(body), types).unwrap_err();
    assert!(e.to_string().contains(message), "{e}");
}

#[test]
fn all_sixteen_fixed_pairs_and_legacy_mask_remapping() {
    for (t, resource) in [0, 12, 32, 44].into_iter().enumerate() {
        for (s, sampler) in [8, 20, 40, 52].into_iter().enumerate() {
            let body = sample(resource, sampler, false);
            let p = compile(&body, GraphicsPipelineTypes4::default());
            assert_eq!(p.sampled_pairs, 1u16 << (t * 4 + s));
            assert_eq!(p.descriptor_index_uniform_mask, 0);
            assert_eq!(p.required_uniform_mask, 0);
            assert!(p.shader.msl.contains(&format!("tex{t}.sample(samp{s}")));
            let old = compile_graphics_pipeline_typed(
                Gfx1030,
                VS,
                &shader(&body),
                GraphicsPipelineTypes::default(),
            );
            if t < 2 && s < 2 {
                assert_eq!(old.unwrap().sampled_pairs, 1u8 << (t * 2 + s));
            } else {
                assert!(old.is_err());
            }
        }
    }
}

#[test]
fn four_simultaneous_bindings_union_and_exact_declarations() {
    let body = [
        sample(0, 8, false),
        sample(12, 20, false),
        sample(32, 40, false),
        sample(44, 52, false),
    ]
    .concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.sampled_pairs, 0x8421);
    for i in 0..4 {
        assert!(p.shader.msl.contains(&format!("[[texture({i})]]")));
        assert!(p.shader.msl.contains(&format!("[[sampler({i})]]")));
    }
}

#[test]
fn direct_uniform_texture_selector_uses_all_four_candidates() {
    let body = [&[mov(60, 31)][..], &sample(60, 20, false)].concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.sampled_pairs, 0x2222);
    assert_eq!(p.descriptor_index_uniform_mask, 0x80);
    assert_eq!(p.pixel_uniform_mask, 0x80);
    assert_eq!(p.required_uniform_mask, 0x80);
    assert!(p.shader.msl.contains("switch (uniforms[7] * 4u + 1u)"));
    assert!(p.shader.msl.contains("default: __builtin_unreachable();"));
    for t in 0..4 {
        assert!(p.shader.msl.contains(&format!("tex{t}.sample(samp1")));
    }
    assert!(
        compile_graphics_pipeline_typed(
            Gfx1030,
            VS,
            &shader(&body),
            GraphicsPipelineTypes::default()
        )
        .is_err()
    );
}

#[test]
fn independent_uniforms_cartesian_and_same_uniform_diagonal() {
    for (source, pairs, mask) in [(24, 0x8421, 1), (25, 0xffff, 3)] {
        let body = [&[mov(60, 24), mov(68, source)][..], &sample(60, 68, false)].concat();
        let p = compile(&body, GraphicsPipelineTypes4::default());
        assert_eq!(p.sampled_pairs, pairs);
        assert_eq!(p.descriptor_index_uniform_mask, mask);
        assert_eq!(p.required_uniform_mask, mask);
        if source == 24 {
            assert!(!p.shader.msl.contains("tex0.sample(samp1"));
        }
    }
}

#[test]
fn selector_snapshot_survives_later_assignment() {
    let body = [
        &[mov(60, 24)][..],
        &sample(60, 8, false),
        &[mov(60, 31)],
        &sample(60, 52, false),
    ]
    .concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.sampled_pairs, 0x9999);
    assert_eq!(p.descriptor_index_uniform_mask, 0x81);
    assert!(p.shader.msl.contains("switch (uniforms[0] * 4u + 0u)"));
    assert!(p.shader.msl.contains("switch (uniforms[7] * 4u + 3u)"));
}

#[test]
fn runtime_branch_requirements_union_both_selector_sources() {
    // cmp s26,0; SCC0 skips first move/sample/branch to the second arm.
    let body = [
        &[0xbf06801a, 0xbf840004, mov(60, 24)][..],
        &sample(60, 8, false),
        &[0xbf820003, mov(60, 31)],
        &sample(60, 52, false),
    ]
    .concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.sampled_pairs, 0x9999);
    assert_eq!(p.descriptor_index_uniform_mask, 0x81);
    assert_eq!(p.required_uniform_mask, 0x85);
}

#[test]
fn conflicting_used_selector_phi_rejects_equal_provenance_accepts() {
    for source in [24, 25] {
        let body = [
            &[
                0xbf06801a,
                0xbf840002,
                mov(60, 24),
                0xbf820001,
                mov(60, source),
            ][..],
            &sample(60, 8, false),
        ]
        .concat();
        if source == 24 {
            assert_eq!(
                compile(&body, GraphicsPipelineTypes4::default()).descriptor_index_uniform_mask,
                1
            );
        } else {
            rejects(&body, GraphicsPipelineTypes4::default(), "provenance");
        }
    }
}

#[test]
fn reserved_writes_computed_indices_and_missing_selector_reject() {
    for dst in [0, 8, 12, 20, 32, 39, 40, 43, 44, 51, 52, 55, 61, 67, 69, 71] {
        rejects(
            &[&[mov(dst, 24)][..], &sample(0, 8, false)].concat(),
            GraphicsPipelineTypes4::default(),
            "reserved",
        );
    }
    for instruction in [mov(60, 128), mov(60, 56), 0x803c8118] {
        rejects(
            &[&[instruction][..], &sample(60, 8, false)].concat(),
            GraphicsPipelineTypes4::default(),
            "reserved",
        );
    }
    rejects(
        &sample(60, 8, false),
        GraphicsPipelineTypes4::default(),
        "provenance",
    );
    rejects(
        &sample(0, 68, false),
        GraphicsPipelineTypes4::default(),
        "provenance",
    );
}

#[test]
fn legacy_s32_scratch_survives_and_extended_reserves_it() {
    let body = [&[mov(32, 24)][..], &sample(0, 8, false)].concat();
    assert!(
        compile_graphics_pipeline_typed(
            Gfx1030,
            VS,
            &shader(&body),
            GraphicsPipelineTypes::default()
        )
        .is_ok()
    );
    rejects(&body, GraphicsPipelineTypes4::default(), "reserved");
}

#[test]
fn homogeneous_types_dimensions_and_storage_exclusion() {
    let body = [&[mov(60, 24)][..], &sample(60, 8, false)].concat();
    let mut t = GraphicsPipelineTypes4::default();
    t.textures[3] = GraphicsSampleType::Uint;
    rejects(&body, t, "homogeneous");
    t.textures = [GraphicsSampleType::FloatArray; 4];
    rejects(&body, t, "dimension");
    t.textures = [GraphicsSampleType::Float; 4];
    t.textures[0] = GraphicsSampleType::StorageFloat;
    rejects(&body, t, "storage");
    for i in 1..4 {
        let mut t = GraphicsPipelineTypes4::default();
        t.textures[i] = GraphicsSampleType::StorageUint;
        rejects(&body, t, "slot0");
    }
}

#[test]
fn indexed_arrays_propagate_constant_and_uniform_layer_requirements() {
    let t = GraphicsPipelineTypes4 {
        textures: [GraphicsSampleType::FloatArray; 4],
        ..Default::default()
    };
    for (layer, requirement, mask) in [
        (
            vec![0x7e0802ff, 0x40000000],
            GraphicsArrayLayer::Constant(2),
            1,
        ),
        (vec![0x7e08021f], GraphicsArrayLayer::Uniform(7), 0x81),
    ] {
        let body = [&[mov(60, 24)][..], &layer, &sample(60, 8, true)].concat();
        let p = compile(&body, t);
        assert_eq!(p.descriptor_index_uniform_mask, 1);
        assert_eq!(p.required_uniform_mask, mask);
        for layers in p.array_layers {
            assert_eq!(layers, vec![requirement]);
        }
    }
}

#[test]
fn sampler_indexing_preserves_fixed_uint_texture_bits() {
    let mut t = GraphicsPipelineTypes4::default();
    t.textures[2] = GraphicsSampleType::Uint;
    t.colors[0] = GraphicsExportType::Uint;
    let body = [&[mov(68, 25)][..], &sample(32, 68, false)].concat();
    let p = compile(&body, t);
    assert_eq!(p.sampled_pairs, 0x0f00);
    assert_eq!(p.descriptor_index_uniform_mask, 2);
    assert!(p.shader.msl.contains("as_type<float>(tex2.sample(samp3"));
}

#[test]
fn bounded_loop_uses_s56_scratch_and_preserves_selector_provenance() {
    // s56=min(s24,2); s60=s25; header cmp; sample; decrement; backedge.
    let body = [
        &[
            0x83b88218,
            mov(60, 25),
            0x7e1002f2,
            0x7e1202f2,
            0x7e1402f2,
            0x7e1602f2,
            0xbf078038,
            0xbf840004,
        ][..],
        &sample(60, 8, false),
        &[0x80b88138, 0xbf82fffa],
    ]
    .concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.descriptor_index_uniform_mask, 2);
    assert_eq!(p.required_uniform_mask, 3);
    assert_eq!(p.sampled_pairs, 0x1111);
    assert!(p.shader.msl.contains("while (!finished)"));
}

#[test]
fn unused_conflicting_selector_phi_does_not_add_index_requirements() {
    let body = [
        &[0xbf06801a, 0xbf840002, mov(60, 24), 0xbf820001, mov(60, 25)][..],
        &sample(0, 8, false),
    ]
    .concat();
    let p = compile(&body, GraphicsPipelineTypes4::default());
    assert_eq!(p.descriptor_index_uniform_mask, 0);
    assert_eq!(p.sampled_pairs, 1);
}

#[test]
fn storage_slot0_allows_fixed_texture1_with_dynamic_sampler() {
    let mut t = GraphicsPipelineTypes4::default();
    t.textures[0] = GraphicsSampleType::StorageFloat;
    let body = [
        &[mov(68, 24)][..],
        &sample(12, 68, false),
        &[0xf0201f08, 0x00000820],
    ]
    .concat();
    let p = compile(&body, t);
    assert_eq!(p.storage, Some(xenora_shader::GraphicsStorageType::Float));
    assert_eq!(p.sampled_pairs, 0x00f0);
    assert_eq!(p.descriptor_index_uniform_mask, 1);
    assert!(p.shader.msl.contains("storage0.write("));
}

#[test]
fn dynamic_uint_and_depth_types_keep_their_sample_interpretation() {
    let body = [&[mov(60, 24)][..], &sample(60, 8, false)].concat();
    for kind in [GraphicsSampleType::Uint, GraphicsSampleType::Depth] {
        let mut t = GraphicsPipelineTypes4 {
            textures: [kind; 4],
            ..Default::default()
        };
        if kind == GraphicsSampleType::Uint {
            t.colors[0] = GraphicsExportType::Uint;
        }
        let p = compile(&body, t);
        assert_eq!(p.sampled_pairs, 0x1111);
        if kind == GraphicsSampleType::Uint {
            assert!(p.shader.msl.contains("as_type<float>(tex3.sample"));
        } else {
            assert!(p.shader.msl.contains("depth2d<float, access::sample> tex3"));
        }
    }
}
