// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::{ExecuteOptions, Wave32Launch, execute, execute_with_options};
fn run(body: &[u32], input: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    execute_with_options(
        &code(body),
        input,
        ExecuteOptions {
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
fn highest_active_writer_zero_and_retention_are_explicit() {
    let mut input = vec![0; 32];
    input[..4].copy_from_slice(&[0, 0, 12, 4]);
    // MBCNT gives distinct data, independent of the input addresses.
    let body = [
        0xd7650003,
        193 | 128 << 9 | 128 << 18,
        0x7e040289,
        0xbefe038f,
        0xdac80000,
        0x02000301,
        0xbf8cc07f,
        0xbefe03c1,
    ];
    let mut expected = vec![9; 32];
    expected[..4].copy_from_slice(&[1, 3, 0, 2]);
    assert_eq!(run(&body, &input).unwrap(), expected);
    for mask in [0, 1, 0x80000000, u32::MAX] {
        let body = [
            0xd7650003,
            193 | 128 << 9 | 128 << 18,
            0x7e040289,
            0xbefe03ff,
            mask,
            0xdac80000,
            0x02000301,
            0xbf8cc07f,
            0xbefe03c1,
        ];
        let mut expected = vec![9; 32];
        for (i, v) in expected.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                *v = 0;
            }
        }
        if mask & 1 != 0 {
            expected[0] = 31 - mask.leading_zeros();
        }
        assert_eq!(run(&body, &[0; 32]).unwrap(), expected);
    }
}
#[test]
fn offsets_wrapping_aliases_and_independent_waves_preserve_snapshots() {
    let input: Vec<u32> = (0..64).map(|i| (i % 32) * 4).collect();
    for offset in [0, 1, 3, 4, 127, 128, 0xff04, 0xffff] {
        let body = [0xdac80000 | offset, 0x01000101, 0xbf8cc07f, 0x7e040301];
        let expected: Vec<u32> = (0..64)
            .map(|i| (((i % 32) + 32 - ((offset >> 2) & 31)) % 32) * 4)
            .collect();
        assert_eq!(run(&body, &input).unwrap(), expected);
    }
    assert!(execute(&code(&[0xdac80000, 0x02000101, 0xbf8cc07f]), &input).is_err());
}
#[test]
fn shared_ds_slot_wait_domain_and_readiness_reject() {
    for body in [
        vec![0xdac80000, 0x02000101, 0xbf8c3f70],
        vec![0xdac80000, 0x02000101, 0x7e040289, 0xbf8cc07f],
        vec![0xdac80000, 0x02000101, 0xd8d4001f, 0x03000001, 0xbf8cc07f],
        vec![0xd8d4001f, 0x03000001, 0xdac80000, 0x02000101, 0xbf8cc07f],
        vec![0xdac80000, 0x02000100, 0xbf8cc07f],
        vec![0xdac80000, 0x02000401, 0xbf8cc07f],
        vec![0xbefe0380, 0xdac80000, 0x02000101, 0xbf8cc07f, 0xbefe03c1],
        vec![0x7e040301, 0xdac80000, 0x03000101],
    ] {
        assert!(run(&body, &[0; 32]).is_err(), "{body:x?}");
    }
}
