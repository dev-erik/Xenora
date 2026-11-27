// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{
    Architecture::Gfx1030, CompileOptions, Wave32Launch, compile, compile_graphics,
    compile_with_options,
};
fn build(body: &[u32]) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    compile_with_options(
        Gfx1030,
        &code(body),
        CompileOptions {
            wave32_launch: Wave32Launch::FullWavesRequired,
        },
    )
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(body);
    words.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn scatter_loop_is_bounded_and_comparison_and_readlane_replay_compose() {
    let body = [
        0xdac80000, 0x02000101, 0xbf8cc07f, 0x7d8a0480, 0x7e100502, 0x7e040208,
    ];
    let s = build(&body).unwrap();
    assert!(s.msl.contains("< 32u; ++w"));
    assert_eq!(s.msl.matches("lane < 32u").count(), 1);
    assert!(s.msl.len() <= 65536);
    assert!(!s.msl.contains("simd_"));
    assert!(compile(Gfx1030, &code(&body)).is_err());
    let fixture = include_bytes!("../../../fixtures/rdna2/wave-permute.bin");
    assert!(
        compile_with_options(
            Gfx1030,
            fixture,
            CompileOptions {
                wave32_launch: Wave32Launch::FullWavesRequired
            }
        )
        .is_ok()
    );
}
#[test]
fn scatter_dependencies_in_address_and_data_reject_including_gather_paths() {
    for tail in [
        vec![0xdac80000, 0x02000102, 0xbf8cc07f],
        vec![0xdac80000, 0x02000201, 0xbf8cc07f],
        vec![
            0xd8d4001f, 0x03000002, 0xbf8cc07f, 0xdac80000, 0x02000301, 0xbf8cc07f,
        ],
    ] {
        let mut body = vec![0xdac80000, 0x02000101, 0xbf8cc07f];
        body.extend(tail);
        let error = build(&body).unwrap_err().to_string();
        assert!(error.contains("nested scatter"), "{error}");
    }
    assert!(
        build(&[
            0xdac80000, 0x03000101, 0xbf8cc07f, 0xdac80000, 0x02000101, 0xbf8cc07f
        ])
        .is_ok()
    );
}
#[test]
fn scatter_scope_does_not_shadow_outer_gather_context() {
    // Gather in the scatter body, followed by a gather of the scatter result.
    let body = [
        0xd8d4041f, 0x03000001, 0xbf8cc07f, 0xdac80000, 0x02000301, 0xbf8cc07f, 0xd8d4c020,
        0x02000002, 0xbf8cc07f,
    ];
    let s = build(&body).unwrap();
    assert!(s.msl.contains("_s"));
    assert!(s.msl.contains("gather_"));
}
#[test]
fn pending_hazards_and_whole_stream_graphics_reject() {
    for body in [
        vec![0xdac80000, 0x02000101, 0xbf8c3f70],
        vec![0xdac80000, 0x02000101, 0x7e040289, 0xbf8cc07f],
        vec![0xdac80000, 0x02000101, 0xd8d4001f, 0x03000001, 0xbf8cc07f],
        vec![0xd8d4001f, 0x03000001, 0xdac80000, 0x02000101, 0xbf8cc07f],
        vec![0xdac80000, 0x02000100, 0xbf8cc07f],
        vec![0xbefe0380, 0xdac80000, 0x02000101, 0xbf8cc07f, 0xbefe03c1],
    ] {
        assert!(build(&body).is_err(), "{body:x?}");
    }
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let prefix: Vec<u8> = [0xbf820002u32, 0xdac80000, 0x02000101]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    for vertex in [false, true] {
        let mut bytes = prefix.clone();
        bytes.extend_from_slice(if vertex { vs } else { ps });
        let error = if vertex {
            compile_graphics(Gfx1030, &bytes, ps)
        } else {
            compile_graphics(Gfx1030, vs, &bytes)
        }
        .unwrap_err();
        assert!(error.to_string().contains("full Wave32 compute contract"));
    }
}

#[test]
fn existing_non_scatter_comparison_work_keeps_its_acceptance_envelope() {
    let mut body = vec![];
    for _ in 0..8 {
        body.extend([0xd7650001, 257 | 128 << 9 | 128 << 18]);
    }
    body.extend(std::iter::repeat_n(0x7d8a0280, 30));
    body.push(0x7e040301);
    // More than 30 * 32 * 9 replay nodes, plus comparison overhead.
    assert!(compile(Gfx1030, &code(&body)).is_ok());
    body.extend([0xdac80000, 0x02000101, 0xbf8cc07f]);
    let error = build(&body).unwrap_err().to_string();
    assert!(error.contains("weighted work"), "{error}");
}
