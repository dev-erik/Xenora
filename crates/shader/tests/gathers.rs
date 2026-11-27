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
fn nested_contexts_replay_gathers_in_comparisons_and_lane_reads() {
    let s = build(&[
        0xd8d4041f, 0x02000001, 0xbf8cc07f, 0xd8d4c020, 0x02000002, 0xbf8cc07f, 0x7d8a0480,
        0xbe89039f, 0xd7600008, 0x02001302, 0x7e040208,
    ])
    .unwrap();
    assert!(s.msl.contains("gather_"));
    assert_eq!(s.msl.matches("lane < 32u").count(), 1);
    assert!(s.msl.contains("lane_index >= grid"));
    assert!(!s.msl.contains("simd_"));
    assert!(s.msl.len() <= 65536);
}
#[test]
fn fixture_forms_compile_only_with_full_waves() {
    for fixture in [
        include_bytes!("../../../fixtures/rdna2/wave-bpermute.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/wave-swizzle.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/wave-permlane16.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/wave-permlanex16.bin").as_slice(),
    ] {
        assert!(compile(Gfx1030, fixture).is_err());
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
}
#[test]
fn hazards_exec_and_wait_domains_reject() {
    for tail in [
        vec![],
        vec![0xbf8c3f70],
        vec![0x7e040289],
        vec![0xd8d4001f, 0x03000001],
        vec![0xe0302000, 0x80000200],
    ] {
        let mut body = vec![0xd8d4001f, 0x02000001];
        body.extend(tail);
        assert!(build(&body).is_err());
    }
    assert!(
        build(&[
            0xe0302000, 0x80000300, 0xd8d4001f, 0x02000001, 0xbf8cc07f, 0x7e040303
        ])
        .is_err()
    );
    assert!(
        build(&[
            0xe0302000, 0x80000300, 0xd8d4001f, 0x02000001, 0xbf8cc07f, 0xbf8c3f70, 0x7e040303
        ])
        .is_ok()
    );
    for exec in [0, 1, 0x80000000] {
        assert!(
            build(&[
                0xbefe03ff, exec, 0xd8d4001f, 0x02000001, 0xbf8cc07f, 0xbefe03c1
            ])
            .is_err()
        );
    }
}
#[test]
fn graphics_rejects_gather_and_lgkm_even_when_skipped() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for op in [
        vec![0xd8d4001f, 0x02000001],
        vec![0xdacc0000, 0x02000101],
        vec![0xd7770002, 257 | 8 << 9 | 9 << 18],
        vec![0xbf8cc07f],
    ] {
        let mut words = vec![0xbf820000 | op.len() as u32];
        words.extend(op);
        for vertex in [false, true] {
            let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            bytes.extend_from_slice(if vertex { vs } else { ps });
            let e = if vertex {
                compile_graphics(Gfx1030, &bytes, ps)
            } else {
                compile_graphics(Gfx1030, vs, &bytes)
            }
            .unwrap_err();
            assert!(e.to_string().contains("gathers and LGKM"));
        }
    }
}
#[test]
fn repeated_nested_gathers_fail_deterministically_at_source_budget() {
    let mut body = vec![];
    for _ in 0..75 {
        body.extend([0xd8d4c020, 0x01000001, 0xbf8cc07f]);
    }
    body.push(0x7e040301);
    let a = build(&body).unwrap_err().to_string();
    assert!(
        a.contains("64 KiB") || a.contains("depth exceeds 64"),
        "{a}"
    );
    assert_eq!(a, build(&body).unwrap_err().to_string());
}

#[test]
fn masked_rotate_rejects_primary_source_conflict() {
    for immediate in [0xc001, 0xc021, 0xc41f, 0xc7ff, 0xd001, 0xdfff] {
        let body = [0xd8d40000 | immediate, 0x02000001, 0xbf8cc07f];
        let error = build(&body).unwrap_err().to_string();
        assert!(error.contains("primary source conflict"), "{error}");
    }
}

#[test]
fn pending_termination_runtime_regions_and_scalar_selector_boundaries() {
    let cases: &[&[u32]] = &[
        // A completed output store does not discharge an unused pending DS result.
        &[0x7e040301, 0xd8d4001f, 0x03000001],
        // Runtime VCC entry rejects an outstanding DS result.
        &[
            0x7e040301, 0x7d8a0280, 0xd8d4001f, 0x03000001, 0xbf860000, 0xbf8cc07f,
        ],
        // Runtime VCC regions do not admit gathers, regardless of branch polarity.
        &[
            0x7e040301, 0x7d8a0280, 0xbf860002, 0xd8d4001f, 0x02000001, 0xbf8cc07f,
        ],
        &[
            0x7e040301, 0x7d8a0280, 0xbf870002, 0xd8d4001f, 0x02000001, 0xbf8cc07f,
        ],
        // Runtime scalar values remain outside this initial PERMLANE selector contract.
        &[0x7e100501, 0xbe890380, 0xd7770002, 257 | 8 << 9 | 9 << 18],
        // Reserved v0 is never a gather source or address.
        &[0xdacc0000, 0x02000100, 0xbf8cc07f],
        &[0xd8d4001f, 0x02000000, 0xbf8cc07f],
        // Initialized retained destination is required for partial-EXEC BPERMUTE.
        &[0xbefe0380, 0xdacc0000, 0x02000101, 0xbf8cc07f, 0xbefe03c1],
    ];
    for body in cases {
        assert!(build(body).is_err(), "{body:x?}");
    }
}
