// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_ref::{DecodedInstruction, Op, ReferenceError, Source, VectorAlu, decode, execute};

const ADD: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");
const WAIT_READ: [u8; 4] = [0x70, 0x3f, 0x8c, 0xbf];

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

#[test]
fn decodes_high_vop2_operands_and_integer_immediates() {
    // AMD RDNA 2 ISA §13: VOP2 packs its second input and destination
    // across byte boundaries. Selectors 195 and 208 encode -3 and -16.
    // The original assembly operands were checked with gfx1030 llvm-mc.
    let code = bytes(&[
        0x37ff_fd81, // v_and_b32_e32 v255, v129, v254
        0x7e04_02c3, // v_mov_b32_e32 v2, -3
        0xbe88_03d0, // s_mov_b32 s8, -16
        0xbf06_ff08,
        0x1234_5678, // s_cmp_eq_u32 s8, 0x12345678
    ]);
    assert_eq!(
        decode(&code),
        Ok(vec![
            DecodedInstruction {
                offset: 0,
                byte_len: 4,
                op: Op::Vector(
                    VectorAlu::And,
                    255,
                    xenora_ref::CompareSource::Vector(129),
                    254
                ),
            },
            DecodedInstruction {
                offset: 4,
                byte_len: 4,
                op: Op::Move(2, Source::Immediate(0xffff_fffd)),
            },
            DecodedInstruction {
                offset: 8,
                byte_len: 4,
                op: Op::ScalarMove(8, Source::Immediate(0xffff_fff0)),
            },
            DecodedInstruction {
                offset: 12,
                byte_len: 8,
                op: Op::CompareEqual(Source::Register(8), Source::Immediate(0x1234_5678)),
            },
        ])
    );
}

#[test]
fn enforces_inclusive_input_and_code_limits() {
    let input_limit_error = ReferenceError {
        offset: 0,
        detail: "input word count outside reference limits",
    };
    assert_eq!(execute(ADD, &[]), Err(input_limit_error));
    assert_eq!(execute(ADD, &[1; 4096]), Ok(vec![0x1234_5679; 4096]));
    assert_eq!(
        execute(ADD, &[1; 4097]),
        Err(ReferenceError {
            offset: 0,
            detail: "input word count outside reference limits",
        })
    );

    let padding_words = (1024 - ADD.len()) / WAIT_READ.len();
    let padded = [WAIT_READ.repeat(padding_words), ADD.to_vec()].concat();
    assert_eq!(padded.len(), 1024);
    assert_eq!(execute(&padded, &[1]), Ok(vec![0x1234_5679]));
    let oversized = [WAIT_READ.as_slice(), &padded].concat();
    assert_eq!(
        execute(&oversized, &[1]),
        Err(ReferenceError {
            offset: 0,
            detail: "instruction stream exceeds reference limit",
        })
    );
}

#[test]
fn rejects_valid_instruction_words_after_end_and_reports_their_offset() {
    let end = &ADD[ADD.len() - 4..];
    let code = [ADD, &WAIT_READ, end].concat();
    let error = execute(&code, &[1]).unwrap_err();
    assert_eq!(
        error,
        ReferenceError {
            offset: ADD.len(),
            detail: "trailing code after s_endpgm",
        }
    );
    assert_eq!(
        error.to_string(),
        "reference byte 0x2c: trailing code after s_endpgm"
    );
}

#[test]
fn preserves_the_index_register_and_rejects_overlapping_loads() {
    let mut writes_index = ADD.to_vec();
    writes_index[5] = 0; // Load into v0, the fixture ABI's index register.
    assert_eq!(
        execute(&writes_index, &[1]),
        Err(ReferenceError {
            offset: 0,
            detail: "invalid load destination or overlapping load",
        })
    );

    let mut second_load = ADD[..8].to_vec();
    second_load[5] = 2; // Distinct destination, still before the first load wait.
    let overlaps = [&ADD[..8], &second_load, &ADD[8..]].concat();
    assert_eq!(
        execute(&overlaps, &[1]),
        Err(ReferenceError {
            offset: 8,
            detail: "invalid load destination or overlapping load",
        })
    );
}

#[test]
fn rejects_a_second_output_store_before_and_after_the_wait() {
    let store = &ADD[28..36];
    for offset in [36, 40] {
        let code = [&ADD[..offset], store, &ADD[offset..]].concat();
        assert_eq!(
            execute(&code, &[1]),
            Err(ReferenceError {
                offset,
                detail: "multiple writes to the output",
            })
        );
    }
}

#[test]
fn protects_descriptor_sgprs_but_allows_ordinary_sgprs() {
    let code = |dst_word: u32| [dst_word.to_le_bytes().as_slice(), ADD].concat();
    assert_eq!(
        execute(&code(0xbe87_0381), &[1]), // s_mov_b32 s7, 1
        Err(ReferenceError {
            offset: 0,
            detail: "write to a reserved resource SGPR",
        })
    );
    assert_eq!(
        execute(&code(0xbe89_0381), &[1]), // s_mov_b32 s9, 1
        Ok(vec![0x1234_5679])
    );
}

#[test]
fn rejects_either_out_of_domain_operand_even_if_the_product_is_normal() {
    fn multiply(a: u32, b: u32) -> Result<Vec<u32>, ReferenceError> {
        let code = bytes(&[
            0x7e04_02ff,
            a, // v_mov_b32_e32 v2, <literal>
            0x7e06_02ff,
            b,           // v_mov_b32_e32 v3, <literal>
            0x1002_0702, // v_mul_f32_e32 v1, v2, v3
        ]);
        execute(&[code.as_slice(), &ADD[28..]].concat(), &[0])
    }

    let one = 1.0f32.to_bits();
    let large_normal = 0x7e80_0000; // 2^126.
    let subnormal = 0x0000_0001; // 2^-149, yielding exact 2^-23 with large_normal.
    let negative_one = (-1.0f32).to_bits();
    assert_eq!(multiply(large_normal, one), Ok(vec![large_normal]));
    for (a, b) in [
        (subnormal, large_normal),
        (large_normal, subnormal),
        (negative_one, one),
        (one, negative_one),
    ] {
        assert_eq!(
            multiply(a, b),
            Err(ReferenceError {
                offset: 16,
                detail: "float operation outside positive, normal, exact binary32 domain",
            })
        );
    }
}

#[test]
fn packed_integer_opcodes_have_independent_typed_decoding() {
    use xenora_ref::{CompareSource, VectorTernary};
    for (first, kind) in [
        (0xd7450004, VectorTernary::XorAdd),
        (0xd54d0004, VectorTernary::LerpU8),
        (0xd5710004, VectorTernary::MaskedSadU8),
    ] {
        assert_eq!(
            decode(&bytes(&[first, 257 | 258 << 9 | 259 << 18])).unwrap()[0].op,
            Op::VectorTernary(
                kind,
                4,
                CompareSource::Vector(1),
                CompareSource::Vector(2),
                CompareSource::Vector(3)
            )
        );
        let parsed = decode(&bytes(&[first, 255 | 128 << 9 | 129 << 18, 0xdeadbeef])).unwrap();
        assert_eq!(parsed[0].byte_len, 12);
        assert_eq!(
            parsed[0].op,
            Op::VectorTernary(
                kind,
                4,
                CompareSource::Immediate(0xdeadbeef),
                CompareSource::Immediate(0),
                CompareSource::Immediate(1)
            )
        );
    }
}
