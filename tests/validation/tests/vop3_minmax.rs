// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte min/max/median fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 6] = [338, 339, 341, 342, 344, 345];
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
const PERM: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];
// Each row: inputs; signed minimum/median/maximum; unsigned counterparts.
const ANCHORS: &[([u32; 3], [u32; 3], [u32; 3])] = &[
    (
        [0x80000000, 0, 0x7fffffff],
        [0x80000000, 0, 0x7fffffff],
        [0, 0x7fffffff, 0x80000000],
    ),
    ([0xffffffff, 0, 1], [0xffffffff, 0, 1], [0, 1, 0xffffffff]),
    (
        [0x80000000, 0xffffffff, 0x7fffffff],
        [0x80000000, 0xffffffff, 0x7fffffff],
        [0x7fffffff, 0x80000000, 0xffffffff],
    ),
    (
        [0x80000000, 0x80000000, 1],
        [0x80000000, 0x80000000, 1],
        [1, 0x80000000, 0x80000000],
    ),
    (
        [0, 0xffffffff, 0xffffffff],
        [0xffffffff, 0xffffffff, 0],
        [0, 0xffffffff, 0xffffffff],
    ),
    (
        [0x7fa12345, 0x80000001, 0xffffffff],
        [0x80000001, 0xffffffff, 0x7fa12345],
        [0x7fa12345, 0x80000001, 0xffffffff],
    ),
    (
        [0xaaaaaaaa, 0x55555555, 0],
        [0xaaaaaaaa, 0, 0x55555555],
        [0, 0x55555555, 0xaaaaaaaa],
    ),
    ([0x80000000; 3], [0x80000000; 3], [0x80000000; 3]),
];
fn rank(op: u32) -> usize {
    match op {
        338 | 339 => 0,
        341 | 342 => 2,
        344 | 345 => 1,
        _ => unreachable!(),
    }
}
fn signed(op: u32) -> bool {
    matches!(op, 338 | 341 | 344)
}
fn literal_case(op: u32, dst: u32, b: u32, c: u32) -> Vec<u8> {
    let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
    body.extend(insn(op, dst, [257, 258, 259]));
    program(&body, dst)
}
// Independent test-local specification: widened mathematical values and an
// extrema network. No production/reference evaluator supplies expectations.
fn spec(op: u32, a: u32, b: u32, c: u32) -> u32 {
    let cv = |x: u32| {
        if signed(op) {
            i64::from(x as i32)
        } else {
            i64::from(x)
        }
    };
    let (a, b, c) = (cv(a), cv(b), cv(c));
    (match rank(op) {
        0 => a.min(b).min(c),
        2 => a.max(b).max(c),
        _ => a.min(b).max(a.max(b).min(c)),
    }) as u32
}
#[test]
fn literal_all_permutations_ties_and_aliases() {
    for &(values, si, un) in ANCHORS {
        for op in OPS {
            for p in PERM {
                let [a, b, c] = p.map(|i| values[i]);
                let want = if signed(op) { si } else { un }[rank(op)];
                for dst in [1, 2, 3, 4, 255] {
                    check(&literal_case(op, dst, b, c), &[a], &[want]);
                }
            }
        }
    }
}
#[test]
fn finite_boundary_cartesian_product() {
    let words = [
        0, 1, 0x7fffffff, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff,
    ];
    for op in OPS {
        for b in words {
            for c in words {
                let expected: Vec<_> = words.iter().map(|&a| spec(op, a, b, c)).collect();
                check(&literal_case(op, 4, b, c), &words, &expected);
            }
        }
    }
}
#[test]
fn raw_sources_identity_literals_and_decode_boundaries() {
    for op in OPS {
        for pos in 0..3 {
            for bad in (108..=125).chain(209..=239).chain(249..=254) {
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
            let want = 1;
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
            let want = 1;
            check(&program(&body, 2), &[1], &[want]);
            body.push(0x7e04026a);
            check(&program(&body, 2), &[1], &[1]);
        }
    }
}

#[test]
fn positive_masks_all_aliases_and_unset_flags() {
    for op in OPS {
        for dst in 1..=4 {
            for mask in [0, 1, 0x80000000, 0xaaaaaaaa, 0xffffffff] {
                let mut body = vec![
                    0x7e040280, 0x7e0602ff, 0x80000000, 0x7e0802ff, 0x13579bdf, 0xbefe03ff, mask,
                ];
                body.extend(insn(op, dst, [257, 258, 259]));
                body.push(0xbefe03c1);
                let answer = match op {
                    338 => 0x80000000,
                    339 => 0,
                    341 => 1,
                    342 => 0x80000000,
                    344 => 0,
                    345 => 1,
                    _ => unreachable!(),
                };
                let old = match dst {
                    1 => 1,
                    2 => 0,
                    3 => 0x80000000,
                    _ => 0x13579bdf,
                };
                let expected: Vec<_> = (0..32)
                    .map(|lane| if mask & (1 << lane) != 0 { answer } else { old })
                    .collect();
                check(&program(&body, dst), &[1; 32], &expected);
            }
        }
    }
}
const INPUT: [u32; 8] = [
    0, 1, 0x7fffffff, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff, 0x7fa12345,
];
// Fixed answers to order(x,0,INT_MIN), one row per opcode in OPS.
const ANSWERS: [[u32; 8]; 6] = [
    [0x80000000; 8],
    [0; 8],
    [0, 1, 0x7fffffff, 0, 0, 0, 0, 0x7fa12345],
    [
        0x80000000, 0x80000000, 0x80000000, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff,
        0x80000000,
    ],
    [0, 0, 0, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff, 0],
    [
        0, 1, 0x7fffffff, 0x80000000, 0x80000000, 0x80000000, 0x80000000, 0x7fa12345,
    ],
];
fn run_case(
    gpu: Option<&xenora_metal::NativeCompute>,
    code: &[u8],
    input: &[u32],
    expected: &[u32],
) {
    check(code, input, expected);
    if let Some(gpu) = gpu {
        let c = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
        assert_eq!(
            gpu.run_words(&c.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap(),
            expected
        );
    }
}
fn batches(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 8]).collect();
    // 72 programs: six operations, six permutations, full and rotating
    // zero/alternating/lane31 EXEC controls.
    // The destination cycles through all three aliases and a distinct register.
    for (row, op) in OPS.into_iter().enumerate() {
        for (pi, p) in PERM.into_iter().enumerate() {
            for mask in [0xffffffff, [0, 0xaaaaaaaa, 0x80000000][pi % 3]] {
                let dst = 1 + (pi % 4) as u32;
                let flag = pi % 2 == 0;
                let mut body = vec![
                    0x7e040280,
                    0x7e0602ff,
                    0x80000000,
                    0x7e0802ff,
                    0x13579bdf,
                    0xbeea0381,
                    if flag { 0xbf068080 } else { 0xbf068180 },
                    0xbefe03ff,
                    mask,
                ];
                body.extend(insn(op, dst, p.map(|i| 257 + i as u32)));
                body.extend([
                    0xbefe03c1,
                    0x7e0a026a,
                    0xbf840001,
                    29 << 25 | dst << 17 | dst << 9 | 261,
                ]);
                let expected: Vec<_> = (0..32)
                    .map(|i| {
                        let old = match dst {
                            1 => INPUT[i % 8],
                            2 => 0,
                            3 => 0x80000000,
                            _ => 0x13579bdf,
                        };
                        let out = if mask & (1 << i) != 0 {
                            ANSWERS[row][i % 8]
                        } else {
                            old
                        };
                        if flag { out ^ 1 } else { out }
                    })
                    .collect();
                run_case(gpu, &program(&body, dst), &input, &expected);
            }
        }
    }
    // Six mixed scalar/literal/inline programs with NaN-shaped integer words.
    for (op, want) in [
        (338, 0xffffffff),
        (339, 0),
        (341, 0x7fa12345),
        (342, 0xffffffff),
        (344, 0),
        (345, 0x7fa12345),
    ] {
        let mut body = vec![0xbe8803ff, 0x7fa12345];
        body.extend(insn(op, 4, [8, 255, 128]));
        body.push(0xffffffff);
        run_case(gpu, &program(&body, 4), &[0; 32], &[want; 32]);
    }
    // Six all-equal shared-literal cases preserve the exact NaN-shaped word.
    for op in OPS {
        let mut body = insn(op, 4, [255; 3]).to_vec();
        body.push(0x7fa12345);
        run_case(gpu, &program(&body, 4), &[0; 32], &[0x7fa12345; 32]);
    }
    // One actual assembled fixture program: 85 native dispatches total.
    run_case(gpu, FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
#[test]
fn cpu_batched_permutations_flags_and_sources() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_minmax_runtime_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    batches(Some(&xenora_metal::NativeCompute::new().unwrap()));
}
#[test]
fn adjacent_float_opcodes_stay_rejected() {
    for op in [337, 340, 343] {
        reject(&program(&insn(op, 2, [128; 3]), 2), 12, true, "");
    }
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-minmax.bin");
const FIXTURE_INPUT: [u32; 12] = [
    0x0, 0x1, 0x7fffffff, 0x80000000, 0x80000001, 0xfffffffe, 0xffffffff, 0x7fa12345, 0x7ffffffe,
    0x2, 0xaaaaaaaa, 0x55555555,
];
// Assembly independently reduces to the sum of its six selected words.
// At x=0: 80000001+0+7ffffffe+80000001+0+7ffffffe = fffffffe.
const FIXTURE_EXPECTED: [u32; 12] = [
    0xfffffffe, 0x0, 0x7fffffff, 0xfffffffe, 0x0, 0x7ffffff7, 0x7ffffffa, 0x7fa12344, 0x7ffffffd,
    0x1, 0x7ffffffb, 0x55555554,
];
#[test]
fn actual_assembled_fixture_literal_anchors() {
    check(FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}

#[test]
fn both_incoming_flag_values_survive() {
    for op in OPS {
        for vcc in [0, 1] {
            for scc in [false, true] {
                let mut body = vec![0xbeea0380 | vcc, if scc { 0xbf068080 } else { 0xbf068180 }];
                body.extend(insn(op, 2, [129; 3]));
                body.extend([0x7e0a026a, 0xbf840001, 29 << 25 | 2 << 17 | 2 << 9 | 261]);
                check(&program(&body, 2), &[0], &[if scc { 1 ^ vcc } else { 1 }]);
            }
        }
    }
}
