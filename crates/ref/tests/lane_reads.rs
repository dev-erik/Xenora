// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::execute;
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn broadcast_uses_each_guest_wave_first_word_for_partial_dispatches() {
    for len in [1, 2, 31, 32, 33, 63, 65] {
        let input: Vec<u32> = (0..len).map(|i| 0x80000000 + i).collect();
        for read in [vec![0x7e100501], vec![0xbe8903a0, 0xd7600008, 0x02001301]] {
            let mut body = read;
            body.extend([0x7e020289, 0x7e040208]);
            let actual = execute(&code(&body), &input).unwrap();
            let expected: Vec<u32> = (0..len).map(|i| 0x80000000 + (i / 32) * 32).collect();
            assert_eq!(actual, expected);
        }
    }
}
#[test]
fn empty_exec_reads_lane_zero_and_inactive_source_retains_its_word() {
    assert_eq!(
        execute(
            &code(&[
                0xbefe0382, 0x7e020289, 0xbefe0380, 0x7e100501, 0xbefe03c1, 0x7e040208
            ]),
            &[17, 23, 42]
        )
        .unwrap(),
        vec![17; 3]
    );
    assert_eq!(
        execute(
            &code(&[
                0xbe890380, 0xbefe0380, 0xd7600008, 0x02001301, 0xbefe03c1, 0x7e040208
            ]),
            &[17, 23, 42]
        )
        .unwrap(),
        vec![17; 3]
    );
}
#[test]
fn higher_lane_is_rejected_even_for_complete_waves() {
    for input in [vec![5], vec![5; 32]] {
        assert!(
            execute(
                &code(&[0xbefe0382, 0x7e100501, 0xbefe03c1, 0x7e040208]),
                &input
            )
            .is_err()
        );
        assert!(
            execute(
                &code(&[0xbe89039f, 0xd7600008, 0x02001301, 0x7e040208]),
                &input
            )
            .is_err()
        );
    }
}
#[test]
fn overwritten_scalar_constant_and_lane_value_never_leak() {
    assert_eq!(
        execute(&code(&[0x7e100501, 0xbe880389, 0x7e040208]), &[17, 23]).unwrap(),
        vec![9; 2]
    );
    assert!(
        execute(
            &code(&[0xbe880380, 0x7e100501, 0xbe890308, 0x7e040209]),
            &[17, 23]
        )
        .is_err()
    );
    assert!(execute(&code(&[0x7e100501, 0xbefe0308, 0x7e040208]), &[17, 23]).is_err());
    assert_eq!(
        execute(
            &code(&[0xbe880380, 0xd7600008, 0x02001101, 0x7e040208]),
            &[17, 23]
        )
        .unwrap(),
        vec![17; 2]
    );
}
#[test]
fn scalar_conditionals_preserve_or_replace_the_lane_value() {
    for (cmp, expected) in [(0xbf068080, 9), (0xbf068180, 17)] {
        let body = [0x7e100501, cmp, 0xbe880589, 0x7e040208];
        assert_eq!(execute(&code(&body), &[17, 23]).unwrap(), vec![expected; 2]);
    }
}
#[test]
fn pending_and_runtime_region_reads_reject_independent_of_mask_value() {
    assert!(
        execute(
            &code(&[0xe0302000, 0x80000100, 0x7e100501, 0xbf8c3f70, 0x7e040208]),
            &[17]
        )
        .is_err()
    );
    for input in [vec![0; 33], vec![1; 33]] {
        for branch in [0xbf860001, 0xbf870001] {
            assert!(execute(&code(&[0x7d8a0280, branch, 0x7e100501, 0x7e040281]), &input).is_err());
        }
    }
}
