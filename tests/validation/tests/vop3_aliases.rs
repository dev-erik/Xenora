// SPDX-License-Identifier: GPL-3.0-or-later
//! Original raw-byte canonical integer e64 alias fixtures and independent literal answers.
#[path = "../independent_ref.rs"]
mod independent_ref;
const OPS: [u32; 23] = [
    265, 266, 267, 268, 273, 274, 275, 276, 286, 293, 294, 295, 282, 278, 280, 283, 284, 285, 439,
    440, 441, 442, 443,
];
fn arity(op: u32) -> usize {
    if op >= 439 { 1 } else { 2 }
}
fn insn(op: u32, dst: u32, mut src: [u32; 3]) -> [u32; 2] {
    for s in &mut src[arity(op)..] {
        *s = 128;
    }
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
// Separate mathematical specification used only by CPU source-domain checks.
// Native results below are stored as literal arrays, never computed by this code.
fn spec(op: u32, a: u32, b: u32) -> u32 {
    let field = |x: u32| {
        let x = i64::from(x % 0x1000000);
        if x >= 0x800000 { x - 0x1000000 } else { x }
    };
    match op {
        265 => (field(a) * field(b)) as u32,
        266 => ((field(a) * field(b)) >> 32) as u32,
        267 => ((u64::from(a % 0x1000000) * u64::from(b % 0x1000000)) % 0x100000000) as u32,
        268 => ((u64::from(a % 0x1000000) * u64::from(b % 0x1000000)) / 0x100000000) as u32,
        273 => (a as i32).min(b as i32) as u32,
        274 => (a as i32).max(b as i32) as u32,
        275 => a.min(b),
        276 => a.max(b),
        286 => !(a ^ b),
        293 => a.wrapping_add(b),
        294 => a.wrapping_sub(b),
        295 => b.wrapping_sub(a),
        282 => b << (a % 32),
        278 => b >> (a % 32),
        280 => ((b as i32) >> (a % 32)) as u32,
        283 => a & b,
        284 => a | b,
        285 => a ^ b,
        439 => !a,
        440 => a.reverse_bits(),
        441 => {
            if a == 0 {
                u32::MAX
            } else {
                a.leading_zeros()
            }
        }
        442 => {
            if a == 0 {
                u32::MAX
            } else {
                a.trailing_zeros()
            }
        }
        443 => {
            let x = if a & 0x80000000 != 0 { !a } else { a };
            if x == 0 { u32::MAX } else { x.leading_zeros() }
        }
        _ => unreachable!(),
    }
}
const ANCHORS: &[(u32, u32, u32, u32)] = &[
    (265, 0x800000, 2, 0xff000000),
    (266, 0x800000, 2, 0xffffffff),
    (267, 0x800000, 2, 0x1000000),
    (268, 0xffffff, 0xffffff, 0xffff),
    (273, 0x80000000, 1, 0x80000000),
    (274, 0x80000000, 1, 1),
    (275, 0x80000000, 1, 1),
    (276, 0x80000000, 1, 0x80000000),
    (286, 0xffffffff, 0, 0),
    (293, 0xffffffff, 1, 0),
    (294, 1, 0, 1),
    (295, 1, 0, 0xffffffff),
    (282, 32, 0x80000001, 0x80000001),
    (278, 31, 0x80000000, 1),
    (280, 31, 0x80000000, 0xffffffff),
    (283, 0x7fa12345, 0xff, 0x45),
    (284, 0x80000000, 1, 0x80000001),
    (285, 0x7fa12345, 0xffffffff, 0x805edcba),
    (439, 0x80000000, 0, 0x7fffffff),
    (440, 1, 0, 0x80000000),
    (441, 0, 0, 0xffffffff),
    (442, 0, 0, 0xffffffff),
    (443, 0xffffffff, 0, 0xffffffff),
    (443, 0x80000000, 0, 1),
    (441, 1, 0, 31),
    (442, 0x80000000, 0, 31),
];
fn e32(op: u32, dst: u32) -> u32 {
    let n = match op {
        265 => 9,
        266 => 10,
        267 => 11,
        268 => 12,
        273 => 17,
        274 => 18,
        275 => 19,
        276 => 20,
        286 => 30,
        293 => 37,
        294 => 38,
        295 => 39,
        282 => 26,
        278 => 22,
        280 => 24,
        283 => 27,
        284 => 28,
        285 => 29,
        439 => 55,
        440 => 56,
        441 => 57,
        442 => 58,
        443 => 59,
        _ => unreachable!(),
    };
    if arity(op) == 1 {
        0x7e000000 | dst << 17 | n << 9 | 257
    } else {
        n << 25 | dst << 17 | 2 << 9 | 257
    }
}
#[test]
fn literal_discriminators_e32_e64_and_aliases() {
    for &(op, a, b, want) in ANCHORS {
        for dst in [1, 2, 3, 255] {
            let mut body = vec![0x7e0402ff, b];
            body.extend(insn(op, dst, [257, 258, 128]));
            check(&program(&body, dst), &[a], &[want]);
            check(&program(&[0x7e0402ff, b, e32(op, dst)], dst), &[a], &[want]);
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
            for bad in (107..=127).chain(209..=239).chain(249..=254) {
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
        for bit in (8..16).chain(27..32) {
            let mut w = insn(op, 2, [129, 129, 128]);
            w[usize::from(bit >= 27)] |= 1 << bit;
            if bit == 15 && matches!(op, 265 | 267 | 293..=295) {
                // The five integer saturation modes now accept the clamp bit.
                let expected = match op {
                    265 | 267 => 1,
                    293 => 2,
                    _ => 0,
                };
                check(&program(&w, 2), &[0], &[expected]);
            } else {
                reject(&program(&w, 2), 12, true, "modifier");
            }
        }
    }
}
#[test]
fn runtime_regions_preserve_old_add_only() {
    for mask in [0, 1, u32::MAX] {
        let mut body = vec![0x7e040287, 0x7d8a0280, 0xbefe03ff, mask, 0xbf860002];
        body.extend(insn(293, 2, [257, 257, 128]));
        body.push(0xbefe03c1);
        check(&program(&body, 2), &[0], &[7]);
        check(&program(&body, 2), &[1], &[if mask == 0 { 7 } else { 2 }]);
        for op in OPS {
            for pos in 0..arity(op) {
                for selector in [8, 106, 129, 240, 255, 257] {
                    if op == 293 && selector == 257 {
                        continue;
                    }
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
const INPUT: [u32; 8] = [0x0, 0x1, 0x1f, 0x20, 0x21, 0x800000, 0xffffffff, 0x80000001];
// (opcode, runtime source position, literal answers); other active source = 3.
const RUNTIME: &[(u32, usize, [u32; 8])] = &[
    (
        265,
        0,
        [0x0, 0x3, 0x5d, 0x60, 0x63, 0xfe800000, 0xfffffffd, 0x3],
    ),
    (
        265,
        1,
        [0x0, 0x3, 0x5d, 0x60, 0x63, 0xfe800000, 0xfffffffd, 0x3],
    ),
    (
        266,
        0,
        [0x0, 0x0, 0x0, 0x0, 0x0, 0xffffffff, 0xffffffff, 0x0],
    ),
    (
        266,
        1,
        [0x0, 0x0, 0x0, 0x0, 0x0, 0xffffffff, 0xffffffff, 0x0],
    ),
    (
        267,
        0,
        [0x0, 0x3, 0x5d, 0x60, 0x63, 0x1800000, 0x2fffffd, 0x3],
    ),
    (
        267,
        1,
        [0x0, 0x3, 0x5d, 0x60, 0x63, 0x1800000, 0x2fffffd, 0x3],
    ),
    (268, 0, [0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0]),
    (268, 1, [0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0, 0x0]),
    (
        273,
        0,
        [0x0, 0x1, 0x3, 0x3, 0x3, 0x3, 0xffffffff, 0x80000001],
    ),
    (
        273,
        1,
        [0x0, 0x1, 0x3, 0x3, 0x3, 0x3, 0xffffffff, 0x80000001],
    ),
    (274, 0, [0x3, 0x3, 0x1f, 0x20, 0x21, 0x800000, 0x3, 0x3]),
    (274, 1, [0x3, 0x3, 0x1f, 0x20, 0x21, 0x800000, 0x3, 0x3]),
    (275, 0, [0x0, 0x1, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3]),
    (275, 1, [0x0, 0x1, 0x3, 0x3, 0x3, 0x3, 0x3, 0x3]),
    (
        276,
        0,
        [0x3, 0x3, 0x1f, 0x20, 0x21, 0x800000, 0xffffffff, 0x80000001],
    ),
    (
        276,
        1,
        [0x3, 0x3, 0x1f, 0x20, 0x21, 0x800000, 0xffffffff, 0x80000001],
    ),
    (
        286,
        0,
        [
            0xfffffffc, 0xfffffffd, 0xffffffe3, 0xffffffdc, 0xffffffdd, 0xff7ffffc, 0x3, 0x7ffffffd,
        ],
    ),
    (
        286,
        1,
        [
            0xfffffffc, 0xfffffffd, 0xffffffe3, 0xffffffdc, 0xffffffdd, 0xff7ffffc, 0x3, 0x7ffffffd,
        ],
    ),
    (
        293,
        0,
        [0x3, 0x4, 0x22, 0x23, 0x24, 0x800003, 0x2, 0x80000004],
    ),
    (
        293,
        1,
        [0x3, 0x4, 0x22, 0x23, 0x24, 0x800003, 0x2, 0x80000004],
    ),
    (
        294,
        0,
        [
            0xfffffffd, 0xfffffffe, 0x1c, 0x1d, 0x1e, 0x7ffffd, 0xfffffffc, 0x7ffffffe,
        ],
    ),
    (
        294,
        1,
        [
            0x3, 0x2, 0xffffffe4, 0xffffffe3, 0xffffffe2, 0xff800003, 0x4, 0x80000002,
        ],
    ),
    (
        295,
        0,
        [
            0x3, 0x2, 0xffffffe4, 0xffffffe3, 0xffffffe2, 0xff800003, 0x4, 0x80000002,
        ],
    ),
    (
        295,
        1,
        [
            0xfffffffd, 0xfffffffe, 0x1c, 0x1d, 0x1e, 0x7ffffd, 0xfffffffc, 0x7ffffffe,
        ],
    ),
    (
        282,
        0,
        [0x3, 0x6, 0x80000000, 0x3, 0x6, 0x3, 0x80000000, 0x6],
    ),
    (
        282,
        1,
        [0x0, 0x8, 0xf8, 0x100, 0x108, 0x4000000, 0xfffffff8, 0x8],
    ),
    (278, 0, [0x3, 0x1, 0x0, 0x3, 0x1, 0x3, 0x0, 0x1]),
    (
        278,
        1,
        [0x0, 0x0, 0x3, 0x4, 0x4, 0x100000, 0x1fffffff, 0x10000000],
    ),
    (280, 0, [0x3, 0x1, 0x0, 0x3, 0x1, 0x3, 0x0, 0x1]),
    (
        280,
        1,
        [0x0, 0x0, 0x3, 0x4, 0x4, 0x100000, 0xffffffff, 0xf0000000],
    ),
    (283, 0, [0x0, 0x1, 0x3, 0x0, 0x1, 0x0, 0x3, 0x1]),
    (283, 1, [0x0, 0x1, 0x3, 0x0, 0x1, 0x0, 0x3, 0x1]),
    (
        284,
        0,
        [0x3, 0x3, 0x1f, 0x23, 0x23, 0x800003, 0xffffffff, 0x80000003],
    ),
    (
        284,
        1,
        [0x3, 0x3, 0x1f, 0x23, 0x23, 0x800003, 0xffffffff, 0x80000003],
    ),
    (
        285,
        0,
        [0x3, 0x2, 0x1c, 0x23, 0x22, 0x800003, 0xfffffffc, 0x80000002],
    ),
    (
        285,
        1,
        [0x3, 0x2, 0x1c, 0x23, 0x22, 0x800003, 0xfffffffc, 0x80000002],
    ),
    (
        439,
        0,
        [
            0xffffffff, 0xfffffffe, 0xffffffe0, 0xffffffdf, 0xffffffde, 0xff7fffff, 0x0, 0x7ffffffe,
        ],
    ),
    (
        440,
        0,
        [
            0x0, 0x80000000, 0xf8000000, 0x4000000, 0x84000000, 0x100, 0xffffffff, 0x80000001,
        ],
    ),
    (441, 0, [0xffffffff, 0x1f, 0x1b, 0x1a, 0x1a, 0x8, 0x0, 0x0]),
    (442, 0, [0xffffffff, 0x0, 0x0, 0x5, 0x0, 0x17, 0x0, 0x0]),
    (
        443,
        0,
        [0xffffffff, 0x1f, 0x1b, 0x1a, 0x1a, 0x8, 0xffffffff, 0x1],
    ),
];
const SHARED: &[(u32, u32)] = &[
    (265, 0xd7a5f099),
    (266, 0x2326),
    (267, 0x61a5f099),
    (268, 0x656d),
    (273, 0x7fa12345),
    (274, 0x7fa12345),
    (275, 0x7fa12345),
    (276, 0x7fa12345),
    (286, 0xffffffff),
    (293, 0xff42468a),
    (294, 0x0),
    (295, 0x0),
    (282, 0xf42468a0),
    (278, 0x3fd091a),
    (280, 0x3fd091a),
    (283, 0x7fa12345),
    (284, 0x7fa12345),
    (285, 0x0),
    (439, 0x805edcba),
    (440, 0xa2c485fe),
    (441, 0x1),
    (442, 0x0),
    (443, 0x1),
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
    let input: Vec<_> = (0..32).map(|i| INPUT[i % 8]).collect();
    // 41 scalar/runtime position programs; SRC1 is generic for every binary.
    for &(op, pos, answers) in RUNTIME {
        let mut src = [8, 8, 128];
        src[pos] = 257;
        let dst = if pos == 0 { 1 } else { 2 };
        let mut body = vec![0xbe880383];
        body.extend(insn(op, dst, src));
        let want: Vec<_> = (0..32).map(|i| answers[i % 8]).collect();
        run_case(gpu, &program(&body, dst), &input, &want);
    }
    // 23 masked/flag programs with literal SRC1, plus 23 shared-literal programs.
    for (i, &(op, _, answers)) in RUNTIME.iter().filter(|(_, pos, _)| *pos == 0).enumerate() {
        let mask = [0, 0xaaaaaaaa, 0x80000000][i % 3];
        let scc = i % 2 == 0;
        let mut body = vec![
            0x7e040287,
            0xbeea0381,
            if scc { 0xbf068080 } else { 0xbf068180 },
            0xbefe03ff,
            mask,
        ];
        body.extend(insn(op, 2, [257, 255, 128]));
        if arity(op) == 2 {
            body.push(3);
        }
        body.extend([
            0xbefe03c1,
            0x7e0a026a,
            0xbf840001,
            29 << 25 | 2 << 17 | 2 << 9 | 261,
        ]);
        let want: Vec<_> = (0..32)
            .map(|lane| {
                let x = if mask & (1 << lane) != 0 {
                    answers[lane % 8]
                } else {
                    7
                };
                if scc { x ^ 1 } else { x }
            })
            .collect();
        run_case(gpu, &program(&body, 2), &input, &want);
    }
    // Five scalar-source unary programs; unary padding never reads s8.
    for (op, want) in [
        (439, 0xfffffffc),
        (440, 0xc0000000),
        (441, 30),
        (442, 0),
        (443, 30),
    ] {
        let mut body = vec![0xbe880383];
        body.extend(insn(op, 2, [8, 128, 128]));
        run_case(gpu, &program(&body, 2), &[0; 32], &[want; 32]);
    }
    for &(op, want) in SHARED {
        let mut body = insn(op, 2, [255, 255, 128]).to_vec();
        body.push(0x7fa12345);
        run_case(gpu, &program(&body, 2), &[0; 32], &[want; 32]);
    }
    // 93 dispatches including the actual assembled rolling-checksum fixture.
    run_case(gpu, FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
#[test]
fn cpu_runtime_literal_batches() {
    batches(None);
}
#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_alias_batches() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    batches(Some(&xenora_metal::NativeCompute::new().unwrap()));
}

#[test]
fn two_scalar_identities_and_decoded_add_compatibility() {
    for op in OPS.into_iter().filter(|op| arity(*op) == 2) {
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
            check(&program(&body, 2), &[0], &[spec(op, 3, 3)]);
        }
    }
    let code = bytes(&insn(293, 3, [257, 258, 128]));
    assert!(matches!(
        xenora_ref::decode(&code).unwrap()[0].op,
        xenora_ref::Op::Add(3, 1, 2)
    ));
    let code = bytes(&insn(293, 3, [257, 129, 128]));
    assert!(matches!(
        xenora_ref::decode(&code).unwrap()[0].op,
        xenora_ref::Op::VectorBinaryAlias(
            xenora_ref::VectorAlias::Integer(xenora_ref::VectorInteger::Add),
            3,
            _,
            _
        )
    ));
}

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vop3-aliases.bin");
const FIXTURE_INPUT: [u32; 12] = [
    0x0, 0x1, 0xffffffff, 0x80000000, 0x80000001, 0x7fffffff, 0x7fa12345, 0x800000, 0xffffff, 0x1f,
    0x20, 0x21,
];
// Assembly folds its 23 results as h=33*h+result, then adds s32=3.
// Multiplication by v2 uses low24(v2)=1; shifts use count3 and x.
const FIXTURE_EXPECTED: [u32; 12] = [
    0xbca2362, 0x95cd6f70, 0xabe4db33, 0xc9067406, 0x15cd77d4, 0x1e9adad5, 0x4be2c95, 0x354a632b,
    0xf54fb065, 0x8db0414, 0x305cee46, 0xba5fb14e,
];
#[test]
fn assembled_alias_fixture_literal_answers() {
    check(FIXTURE, &FIXTURE_INPUT, &FIXTURE_EXPECTED);
}
