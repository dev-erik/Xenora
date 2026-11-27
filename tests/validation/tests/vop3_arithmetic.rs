// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte signed-field/shift arithmetic fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 4] = [329, 838, 839, 879];
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
    (329, 8, 0, 4, 0xfffffff8),
    (329, 7, 0, 4, 7),
    (329, 0x80000000, 31, 2, 0xffffffff),
    (329, 0x80000000, 0, 31, 0),
    (329, 0xffffffff, 0, 32, 0),
    (329, 1, 0, 33, 0xffffffff),
    (329, 0x40000000, 30, 2, 1),
    (329, 0xc0000000, 30, 2, 0xffffffff),
    (838, 3, 4, 5, 0x35),
    (839, 3, 4, 5, 0xe0),
    (879, 3, 4, 5, 0x35),
    (838, 3, 1, 2, 8),
    (879, 3, 1, 2, 6),
    (838, 0xffffffff, 1, 2, 0),
    (839, 0xffffffff, 1, 31, 0),
    (879, 0x80000001, 32, 4, 0x80000005),
    (838, 0x80000001, 33, 0xffffffff, 1),
    (839, 0x7fffffff, 1, 33, 0),
];
fn literal_case(op: u32, dst: u32, b: u32, c: u32) -> Vec<u8> {
    let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
    body.extend(insn(op, dst, [257, 258, 259]));
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
// A test-local signed shift specification, separate from both reference
// algorithms. Literal anchors above pin zero width and crossing fields.
fn spec(op: u32, a: u32, b: u32, c: u32) -> u32 {
    match op {
        329 => {
            let o = b & 31;
            let w = c & 31;
            if w == 0 {
                0
            } else if o + w >= 32 {
                ((a as i32) >> o) as u32
            } else {
                (((a << (32 - o - w)) as i32) >> (32 - w)) as u32
            }
        }
        838 => ((u128::from(a) << (b % 32)) + u128::from(c)) as u32,
        839 => ((u128::from(a) + u128::from(b)) << (c % 32)) as u32,
        879 => ((u64::from(a) << (b % 32)) | u64::from(c)) as u32,
        _ => unreachable!(),
    }
}
#[test]
fn cpu_boundaries_one_hot_complements_and_masked_counts() {
    let mut data = vec![0, 0xffffffff, 0x80000000, 0x7fffffff, 0x7fa12345];
    for bit in 0..32 {
        data.extend([1 << bit, !(1 << bit)]);
    }
    for b in 0..32 {
        for c in 0..32 {
            let expected: Vec<_> = data.iter().map(|&a| spec(329, a, b, c)).collect();
            check(&literal_case(329, 4, b, c), &data, &expected);
        }
    }
    for op in OPS {
        for count in (0..32).chain([32, 33, 63, u32::MAX]) {
            let (b, c) = if op == 839 {
                (0xffffffff, count)
            } else {
                (count, 33)
            };
            let expected: Vec<_> = data.iter().map(|&a| spec(op, a, b, c)).collect();
            check(&literal_case(op, 4, b, c), &data, &expected);
        }
    }
}
#[test]
fn raw_sources_identity_literals_and_decode_boundaries() {
    for op in OPS {
        for pos in 0..3 {
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
                // count controls use x=1 and the separate test-local word specification.
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
                let mut values = [0; 3];
                values[pos] = value;
                let answer = spec(op, values[0], values[1], values[2]);
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
        for src in [
            [8, 8, 8],
            [8, 9, 8],
            [255, 255, 255],
            [106, 8, 106],
            [8, 255, 255],
            [8, 9, 129],
        ] {
            let mut body = vec![0xbe880381, 0xbe890381, 0xbeea0381];
            body.extend(insn(op, 2, src));
            if src[..3].contains(&255) {
                body.push(1);
            }
            let want = match op {
                329 => 0,
                838 => 3,
                839 => 4,
                879 => 3,
                _ => unreachable!(),
            };
            check(&program(&body, 2), &[0], &[want]);
        }
        {
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
        for pos in 0..3 {
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
            body.extend(insn(op, 2, [257, 257, 257]));
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
// Runtime width for signed BFE; runtime shift count for the three shift forms.
// BFE extracts all ones from offset31. Shift operands are 3 and 2.
const ANSWERS: [[u32; 8]; 4] = [
    [
        0, 0xffffffff, 0xffffffff, 0, 0xffffffff, 0xffffffff, 0xffffffff, 0,
    ],
    [5, 8, 0x80000002, 5, 8, 0x80000002, 0x80000002, 5],
    [5, 10, 0x80000000, 5, 10, 0x80000000, 0x80000000, 5],
    [3, 6, 0x80000002, 3, 6, 0x80000002, 0x80000002, 3],
];
fn run_case(
    gpu: Option<&xenora_metal::NativeCompute>,
    code: &[u8],
    input: &[u32],
    expected: &[u32],
) {
    check(code, input, expected);
    if let Some(gpu) = gpu {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap(),
            expected
        );
    }
}
fn batches(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 8]).collect();
    // 48 dispatches: four forms, all three aliases plus distinct, three masks.
    for (row, op) in OPS.into_iter().enumerate() {
        let (b, c, src) = match op {
            329 => (0xffffffff, 31, [258, 259, 257]),
            839 => (3, 2, [258, 259, 257]),
            _ => (3, 2, [258, 257, 259]),
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
                let expected: Vec<_> = (0..32)
                    .map(|lane| {
                        let old = match dst {
                            1 => INPUT[lane % 8],
                            2 => b,
                            3 => c,
                            _ => 0x13579bdf,
                        };
                        let result = if mask & (1 << lane) != 0 {
                            ANSWERS[row][lane % 8]
                        } else {
                            old
                        };
                        if flag { result ^ 1 } else { result }
                    })
                    .collect();
                run_case(gpu, &program(&body, dst), &input, &expected);
            }
        }
    }
    // Eight source-class/discriminator dispatches. Expectations remain literals.
    for (op, a, b, c, want) in [
        (329, 0x80000000, 31, 2, 0xffffffff),
        (329, 8, 0, 4, 0xfffffff8),
        (838, 3, 1, 2, 8),
        (838, 0xffffffff, 1, 2, 0),
        (839, 3, 4, 5, 0xe0),
        (839, 0xffffffff, 1, 31, 0),
        (879, 3, 1, 2, 6),
        (879, 0x7fa12345, 0, 0, 0x7fa12345),
    ] {
        let mut body = vec![0xbe8803ff, b];
        body.extend(insn(op, 4, [255, 8, 128 + c]));
        body.push(a);
        run_case(gpu, &program(&body, 4), &[a; 32], &[want; 32]);
    }
    run_case(gpu, FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
#[test]
fn cpu_batched_runtime_masks_flags_and_fixture() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_arithmetic_runtime_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    batches(Some(&xenora_metal::NativeCompute::new().unwrap()));
}
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-arithmetic.bin");
const FIXTURE_INPUT: [u32; 8] = [
    0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x12345678, 0x7fa12345, 0x00000ff0,
];
// Assembly: f is signed byte bits4..11; v3=2*x+f, v4=8*(v3+f), v5=2*v4 OR f.
const FIXTURE_EXPECTED: [u32; 8] = [
    0x00000000, 0x00000020, 0xffffffff, 0x00000000, 0xffffffff, 0x468adbe7, 0xf4246f34, 0xffffffff,
];
#[test]
fn pending_and_reserved_reads() {
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
        for pos in 0..3 {
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
fn all_inline_float_words_and_vcc_sources() {
    let words = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for op in OPS {
        for pos in 0..3 {
            for (i, word) in words.into_iter().enumerate() {
                let mut src = [129; 3];
                src[pos] = 240 + i as u32;
                let mut values = [1; 3];
                values[pos] = word;
                check(
                    &program(&insn(op, 2, src), 2),
                    &[0],
                    &[spec(op, values[0], values[1], values[2])],
                );
            }
            let mut src = [129; 3];
            src[pos] = 106;
            let mut body = vec![0x7d8a0280];
            body.extend(insn(op, 2, src));
            let want = match op {
                329 => 0,
                838 | 879 => 3,
                839 => 4,
                _ => unreachable!(),
            };
            check(&program(&body, 2), &[1], &[want]);
            body.push(0x7e04026a);
            check(&program(&body, 2), &[1], &[1]);
        }
    }
}

#[test]
fn lane31_and_each_runtime_source_position() {
    // Positive lane31 write catches top-bit EXEC handling, independently of
    // the full/alternating native batches. Runtime values exercise every input.
    for &(op, a, b, c, want) in ANCHORS {
        for pos in 0..3 {
            let values = [a, b, c];
            let mut body = vec![
                0x7e0402ff, a, 0x7e0602ff, b, 0x7e0802ff, c, 0x7e0a02ff, 0x13579bdf,
            ];
            let mut src = [258, 259, 260];
            src[pos] = 257;
            body.extend([0xbefe03ff, 0x80000000]);
            body.extend(insn(op, 5, src));
            body.push(0xbefe03c1);
            let mut expected = [0x13579bdf; 32];
            expected[31] = want;
            check(&program(&body, 5), &[values[pos]; 32], &expected);
        }
    }
}
