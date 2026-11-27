// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw VOP3 programs. Literal answers are independent of production.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 6] = [361, 362, 364, 330, 877, 881];
fn binary(op: u32) -> bool {
    matches!(op, 361 | 362 | 364)
}
fn instruction(op: u32, dst: u32, sources: [u32; 3]) -> [u32; 2] {
    [
        0xd4000000 | op << 16 | dst,
        sources[0] | sources[1] << 9 | sources[2] << 18,
    ]
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
    let err = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match &err {
        // The compiler checks whole-dword framing before invoking its decoder.
        // Both references below still attribute truncation to the owning opcode.
        xenora_shader::CompileError::InvalidLength { bytes }
            if decode && !code.len().is_multiple_of(4) =>
        {
            assert_eq!(*bytes, code.len());
        }
        xenora_shader::CompileError::Decode(e) if decode && code.len().is_multiple_of(4) => {
            assert_eq!(e.offset, at);
        }
        xenora_shader::CompileError::Verify { offset, .. } if !decode => assert_eq!(*offset, at),
        _ => panic!("wrong rejection {err:?}"),
    }
    assert!(err.to_string().contains(intent), "{err}");
    for input in [0, 1] {
        let a = xenora_ref::execute(code, &[input]).unwrap_err();
        assert_eq!(a.offset, at, "{a:?}");
        let b = independent_ref::execute(code, &[input]).unwrap_err();
        assert!(b.starts_with(&format!("{at:#x}:")), "{b}");
    }
}
const PRODUCTS: &[(u32, u32, [u32; 3])] = &[
    (0, 0xffffffff, [0, 0, 0]),
    (1, 0xffffffff, [0xffffffff, 0, 0xffffffff]),
    (0x01000000, 0x100, [0, 1, 1]),
    (0xffffffff, 0xffffffff, [1, 0xfffffffe, 0]),
    (0x80000000, 2, [0, 1, 0xffffffff]),
    (0x80000000, 0x80000000, [0, 0x40000000, 0x40000000]),
    (0x7fffffff, 0x7fffffff, [1, 0x3fffffff, 0x3fffffff]),
    (0xffff0001, 0x1ffff, [0x2ffff, 0x1fffd, 0xfffffffe]),
    (0x12345678, 0x9abcdef0, [0x242d2080, 0x0b00ea4e, 0xf8cc93d6]),
];
#[test]
fn literal_products_and_ternaries_alias_every_active_source() {
    for &(a, b, answers) in PRODUCTS {
        for (op, answer) in OPS[..3].iter().copied().zip(answers) {
            for dst in [1, 2, 4, 255] {
                let mut body = vec![0x7e0402ff, b];
                body.extend(instruction(op, dst, [257, 258, 128]));
                check(&program(&body, dst), &[a], &[answer]);
            }
        }
    }
    for (op, a, b, c, answer) in [
        (330, 0x0f0f0f0f, 0xaaaaaaaa, 0x55555555, 0x5a5a5a5a),
        (330, 0x0f0f0f0f, 0x55555555, 0xaaaaaaaa, 0xa5a5a5a5),
        (330, 0, 0xdeadbeef, 0x7fa12345, 0x7fa12345),
        (330, 0xffffffff, 0x80000000, 0, 0x80000000),
        (877, 0xffffffff, 1, 7, 7),
        (877, 0xffffffff, 0xffffffff, 0xffffffff, 0xfffffffd),
        (877, 0x7fffffff, 1, 0, 0x80000000),
        (877, 0xffff, 1, 0x10000, 0x20000),
        (881, 15, 0x33, 0x80, 0x83),
        (881, 0xffffffff, 0x7fa12345, 0, 0x7fa12345),
        (881, 0, 0xffffffff, 0x80000000, 0x80000000),
    ] {
        for dst in [1, 2, 3, 4, 255] {
            let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
            body.extend(instruction(op, dst, [257, 258, 259]));
            check(&program(&body, dst), &[a], &[answer]);
        }
    }
    for bit in 0..32 {
        let one = 2u32.pow(bit);
        for (op, sources, expected) in [
            (330, [257, 193, 128], one),
            (330, [257, 128, 193], !one),
            (877, [257, 128, 128], one),
            (881, [257, 193, 128], one),
        ] {
            check(
                &program(&instruction(op, 2, sources), 2),
                &[one],
                &[expected],
            );
        }
    }
}
#[test]
fn shared_literals_and_scalar_bus_identity_not_values() {
    for op in OPS {
        let arity = if binary(op) { 2 } else { 3 };
        for raw in [
            [8, 8, 8],
            [8, 9, 8],
            [255, 255, 255],
            [8, 255, 255],
            [106, 8, 106],
            [8, 9, 129],
        ] {
            let mut raw = raw;
            if arity == 2 {
                raw[2] = 128;
            }
            // Every active source is one; expected values distinguish op arity.
            let answer = match op {
                361 | 330 | 881 => 1,
                362 | 364 => 0,
                _ => 3,
            };
            let mut body = vec![0xbe880381, 0xbe890381, 0xbeea0381];
            body.extend(instruction(op, 2, raw));
            if raw[..arity].contains(&255) {
                body.push(1);
            }
            check(&program(&body, 2), &[0], &[answer]);
        }
    }
    for op in [330, 877, 881] {
        for mut raw in [[8, 9, 10], [8, 9, 255], [106, 8, 255]] {
            for _ in 0..3 {
                let mut body = vec![0xbe880381, 0xbe890381, 0xbe8a0381, 0xbeea0381, 0xbefe0380];
                body.extend(instruction(op, 2, raw));
                if raw.contains(&255) {
                    body.push(1);
                }
                reject(&program(&body, 1), 32, true, "scalar-bus");
                raw.rotate_left(1);
            }
        }
    }
}
#[test]
fn each_position_admits_source_classes_and_raw_inline_words() {
    let words = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for (i, bits) in words.into_iter().enumerate() {
        // MUL_LO by one and ADD3 with zeros return the selected bits literally.
        for op in [361, 877] {
            let n = if binary(op) { 2 } else { 3 };
            for pos in 0..n {
                let mut src = if binary(op) {
                    [129, 129, 128]
                } else {
                    [128; 3]
                };
                src[pos] = 240 + i as u32;
                check(&program(&instruction(op, 2, src), 2), &[0], &[bits]);
            }
        }
    }
    for op in OPS {
        for pos in 0..if binary(op) { 2 } else { 3 } {
            for selector in [
                8, 105, 106, 128, 129, 192, 193, 208, 255, 257, 496, 504, 511,
            ] {
                // All other inputs zero: every product/AND_OR is zero except
                // AND_OR SRC2; BFI returns SRC2; ADD3 returns the selected word.
                let bits = match selector {
                    128 => 0,
                    192 => 64,
                    193 => 0xffffffff,
                    208 => 0xfffffff0,
                    _ => 1,
                };
                let mut raw = [128; 3];
                raw[pos] = selector;
                let mut body = vec![0xbe880381, 0xbee90381, 0xbeea0381];
                if selector >= 496 {
                    body.extend([0x7e0002ff | (selector - 256) << 17, bits]);
                }
                body.extend(instruction(op, 2, raw));
                if selector == 255 {
                    body.push(bits);
                }
                let expected = if op == 877 || (matches!(op, 330 | 881) && pos == 2) {
                    bits
                } else {
                    0
                };
                check(&program(&body, 2), &[1], &[expected]);
            }
        }
    }
}
#[test]
fn decode_exclusions_lengths_and_padding_are_checked_before_exec() {
    for op in OPS {
        for pos in 0..if binary(op) { 2 } else { 3 } {
            for bad in (107..=127).chain(209..=239).chain(249..=254) {
                let mut raw = [128; 3];
                raw[pos] = bad;
                let mut body = vec![0xbefe0380];
                body.extend(instruction(op, 2, raw));
                reject(&program(&body, 1), 16, true, "");
            }
            let mut raw = [128; 3];
            raw[pos] = 255;
            let encoded = bytes(&instruction(op, 2, raw));
            for available in 0..4 {
                let mut truncated = encoded.clone();
                truncated.extend_from_slice(&[1, 2, 3][..available]);
                reject(&truncated, 0, true, "");
            }
        }
        for count in 4..8 {
            reject(&bytes(&instruction(op, 2, [128; 3]))[..count], 0, true, "");
        }
        for bit in (8..16).chain(27..32) {
            let mut insn = instruction(op, 2, [128; 3]);
            insn[usize::from(bit >= 27)] |= 1 << bit;
            reject(&program(&insn, 1), 12, true, "modifier");
        }
        if binary(op) {
            for padding in [0, 106, 129, 255, 511] {
                reject(
                    &program(&instruction(op, 2, [128, 128, padding]), 1),
                    12,
                    true,
                    "SRC2",
                );
            }
        }
    }
}
#[test]
fn exec_readiness_and_runtime_regions_keep_their_boundaries() {
    for op in OPS {
        let n = if binary(op) { 2 } else { 3 };
        for pos in 0..n {
            for bad in [20, 106, 265] {
                let mut raw = [128; 3];
                raw[pos] = bad;
                let mut body = vec![0x7e040287, 0xbefe0380];
                body.extend(instruction(op, 2, raw));
                body.push(0xbefe03c1);
                check(&program(&body, 2), &[0], &[7]);
                for mask in [1, 0x80000000, 0xffffffff] {
                    let mut body = vec![0x7e040287, 0xbefe03ff, mask];
                    body.extend(instruction(op, 2, raw));
                    body.push(0xbefe03c1);
                    reject(
                        &program(&body, 2),
                        24,
                        false,
                        if bad == 20 {
                            "no known value"
                        } else {
                            "uninitialized"
                        },
                    );
                }
            }
        }
        let mut body = vec![0xbefe0381];
        body.extend(instruction(op, 2, [128; 3]));
        body.push(0xbefe03c1);
        reject(&program(&body, 2), 16, false, "partial EXEC");
        for mask in [0, 1, 0xffffffff] {
            // Ready operands and destination: rejection must be the region rule.
            let mut body = vec![0x7e040287, 0x7d8a0280, 0xbefe03ff, mask, 0xbf860002];
            body.extend(instruction(
                op,
                2,
                [257, 257, if binary(op) { 128 } else { 257 }],
            ));
            body.push(0xbefe03c1);
            reject(&program(&body, 2), 32, false, "runtime");
        }
    }
}
const INPUT: [u32; 8] = [
    0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x01000000, 0x7fa12345, 0xffff0001,
];
const ANSWERS: [[u32; 8]; 6] = [
    [
        0, 0xffffffff, 1, 0x80000000, 0x80000001, 0xff000000, 0x805edcbb, 0xffff,
    ],
    [
        0, 0, 0xfffffffe, 0x7fffffff, 0x7ffffffe, 0xffffff, 0x7fa12344, 0xffff0000,
    ],
    [0, 0xffffffff, 0, 0, 0xffffffff, 0xffffffff, 0xffffffff, 0],
    [
        0x80000000, 0x80000001, 0xffffffff, 0x80000000, 0xffffffff, 0x81000000, 0xffa12345,
        0xffff0001,
    ],
    [
        0x7fffffff, 0x80000000, 0x7ffffffe, 0xffffffff, 0xfffffffe, 0x80ffffff, 0xffa12344,
        0x7fff0000,
    ],
    [
        0x80000000, 0x80000001, 0xffffffff, 0x80000000, 0xffffffff, 0x81000000, 0xffa12345,
        0xffff0001,
    ],
];
fn batched_case(op: u32, dst: u32, mask: u32, scalar: bool, flag: bool) -> Vec<u8> {
    let mut body = vec![
        0x7e0402c1,
        0x7e0602ff,
        0x80000000,
        0x7e0802ff,
        0x13579bdf,
        0xbe8803c1,
        0xbeea0381,
        if flag { 0xbf068080 } else { 0xbf068180 },
        0xbefe03ff,
        mask,
    ];
    let raw = if scalar {
        [257, 8, if binary(op) { 128 } else { 255 }]
    } else {
        [257, 258, if binary(op) { 128 } else { 259 }]
    };
    body.extend(instruction(op, dst, raw));
    if scalar && !binary(op) {
        body.push(0x80000000);
    }
    body.extend([
        0xbefe03c1,
        0x7e0a026a,
        0xbf840001,
        29 << 25 | dst << 17 | dst << 9 | 261,
    ]);
    program(&body, dst)
}
#[test]
fn batched_masks_and_flags_have_literal_cpu_answers() {
    run_batches(None);
}
fn run_batches(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 8]).collect();
    // 72 alias/mask programs plus six scalar/literal representatives = 78.
    for (row, op) in OPS.into_iter().enumerate() {
        let cases = (1..=4)
            .flat_map(|dst| [0, 0xffffffff, 0xaaaaaaaa].map(move |mask| (dst, mask, false)))
            .chain(std::iter::once((4, 0xffffffff, true)));
        for (dst, mask, scalar) in cases {
            let flag = dst % 2 == 0;
            let code = batched_case(op, dst, mask, scalar, flag);
            let expected: Vec<_> = (0..32)
                .map(|lane| {
                    let old = match dst {
                        1 => INPUT[lane % 8],
                        2 => 0xffffffff,
                        3 => 0x80000000,
                        _ => 0x13579bdf,
                    };
                    let value = if mask & (1 << lane) != 0 {
                        ANSWERS[row][lane % 8]
                    } else {
                        old
                    };
                    if flag { value ^ 1 } else { value }
                })
                .collect();
            check(&code, &input, &expected);
            if let Some(gpu) = gpu {
                let compiled =
                    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
                assert_eq!(
                    gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
                        .unwrap(),
                    expected
                );
            }
        }
    }
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_vop3_integer_batched_literal_answers() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    run_batches(Some(&gpu));
    let fixture = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, FIXTURE).unwrap();
    assert_eq!(
        gpu.run_words(&fixture.msl, xenora_shader::ENTRY_POINT, &INPUT)
            .unwrap(),
        FIXTURE_ANSWERS
    );
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-integer.bin");
// Assembly multiplier is 80000001; BFI mask is 0f0f0f0f. For x=7fa12345:
// lo=ffa12345, hi_u=3fd091a2, hi_i=c02f6e5d, BFI=3fd193a5,
// ADD3=00010203, final AND_OR=3fd193a3. Expectations are literal words.
const FIXTURE_ANSWERS: [u32; 8] = [
    1, 1, 0x8f0f0f00, 0x40000000, 0x3fffffff, 0x01800001, 0x3fd193a3, 0x7fff8001,
];
#[test]
fn assembled_vop3_fixture_literal_answers() {
    check(FIXTURE, &INPUT, &FIXTURE_ANSWERS);
}

#[test]
fn unset_flags_pending_reads_and_reserved_registers_stay_invalid() {
    for op in OPS {
        let insn = instruction(op, 2, [128; 3]);
        let mut body = insn.to_vec();
        body.extend([0xbf840001, 0x7e040280]);
        reject(&program(&body, 2), 20, false, "SCC");
        let mut body = insn.to_vec();
        body.push(0x7e04026a);
        reject(&program(&body, 2), 20, false, "known value");
        for dst in [0, 1] {
            let insn = instruction(op, dst, [128; 3]);
            let mut words = vec![0xe0302000, 0x80000100];
            words.extend(insn);
            words.extend([0xbf8c3f70, 0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
            reject(
                &bytes(&words),
                8,
                false,
                "writes the index register or replaces a pending load",
            );
        }
        for pos in 0..if binary(op) { 2 } else { 3 } {
            for src in [256, 257] {
                let mut raw = [128; 3];
                raw[pos] = src;
                let mut words = vec![0xe0302000, 0x80000100];
                words.extend(instruction(op, 2, raw));
                words.extend([0xbf8c3f70, 0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
                reject(&bytes(&words), 8, false, "source register");
            }
        }
        // All coincident active sources: literal answers differ by operation.
        let raw = [257, 257, if binary(op) { 128 } else { 257 }];
        let answer = match op {
            361 | 330 | 881 => 1,
            362 | 364 => 0,
            _ => 3,
        };
        check(&program(&instruction(op, 1, raw), 1), &[1], &[answer]);
    }
}

#[test]
fn lane_derived_vcc_sources_and_exec_preservation() {
    for op in OPS {
        for position in 0..if binary(op) { 2 } else { 3 } {
            let mut raw = [129; 3];
            if binary(op) {
                raw[2] = 128;
            }
            raw[position] = 106;
            let mut body = vec![0x7d8a0280]; // lane-derived VCC=1 for the one input lane
            body.extend(instruction(op, 2, raw));
            let answer = match op {
                361 | 330 | 881 => 1,
                362 | 364 => 0,
                _ => 3,
            };
            check(&program(&body, 2), &[1], &[answer]);
        }
        for mask in [0, 1, 0xffffffff] {
            let mut body = vec![0x7e040287, 0x7e060280, 0xbefe03ff, mask];
            body.extend(instruction(op, 3, [128; 3]));
            body.extend([0xbf880001, 0x7e040289, 0xbefe03c1]);
            check(&program(&body, 2), &[0], &[if mask == 0 { 7 } else { 9 }]);
        }
    }
}
