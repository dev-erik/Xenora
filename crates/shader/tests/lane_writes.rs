// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{Architecture::Gfx1030, compile, compile_graphics};
fn bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
#[test]
fn explicit_mask_ignores_exec_and_replays_in_lane_read_cone() {
    for selector in [0, 31, 32, 63] {
        let result = compile(
            Gfx1030,
            &code(&[
                0xbe880387, 0xbe8903ff, selector, 0xbefe0380, 0xd7610001, 0x02001208, 0x7e140501,
                0xbefe03c1,
            ]),
        )
        .unwrap();
        assert!(
            result
                .msl
                .contains(&format!("0x{:08x}u >>", 1u32 << (selector & 31)))
        );
        assert!(result.msl.contains("input[(index & ~31u)]"));
        assert!(!result.msl.contains("simd_"));
    }
}
#[test]
fn runtime_scalar_snapshot_is_supported_but_runtime_index_is_not() {
    assert!(
        compile(
            Gfx1030,
            &code(&[
                0x7e100501, 0xbe890381, 0xbefe0380, 0xd7610001, 0x02001208, 0xbe880389, 0xbefe03c1
            ])
        )
        .is_ok()
    );
    assert!(
        compile(
            Gfx1030,
            &code(&[0x7e120501, 0xbe880387, 0xd7610001, 0x02001208])
        )
        .is_err()
    );
    // Data and index may alias when known; descriptors are not implicit constants.
    assert!(compile(Gfx1030, &code(&[0xbe880380, 0xd7610001, 0x02001008])).is_ok());
    assert!(compile(Gfx1030, &code(&[0xbe890380, 0xd7610001, 0x02001200])).is_err());
}
#[test]
fn unready_destinations_reject_even_exec_zero() {
    for dst in [0, 2] {
        assert!(
            compile(
                Gfx1030,
                &code(&[
                    0xbe880387,
                    0xbe890380,
                    0xbefe0380,
                    0xd7610000 | dst,
                    0x02001208,
                    0xbefe03c1
                ])
            )
            .is_err()
        );
    }
    assert!(
        compile(
            Gfx1030,
            &code(&[
                0xbe880387, 0xbe890380, 0xe0302000, 0x80000100, 0xbefe0380, 0xd7610001, 0x02001208,
                0xbf8c3f70, 0xbefe03c1
            ])
        )
        .is_err()
    );
}
#[test]
fn runtime_regions_and_skipped_graphics_reject() {
    for branch in [0xbf860002, 0xbf870002] {
        assert!(
            compile(
                Gfx1030,
                &code(&[
                    0xbe880387, 0xbe890380, 0x7d8a0280, branch, 0xd7610001, 0x02001208
                ])
            )
            .is_err()
        );
    }
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for skip in [false, true] {
        let mut w = vec![];
        if skip {
            w.push(0xbf820002);
        }
        w.extend([0xd7610001, 0x02001208]);
        for vertex in [false, true] {
            let mut stream = bytes(&w);
            stream.extend_from_slice(if vertex { vs } else { ps });
            let err = if vertex {
                compile_graphics(Gfx1030, &stream, ps)
            } else {
                compile_graphics(Gfx1030, vs, &stream)
            }
            .unwrap_err();
            assert!(err.to_string().contains("lane writes require"));
        }
    }
}

// SRC0 constants only: SRC1 stays an initialized normal SGPR selector.
fn constant_write(src: u32, literal: Option<u32>, dst: u32) -> Vec<u32> {
    let mut words = vec![0xd7610000 | dst, src | 9 << 9 | 128 << 18];
    if let Some(bits) = literal {
        words.push(bits);
    }
    words
}

#[test]
fn constant_data_compiles_in_both_wave_sizes_with_exec_independent_writes() {
    use xenora_shader::{WaveConfig, WaveLaunch, WaveSize, compile_with_wave_config};
    let sources = (128..=208)
        .map(|src| (src, None))
        .chain((240..=248).map(|src| (src, None)))
        .chain([0, 0xffffffff, 0x80000000, 0x7fa12345, 0x01234567].map(|v| (255, Some(v))));
    for (src, literal) in sources {
        for size in [WaveSize::Wave32, WaveSize::Wave64] {
            for selector in [0, 31, 32, 63, 64, 127] {
                for mask in [0, 1, u32::MAX] {
                    // Exhaust sources once per wave size; cross boundary/mask cases
                    // only for representative inline and literal encodings.
                    if !matches!(src, 129 | 255) && (selector != 31 || mask != u32::MAX) {
                        continue;
                    }
                    let mut body = vec![0xbe8903ff, selector, 0xbefe03ff, mask];
                    if size == WaveSize::Wave64 {
                        body.extend([0xbeff03ff, mask]);
                    }
                    body.extend(constant_write(src, literal, 1));
                    body.push(0xbefe03c1);
                    if size == WaveSize::Wave64 {
                        body.push(0xbeff03c1);
                    }
                    let compiled = compile_with_wave_config(
                        Gfx1030,
                        &code(&body),
                        WaveConfig {
                            size,
                            launch: if size == WaveSize::Wave64 {
                                WaveLaunch::FullWavesRequired
                            } else {
                                WaveLaunch::PartialAllowed
                            },
                        },
                    )
                    .unwrap();
                    assert_eq!(compiled.wave_config().size, size);
                }
            }
        }
    }
}

#[test]
fn constant_data_preserves_destination_and_selector_readiness() {
    for literal in [None, Some(0x7fa12345)] {
        let src = if literal.is_some() { 255 } else { 129 };
        for dst in [0, 2] {
            let mut body = vec![0xbe890380, 0xbefe0380];
            body.extend(constant_write(src, literal, dst));
            body.push(0xbefe03c1);
            let err = compile(Gfx1030, &code(&body)).unwrap_err();
            assert!(err.to_string().contains("source register"), "{err}");
        }
        // v1 is pending after a second load, even with EXEC0 during the write.
        let mut body = vec![0xbe890380, 0xe0302000, 0x80000100, 0xbefe0380];
        body.extend(constant_write(src, literal, 1));
        body.extend([0xbf8c3f70, 0xbefe03c1]);
        let err = compile(Gfx1030, &code(&body)).unwrap_err();
        assert!(err.to_string().contains("pending a load wait"), "{err}");
        for setup in [vec![], vec![0x7e120501]] {
            // unset or runtime s9
            let mut body = setup;
            body.extend(constant_write(src, literal, 1));
            assert!(compile(Gfx1030, &code(&body)).is_err());
        }
    }
}

#[test]
fn constant_data_retains_decode_and_runtime_region_exclusions() {
    for src in [
        106, 107, 108, 124, 125, 126, 127, 209, 239, 249, 250, 254, 256, 511,
    ] {
        let mut body = vec![0xbe890380];
        body.extend(constant_write(src, None, 1));
        assert!(matches!(
            compile(Gfx1030, &code(&body)),
            Err(xenora_shader::CompileError::Decode(_))
        ));
    }
    for branch in [0xbf860003, 0xbf870003] {
        let mut body = vec![0xbe890380, 0x7d8a0280, branch];
        body.extend(constant_write(255, Some(0x7fa12345), 1));
        let err = compile(Gfx1030, &code(&body)).unwrap_err();
        assert!(
            matches!(err, xenora_shader::CompileError::Verify { .. }),
            "{err}"
        );
    }
    // Non-register selectors remain rejected even with literal data.
    for lane in [106, 124, 128, 255, 256] {
        let words = [0xd7610001, 255 | lane << 9 | 128 << 18, 0x12345678];
        assert!(matches!(
            compile(Gfx1030, &code(&words)),
            Err(xenora_shader::CompileError::Decode(_))
        ));
    }
}

#[test]
fn literal_write_is_rejected_in_graphics_even_when_skipped() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    for skip in [false, true] {
        let mut prefix = Vec::new();
        if skip {
            prefix.push(0xbf820003);
        }
        prefix.extend(constant_write(255, Some(0x7fa12345), 1));
        for vertex in [false, true] {
            let mut stream = bytes(&prefix);
            stream.extend_from_slice(if vertex { vs } else { ps });
            let err = if vertex {
                compile_graphics(Gfx1030, &stream, ps)
            } else {
                compile_graphics(Gfx1030, vs, &stream)
            }
            .unwrap_err();
            assert!(err.to_string().contains("lane writes require"), "{err}");
        }
    }
}
