// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_ref::{WaveConfig, WaveLaunch, WaveSize, execute_with_wave_config};
fn config() -> WaveConfig {
    WaveConfig {
        size: WaveSize::Wave64,
        launch: WaveLaunch::FullWavesRequired,
    }
}
#[test]
fn launch_contract() {
    let code = include_bytes!("../../../fixtures/rdna2/add.bin");
    for n in [0, 1, 32, 63, 65, 4097] {
        assert!(execute_with_wave_config(code, &vec![1; n], config()).is_err());
    }
    for n in [64, 128, 192] {
        assert!(execute_with_wave_config(code, &vec![1; n], config()).is_ok());
    }
    assert!(
        execute_with_wave_config(
            code,
            &[1; 64],
            WaveConfig {
                size: WaveSize::Wave64,
                launch: WaveLaunch::PartialAllowed
            }
        )
        .is_err()
    );
}
fn code(body: &[u32]) -> Vec<u8> {
    let mut w = vec![0xe0302000u32, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn run(body: &[u32], input: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    execute_with_wave_config(&code(body), input, config())
}
fn mov(s: u32, value: u32) -> [u32; 2] {
    [0xbe8003ff | s << 16, value]
}
fn mb(high: bool, dst: u32, a: u32, b: u32) -> [u32; 2] {
    [
        0xd4000000 | (if high { 870 } else { 869 }) << 16 | dst,
        a | b << 9 | 128 << 18,
    ]
}
#[test]
fn legacy_variant_import_and_partial_wave_are_preserved() {
    use xenora_ref::Wave32Launch::FullWavesRequired;
    assert_eq!(FullWavesRequired, WaveLaunch::FullWavesRequired);
    assert_eq!(WaveConfig::default().size, WaveSize::Wave32);
    assert_eq!(
        xenora_ref::execute(&code(&[0x7e040301]), &[7]).unwrap(),
        [7]
    );
}
#[test]
fn mbcnt_boundaries_overflow_and_separate_waves() {
    for high in [false, true] {
        for add in [128, 193] {
            let out = run(&mb(high, 2, 193, add), &[0; 128]).unwrap();
            let mut expected = Vec::new();
            for _ in 0..2 {
                for lane in 0u32..64 {
                    let n = if high {
                        lane.saturating_sub(32)
                    } else {
                        lane.min(32)
                    };
                    expected.push(if add == 193 { n.wrapping_sub(1) } else { n });
                }
            }
            assert_eq!(out, expected);
        }
    }
}
#[test]
fn reads_select_both_halves_and_mask_six_bits() {
    let input: Vec<_> = (0..128).collect();
    for selector in [31, 32, 63, 64, 127] {
        let mut b = mov(9, selector).to_vec();
        b.extend([0xd7600008, 0x02001301, 0x7e040208]);
        let out = run(&b, &input).unwrap();
        let want: Vec<_> = (0..128).map(|i| (i / 64) * 64 + (selector & 63)).collect();
        assert_eq!(out, want);
    }
    for (hi, want) in [(1, 32), (0x80000000, 63), (0, 0)] {
        let mut b = mov(126, 0).to_vec();
        b.extend(mov(127, hi));
        b.push(0x7e100501);
        b.extend(mov(126, u32::MAX));
        b.extend(mov(127, u32::MAX));
        b.push(0x7e040208);
        assert_eq!(run(&b, &input[..64]).unwrap(), vec![want; 64]);
    }
}
#[test]
fn partial_exec_high_half_retains_low_destinations() {
    let mut b = vec![0x7e040301];
    b.extend(mov(126, 0));
    b.extend(mov(127, 1));
    b.push(0x7e040289);
    b.extend(mov(126, u32::MAX));
    b.extend(mov(127, u32::MAX));
    let input: Vec<_> = (0..64).collect();
    let mut want = input.clone();
    want[32] = 9;
    assert_eq!(run(&b, &input).unwrap(), want);
}
#[test]
fn wave32_unknown_high_and_explicit_writes() {
    for src in [107, 127] {
        assert!(xenora_ref::execute(&code(&[0x7e040200 | src]), &[1]).is_err());
        let mut b = mov(src, 7).to_vec();
        b.push(0x7e040200 | src);
        assert_eq!(xenora_ref::execute(&code(&b), &[1]).unwrap(), [7]);
    }
    // Compare EQ v1,v1 fully overwrites VCC, but defines no Wave32 upper value.
    for empty in [false, true] {
        let mut b = mov(107, 7).to_vec();
        if empty {
            b.extend(mov(126, 0));
        }
        b.push(0x7d840301);
        b.extend(mov(126, u32::MAX));
        b.push(0x7e04026b);
        assert!(xenora_ref::execute(&code(&b), &[1]).is_err());
    }
}
#[test]
fn comparisons_materialize_both_halves_and_halfwrites_preserve_other() {
    let mut input = vec![1; 64];
    input[32] = 0;
    // EQ 0,v1: only bit32 is set.
    for (half, want) in [(106, 0), (107, 1)] {
        let b = [0x7d840280, 0x7e040200 | half];
        assert_eq!(run(&b, &input).unwrap(), vec![want; 64]);
    }
    let mut b = vec![0x7d840280];
    b.extend(mov(106, 0));
    b.push(0x7e04026b);
    assert_eq!(run(&b, &input).unwrap(), vec![1; 64]);
    let mut b = mov(106, 0).to_vec();
    b.push(0xbf860001);
    b.push(0x7e040289);
    assert!(run(&b, &input).is_err()); // Unknown high cannot become zero.
}
#[test]
fn overlap_is_rejected_even_when_branch_skips_instruction() {
    for instruction in [
        vec![0xd7600009, 0x02001301],
        vec![0x7d84026a],
        vec![0x7d84026b],
    ] {
        let mut b = mov(9, 0).to_vec();
        b.push(0xbf820000 | instruction.len() as u32);
        b.extend(instruction);
        b.push(0x7e040301);
        let e = run(&b, &[1; 64]).unwrap_err();
        assert!(e.detail.contains("overlap"), "{e}");
    }
}
#[test]
fn ds_gather_and_scatter_stay_in_half_wave() {
    let input: Vec<_> = (0..128).collect();
    let b = [0x7e0602ff, 128, 0xdacc0000, 0x02000103, 0xbf8cc07f];
    let want: Vec<_> = (0..128).map(|i| (i / 32) * 32).collect();
    assert_eq!(run(&b, &input).unwrap(), want);
    let b = [0x7e0602ff, 128, 0xdac80000, 0x02000103, 0xbf8cc07f];
    let want: Vec<_> = (0..128)
        .map(|i| if i % 32 == 0 { i + 31 } else { 0 })
        .collect();
    assert_eq!(run(&b, &input).unwrap(), want);
}
#[test]
fn swizzle_permlane_and_nested_capture_keep_half_context() {
    let input: Vec<_> = (0..64).collect();
    assert_eq!(
        run(&[0xd8d4041f, 0x02000001, 0xbf8cc07f], &input).unwrap(),
        (0..64).map(|i| i ^ 1).collect::<Vec<_>>()
    );
    let b = [
        0xbe8803ff,
        0x76543210,
        0xbe8903ff,
        0xfedcba98,
        0xd7780002,
        257 | 8 << 9 | 9 << 18,
    ];
    assert_eq!(
        run(&b, &input).unwrap(),
        (0..64).map(|i| i ^ 16).collect::<Vec<_>>()
    );
    let b = [
        0xd8d4041f, 0x02000001, 0xbf8cc07f, 0xbe8903a0, 0xd7600008, 0x02001302, 0x7e040289,
        0x7e040208,
    ];
    assert_eq!(run(&b, &input).unwrap(), vec![33; 64]);
}
#[test]
fn runtime_mask_scalar_alu_rejects_but_vector_broadcast_succeeds() {
    assert_eq!(
        run(&[0x7d840301, 0x7e04026b], &[1; 64]).unwrap(),
        vec![u32::MAX; 64]
    );
    let e = run(&[0x7d840301, 0x8008806b, 0x7e040208], &[1; 64]).unwrap_err();
    assert!(e.detail.contains("runtime"), "{e}");
    let mut b = mov(107, 7).to_vec();
    b.extend([0x8008806b, 0x7e040208]);
    assert_eq!(run(&b, &[1; 64]).unwrap(), vec![7; 64]);
}
#[test]
fn runtime_vcc_branches_keep_high_provenance_after_low_write() {
    for nonzero in [false, true] {
        for hit in [false, true] {
            let mut input = vec![1; 64];
            if hit {
                input[32] = 0;
            }
            let mut b = vec![0x7e040301, 0x7d840280];
            b.extend(mov(106, 0));
            b.extend([if nonzero { 0xbf870001 } else { 0xbf860001 }, 0x7e040289]);
            let taken = hit == nonzero;
            assert_eq!(
                run(&b, &input).unwrap(),
                if taken { input } else { vec![9; 64] }
            );
        }
    }
    // High stays runtime after replacing low; forbidden region must still reject.
    let mut b = vec![0x7e040301, 0x7d840301];
    b.extend(mov(106, 0));
    b.extend([0xbf870001, 0xbe880380]);
    assert!(
        run(&b, &[1; 64])
            .unwrap_err()
            .detail
            .contains("non-integer-vector")
    );
}
#[test]
fn exec_branches_observe_high_half_and_writelane_ignores_exec() {
    for nonzero in [false, true] {
        let mut b = vec![0x7e040301];
        b.extend(mov(126, 0));
        b.extend(mov(127, 1));
        b.extend([if nonzero { 0xbf890001 } else { 0xbf880001 }, 0x7e040289]);
        b.extend(mov(126, u32::MAX));
        b.extend(mov(127, u32::MAX));
        let mut want = vec![5; 64];
        if !nonzero {
            want[32] = 9;
        }
        assert_eq!(run(&b, &[5; 64]).unwrap(), want);
    }
    let mut b = vec![0x7e040301];
    b.extend(mov(8, 99));
    b.extend(mov(9, 127));
    b.extend(mov(126, 0));
    b.extend(mov(127, 0));
    b.extend([0xd7610002, 8 | 9 << 9 | 128 << 18]);
    b.extend(mov(126, u32::MAX));
    b.extend(mov(127, u32::MAX));
    let mut want = vec![5; 64];
    want[63] = 99;
    assert_eq!(run(&b, &[5; 64]).unwrap(), want);
}
#[test]
fn ds_partial_masks_waits_and_nested_scatter_capture() {
    let mut b = vec![0x7e040289, 0x7e060280];
    b.extend(mov(126, 1));
    b.extend(mov(127, 2));
    b.extend([0xdacc0000, 0x02000103, 0xbf8cc07f]);
    b.extend(mov(126, u32::MAX));
    b.extend(mov(127, u32::MAX));
    let mut want = vec![9; 64];
    want[0] = 7;
    want[33] = 0;
    assert_eq!(run(&b, &[7; 64]).unwrap(), want);
    assert!(run(&[0xdacc0000, 0x02000101, 0xbf8c3f70], &[0; 64]).is_err());
    // Gather rotates within each half; scatter then reads its captured data.
    let input: Vec<_> = (0..64).collect();
    let b = [
        0xd8d4041f, 0x03000001, 0xbf8cc07f, 0x7e080280, 0xdac80000, 0x02000304, 0xbf8cc07f,
    ];
    let mut want = vec![0; 64];
    want[0] = 30;
    want[32] = 62;
    assert_eq!(run(&b, &input).unwrap(), want);
}
