// SPDX-License-Identifier: GPL-3.0-or-later
//! Independently encoded VOP1 admission/provenance checks, not native result evidence.
use xenora_shader::{
    Architecture::Gfx1030, GraphicsExportType, GraphicsPipelineTypes,
    compile_graphics_pipeline_typed,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn vop(op: u32, dst: u32, a: u32, b: u32) -> u32 {
    op << 25 | dst << 17 | b << 9 | a
}
fn prefix() -> Vec<u32> {
    vec![0x7e180280, vop(37, 8, 24, 12), vop(37, 9, 25, 12)]
}
fn finish(mut code: Vec<u32>, result: u32) -> Vec<u8> {
    code.extend([0xf800180f, result * 0x01010101, 0xbf810000]);
    bytes(&code)
}
fn types() -> GraphicsPipelineTypes {
    let mut t = GraphicsPipelineTypes::default();
    t.colors[0] = GraphicsExportType::Uint;
    t
}

// AMD RDNA2 VOP1: NOT, BFREV, FFBH_U, FFBL, FFBH_I, respectively.
const OPS: [u32; 5] = [55, 56, 57, 58, 59];
fn unary(op: u32, dst: u32, source: u32) -> u32 {
    0x7e000000 | dst << 17 | op << 9 | source
}

#[test]
fn all_unary_forms_accept_runtime_words_and_aliases() {
    for opcode in OPS {
        let baseline =
            compile_graphics_pipeline_typed(Gfx1030, VS, &finish(prefix(), 8), types()).unwrap();
        assert_eq!(baseline.required_uniform_mask, 3);
        for dst in [8, 10, 255] {
            let mut code = prefix();
            code.push(unary(opcode, dst, 264));
            let compiled =
                compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, dst), types()).unwrap();
            assert_eq!(compiled.required_uniform_mask, 3);
        }
    }
}

#[test]
fn vector_unary_scalar_and_literal_sources_preserve_admission_checks() {
    for opcode in OPS {
        for source in [
            24, 25, 31, 128, 192, 193, 208, 240, 241, 242, 243, 244, 245, 246, 247, 248, 255,
        ] {
            let mut code = prefix();
            code.push(unary(opcode, 10, source));
            if source == 255 {
                code.push(0x7fc12345);
            } // Integer payload, not FP arithmetic.
            compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).unwrap();
        }
        for source in [
            0,
            23,
            32,
            105,
            106,
            107,
            108,
            112,
            124,
            125,
            126,
            127,
            249,
            254,
            256 + 35,
        ] {
            let mut code = prefix();
            code.push(unary(opcode, 10, source));
            assert!(
                compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).is_err()
            );
        }
        for dst in [0, 1] {
            let mut code = prefix();
            code.push(unary(opcode, dst, 264));
            assert!(
                compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, dst), types()).is_err()
            );
        }
        for local in [32, 105] {
            // s_mov_b32 local, literal: initialized normal SGPR boundaries.
            let mut code = vec![0xbe8003ff | local << 16, 0x7fc12345];
            code.extend(prefix());
            code.push(unary(opcode, 10, local));
            compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).unwrap();
        }
    }
}

#[test]
fn integer_words_cannot_escape_to_float_exports_or_float_arithmetic() {
    for opcode in OPS {
        let mut code = prefix();
        code.push(unary(opcode, 10, 264));
        let pixel = finish(code.clone(), 10);
        compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
        assert!(
            compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, GraphicsPipelineTypes::default())
                .is_err()
        );
        code.push(vop(3, 11, 266, 12));
        assert!(compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 11), types()).is_err());
    }
}

#[test]
fn direct_uniform_layer_carriers_cannot_be_laundered_into_integer_arithmetic() {
    for opcode in OPS {
        use xenora_shader::GraphicsSampleType;
        let vertex = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let original = include_bytes!("../../../fixtures/rdna2/x9-array-uniform-ps.bin");
        let mut array_types = GraphicsPipelineTypes::default();
        array_types.textures[0] = GraphicsSampleType::FloatArray;
        compile_graphics_pipeline_typed(Gfx1030, vertex, original, array_types).unwrap();
        let split = original.len() - 12;
        let mut pixel = original[..split].to_vec();
        pixel.extend_from_slice(&unary(opcode, 12, 256 + 4).to_le_bytes());
        pixel.extend_from_slice(&original[split..]);
        let error =
            compile_graphics_pipeline_typed(Gfx1030, vertex, &pixel, array_types).unwrap_err();
        assert!(
            error.to_string().contains("only as validated array layers"),
            "{error}"
        );
    }
}

#[test]
fn vector_unary_operations_do_not_initialize_scc() {
    for opcode in OPS {
        let mut code = prefix();
        code.push(unary(opcode, 10, 264));
        code.push(0xbf840000);
        let error =
            compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).unwrap_err();
        assert!(
            error.to_string().contains("SCC is uninitialized"),
            "{error}"
        );
    }
}

#[test]
fn vertex_integer_temporaries_cannot_escape_through_position_exports() {
    for opcode in OPS {
        let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
        let mut safe = bytes(&[unary(opcode, 8, 256)]);
        safe.extend_from_slice(VS);
        compile_graphics_pipeline_typed(Gfx1030, &safe, ps, GraphicsPipelineTypes::default())
            .unwrap();
        let mut unsafe_export = bytes(&[unary(opcode, 0, 256)]);
        unsafe_export.extend_from_slice(VS);
        assert!(
            compile_graphics_pipeline_typed(
                Gfx1030,
                &unsafe_export,
                ps,
                GraphicsPipelineTypes::default()
            )
            .is_err()
        );
    }
}

#[test]
fn vector_unary_operations_preserve_both_existing_scc_values() {
    for opcode in OPS {
        for flag in [false, true] {
            let mut code = prefix();
            code.push(if flag { 0xbf068080 } else { 0xbf068180 });
            code.push(unary(opcode, 10, 264));
            code.push(0xbf840001);
            if flag {
                code.push(0xbf820001);
            }
            // Only an incorrectly changed SCC would reach this forbidden write.
            code.push(0x7e000280);
            compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 10), types()).unwrap();
        }
    }
}

#[test]
fn integer_provenance_survives_mixed_branch_and_loop_phis() {
    for opcode in OPS {
        for program in [
            vec![
                0x7e100280,
                0x7e180280,
                0xbf078018,
                0xbf840001,
                unary(opcode, 8, 129),
            ],
            vec![
                0x83a08418,
                0x7e180280,
                0x7e100280,
                0xbf078020,
                0xbf840003,
                unary(opcode, 8, 264), // loop-carried alias
                0x80a08120,
                0xbf82fffb,
            ],
        ] {
            let pixel = finish(program, 8);
            compile_graphics_pipeline_typed(Gfx1030, VS, &pixel, types()).unwrap();
            let error = compile_graphics_pipeline_typed(
                Gfx1030,
                VS,
                &pixel,
                GraphicsPipelineTypes::default(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("integer"), "{error}");
        }
    }
}

#[test]
fn integer_coordinates_and_storage_payloads_cannot_escape_their_contracts() {
    for opcode in OPS {
        use xenora_shader::GraphicsSampleType;
        let vertex = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let original = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
        compile_graphics_pipeline_typed(
            Gfx1030,
            vertex,
            original,
            GraphicsPipelineTypes::default(),
        )
        .unwrap();
        let mut sample = original[..16].to_vec();
        sample.extend_from_slice(&unary(opcode, 2, 258).to_le_bytes());
        sample.extend_from_slice(&original[16..]);
        let error = compile_graphics_pipeline_typed(
            Gfx1030,
            vertex,
            &sample,
            GraphicsPipelineTypes::default(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("cannot be floating texture coordinates"),
            "{error}"
        );

        let initial = vec![0x7e1002f2, 0x7e120280, 0x7e140280, 0x7e160280, 0x7e1802f2];
        let mut storage_types = GraphicsPipelineTypes::default();
        storage_types.textures[0] = GraphicsSampleType::StorageFloat;
        let store = [0xf0201f08, 0x00000820];
        let mut valid = initial.clone();
        valid.extend(store);
        compile_graphics_pipeline_typed(Gfx1030, VS, &finish(valid, 12), storage_types).unwrap();
        for (operation, reason) in [
            (unary(opcode, 8, 264), "RGBA storage cannot consume"),
            (unary(opcode, 13, 288), "pixel-coordinate bits may only"),
        ] {
            let mut code = initial.clone();
            code.push(operation);
            code.extend(store);
            let error =
                compile_graphics_pipeline_typed(Gfx1030, VS, &finish(code, 12), storage_types)
                    .unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
    }
}

#[test]
fn integer_vertex_parameters_and_float_arithmetic_are_rejected() {
    for opcode in OPS {
        let varying = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
        compile_graphics_pipeline_typed(Gfx1030, varying, ps, GraphicsPipelineTypes::default())
            .unwrap();
        for prefix in [
            vec![unary(opcode, 4, 260)],
            vec![unary(opcode, 8, 256), vop(3, 0, 264, 1)],
        ] {
            let mut vertex = bytes(&prefix);
            vertex.extend_from_slice(varying);
            let error = compile_graphics_pipeline_typed(
                Gfx1030,
                &vertex,
                ps,
                GraphicsPipelineTypes::default(),
            )
            .unwrap_err();
            assert!(error.to_string().contains("integer bits"), "{error}");
        }
    }
}
