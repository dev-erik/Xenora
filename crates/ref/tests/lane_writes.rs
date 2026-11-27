// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::execute;
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn selects_low_five_bits_ignores_exec_and_retains_partial_waves() {
    for len in [1, 2, 31, 32, 33, 63, 65] {
        let input: Vec<u32> = (0..len).map(|i| 100 + i).collect();
        for selector in [0, 31, 32, 63, 0xffffffff] {
            for exec in [0, 1, 2, u32::MAX] {
                let program = code(&[
                    0xbe880387, 0xbe8903ff, selector, 0xbefe03ff, exec, 0xd7610001, 0x02001208,
                    0xbefe03c1,
                ]);
                let expected: Vec<u32> = input
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| if i as u32 % 32 == selector & 31 { 7 } else { v })
                    .collect();
                assert_eq!(execute(&program, &input).unwrap(), expected);
            }
        }
    }
}
#[test]
fn runtime_scalar_and_old_destination_are_snapshots() {
    let program = code(&[
        0x7e100501, 0xbe890381, 0xbefe0380, 0xd7610001, 0x02001208, 0xbe880389, 0xbefe03c1,
    ]);
    assert_eq!(execute(&program, &[10, 20, 30]).unwrap(), [10, 10, 30]);
}
#[test]
fn rejects_unready_destinations_and_runtime_selector_even_exec_zero() {
    for dst in [0, 2] {
        assert!(
            execute(
                &code(&[
                    0xbe880387,
                    0xbe890380,
                    0xbefe0380,
                    0xd7610000 | dst,
                    0x02001208,
                    0xbefe03c1
                ]),
                &[1]
            )
            .is_err()
        );
    }
    assert!(
        execute(
            &code(&[0x7e120501, 0xbe880387, 0xd7610001, 0x02001208]),
            &[1]
        )
        .is_err()
    );
    assert!(
        execute(
            &code(&[
                0xbe880387, 0xbe890380, 0xe0302000, 0x80000100, 0xbefe0380, 0xd7610001, 0x02001208,
                0xbf8c3f70, 0xbefe03c1
            ]),
            &[1]
        )
        .is_err()
    );
}

#[test]
fn constant_data_encodings_and_literals_execute_in_both_wave_sizes() {
    use xenora_ref::{WaveConfig, WaveLaunch, WaveSize, execute_with_wave_config};
    let cases = [
        (128, 0),
        (192, 64),
        (193, u32::MAX),
        (208, 0xfffffff0),
        (240, 0x3f000000),
        (241, 0xbf000000),
        (242, 0x3f800000),
        (243, 0xbf800000),
        (244, 0x40000000),
        (245, 0xc0000000),
        (246, 0x40800000),
        (247, 0xc0800000),
        (248, 0x3e22f983),
        (255, 0xdeadbeef),
    ];
    for (size, width) in [(WaveSize::Wave32, 32), (WaveSize::Wave64, 64)] {
        for (source, value) in cases {
            let mut body = vec![
                0xbe8903ff,
                width - 1,
                0xbefe0380,
                0xbeff0380,
                0xd7610001,
                0x02001200 | source,
            ];
            if source == 255 {
                body.push(value);
            }
            body.extend([0xbefe03c1, 0xbeff03c1]);
            let input: Vec<u32> = (0..width * 2).map(|i| 1000 + i).collect();
            let mut expected = input.clone();
            expected[width as usize - 1] = value;
            expected[width as usize * 2 - 1] = value;
            assert_eq!(
                execute_with_wave_config(
                    &code(&body),
                    &input,
                    WaveConfig {
                        size,
                        launch: WaveLaunch::FullWavesRequired
                    }
                )
                .unwrap(),
                expected,
                "source {source}, width {width}"
            );
        }
    }
}
#[test]
fn literal_decode_consumes_third_word_and_rejects_truncation() {
    use xenora_ref::{Op, ScalarSource, decode};
    let bytes: Vec<u8> = [0xd7610001u32, 0x020012ff, 0x12345678, 0xbf810000]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    let decoded = decode(&bytes).unwrap();
    assert_eq!(
        decoded[0].op,
        Op::WriteLane {
            dst: 1,
            src: ScalarSource::Immediate(0x12345678),
            index: 9
        }
    );
    assert_eq!(decoded[1].offset, 12);
    for len in [4, 8, 9, 10, 11] {
        assert!(decode(&bytes[..len]).is_err(), "length {len}");
    }
}
#[test]
fn constant_sources_do_not_relax_selector_modifier_or_destination_contract() {
    use xenora_ref::decode;
    for source in [106, 107, 124, 126, 127, 209, 239, 249, 254, 256] {
        let bytes: Vec<u8> = [0xd7610001u32, 0x02001200 | source]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(decode(&bytes).is_err(), "source {source}");
    }
    for (first, second) in [
        (0xd7618001, 0x02001280),
        (0xd7610101, 0x02001280),
        (0xd7610001, 0x0200d480),
        (0xd7610001, 0x02010080),
        (0xd7610001, 0x02041280),
        (0xd7610001, 0x0a001280),
    ] {
        let bytes: Vec<u8> = [first, second]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        assert!(decode(&bytes).is_err());
    }
    for dst in [0, 2] {
        assert!(
            execute(
                &code(&[
                    0xbe890380,
                    0xbefe0380,
                    0xd7610000 | dst,
                    0x02001280,
                    0xbefe03c1
                ]),
                &[1]
            )
            .is_err()
        );
    }
}

#[test]
fn runtime_selector_uses_guest_width_and_captured_data_in_full_waves() {
    use xenora_ref::{WaveConfig, WaveLaunch, WaveSize, execute_with_wave_config};
    for (size, width) in [(WaveSize::Wave32, 32usize), (WaveSize::Wave64, 64)] {
        for selector in [0, 31, 32, 63, 64, 127, u32::MAX] {
            let mut input: Vec<u32> = (0..width * 2).map(|i| 1000 + i as u32).collect();
            input[0] = selector;
            input[width] = selector.wrapping_add(1);
            // Capture s9 from v1, use that same SGPR for data and selector,
            // then overwrite it. Empty EXEC cannot suppress the write.
            let p = code(&[
                0x7e120501, 0xbefe0380, 0xbeff0380, 0xd7610001, 0x02001209, 0xbe890387, 0xbefe03c1,
                0xbeff03c1,
            ]);
            let mut expected = input.clone();
            for base in [0, width] {
                expected[base + (input[base] as usize & (width - 1))] = input[base];
            }
            assert_eq!(
                execute_with_wave_config(
                    &p,
                    &input,
                    WaveConfig {
                        size,
                        launch: WaveLaunch::FullWavesRequired
                    }
                )
                .unwrap(),
                expected
            );
            assert!(execute(&p, &input).is_err()); // PartialAllowed rejects even a complete population.
        }
    }
}
#[test]
fn runtime_selector_is_not_recomputed_after_its_source_vgpr_changes() {
    use xenora_ref::{WaveConfig, WaveLaunch, WaveSize, execute_with_wave_config};
    for (size, width) in [(WaveSize::Wave32, 32usize), (WaveSize::Wave64, 64)] {
        let mut input: Vec<u32> = (0..width).map(|i| 100 + i as u32).collect();
        input[0] = width as u32; // selects lane zero in either mode
        let p = code(&[0x7e120501, 0xd7610001, 0x02001287, 0xd7610001, 0x02001289]);
        let mut expected = input.clone();
        expected[0] = 9;
        assert_eq!(
            execute_with_wave_config(
                &p,
                &input,
                WaveConfig {
                    size,
                    launch: WaveLaunch::FullWavesRequired
                }
            )
            .unwrap(),
            expected
        );
    }
}
#[test]
fn runtime_selector_retains_destination_readiness_checks_with_empty_exec() {
    use xenora_ref::{WaveConfig, WaveLaunch, WaveSize, execute_with_wave_config};
    for (size, width) in [(WaveSize::Wave32, 32usize), (WaveSize::Wave64, 64)] {
        for dst in [0, 1, 2] {
            let mut b = vec![0x7e120501];
            if dst == 1 {
                b.extend([0xe0302000, 0x80000100]);
            } // pending reload
            b.extend([
                0xbefe0380,
                0xbeff0380,
                0xd7610000 | dst,
                0x02001287,
                0xbf8c3f70,
                0xbefe03c1,
                0xbeff03c1,
            ]);
            assert!(
                execute_with_wave_config(
                    &code(&b),
                    &vec![0; width],
                    WaveConfig {
                        size,
                        launch: WaveLaunch::FullWavesRequired
                    }
                )
                .is_err()
            );
        }
    }
}
