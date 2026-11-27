// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::Wave32Launch::FullWavesRequired;
use xenora_shader::{
    Architecture::Gfx1030, WaveConfig, WaveLaunch, WaveSize, compile, compile_with_wave_config,
};
fn program(body: &[u32]) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(body);
    words.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn config() -> WaveConfig {
    WaveConfig {
        size: WaveSize::Wave64,
        launch: FullWavesRequired,
    }
}
#[test]
fn configuration_and_legacy_variant_import() {
    let code = program(&[0x7e040301]);
    let shader = compile_with_wave_config(Gfx1030, &code, config()).unwrap();
    assert_eq!(shader.wave_config(), config());
    assert_eq!(shader.launch_policy(), WaveLaunch::FullWavesRequired);
    assert_eq!(shader.wave32_launch(), FullWavesRequired);
    for n in [0, 1, 32, 63, 65, 4097] {
        assert!(shader.validate_launch(n).is_err());
    }
    for n in [64, 128, 192, 4096] {
        assert!(shader.validate_launch(n).is_ok());
    }
    assert!(shader.msl.find("grid & 63u").unwrap() < shader.msl.find("input[index]").unwrap());
    assert!(
        compile_with_wave_config(
            Gfx1030,
            &code,
            WaveConfig {
                size: WaveSize::Wave64,
                launch: WaveLaunch::PartialAllowed
            }
        )
        .is_err()
    );
    assert_eq!(
        compile(Gfx1030, &code).unwrap().wave_config(),
        WaveConfig::default()
    );
}
#[test]
fn wave32_initial_exec_high_is_unknown_but_explicit_write_is_readable() {
    assert!(compile(Gfx1030, &program(&[0x7e04027f])).is_err());
    assert!(compile(Gfx1030, &program(&[0xbeff0381, 0x7e04027f])).is_ok());
}
#[test]
fn original_readlane_alias_rejected_before_skipping() {
    let code = program(&[0xbf820003, 0xd7600009, 0x02001301, 0x7e040301]);
    assert!(
        compile_with_wave_config(Gfx1030, &code, config())
            .unwrap_err()
            .to_string()
            .contains("overlap")
    );
}
fn build(body: &[u32]) -> xenora_shader::CompiledShader {
    compile_with_wave_config(Gfx1030, &program(body), config()).unwrap()
}
fn mov(dst: u32, value: u32) -> [u32; 2] {
    [0xbe8003ff | dst << 16, value]
}
#[test]
fn comparison_halves_are_separate_and_display_is_contextual() {
    let s = build(&[0x7d840301, 0x7e04026b]);
    assert_eq!(s.msl.matches("for (uint lane = 0u; lane < 32u").count(), 1);
    assert_eq!(s.msl.matches("for (uint lane = 32u; lane < 64u").count(), 1);
    assert!(
        s.disassembly.contains("v_cmp_eq_u32_e32 vcc, v1, v1"),
        "{}",
        s.disassembly
    );
    assert!(s.disassembly.contains("v_mov_b32_e32 v2, vcc_hi"));
    let old = compile(Gfx1030, &program(&[0x7d840301, 0x7e04026a])).unwrap();
    assert!(old.disassembly.contains("v_cmp_eq_u32_e32 vcc_lo,"));
}
#[test]
fn wave32_comparison_high_stays_unknown_even_under_empty_exec() {
    for empty in [false, true] {
        let mut b = mov(107, 7).to_vec();
        if empty {
            b.extend(mov(126, 0));
        }
        b.push(0x7d840301);
        b.extend(mov(126, u32::MAX));
        b.push(0x7e04026b);
        assert!(compile(Gfx1030, &program(&b)).is_err());
        b.splice(b.len() - 1..b.len() - 1, mov(107, 9));
        assert!(compile(Gfx1030, &program(&b)).is_ok());
    }
}
#[test]
fn runtime_mask_half_writes_preserve_other_projection_and_join() {
    for dst in [106, 107] {
        for branch in [0xbf860001, 0xbf870001] {
            let mut b = vec![0x7e040301, 0x7d840280];
            b.extend(mov(dst, 0));
            b.extend([branch, 0x7e040289, 0x7d840480]);
            let s = build(&b);
            assert!(s.ir.contains("join.vcc"));
            assert!(s.msl.contains(" | "));
            assert_eq!(s.msl.matches("for (uint lane").count(), 4);
        }
    }
}
#[test]
fn unknown_high_mask_and_runtime_scalar_alu_are_rejected() {
    for branch in [0xbf860001, 0xbf870001] {
        for lo in [0, 1] {
            let mut b = mov(106, lo).to_vec();
            b.extend([branch, 0x7e040289]);
            assert!(
                compile_with_wave_config(Gfx1030, &program(&b), config())
                    .unwrap_err()
                    .to_string()
                    .contains("VCC_HI")
            );
        }
    }
    // Runtime VCC cannot be fed to scalar move/ALU, including its own half.
    for dst in [8, 106, 107, 126, 127] {
        let b = [0x7d840301, 0xbe800300 | dst << 16 | 107, 0x7e040301];
        assert!(
            compile_with_wave_config(Gfx1030, &program(&b), config())
                .unwrap_err()
                .to_string()
                .contains("runtime scalar ALU")
        );
    }
}
#[test]
fn comparison_alias_rejects_original_source_in_skipped_and_empty_exec_code() {
    for half in [106, 107] {
        for prefix in [vec![0xbf820001], vec![0xbefe0380, 0xbeff0380]] {
            let mut b = prefix;
            b.push(0x7d840200 | half);
            b.extend([0xbefe03c1, 0xbeff03c1, 0x7e040301]);
            assert!(
                compile_with_wave_config(Gfx1030, &program(&b), config())
                    .unwrap_err()
                    .to_string()
                    .contains("overlap")
            );
        }
    }
}
#[test]
fn readfirst_high_exec_and_readlane_snapshot_use_six_bits() {
    for (hi, selected) in [(1, 32), (0x80000000, 63), (0, 0)] {
        let mut b = mov(126, 0).to_vec();
        b.extend(mov(127, hi));
        b.push(0x7e100501);
        b.extend(mov(126, u32::MAX));
        b.extend(mov(127, u32::MAX));
        b.push(0x7e040208);
        let s = build(&b);
        assert!(s.ir.contains(&format!("const.u32 0x{selected:08x}")));
        assert!(s.msl.contains("index & ~63u"));
    }
    // Lane-derived selector is captured before the destination SGPR is overwritten.
    let s = build(&[0x7e120501, 0xd7600008, 0x02001301, 0xbe890380, 0x7e040208]);
    assert!(s.msl.contains(" & 63u)"));
    assert!(!s.msl.contains("simd_"));
}
#[test]
fn mbcnt_boundaries_and_writelane_use_guest_lane64() {
    let s = build(&[
        0xd7650002,
        193 | 128 << 9 | 128 << 18,
        0xd7660002,
        193 | 258 << 9 | 128 << 18,
    ]);
    assert!(s.msl.contains(">= 32u ? 0xffffffffu"));
    assert!(s.msl.contains("<= 32u ? 0u"));
    assert!(s.msl.contains("index & 63u"));
    assert!(s.msl.contains("& 31u"));
    // WRITE lane63 ignores empty EXEC; initialized destination remains for others.
    let s = build(&[
        0x7e040301,
        0xbe880389,
        0xbe8903bf,
        0xbefe0380,
        0xbeff0380,
        0xd7610002,
        8 | 9 << 9 | 128 << 18,
        0xbefe03c1,
        0xbeff03c1,
    ]);
    assert!(s.ir.contains("mask=0x8000000000000000"));
}
#[test]
fn ds_half_routing_survives_nested_gather_scatter_and_comparison() {
    let s = build(&[
        0xd8d4041f, 0x03000001, 0xbf8cc07f, 0xdac80000, 0x02000301, 0xbf8cc07f, 0xd8d4c020,
        0x02000002, 0xbf8cc07f,
    ]);
    assert!(s.msl.contains(" & 32u) | w"));
    assert!(s.msl.contains("index & ~63u"));
    assert!(s.msl.contains(" & 31u]"));
    assert!(s.msl.contains("_s"));
    assert!(s.msl.len() <= 65536);
}
#[test]
fn full_exec_and_distinct_pending_ds_stay_required() {
    for b in [
        vec![0xbeff0380, 0x7e040301],
        vec![0xdacc0000, 0x02000101, 0xbf8c3f70],
        vec![0xd8d4001f, 0x02000001, 0x7e040289, 0xbf8cc07f],
    ] {
        assert!(compile_with_wave_config(Gfx1030, &program(&b), config()).is_err());
    }
}

#[test]
fn wave64_scatter_comparison_charges_both_halves_against_work_cap() {
    let e = compile_with_wave_config(
        Gfx1030,
        &program(&[0xdac80000, 0x02000101, 0xbf8cc07f, 0x7d840480]),
        config(),
    )
    .unwrap_err();
    assert!(e.to_string().contains("weighted work exceeds 8192"));
    let s = build(&[0xd8d4041f, 0x02000001, 0xbf8cc07f, 0x7d840480]);
    assert!(s.msl.contains("lane = 32u"));
}

#[test]
fn skipped_graphics_new_mask_vector_sources_fail_closed() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for source in [107, 126, 127] {
        for vertex in [false, true] {
            let mut code: Vec<u8> = [0xbf820001u32, 0x7e040200 | source]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            code.extend_from_slice(if vertex { vs } else { ps });
            let result = if vertex {
                xenora_shader::compile_graphics(Gfx1030, &code, ps)
            } else {
                xenora_shader::compile_graphics(Gfx1030, vs, &code)
            };
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("mask halves require compute")
            );
        }
    }
}
