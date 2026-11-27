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
