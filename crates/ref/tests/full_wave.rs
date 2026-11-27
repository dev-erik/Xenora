// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::{ExecuteOptions, Wave32Launch, execute, execute_with_options};
fn options() -> ExecuteOptions {
    ExecuteOptions {
        wave32_launch: Wave32Launch::FullWavesRequired,
    }
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn all_selected_lanes_and_exec_masks_use_actual_wave_state() {
    let input: Vec<u32> = (100..164).collect();
    for lane in 0..32 {
        let p = code(&[0xbe8903ff, lane + 32, 0xd7600008, 0x02001301, 0x7e040208]);
        let expected: Vec<u32> = (0..64).map(|i| 100 + (i / 32) * 32 + lane).collect();
        assert_eq!(
            execute_with_options(&p, &input, options()).unwrap(),
            expected
        );
        let p = code(&[0xbefe03ff, 1 << lane, 0x7e100501, 0xbefe03c1, 0x7e040208]);
        assert_eq!(
            execute_with_options(&p, &input, options()).unwrap(),
            expected
        );
    }
}
#[test]
fn runtime_selector_and_destination_alias_preserve_snapshot() {
    let mut input: Vec<u32> = (100..164).collect();
    input[0] = 31;
    input[32] = 33;
    let p = code(&[0x7e100501, 0xd7600008, 0x02001101, 0x7e020289, 0x7e040208]);
    assert!(execute(&p, &input).is_err());
    let expected: Vec<u32> = (0..64).map(|i| if i < 32 { 131 } else { 133 }).collect();
    assert_eq!(
        execute_with_options(&p, &input, options()).unwrap(),
        expected
    );
}
#[test]
fn rejects_invalid_launch_lengths_without_inventing_lanes() {
    let p = code(&[0xbe89039f, 0xd7600008, 0x02001301, 0x7e040208]);
    for n in [0, 1, 31, 33, 4095, 4097] {
        assert!(execute_with_options(&p, &vec![9; n], options()).is_err());
    }
    assert_eq!(
        execute_with_options(&p, &vec![9; 4096], options()).unwrap(),
        vec![9; 4096]
    );
}
