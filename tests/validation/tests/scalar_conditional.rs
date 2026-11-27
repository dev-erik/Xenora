// SPDX-License-Identifier: GPL-3.0-or-later
//! Original scalar bit/conditional programs; literal answers from RDNA2 12.2–12.4.
#[path = "../independent_ref.rs"]
mod independent_ref;

const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/scalar-conditional.bin");
const FIXTURE_INPUT: [u32; 7] = [
    0, 1, 0xffffffff, 0x80000000, 0x7fffffff, 0x7fa12345, 0x005edcbb,
];
// Set bit31: ffa12345; clear bit0: ffa12344. Both predicates are true,
// so s9=ffa12344 and s10=ffffffff. The two XORs leave the mask 005edcbb.
const FIXTURE_EXPECTED: [u32; 7] = [
    0x005edcbb, 0x005edcba, 0xffa12344, 0x805edcbb, 0x7fa12344, 0x7ffffffe, 0,
];

#[test]
fn assembled_fixture_has_independent_literal_answers() {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, FIXTURE).unwrap();
    assert_eq!(
        xenora_ref::execute(FIXTURE, &FIXTURE_INPUT).unwrap(),
        FIXTURE_EXPECTED
    );
    assert_eq!(
        independent_ref::execute(FIXTURE, &FIXTURE_INPUT).unwrap(),
        FIXTURE_EXPECTED
    );
}

fn sop1(op: u32, dst: u32, src: u32) -> u32 {
    0xbe800000 | dst << 16 | op << 8 | src
}
fn sopc(op: u32, src: u32, index: u32) -> u32 {
    0xbf000000 | op << 16 | index << 8 | src
}
fn cmovk(dst: u32, imm: u16) -> u32 {
    0xb1000000 | dst << 16 | u32::from(imm)
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn program(body: &[u32], output: u32) -> Vec<u8> {
    let mut words = body.to_vec();
    words.extend([
        0xbefe03c1,
        0x7e020200 | output,
        0xe0702000,
        0x80010100,
        0xbbfd0000,
        0xbf810000,
    ]);
    bytes(&words)
}
fn check(code: &[u8], expected: u32) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code)
        .unwrap_or_else(|e| panic!("positive {code:02x?}: {e:?}"));
    assert_eq!(xenora_ref::execute(code, &[0; 32]).unwrap(), [expected; 32]);
    assert_eq!(
        independent_ref::execute(code, &[0; 32]).unwrap(),
        [expected; 32]
    );
}
fn reject(code: &[u8], at: usize, diagnostic: &str, decode: bool) {
    let e = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err();
    match &e {
        xenora_shader::CompileError::Decode(e) if decode => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } if !decode => assert_eq!(*offset, at),
        _ => panic!("wrong rejection: {e:?}"),
    }
    assert!(e.to_string().contains(diagnostic), "{e}");
    let a = xenora_ref::execute(code, &[0]).unwrap_err();
    assert_eq!(a.offset, at, "{a:?}");
    let b = independent_ref::execute(code, &[0]).unwrap_err();
    assert!(b.starts_with(&format!("{at:#x}:")), "{b}");
    if diagnostic == "SCC" {
        assert!(a.detail.contains("SCC"));
        assert!(b.contains("SCC"));
    }
}
fn flag(flag: bool) -> u32 {
    if flag { 0xbf068080 } else { 0xbf068180 }
}
fn observe_flag(body: &mut Vec<u32>) {
    // s9 starts at 7; incoming true SCC changes it to 9.
    body.extend([0xbe890387, 0xbf840001, 0xbe890389]);
}

#[test]
fn bitset_literal_words_aliases_flags_and_exec() {
    let cases = [
        (0, 0, 0, 1),
        (0xffffffff, 0, 0xfffffffe, 0xffffffff),
        (0xffffffff, 31, 0x7fffffff, 0xffffffff),
        (0x80000000, 63, 0, 0x80000000),
        (0x55555555, 33, 0x55555555, 0x55555557),
        (0xaaaaaaaa, 32, 0xaaaaaaaa, 0xaaaaaaab),
        (0, 0xffffffff, 0, 0x80000000),
    ];
    for (old, index, clear, set) in cases {
        for (op, answer) in [(27, clear), (29, set)] {
            for mask in [0, 1, 0xffffffff] {
                for incoming in [None, Some(false), Some(true)] {
                    let mut body = vec![0xbe8803ff, old, 0xbefe03ff, mask];
                    if let Some(f) = incoming {
                        body.push(flag(f));
                    }
                    body.extend([sop1(op, 8, 255), index]);
                    check(&program(&body, 8), answer);
                    if let Some(f) = incoming {
                        observe_flag(&mut body);
                        check(&program(&body, 9), if f { 9 } else { 7 });
                    } else {
                        let at = body.len() * 4;
                        body.push(0xbf840000);
                        reject(&program(&body, 8), at, "SCC", false);
                    }
                }
            }
        }
    }
    for (old, clear, set) in [
        (0, 0, 1),
        (1, 1, 3),
        (31, 31, 0x8000001f),
        (0xffffffff, 0x7fffffff, 0xffffffff),
    ] {
        for (op, answer) in [(27, clear), (29, set)] {
            check(&program(&[0xbe8803ff, old, sop1(op, 8, 8)], 8), answer);
        }
    }
    // Independent one-hot/complement anchors exercise every index.
    for bit in 0..32 {
        let one = 2u32.pow(bit);
        check(&program(&[0xbe8803ff, one, sop1(27, 8, 128 + bit)], 8), 0);
        check(
            &program(&[0xbe8803ff, !one, sop1(29, 8, 128 + bit)], 8),
            0xffffffff,
        );
    }
}

#[test]
fn bitcompare_initializes_replaces_scc_and_preserves_sources() {
    for bit in 0..32 {
        for (word, is_one) in [
            (0, false),
            (0xffffffff, true),
            (2u32.pow(bit), true),
            (!2u32.pow(bit), false),
        ] {
            for op in [12, 13] {
                for incoming in [None, Some(false), Some(true)] {
                    let mut body = vec![0xbe8803ff, word, 0xbe8903ff, bit + 32, 0xbefe0380];
                    if let Some(f) = incoming {
                        body.push(flag(f));
                    }
                    body.push(sopc(op, 8, 9));
                    check(&program(&body, 8), word);
                    check(&program(&body, 9), bit + 32);
                    observe_flag(&mut body);
                    check(&program(&body, 9), if is_one == (op == 13) { 9 } else { 7 });
                }
            }
        }
    }
    for op in [12, 13] {
        for words in [
            vec![sopc(op, 255, 159), 0x80000000],
            vec![sopc(op, 193, 255), 63],
        ] {
            let mut body = words;
            observe_flag(&mut body);
            check(&program(&body, 9), if op == 13 { 9 } else { 7 });
        }
    }
}

#[test]
fn conditional_moves_are_lazy_preserve_flags_and_execute_without_exec() {
    for mask in [0, 1, 0xffffffff] {
        for take in [false, true] {
            for (src, extra, answer) in [
                (9, Some(0x7fa12345), 0x7fa12345),
                (193, None, 0xffffffff),
                (248, None, 0x3e22f983),
                (255, Some(0x80000000), 0x80000000),
            ] {
                let mut body = vec![0xbe880387, 0xbefe03ff, mask, flag(take)];
                if src == 9 {
                    body.extend([0xbe8903ff, extra.unwrap()]);
                }
                body.push(sop1(5, 8, src));
                if src == 255 {
                    body.push(extra.unwrap());
                }
                check(&program(&body, 8), if take { answer } else { 7 });
                observe_flag(&mut body);
                check(&program(&body, 9), if take { 9 } else { 7 });
            }
            for (imm, answer) in [
                (0, 0),
                (1, 1),
                (0x7fff, 0x7fff),
                (0x8000, 0xffff8000),
                (0xffff, 0xffffffff),
            ] {
                let mut body = vec![0xbe880387, 0xbefe03ff, mask, flag(take), cmovk(8, imm)];
                check(&program(&body, 8), if take { answer } else { 7 });
                observe_flag(&mut body);
                check(&program(&body, 9), if take { 9 } else { 7 });
            }
        }
    }
    for dst in [8, 105] {
        check(
            &program(&[flag(true), sop1(5, dst, 255), 0xdeadbeef], dst),
            0xdeadbeef,
        );
        check(&program(&[flag(true), cmovk(dst, 0x8000)], dst), 0xffff8000);
        for src in [dst, 20] {
            check(&program(&[flag(false), sop1(5, dst, src)], 128), 0);
            reject(
                &program(&[flag(false), sop1(5, dst, src)], dst),
                12,
                "no known value",
                false,
            );
            reject(
                &program(&[flag(true), sop1(5, dst, src)], 128),
                4,
                "no known value",
                false,
            );
        }
        check(&program(&[flag(false), cmovk(dst, 1)], 128), 0);
        reject(
            &program(&[flag(false), cmovk(dst, 1)], dst),
            12,
            "no known value",
            false,
        );
        for instruction in [sop1(5, dst, 128), cmovk(dst, 1)] {
            reject(&program(&[instruction], 128), 0, "SCC", false);
        }
    }
}

#[test]
fn invalid_reads_destinations_and_encodings_do_not_hide_behind_false_or_zero_exec() {
    for op in [27, 29] {
        reject(
            &program(&[sop1(op, 8, 128)], 128),
            0,
            "no known value",
            false,
        );
        reject(
            &program(&[0xbe880380, 0xbefe0380, sop1(op, 8, 20)], 128),
            8,
            "no known value",
            false,
        );
    }
    for op in [12, 13] {
        for (a, b) in [(20, 128), (128, 20)] {
            reject(
                &program(&[0xbefe0380, sopc(op, a, b)], 128),
                4,
                "no known value",
                false,
            );
        }
        reject(&bytes(&[sopc(op, 255, 255), 0]), 0, "", true);
    }
    for dst in 0..8 {
        for instruction in [
            sop1(5, dst, 20),
            cmovk(dst, 1),
            sop1(27, dst, 128),
            sop1(29, dst, 128),
        ] {
            reject(
                &program(&[flag(false), instruction], 128),
                4,
                "descriptor",
                false,
            );
        }
    }
    for op in [5, 27, 29] {
        for src in (108..=125).chain(209..=239).chain(249..=254) {
            reject(&program(&[flag(false), sop1(op, 8, src)], 128), 4, "", true);
        }
        for dst in [106, 107, 126, 127] {
            reject(
                &program(&[flag(false), sop1(op, dst, 128)], 128),
                4,
                "",
                true,
            );
        }
        reject(&bytes(&[flag(false), sop1(op, 8, 255)]), 4, "", true);
    }
    for op in [6, 28, 30] {
        reject(&program(&[sop1(op, 8, 128)], 128), 0, "", true);
    }
    for op in [14, 15] {
        reject(&program(&[sopc(op, 128, 128)], 128), 0, "", true);
    }
    for dst in [106, 127] {
        reject(&program(&[flag(false), cmovk(dst, 0)], 128), 4, "", true);
    }
}

#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn native_scalar_conditional_literal_results_and_flags() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    let fixture = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, FIXTURE).unwrap();
    assert_eq!(
        gpu.run_words(&fixture.msl, xenora_shader::ENTRY_POINT, &FIXTURE_INPUT)
            .unwrap(),
        FIXTURE_EXPECTED
    );

    // Eight original programs, each batching a full wave. The last two observe
    // BITCMP through SCC-controlled writes rather than a production evaluator.
    let cases = [
        (vec![0xbe8803c1, sop1(27, 8, 159)], 0x7fffffff),
        (vec![0xbe880380, sop1(29, 8, 255), 63], 0x80000000),
        (vec![flag(false), 0xbe880387, sop1(5, 8, 20)], 7),
        (vec![flag(true), sop1(5, 8, 255), 0x7fa12345], 0x7fa12345),
        (vec![flag(true), cmovk(8, 0x8000)], 0xffff8000),
        (vec![flag(false), 0xbe880387, cmovk(8, 0xffff)], 7),
        (vec![sopc(12, 128, 159), cmovk(8, 9)], 9),
        (vec![sopc(13, 193, 159), cmovk(8, 9)], 9),
    ];
    for (case, (body, answer)) in cases.into_iter().enumerate() {
        for mask in [0, 0xffffffff] {
            let mut masked = vec![flag(true), 0xbefe03ff, mask];
            masked.extend(&body);
            let mut with_flag = masked.clone();
            observe_flag(&mut with_flag);
            for (code, answer) in [
                (program(&masked, 8), answer),
                (
                    program(&with_flag, 9),
                    if matches!(case, 2 | 5) { 7 } else { 9 },
                ),
            ] {
                check(&code, answer);
                let compiled =
                    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
                assert_eq!(
                    gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[0; 32])
                        .unwrap(),
                    [answer; 32]
                );
            }
        }
    }
}

#[test]
fn all_forms_preserve_vcc_exec_and_inline_word_bits() {
    for instruction in [
        sop1(27, 8, 128),
        sop1(29, 8, 128),
        sopc(12, 128, 128),
        sopc(13, 128, 128),
        sop1(5, 8, 129),
        cmovk(8, 1),
    ] {
        for mask in [0, 1, 0xffffffff] {
            let mut body = vec![
                0xbeea03ff,
                0x55aa55aa,
                0xbe880387,
                flag(true),
                0xbefe03ff,
                mask,
                instruction,
            ];
            check(&program(&body, 106), 0x55aa55aa);
            body.extend([0xbe890387, 0xbf880001, 0xbe890389]);
            check(&program(&body, 9), if mask == 0 { 7 } else { 9 });
        }
    }
    let words = [
        0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000,
        0xc0800000, 0x3e22f983,
    ];
    for (i, word) in words.into_iter().enumerate() {
        let src = 240 + i as u32;
        check(&program(&[flag(true), sop1(5, 8, src)], 8), word);
        check(
            &program(&[0xbe880380, sop1(29, 8, src)], 8),
            if i == 8 { 8 } else { 1 },
        );
        for op in [12, 13] {
            let mut body = vec![sopc(op, src, 128)];
            observe_flag(&mut body);
            check(
                &program(&body, 9),
                if (i == 8) == (op == 13) { 9 } else { 7 },
            );
        }
    }
    for op in [12, 13] {
        for bad in (108..=125).chain(209..=239).chain(249..=254) {
            for (a, b) in [(bad, 128), (128, bad)] {
                reject(&program(&[sopc(op, a, b)], 128), 0, "", true);
            }
        }
    }
}

#[test]
fn truncated_scalar_literals_are_attributed_to_the_owning_instruction() {
    for instruction in [
        sop1(5, 8, 255),
        sop1(27, 8, 255),
        sop1(29, 8, 255),
        sopc(12, 255, 128),
        sopc(12, 128, 255),
        sopc(13, 255, 128),
        sopc(13, 128, 255),
    ] {
        for available in 0..4 {
            let mut code = bytes(&[flag(false), instruction]);
            code.extend_from_slice(&[0x45, 0x23, 0xa1][..available]);
            if available == 0 {
                reject(&code, 4, "", true);
            } else {
                // The compiler rejects non-word-aligned fixtures before decode.
                assert!(
                    matches!(xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code),
                    Err(xenora_shader::CompileError::InvalidLength { bytes }) if bytes == code.len())
                );
                assert_eq!(xenora_ref::execute(&code, &[0]).unwrap_err().offset, 4);
            }
            assert_eq!(
                independent_ref::execute(&code, &[0]).unwrap_err(),
                "0x4: truncated scalar literal"
            );
        }
    }
}

#[test]
fn mask_sources_keep_initialization_and_conditional_read_rules() {
    for src in [106, 107, 127] {
        for op in [5, 27, 29] {
            reject(
                &program(&[0xbe880380, flag(true), sop1(op, 8, src)], 128),
                8,
                "no known value",
                false,
            );
        }
        for op in [12, 13] {
            for (a, b) in [(src, 128), (128, src)] {
                reject(&program(&[sopc(op, a, b)], 128), 0, "no known value", false);
            }
        }
    }
    // These new mask operands are outside the legacy third reader's coverage.
    // False CMOV does not read its unknown source; EXEC_LO starts at all ones.
    for (body, expected) in [
        (vec![0xbe880387, flag(false), sop1(5, 8, 106)], 7),
        (vec![flag(true), sop1(5, 8, 126)], 0xffffffff),
        (vec![0xbe8803c1, sop1(27, 8, 126)], 0x7fffffff),
        (vec![0xbe880380, sop1(29, 8, 126)], 0x80000000),
    ] {
        let code = program(&body, 8);
        xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            xenora_ref::execute(&code, &[0; 32]).unwrap(),
            [expected; 32]
        );
    }
}
