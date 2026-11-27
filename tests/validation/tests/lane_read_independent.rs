// SPDX-License-Identifier: GPL-3.0-or-later
//! Original test-local expectations from AMD RDNA2 70648 sections 12.8/12.12,
//! printed pp137/183; public text also supplied as /private/tmp/clamp-rdna2.txt.
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! READFIRST chooses the lowest EXEC bit, or zero for empty EXEC. READLANE
//! ignores EXEC and selects the low five index bits. This bounded ABI admits
//! only guaranteed lane0, normal SGPR selectors, and runtime scalar-to-VGPR MOV.
//! LLVM 23.1.6 ARM64 verified VOP1 READFIRST and VOP3 READLANE only; the
//! inventory's VOP3 READFIRST opcode386 is not claimed as LLVM-verified.
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const WAIT: u32 = 0xbf8c3f70;
const END: u32 = 0xbf810000;
const LENGTHS: [usize; 6] = [1, 31, 32, 33, 63, 65];
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/lane-read.bin");
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn finish(mut words: Vec<u32>, dst: u32) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, END]);
    bytes(&words)
}
fn first(dst: u32, src: u32) -> Vec<u32> {
    vec![0x7e000500 | dst << 17 | src]
}
fn lane(dst: u32, src: u32, selector: u32) -> Vec<u32> {
    vec![0xd7600000 | dst, 0x02000100 | src | selector << 9]
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| {
            let base = [0x80000001, 0xffffffff, 0x7fffffff][i / 32 % 3];
            if i % 5 == 0 || i % 32 == 0 {
                base
            } else {
                base ^ (i as u32 * 0x10203)
            }
        })
        .collect()
}
fn broadcast(data: &[u32]) -> Vec<u32> {
    data.chunks(32)
        .flat_map(|wave| vec![wave[0]; wave.len()])
        .collect()
}
fn equality_masks(data: &[u32]) -> Vec<u32> {
    data.chunks(32)
        .flat_map(|wave| {
            let mask = wave
                .iter()
                .enumerate()
                .fold(0, |m, (i, &v)| m | (u32::from(v == wave[0]) << i));
            vec![mask; wave.len()]
        })
        .collect()
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        let data = input(len);
        for selector in [None, Some(0), Some(32), Some(64), Some(0xffffffe0)] {
            let execs: &[u32] = if selector.is_none() {
                &[0, 1, 0x80000001, u32::MAX]
            } else {
                &[0, 2, 0x80000000]
            };
            for &exec in execs {
                for comparison in [false, true] {
                    let mut w = LOAD.to_vec();
                    w.extend([WAIT, 0x7e040301]); // retain original per-lane input in v2
                    // A write under EXEC with lane0 disabled must retain lane0.
                    w.extend([0xbefe03ff, 0xfffffffe, 0x7e020289, 0xbefe03ff, exec]);
                    if let Some(index) = selector {
                        w.extend([0xbe8903ff, index]);
                        w.extend(lane(8, 1, 9));
                    } else {
                        w.extend(first(8, 1));
                    }
                    w.extend([0xbefe03c1, 0x7e020280, 0x7e060208]); // overwrite source after capture
                    let (dst, want) = if comparison {
                        w.extend([0x7d840503, 0x7e08026a]);
                        (4, equality_masks(&data))
                    } else {
                        (3, broadcast(&data))
                    };
                    cases.push((finish(w, dst), data.clone(), want));
                }
            }
        }
        // Replay a shift/mask source expression, not just a direct buffer load.
        for read_first in [false, true] {
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0x7e0402ff, 0x00ffffff, 0x36020302, 0x34020281]);
            if read_first {
                w.extend(first(8, 1));
            } else {
                w.push(0xbe8903a0);
                w.extend(lane(8, 1, 9));
            }
            w.extend([0x7e020280, 0x7e060208]);
            let want = broadcast(&data)
                .iter()
                .map(|v| (v % 0x1000000) * 2)
                .collect();
            cases.push((finish(w, 3), data.clone(), want));
        }
        // Constant overwrite invalidates runtime scalar state, and a later lane
        // read invalidates the constant. v3 retains its earlier scalar snapshot.
        let mut w = LOAD.to_vec();
        w.push(WAIT);
        w.extend(first(8, 1));
        w.extend([0x7e060208, 0xbe88038d, 0x7e080208, 0x7e020289]);
        w.extend(first(8, 1));
        w.extend([0x7e0a0208]);
        cases.push((finish(w.clone(), 3), data.clone(), broadcast(&data)));
        cases.push((finish(w.clone(), 4), data.clone(), vec![13; len]));
        cases.push((finish(w, 5), data.clone(), vec![9; len]));
        // MBCNT source replay must use selected lane0, not the current invocation.
        cases.push((FIXTURE.to_vec(), data.clone(), broadcast(&data)));
    }
    cases.extend(state_cases());
    cases
}
fn state_cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        for read_first in [false, true] {
            let read = |dst| {
                if read_first {
                    first(dst, 1)
                } else {
                    lane(dst, 1, 9)
                }
            };
            let data = input(len);
            for taken in [false, true] {
                for conditional_move in [false, true] {
                    let mut w = LOAD.to_vec();
                    w.extend([WAIT, 0xbe890380]);
                    w.extend(read(8));
                    w.push(if taken { 0xbf068080 } else { 0xbf068081 });
                    if conditional_move {
                        w.push(0xbe88058d);
                    } else {
                        w.extend([0xbf840001, 0xbe88038d]);
                    }
                    w.push(0x7e060208);
                    let want = if taken {
                        vec![13; len]
                    } else {
                        broadcast(&data)
                    };
                    cases.push((finish(w, 3), data.clone(), want));
                }
            }
            // Repeated reads of the same VGPR capture different source versions.
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0xbe890380]);
            w.extend(read(8));
            w.push(0x7e020289);
            w.extend(read(10));
            w.extend([0x7e060208, 0x7e08020a]);
            cases.push((finish(w.clone(), 3), data.clone(), broadcast(&data)));
            cases.push((finish(w, 4), data.clone(), vec![9; len]));

            // Runtime VCCNZ preserves the lane-read MOV result on its taken edge;
            // the other edge writes 9. A later compare must replay this joined
            // value for every lane of its own wave, including the partial tail.
            let data: Vec<u32> = (0..len)
                .map(|i| if i % 64 == 0 { 0 } else { i as u32 + 100 })
                .collect();
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0xbe890380, 0x7e040301]);
            w.extend(read(8));
            w.extend([
                0x7e060208, 0x7d840280, 0xbf870001, 0x7e060289, 0x7d840503, 0x7e08026a,
            ]);
            let want = data
                .chunks(32)
                .flat_map(|wave| {
                    let selected = if wave.contains(&0) { wave[0] } else { 9 };
                    let mask = wave
                        .iter()
                        .enumerate()
                        .fold(0, |m, (i, &v)| m | (u32::from(v == selected) << i));
                    vec![mask; wave.len()]
                })
                .collect();
            cases.push((finish(w, 4), data, want));
        }
    }
    cases
}
#[test]
fn conditional_overwrites_repeated_reads_and_runtime_join_replay() {
    assert_eq!(state_cases().len(), 84);
    for (code, data, want) in state_cases() {
        check(&code, &data, &want);
    }
}
fn check(code: &[u8], data: &[u32], want: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, data).unwrap(), want);
}
fn rejects(code: &[u8], at: usize) {
    let error = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match error {
        xenora_shader::CompileError::Decode(e) => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } => assert_eq!(offset, at),
        other => panic!("unexpected rejection: {other:?}"),
    }
    assert_eq!(
        xenora_ref::execute(code, &[0x80000001; 33])
            .unwrap_err()
            .offset,
        at
    );
}
#[test]
fn llvm_words_and_original_fixture() {
    assert_eq!(first(8, 1), [0x7e100501]);
    assert_eq!(lane(8, 1, 9), [0xd7600008, 0x02001301]);
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(first(8, 1));
    w.extend([0xd7650002, 0x020100c1, 0xbe8903a0]);
    w.extend(lane(10, 2, 9));
    w.extend([0x7e020280, 0x7e060208, 0x7e08020a, 0x3a060903]);
    assert_eq!(FIXTURE, finish(w, 3));
}
#[test]
fn guaranteed_lane_zero_snapshots_exec_and_replay() {
    assert_eq!(cases().len(), 312);
    for (code, data, want) in cases() {
        check(&code, &data, &want);
    }
}
#[test]
fn other_lanes_and_undefined_or_runtime_selectors_reject() {
    for exec in [2, 0x80000000, 0xfffffffe] {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbefe03ff, exec]);
        w.extend(first(8, 1));
        w.extend([0xbefe03c1, 0x7e060208]);
        rejects(&finish(w, 3), 20);
    }
    for index in [1, 31, 33, 63, u32::MAX] {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe8903ff, index]);
        w.extend(lane(8, 1, 9));
        w.push(0x7e060208);
        rejects(&finish(w, 3), 20);
    }
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(lane(8, 1, 9));
    w.push(0x7e060208);
    rejects(&finish(w, 3), 12);
    let mut w = LOAD.to_vec();
    w.push(WAIT);
    w.extend(first(9, 1));
    w.extend(lane(8, 1, 9));
    w.push(0x7e060208);
    rejects(&finish(w, 3), 16);
}
#[test]
fn sources_require_initialization_and_load_readiness_even_with_empty_exec() {
    for read_first in [false, true] {
        for exec in [0x80, 0xc1] {
            let mut w = vec![0xbe890380, 0xbefe0300 | exec];
            w.extend(if read_first {
                first(8, 1)
            } else {
                lane(8, 1, 9)
            });
            w.extend([0xbefe03c1, 0x7e060208]);
            rejects(&finish(w, 3), 8);
        }
        let mut w = vec![0xbe890380];
        w.extend(LOAD);
        w.extend(if read_first {
            first(8, 1)
        } else {
            lane(8, 1, 9)
        });
        w.extend([WAIT, 0x7e060208]);
        rejects(&finish(w, 3), 12);
    }
}
#[test]
fn restricted_scalar_consumers_and_operand_forms_reject() {
    for consumer in [0x80098008, 0x7d840208, 0x36060208] {
        let mut w = LOAD.to_vec();
        w.push(WAIT);
        w.extend(first(8, 1));
        w.extend([consumer, 0x7e060208]);
        rejects(&finish(w, 3), 16);
    }
    for dst in [0, 7, 106, 127] {
        let mut w = LOAD.to_vec();
        w.push(WAIT);
        w.extend(first(dst, 1));
        rejects(&finish(w, 1), 12);
    }
    for selector in [106, 126, 128, 160, 192, 255, 257] {
        let mut w = LOAD.to_vec();
        w.push(WAIT);
        w.extend(lane(8, 1, selector));
        rejects(&finish(w, 1), 12);
    }
    for bit in 8..16 {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe890380]);
        let mut read = lane(8, 1, 9);
        read[0] |= 1 << bit;
        w.extend(read);
        rejects(&finish(w, 1), 16);
    }
    for bit in 27..32 {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe890380]);
        let mut read = lane(8, 1, 9);
        read[1] |= 1 << bit;
        w.extend(read);
        rejects(&finish(w, 1), 16);
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_lane_read_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, data, want) in cases() {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &data)
                .unwrap(),
            want
        );
    }
}
