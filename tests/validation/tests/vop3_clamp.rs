// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte integer saturation fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 5] = [265, 267, 293, 294, 295];
fn arity(_: u32) -> usize {
    2
}
fn insn(op: u32, dst: u32, mut src: [u32; 3]) -> [u32; 2] {
    for s in &mut src[arity(op)..] {
        *s = 128;
    }
    [
        0xd4008000 | op << 16 | dst,
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
fn spec(op: u32, a: u32, b: u32) -> u32 {
    let field = |x: u32| {
        let x = i128::from(x % 0x1000000);
        if x >= 0x800000 { x - 0x1000000 } else { x }
    };
    let n = match op {
        265 => field(a) * field(b),
        267 => i128::from(a % 0x1000000) * i128::from(b % 0x1000000),
        293 => i128::from(a) + i128::from(b),
        294 => i128::from(a) - i128::from(b),
        295 => i128::from(b) - i128::from(a),
        _ => unreachable!(),
    };
    if op == 265 {
        n.clamp(-2147483648, 2147483647) as u32
    } else {
        n.clamp(0, 4294967295) as u32
    }
}
// opcode,a,b,saturated,ordinary wrap: explicit independent boundary anchors.
const ANCHORS: &[(u32, u32, u32, u32, u32)] = &[
    (265, 0x8000, 0x10000, 0x7fffffff, 0x80000000),
    (265, 0x7fff, 0x10000, 0x7fff0000, 0x7fff0000),
    (265, 0xff8000, 0x10000, 0x80000000, 0x80000000),
    (265, 0xff7fff, 0x10000, 0x80000000, 0x7fff0000),
    (265, 0x7fffff, 256, 0x7fffff00, 0x7fffff00),
    (265, 0x7fffff, 257, 0x7fffffff, 0x807ffeff),
    (265, 0x800000, 255, 0x80800000, 0x80800000),
    (265, 0x800000, 256, 0x80000000, 0x80000000),
    (265, 0x800000, 257, 0x80000000, 0x7f800000),
    (265, 0x800000, 0xffff00, 0x7fffffff, 0x80000000),
    (265, 0x800000, 0x800000, 0x7fffffff, 0),
    (265, 0xffffff, 2, 0xfffffffe, 0xfffffffe),
    (267, 0xffff, 0x10000, 0xffff0000, 0xffff0000),
    (267, 0x10000, 0x10000, 0xffffffff, 0),
    (267, 0x10001, 0xffff, 0xffffffff, 0xffffffff),
    (267, 0x10002, 0xffff, 0xffffffff, 0xfffe),
    (267, 0xffffff, 0xffffff, 0xffffffff, 0xfe000001),
    (267, 0, 0xffffff, 0, 0),
    (293, 0xffffffff, 1, 0xffffffff, 0),
    (293, 0x80000000, 0x80000000, 0xffffffff, 0),
    (293, 0xfffffffe, 1, 0xffffffff, 0xffffffff),
    (293, 0xffffffff, 0, 0xffffffff, 0xffffffff),
    (293, 1, 2, 3, 3),
    (294, 0, 1, 0, 0xffffffff),
    (294, 1, 0, 1, 1),
    (294, 0xffffffff, 0xffffffff, 0, 0),
    (295, 1, 0, 0, 0xffffffff),
    (295, 0, 1, 1, 1),
    (295, 0xffffffff, 0xffffffff, 0, 0),
];
#[test]
fn literal_boundaries_saturate_before_wrap_and_alias() {
    for &(op, a, b, want, wrap) in ANCHORS {
        for dst in [1, 2, 3, 255] {
            let mut body = vec![0x7e0402ff, b];
            body.extend(insn(op, dst, [257, 258, 128]));
            check(&program(&body, dst), &[a], &[want]);
            body[2] &= !0x8000;
            check(&program(&body, dst), &[a], &[wrap]);
        }
    }
}
#[test]
fn multiply_high_bytes_do_not_change_fields() {
    for &(op, a, b, want, _) in ANCHORS.iter().filter(|r| matches!(r.0, 265 | 267)) {
        for higha in [0, 0x7f000000, 0x80000000, 0xff000000] {
            for highb in [0, 0x12000000, 0xff000000] {
                let a = (a & 0xffffff) | higha;
                let b = (b & 0xffffff) | highb;
                let mut body = vec![0x7e0402ff, b];
                body.extend(insn(op, 3, [257, 258, 128]));
                check(&program(&body, 3), &[a], &[want]);
            }
        }
    }
}
#[test]
fn active_sources_literals_inline_words_and_masks() {
    let floats = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for op in OPS {
        for pos in 0..arity(op) {
            for selector in [
                8, 105, 106, 128, 129, 192, 193, 208, 240, 241, 242, 243, 244, 245, 246, 247, 248,
                255, 257, 511,
            ] {
                let value = match selector {
                    128 => 0,
                    129 => 1,
                    192 => 64,
                    193 => u32::MAX,
                    208 => 0xfffffff0,
                    240..=248 => floats[(selector - 240) as usize],
                    _ => 3,
                };
                let mut src = [129, 129, 128];
                src[pos] = selector;
                let mut body = vec![0xbe880383, 0xbee90383, 0xbeea0383, 0x7ffe0283];
                body.extend(insn(op, 2, src));
                if selector == 255 {
                    body.push(3);
                }
                let mut v = [1, 1];
                v[pos] = value;
                check(&program(&body, 2), &[3], &[spec(op, v[0], v[1])]);
            }
            for unknown in [20, 106, 265] {
                let mut src = [129, 129, 128];
                src[pos] = unknown;
                let mut body = vec![0x7e040287, 0xbefe0380];
                body.extend(insn(op, 2, src));
                body.push(0xbefe03c1);
                check(&program(&body, 2), &[1], &[7]);
                for mask in [1, 0x80000000, u32::MAX] {
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
        for subset in 1..(1 << arity(op)) {
            let mut src = [129, 129, 128];
            for (pos, v) in src.iter_mut().enumerate().take(arity(op)) {
                if subset & (1 << pos) != 0 {
                    *v = 255;
                }
            }
            let mut body = insn(op, 2, src).to_vec();
            body.push(1);
            check(&program(&body, 2), &[0], &[spec(op, 1, 1)]);
        }
        for mask in [0, 1, 0x80000000, 0xaaaaaaaa, u32::MAX] {
            for dst in [1, 2, 3] {
                let mut body = vec![0x7e040283, 0x7e060287, 0xbefe03ff, mask];
                body.extend(insn(op, dst, [257, 258, 128]));
                body.push(0xbefe03c1);
                let old = match dst {
                    1 => 1,
                    2 => 3,
                    _ => 7,
                };
                let result = spec(op, 1, 3);
                let wants: Vec<_> = (0..32)
                    .map(|i| if mask & (1 << i) != 0 { result } else { old })
                    .collect();
                check(&program(&body, dst), &[1; 32], &wants);
            }
        }
    }
}
#[test]
fn decode_rejections_padding_modifiers_truncations() {
    for op in OPS {
        for pos in 0..arity(op) {
            for bad in (108..=125).chain(209..=239).chain(249..=254) {
                let mut src = [129, 129, 128];
                src[pos] = bad;
                reject(&program(&insn(op, 2, src), 2), 12, true, "");
            }
            let mut src = [129, 129, 128];
            src[pos] = 255;
            let code = bytes(&insn(op, 2, src));
            for n in 0..4 {
                let mut c = code.clone();
                c.extend(&[1, 2, 3][..n]);
                reject(&c, 0, true, "");
            }
        }
        for n in 4..8 {
            reject(&bytes(&insn(op, 2, [129, 129, 128]))[..n], 0, true, "");
        }
        for pos in arity(op)..3 {
            for raw in 0..512 {
                if raw == 128 {
                    continue;
                }
                let mut w = insn(op, 2, [129, 129, 128]);
                w[1] = (w[1] & !(511 << (pos * 9))) | raw << (pos * 9);
                reject(&program(&w, 2), 12, true, "");
            }
        }
        for bit in (8..15).chain(27..32) {
            let mut w = insn(op, 2, [129, 129, 128]);
            w[usize::from(bit >= 27)] |= 1 << bit;
            reject(&program(&w, 2), 12, true, "modifier");
        }
    }
}
#[test]
fn flags_unset_preserved_and_pending_reads() {
    for op in OPS {
        let mut body = insn(op, 2, [129, 129, 128]).to_vec();
        body.extend([0xbf840001, 0x7e040280]);
        reject(&program(&body, 2), 20, false, "SCC");
        let mut body = insn(op, 2, [129, 129, 128]).to_vec();
        body.push(0x7e04026a);
        reject(&program(&body, 2), 20, false, "known value");
        let mut body = vec![0xbefe0381];
        body.extend(insn(op, 2, [129, 129, 128]));
        body.push(0xbefe03c1);
        reject(&program(&body, 2), 16, false, "partial EXEC");
        for vcc in [0, 1] {
            for scc in [false, true] {
                let mut body = vec![0xbeea0380 | vcc, if scc { 0xbf068080 } else { 0xbf068180 }];
                body.extend(insn(op, 2, [129, 129, 128]));
                body.extend([0x7e0a026a, 0xbf840001, 29 << 25 | 2 << 17 | 2 << 9 | 261]);
                let result = spec(op, 1, 1);
                check(
                    &program(&body, 2),
                    &[0],
                    &[if scc { result ^ vcc } else { result }],
                );
            }
        }
        for pos in 0..arity(op) {
            for source in [256, 257] {
                let mut src = [129, 129, 128];
                src[pos] = source;
                let mut w = vec![0xe0302000, 0x80000100];
                w.extend(insn(op, 2, src));
                w.extend([0xbf8c3f70, 0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
                reject(&bytes(&w), 8, false, "source register");
            }
        }
        for dst in [0, 1] {
            let mut w = vec![0xe0302000, 0x80000100];
            w.extend(insn(op, dst, [129, 129, 128]));
            w.extend([0xbf8c3f70, 0xe0702000, 0x80010100, 0xbbfd0000, 0xbf810000]);
            reject(
                &bytes(&w),
                8,
                false,
                "writes the index register or replaces a pending load",
            );
        }
    }
}
#[test]
fn clamped_regions_excluded_even_with_zero_exec() {
    for mask in [0, 1, u32::MAX] {
        let mut old = vec![0x7e040287, 0x7d8a0280, 0xbefe03ff, mask, 0xbf860002];
        let mut add = insn(293, 2, [257, 257, 128]);
        add[0] &= !0x8000;
        old.extend(add);
        old.push(0xbefe03c1);
        check(&program(&old, 2), &[0], &[7]);
        check(&program(&old, 2), &[1], &[if mask == 0 { 7 } else { 2 }]);
        for op in OPS {
            for pos in 0..2 {
                for selector in [8, 106, 129, 240, 255, 257] {
                    let mut src = [257, 257, 128];
                    src[pos] = selector;
                    let mut body = vec![
                        0xbe880381,
                        0x7e040287,
                        0x7d8a0280,
                        0xbefe03ff,
                        mask,
                        if selector == 255 {
                            0xbf860003
                        } else {
                            0xbf860002
                        },
                    ];
                    body.extend(insn(op, 2, src));
                    if selector == 255 {
                        body.push(1);
                    }
                    body.push(0xbefe03c1);
                    reject(&program(&body, 2), 36, false, "runtime");
                }
            }
        }
    }
}
#[test]
fn unsupported_clamp_modes_stay_rejected() {
    for op in [266, 268, 273, 286, 334, 338, 439] {
        let words = [0xd4008002 | op << 16, 129 | 129 << 9 | 128 << 18];
        reject(&program(&words, 2), 12, true, "modifier");
    }
}
// Literal runtime input/answer rows; no evaluator generates native expectations.
type Row = (u32, usize, u32, [u32; 8], [u32; 8]);
const ROWS: &[Row] = &[
    (
        265,
        0,
        0x100,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0x100, 0x7fffff00, 0x80000000, 0x80000100, 0xffffff00, 0x80000000, 0x7fffff00,
        ],
    ),
    (
        265,
        1,
        0x100,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0x100, 0x7fffff00, 0x80000000, 0x80000100, 0xffffff00, 0x80000000, 0x7fffff00,
        ],
    ),
    (
        265,
        0,
        0x101,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0x101, 0x7fffffff, 0x80000000, 0x80000000, 0xfffffeff, 0x80000000, 0x7fffffff,
        ],
    ),
    (
        265,
        1,
        0x101,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0x101, 0x7fffffff, 0x80000000, 0x80000000, 0xfffffeff, 0x80000000, 0x7fffffff,
        ],
    ),
    (
        265,
        0,
        0xffff00,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0xffffff00, 0x80000100, 0x7fffffff, 0x7fffff00, 0x100, 0x7fffffff, 0x80000100,
        ],
    ),
    (
        265,
        1,
        0xffff00,
        [
            0x0, 0x1, 0x7fffff, 0x800000, 0x800001, 0xffffff, 0xff800000, 0x7f7fffff,
        ],
        [
            0x0, 0xffffff00, 0x80000100, 0x7fffffff, 0x7fffff00, 0x100, 0x7fffffff, 0x80000100,
        ],
    ),
    (
        267,
        0,
        0xffff,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0xffff, 0xfffe0001, 0xffff0000, 0xffffffff, 0xffffffff, 0xfffe0001, 0xffff0000,
        ],
    ),
    (
        267,
        1,
        0xffff,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0xffff, 0xfffe0001, 0xffff0000, 0xffffffff, 0xffffffff, 0xfffe0001, 0xffff0000,
        ],
    ),
    (
        267,
        0,
        0x10000,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0x10000, 0xffff0000, 0xffffffff, 0xffffffff, 0xffffffff, 0xffff0000, 0xffffffff,
        ],
    ),
    (
        267,
        1,
        0x10000,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0x10000, 0xffff0000, 0xffffffff, 0xffffffff, 0xffffffff, 0xffff0000, 0xffffffff,
        ],
    ),
    (
        267,
        0,
        0x10001,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0x10001, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        267,
        1,
        0x10001,
        [
            0x0, 0x1, 0xffff, 0x10000, 0x10001, 0xffffff, 0xab00ffff, 0xff010000,
        ],
        [
            0x0, 0x10001, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff,
        ],
    ),
    (
        293,
        0,
        0x1,
        [
            0x0, 0x1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x800001, 0x7fa12345,
        ],
        [
            0x1, 0x2, 0x80000000, 0x80000001, 0xffffffff, 0xffffffff, 0x800002, 0x7fa12346,
        ],
    ),
    (
        293,
        1,
        0x1,
        [
            0x0, 0x1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x800001, 0x7fa12345,
        ],
        [
            0x1, 0x2, 0x80000000, 0x80000001, 0xffffffff, 0xffffffff, 0x800002, 0x7fa12346,
        ],
    ),
    (
        293,
        0,
        0x80000000,
        [
            0x0, 0x1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x800001, 0x7fa12345,
        ],
        [
            0x80000000, 0x80000001, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0x80800001,
            0xffa12345,
        ],
    ),
    (
        293,
        1,
        0x80000000,
        [
            0x0, 0x1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x800001, 0x7fa12345,
        ],
        [
            0x80000000, 0x80000001, 0xffffffff, 0xffffffff, 0xffffffff, 0xffffffff, 0x80800001,
            0xffa12345,
        ],
    ),
    (
        294,
        0,
        0x1,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [
            0x0, 0x0, 0x1, 0x7ffffffe, 0x7fffffff, 0xfffffffd, 0xfffffffe, 0x7fa12344,
        ],
    ),
    (
        294,
        1,
        0x1,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0],
    ),
    (
        294,
        0,
        0xffffffff,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0],
    ),
    (
        294,
        1,
        0xffffffff,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [
            0xffffffff, 0xfffffffe, 0xfffffffd, 0x80000000, 0x7fffffff, 0x1, 0x0, 0x805edcba,
        ],
    ),
    (
        295,
        0,
        0x1,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0],
    ),
    (
        295,
        1,
        0x1,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [
            0x0, 0x0, 0x1, 0x7ffffffe, 0x7fffffff, 0xfffffffd, 0xfffffffe, 0x7fa12344,
        ],
    ),
    (
        295,
        0,
        0xffffffff,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [
            0xffffffff, 0xfffffffe, 0xfffffffd, 0x80000000, 0x7fffffff, 0x1, 0x0, 0x805edcba,
        ],
    ),
    (
        295,
        1,
        0xffffffff,
        [
            0x0, 0x1, 0x2, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff, 0x7fa12345,
        ],
        [0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0],
    ),
    (
        265,
        0,
        0x10000,
        [
            0x7fff, 0x8000, 0xff8000, 0xff7fff, 0x7f008000, 0xffff8000, 0xab007fff, 0x80ff7fff,
        ],
        [
            0x7fff0000, 0x7fffffff, 0x80000000, 0x80000000, 0x7fffffff, 0x80000000, 0x7fff0000,
            0x80000000,
        ],
    ),
    (
        265,
        1,
        0x10000,
        [
            0x7fff, 0x8000, 0xff8000, 0xff7fff, 0x7f008000, 0xffff8000, 0xab007fff, 0x80ff7fff,
        ],
        [
            0x7fff0000, 0x7fffffff, 0x80000000, 0x80000000, 0x7fffffff, 0x80000000, 0x7fff0000,
            0x80000000,
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
    // 26 runtime rows, both active positions; each compilation batches 32 lanes.
    for &(op, pos, b, xs, ys) in ROWS {
        let mut src = [8, 8, 128];
        src[pos] = 257;
        let dst = if pos == 0 { 1 } else { 2 };
        let mut body = vec![0xbe8803ff, b];
        body.extend(insn(op, dst, src));
        let input: Vec<_> = (0..32).map(|i| xs[i % 8]).collect();
        let want: Vec<_> = (0..32).map(|i| ys[i % 8]).collect();
        run_case(gpu, &program(&body, dst), &input, &want);
    }
    // 20 masks/flags programs: one literal companion, all four EXEC controls.
    for op in OPS {
        let &(_, _, b, xs, ys) = ROWS.iter().find(|r| r.0 == op && r.1 == 0).unwrap();
        for (i, mask) in [0, u32::MAX, 0xaaaaaaaa, 0x80000000]
            .into_iter()
            .enumerate()
        {
            let mut body = vec![
                0x7e040287,
                0xbeea0381,
                if i % 2 == 0 { 0xbf068080 } else { 0xbf068180 },
                0xbefe03ff,
                mask,
            ];
            body.extend(insn(op, 2, [257, 255, 128]));
            body.push(b);
            body.extend([
                0xbefe03c1,
                0x7e0a026a,
                0xbf840001,
                29 << 25 | 2 << 17 | 2 << 9 | 261,
            ]);
            let input: Vec<_> = (0..32).map(|j| xs[j % 8]).collect();
            let want: Vec<_> = (0..32)
                .map(|j| {
                    let x = if mask & (1 << j) != 0 { ys[j % 8] } else { 7 };
                    if i % 2 == 0 { x ^ 1 } else { x }
                })
                .collect();
            run_case(gpu, &program(&body, 2), &input, &want);
        }
    }
    // Shared literal means one payload and one identity, not two independent words.
    for (op, word, want) in [
        (265, 0x800000, 0x7fffffff),
        (267, 0xffffff, 0xffffffff),
        (293, 0x80000000, 0xffffffff),
        (294, 0xffffffff, 0),
        (295, 0xffffffff, 0),
    ] {
        let mut body = insn(op, 2, [255, 255, 128]).to_vec();
        body.push(word);
        run_case(gpu, &program(&body, 2), &[0; 32], &[want; 32]);
    }
    // 52 dispatches including one execution of the actual assembled fixture.
    run_case(gpu, FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
#[test]
fn cpu_batched_runtime_boundaries() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_clamp_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    batches(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-clamp.bin");
const FIXTURE_INPUT: [u32; 12] = [
    0x0, 0x1, 0x2, 0xff, 0x100, 0x101, 0x7fffff, 0x800000, 0xffffff, 0xffffffff, 0x7fa12345,
    0xff000001,
];
// Original assembly folds ten results as h=33*h+r. The four subtraction
// results are zero; both unsigned additions and the constant U24 product are MAX.
const FIXTURE_EXPECTED: [u32; 12] = [
    0xbc22111c, 0x3d55ab1a, 0xbe894518, 0x6e88751e, 0xefbc0f1c, 0x4bd5ab1a, 0x4bd5ab1a, 0x4fe03c3b,
    0x436acd5c, 0x436acd5c, 0x4fe03c3b, 0x3d55ab1a,
];
#[test]
fn actual_clamp_fixture_literal_answers() {
    check(FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}

#[test]
fn two_scalar_identities_and_exact_clamp_api() {
    use xenora_ref::{ClampOp, CompareSource, Op};
    for (op, kind, want) in [
        (265, ClampOp::MulI24, 9),
        (267, ClampOp::MulU24, 9),
        (293, ClampOp::AddU32, 6),
        (294, ClampOp::SubU32, 0),
        (295, ClampOp::SubrevU32, 0),
    ] {
        for src in [
            [8, 9, 128],
            [8, 8, 128],
            [106, 8, 128],
            [8, 255, 128],
            [255, 8, 128],
        ] {
            let mut body = vec![0xbe880383, 0xbe890383, 0xbeea0383];
            body.extend(insn(op, 2, src));
            if src.contains(&255) {
                body.push(3);
            }
            check(&program(&body, 2), &[0], &[want]);
        }
        let code = bytes(&insn(op, 3, [257, 258, 128]));
        assert_eq!(
            xenora_ref::decode(&code).unwrap()[0].op,
            Op::Clamped(kind, 3, CompareSource::Vector(1), CompareSource::Vector(2))
        );
    }
    let mut words = insn(293, 3, [257, 258, 128]);
    words[0] &= !0x8000;
    assert_eq!(
        xenora_ref::decode(&bytes(&words)).unwrap()[0].op,
        Op::Add(3, 1, 2)
    );
}
