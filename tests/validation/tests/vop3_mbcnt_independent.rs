// SPDX-License-Identifier: GPL-3.0-or-later
//! Original independent expectations: AMD RDNA2 ISA 70648, sections 6.2 and
//! 12.12 (printed page 184), opcodes 869/870:
//! https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf
//! LO adds SRC0 bits strictly below ThreadPosition; HI copies SRC1 in Wave32.
//! EXEC gates writes, not counted bits. No production oracle produces answers.
//! LLVM 23.1.6 ARM64 gfx1030 +wavefrontsize32 independently confirms encodings.
const LO: u32 = 869;
const HI: u32 = 870;
const LENGTHS: [usize; 6] = [1, 31, 32, 33, 63, 65];
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/mbcnt.bin");
fn insn(op: u32, dst: u32, a: u32, b: u32) -> [u32; 2] {
    [0xd4000000 | op << 16 | dst, a | b << 9 | 128 << 18]
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn program(body: &[u32], dst: u32) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(body);
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&words)
}
// Enumerate bits, rather than duplicate a lower-mask/popcount implementation.
fn answer(op: u32, mask: u32, acc: u32, index: usize) -> u32 {
    let count = if op == LO {
        (0..index % 32)
            .filter(|bit| mask & (1u32 << bit) != 0)
            .count()
    } else {
        0
    };
    ((u64::from(acc) + count as u64) % 0x1_0000_0000) as u32
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| {
            [
                u32::MAX,
                0x80000001,
                0x55555555,
                0,
                0x40000002,
                0x80000000,
                0x7fffffff,
            ][i % 7]
        })
        .collect()
}
fn compare_answers(data: &[u32]) -> Vec<u32> {
    data.chunks(32)
        .flat_map(|wave| {
            let mask = wave.iter().enumerate().fold(0, |bits, (lane, &x)| {
                bits | (u32::from(answer(HI, u32::MAX, answer(LO, x, 0, lane), lane) == 1) << lane)
            });
            vec![mask; wave.len()]
        })
        .collect()
}
fn check(code: &[u8], data: &[u32], expected: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code)
        .unwrap_or_else(|e| panic!("{code:02x?}: {e:?}"));
    assert_eq!(xenora_ref::execute(code, data).unwrap(), expected);
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        let data = input(len);
        for op in [LO, HI] {
            for acc in [0, 1, u32::MAX, 0xfffffff0] {
                for dst in [1, 2, 255] {
                    let mut body = vec![0x7e0402ff, acc];
                    body.extend(insn(op, dst, 257, 258));
                    let expected = data
                        .iter()
                        .enumerate()
                        .map(|(i, &m)| answer(op, m, acc, i))
                        .collect();
                    cases.push((program(&body, dst), data.clone(), expected));
                }
            }
            // Both operands and destination alias; reads precede the write.
            let expected = data
                .iter()
                .enumerate()
                .map(|(i, &m)| answer(op, m, m, i))
                .collect();
            cases.push((program(&insn(op, 1, 257, 257), 1), data.clone(), expected));
            for exec in [0, 0x80000002, 0x55555555, u32::MAX] {
                let mut body = vec![0x7e0402ff, 0xa5a5a5a5, 0xbefe03ff, exec];
                body.extend(insn(op, 2, 193, 129)); // -1,1: includes inactive SRC0 bits.
                body.push(0xbefe03c1);
                let expected = (0..len)
                    .map(|i| {
                        if exec & (1 << (i % 32)) == 0 {
                            0xa5a5a5a5
                        } else {
                            answer(op, u32::MAX, 1, i)
                        }
                    })
                    .collect();
                cases.push((program(&body, 2), data.clone(), expected));
            }
        }
        cases.push((FIXTURE.to_vec(), data.clone(), compare_answers(&data)));
    }
    cases.extend(source_cases());
    cases.extend(comparison_cone_cases());
    cases
}
#[test]
fn exact_llvm_words_and_literal_lane_anchors() {
    assert_eq!(insn(LO, 2, 257, 258), [0xd7650002, 0x02020501]);
    assert_eq!(insn(HI, 2, 257, 258), [0xd7660002, 0x02020501]);
    assert_eq!(insn(LO, 2, 193, 128), [0xd7650002, 0x020100c1]);
    for (lane, expected) in [(0, u32::MAX), (1, 0), (31, 30)] {
        assert_eq!(answer(LO, u32::MAX, u32::MAX, lane), expected);
        assert_eq!(answer(HI, u32::MAX, u32::MAX, lane), u32::MAX);
    }
    assert_eq!(answer(LO, 0x80000001, 0, 31), 1);
    assert_eq!(answer(LO, 0x80000000, 0, 31), 0);
    let mut body = insn(LO, 2, 257, 128).to_vec();
    body.extend(insn(HI, 2, 193, 258));
    body.extend([0x7d840481, 0x7e06026a]);
    assert_eq!(FIXTURE, program(&body, 3));
}
#[test]
fn independent_wave32_values_aliases_exec_and_comparison_cones() {
    for (code, data, expected) in cases() {
        check(&code, &data, &expected);
    }
}
fn source_cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for op in [LO, HI] {
        for (source, value) in [
            (8, 0x80000001),
            (128, 0),
            (129, 1),
            (193, u32::MAX),
            (255, 0x80000001),
        ] {
            for position in 0..2 {
                let mut body = vec![0xbe8803ff, 0x80000001];
                let mut src = [257, 257];
                src[position] = source;
                body.extend(insn(op, 2, src[0], src[1]));
                if source == 255 {
                    body.push(value);
                }
                let data = input(65);
                let expected: Vec<_> = data
                    .iter()
                    .enumerate()
                    .map(|(i, &x)| {
                        if position == 0 {
                            answer(op, value, x, i)
                        } else {
                            answer(op, x, value, i)
                        }
                    })
                    .collect();
                cases.push((program(&body, 2), data, expected));
            }
        }
    }
    cases
}
fn comparison_cone_cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        // Constant -1 and 0 still produce the lane number, never a uniform value.
        // Equality selects exactly bit 1 or bit 31 when that lane exists.
        // These literal masks do not call the test-local MBCNT evaluator.
        for (equal, full_mask) in [(1, 0x00000002), (31, 0x80000000)] {
            let mut body = insn(LO, 2, 193, 128).to_vec();
            body.extend([0x7d840480 | equal, 0x7e06026a]);
            let data = input(len);
            let expected = data
                .chunks(32)
                .flat_map(|wave| {
                    let mask = if wave.len() > equal as usize {
                        full_mask
                    } else {
                        0
                    };
                    vec![mask; wave.len()]
                })
                .collect();
            cases.push((program(&body, 3), data, expected));
        }
        // Seed 1, overwrite lanes 1/30 with HI(-1,0) under partial EXEC,
        // restore EXEC, then compare against 1. Inactive lanes must retain 1
        // during comparison-expression recomputation as well as direct writes.
        let mut body = vec![0x7e040281, 0xbefe03ff, 0x40000002];
        body.extend(insn(HI, 2, 193, 128));
        body.extend([0xbefe03c1, 0x7d840481, 0x7e06026a]);
        let data = input(len);
        let expected = data
            .chunks(32)
            .flat_map(|wave| {
                let mask = match wave.len() {
                    1 => 0x00000001,
                    31 => 0x3ffffffd,
                    32 => 0xbffffffd,
                    _ => unreachable!("only the explicitly chosen partial-wave lengths"),
                };
                vec![mask; wave.len()]
            })
            .collect();
        cases.push((program(&body, 3), data, expected));
    }
    cases
}
#[test]
fn scalar_inline_literal_sources_in_both_positions() {
    for (code, data, expected) in source_cases() {
        check(&code, &data, &expected);
    }
}
#[test]
fn constant_lo_and_masked_hi_comparison_cones() {
    for (code, data, expected) in comparison_cone_cases() {
        check(&code, &data, &expected);
    }
}
fn reject(body: &[u32], decode: bool) {
    let code = program(body, 1);
    let error = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap_err();
    match error {
        xenora_shader::CompileError::Decode(e) if decode => assert_eq!(e.offset, 12),
        xenora_shader::CompileError::Verify { offset, .. } if !decode => assert_eq!(offset, 12),
        other => panic!("unexpected rejection: {other:?}"),
    }
    assert_eq!(xenora_ref::execute(&code, &[0]).unwrap_err().offset, 12);
}
#[test]
fn rejects_unsupported_modifiers_special_sources_and_uninitialized_sources() {
    for op in [LO, HI] {
        for bit in 8..16 {
            let mut words = insn(op, 2, 257, 128);
            words[0] |= 1 << bit;
            reject(&words, true);
        }
        for bit in 27..32 {
            let mut words = insn(op, 2, 257, 128);
            words[1] |= 1 << bit;
            reject(&words, true);
        }
        for source in [108, 109, 110, 124, 125, 209, 239, 249, 250, 254] {
            reject(&insn(op, 2, source, 128), true);
            reject(&insn(op, 2, 128, source), true);
        }
        for sources in [[259, 128], [128, 259], [8, 128], [128, 8]] {
            reject(&insn(op, 2, sources[0], sources[1]), false);
        }
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_batched_mbcnt_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, data, expected) in cases() {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &data)
                .unwrap(),
            expected
        );
    }
}
