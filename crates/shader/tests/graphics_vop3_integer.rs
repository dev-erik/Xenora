// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent VOP3 admission/provenance checks; no native arithmetic claims.
use xenora_shader::{
    Architecture::Gfx1030, GraphicsExportType, GraphicsPipelineTypes, GraphicsSampleType,
    compile_graphics_pipeline_typed,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const OPS: [u32; 65] = [
    837,
    333,
    369,
    361,
    362,
    364,
    330,
    877,
    881,
    867,
    868,
    328,
    334,
    882,
    376,
    329,
    838,
    839,
    879,
    338,
    339,
    341,
    342,
    344,
    345,
    265,
    266,
    267,
    268,
    273,
    274,
    275,
    276,
    286,
    293,
    294,
    295,
    282,
    278,
    280,
    283,
    284,
    285,
    439,
    440,
    441,
    442,
    443,
    265 | 0x8000,
    267 | 0x8000,
    293 | 0x8000,
    294 | 0x8000,
    295 | 0x8000,
    322,
    323,
    346,
    347,
    348,
    349,
    322 | 0x8000,
    323 | 0x8000,
    346 | 0x8000,
    347 | 0x8000,
    348 | 0x8000,
    349 | 0x8000,
];
fn arity(op: u32) -> usize {
    let op = op & 1023;
    match op {
        439..=443 => 1,
        265 | 266 | 267 | 268 | 273 | 274 | 275 | 276 | 286 | 293 | 294 | 295 | 282 | 278 | 280
        | 283 | 284 | 285 | 361 | 362 | 364 | 867 | 868 => 2,
        _ => 3,
    }
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn vop(op: u32, dst: u32, a: u32, b: u32) -> u32 {
    op << 25 | dst << 17 | b << 9 | a
}
fn operation(op: u32, dst: u32, position: usize, source: u32) -> Vec<u32> {
    let mut raw = [128; 3];
    raw[..arity(op)].fill(129);
    raw[position] = source;
    let mut words = vec![
        0xd4000000 | (op & 1023) << 16 | (op & 0x8000) | dst,
        raw[0] | raw[1] << 9 | raw[2] << 18,
    ];
    if source == 255 {
        words.push(0x7fc12345);
    }
    words
}
fn prefix() -> Vec<u32> {
    vec![0x7e180280, vop(37, 8, 24, 12), vop(37, 9, 25, 12)]
}
fn finish(mut words: Vec<u32>, result: u32) -> Vec<u8> {
    words.extend([0xf800180f, result * 0x01010101, 0xbf810000]);
    bytes(&words)
}
fn types() -> GraphicsPipelineTypes {
    let mut t = GraphicsPipelineTypes::default();
    t.colors[0] = GraphicsExportType::Uint;
    t
}
fn rejects(vertex: &[u8], pixel: &[u8], t: GraphicsPipelineTypes, reason: &str) {
    let error = compile_graphics_pipeline_typed(Gfx1030, vertex, pixel, t).unwrap_err();
    assert!(
        error.to_string().contains(reason),
        "expected {reason}: {error}"
    );
}
#[test]
fn every_operation_position_reads_isolated_uniform_and_accepts_source_classes() {
    for op in OPS {
        for position in 0..arity(op) {
            // No prefix uniform reads can hide a missed source dependency (especially SRC2).
            let baseline = finish(operation(op, 10, position, 129), 10);
            assert_eq!(
                compile_graphics_pipeline_typed(Gfx1030, VS, &baseline, types())
                    .unwrap()
                    .required_uniform_mask,
                0
            );
            for (source, mask) in [
                (24, 1),
                (31, 128),
                (128, 0),
                (192, 0),
                (193, 0),
                (208, 0),
                (240, 0),
                (241, 0),
                (242, 0),
                (243, 0),
                (244, 0),
                (245, 0),
                (246, 0),
                (247, 0),
                (248, 0),
                (255, 0),
            ] {
                let pixel = finish(operation(op, 10, position, source), 10);
                assert_eq!(
                    compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types())
                        .unwrap()
                        .required_uniform_mask,
                    mask,
                    "op={op} position={position} source={source}"
                );
            }
            for local in [32, 105] {
                for (source, mask) in [(31, 128), (129, 0)] {
                    let mut code = vec![0xbe800300 | local << 16 | source];
                    code.extend(operation(op, 10, position, local));
                    assert_eq!(
                        compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types())
                            .unwrap()
                            .required_uniform_mask,
                        mask
                    );
                }
            }
            for dst in [8, 10, 255] {
                let mut code = prefix();
                code.extend(operation(op, dst, position, 264));
                assert_eq!(
                    compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, dst), types())
                        .unwrap()
                        .required_uniform_mask,
                    3
                );
            }
        }
    }
}
#[test]
fn every_position_rejects_descriptors_vcc_unready_and_reserved_pixel_destinations() {
    for op in OPS {
        for position in 0..arity(op) {
            for (source, reason) in [
                (0, "other SGPRs"),
                (23, "other SGPRs"),
                (106, "other SGPRs"),
                (32, "scalar local is uninitialized"),
                (105, "scalar local is uninitialized"),
                (256, "VGPR is uninitialized or reserved"),
                (257, "VGPR is uninitialized or reserved"),
                (291, "VGPR is uninitialized or reserved"),
            ] {
                let mut code = prefix();
                code.extend(operation(op, 10, position, source));
                rejects(VS, &finish(code, 10), types(), reason);
            }
            for source in [107, 108, 112, 124, 125, 126, 127, 249, 254] {
                let mut code = prefix();
                code.extend(operation(op, 10, position, source));
                assert!(
                    compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types())
                        .is_err()
                );
            }
            for dst in [0, 1] {
                let mut code = prefix();
                code.extend(operation(op, dst, position, 264));
                rejects(
                    VS,
                    &finish(code, dst),
                    types(),
                    "pixel VGPRs 0 and 1 are reserved",
                );
            }
        }
    }
}
#[test]
fn every_position_preserves_unset_true_and_false_scc() {
    for op in OPS {
        for position in 0..arity(op) {
            let mut code = prefix();
            code.extend(operation(op, 10, position, 264));
            code.push(0xbf840000);
            rejects(VS, &finish(code, 10), types(), "SCC is uninitialized");
            for flag in [false, true] {
                let mut code = prefix();
                code.push(if flag { 0xbf068080 } else { 0xbf068180 });
                code.extend(operation(op, 10, position, 264));
                code.push(0xbf840001);
                if flag {
                    code.push(0xbf820001);
                }
                code.push(0x7e000280); // A changed SCC reaches a forbidden pixel write.
                compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).unwrap();
            }
        }
    }
}
#[test]
fn typed_uint_positive_float_export_and_arithmetic_negative() {
    for op in OPS {
        for position in 0..arity(op) {
            let mut code = prefix();
            code.extend(operation(op, 10, position, 264));
            let pixel = finish(code.clone(), 10);
            compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
            rejects(VS, &pixel, GraphicsPipelineTypes::default(), "integer");
            code.push(vop(3, 11, 266, 12));
            rejects(VS, &finish(code, 11), types(), "integer bits");
        }
    }
}
#[test]
fn uniform_float_layer_cannot_enter_any_source_position() {
    let vertex = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let original = include_bytes!("../../../fixtures/rdna2/x9-array-uniform-ps.bin");
    let mut t = GraphicsPipelineTypes::default();
    t.textures[0] = GraphicsSampleType::FloatArray;
    compile_graphics_pipeline_typed(Gfx1030, vertex, original, t).unwrap();
    for op in OPS {
        for position in 0..arity(op) {
            let split = original.len() - 12;
            let mut pixel = original[..split].to_vec();
            pixel.extend(bytes(&operation(op, 12, position, 260)));
            pixel.extend_from_slice(&original[split..]);
            rejects(vertex, &pixel, t, "only as validated array layers");
        }
    }
}
#[test]
fn integer_texture_coordinates_storage_payloads_and_pixel_coordinate_inputs_reject() {
    let vertex = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let original = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
    compile_graphics_pipeline_typed(Gfx1030, vertex, original, GraphicsPipelineTypes::default())
        .unwrap();
    let initial = vec![0x7e1002f2, 0x7e120280, 0x7e140280, 0x7e160280, 0x7e1802f2];
    let mut t = GraphicsPipelineTypes::default();
    t.textures[0] = GraphicsSampleType::StorageFloat;
    let store = [0xf0201f08, 0x00000820];
    let mut valid = initial.clone();
    valid.extend(store);
    compile_graphics_pipeline_typed(Gfx1030, VS, &finish(valid, 12), t).unwrap();
    for op in OPS {
        for position in 0..arity(op) {
            let mut sample = original[..16].to_vec();
            sample.extend(bytes(&operation(op, 2, position, 258)));
            sample.extend_from_slice(&original[16..]);
            rejects(
                vertex,
                &sample,
                GraphicsPipelineTypes::default(),
                "cannot be floating texture coordinates",
            );
            for (dst, source, reason) in [
                (8, 264, "RGBA storage cannot consume"),
                (13, 288, "pixel-coordinate bits may only"),
                (13, 289, "pixel-coordinate bits may only"),
            ] {
                let mut code = initial.clone();
                code.extend(operation(op, dst, position, source));
                code.extend(store);
                rejects(VS, &finish(code, 12), t, reason);
            }
        }
    }
}
#[test]
fn vertex_position_parameter_and_float_arithmetic_keep_integer_provenance() {
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let varying = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let varying_ps = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
    compile_graphics_pipeline_typed(
        Gfx1030,
        varying,
        varying_ps,
        GraphicsPipelineTypes::default(),
    )
    .unwrap();
    for op in OPS {
        for position in 0..arity(op) {
            let mut safe = bytes(&operation(op, 8, position, 256));
            safe.extend_from_slice(VS);
            compile_graphics_pipeline_typed(Gfx1030, &safe, ps, GraphicsPipelineTypes::default())
                .unwrap();
            let mut pos = bytes(&operation(op, 0, position, 256));
            pos.extend_from_slice(VS);
            rejects(&pos, ps, GraphicsPipelineTypes::default(), "integer bits");
            let mut param = bytes(&operation(op, 4, position, 260));
            param.extend_from_slice(varying);
            rejects(
                &param,
                varying_ps,
                GraphicsPipelineTypes::default(),
                "integer bits",
            );
            let mut words = operation(op, 8, position, 256);
            words.push(vop(3, 0, 264, 1));
            let mut float = bytes(&words);
            float.extend_from_slice(varying);
            rejects(
                &float,
                varying_ps,
                GraphicsPipelineTypes::default(),
                "integer bits",
            );
        }
    }
}
#[test]
fn branch_and_loop_phis_preserve_integer_provenance_and_alias_writes() {
    for op in OPS {
        for position in 0..arity(op) {
            // Runtime branch skips exactly the two-word VOP3 write.
            let mut branch = vec![0x7e100280, 0x7e180280, 0xbf078018, 0xbf840002];
            branch.extend(operation(op, 8, position, 129));
            // Clamp a uniform trip count to four, then carry the aliased destination.
            let mut counted = vec![0x83a08418, 0x7e180280, 0x7e100280, 0xbf078020, 0xbf840004];
            counted.extend(operation(op, 8, position, 264));
            counted.extend([0x80a08120, 0xbf82fffa]);
            for code in [branch, counted] {
                let pixel = finish(code, 8);
                compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
                rejects(VS, &pixel, GraphicsPipelineTypes::default(), "integer");
            }
        }
    }
}
