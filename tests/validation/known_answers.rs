// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod independent_ref;

#[cfg(test)]
mod review {
    use super::independent_ref;
    use xenora_shader::{Architecture, compile};

    const FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/add.bin");

    const OR_FIXTURE: &[u8; 40] = include_bytes!("../../fixtures/rdna2/or.bin");
    const OR_INPUT: [u32; 7] = [
        0,
        1,
        u32::MAX,
        0x8055_00aa,
        0x7faa_ff55,
        0x8000_0000,
        0x0055_00aa,
    ];
    const OR_EXPECTED: [u32; 7] = [
        0x8055_00aa,
        0x8055_00ab,
        u32::MAX,
        0x8055_00aa,
        u32::MAX,
        0x8055_00aa,
        0x8055_00aa,
    ];

    #[test]
    fn vector_or_matches_known_answers_and_preserves_unmasked_bits() {
        verify(OR_FIXTURE, &OR_INPUT, Some(&OR_EXPECTED));
        let input: Vec<_> = (0..32).map(|bit| 1_u32 << bit).collect();
        let expected: Vec<_> = input
            .iter()
            .map(|word| 0x8055_00aa + (word & 0x7faa_ff55))
            .collect();
        verify(OR_FIXTURE, &input, Some(&expected));

        let mut renamed = *OR_FIXTURE;
        // Reuse v255 as both destination and the second source.
        renamed[20..24].copy_from_slice(&0x39ff_ff01_u32.to_le_bytes());
        renamed[29] = 255;
        verify(&renamed, &OR_INPUT, Some(&OR_EXPECTED));
    }

    #[test]
    fn vector_or_rejects_invalid_operands_and_incomplete_streams() {
        for instruction in [
            0x3803_ff03_u32, // Uninitialized v3 in SRC0.
            0x3802_0701,     // Uninitialized v3 in VSRC1.
            0x3801_ff01,     // Write to synthetic index v0.
            0x3803_fe6b,     // Reserved SRC0 selector107.
        ] {
            let mut code = *OR_FIXTURE;
            code[20..24].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &OR_INPUT, None);
        }
        for end in 0..OR_FIXTURE.len() {
            verify(&OR_FIXTURE[..end], &OR_INPUT, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn vector_or_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, OR_FIXTURE).unwrap();
        let input: Vec<_> = OR_INPUT.into_iter().cycle().take(131).collect();
        let expected: Vec<_> = OR_EXPECTED.into_iter().cycle().take(input.len()).collect();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    const ASHR_FIXTURE: &[u8; 40] = include_bytes!("../../fixtures/rdna2/ashr.bin");
    const ASHR_VALUE: i32 = i32::MIN + 1; // The fixture literal 0x8000_0001.

    fn arithmetic_shift_expected(value: i32, count: u32) -> u32 {
        // Floor division of the sign-extended value by 2^(count mod 32),
        // reduced modulo 2^32. Derived from the definition of an arithmetic
        // right shift; no host `>>` or production code is involved.
        i64::from(value)
            .div_euclid(1_i64 << (count % 32))
            .rem_euclid(1_i64 << 32) as u32
    }

    fn arithmetic_right_shift_words() -> (Vec<u32>, Vec<u32>) {
        let input: Vec<u32> = (0..64).chain([0x8000_0000, u32::MAX]).collect();
        let expected = input
            .iter()
            .map(|&count| arithmetic_shift_expected(ASHR_VALUE, count))
            .collect();
        (input, expected)
    }

    #[test]
    fn arithmetic_right_shift_preserves_sign_and_wraps_counts() {
        let (input, expected) = arithmetic_right_shift_words();
        assert_eq!(xenora_ref::execute(ASHR_FIXTURE, &input).unwrap(), expected);
        let compiled = compile(Architecture::Gfx1030, ASHR_FIXTURE).unwrap();
        assert!(compiled.ir.contains("ashr.i32 t1, t0"));
        assert!(
            compiled
                .msl
                .contains("as_type<uint>(as_type<int>(t1) >> (t0 & 31u))")
        );
    }

    #[test]
    fn arithmetic_right_shift_matches_explicit_checkpoints() {
        // Hand-derived from floor(-2147483647 / 2^(count mod 32)) mod 2^32.
        // Counts above 31 alias because the hardware masks the count.
        const CHECKPOINTS: &[(u32, u32)] = &[
            (0, 0x8000_0001),
            (1, 0xc000_0000),
            (2, 0xe000_0000),
            (3, 0xf000_0000),
            (4, 0xf800_0000),
            (7, 0xff00_0000),
            (8, 0xff80_0000),
            (15, 0xffff_0000),
            (16, 0xffff_8000),
            (23, 0xffff_ff00),
            (24, 0xffff_ff80),
            (30, 0xffff_fffe),
            (31, 0xffff_ffff),
            (32, 0x8000_0001),
            (33, 0xc000_0000),
            (63, 0xffff_ffff),
            (0x8000_0000, 0x8000_0001), // Masked to a count of zero.
            (u32::MAX, 0xffff_ffff),    // Masked to a count of 31.
        ];
        for &(count, expected) in CHECKPOINTS {
            assert_eq!(
                arithmetic_shift_expected(ASHR_VALUE, count),
                expected,
                "derivation for count {count:#x}"
            );
            let input = [count];
            assert_eq!(
                xenora_ref::execute(ASHR_FIXTURE, &input).unwrap(),
                [expected],
                "reference for count {count:#x}"
            );
            assert_eq!(
                independent_ref::execute(ASHR_FIXTURE, &input).unwrap(),
                [expected],
                "independent oracle for count {count:#x}"
            );
        }
        // A logical shift would have produced 0x4000_0000 for count one.
        assert_ne!(arithmetic_shift_expected(ASHR_VALUE, 1), 0x4000_0000);
    }

    #[test]
    fn arithmetic_right_shift_positive_control_and_renamed_destination() {
        let (input, _) = arithmetic_right_shift_words();
        let mut positive = *ASHR_FIXTURE;
        // Replace only the literal word at offset 0x10.
        positive[16..20].copy_from_slice(&0x7fff_ffff_u32.to_le_bytes());
        let expected: Vec<u32> = input
            .iter()
            .map(|&count| arithmetic_shift_expected(0x7fff_ffff, count))
            .collect();
        assert_eq!(expected[0], 0x7fff_ffff);
        assert_eq!(expected[1], 0x3fff_ffff);
        assert_eq!(expected[30], 1);
        assert_eq!(expected[31], 0);
        verify(&positive, &input, Some(&expected));

        // Keep the value source at v255 and move the result to v254 so the
        // destination no longer aliases the value input.
        let mut renamed = *ASHR_FIXTURE;
        renamed[20..24].copy_from_slice(&0x31fd_ff01_u32.to_le_bytes());
        renamed[29] = 0xfe; // buffer_store_dword v254
        let (_, expected) = arithmetic_right_shift_words();
        verify(&renamed, &input, Some(&expected));
    }

    #[test]
    fn arithmetic_right_shift_rejects_invalid_operands_and_incomplete_streams() {
        let (input, _) = arithmetic_right_shift_words();
        for instruction in [
            0x31ff_ff03_u32, // Uninitialized v3 as the count.
            0x31fe_0701,     // Uninitialized v3 as the value.
            0x3001_ff01,     // Write to synthetic index v0.
            0x31ff_ff80,     // Uninitialized VGPR v128 (nine-bit SRC0).
        ] {
            let mut code = *ASHR_FIXTURE;
            code[20..24].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &input, None);
        }
        for end in 0..ASHR_FIXTURE.len() {
            verify(&ASHR_FIXTURE[..end], &input, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn arithmetic_right_shift_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let (input, expected) = arithmetic_right_shift_words();
        let compiled = compile(Architecture::Gfx1030, ASHR_FIXTURE).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    fn wave32_masked_ashr() -> Vec<u8> {
        // Preserve the original input load, wait and v255 literal.
        let mut code = ASHR_FIXTURE[..20].to_vec();
        code.extend(words(&[
            0xbeff_03c1, // s_mov_b32 exec_hi, -1
            0xbeeb_03c1, // s_mov_b32 vcc_hi, -1
            0xbeea_0380, // s_mov_b32 vcc_lo, 0
            0xbf88_0001, // s_cbranch_execz: initial low mask is full
            0xbf82_0001, // s_branch past the incorrect-path overwrite
            0x7ffe_0287, // v_mov_b32 v255, 7
            0xbefe_0380, // s_mov_b32 exec_lo, 0
            0x7ffe_0287, // inactive v_mov_b32 leaves v255 unchanged
            0xbf88_0001, // s_cbranch_execz: low half zero despite high half
            0xbeea_0381, // setting vcc_lo on this wrong path changes the result
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
            0xbf86_0001, // s_cbranch_vccz: low half zero despite high half
            0x7ffe_0287, // v_mov_b32 v255, 7 if VCC_LO is nonzero
        ]));
        code.extend_from_slice(&ASHR_FIXTURE[20..]);
        code
    }

    #[test]
    fn wave32_exec_and_vcc_zero_checks_use_low_halves() {
        let input = [0, 1, 31, 32, 63];
        let mut code = wave32_masked_ashr();
        verify(
            &code,
            &input,
            Some(&[0x8000_0001, 0xc000_0000, u32::MAX, 0x8000_0001, u32::MAX]),
        );
        let compiled = compile(Architecture::Gfx1030, &code).unwrap();
        assert!(compiled.ir.contains("ashr.i32"));
        assert!(compiled.msl.contains(" >> (t0 & 31u)"));
        assert_eq!(compiled.ir.matches("taken=true").count(), 3);
        assert_eq!(compiled.ir.matches("taken=false").count(), 1);

        // Change only the VCC_LO initializer. The VCCZ branch falls through.
        code[28] = 0x81;
        verify(&code, &input, Some(&[7, 3, 0, 7, 0]));
    }

    #[test]
    fn wave32_rejects_invalid_branches_and_empty_exec_memory() {
        let code = wave32_masked_ashr();
        for (offset, encoding) in [
            (28, 0xbee9_0380u32), // Initialize s105 instead of VCC_LO.
            (52, 0xbf88_ffff),    // Backward EXECZ branch.
            (64, 0xbf86_0003),    // VCCZ branch into the second store word.
        ] {
            let mut rejected = code.clone();
            rejected[offset..offset + 4].copy_from_slice(&encoding.to_le_bytes());
            verify(&rejected, &[1], None);
        }
        // A reachable vector memory instruction with empty EXEC is unsupported.
        let mut rejected = code;
        rejected[48..52].copy_from_slice(&0xe030_2000_u32.to_le_bytes());
        rejected.splice(52..52, 0x8000_0100_u32.to_le_bytes());
        verify(&rejected, &[1], None);
    }

    fn partial_exec_ashr() -> Vec<u8> {
        // Original gfx1030 instructions checked with llvm-mc -show-encoding.
        // AMD's RDNA 2 ISA, section 6.2.2, predicates VGPR writes on EXEC.
        let mut code = ASHR_FIXTURE[..20].to_vec();
        code.extend(words(&[
            0x7e04_0287, // v_mov_b32 v2, 7
            0xbefe_03ff,
            0x8000_0001, // s_mov_b32 exec_lo, 0x80000001
            0xbf88_0001, // EXECZ would skip the move if the mask were empty
            0x7ffe_0302, // v_mov_b32 v255, v2
            0xd525_00ff,
            0x0202_03ff, // v_add_nc_u32_e64 v255, v255, v1
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
        ]));
        code.extend_from_slice(&ASHR_FIXTURE[20..]);
        code
    }

    fn partial_exec_expected() -> [u32; 67] {
        let mut expected = [0xc000_0000; 67];
        for index in [0, 31, 32, 63, 64] {
            expected[index] = 4; // (7 + 1) arithmetic-shifted right one bit.
        }
        expected
    }

    #[test]
    fn partial_wave32_updates_only_the_selected_lanes() {
        verify(
            &partial_exec_ashr(),
            &[1; 67],
            Some(&partial_exec_expected()),
        );
    }

    #[test]
    fn partial_wave32_rejects_vector_memory_and_uninitialized_destinations() {
        let mut uninitialized = partial_exec_ashr();
        // Replace the masked move's destination with uninitialized v254.
        uninitialized[36..40].copy_from_slice(&0x7ffc_0302_u32.to_le_bytes());
        verify(&uninitialized, &[1; 33], None);

        let mut masked_memory = partial_exec_ashr();
        // Leave the partial mask in place for the final buffer store.
        masked_memory.drain(48..52);
        verify(&masked_memory, &[1; 33], None);
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn partial_wave32_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, &partial_exec_ashr()).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &[1; 67])
            .unwrap();
        assert_eq!(actual, partial_exec_expected());
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn wave32_masked_ashr_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let (input, expected) = arithmetic_right_shift_words();
        let compiled = compile(Architecture::Gfx1030, &wave32_masked_ashr()).unwrap();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    const SHIFT_FIXTURE: &[u8; 56] = include_bytes!("../../fixtures/rdna2/shifts.bin");
    const SHIFT_INPUT: [u32; 12] = [
        0,
        1,
        2,
        30,
        31,
        32,
        33,
        63,
        64,
        0x8000_0000,
        0x8000_0021,
        u32::MAX,
    ];
    const SHIFT_EXPECTED: [u32; 12] = [
        2,
        0x4000_0006,
        0x2000_000c,
        0xc000_0002,
        0x8000_0001,
        2,
        0x4000_0006,
        0x8000_0001,
        2,
        2,
        0x4000_0006,
        0x8000_0001,
    ];

    #[test]
    fn vector_shifts_match_known_answers_with_aliased_operands() {
        verify(SHIFT_FIXTURE, &SHIFT_INPUT, Some(&SHIFT_EXPECTED));
    }

    #[test]
    fn legacy_fixture_inline_operands_now_have_known_results() {
        let mut code = *OR_FIXTURE;
        code[20..24].copy_from_slice(&0x3803_fe80_u32.to_le_bytes());
        verify(&code, &OR_INPUT, Some(&[0x8055_00aa; OR_INPUT.len()]));
        let mut code = *SHIFT_FIXTURE;
        code[32..36].copy_from_slice(&0x3402_0480_u32.to_le_bytes());
        // LSHL count0 retains80000003; XOR with the preceding logical shift.
        let expected = [
            2, 0xc0000003, 0xa0000003, 0x80000001, 0x80000002, 2, 0xc0000003, 0x80000002, 2, 2,
            0xc0000003, 0x80000002,
        ];
        verify(&code, &SHIFT_INPUT, Some(&expected));
    }

    #[test]
    fn vector_shifts_reject_invalid_operands_and_incomplete_streams() {
        for (offset, instruction) in [
            (28, 0x2dff_ff03_u32), // Uninitialized count.
            (28, 0x2dfe_0701),     // Uninitialized value.
            (28, 0x2c01_ff01),     // Write to synthetic index.
            (28, 0x2dff_fe08),     // Uninitialized SGPR count.
            (32, 0x3402_0503),     // Uninitialized count.
            (32, 0x3402_0701),     // Uninitialized value.
            (32, 0x3400_0501),     // Write to synthetic index.
            (32, 0x3402_046b),     // Reserved count selector107.
        ] {
            let mut code = *SHIFT_FIXTURE;
            code[offset..offset + 4].copy_from_slice(&instruction.to_le_bytes());
            verify(&code, &SHIFT_INPUT, None);
        }
        for end in 28..SHIFT_FIXTURE.len() {
            verify(&SHIFT_FIXTURE[..end], &SHIFT_INPUT, None);
        }
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn vector_shifts_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let compiled = compile(Architecture::Gfx1030, SHIFT_FIXTURE).unwrap();
        let input: Vec<_> = SHIFT_INPUT.into_iter().cycle().take(131).collect();
        let expected: Vec<_> = SHIFT_EXPECTED
            .into_iter()
            .cycle()
            .take(input.len())
            .collect();
        let actual = xenora_metal::NativeCompute::new()
            .unwrap()
            .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &input)
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn inline_float_selectors_are_exact_words_in_each_reference_reader() {
        // Independent literal answers: selector, raw bits, bitwise NOT bits.
        // Neither reference's mapping nor production semantics supplies this table.
        let cases: &[(u32, u32, u32)] = &[
            (240, 0x3f000000, 0xc0ffffff),
            (241, 0xbf000000, 0x40ffffff),
            (242, 0x3f800000, 0xc07fffff),
            (243, 0xbf800000, 0x407fffff),
            (244, 0x40000000, 0xbfffffff),
            (245, 0xc0000000, 0x3fffffff),
            (246, 0x40800000, 0xbf7fffff),
            (247, 0xc0800000, 0x3f7fffff),
            (248, 0x3e22f983, 0xc1dd067c),
        ];
        let check = |body: &[u32], expected: u32| {
            let mut program = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
            program.extend_from_slice(body);
            program.extend([0xe0702000, 0x80010200, 0xbbfd0000, 0xbf810000]);
            verify(&words(&program), &[0], Some(&[expected]));
        };
        for &(selector, bits, inverted) in cases {
            // Four-byte V_MOV and S_MOV, followed immediately by real instructions.
            check(&[0x7e040200 | selector], bits);
            check(&[0xbe880300 | selector, 0x7e040208], bits);
            // SOP1 NOT sets SCC; its literal result tests all 32 operand bits.
            check(&[0xbe880700 | selector, 0x7e040208], inverted);
            // SOP2 ADD with integer zero, both source slots, then two inline
            // XOR operands. Inline+literal must consume exactly one extra word.
            check(&[0x80088000 | selector, 0x7e040208], bits);
            check(&[0x80080080 | selector << 8, 0x7e040208], bits);
            check(&[0x89080000 | selector << 8 | selector, 0x7e040208], 0);
            check(&[0x8008ff00 | selector, 0, 0x7e040208], bits);
            check(&[0x800800ff | selector << 8, 0, 0x7e040208], bits);
            // SOPC EQ_U32: both inline operands and inline+literal in either
            // position. Branch observation distinguishes true from false.
            for operands in [
                vec![0xbf060000 | selector << 8 | selector],
                vec![0xbf06ff00 | selector, bits],
                vec![0xbf0600ff | selector << 8, bits],
            ] {
                let mut body = vec![0x7e040280];
                body.extend(operands);
                body.extend([0xbf840001, 0x7e040281]);
                check(&body, 1);
            }
            // Integer VOP2 ADD_NC sees the raw word, with v1 loaded as zero.
            check(&[0x4a040200 | selector], bits);
            // Integer VOPC EQ_U32 produces one active bit in this one-lane wave.
            check(&[0x7e0602ff, bits, 0x7d840600 | selector, 0x7e04026a], 1);
            check(
                &[0x7e0602ff, inverted, 0x7d840600 | selector, 0x7e04026a],
                0,
            );
            // Nine-bit classification must keep v240..v248 distinct from inline
            // codes with the same low byte. Every selected VGPR contains 7.
            check(&[0x7e000280 | selector << 17 | 7, 0x7e040300 | selector], 7);
        }
        // Integer inline one and FP inline one are different raw words.
        check(&[0x7e040281], 1);
        check(&[0x7e0402f2], 0x3f800000);
    }

    fn words(code: &[u32]) -> Vec<u8> {
        code.iter().copied().flat_map(u32::to_le_bytes).collect()
    }

    fn verify(code: &[u8], input: &[u32], expected: Option<&[u32]>) {
        let compiled = compile(Architecture::Gfx1030, code);
        let reference = xenora_ref::execute(code, input);
        let independent = independent_ref::execute(code, input);
        assert_eq!(compiled.is_ok(), expected.is_some(), "{compiled:?}");
        assert_eq!(reference.as_deref().ok(), expected, "{reference:?}");
        assert_eq!(independent.as_deref().ok(), expected, "{independent:?}");
    }

    fn check(input: &[u32]) {
        let expected: Vec<_> = input
            .iter()
            .map(|&x| ((u64::from(x) + 0x1234_5678) % (1_u64 << 32)) as u32)
            .collect();
        assert_eq!(xenora_ref::execute(FIXTURE, input).unwrap(), expected);
    }

    #[test]
    fn independently_derived_addition_for_boundary_and_pseudorandom_inputs() {
        let wrap_at = 0xEDCB_A988;
        for base in [0, wrap_at - 0x8000, 0x7fff_8000, 0xffff_0000] {
            let input: Vec<_> = (base..=base + u32::from(u16::MAX)).collect();
            for chunk in input.chunks(4096) {
                check(chunk);
            }
        }
        let mut seed = 0xbfac_128d_u32;
        let input: Vec<_> = (0..4096)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            })
            .collect();
        check(&input);
    }

    #[test]
    fn compiler_and_reference_agree_on_acceptance_of_each_single_bit_mutation() {
        for bit in 0..FIXTURE.len() * 8 {
            let mut code = FIXTURE.to_vec();
            code[bit / 8] ^= 1 << (bit % 8);
            assert_eq!(
                compile(Architecture::Gfx1030, &code).is_ok(),
                xenora_ref::execute(&code, &[42]).is_ok(),
                "acceptance differed at byte {}, bit {}",
                bit / 8,
                bit % 8,
            );
        }
    }

    #[test]
    fn scalar_compare_selects_both_forward_edges() {
        let fixture = include_bytes!("../../fixtures/rdna2/bitwise-branch.bin");
        let input = [0, 1, 0x00ff_00ff, 0xff00_ff00, u32::MAX, 0xedcb_a988];
        let expected = [
            0x1234_5678,
            0x1234_5679,
            0x12cb_5687,
            0x1234_5678,
            0x12cb_5687,
            0x12ff_56f0,
        ];
        assert_eq!(xenora_ref::execute(fixture, &input).unwrap(), expected);
        let taken = compile(Architecture::Gfx1030, fixture).unwrap();
        assert!(taken.msl.contains(" & "));
        assert!(taken.msl.contains(" ^ "));

        let mut equal = *fixture;
        assert_eq!(equal[17], 0x86); // Inline unsigned 6 in the SCC comparison.
        equal[17] = 0x87; // Compare 7 with 7 and fall through to v2 = 0.
        assert_eq!(xenora_ref::execute(&equal, &input).unwrap(), [0; 6]);
        let fallthrough = compile(Architecture::Gfx1030, &equal).unwrap();
        assert!(!fallthrough.msl.contains(" & "));
        assert!(!fallthrough.msl.contains(" ^ "));

        let mut into_literal = *fixture;
        into_literal[20] = 3; // PC + 4 + 3 * 4 = 36, the literal of the instruction at 32.
        assert!(compile(Architecture::Gfx1030, &into_literal).is_err());
        assert!(xenora_ref::execute(&into_literal, &input).is_err());
    }

    #[test]
    fn float_results_and_rejections_follow_the_exact_domain() {
        let fixture = include_bytes!("../../fixtures/rdna2/float-exact.bin");
        let input = [
            0,
            0x003f_ffff,
            0x0040_0000,
            0x0040_0001,
            0x007f_ffff,
            0x7f80_0000,
            0x7fc0_0001,
            u32::MAX,
        ];
        // Hand-derived: (1 + 5/2) * 5/2 = 35/4; (3/2 + 5/2) * 5/2 = 10.
        let expected = [
            0x410c_0000,
            0x410c_0000,
            0x4120_0000,
            0x4120_0000,
            0x4120_0000,
            0x410c_0000,
            0x4120_0000,
            0x4120_0000,
        ];
        assert_eq!(xenora_ref::execute(fixture, &input).unwrap(), expected);
        assert!(compile(Architecture::Gfx1030, fixture).is_ok());

        for rejected_literal in [1_u32, 0x3380_0000, 0x0080_0000, 0xff80_0000] {
            // Subnormal, midpoint rounding at 1, binary64 absorption, infinity.
            let mut changed = *fixture;
            changed[36..40].copy_from_slice(&rejected_literal.to_le_bytes());
            assert!(
                compile(Architecture::Gfx1030, &changed).is_err(),
                "{rejected_literal:08x}"
            );
            assert!(
                xenora_ref::execute(&changed, &[0]).is_err(),
                "{rejected_literal:08x}"
            );
        }
    }

    #[test]
    fn second_interpreter_agrees_with_reference_on_boundary_and_random_inputs() {
        let fixtures: [(&[u8], &str); 4] = [
            (include_bytes!("../../fixtures/rdna2/add.bin"), "add"),
            (
                include_bytes!("../../fixtures/rdna2/bitwise-branch.bin"),
                "bitwise-branch",
            ),
            (
                include_bytes!("../../fixtures/rdna2/float-exact.bin"),
                "float-exact",
            ),
            (include_bytes!("../../fixtures/rdna2/ashr.bin"), "ashr"),
        ];
        let mut inputs: Vec<u32> = vec![
            0,
            1,
            u32::MAX,
            0x8000_0000,
            0x7fff_ffff,
            0xedcb_a988,
            0x0040_0000,
            0x0040_0001,
            0x007f_ffff,
            0x0080_0000,
            0x00ff_00ff,
            0xff00_ff00,
        ];
        for base in [
            0xedcb_a988u32.wrapping_sub(64),
            0x0040_0000u32.wrapping_sub(64),
        ] {
            inputs.extend((0..128).map(|i| base.wrapping_add(i)));
        }
        let mut seed = 0x9e37_79b9u32;
        inputs.extend((0..3000).map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        }));
        for (code, name) in fixtures {
            let expected = xenora_ref::execute(code, &inputs).unwrap();
            let actual = independent_ref::execute(code, &inputs).unwrap();
            assert_eq!(actual, expected, "fixture {name}");
        }
    }

    #[test]
    fn binary32_boundaries_follow_exact_significand_arithmetic() {
        const ADD: u32 = 0x0602_0702;
        const MUL: u32 = 0x1002_0702;
        let input = [0, 0x0040_0000, u32::MAX];
        let one = 0x3f80_0000;
        let min_normal = 0x0080_0000;
        let max_normal = 0x7f7f_ffff;
        for (op, a, b, expected) in [
            (ADD, one, 0x3400_0000, Some(0x3f80_0001)), // 1 + 2^-23
            (ADD, 0x3f7f_ffff, 0x3380_0000, Some(one)), // (1 - 2^-24) + 2^-24
            (ADD, min_normal, min_normal, Some(0x0100_0000)), // 2^-125
            (ADD, one, 0x3380_0000, None),              // Midpoint 1 + 2^-24
            (ADD, one, min_normal, None), // Absorbed in binary64 if computed in host FP
            (ADD, max_normal, max_normal, None), // Overflow
            (MUL, 0x3fc0_0000, 0x3fc0_0000, Some(0x4010_0000)), // (3/2)^2 = 9/4
            (MUL, 0x7e80_0000, min_normal, Some(one)), // 2^126 × 2^-126
            (MUL, max_normal, one, Some(max_normal)),
            (MUL, 0x4580_0800, 0x4580_0800, None), // 4097² = 16,785,409, an odd integer
            (MUL, min_normal, 0x3f00_0000, None),  // 2^-127 is subnormal
            (MUL, max_normal, 0x4000_0000, None),  // Overflow
            (ADD, 0, one, None),                   // Positive zero
            (ADD, one, 0x8000_0000, None),         // Negative zero
            (ADD, 0xbf80_0000, 0x4000_0000, None), // Result normal, operand negative
            (ADD, 0x4000_0000, 0xbf80_0000, None), // Same result, other operand negative
            (ADD, one, 1, None),                   // Smallest positive subnormal
            (MUL, 1, 0x7e80_0000, None), // Subnormal operand would yield exact normal 2^-23
            (MUL, 0x7e80_0000, 1, None),
            (ADD, one, 0x7f80_0000, None), // Infinity
            (MUL, 0xff80_0000, one, None), // Negative infinity
            (ADD, one, 0x7fc0_0001, None), // Quiet NaN with payload
            (MUL, 0x7f80_0001, one, None), // Signaling NaN with payload
        ] {
            // Force literal moves into v2 and v3. VOP2 writes v1, the existing
            // store source. The exact words were cross-checked with gfx1030 llvm-mc.
            let mut code = words(&[0x7e04_02ff, a, 0x7e06_02ff, b, op]);
            code.extend_from_slice(&FIXTURE[28..]);
            let expected = expected.map(|word| [word; 3]);
            verify(
                &code,
                &input,
                expected.as_ref().map(|values| values.as_slice()),
            );
        }
    }

    #[test]
    fn vector_register_renaming_and_in_place_add_preserve_known_answers() {
        let input = [0, 1, 0xedcb_a988, 0x8000_0000, u32::MAX];
        let expected = [0x1234_5678, 0x1234_5679, 0, 0x9234_5678, 0x1234_5677];
        let renamed = words(&[
            0xe030_2000,
            0x8000_ff00, // Load v255 instead of v1.
            0xbf8c_3f70,
            0x7f00_02ff,
            0x1234_5678, // Move the literal to v128.
            0xd525_00ff,
            0x0203_01ff, // Add v255, v255, v128.
            0x7ffc_03ff, // Copy v255 to v254 before storing.
            0xe070_2000,
            0x8001_fe00, // Store v254.
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(FIXTURE, &input, Some(&expected));
        verify(&renamed, &input, Some(&expected));
    }

    #[test]
    fn scalar_register_copy_and_literal_comparison_select_both_edges() {
        let input = [0, u32::MAX];
        for (literal, expected) in [(0xffff_fff0, [1, 1]), (0xffff_fff1, [2, 2])] {
            let mut code = words(&[
                0xbee9_03d0, // s_mov_b32 s105, -16
                0xbe89_0369, // s_mov_b32 s9, s105
                0xbf06_ff09,
                literal,     // s_cmp_eq_u32 s9, lit(<word>)
                0xbf84_0002, // Skip the next move and branch if unequal.
                0x7e02_0281, // v_mov_b32_e32 v1, 1
                0xbf82_0001, // Skip the unequal case's move.
                0x7e02_0282, // v_mov_b32_e32 v1, 2
            ]);
            code.extend_from_slice(&FIXTURE[28..]);
            verify(&code, &input, Some(&expected));
            for end in 0..code.len() {
                verify(&code[..end], &input, None);
            }
        }
    }

    #[test]
    fn skipped_code_still_has_to_obey_the_fixture_layout_and_bindings() {
        for bad_prefix in [
            &[0xbf82_0001u32, 0xbf81_0000][..],       // Early termination.
            &[0xbf82_0001, 0xbeec_0380],              // Privileged TTMP0 destination.
            &[0xbf82_0002, 0xe030_3000, 0x8000_0100], // Unsupported OFFEN with IDXEN.
            &[0xbf82_0002, 0xe030_2000, 0x8000_0101], // v1 instead of v0 as index.
            &[0xbf82_0002, 0xe030_2000, 0x8001_0100], // Load the wrong buffer.
            &[0xbf82_0002, 0xe070_2000, 0x8000_0100], // Store to the input buffer.
        ] {
            let mut code = words(bad_prefix);
            code.extend_from_slice(FIXTURE);
            verify(&code, &[1], None);
        }
        // A well-formed but unreachable move need not have an initialized source.
        let mut skipped_move = words(&[0xbf82_0001, 0x7e02_03ff]);
        skipped_move.extend_from_slice(FIXTURE);
        verify(&skipped_move, &[1], Some(&[0x1234_5679]));
    }

    #[test]
    fn reachable_vector_writes_respect_pending_loads_and_the_index_register() {
        let mut pending = FIXTURE.to_vec();
        pending.splice(8..8, 0x7e02_0281_u32.to_le_bytes()); // Move 1 to pending v1.
        verify(&pending, &[1], None);
        let mut overwrites_index = FIXTURE.to_vec();
        overwrites_index[20] = 0; // Change the VOP3 add's destination from v1 to v0.
        verify(&overwrites_index, &[1], None);
    }

    #[test]
    fn rejects_overlapping_reads_even_if_the_later_read_would_supply_the_output() {
        let code = words(&[
            0xe030_2000,
            0x8000_0100, // Load into v1 without waiting.
            0xe030_2000,
            0x8000_0200, // Load into v2 while v1 is pending.
            0xbf8c_3f70,
            0xe070_2000,
            0x8001_0200, // Store v2, leaving v1 unused.
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(&code, &[0x8765_4321], None);
    }

    #[test]
    fn rejects_a_second_store_and_requires_waiting_for_a_following_load() {
        for (at, extra) in [
            (36, &FIXTURE[28..36]), // Second store before the original store wait.
            (40, &FIXTURE[28..40]), // Second store and wait after a completed output.
            (40, &FIXTURE[..8]),    // Load after the completed output, without a load wait.
        ] {
            let mut code = FIXTURE.to_vec();
            code.splice(at..at, extra.iter().copied());
            verify(&code, &[1], None);
        }
    }

    #[test]
    fn distinguishes_descriptor_and_ordinary_scalar_registers() {
        for (mov, expected) in [
            (0xbe87_0381, None), // s_mov_b32 s7, 1 writes a reserved descriptor.
            (0xbe88_0381, Some([0x1234_5679])), // s8 sits at the boundary.
            (0xbe89_0381, Some([0x1234_5679])), // s9 is an ordinary scalar.
        ] {
            let mut code = words(&[mov]);
            code.extend_from_slice(FIXTURE);
            verify(&code, &[1], expected.as_ref().map(|words| words.as_slice()));
        }
    }

    const SCALAR_ALU_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-alu.bin");
    const SCALAR_LOGIC_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-logic.bin");
    // Hand-derived in fixtures/rdna2/scalar-logic.s from the guide's 12.1 pseudocode.
    const SCALAR_LOGIC_KEY: u32 = 0x887f_00f8;
    const SCALAR_COMPARE_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-compare.bin");
    // Hand-derived in fixtures/rdna2/scalar-compare.s from the guide's 12.4 pseudocode.
    const SCALAR_COMPARE_KEY: u32 = 0x0000_0d5b;
    const SCALAR_UNARY_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-unary.bin");
    // Hand-derived in fixtures/rdna2/scalar-unary.s from the guide's 12.3 pseudocode.
    const SCALAR_UNARY_KEY: u32 = 0x0f00_80f5;
    const SCALAR_ARITH_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/scalar-arith.bin");
    // Hand-derived in fixtures/rdna2/scalar-arith.s from the guide's 12.1 pseudocode.
    const SCALAR_ARITH_KEY: u32 = 0x8115_7dfe;
    const VECTOR_INTEGER_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/vector-integer.bin");

    /// fixtures/rdna2/vector-integer.s for one lane input `x`, written from the
    /// guide's 12.7 pseudocode: exact products of the 24-bit fields (sign-
    /// extended for the I24 forms), D = the low or high 32 bits.
    fn vector_integer_key(x: u32) -> u32 {
        let i24 = |v: u32| i64::from(((v << 8) as i32) >> 8);
        let u24 = |v: u32| i64::from(v & 0x00ff_ffff);
        let low = |product: i64| product as u32;
        let high = |product: i64| (product >> 32) as u32;
        let s8 = 0x0080_0001_u32;
        let v2 = low(i24(x) * -3);
        let v3 = high(i24(s8) * i24(x));
        let v4 = low(0x0012_3456 * u24(x));
        let v5 = high(u24(s8) * u24(x));
        let v6 = (x as i32).min(v2 as i32) as u32;
        let v7 = 5_i32.max(x as i32) as u32;
        let v8 = v3.min(x);
        let v9 = s8.max(x);
        let v10 = !(v4 ^ v5);
        let v11 = v6.wrapping_add(v7);
        let v12 = v8.wrapping_sub(v9);
        let v13 = v11.wrapping_sub(v10);
        v2 ^ v3 ^ v4 ^ v5 ^ v6 ^ v7 ^ v8 ^ v9 ^ v10 ^ v11 ^ v12 ^ v13
    }

    /// LLVM-assembled: v_cmp_ne_u32_e32 vcc_lo, 0, v1 then
    /// v_add_nc_u32_e32 v2, vcc_lo, v1, with the usual load and store.
    const VCC_LANE_ADD: [u32; 9] = [
        0xe030_2000,
        0x8000_0100,
        0xbf8c_3f70,
        0x7d8a_0280,
        0x4a04_026a,
        0xe070_2000,
        0x8001_0200,
        0xbbfd_0000,
        0xbf81_0000,
    ];

    /// Lane inputs covering both 24-bit signs, the bits above 23, zero and the
    /// signed and unsigned extremes.
    const VECTOR_INTEGER_INPUT: [u32; 12] = [
        0,
        1,
        5,
        0x007f_ffff,
        0x0080_0000,
        0x00ff_ffff,
        0x0123_4567,
        0x7fff_ffff,
        0x8000_0000,
        0x89ab_cdef,
        0xff80_0001,
        u32::MAX,
    ];
    // SOP2 opcodes of the supported subset (RDNA 2 XML opcode table).
    const SOP2_OPCODES: [u32; 9] = [0, 1, 2, 3, 4, 5, 14, 16, 18];
    const SCALAR_VALUES: [u32; 10] = [
        0,
        1,
        2,
        0x7fff_ffff,
        0x8000_0000,
        0x8000_0001,
        0xffff_fffe,
        u32::MAX,
        0x1234_5678,
        0xedcb_a988,
    ];

    /// ENCODING 0b10 in bits 30-31, OP 23-29, SDST 16-22, SSRC1 8-15, SSRC0 0-7.
    fn sop2(opcode: u32, dst: u32, src0: u32, src1: u32) -> u32 {
        0x8000_0000 | opcode << 23 | dst << 16 | src1 << 8 | src0
    }

    /// D and SCC from the operation's definition, with exact integers: additions
    /// carry out of bit 31, subtractions borrow, bitwise results set SCC iff nonzero.
    fn sop2_expected(opcode: u32, a: u32, b: u32, carry: bool) -> (u32, bool) {
        let (a, b) = (u64::from(a), u64::from(b));
        let c = u64::from(carry && matches!(opcode, 4 | 5));
        let (exact, scc) = match opcode {
            0 | 4 => (a + b + c, a + b + c >= 1 << 32),
            1 | 5 => (((1 << 32) + a - b - c) % (1 << 32), b + c > a),
            // Signed: the exact result of the sign-extended operands must fit in
            // [-2^31, 2^31), else SCC = overflow (RDNA 2 ISA guide 5.3 and 12.1).
            2 | 3 => {
                let signed = |value: u64| i64::from(value as u32 as i32);
                let exact = if opcode == 2 {
                    signed(a) + signed(b)
                } else {
                    signed(a) - signed(b)
                };
                let overflow = !(-(1_i64 << 31)..(1_i64 << 31)).contains(&exact);
                (exact.rem_euclid(1 << 32) as u64, overflow)
            }
            14 => (a & b, a & b != 0),
            16 => (a | b, a | b != 0),
            _ => (a ^ b, a ^ b != 0),
        };
        ((exact % (1 << 32)) as u32, scc)
    }

    /// s8 = a, s9 = b, SCC = `carry` (if set), then `alu` (with an optional literal),
    /// then v2 = s<result>; lanes are XORed with their input only when SCC is set.
    fn sop2_program(alu: &[u32], result: u32, a: u32, b: u32, carry: Option<bool>) -> Vec<u8> {
        let mut words = vec![0xe030_2000, 0x8000_0100, 0xbf8c_3f70];
        words.extend([0xbe88_03ff, a, 0xbe89_03ff, b]);
        if let Some(carry) = carry {
            // s_cmp_eq_u32 0, 0 sets SCC; s_cmp_eq_u32 0, 1 clears it.
            words.push(if carry { 0xbf06_8080 } else { 0xbf06_8180 });
        }
        words.extend(alu);
        words.extend([0x7e04_0200 | result, 0xbf84_0001, 0x3a04_0501]);
        words.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }

    fn lanes(value: u32, scc: bool) -> [u32; 2] {
        [value, if scc { !value } else { value }]
    }

    /// Every supported opcode over boundary operands, with SCC both clear and set
    /// before the carry-consuming forms.
    fn sop2_vectors() -> Vec<(Vec<u8>, [u32; 2])> {
        let mut vectors = Vec::new();
        for opcode in SOP2_OPCODES {
            for a in SCALAR_VALUES {
                for b in SCALAR_VALUES {
                    for carry in [false, true] {
                        let (value, scc) = sop2_expected(opcode, a, b, carry);
                        let code = sop2_program(&[sop2(opcode, 10, 8, 9)], 10, a, b, Some(carry));
                        vectors.push((code, lanes(value, scc)));
                    }
                }
            }
        }
        vectors
    }

    #[test]
    fn scalar_alu_fixture_chains_carry_borrow_and_bitwise_scc() {
        let input = [0, 1, u32::MAX, 0x0000_000e, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ 0x0000_000e).collect();
        verify(SCALAR_ALU_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_ALU_FIXTURE).unwrap();
        // The scalar results reach the kernel as constants; the branch is resolved.
        assert!(compiled.disassembly.contains("s_subb_u32 s11, s9, s8"));
        assert!(compiled.ir.contains("const.u32"), "{}", compiled.ir);
        // Clearing the final AND's SCC-zero edge (s15 = s14 & 15 = 0) is observable:
        // with 31 instead of 15 the result is 0x10, SCC = 1, and v2 = 0.
        let mut nonzero = SCALAR_ALU_FIXTURE.to_vec();
        assert_eq!(nonzero[0x39], 0x8f);
        nonzero[0x39] = 0x9f;
        verify(&nonzero, &input, Some(&[0; 5]));
    }

    #[test]
    fn scalar_logic_fixture_chains_registry_logical_and_shift_forms() {
        let input = [0, 1, u32::MAX, SCALAR_LOGIC_KEY, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ SCALAR_LOGIC_KEY).collect();
        verify(SCALAR_LOGIC_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_LOGIC_FIXTURE).unwrap();
        assert!(compiled.disassembly.contains("s_ashr_i32 s17, s15, 31"));
        // The SCC-zero edge is observable: with a shift count of 30 instead of
        // 31, s17 = 0x787ffff8 >> 30 = 1, SCC = 1, and v2 = 0.
        let mut nonzero = SCALAR_LOGIC_FIXTURE.to_vec();
        let at = nonzero
            .windows(4)
            .position(|word| word == [0x0f, 0x9f, 0x11, 0x91])
            .unwrap();
        nonzero[at + 1] = 0x9e;
        verify(&nonzero, &input, Some(&[0; 5]));
    }

    #[test]
    fn scalar_compare_fixture_records_every_registry_compare() {
        let input = [0, 1, u32::MAX, SCALAR_COMPARE_KEY, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ SCALAR_COMPARE_KEY).collect();
        verify(SCALAR_COMPARE_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_COMPARE_FIXTURE).unwrap();
        assert!(compiled.disassembly.contains("s_cmp_le_u32 s9, 64"));
    }

    #[test]
    fn scalar_unary_fixture_chains_registry_unary_forms() {
        let input = [0, 1, u32::MAX, SCALAR_UNARY_KEY, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ SCALAR_UNARY_KEY).collect();
        verify(SCALAR_UNARY_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_UNARY_FIXTURE).unwrap();
        assert!(compiled.disassembly.contains("s_abs_i32 s20, 0"));
        // SCC stays clear across the SEXT and FF1 that follow s_abs_i32 s20, 0:
        // with s_abs_i32 s20, 1 instead, SCC = 1 and v2 = 0.
        let mut set = SCALAR_UNARY_FIXTURE.to_vec();
        let at = set
            .windows(4)
            .position(|word| word == [0x80, 0x34, 0x94, 0xbe])
            .unwrap();
        set[at] = 0x81;
        verify(&set, &input, Some(&[0; 5]));
    }

    #[test]
    fn vector_integer_fixture_computes_each_lane_from_its_input() {
        let expected: Vec<_> = VECTOR_INTEGER_INPUT
            .iter()
            .map(|&x| vector_integer_key(x))
            .collect();
        verify(
            VECTOR_INTEGER_FIXTURE,
            &VECTOR_INTEGER_INPUT,
            Some(&expected),
        );
        // The results stay runtime per-lane values: the only constants are the
        // six uniform SRC0 operands (-3, s8 three times, the literal and 5), and
        // the twelve lane inputs give twelve different results.
        let compiled = compile(Architecture::Gfx1030, VECTOR_INTEGER_FIXTURE).unwrap();
        assert!(compiled.ir.contains("mul24.i32 t1, t0"), "{}", compiled.ir);
        assert!(compiled.msl.contains("mulhi("), "{}", compiled.msl);
        assert_eq!(
            compiled.ir.matches(" = const.u32 ").count(),
            6,
            "{}",
            compiled.ir
        );
        let distinct: std::collections::BTreeSet<_> = expected.iter().collect();
        assert_eq!(distinct.len(), expected.len());
    }

    #[test]
    fn vector_integer_reads_a_lane_derived_vcc_lo() {
        // LLVM: v_cmp_ne_u32_e32 vcc_lo, 0, v1; v_add_nc_u32_e32 v2, vcc_lo, v1.
        // Lane 0 (x = 0) fails the compare and lane 1 (x = 7) passes, so the
        // mask is 0b10 and every lane adds 2.
        let code = words(&VCC_LANE_ADD);
        verify(&code, &[0, 7], Some(&[2, 9]));
        verify(&code, &[7, 7], Some(&[10, 10]));
    }

    #[test]
    fn vector_integer_is_refused_in_a_runtime_vccz_skip_region() {
        // LLVM: v_mov_b32 v2, 0; v_cmp_ne_u32 vcc_lo, 0, v1; s_cbranch_vccz
        // over v_add_nc_u32_e32 v2, vcc_lo, v1 (at 0x18), whatever the mask.
        let code = words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_0280,
            0x7d8a_0280,
            0xbf86_0001,
            0x4a04_026a,
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        rejected_at_with(&code, 0x18, &[0, 7]);
        rejected_at_with(&code, 0x18, &[7, 7]);
    }

    #[test]
    fn scalar_arith_fixture_chains_registry_arithmetic_forms() {
        let input = [0, 1, u32::MAX, SCALAR_ARITH_KEY, 0x1234_5678];
        let expected: Vec<_> = input.iter().map(|word| word ^ SCALAR_ARITH_KEY).collect();
        verify(SCALAR_ARITH_FIXTURE, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, SCALAR_ARITH_FIXTURE).unwrap();
        assert!(compiled.disassembly.contains("s_lshl2_add_u32 s25, s9, s9"));
        // SCC stays clear across the CSELECT and MUL_HI that follow
        // s_lshl2_add_u32 s25, s9, s9: with s_lshl2_add_u32 s25, s9, -1 instead,
        // 20 + 0xffffffff carries, SCC = 1 and v2 = 0.
        let mut set = SCALAR_ARITH_FIXTURE.to_vec();
        let at = set
            .windows(4)
            .position(|word| word == [0x09, 0x09, 0x99, 0x97])
            .unwrap();
        set[at + 1] = 0xc1;
        verify(&set, &input, Some(&[0; 5]));
    }

    /// The translator's own folding: v2 = D as a constant, and the SCC-zero edge
    /// is taken exactly when SCC is clear.
    fn assert_translated(code: &[u8], expected: &[u32; 2]) {
        let ir = compile(Architecture::Gfx1030, code).unwrap().ir;
        assert!(
            ir.contains(&format!("const.u32 0x{:08x}", expected[0])),
            "{ir}"
        );
        let taken = expected[0] == expected[1];
        assert!(ir.contains(&format!("taken={taken}")), "{ir}");
    }

    /// Aliased destinations, literals in either source and inline integers.
    fn sop2_operand_cases() -> Vec<(Vec<u8>, [u32; 2])> {
        let (a, b) = (0xffff_fff0, 0x0000_0011);
        let mut cases = Vec::new();
        for opcode in SOP2_OPCODES {
            for (dst, src0, src1) in [(8, 8, 9), (9, 8, 9), (8, 8, 8), (9, 9, 9)] {
                let x = if src0 == 9 { b } else { a };
                let y = if src1 == 8 { a } else { b };
                let (value, scc) = sop2_expected(opcode, x, y, true);
                let code = sop2_program(&[sop2(opcode, dst, src0, src1)], dst, a, b, Some(true));
                cases.push((code, lanes(value, scc)));
            }
            let (value, scc) = sop2_expected(opcode, a, b, true);
            cases.push((
                sop2_program(&[sop2(opcode, 10, 8, 255), b], 10, a, 7, Some(true)),
                lanes(value, scc),
            ));
            let (value, scc) = sop2_expected(opcode, b, a, true);
            cases.push((
                sop2_program(&[sop2(opcode, 10, 255, 8), b], 10, a, 7, Some(true)),
                lanes(value, scc),
            ));
            for (selector, inline) in [(0x80, 0), (0xc0, 64), (0xc1, u32::MAX), (0xd0, 0xffff_fff0)]
            {
                let (value, scc) = sop2_expected(opcode, a, inline, true);
                let code = sop2_program(&[sop2(opcode, 10, 8, selector)], 10, a, b, Some(true));
                cases.push((code, lanes(value, scc)));
            }
        }
        cases
    }

    #[test]
    fn scalar_alu_matches_definitions_on_boundary_operands() {
        let vectors = sop2_vectors();
        assert_eq!(vectors.len(), 9 * 10 * 10 * 2);
        for (code, expected) in &vectors {
            verify(code, &[0, u32::MAX], Some(expected));
            assert_translated(code, expected);
        }
    }

    #[test]
    fn signed_add_and_subtract_set_scc_on_overflow_not_carry() {
        // (opcode, S0, S1, D, SCC), worked by hand from RDNA 2 ISA guide 12.1.
        for (opcode, a, b, value, scc) in [
            (2, 0x7fff_ffff, 1, 0x8000_0000, true), // MAX + 1: overflow, no carry
            (2, u32::MAX, 1, 0, false),             // -1 + 1: carry, no overflow
            (2, 0x8000_0000, u32::MAX, 0x7fff_ffff, true), // MIN + -1: overflow
            (2, 0x8000_0000, 0x7fff_ffff, u32::MAX, false), // MIN + MAX = -1
            (3, 0x8000_0000, 1, 0x7fff_ffff, true), // MIN - 1: overflow
            (3, 0, 1, u32::MAX, false),             // 0 - 1: borrow, no overflow
            (3, 0x7fff_ffff, u32::MAX, 0x8000_0000, true), // MAX - -1: overflow
            (3, 0x8000_0000, 0x8000_0000, 0, false), // MIN - MIN = 0
        ] {
            let code = sop2_program(&[sop2(opcode, 10, 8, 9)], 10, a, b, Some(false));
            let expected = lanes(value, scc);
            assert_eq!(sop2_expected(opcode, a, b, false), (value, scc));
            verify(&code, &[0, u32::MAX], Some(&expected));
            assert_translated(&code, &expected);
        }
        for (code, expected) in signed_immediate_overflows() {
            verify(&code, &[0, u32::MAX], Some(&expected));
            assert_translated(&code, &expected);
        }
    }

    /// Overflow through an inline and a literal source, observed by the branch.
    fn signed_immediate_overflows() -> Vec<(Vec<u8>, [u32; 2])> {
        vec![
            // s_add_i32 s10, s8, 1 with s8 = MAX: 0x80000000, overflow.
            (
                sop2_program(&[sop2(2, 10, 8, 0x81)], 10, 0x7fff_ffff, 0, Some(false)),
                lanes(0x8000_0000, true),
            ),
            // s_sub_i32 s10, s8, 0x12345678 with s8 = MIN: 0x6dcba988, overflow.
            (
                sop2_program(
                    &[sop2(3, 10, 8, 255), 0x1234_5678],
                    10,
                    0x8000_0000,
                    0,
                    Some(false),
                ),
                lanes(0x6dcb_a988, true),
            ),
            // s_add_i32 s10, 0x40000000, s8 with s8 = 0x40000000: 0x80000000, overflow.
            (
                sop2_program(
                    &[sop2(2, 10, 255, 8), 0x4000_0000],
                    10,
                    0x4000_0000,
                    0,
                    Some(false),
                ),
                lanes(0x8000_0000, true),
            ),
            // s_sub_i32 s10, -16, s8 with s8 = MAX: 0x7ffffff1, overflow.
            (
                sop2_program(&[sop2(3, 10, 0xd0, 8)], 10, 0x7fff_ffff, 0, Some(false)),
                lanes(0x7fff_fff1, true),
            ),
        ]
    }

    #[test]
    fn scalar_alu_aliasing_inline_and_literal_operands() {
        let cases = sop2_operand_cases();
        assert_eq!(cases.len(), 9 * 10);
        for (code, expected) in &cases {
            verify(code, &[0, u32::MAX], Some(expected));
            assert_translated(code, expected);
        }
    }

    /// All three implementations reject `code`, and at byte `offset`.
    fn rejected_at(code: &[u8], offset: usize) {
        rejected_at_with(code, offset, &[0, 1]);
    }

    fn rejected_at_with(code: &[u8], offset: usize, input: &[u32]) {
        let error = compile(Architecture::Gfx1030, code).unwrap_err();
        let at = match error {
            xenora_shader::CompileError::Verify { offset, .. } => offset,
            xenora_shader::CompileError::Decode(ref decode) => decode.offset,
            ref other => panic!("{other:?}"),
        };
        assert_eq!(at, offset, "{error:?}");
        assert_eq!(xenora_ref::execute(code, input).unwrap_err().offset, offset);
        let independent = independent_ref::execute(code, input).unwrap_err();
        assert!(
            independent.starts_with(&format!("{offset:#x}:")),
            "{independent}"
        );
    }

    #[test]
    fn scalar_alu_rejects_forms_outside_the_subset() {
        // load 8, wait 4, two literal moves 8 each, s_cmp 4: the SOP2 word is at 0x20.
        for opcode in [11, 15, 17, 19] {
            // s_cselect_b64 and the 64-bit bitwise forms.
            rejected_at(
                &sop2_program(&[sop2(opcode, 10, 8, 9)], 10, 1, 2, Some(false)),
                0x20,
            );
        }
        for opcode in [4, 5] {
            // Without the s_cmp, the carry-in reads SCC before anything set it.
            rejected_at(
                &sop2_program(&[sop2(opcode, 10, 8, 9)], 10, 1, 2, None),
                0x1c,
            );
        }
        for (alu, result) in [
            (&[sop2(0, 4, 8, 9)][..], 9),    // Resource descriptor destination.
            (&[sop2(0, 106, 8, 9)][..], 9),  // VCC_LO destination.
            (&[sop2(0, 10, 8, 11)][..], 9),  // Uninitialized s11.
            (&[sop2(0, 10, 106, 9)][..], 9), // VCC_LO source.
            (&[sop2(0, 10, 255, 255), 0x1234_5678][..], 9), // Two literal sources.
        ] {
            // v2 reads s9, which is valid, so only the SOP2 word can fail.
            rejected_at(&sop2_program(alu, result, 1, 2, Some(false)), 0x20);
        }
        // v_mov_b32 from VCC_LO before anything wrote VCC_LO.
        let mut code = sop2_program(&[sop2(0, 10, 8, 9)], 10, 1, 2, Some(false));
        assert_eq!(code[0x24..0x28], 0x7e04_020a_u32.to_le_bytes());
        code[0x24] = 106;
        rejected_at(&code, 0x24);
    }

    #[test]
    fn sgpr_moves_need_known_values_even_for_inactive_lanes() {
        let words: [u32; 11] = [
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70, // load v1, wait
            0x7e04_0280, // v_mov_b32 v2, 0
            0xbefe_0382, // s_mov_b32 exec_lo, 2
            0x7e04_0208, // v_mov_b32 v2, s8 (s8 never written)
            0xbefe_03c1, // s_mov_b32 exec_lo, -1
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ];
        let code: Vec<u8> = words[..11]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        // One lane, inactive under EXEC_LO = 2: the interpreters must not skip the check.
        rejected_at_with(&code, 0x14, &[0]);
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn scalar_alu_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let metal = xenora_metal::NativeCompute::new().unwrap();
        let run = |code: &[u8], input: &[u32]| {
            let compiled = compile(Architecture::Gfx1030, code).unwrap();
            metal
                .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap()
        };
        let input: Vec<u32> = (0..64).map(|lane| lane * 0x0101_0101).collect();
        let expected: Vec<_> = input.iter().map(|word| word ^ 0x0000_000e).collect();
        assert_eq!(run(SCALAR_ALU_FIXTURE, &input), expected);
        let logic: Vec<_> = input.iter().map(|word| word ^ SCALAR_LOGIC_KEY).collect();
        assert_eq!(run(SCALAR_LOGIC_FIXTURE, &input), logic);
        let compares: Vec<_> = input.iter().map(|word| word ^ SCALAR_COMPARE_KEY).collect();
        assert_eq!(run(SCALAR_COMPARE_FIXTURE, &input), compares);
        let unary: Vec<_> = input.iter().map(|word| word ^ SCALAR_UNARY_KEY).collect();
        assert_eq!(run(SCALAR_UNARY_FIXTURE, &input), unary);
        let arith: Vec<_> = input.iter().map(|word| word ^ SCALAR_ARITH_KEY).collect();
        assert_eq!(run(SCALAR_ARITH_FIXTURE, &input), arith);
        let lanes: Vec<_> = VECTOR_INTEGER_INPUT
            .iter()
            .map(|&x| vector_integer_key(x))
            .collect();
        assert_eq!(run(VECTOR_INTEGER_FIXTURE, &VECTOR_INTEGER_INPUT), lanes);
        assert_eq!(run(&words(&VCC_LANE_ADD), &[0, 7]), [2, 9]);
        let mut vectors = sop2_vectors();
        vectors.extend(sop2_operand_cases());
        vectors.extend(signed_immediate_overflows());
        for (code, expected) in &vectors {
            assert_eq!(&run(code, &[0, u32::MAX]), expected);
        }
        eprintln!(
            "{} SOP2 programs and fixtures matched on native Metal",
            vectors.len() + 7
        );
    }

    // ---- VOPC integer compares (docs/shaders/vector-compare.md) ----

    const COMPARE_FIXTURE: &[u8] = include_bytes!("../../fixtures/rdna2/vector-compare.bin");
    const COMPARE_VALUES: [u32; 10] = [
        0,
        1,
        0x7fff_fffe,
        0x7fff_ffff,
        0x8000_0000,
        0x8000_0001,
        0xffff_fff0,
        0xffff_fffe,
        u32::MAX,
        0x0f0f_0f0f,
    ];
    /// VOPC opcodes 129-134 (`_i32`) and 193-198 (`_u32`), RDNA 2 XML / ISA Table 61.
    const COMPARE_OPCODES: [u32; 12] = [129, 130, 131, 132, 133, 134, 193, 194, 195, 196, 197, 198];

    /// The test from the opcode's definition: offsets LT, EQ, LE, GT, NE, GE;
    /// `_i32` compares two's-complement values, `_u32` the unsigned values.
    fn compare_holds(opcode: u32, s0: u32, s1: u32) -> bool {
        let (a, b) = if opcode < 192 {
            (i128::from(s0 as i32), i128::from(s1 as i32))
        } else {
            (i128::from(s0), i128::from(s1))
        };
        let difference = a - b;
        match opcode & 0x3f {
            1 => difference < 0,
            2 => difference == 0,
            3 => difference <= 0,
            4 => difference > 0,
            5 => difference != 0,
            6 => difference >= 0,
            _ => unreachable!(),
        }
    }

    /// VCC_LO for one wave: bit n set iff lane n exists, EXEC[n] and the test pass.
    fn wave_mask(wave: &[u32], exec: u32, test: impl Fn(u32) -> bool) -> u32 {
        wave.iter()
            .enumerate()
            .filter(|&(lane, &x)| exec >> lane & 1 == 1 && test(x))
            .fold(0, |mask, (lane, _)| mask | 1 << lane)
    }

    /// VOPC word: encoding 0b0111110, OP 17-24, VSRC1 9-16, SRC0 0-8.
    fn vopc(opcode: u32, src0: u32, vsrc1: u32) -> u32 {
        0x7c00_0000 | opcode << 17 | vsrc1 << 9 | src0
    }

    /// The fixture's result, written from its listing with explicit relations.
    fn compare_fixture_expected(input: &[u32]) -> Vec<u32> {
        let mut output = Vec::new();
        for wave in input.chunks(32) {
            let full = u32::MAX;
            let rows: [&dyn Fn(u32) -> bool; 10] = [
                &|x| (x as i32) < -1,
                &|x| x != u32::MAX,
                &|x| x == 0,
                &|x| x == 0x8000_0000,
                &|x| x == 0x7fff_ffff,
                &|x| x >= 0x7fff_ffff,
                &|x| (x as i32) < -16,
                &|x| x < 64,
                &|x| x != 0, // ne_u32 against the zero mask of ne_i32 v1, v1
                &|x| x == 0x8000_0000 || x == 0x8000_0001,
            ];
            let folded = rows
                .iter()
                .fold(0u32, |acc, test| (acc << 1) ^ wave_mask(wave, full, test));
            let partial = wave_mask(wave, 0x5555_5555, |x| x >= 0x0f0f_0f0f);
            let runtime = wave_mask(wave, full, |x| x > 0x0f0f_0f0f);
            for lane in 0..wave.len() {
                let v7 = if lane % 2 == 0 { partial } else { 0xa5a5_a5a5 };
                let mut v2 = folded ^ v7;
                if runtime != 0 {
                    v2 ^= 0x0f0f_0f0f;
                }
                output.push(v2);
            }
        }
        output
    }

    /// A boundary wave (branch not taken), a wave of small values (VCC_LO = 0
    /// at the runtime branch, so it is taken) and a final partial wave.
    fn compare_fixture_input() -> Vec<u32> {
        let mut input: Vec<u32> = (0..32)
            .map(|lane| COMPARE_VALUES[lane % 10] ^ (lane as u32 / 10))
            .collect();
        input.extend((0..32).map(|lane| (lane * 0x0101_0101) & 0x0f0f_0f0f));
        // Its only values above 0x0f0f0f0f are negative as i32.
        input.extend([
            0x8000_0000,
            0,
            u32::MAX,
            63,
            64,
            1,
            0xffff_ffef,
            0x8000_0001,
            16,
        ]);
        input
    }

    #[test]
    fn compare_fixture_matches_exact_definitions() {
        let input = compare_fixture_input();
        let expected = compare_fixture_expected(&input);
        verify(COMPARE_FIXTURE, &input, Some(&expected));
        // Both runtime branch outcomes occur in this dispatch.
        assert_ne!(expected[0] ^ expected[32], 0);
        let compiled = compile(Architecture::Gfx1030, COMPARE_FIXTURE).unwrap();
        assert_eq!(
            compiled.ir.matches("wave.mask").count(),
            13,
            "{}",
            compiled.ir
        );
        assert!(compiled.ir.contains("branch.vccz"), "{}", compiled.ir);
        assert!(compiled.ir.contains("join.vccz"), "{}", compiled.ir);
        // No assumption that a Metal SIMD group or threadgroup is an RDNA wave.
        assert!(compiled.msl.contains("[[threads_per_grid]]"));
        for forbidden in ["simd_", "threadgroup", "barrier", "quad_"] {
            assert!(!compiled.msl.contains(forbidden), "{forbidden}");
        }
    }

    #[test]
    fn compare_fixture_inputs_distinguish_signedness_in_every_ordered_slot() {
        let input = compare_fixture_input();
        let expected = compare_fixture_expected(&input);
        let mut flipped = 0;
        for offset in (0..COMPARE_FIXTURE.len()).step_by(4) {
            let word = u32::from_le_bytes(COMPARE_FIXTURE[offset..offset + 4].try_into().unwrap());
            let opcode = word >> 17 & 0xff;
            // Ordered relations only; eq and ne do not depend on signedness.
            if word >> 25 != 0x3e || !matches!(opcode & 0x3f, 1 | 3 | 4 | 6) {
                continue;
            }
            let mut code = COMPARE_FIXTURE.to_vec();
            code[offset + 2] ^= 0x80; // opcode bit 6: _i32 <-> _u32
            let changed = xenora_ref::execute(&code, &input).unwrap();
            assert_ne!(changed, expected, "slot at 0x{offset:x}");
            assert_eq!(independent_ref::execute(&code, &input).unwrap(), changed);
            flipped += 1;
        }
        assert_eq!(flipped, 9);
    }

    /// v1 = input, v3 = `b`; vcc_lo = form(v1, v3) or form(v3, v1); output vcc_lo.
    fn compare_program(opcode: u32, b: u32, swapped: bool) -> Vec<u8> {
        let compare = if swapped {
            vopc(opcode, 256 + 3, 1)
        } else {
            vopc(opcode, 256 + 1, 3)
        };
        words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70, // load v1, wait
            0x7e06_02ff,
            b, // v_mov_b32 v3, b
            compare,
            0x7e04_026a, // v_mov_b32 v2, vcc_lo
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ])
    }

    fn compare_lanes() -> Vec<u32> {
        (0..32).map(|lane| COMPARE_VALUES[lane % 10]).collect()
    }

    fn compare_vectors() -> Vec<(Vec<u8>, Vec<u32>)> {
        let input = compare_lanes();
        let mut vectors = Vec::new();
        for opcode in COMPARE_OPCODES {
            for b in COMPARE_VALUES {
                for swapped in [false, true] {
                    let mask = wave_mask(&input, u32::MAX, |x| {
                        if swapped {
                            compare_holds(opcode, b, x)
                        } else {
                            compare_holds(opcode, x, b)
                        }
                    });
                    vectors.push((compare_program(opcode, b, swapped), vec![mask; 32]));
                }
            }
        }
        vectors
    }

    #[test]
    fn every_compare_form_matches_definitions_on_boundary_operands() {
        // Checkpoints derived by hand: 0x80000000 is the least i32, the greatest u32 half.
        assert!(compare_holds(129, 0x8000_0000, 0x7fff_ffff));
        assert!(!compare_holds(193, 0x8000_0000, 0x7fff_ffff));
        assert!(compare_holds(132, 0, u32::MAX) && !compare_holds(196, 0, u32::MAX));
        assert!(compare_holds(131, 5, 5) && compare_holds(198, 5, 5));
        let vectors = compare_vectors();
        assert_eq!(vectors.len(), 12 * 10 * 2);
        let input = compare_lanes();
        for (code, expected) in &vectors {
            verify(code, &input, Some(expected));
        }
    }

    /// v2 = 0xdeadbeef; EXEC_LO = `exec`; compare v1 < 0x10 (unsigned);
    /// v2 = vcc_lo under the same EXEC; EXEC_LO = -1.
    fn partial_compare_program(exec: u32) -> Vec<u8> {
        words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_02ff,
            0xdead_beef,
            0xbefe_03ff,
            exec,
            vopc(196, 0x90, 1), // v_cmp_gt_u32_e32 vcc_lo, 16, v1
            0x7e04_026a,
            0xbefe_03c1,
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ])
    }

    #[test]
    fn partial_exec_masks_inactive_lanes_and_keeps_their_destinations() {
        let input: Vec<u32> = (0..32).collect();
        for exec in [u32::MAX, 0x5555_5555, 0x8000_8001, 1 << 31] {
            // Lanes 0-15 hold values below 16 and would all pass.
            let mask = wave_mask(&input, exec, |x| x < 16);
            let expected: Vec<u32> = (0..32)
                .map(|lane| {
                    if exec >> lane & 1 == 1 {
                        mask
                    } else {
                        0xdead_beef
                    }
                })
                .collect();
            verify(&partial_compare_program(exec), &input, Some(&expected));
        }
        assert_eq!(wave_mask(&input, 0x5555_5555, |x| x < 16), 0x5555);
        // Empty EXEC: the compare reads nothing, and the move does not run either.
        verify(
            &partial_compare_program(0),
            &input,
            Some(&[0xdead_beef; 32]),
        );
    }

    /// vcc_lo = -1; EXEC_LO = 0; compare; s_cbranch_vccz over a write; EXEC_LO = -1;
    /// v2 = vcc_lo ^ marker. Hardware writes VCC_LO = 0 even with empty EXEC.
    fn empty_exec_program(src0: u32) -> Vec<u8> {
        words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e06_0281, // v_mov_b32 v3, 1
            0xbeea_03c1, // s_mov_b32 vcc_lo, -1
            0xbefe_0380, // s_mov_b32 exec_lo, 0
            vopc(194, src0, 1),
            0xbf86_0001, // s_cbranch_vccz (known VCC_LO = 0: taken)
            0xbeea_03c1, // s_mov_b32 vcc_lo, -1 on the wrong path
            0xbefe_03c1,
            0x7e04_026a, // v_mov_b32 v2, vcc_lo
            0x3a04_0702, // v_xor_b32 v2, v2, v3
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ])
    }

    #[test]
    fn empty_exec_clears_a_stale_mask_without_reading_sources() {
        let input = [7, 0, u32::MAX];
        // v1 == v1 everywhere: only the EXEC rule makes the mask zero.
        verify(&empty_exec_program(256 + 1), &input, Some(&[1; 3]));
        // Under empty EXEC an unknown SGPR, v9 and even vcc_lo-from-a-compare are not read.
        verify(&empty_exec_program(20), &input, Some(&[1; 3]));
        verify(&empty_exec_program(256 + 9), &input, Some(&[1; 3]));
        let ir = compile(Architecture::Gfx1030, &empty_exec_program(256 + 1))
            .unwrap()
            .ir;
        assert!(
            !ir.contains("wave.mask") && ir.contains("taken=true"),
            "{ir}"
        );
    }

    /// Compare v1 != 0 (VCC lane-derived), then `s_cbranch_vccz` over `region`.
    fn runtime_branch_program(region: &[u32], before_branch: &[u32]) -> Vec<u8> {
        let mut code = vec![
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_0281, // v_mov_b32 v2, 1
            0x7e06_0282, // v_mov_b32 v3, 2
            0x7d8a_0280, // v_cmp_ne_u32_e32 vcc_lo, 0, v1
        ];
        code.extend(before_branch);
        code.push(0xbf86_0000 | region.len() as u32);
        code.extend(region);
        code.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
        words(&code)
    }

    /// Not taken: v2 = (1 + 2) ^ vcc_lo, v3 = vcc_lo. Taken: v2 = 1.
    const RUNTIME_REGION: [u32; 4] = [
        0xd525_0002,
        0x0202_0702, // v_add_nc_u32_e64 v2, v2, v3
        0x7e06_026a, // v_mov_b32 v3, vcc_lo
        0x3a04_0702, // v_xor_b32 v2, v2, v3
    ];

    fn runtime_expected(input: &[u32]) -> Vec<u32> {
        input
            .chunks(32)
            .flat_map(|wave| {
                let mask = wave_mask(wave, u32::MAX, |x| x != 0);
                let value = if mask == 0 { 1 } else { 3 ^ mask };
                vec![value; wave.len()]
            })
            .collect()
    }

    fn runtime_input() -> Vec<u32> {
        // Wave 0 all zero (taken), wave 1 one nonzero lane, wave 2 partial and zero.
        let mut input = vec![0; 32];
        input.extend((0..32).map(|lane| u32::from(lane == 17) * 0x8000_0000));
        input.extend([0; 5]);
        input
    }

    #[test]
    fn runtime_vccz_takes_each_edge_from_lane_data_in_one_dispatch() {
        assert_eq!(vopc(197, 0x80, 1), 0x7d8a_0280);
        let code = runtime_branch_program(&RUNTIME_REGION, &[]);
        let mut input = runtime_input();
        let expected = runtime_expected(&input);
        assert_eq!(expected[0], 1);
        assert_eq!(expected[32], 3 ^ 1 << 17);
        verify(&code, &input, Some(&expected));
        // One nonzero lane in the final partial wave flips only that wave's edge.
        input[64 + 4] = 9;
        let expected = runtime_expected(&input);
        assert_eq!(expected[64], 3 ^ 1 << 4);
        verify(&code, &input, Some(&expected));
        let compiled = compile(Architecture::Gfx1030, &code).unwrap();
        assert!(compiled.ir.contains("branch.vccz"), "{}", compiled.ir);
        assert_eq!(
            compiled.ir.matches("join.vccz").count(),
            2,
            "{}",
            compiled.ir
        );
        // A branch on a known VCC_LO is still resolved during translation.
        let known = runtime_branch_program(&RUNTIME_REGION, &[0xbeea_0380]); // vcc_lo = 0
        verify(&known, &input, Some(&vec![1; input.len()]));
        assert!(
            !compile(Architecture::Gfx1030, &known)
                .unwrap()
                .ir
                .contains("branch.vccz")
        );
    }

    /// load v1; v2 = 1; compare v1 != 0; `middle`; v2 = new vcc_lo; store v2.
    fn mask_chain_program(middle: &[u32]) -> Vec<u8> {
        let mut code = vec![
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_0281, // v_mov_b32 v2, 1
            0x7d8a_0280, // v_cmp_ne_u32_e32 vcc_lo, 0, v1
        ];
        code.extend(middle);
        code.extend([
            0x7e04_026a,
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        words(&code)
    }

    #[test]
    fn compare_masks_feed_later_compares_and_lane_varying_joins() {
        // v_cmp_eq_u32 vcc_lo, vcc_lo, v1: lane n tests old_mask == x_n.
        let code = mask_chain_program(&[vopc(194, 106, 1)]);
        let input = [1, 0, 0, 0];
        // old mask = 0b0001; only lane 0 holds 1 == old mask.
        verify(&code, &input, Some(&[1; 4]));
        let input = [0, 2, 3, 0];
        // old mask = 0b0110 = 6; no lane holds 6.
        verify(&code, &input, Some(&[0; 4]));
        let input = [0, 6, 6, 0];
        verify(&code, &input, Some(&[6; 4]));
        // A join whose value differs per lane feeds a compare: region v2 = v1,
        // then v_cmp_eq_u32 vcc_lo, 0, v2.
        let code = mask_chain_program(&[0xbf86_0001, 0x7e04_0301, vopc(194, 0x80, 2)]);
        // Not taken (lane 0 nonzero): v2 = x, so the mask marks the zero lanes.
        verify(&code, &[1, 0, 5, 0], Some(&[0b1010; 4]));
        // Taken (all zero): v2 keeps 1 everywhere, so no lane is zero.
        verify(&code, &[0, 0, 0, 0], Some(&[0; 4]));
    }

    #[test]
    fn runtime_regions_follow_exec_store_and_immediate_rules() {
        // Under empty EXEC a region source is not read, so v9 need not be ready.
        let code = {
            let mut words_ = vec![
                0xe030_2000,
                0x8000_0100,
                0xbf8c_3f70,
                0x7e04_0281, // v_mov_b32 v2, 1
                0x7d8a_0280, // v_cmp_ne_u32_e32 vcc_lo, 0, v1
                0xbefe_0380, // s_mov_b32 exec_lo, 0
                0xbf86_0001, // runtime s_cbranch_vccz over one word
                0x7e04_0309, // v_mov_b32 v2, v9 (v9 uninitialized, not read)
                0xbefe_03c1, // s_mov_b32 exec_lo, -1
            ];
            words_.extend([0xe070_2000, 0x8001_0200, 0xbbfd_0000, 0xbf81_0000]);
            words(&words_)
        };
        verify(&code, &[0], Some(&[1]));
        verify(&code, &[1], Some(&[1]));
        // A store outstanding at the branch is allowed; its wait follows the target.
        let code = words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_0281, // v_mov_b32 v2, 1
            0x7d8a_0280, // v_cmp_ne_u32_e32 vcc_lo, 0, v1
            0xe070_2000,
            0x8001_0200, // store v2
            0xbf86_0001,
            0x7e04_0282, // v_mov_b32 v2, 2 (after the store: not observed)
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(&code, &[0, 7], Some(&[1, 1]));
        // The inline +1.0f bit pattern is an immediate like any other in a region,
        // and an exact FP32 add after the join is accepted when both edges allow it.
        let code = words(&[
            0xe030_2000,
            0x8000_0100,
            0xbf8c_3f70,
            0x7e04_02f2, // v_mov_b32 v2, 1.0
            0x7d8a_0280, // v_cmp_ne_u32_e32 vcc_lo, 0, v1
            0xbf86_0002,
            0x7e04_02ff,
            0x4000_0000, // v_mov_b32 v2, 2.0
            0x0604_0502, // v_add_f32 v2, v2, v2
            0xe070_2000,
            0x8001_0200,
            0xbbfd_0000,
            0xbf81_0000,
        ]);
        verify(&code, &[0], Some(&[0x4000_0000]));
        verify(&code, &[1], Some(&[0x4080_0000]));
    }

    fn initialized_exec_lo_compare() -> Vec<u8> {
        let mut code = compare_program(129, 5, false);
        code[0x14..0x18].copy_from_slice(&vopc(129, 126, 3).to_le_bytes());
        code
    }

    #[test]
    fn compares_accept_initialized_exec_lo() {
        // ABI initializes EXEC_LO to all ones: signed -1 < 5 is true.
        // Only present lanes contribute, so the final one-lane wave has mask1.
        // The legacy third reference does not cover this newly admitted operand;
        // keep its existing three-way rejection checks unchanged below.
        let code = initialized_exec_lo_compare();
        compile(Architecture::Gfx1030, &code).unwrap();
        for expected in [
            vec![1],
            vec![0x7fff_ffff; 31],
            vec![u32::MAX; 32],
            [vec![u32::MAX; 32], vec![1]].concat(),
        ] {
            assert_eq!(
                xenora_ref::execute(&code, &vec![0; expected.len()]).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn compares_and_runtime_branches_reject_outside_the_contract() {
        let base = compare_program(129, 5, false);
        // The VOPC word is at 0x14; change only its SRC0 selector.
        let with_src0 = |selector: u32| {
            let mut code = base.clone();
            let word = vopc(129, selector, 3);
            code[0x14..0x18].copy_from_slice(&word.to_le_bytes());
            code
        };
        for selector in [107, 124, 125, 127, 233, 249, 250, 251, 253, 254] {
            rejected_at(&with_src0(selector), 0x14);
        }
        rejected_at(&with_src0(20), 0x14); // s20 has no known value
        rejected_at(&with_src0(106), 0x14); // vcc_lo uninitialized
        rejected_at(&with_src0(256 + 9), 0x14); // v9 uninitialized
        rejected_at(&with_src0(256), 0x14); // v0 is the resource index
        // VSRC1 uninitialized.
        let mut code = base.clone();
        code[0x14..0x18].copy_from_slice(&vopc(129, 256 + 1, 9).to_le_bytes());
        rejected_at(&code, 0x14);
        // v_cmpx (writes EXEC), F and T forms, and the VOP3 encoding.
        for word in [vopc(210, 257, 3), vopc(128, 257, 3), vopc(135, 257, 3)] {
            let mut code = base.clone();
            code[0x14..0x18].copy_from_slice(&word.to_le_bytes());
            rejected_at(&code, 0x14);
        }
        let mut e64 = base[..0x14].to_vec();
        e64.extend(words(&[0xd481_006a, 0x0002_0701]));
        e64.extend_from_slice(&base[0x18..]);
        rejected_at(&e64, 0x14);

        // Runtime s_cbranch_vccz at 0x18; the region starts at 0x1c.
        let reject_region = |region: &[u32], at: usize| {
            rejected_at_with(&runtime_branch_program(region, &[]), at, &[0, 1]);
        };
        reject_region(&[0xbe88_0380], 0x1c); // s_mov_b32 s8, 0
        reject_region(&[0x0604_0702], 0x1c); // v_add_f32 v2, v2, v3
        reject_region(&[0x7c02_0302], 0x1c); // v_cmp_lt_i32 in the region
        reject_region(&[0xbf82_0000], 0x1c); // s_branch in the region
        reject_region(&[0x7e0a_0281], 0x1c); // v_mov v5, 1: v5 not initialized
        reject_region(&[0x7e04_0281, 0x6a04_0b02], 0x20); // second word reads v5
        reject_region(&[0x7e04_0214], 0x1c); // v_mov v2, s20 (unknown)
        reject_region(&[0xe070_2000, 0x8001_0200], 0x1c); // store in the region
        // A load outstanding at the branch (now at 0x20).
        rejected_at_with(
            &runtime_branch_program(&RUNTIME_REGION, &[0xe030_2000, 0x8000_0500]),
            0x20,
            &[0, 1],
        );
        // The same checks hold under empty EXEC and when the branch is taken
        // (input 0: VCC_LO = 0): s_mov_b32 exec_lo, 0 precedes the branch at 0x1c.
        let code = runtime_branch_program(&[0x7e0a_0281], &[0xbefe_0380]);
        rejected_at_with(&code, 0x20, &[0]);
    }

    #[test]
    #[ignore = "requires a native Metal device with API and shader validation"]
    fn vector_compare_native_metal_known_answers() {
        assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
        assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
        let metal = xenora_metal::NativeCompute::new().unwrap();
        let run = |code: &[u8], input: &[u32]| {
            let compiled = compile(Architecture::Gfx1030, code).unwrap();
            metal
                .run_words(&compiled.msl, xenora_shader::ENTRY_POINT, input)
                .unwrap()
        };
        let mut programs = 0;
        assert_eq!(
            run(&initialized_exec_lo_compare(), &[0; 33]),
            [vec![u32::MAX; 32], vec![1]].concat()
        );
        programs += 1;
        let input = compare_fixture_input();
        assert_eq!(
            run(COMPARE_FIXTURE, &input),
            compare_fixture_expected(&input)
        );
        programs += 1;
        let lanes = compare_lanes();
        for (code, expected) in compare_vectors() {
            assert_eq!(run(&code, &lanes), expected);
            programs += 1;
        }
        let counting: Vec<u32> = (0..32).collect();
        for exec in [u32::MAX, 0x5555_5555, 0x8000_8001, 1 << 31, 0] {
            let mask = wave_mask(&counting, exec, |x| x < 16);
            let expected: Vec<u32> = (0..32)
                .map(|lane| {
                    if exec >> lane & 1 == 1 {
                        mask
                    } else {
                        0xdead_beef
                    }
                })
                .collect();
            assert_eq!(run(&partial_compare_program(exec), &counting), expected);
            programs += 1;
        }
        for src0 in [256 + 1, 20, 256 + 9] {
            assert_eq!(run(&empty_exec_program(src0), &[7, 0, u32::MAX]), [1; 3]);
            programs += 1;
        }
        let code = runtime_branch_program(&RUNTIME_REGION, &[]);
        let mut input = runtime_input();
        assert_eq!(run(&code, &input), runtime_expected(&input));
        input[64 + 4] = 9;
        assert_eq!(run(&code, &input), runtime_expected(&input));
        programs += 1;
        let chain = mask_chain_program(&[vopc(194, 106, 1)]);
        for (input, mask) in [([1, 0, 0, 0], 1), ([0, 2, 3, 0], 0), ([0, 6, 6, 0], 6)] {
            assert_eq!(run(&chain, &input), [mask; 4]);
        }
        let join = mask_chain_program(&[0xbf86_0001, 0x7e04_0301, vopc(194, 0x80, 2)]);
        assert_eq!(run(&join, &[1, 0, 5, 0]), [0b1010; 4]);
        assert_eq!(run(&join, &[0, 0, 0, 0]), [0; 4]);
        programs += 2;
        eprintln!("{programs} VOPC programs matched on native Metal");
    }
}
