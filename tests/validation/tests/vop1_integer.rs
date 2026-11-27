// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent byte programs and literal integer VOP1 answers, AMD RDNA2 12.8.
//! Neither interpreter imports production ISA/shader semantics. This suite
//! compares each interpreter directly with expected words, not with the other.
#[path = "../independent_ref.rs"]
mod independent_ref;

const OPS: [u32; 5] = [55, 56, 57, 58, 59];
const ANCHORS: &[(u32, [u32; 5])] = &[
    (0, [0xffffffff, 0, 0xffffffff, 0xffffffff, 0xffffffff]),
    (0xffffffff, [0, 0xffffffff, 0, 0, 0xffffffff]),
    (1, [0xfffffffe, 0x80000000, 31, 0, 31]),
    (0x80000000, [0x7fffffff, 1, 0, 31, 1]),
    (0x40000000, [0xbfffffff, 2, 1, 30, 1]),
    (0xfffffffe, [1, 0x7fffffff, 0, 1, 31]),
    (0xffff0000, [0x0000ffff, 0x0000ffff, 0, 16, 16]),
    (0x01234567, [0xfedcba98, 0xe6a2c480, 7, 0, 7]),
    (0xaaaaaaaa, [0x55555555, 0x55555555, 0, 1, 1]),
    (0x55555555, [0xaaaaaaaa, 0xaaaaaaaa, 1, 0, 1]),
    (0x7fa12345, [0x805edcba, 0xa2c485fe, 1, 0, 1]),
];

// Bind the actual assembled fixture, not a reconstructed instruction stream.
const COMBINED_FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/vector-unary.bin");
const COMBINED_INPUT: [u32; 11] = [
    0, 0xffffffff, 1, 0x80000000, 0x40000000, 0xfffffffe, 0xffff0000, 0x01234567, 0xaaaaaaaa,
    0x55555555, 0x7fa12345,
];
// Source computes NOT(x) XOR reverse(x) XOR high(x) XOR low(x) XOR signed_high(x).
// These are literal answers derived by bit position from that source logic,
// not a fold of interpreter output or calls to production unary helpers.
// For 0x80000000: 7fffffff ^ 1 ^ 0 ^ 31 ^ 1 = 7fffffe0.
// For 0x40000000: bfffffff ^ 2 ^ 1 ^ 30 ^ 1 = bfffffe3.
// For 0x01234567: fedcba98 ^ e6a2c480 ^ 7 ^ 0 ^ 7 = 187e7e18.
// For 0x7fa12345: 805edcba ^ a2c485fe ^ 1 ^ 0 ^ 1 = 229a5944.
const COMBINED_EXPECTED: [u32; 11] = [
    0, 0, 0x7ffffffe, 0x7fffffe0, 0xbfffffe3, 0x7fffffe0, 0, 0x187e7e18, 0, 0, 0x229a5944,
];

#[test]
fn assembled_combined_fixture_compiles_and_matches_both_independent_references() {
    // CPU three-way check: compiler admission plus each byte-decoding interpreter
    // against literal answers. Translated execution is checked in the ignored
    // native test below; compilation alone is not a native result assertion.
    check(COMBINED_FIXTURE, &COMBINED_INPUT, &COMBINED_EXPECTED);
}

fn unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e000000 | dst << 17 | op << 9 | src
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn program(body: &[u32], output: u32) -> Vec<u8> {
    let mut code = vec![0xe0302000, 0x80000100, 0xbf8c3f70]; // load v1; wait
    code.extend_from_slice(body);
    code.extend([0xe0702000, 0x80010000 | output << 8, 0xbbfd0000, 0xbf810000]);
    bytes(&code)
}
fn check(code: &[u8], input: &[u32], expected: &[u32]) {
    // Production admission is separate from the independent literal oracle.
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code)
        .unwrap_or_else(|error| panic!("production rejected positive {code:02x?}: {error:?}"));
    assert_eq!(
        xenora_ref::execute(code, input).unwrap(),
        expected,
        "first reference: {code:02x?}"
    );
    assert_eq!(
        independent_ref::execute(code, input).unwrap(),
        expected,
        "second reference: {code:02x?}"
    );
}
fn reject(code: &[u8], input: &[u32]) -> xenora_shader::CompileError {
    let error = match xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code) {
        Ok(_) => panic!("production accepted negative {code:02x?}"),
        Err(error) => error,
    };
    assert!(
        xenora_ref::execute(code, input).is_err(),
        "first accepted {code:02x?}"
    );
    assert!(
        independent_ref::execute(code, input).is_err(),
        "second accepted {code:02x?}"
    );
    error
}

fn reject_verify(code: &[u8], input: &[u32], expected_offset: usize, diagnostic: &str) {
    // An unrelated decoder error or later missing output must not satisfy a
    // semantic rejection (especially zero/partial EXEC and runtime regions).
    match reject(code, input) {
        xenora_shader::CompileError::Verify { offset, reason } => {
            assert_eq!(
                offset, expected_offset,
                "{diagnostic}: {reason}; {code:02x?}"
            );
            assert!(
                reason.contains(diagnostic),
                "expected {diagnostic:?}, got {reason:?}"
            );
        }
        other => panic!("expected verification rejection {diagnostic:?}, got {other:?}"),
    }
}

#[test]
fn literal_anchors_and_destination_aliases() {
    for &(input, results) in ANCHORS {
        for (op, expected) in OPS.into_iter().zip(results) {
            for dst in [1, 2, 255] {
                check(&program(&[unary(op, dst, 257)], dst), &[input], &[expected]);
            }
        }
    }
}

#[test]
fn every_one_hot_and_complement_has_independent_positional_answers() {
    for bit in 0..32 {
        let one = 1u32 << bit;
        // Positional identities, not a copy of either reference's scan algorithm.
        let sign_run = if bit == 31 { 1 } else { 31 - bit };
        let answers = [!one, 1 << (31 - bit), 31 - bit, bit, sign_run];
        let complements = [
            one,
            !(1 << (31 - bit)),
            if bit == 31 { 1 } else { 0 },
            if bit == 0 { 1 } else { 0 },
            sign_run,
        ];
        for (op, (a, b)) in OPS.into_iter().zip(answers.into_iter().zip(complements)) {
            check(&program(&[unary(op, 1, 257)], 1), &[one, !one], &[a, b]);
        }
    }
}

#[test]
fn scalar_vcc_literal_and_high_vgpr_sources_decode_independently() {
    for &(value, answers) in ANCHORS {
        for (op, expected) in OPS.into_iter().zip(answers) {
            for reg in [8, 105, 106] {
                let code = program(&[0xbe8003ff | reg << 16, value, unary(op, 2, reg)], 2);
                check(&code, &[0], &[expected]);
            }
            check(&program(&[unary(op, 2, 255), value], 2), &[0], &[expected]);
            // 496..504 are v240..v248, never inline 240..248 after truncation.
            for reg in 240..=248 {
                let code = program(
                    &[0x7e0002ff | reg << 17, value, unary(op, reg, 256 + reg)],
                    reg,
                );
                check(&code, &[0], &[expected]);
            }
        }
    }
    // Inline integer endpoints and all nine float-named raw bit selectors.
    // Values below are explicit NOT answers; also run the other four operations
    // against the literal anchors for inline zero, one and minus one.
    for (selector, expected) in [
        (128, 0xffffffffu32),
        (129, 0xfffffffe),
        (192, 0xffffffbf),
        (193, 0),
        (208, 15),
        (240, 0xc0ffffff),
        (241, 0x40ffffff),
        (242, 0xc07fffff),
        (243, 0x407fffff),
        (244, 0xbfffffff),
        (245, 0x3fffffff),
        (246, 0xbf7fffff),
        (247, 0x3f7fffff),
        (248, 0xc1dd067c),
    ] {
        check(&program(&[unary(55, 2, selector)], 2), &[0], &[expected]);
    }
    for (selector, index) in [(128, 0), (129, 2), (193, 1)] {
        for (op, answer) in OPS.into_iter().zip(ANCHORS[index].1) {
            check(&program(&[unary(op, 2, selector)], 2), &[0], &[answer]);
        }
    }
}

#[test]
fn exec_masks_preserve_initialized_inactive_destinations() {
    for mask in [0u32, 0xffffffff, 1, 0x80000000, 0x55555555, 0xaaaaaaaa] {
        for (op, answer) in OPS.into_iter().zip(ANCHORS[2].1) {
            for dst in [1, 2] {
                let code = program(
                    &[
                        0x7e0402ff,
                        0x13579bdf, // inactive distinct-destination sentinel
                        0xbefe03ff,
                        mask,
                        unary(op, dst, 257),
                        0xbefe03c1, // restore full EXEC before the store
                    ],
                    dst,
                );
                let expected: Vec<_> = (0..32)
                    .map(|lane| {
                        if mask & (1 << lane) != 0 {
                            answer
                        } else if dst == 1 {
                            1
                        } else {
                            0x13579bdf
                        }
                    })
                    .collect();
                check(&code, &[1; 32], &expected);
            }
        }
    }
}

#[test]
fn zero_exec_skips_reads_but_nonzero_partial_exec_validates_all_lanes() {
    for op in OPS {
        for src in [20, 106, 259] {
            // undefined SGPR, VCC and VGPR
            check(
                &program(&[0x7e040281, 0xbefe0380, unary(op, 2, src), 0xbefe03c1], 2),
                &[0],
                &[1],
            );
            for mask in [1u32, 0x80000000, 0xffffffff] {
                let diagnostic = match src {
                    20 => "scalar source has no known value",
                    106 => "vector unary reads uninitialized VCC_LO",
                    _ => "source register is uninitialized",
                };
                reject_verify(
                    &program(
                        &[0x7e040281, 0xbefe03ff, mask, unary(op, 2, src), 0xbefe03c1],
                        2,
                    ),
                    &[0],
                    24,
                    diagnostic,
                );
            }
        }
        // Empty EXEC need not define its destination; store an existing register.
        check(
            &program(&[0xbefe0380, unary(op, 3, 259), 0xbefe03c1], 1),
            &[7],
            &[7],
        );
        // A nonzero partial mask cannot introduce a new destination.
        reject_verify(
            &program(&[0xbefe0381, unary(op, 2, 257), 0xbefe03c1], 2),
            &[7],
            16,
            "partial EXEC requires an initialized vector destination",
        );
    }
}

#[test]
fn unary_preserves_scc_including_unset_and_vcc_including_lane_masks() {
    for op in OPS {
        for flag in [false, true] {
            let code = program(
                &[
                    if flag { 0xbf068080 } else { 0xbf068180 },
                    unary(op, 2, 257),
                    0x7e060287, // v3=7, MOV preserves SCC
                    0xbf840001, // if SCC=0 skip v3=9
                    0x7e060289,
                ],
                3,
            );
            check(&code, &[0, 0xffffffff], &[if flag { 9 } else { 7 }; 2]);
        }
        reject_verify(
            &program(&[unary(op, 2, 257), 0xbf840001, 0x7e040280], 2),
            &[0],
            16,
            "conditional branch reads uninitialized SCC",
        );
        // Scalar-written VCC is preserved, including when used as the source.
        check(
            &program(&[0xbeea03ff, 0x55aa55aa, unary(op, 2, 106), 0x7e06026a], 3),
            &[0],
            &[0x55aa55aa],
        );
        // A lane-derived mask (lanes 1 and 3) must remain 0b1010 after unary.
        check(
            &program(&[0x7d8a0280, unary(op, 2, 106), 0x7e06026a], 3),
            &[0, 1, 0, 1],
            &[10; 4],
        );
        // Reading an unset VCC after unary still fails.
        reject_verify(
            &program(&[unary(op, 2, 257), 0x7e06026a], 3),
            &[0],
            16,
            "scalar source has no known value",
        );
    }
}

#[test]
fn invalid_sources_pending_loads_destinations_and_runtime_regions_stay_rejected() {
    for op in OPS {
        for src in (107..=127).chain(209..=239).chain(249..=254) {
            reject(&program(&[unary(op, 2, src)], 2), &[0]);
            // Empty execution never bypasses malformed instruction decoding.
            reject(
                &program(&[0xbefe0380, unary(op, 2, src), 0xbefe03c1], 1),
                &[0],
            );
        }
        reject(&bytes(&[unary(op, 2, 255)]), &[0]); // missing literal
        reject_verify(
            &program(&[unary(op, 0, 257)], 1),
            &[0],
            12,
            "instruction writes the index register",
        );
        reject_verify(
            &program(&[unary(op, 2, 256)], 2),
            &[0],
            12,
            "source register is uninitialized, pending a load wait, or reserved for indexing",
        );
        // No wait between the load and the source read / pending destination write.
        for (instruction, diagnostic) in [
            (
                unary(op, 2, 257),
                "source register is uninitialized, pending a load wait",
            ),
            (
                unary(op, 1, 129),
                "instruction writes the index register or replaces a pending load",
            ),
        ] {
            reject_verify(
                &bytes(&[
                    0xe0302000,
                    0x80000100,
                    instruction,
                    0xbf8c3f70,
                    0xe0702000,
                    0x80010200,
                    0xbbfd0000,
                    0xbf810000,
                ]),
                &[0],
                8,
                diagnostic,
            );
        }
        // Runtime VCC region remains excluded even with initialized registers,
        // whether its branch would be taken or not.
        let code = program(&[0x7e040280, 0x7d8a0280, 0xbf860001, unary(op, 2, 257)], 2);
        reject_verify(&code, &[0], 24, "code skipped by a runtime s_cbranch_vccz");
        reject_verify(&code, &[1], 24, "code skipped by a runtime s_cbranch_vccz");
        // VOP3 aliases are outside this batch.
        reject(
            &program(&[0xd0000002 | (op + 384) << 16, 0x02020101], 2),
            &[0],
        );
    }
}

#[test]
#[ignore = "requires native Metal with API and shader validation"]
fn integer_vop1_native_batched_runtime_inputs_and_exec_masks() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    // 75 runtime input words in one dispatch: 11 literal anchors plus all 32
    // one-hot words and complements. Five operations * six masks = 30 programs,
    // plus one assembled combined fixture below: 31 total. No per-input shader
    // compilation and no change to the native gate deadline.
    let mut inputs: Vec<u32> = ANCHORS.iter().map(|&(value, _)| value).collect();
    let mut answers: Vec<[u32; 5]> = ANCHORS.iter().map(|&(_, result)| result).collect();
    for bit in 0..32 {
        let one = 1u32 << bit;
        let sign_run = if bit == 31 { 1 } else { 31 - bit };
        inputs.extend([one, !one]);
        answers.push([!one, 1 << (31 - bit), 31 - bit, bit, sign_run]);
        answers.push([
            one,
            !(1 << (31 - bit)),
            if bit == 31 { 1 } else { 0 },
            if bit == 0 { 1 } else { 0 },
            sign_run,
        ]);
    }
    assert_eq!(inputs.len(), 75);
    for (column, op) in OPS.into_iter().enumerate() {
        for mask in [0u32, 0xffffffff, 1, 0x80000000, 0x55555555, 0xaaaaaaaa] {
            let code = program(
                &[
                    0x7e0402ff,
                    0x13579bdf,
                    0xbefe03ff,
                    mask,
                    unary(op, 2, 257),
                    0xbefe03c1,
                ],
                2,
            );
            let expected: Vec<_> = answers
                .iter()
                .enumerate()
                .map(|(index, result)| {
                    if mask & (1 << (index % 32)) != 0 {
                        result[column]
                    } else {
                        0x13579bdf
                    }
                })
                .collect();
            let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code)
                .unwrap_or_else(|error| panic!("op={op} mask={mask:#010x}: {error:?}"));
            let actual = gpu
                .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &inputs)
                .unwrap();
            assert_eq!(actual, expected, "op={op} mask={mask:#010x}");
        }
    }
    // One additional compilation and one batched execution of the checked-in
    // assembled fixture. Every readback word is compared with a literal answer.
    let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, COMBINED_FIXTURE)
        .expect("compile assembled combined vector-unary fixture");
    let actual = gpu
        .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &COMBINED_INPUT)
        .unwrap();
    assert_eq!(
        actual, COMBINED_EXPECTED,
        "assembled vector-unary.bin readback"
    );
}
