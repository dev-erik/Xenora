// SPDX-License-Identifier: GPL-3.0-or-later
//! Original independent literal answers, AMD RDNA2 ISA 70648 section 12.14,
//! printed p210 (PDF page218): op8/9/10/11 = UBYTE/SBYTE/USHORT/SSHORT.
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! Zero/sign extension is architectural; low subword and four-byte per-lane
//! stride are this project's bounded ABI. LLVM 23.1.6 native ARM64 gfx1030
//! gives e0202000/e0242000/e0282000/e02c2000 with second word80000100 for v1.
#[path = "../independent_ref.rs"]
mod independent_ref;
const WAIT: u32 = 0xbf8c3f70;
const VM: u32 = 0xbc7d0000;
const VS: u32 = 0xbbfd0000;
const END: u32 = 0xbf810000;
const LENGTHS: [usize; 5] = [1, 31, 32, 33, 65];
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/buffer-narrow-load.bin");
// Input, unsigned byte, signed byte, unsigned halfword, signed halfword.
// Expected values are literal; neither interpreter generates this table.
const ROWS: [[u32; 5]; 12] = [
    [0xdead007f, 0x7f, 0x7f, 0x7f, 0x7f],
    [0xabcd0080, 0x80, 0xffffff80, 0x80, 0x80],
    [0xa50000ff, 0xff, 0xffffffff, 0xff, 0xff],
    [0xdead7fff, 0xff, 0xffffffff, 0x7fff, 0x7fff],
    [0x12348000, 0, 0, 0x8000, 0xffff8000],
    [0x1234ffff, 0xff, 0xffffffff, 0xffff, 0xffffffff],
    [0xffff0000, 0, 0, 0, 0],
    [0x7fff8001, 1, 1, 0x8001, 0xffff8001],
    [0x80000001, 1, 1, 1, 1],
    [0x00000100, 0, 0, 0x100, 0x100],
    [0xffffffff, 0xff, 0xffffffff, 0xffff, 0xffffffff],
    [0, 0, 0, 0, 0],
];
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn load(op: u32, dst: u32) -> [u32; 2] {
    [0xe0002000 | op << 18, 0x80000000 | dst << 8]
}
fn finish(mut words: Vec<u32>, dst: u32) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010000 | dst << 8, VS, END]);
    bytes(&words)
}
fn data(len: usize) -> Vec<u32> {
    (0..len).map(|i| ROWS[i % ROWS.len()][0]).collect()
}
fn values(op: u32, len: usize) -> Vec<u32> {
    let column = if op == 12 { 0 } else { (op - 7) as usize };
    (0..len).map(|i| ROWS[i % ROWS.len()][column]).collect()
}
fn masks(values: &[u32], negative: bool) -> Vec<u32> {
    values
        .chunks(32)
        .flat_map(|wave| {
            let mask = wave.iter().enumerate().fold(0, |m, (lane, &v)| {
                let pass = if negative { v >= 0x80000000 } else { v == 0 };
                m | (u32::from(pass) << lane)
            });
            vec![mask; wave.len()]
        })
        .collect()
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        for op in 8..=11 {
            for wait in [WAIT, VM] {
                for mode in 0..3 {
                    let mut words = load(op, 1).to_vec();
                    words.extend([wait, wait]); // both forms must tolerate a repeated wait
                    let want = values(op, len);
                    let (dst, want) = match mode {
                        0 => (1, want),
                        1 => {
                            words.extend([0x7d840280, 0x7e06026a]);
                            (3, masks(&want, false))
                        }
                        _ => {
                            words.extend([0x7e040280, 0x7d020501, 0x7e06026a]);
                            (3, masks(&want, true))
                        }
                    };
                    cases.push((finish(words, dst), data(len), want));
                }
            }
        }
        // Every ordered pair, including dword, reuses the same destination.
        // A wait must publish the current load format, not retain its predecessor.
        for first in 8..=12 {
            for second in 8..=12 {
                let mut words = load(first, 1).to_vec();
                words.push(WAIT);
                words.extend(load(second, 1));
                words.push(VM);
                cases.push((finish(words, 1), data(len), values(second, len)));
            }
        }
        let want = (0..len)
            .map(|i| ROWS[i % ROWS.len()][1..].iter().fold(0, |x, v| x ^ v))
            .collect();
        cases.push((FIXTURE.to_vec(), data(len), want));
    }
    cases.extend(upper_bit_cases());
    cases
}
fn upper_bit_cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for op in 8..=11 {
        let (unsigned_column, low_mask) = if op < 10 { (1, 0xff) } else { (3, 0xffff) };
        for len in LENGTHS {
            for upper in [0, 0x55555555, u32::MAX] {
                // The same literal low subword is surrounded by independently
                // varied high bits; its expected extension cannot change.
                let input = (0..len)
                    .map(|i| ROWS[i % ROWS.len()][unsigned_column] | (upper & !low_mask))
                    .collect();
                let mut words = load(op, 1).to_vec();
                words.push(VM);
                cases.push((finish(words, 1), input, values(op, len)));
            }
        }
    }
    cases
}
#[test]
fn initialized_old_destination_stays_unreadable_and_unwritable_until_wait() {
    for op in 8..=11 {
        for wait in [None, Some(VS)] {
            for operation in [0x7e040301, 0x7e020280, 0x7d840280] {
                // Seed v1, then replace it asynchronously. An earlier defined
                // value must not permit a read, overwrite, or comparison.
                let mut words = vec![0x7e020287];
                words.extend(load(op, 1));
                if let Some(wait) = wait {
                    words.push(wait);
                }
                let at = words.len() * 4;
                words.extend([operation, WAIT]);
                rejects(&finish(words, 1), at);
            }
        }
        for next in 8..=12 {
            let mut words = vec![0x7e020287];
            words.extend(load(op, 1));
            words.extend(load(next, 1)); // same pending destination, not just overlap
            words.push(WAIT);
            rejects(&finish(words, 1), 12);
        }
    }
}
fn check(code: &[u8], input: &[u32], want: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, input).unwrap(), want);
    assert_eq!(independent_ref::execute(code, input).unwrap(), want);
}
fn rejects(code: &[u8], at: usize) {
    let error = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match error {
        xenora_shader::CompileError::Decode(e) => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } => assert_eq!(offset, at),
        other => panic!("unexpected {other:?}"),
    }
    let primary = xenora_ref::execute(code, &[0x12348080; 33]).unwrap_err();
    assert_eq!(primary.offset, at, "{primary}");
    let independent = independent_ref::execute(code, &[0x12348080; 33]).unwrap_err();
    assert!(
        independent.starts_with(&format!("{at:#x}:")),
        "{independent}"
    );
}
#[test]
fn llvm_words_fixture_and_literal_boundary_answers() {
    for (op, first) in [
        (8, 0xe0202000),
        (9, 0xe0242000),
        (10, 0xe0282000),
        (11, 0xe02c2000),
    ] {
        assert_eq!(load(op, 1), [first, 0x80000100]);
        // Every boundary tested on lane0 separately, including dirty upper bits.
        for row in ROWS {
            check(
                &finish(vec![first, 0x80000100, WAIT], 1),
                &[row[0]],
                &[row[(op - 7) as usize]],
            );
        }
    }
    let mut words = Vec::new();
    for op in 8..=11 {
        words.extend(load(op, op - 7));
        words.push(if op % 2 == 0 { WAIT } else { VM });
    }
    words.extend([0x3a0a0501, 0x3a0a0705, 0x3a0a0905]);
    assert_eq!(FIXTURE, finish(words, 5));
}
#[test]
fn literal_values_comparison_cones_and_sequential_loads() {
    assert_eq!(cases().len(), 310);
    for (code, input, want) in cases() {
        check(&code, &input, &want);
    }
    // Confirm the comparison corpus distinguishes raw words from extensions.
    for op in 8..=11 {
        assert_ne!(masks(&data(32), false), masks(&values(op, 32), false));
    }
    for op in [9, 11] {
        assert_ne!(masks(&data(32), true), masks(&values(op, 32), true));
    }
}
#[test]
fn waits_preserve_readiness_and_load_store_separation() {
    for op in 8..=11 {
        let words = load(op, 1).to_vec();
        rejects(&finish(words.clone(), 1), 8); // store reads pending destination
        for wait in [None, Some(VS)] {
            for read_or_write in [0x7e040301, 0x7e020280] {
                // read v1 / overwrite v1
                let mut w = words.clone();
                if let Some(wait) = wait {
                    w.push(wait);
                }
                let at = w.len() * 4;
                w.extend([read_or_write, WAIT]);
                rejects(&finish(w, 1), at);
            }
        }
        for next in 8..=12 {
            let mut w = words.clone();
            w.extend(load(next, 2));
            w.push(WAIT);
            rejects(&finish(w, 1), 8);
        }
        // Load waits never discharge a store; conversely VS does not ready a load.
        for wait in [WAIT, VM] {
            let mut w = words.clone();
            w.extend([wait, 0xe0702000, 0x80010100, wait, END]);
            rejects(&bytes(&w), 24);
        }
        let mut w = words.clone();
        w.extend([VS, END]);
        rejects(&bytes(&w), 12);
    }
}
#[test]
fn address_flags_resources_index_and_nonfull_exec_are_rejected() {
    for op in 8..=11 {
        let base = load(op, 1);
        for bit in 0..18 {
            let mut w = base.to_vec();
            w[0] ^= 1 << bit;
            w.push(WAIT);
            rejects(&finish(w, 1), 0);
        }
        for bit in (0..8).chain(16..32) {
            let mut w = base.to_vec();
            w[1] ^= 1 << bit;
            w.push(WAIT);
            rejects(&finish(w, 1), 0);
        }
        let mut w = load(op, 0).to_vec();
        w.push(WAIT);
        rejects(&finish(w, 1), 0);
        for exec in [0, 1, 0x80000000, 0x7fffffff] {
            let mut w = vec![0xbefe03ff, exec];
            w.extend(base);
            w.extend([WAIT, 0xbefe03c1]);
            rejects(&finish(w, 1), 8);
        }
        // A mutated address register remains invalid even when initialized.
        let mut w = vec![0x7e040280];
        w.extend([base[0], base[1] | 2]);
        w.push(WAIT);
        rejects(&finish(w, 1), 4);
    }
}
#[test]
fn truncated_narrow_instructions_are_not_accepted() {
    for op in 8..=11 {
        let encoded = bytes(&load(op, 1));
        for len in 1..8 {
            assert!(
                xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &encoded[..len])
                    .is_err()
            );
            assert!(xenora_ref::execute(&encoded[..len], &[0]).is_err());
            assert!(independent_ref::execute(&encoded[..len], &[0]).is_err());
        }
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_buffer_narrow_load_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, input, want) in cases() {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
                .unwrap(),
            want
        );
    }
}
