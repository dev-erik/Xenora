// SPDX-License-Identifier: GPL-3.0-or-later
//! Original fixed answers from AMD RDNA2 p185; no production evaluator supplies expectations.
#[path = "../independent_ref.rs"]
mod independent_ref;
use xenora_shader::{Architecture::Gfx1030, compile};
const ANSWERS: &[(u32, u32, u32)] = &[
    (0, 1, 0xffffffff),
    (0x80000000, 1, 0x7fffffff),
    (0x7fffffff, 0xffffffff, 0x80000000),
    (0xffffffff, 0x80000000, 0x7fffffff),
    (0x80000000, 0x7fffffff, 1),
    (0x7fa12345, 0x12345678, 0x6d6ccccd),
    (0xffffffff, 0xffffffff, 0),
];
fn insn(dst: u32, src: [u32; 2]) -> [u32; 2] {
    [0xd7760000 | dst, src[0] | src[1] << 9 | 128 << 18]
}
fn program(body: &[u32], dst: u32) -> Vec<u8> {
    let mut w = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    w.extend(body);
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
struct Case {
    code: Vec<u8>,
    input: Vec<u32>,
    expected: Vec<u32>,
}
fn cases(clamp: bool) -> Vec<Case> {
    let insn = |dst, src| {
        let mut words = insn(dst, src);
        if clamp {
            words[0] |= 0x8000;
        }
        words
    };
    let mut cases = Vec::new();
    for (index, &(a, b, wrapped)) in ANSWERS.iter().enumerate() {
        let want = if clamp {
            [
                0xffffffff, 0x80000000, 0x7fffffff, 0x7fffffff, 0x80000000, 0x6d6ccccd, 0,
            ][index]
        } else {
            wrapped
        };
        for dst in [1, 2, 255] {
            for mask in [u32::MAX, 0x80000005, 0] {
                let old = match dst {
                    1 => a,
                    2 => b,
                    _ => 0x13579bdf,
                };
                let mut body = vec![0x7e0402ff, b];
                if dst == 255 {
                    body.extend([0x7e0002ff | dst << 17, old]);
                }
                body.extend([0xbefe03ff, mask]);
                body.extend(insn(dst, [257, 258]));
                body.push(0xbefe03c1);
                cases.push(Case {
                    code: program(&body, dst),
                    input: vec![a; 35],
                    expected: (0..35)
                        .map(|i| {
                            if mask & (1 << (i % 32)) != 0 {
                                want
                            } else {
                                old
                            }
                        })
                        .collect(),
                });
            }
        }
        for pos in 0..2 {
            for selector in [8, 255] {
                let mut src = [257, 258];
                src[pos] = selector;
                let value = [a, b][pos];
                let mut body = vec![0x7e0402ff, b, 0xbe8803ff, value];
                body.extend(insn(3, src));
                if selector == 255 {
                    body.push(value);
                }
                cases.push(Case {
                    code: program(&body, 3),
                    input: vec![a; 3],
                    expected: vec![want; 3],
                });
            }
        }
    }
    for (src, literal, want) in [
        ([255, 255], Some(0x80000000), 0),
        ([240, 129], None, 0x3effffff),
        ([129, 240], None, 0xc1000001),
        ([193, 129], None, 0xfffffffe),
    ] {
        let mut body = insn(3, src).to_vec();
        if let Some(v) = literal {
            body.push(v);
        }
        cases.push(Case {
            code: program(&body, 3),
            input: vec![0; 32],
            expected: vec![want; 32],
        });
    }
    for vcc in [0, 1] {
        for scc in [false, true] {
            let mut body = vec![0xbeea0380 | vcc, if scc { 0xbf068080 } else { 0xbf068180 }];
            body.extend(insn(2, [128, 129]));
            body.extend([0x7e0a026a, 0xbf840001, 29 << 25 | 2 << 17 | 2 << 9 | 261]);
            cases.push(Case {
                code: program(&body, 2),
                input: vec![0],
                expected: vec![if scc { u32::MAX ^ vcc } else { u32::MAX }],
            });
        }
    }
    assert_eq!(cases.len(), 99);
    cases
}
fn check(case: &Case, gpu: Option<&xenora_metal::NativeCompute>) {
    assert_eq!(
        xenora_ref::execute(&case.code, &case.input).unwrap(),
        case.expected
    );
    assert_eq!(
        independent_ref::execute(&case.code, &case.input).unwrap(),
        case.expected
    );
    let shader = compile(Gfx1030, &case.code).unwrap();
    if let Some(gpu) = gpu {
        assert_eq!(
            gpu.run_compiled_words(&shader, &case.input).unwrap(),
            case.expected
        );
    }
}
#[test]
fn fixed_boundaries_aliases_sources_masks_and_flags() {
    for case in cases(false) {
        check(&case, None);
    }
}
#[test]
fn modifiers_and_noncanonical_padding_stay_rejected() {
    for clamp in [false, true] {
        for bit in (8..15).chain(27..32) {
            let mut words = insn(2, [129, 129]);
            if clamp {
                words[0] |= 0x8000;
            }
            words[usize::from(bit >= 27)] |= 1 << bit;
            let code = program(&words, 2);
            assert!(compile(Gfx1030, &code).is_err());
            assert!(xenora_ref::execute(&code, &[0]).is_err());
            assert!(independent_ref::execute(&code, &[0]).is_err());
        }
        let mut words = insn(2, [129, 129]);
        if clamp {
            words[0] |= 0x8000;
        }
        words[1] &= !(511 << 18);
        let code = program(&words, 2);
        assert!(compile(Gfx1030, &code).is_err());
        assert!(xenora_ref::execute(&code, &[0]).is_err());
        assert!(independent_ref::execute(&code, &[0]).is_err());
    }
}

#[test]
#[ignore = "native Metal validation must be enabled"]
fn native_sub_nc_i32_compute() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for case in cases(false).into_iter().chain(cases(true)) {
        check(&case, Some(&gpu));
    }
    check(&fixture_case(), Some(&gpu));
}

fn fixture_case() -> Case {
    Case {
        code: include_bytes!("../../../fixtures/rdna2/vop3-sub-nc-i32.bin").to_vec(),
        input: vec![0, 1, 0x80000000, 0x7fffffff, 0xffffffff],
        expected: vec![0xffffffff, 0, 0x7fffffff, 0x7ffffffe, 0xfffffffe],
    }
}
#[test]
fn assembled_fixture_known_answers() {
    check(&fixture_case(), None);
}

#[test]
fn signed_saturation_boundaries_aliases_sources_masks_and_flags() {
    for case in cases(true) {
        check(&case, None);
    }
}
