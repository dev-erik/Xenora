// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte bitfield/count fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 6] = [328, 867, 334, 868, 882, 376];
fn arity(op: u32) -> usize {
    if matches!(op, 867 | 868) { 2 } else { 3 }
}
fn insn(op: u32, dst: u32, src: [u32; 3]) -> [u32; 2] {
    [
        0xd4000000 | op << 16 | dst,
        src[0] | src[1] << 9 | src[2] << 18,
    ]
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn program(body: &[u32], dst: u32) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&w)
}
fn check(code: &[u8], input: &[u32], expected: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code)
        .unwrap_or_else(|e| panic!("{code:02x?}: {e:?}"));
    assert_eq!(
        xenora_ref::execute(code, input).unwrap(),
        expected,
        "primary {code:02x?}"
    );
    assert_eq!(
        independent_ref::execute(code, input).unwrap(),
        expected,
        "secondary {code:02x?}"
    );
}
fn reject(code: &[u8], at: usize, decode: bool, intent: &str) {
    let e = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match &e {
        xenora_shader::CompileError::InvalidLength { bytes }
            if decode && !code.len().is_multiple_of(4) =>
        {
            assert_eq!(*bytes, code.len())
        }
        xenora_shader::CompileError::Decode(e) if decode && code.len().is_multiple_of(4) => {
            assert_eq!(e.offset, at)
        }
        xenora_shader::CompileError::Verify { offset, .. } if !decode => assert_eq!(*offset, at),
        _ => panic!("wrong rejection {e:?}"),
    }
    assert!(e.to_string().contains(intent), "{e}");
    for x in [0, 1] {
        let a = xenora_ref::execute(code, &[x]).unwrap_err();
        assert_eq!(a.offset, at, "{a:?}");
        let b = independent_ref::execute(code, &[x]).unwrap_err();
        assert!(b.starts_with(&format!("{at:#x}:")), "{b}");
    }
}
const ANCHORS: &[(u32, u32, u32, u32, u32)] = &[
    (328, 0xffffffff, 0, 32, 0),
    (328, 0x80000000, 31, 1, 1),
    (328, 0x80000000, 31, 2, 1),
    (328, 0x12345678, 4, 8, 0x67),
    (328, 1, 31, 2, 0),
    (867, 0, 31, 0, 0),
    (867, 32, 0, 0, 0),
    (867, 33, 31, 0, 0x80000000),
    (867, 31, 1, 0, 0xfffffffe),
    (867, 31, 31, 0, 0x80000000),
    (334, 0x11223344, 0x55667788, 0, 0x55667788),
    (334, 0x11223344, 0x55667788, 4, 0x45566778),
    (334, 0x11223344, 0x55667788, 8, 0x44556677),
    (334, 0x11223344, 0x55667788, 31, 0x22446688),
    (334, 0x11223344, 0x55667788, 32, 0x55667788),
    (334, 0x11223344, 0x55667788, 33, 0x2ab33bc4),
    (334, 0x11223344, 0x55667788, 0xffffffff, 0x22446688),
    (334, 1, 0, 1, 0x80000000),
    (868, 0xffffffff, 0xffffffe0, 0, 0),
    (868, 0x80000000, 0xffffffff, 0, 0),
    (868, 0, 0x12345678, 0, 0x12345678),
    (868, 0xaaaaaaaa, 1, 0, 17),
    (882, 15, 0x33, 0x55, 0x7f),
    (376, 15, 0x33, 0x55, 0x69),
    (882, 0x7fa12345, 0x7fa12345, 0, 0x7fa12345),
    (376, 0x7fa12345, 0x7fa12345, 0, 0),
];
fn literal_case(op: u32, dst: u32, b: u32, c: u32) -> Vec<u8> {
    let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
    body.extend(insn(
        op,
        dst,
        [257, 258, if arity(op) == 2 { 128 } else { 259 }],
    ));
    program(&body, dst)
}
#[test]
fn literal_anchors_and_all_destination_aliases() {
    for &(op, a, b, c, want) in ANCHORS {
        for dst in [1, 2, 3, 4, 255] {
            check(&literal_case(op, dst, b, c), &[a], &[want]);
        }
    }
}
#[test]
fn cpu_field_boundaries_and_truth_tables() {
    // Test-local word specification uses a 64-bit window; fixed rows above
    // separately anchor clipping, masked widths, and the high/low ordering.
    for offset in 0..32 {
        for width in 0..32 {
            let input = [
                0, 0xffffffff, 0x80000000, 0x7fffffff, 0x55555555, 0xaaaaaaaa,
            ];
            let mask = 2u64.pow(width) - 1;
            let expected: Vec<_> = input
                .iter()
                .map(|&x| ((u64::from(x) / 2u64.pow(offset)) & mask) as u32)
                .collect();
            check(&literal_case(328, 4, offset, width), &input, &expected);
            let want = ((mask * 2u64.pow(offset)) % 0x100000000) as u32;
            check(&literal_case(867, 4, offset, 0), &[width], &[want]);
        }
    }
    for bit in 0..32 {
        let one = 2u32.pow(bit);
        check(&literal_case(868, 4, 0, 0), &[one, !one], &[1, 31]);
        check(&literal_case(868, 4, 0xffffffff, 0), &[one, !one], &[0, 30]);
        for count in 0..32 {
            // Track a single bit in the conceptual concatenation, without shifts.
            let high_position = 32 + bit;
            let low_position = bit;
            let expected_high = if (count..count + 32).contains(&high_position) {
                2u32.pow(high_position - count)
            } else {
                0
            };
            let expected_low = if (count..count + 32).contains(&low_position) {
                2u32.pow(low_position - count)
            } else {
                0
            };
            check(&literal_case(334, 4, 0, count), &[one], &[expected_high]);
            check(&literal_case(334, 4, one, count), &[0], &[expected_low]);
        }
        for truth in 0u32..8 {
            let a = if truth & 1 != 0 { one } else { 0 };
            let b = if truth & 2 != 0 { one } else { 0 };
            let c = if truth & 4 != 0 { one } else { 0 };
            check(
                &literal_case(882, 4, b, c),
                &[a],
                &[if truth == 0 { 0 } else { one }],
            );
            check(
                &literal_case(376, 4, b, c),
                &[a],
                &[if matches!(truth, 1 | 2 | 4 | 7) {
                    one
                } else {
                    0
                }],
            );
        }
    }
}
#[test]
fn raw_sources_identity_literals_and_decode_boundaries() {
    for op in OPS {
        for pos in 0..arity(op) {
            for bad in (107..=127).chain(209..=239).chain(249..=254) {
                let mut src = [128; 3];
                src[pos] = bad;
                let mut body = vec![0xbefe0380];
                body.extend(insn(op, 2, src));
                reject(&program(&body, 1), 16, true, "");
            }
            for srcval in [
                8, 105, 106, 128, 129, 192, 193, 208, 240, 248, 255, 257, 496, 504, 511,
            ] {
                // Zero-valued companions isolate source-position behavior. For awkward
                // count controls use x=1, whose answers are literal and easy to distinguish.
                let value = match srcval {
                    128 => 0,
                    192 => 64,
                    193 => 0xffffffff,
                    208 => 0xfffffff0,
                    240 => 0x3f000000,
                    248 => 0x3e22f983,
                    _ => 1,
                };
                let mut src = [128; 3];
                src[pos] = srcval;
                let mut body = vec![0xbe880381, 0xbee90381, 0xbeea0381];
                if srcval >= 496 {
                    body.extend([0x7e0002ff | (srcval - 256) << 17, 1]);
                }
                body.extend(insn(op, 2, src));
                if srcval == 255 {
                    body.push(1);
                }
                let answer = match op {
                    328 => 0,
                    867 => {
                        if pos == 0 {
                            match srcval {
                                128 | 192 | 240 => 0,
                                193 => 0x7fffffff,
                                208 => 0xffff,
                                248 => 7,
                                _ => 1,
                            }
                        } else {
                            0
                        }
                    }
                    334 => {
                        if pos == 1 {
                            value
                        } else {
                            0
                        }
                    }
                    868 => {
                        if pos == 1 {
                            value
                        } else {
                            match srcval {
                                128 => 0,
                                192 => 1,
                                193 => 32,
                                208 => 28,
                                240 => 6,
                                248 => 16,
                                _ => 1,
                            }
                        }
                    }
                    _ => value,
                };
                check(&program(&body, 2), &[1], &[answer]);
            }
            let mut src = [128; 3];
            src[pos] = 255;
            let encoded = bytes(&insn(op, 2, src));
            for n in 0..4 {
                let mut code = encoded.clone();
                code.extend_from_slice(&[1, 2, 3][..n]);
                reject(&code, 0, true, "");
            }
        }
        for n in 4..8 {
            reject(&bytes(&insn(op, 2, [128; 3]))[..n], 0, true, "");
        }
        for bit in (8..16).chain(27..32) {
            let mut w = insn(op, 2, [128; 3]);
            w[usize::from(bit >= 27)] |= 1 << bit;
            reject(&program(&w, 1), 12, true, "modifier");
        }
        if arity(op) == 2 {
            for pad in [0, 106, 129, 255, 511] {
                reject(&program(&insn(op, 2, [128, 128, pad]), 1), 12, true, "SRC2");
            }
        }
        for mut src in [
            [8, 8, 8],
            [8, 9, 8],
            [255, 255, 255],
            [106, 8, 106],
            [8, 255, 255],
            [8, 9, 129],
        ] {
            if arity(op) == 2 {
                src[2] = 128;
            }
            let mut body = vec![0xbe880381, 0xbe890381, 0xbeea0381];
            body.extend(insn(op, 2, src));
            if src[..arity(op)].contains(&255) {
                body.push(1);
            }
            let want = match op {
                328 => 0,
                867 | 868 => 2,
                334 => 0x80000000,
                882 | 376 => 1,
                _ => unreachable!(),
            };
            check(&program(&body, 2), &[0], &[want]);
        }
        if arity(op) == 3 {
            for mut src in [[8, 9, 10], [8, 9, 255], [106, 8, 255]] {
                for _ in 0..3 {
                    let mut body = vec![0xbe880381, 0xbe890381, 0xbe8a0381, 0xbeea0381, 0xbefe0380];
                    body.extend(insn(op, 2, src));
                    if src.contains(&255) {
                        body.push(1);
                    }
                    reject(&program(&body, 1), 32, true, "scalar-bus");
                    src.rotate_left(1);
                }
            }
        }
    }
}
#[test]
fn masks_flags_readiness_and_region_exclusion() {
    for op in OPS {
        for pos in 0..arity(op) {
            for unknown in [20, 106, 265] {
                let mut src = [128; 3];
                src[pos] = unknown;
                let mut body = vec![0x7e040287, 0xbefe0380];
                body.extend(insn(op, 2, src));
                body.push(0xbefe03c1);
                check(&program(&body, 2), &[0], &[7]);
                for mask in [1, 0x80000000, 0xffffffff] {
                    let mut body = vec![0x7e040287, 0xbefe03ff, mask];
                    body.extend(insn(op, 2, src));
                    body.push(0xbefe03c1);
                    reject(
                        &program(&body, 2),
                        24,
                        false,
                        if unknown == 20 {
                            "no known value"
                        } else {
                            "uninitialized"
                        },
                    );
                }
            }
        }
        let mut body = vec![0xbefe0381];
        body.extend(insn(op, 2, [128; 3]));
        body.push(0xbefe03c1);
        reject(&program(&body, 2), 16, false, "partial EXEC");
        let mut body = insn(op, 2, [128; 3]).to_vec();
        body.extend([0xbf840001, 0x7e040280]);
        reject(&program(&body, 2), 20, false, "SCC");
        let mut body = insn(op, 2, [128; 3]).to_vec();
        body.push(0x7e04026a);
        reject(&program(&body, 2), 20, false, "known value");
        for mask in [0, 1, 0xffffffff] {
            let mut body = vec![0x7e040287, 0x7d8a0280, 0xbefe03ff, mask, 0xbf860002];
            body.extend(insn(
                op,
                2,
                [257, 257, if arity(op) == 2 { 128 } else { 257 }],
            ));
            body.push(0xbefe03c1);
            reject(&program(&body, 2), 32, false, "runtime");
            let mut body = vec![0x7e040287, 0x7e060280, 0xbefe03ff, mask];
            body.extend(insn(op, 3, [128; 3]));
            body.extend([0xbf880001, 0x7e040289, 0xbefe03c1]);
            check(&program(&body, 2), &[0], &[if mask == 0 { 7 } else { 9 }]);
        }
    }
}
const INPUT: [u32; 8] = [0, 1, 31, 32, 33, 63, 0xffffffff, 0x80000000];
// BFE/MAX>>31, BFM offset31, ALIGN asymmetric halves/count, BCNT+MAX,
// OR3/XOR3 with 33 and55. These are fixed literal arrays, not evaluator output.
const ANSWERS: [[u32; 8]; 6] = [
    [0, 1, 1, 0, 1, 1, 1, 0],
    [
        0, 0x80000000, 0x80000000, 0, 0x80000000, 0x80000000, 0x80000000, 0,
    ],
    [
        0x55667788, 0x2ab33bc4, 0x22446688, 0x55667788, 0x2ab33bc4, 0x22446688, 0x22446688,
        0x55667788,
    ],
    [0xffffffff, 0, 4, 0, 1, 5, 31, 0],
    [0x77, 0x77, 0x7f, 0x77, 0x77, 0x7f, 0xffffffff, 0x80000077],
    [0x66, 0x67, 0x79, 0x46, 0x47, 0x59, 0xffffff99, 0x80000066],
];
fn batches(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 8]).collect();
    for (row, op) in OPS.into_iter().enumerate() {
        let (b, c, src) = match op {
            328 => (0xffffffff, 31, [258, 259, 257]),
            867 => (0, 31, [257, 259, 128]),
            334 => (0x11223344, 0x55667788, [258, 259, 257]),
            868 => (0xffffffff, 0, [257, 258, 128]),
            _ => (0x33, 0x55, [257, 258, 259]),
        };
        for dst in 1..=4 {
            for mask in [0, 0xffffffff, 0xaaaaaaaa] {
                let flag = dst % 2 == 0;
                let mut body = vec![
                    0x7e0402ff,
                    b,
                    0x7e0602ff,
                    c,
                    0x7e0802ff,
                    0x13579bdf,
                    0xbeea0381,
                    if flag { 0xbf068080 } else { 0xbf068180 },
                    0xbefe03ff,
                    mask,
                ];
                body.extend(insn(op, dst, src));
                body.extend([
                    0xbefe03c1,
                    0x7e0a026a,
                    0xbf840001,
                    29 << 25 | dst << 17 | dst << 9 | 261,
                ]);
                let code = program(&body, dst);
                let expected: Vec<_> = (0..32)
                    .map(|lane| {
                        let old = match dst {
                            1 => INPUT[lane % 8],
                            2 => b,
                            3 => c,
                            _ => 0x13579bdf,
                        };
                        let x = if mask & (1 << lane) != 0 {
                            ANSWERS[row][lane % 8]
                        } else {
                            old
                        };
                        if flag { x ^ 1 } else { x }
                    })
                    .collect();
                check(&code, &input, &expected);
                if let Some(gpu) = gpu {
                    let compiled =
                        xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code)
                            .unwrap();
                    assert_eq!(
                        gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
                            .unwrap(),
                        expected
                    );
                }
            }
        }
    }
    // Two additional literal boundary programs, batching all lanes: crossing
    // BFE and all-ones BCNT accumulator wrap. Total 74 native dispatches.
    for (op, a, b, c, want) in [
        (328, 0x80000000, 31, 2, 1),
        (868, 0xffffffff, 0xffffffe0, 0, 0),
    ] {
        let mut body = vec![0xbe8803ff, b];
        if op == 328 {
            body.extend(insn(op, 4, [255, 8, 128 + c]));
            body.push(a);
        } else {
            body.extend(insn(op, 4, [257, 255, 128]));
            body.push(b);
        }
        let code = program(&body, 4);
        check(&code, &[a; 32], &[want; 32]);
        if let Some(gpu) = gpu {
            let compiled =
                xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
            assert_eq!(
                gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[a; 32])
                    .unwrap(),
                [want; 32]
            );
        }
    }
}
#[test]
fn cpu_batched_runtime_anchors_masks_and_flags() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_bitfield_runtime_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    batches(Some(&gpu));
    let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, FIXTURE).unwrap();
    assert_eq!(
        gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &FIXTURE_INPUT)
            .unwrap(),
        FIXTURE_EXPECTED
    );
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-bitfield.bin");
const FIXTURE_INPUT: [u32; 8] = [
    0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x12345678, 0x7fa12345, 0x00000ff0,
];
// For x=12345678: BFE=67, BFM=ff0, ALIGNBIT=78000000,
// BCNT=74, OR3=78000ff4, XOR3=6a3459eb. Literal outputs only.
const FIXTURE_EXPECTED: [u32; 8] = [
    0x00000ff0, 0x01000ff0, 0x00fff0ff, 0x80000ff1, 0x80fff0fe, 0x6a3459eb, 0x3aa12c85, 0xf00000f8,
];
#[test]
fn assembled_bitfield_fixture_literal_answers() {
    check(FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}

#[test]
fn high_control_bits_pending_and_reserved_reads() {
    for offset in [0, 1, 31, 32, 33, 63, 0xffffffffu32] {
        for width in [0, 1, 31, 32, 33, 63, 0xffffffffu32] {
            let o = offset % 32;
            let w = width % 32;
            let mask = 2u64.pow(w) - 1;
            let extract = ((0xffffffffu64 / 2u64.pow(o)) & mask) as u32;
            let field = (mask * 2u64.pow(o) % 0x100000000) as u32;
            check(
                &literal_case(328, 4, offset, width),
                &[0xffffffff],
                &[extract],
            );
            check(&literal_case(867, 4, offset, 0), &[width], &[field]);
        }
    }
    for op in OPS {
        for dst in [0, 1] {
            let mut words = vec![0xe0302000, 0x80000100];
            words.extend(insn(op, dst, [128; 3]));
            words.extend([0xbf8c3f70, 0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
            reject(
                &bytes(&words),
                8,
                false,
                "writes the index register or replaces a pending load",
            );
        }
        for pos in 0..arity(op) {
            for source in [256, 257] {
                let mut src = [128; 3];
                src[pos] = source;
                let mut words = vec![0xe0302000, 0x80000100];
                words.extend(insn(op, 2, src));
                words.extend([0xbf8c3f70, 0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
                reject(&bytes(&words), 8, false, "source register");
            }
        }
    }
}

#[test]
fn every_inline_word_and_lane_derived_vcc_position() {
    let bits = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for (i, word) in bits.into_iter().enumerate() {
        for pos in 0..3 {
            for op in [882, 376] {
                let mut src = [128; 3];
                src[pos] = 240 + i as u32;
                check(&program(&insn(op, 2, src), 2), &[0], &[word]);
            }
        }
    }
    for op in OPS {
        for pos in 0..arity(op) {
            let mut src = [129; 3];
            if arity(op) == 2 {
                src[2] = 128;
            }
            src[pos] = 106;
            let mut body = vec![0x7d8a0280];
            body.extend(insn(op, 2, src));
            let answer = match op {
                328 => 0,
                867 | 868 => 2,
                334 => 0x80000000,
                _ => 1,
            };
            check(&program(&body, 2), &[1], &[answer]);
            body.push(0x7e04026a);
            check(&program(&body, 2), &[1], &[1]);
        }
    }
}
