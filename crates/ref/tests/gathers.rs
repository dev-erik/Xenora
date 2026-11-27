// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::{ExecuteOptions, Wave32Launch, execute_with_options};
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
fn swizzle_modes_have_literal_lane_maps() {
    let input: Vec<u32> = (0..64).collect();
    let modes: &[(u16, [u32; 32])] = &[
        (
            0x041f,
            [
                1, 0, 3, 2, 5, 4, 7, 6, 9, 8, 11, 10, 13, 12, 15, 14, 17, 16, 19, 18, 21, 20, 23,
                22, 25, 24, 27, 26, 29, 28, 31, 30,
            ],
        ),
        (
            0x801b,
            [
                3, 2, 1, 0, 7, 6, 5, 4, 11, 10, 9, 8, 15, 14, 13, 12, 19, 18, 17, 16, 23, 22, 21,
                20, 27, 26, 25, 24, 31, 30, 29, 28,
            ],
        ),
        (
            0xc020,
            [
                1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
                24, 25, 26, 27, 28, 29, 30, 31, 0,
            ],
        ),
        (
            0xc420,
            [
                31, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                22, 23, 24, 25, 26, 27, 28, 29, 30,
            ],
        ),
        (
            0xe000,
            [
                0, 16, 8, 24, 4, 20, 12, 28, 2, 18, 10, 26, 6, 22, 14, 30, 1, 17, 9, 25, 5, 21, 13,
                29, 3, 19, 11, 27, 7, 23, 15, 31,
            ],
        ),
    ];
    for &(offset, map) in modes {
        let expected: Vec<u32> = map.into_iter().chain(map.map(|v| v + 32)).collect();
        assert_eq!(
            run(
                &[0xd8d40000 | u32::from(offset), 0x02000001, 0xbf8cc07f],
                &input
            )
            .unwrap(),
            expected
        );
    }
}
#[test]
fn bpermute_snapshots_source_mask_and_retained_destination() {
    let input: Vec<u32> = (0..32).map(|i| 4 * ((i + 1) % 32)).collect();
    for mask in [0, 1, 3, u32::MAX] {
        let body = [
            0x7e040289, 0xbefe03ff, mask, 0xdaccff00, 0x02000101, 0xbf8c3f70, 0xbf8cc07f,
            0xbefe03c1,
        ];
        let expected: Vec<u32> = (0..32)
            .map(|i| {
                if mask & (1 << i) == 0 {
                    9
                } else if mask & (1 << ((i + 1) % 32)) == 0 {
                    0
                } else {
                    input[(i + 1) % 32]
                }
            })
            .collect();
        assert_eq!(run(&body, &input).unwrap(), expected);
    }
}
#[test]
fn permlane_rows_fi_bc_and_destination_alias() {
    let input: Vec<u32> = (0..32).collect();
    for cross in [false, true] {
        for flags in [0, 0x800, 0x1000, 0x1800] {
            let body = [
                0xbe8803ff,
                0x76543210,
                0xbe8903ff,
                0xfedcba98,
                (if cross { 0xd7780001 } else { 0xd7770001 }) | flags,
                257 | 8 << 9 | 9 << 18,
                0x7e040301,
            ];
            let expected: Vec<u32> = (0..32).map(|i| if cross { i ^ 16 } else { i }).collect();
            assert_eq!(run(&body, &input).unwrap(), expected);
        }
    }
}
#[test]
fn nested_gathers_then_readlane_preserve_issue_snapshots() {
    let input: Vec<u32> = (0..32).map(|i| i + 100).collect();
    // swap neighbors then rotate, capture lane31, overwrite the source afterwards.
    let body = [
        0xd8d4041f, 0x02000001, 0xbf8cc07f, 0xd8d4c020, 0x02000002, 0xbf8cc07f, 0xbe89039f,
        0xd7600008, 0x02001302, 0x7e040289, 0x7e040208,
    ];
    assert_eq!(run(&body, &input).unwrap(), vec![101; 32]);
}
#[test]
fn ds_wait_is_independent_and_pending_accesses_reject() {
    let input = vec![5; 32];
    for tail in [
        vec![],
        vec![0xbf8c3f70],
        vec![0x7e040289],
        vec![0xd8d4001f, 0x03000001],
        vec![0xe0302000, 0x80000200],
    ] {
        let mut body = vec![0xd8d4001f, 0x02000001];
        body.extend(tail);
        assert!(run(&body, &input).is_err());
    }
    // VM v3 and DS v2 can be pending independently; LGKM does not release v3.
    assert!(
        run(
            &[
                0xe0302000, 0x80000300, 0xd8d4001f, 0x02000001, 0xbf8cc07f, 0x7e040303
            ],
            &input
        )
        .is_err()
    );
    assert!(
        run(
            &[
                0xe0302000, 0x80000300, 0xd8d4001f, 0x02000001, 0xbf8cc07f, 0xbf8c3f70, 0x7e040303
            ],
            &input
        )
        .is_ok()
    );
}

#[test]
fn masked_rotate_rejects_primary_source_conflict() {
    for immediate in [0xc001, 0xc021, 0xc41f, 0xc7ff, 0xd001, 0xdfff] {
        let body = [0xd8d40000 | immediate, 0x02000001, 0xbf8cc07f];
        let error = run(&body, &[0; 32]).unwrap_err().to_string();
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
        assert!(run(body, &[4; 32]).is_err(), "{body:x?}");
    }
}
