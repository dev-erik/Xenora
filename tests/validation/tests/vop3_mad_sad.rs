// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte MAD24/SAD wrap and clamp fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
// Bit16 in the test-only operation key requests clamp; it is not an ISA opcode.
const OPS: [u32; 12] = [
    322, 323, 346, 347, 348, 349, 0x10142, 0x10143, 0x1015a, 0x1015b, 0x1015c, 0x1015d,
];
fn insn(op: u32, dst: u32, src: [u32; 3]) -> [u32; 2] {
    [
        0xd4000000 | (op & 1023) << 16 | (op >> 1 & 0x8000) | dst,
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
fn spec(op: u32, a: u32, b: u32, c: u32) -> u32 {
    let signed24 = |x: u32| i128::from(((x << 8) as i32) >> 8);
    let kind = op & 1023;
    let value = match kind {
        322 => signed24(a) * signed24(b) + i128::from(c as i32),
        323 => i128::from(a & 0xffffff) * i128::from(b & 0xffffff) + i128::from(c),
        _ => {
            let width = match kind {
                346 | 347 => 8,
                348 => 16,
                349 => 32,
                _ => unreachable!(),
            };
            let mask = (1u64 << width) - 1;
            let mut total = 0i128;
            for lane in 0..32 / width {
                let x = ((u64::from(a) >> (lane * width)) & mask) as i128;
                let y = ((u64::from(b) >> (lane * width)) & mask) as i128;
                total += (x - y).abs();
            }
            total * (if kind == 347 { 65536 } else { 1 }) + i128::from(c)
        }
    };
    if op & 0x10000 == 0 {
        value as u32
    } else if kind == 322 {
        value.clamp(-2147483648, 2147483647) as u32
    } else {
        value.min(4294967295) as u32
    }
}
// opcode,a,b,c,wrap,clamp. Explicit complete-result anchors, no production oracle.
const ANCHORS: &[(u32, u32, u32, u32, u32, u32)] = &[
    (322, 0xffffff, 2, 3, 1, 1),
    (323, 0xffffff, 2, 3, 0x2000001, 0x2000001),
    (322, 0x800000, 2, 0, 0xff000000, 0xff000000),
    (323, 0x800000, 2, 0, 0x1000000, 0x1000000),
    (322, 0x7fffff, 0x7fffff, 0xffffffff, 0xff000000, 0x7fffffff),
    (323, 0x7fffff, 0x7fffff, 0xffffffff, 0xff000000, 0xffffffff),
    (322, 0x8000, 0x10000, 0xffffffff, 0x7fffffff, 0x7fffffff),
    (322, 0xff7fff, 0x10000, 0x10000, 0x80000000, 0x80000000),
    (322, 0xff7fff, 0x10000, 0xffff, 0x7fffffff, 0x80000000),
    (322, 0x8000, 0x10000, 0, 0x80000000, 0x7fffffff),
    (323, 0x10000, 0x10000, 0, 0, 0xffffffff),
    (346, 0x00112233, 0x33221100, 7, 0x8f, 0x8f),
    (347, 0x00112233, 0x33221100, 7, 0x880007, 0x880007),
    (346, 0xffffffff, 0, 0xffffffff, 0x3fb, 0xffffffff),
    (347, 0xffffffff, 0, 0xffffffff, 0x3fbffff, 0xffffffff),
    (347, 0xffffffff, 0, 7, 0x3fc0007, 0x3fc0007),
    (348, 0xffffffff, 0, 0xffffffff, 0x1fffd, 0xffffffff),
    (349, 0xffffffff, 0, 1, 0, 0xffffffff),
    (349, 0x80000000, 0, 0, 0x80000000, 0x80000000),
    (349, 0, 0xffffffff, 0, 0xffffffff, 0xffffffff),
];
fn literal_case(op: u32, dst: u32, b: u32, c: u32) -> Vec<u8> {
    let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
    body.extend(insn(op, dst, [257, 258, 259]));
    program(&body, dst)
}
#[test]
fn literal_cancellation_final_clamp_and_all_aliases() {
    for &(base, a, b, c, wrap, sat) in ANCHORS {
        for (op, want) in [(base, wrap), (base | 0x10000, sat)] {
            for dst in [1, 2, 3, 4, 255] {
                check(&literal_case(op, dst, b, c), &[a], &[want]);
            }
            check(&literal_case(op, 4, a, c), &[b], &[want]);
            if matches!(base, 322 | 323) {
                check(
                    &literal_case(op, 4, b | 0xab000000, c),
                    &[a | 0xff000000],
                    &[want],
                );
            }
        }
    }
}
#[test]
fn isolated_components_equal_values_and_accumulator_carry() {
    for op in OPS {
        for bit in 0..32 {
            let one = 1 << bit;
            let data = [0, one, !one, u32::MAX, 0x80000000, 0x7fffffff];
            for b in [one, !one] {
                for c in [0, 1, 0xffff, 0x10000, u32::MAX] {
                    let wants: Vec<_> = data.iter().map(|&a| spec(op, a, b, c)).collect();
                    check(&literal_case(op, 4, b, c), &data, &wants);
                }
            }
        }
        if matches!(op & 1023, 346..=349) {
            for x in [0, u32::MAX, 0x7fa12345] {
                for c in [0, 7, 0x80000000, u32::MAX] {
                    check(&literal_case(op, 4, x, c), &[x], &[c]);
                }
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
        for bit in (8..15).chain(27..32) {
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
            let want = spec(op, 1, 1, 1);
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
            let want = spec(op, 1, 1, 1);
            check(&program(&body, 2), &[1], &[want]);
            body.push(0x7e04026a);
            check(&program(&body, 2), &[1], &[1]);
        }
    }
}

#[test]
fn masks_aliases_and_both_incoming_flags() {
    for op in OPS {
        for dst in 1..=4 {
            for mask in [0, 1, 0x80000000, 0xaaaaaaaa, u32::MAX] {
                let mut body = vec![0x7e040283, 0x7e060285, 0x7e080287, 0xbefe03ff, mask];
                body.extend(insn(op, dst, [257, 258, 259]));
                body.push(0xbefe03c1);
                let old = match dst {
                    1 => 1,
                    2 => 3,
                    3 => 5,
                    _ => 7,
                };
                let result = spec(op, 1, 3, 5);
                let wants: Vec<_> = (0..32)
                    .map(|i| if mask & (1 << i) != 0 { result } else { old })
                    .collect();
                check(&program(&body, dst), &[1; 32], &wants);
            }
        }
        for vcc in [0, 1] {
            for scc in [false, true] {
                let mut body = vec![0xbeea0380 | vcc, if scc { 0xbf068080 } else { 0xbf068180 }];
                body.extend(insn(op, 2, [129; 3]));
                body.extend([0x7e0a026a, 0xbf840001, 29 << 25 | 2 << 17 | 2 << 9 | 261]);
                let result = spec(op, 1, 1, 1);
                check(
                    &program(&body, 2),
                    &[0],
                    &[if scc { result ^ vcc } else { result }],
                );
            }
        }
    }
}
const INPUT: [u32; 12] = [
    0x0, 0x1, 0xffffffff, 0x80000000, 0x7fffffff, 0x8000, 0x10000, 0xff7fff, 0x7fa12345,
    0xffff0000, 0xffffff, 0xff000001,
];
type Row = (u32, usize, [u32; 3], [u32; 12]);
const ROWS: &[Row] = &[
    (
        322,
        0,
        [0x8000, 0x10000, 0xffffffff],
        [
            0xffffffff, 0xffff, 0xfffeffff, 0xffffffff, 0xfffeffff, 0x7fffffff, 0xffffffff,
            0x7ffeffff, 0x2344ffff, 0xffffffff, 0xfffeffff, 0xffff,
        ],
    ),
    (
        322,
        1,
        [0x8000, 0x10000, 0xffffffff],
        [
            0xffffffff, 0x7fff, 0xffff7fff, 0xffffffff, 0xffff7fff, 0x3fffffff, 0x7fffffff,
            0xbfff7fff, 0x91a27fff, 0x7fffffff, 0xffff7fff, 0x7fff,
        ],
    ),
    (
        322,
        2,
        [0x8000, 0x10000, 0xffffffff],
        [
            0x80000000, 0x80000001, 0x7fffffff, 0x0, 0xffffffff, 0x80008000, 0x80010000,
            0x80ff7fff, 0xffa12345, 0x7fff0000, 0x80ffffff, 0x7f000001,
        ],
    ),
    (
        65858,
        0,
        [0x8000, 0x10000, 0xffffffff],
        [
            0xffffffff, 0xffff, 0xfffeffff, 0xffffffff, 0xfffeffff, 0x7fffffff, 0x7fffffff,
            0x80000000, 0x80000000, 0x80000000, 0xfffeffff, 0xffff,
        ],
    ),
    (
        65858,
        1,
        [0x8000, 0x10000, 0xffffffff],
        [
            0xffffffff, 0x7fff, 0xffff7fff, 0xffffffff, 0xffff7fff, 0x3fffffff, 0x7fffffff,
            0xbfff7fff, 0x80000000, 0x80000000, 0xffff7fff, 0x7fff,
        ],
    ),
    (
        65858,
        2,
        [0x8000, 0x10000, 0xffffffff],
        [
            0x7fffffff, 0x7fffffff, 0x7fffffff, 0x0, 0x7fffffff, 0x7fffffff, 0x7fffffff,
            0x7fffffff, 0x7fffffff, 0x7fff0000, 0x7fffffff, 0x7f000001,
        ],
    ),
    (
        323,
        0,
        [0x10000, 0x10000, 0x1],
        [
            0x1, 0x10001, 0xffff0001, 0x1, 0xffff0001, 0x80000001, 0x1, 0x7fff0001, 0x23450001,
            0x1, 0xffff0001, 0x10001,
        ],
    ),
    (
        323,
        1,
        [0x10000, 0x10000, 0x1],
        [
            0x1, 0x10001, 0xffff0001, 0x1, 0xffff0001, 0x80000001, 0x1, 0x7fff0001, 0x23450001,
            0x1, 0xffff0001, 0x10001,
        ],
    ),
    (
        323,
        2,
        [0x10000, 0x10000, 0x1],
        [
            0x0, 0x1, 0xffffffff, 0x80000000, 0x7fffffff, 0x8000, 0x10000, 0xff7fff, 0x7fa12345,
            0xffff0000, 0xffffff, 0xff000001,
        ],
    ),
    (
        65859,
        0,
        [0x10000, 0x10000, 0x1],
        [
            0x1, 0x10001, 0xffffffff, 0x1, 0xffffffff, 0x80000001, 0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff, 0xffffffff, 0x10001,
        ],
    ),
    (
        65859,
        1,
        [0x10000, 0x10000, 0x1],
        [
            0x1, 0x10001, 0xffffffff, 0x1, 0xffffffff, 0x80000001, 0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff, 0xffffffff, 0x10001,
        ],
    ),
    (
        65859,
        2,
        [0x10000, 0x10000, 0x1],
        [
            0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        346,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0x403, 0x87, 0x383, 0x87, 0x8, 0x284, 0x18f, 0x205, 0x304, 0x107,
        ],
    ),
    (
        346,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x403, 0x402, 0x7, 0x383, 0x87, 0x383, 0x402, 0x186, 0x27b, 0x205, 0x106, 0x303,
        ],
    ),
    (
        346,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc, 0x3fd, 0x3fb, 0x800003fc, 0x800003fb, 0x83fc, 0x103fc, 0xff83fb, 0x7fa12741,
            0xffff03fc, 0x10003fb, 0xff0003fd,
        ],
    ),
    (
        65882,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0x403, 0x87, 0x383, 0x87, 0x8, 0x284, 0x18f, 0x205, 0x304, 0x107,
        ],
    ),
    (
        65882,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x403, 0x402, 0x7, 0x383, 0x87, 0x383, 0x402, 0x186, 0x27b, 0x205, 0x106, 0x303,
        ],
    ),
    (
        65882,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc, 0x3fd, 0xffffffff, 0x800003fc, 0x800003fb, 0x83fc, 0x103fc, 0xff83fb,
            0x7fa12741, 0xffff03fc, 0x10003fb, 0xff0003fd,
        ],
    ),
    (
        347,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x10007, 0x3fc0007, 0x800007, 0x37c0007, 0x800007, 0x10007, 0x27d0007, 0x1880007,
            0x1fe0007, 0x2fd0007, 0x1000007,
        ],
    ),
    (
        347,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc0007, 0x3fb0007, 0x7, 0x37c0007, 0x800007, 0x37c0007, 0x3fb0007, 0x17f0007,
            0x2740007, 0x1fe0007, 0xff0007, 0x2fc0007,
        ],
    ),
    (
        347,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc0000, 0x3fc0001, 0x3fbffff, 0x83fc0000, 0x83fbffff, 0x3fc8000, 0x3fd0000,
            0x4fb7fff, 0x839d2345, 0x3fb0000, 0x4fbffff, 0x2fc0001,
        ],
    ),
    (
        65883,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x10007, 0x3fc0007, 0x800007, 0x37c0007, 0x800007, 0x10007, 0x27d0007, 0x1880007,
            0x1fe0007, 0x2fd0007, 0x1000007,
        ],
    ),
    (
        65883,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc0007, 0x3fb0007, 0x7, 0x37c0007, 0x800007, 0x37c0007, 0x3fb0007, 0x17f0007,
            0x2740007, 0x1fe0007, 0xff0007, 0x2fc0007,
        ],
    ),
    (
        65883,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x3fc0000, 0x3fc0001, 0xffffffff, 0x83fc0000, 0x83fbffff, 0x3fc8000, 0x3fd0000,
            0x4fb7fff, 0x839d2345, 0xffffffff, 0x4fbffff, 0xffffffff,
        ],
    ),
    (
        348,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0x20005, 0x8007, 0x18005, 0x8007, 0x8, 0x8105, 0xa2ed, 0x10006, 0x10105,
            0xff08,
        ],
    ),
    (
        348,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x20005, 0x20004, 0x7, 0x18005, 0x8007, 0x18005, 0x20004, 0x17f07, 0x15d1f, 0x10006,
            0xff07, 0x10104,
        ],
    ),
    (
        348,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x1fffe, 0x1ffff, 0x1fffd, 0x8001fffe, 0x8001fffd, 0x27ffe, 0x2fffe, 0x1017ffd,
            0x7fa32343, 0xfffe, 0x101fffd, 0xff01ffff,
        ],
    ),
    (
        65884,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0x20005, 0x8007, 0x18005, 0x8007, 0x8, 0x8105, 0xa2ed, 0x10006, 0x10105,
            0xff08,
        ],
    ),
    (
        65884,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x20005, 0x20004, 0x7, 0x18005, 0x8007, 0x18005, 0x20004, 0x17f07, 0x15d1f, 0x10006,
            0xff07, 0x10104,
        ],
    ),
    (
        65884,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0x1fffe, 0x1ffff, 0xffffffff, 0x8001fffe, 0x8001fffd, 0x27ffe, 0x2fffe, 0x1017ffd,
            0x7fa32343, 0xffffffff, 0x101fffd, 0xff01ffff,
        ],
    ),
    (
        349,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0x6, 0x80000007, 0x80000006, 0x8007, 0x10007, 0xff8006, 0x7fa1234c,
            0xffff0007, 0x1000006, 0xff000008,
        ],
    ),
    (
        349,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0x6, 0x5, 0x7, 0x80000006, 0x80000007, 0xffff8006, 0xffff0006, 0xff008007, 0x805edcc1,
            0x10006, 0xff000007, 0x1000005,
        ],
    ),
    (
        349,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0xffffffff, 0x0, 0xfffffffe, 0x7fffffff, 0x7ffffffe, 0x7fff, 0xffff, 0xff7ffe,
            0x7fa12344, 0xfffeffff, 0xfffffe, 0xff000000,
        ],
    ),
    (
        65885,
        0,
        [0xffffffff, 0x0, 0x7],
        [
            0x7, 0x8, 0xffffffff, 0x80000007, 0x80000006, 0x8007, 0x10007, 0xff8006, 0x7fa1234c,
            0xffff0007, 0x1000006, 0xff000008,
        ],
    ),
    (
        65885,
        1,
        [0xffffffff, 0x0, 0x7],
        [
            0xffffffff, 0xffffffff, 0x7, 0x80000006, 0x80000007, 0xffff8006, 0xffff0006,
            0xff008007, 0x805edcc1, 0x10006, 0xff000007, 0x1000005,
        ],
    ),
    (
        65885,
        2,
        [0xffffffff, 0x0, 0x7],
        [
            0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
            0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
];
fn run_case(gpu: Option<&xenora_metal::NativeCompute>, code: &[u8], input: &[u32], want: &[u32]) {
    check(code, input, want);
    if let Some(gpu) = gpu {
        let c = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
        assert_eq!(
            gpu.run_words(&c.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap(),
            want
        );
    }
}
fn batches(gpu: Option<&xenora_metal::NativeCompute>) {
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 12]).collect();
    // 36 runtime rows: all12 modes, all three active source positions.
    for &(op, pos, fixed, answers) in ROWS {
        let mut body = Vec::new();
        let mut src = [257; 3];
        let mut reg = 8;
        for i in 0..3 {
            if i != pos {
                body.extend([0xbe8003ff | reg << 16, fixed[i]]);
                src[i] = reg;
                reg += 1;
            }
        }
        body.extend(insn(op, 1, src));
        let want: Vec<_> = (0..32).map(|i| answers[i % 12]).collect();
        run_case(gpu, &program(&body, 1), &input, &want);
    }
    // Twenty-four masked flag-observer programs and twelve shared-literal programs.
    for (i, &op) in OPS.iter().enumerate() {
        let &(_, _, fixed, answers) = ROWS.iter().find(|r| r.0 == op && r.1 == 0).unwrap();
        for mask in [0, [0xaaaaaaaa, 0x80000000][i % 2]] {
            let flag = i % 2 == 0;
            let mut body = vec![
                0x7e0402ff,
                fixed[1],
                0x7e0602ff,
                fixed[2],
                0x7e080287,
                0xbeea0381,
                if flag { 0xbf068080 } else { 0xbf068180 },
                0xbefe03ff,
                mask,
            ];
            body.extend(insn(op, 4, [257, 258, 259]));
            body.extend([
                0xbefe03c1,
                0x7e0a026a,
                0xbf840001,
                29 << 25 | 4 << 17 | 4 << 9 | 261,
            ]);
            let want: Vec<_> = (0..32)
                .map(|j| {
                    let x = if mask & (1 << j) != 0 {
                        answers[j % 12]
                    } else {
                        7
                    };
                    if flag { x ^ 1 } else { x }
                })
                .collect();
            run_case(gpu, &program(&body, 4), &input, &want);
        }
        let mut body = insn(op, 4, [255; 3]).to_vec();
        body.push(0xffffffff);
        let want = match op {
            322 | 0x10142 => 0,
            323 => 0xfe000000,
            _ => 0xffffffff,
        };
        run_case(gpu, &program(&body, 4), &[0; 32], &[want; 32]);
    }
    for (a, b, c, want) in [
        (0x8000, 0x10000, 0xffffffff, 0x7fffffff),
        (0xff7fff, 0x10000, 0x10000, 0x80000000),
    ] {
        let mut body = vec![0x7e0402ff, b, 0x7e0602ff, c];
        body.extend(insn(0x10142, 4, [255, 258, 259]));
        body.push(a);
        run_case(gpu, &program(&body, 4), &[0; 32], &[want; 32]);
    }
    run_case(gpu, FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
#[test]
fn cpu_batched_literal_answers() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_mad_sad_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    batches(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-mad-sad.bin");
const FIXTURE_INPUT: [u32; 12] = [
    0x0, 0x1, 0xffffffff, 0x80000000, 0x7fffffff, 0x8000, 0x10000, 0xff7fff, 0x7fa12345,
    0xffff0000, 0xffffff, 0xff000001,
];
// Assembly folds 28 results using h=33*h+r. Its final four results are
// 7fffffff,7fffffff,80000000,80000000: signed cancellation in both modes.
const FIXTURE_EXPECTED: [u32; 12] = [
    0xc5de3a44, 0xd66bedad, 0x4bc02257, 0xf8a5eac4, 0x18f871d7, 0x8c796ac4, 0x1080c606, 0xe5fb2038,
    0xecfa0af5, 0xb8f14221, 0xac9650b8, 0x7595bf4c,
];
#[test]
fn assembled_mad_sad_fixture_literal_answers() {
    check(FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}

#[test]
fn all_seven_shared_literal_positions_and_exact_api() {
    for op in OPS {
        for mask in 1..8 {
            let mut src = [129; 3];
            let mut values = [1; 3];
            for pos in 0..3 {
                if mask & (1 << pos) != 0 {
                    src[pos] = 255;
                    values[pos] = 3;
                }
            }
            let mut body = insn(op, 2, src).to_vec();
            body.push(3);
            check(
                &program(&body, 2),
                &[0],
                &[spec(op, values[0], values[1], values[2])],
            );
        }
    }
    use xenora_ref::{CompareSource, MadSadOp, Op};
    for (opcode, kind) in [
        (322, MadSadOp::MadI32I24),
        (323, MadSadOp::MadU32U24),
        (346, MadSadOp::SadU8),
        (347, MadSadOp::SadHiU8),
        (348, MadSadOp::SadU16),
        (349, MadSadOp::SadU32),
    ] {
        for clamp in [false, true] {
            let key = opcode | if clamp { 0x10000 } else { 0 };
            let code = bytes(&insn(key, 4, [257, 258, 259]));
            assert_eq!(
                xenora_ref::decode(&code).unwrap()[0].op,
                Op::MadSad(
                    kind,
                    clamp,
                    4,
                    CompareSource::Vector(1),
                    CompareSource::Vector(2),
                    CompareSource::Vector(3)
                )
            );
        }
    }
}
