// SPDX-License-Identifier: GPL-3.0-or-later
//! Literal opcode/operand expectations, independent of production metadata.
use xenora_decoder::{
    Architecture::Gfx1030, DecodeErrorKind, Instruction, VectorSource, VectorTernaryOp, decode,
    decode_at,
};
const OPS: [u32; 2] = [883, 885];
const FLOAT: [u32; 9] = [
    0x3f000000, 0xbf000000, 0x3f800000, 0xbf800000, 0x40000000, 0xc0000000, 0x40800000, 0xc0800000,
    0x3e22f983,
];
fn arity(_op: u32) -> usize {
    3
}
fn code(op: u32, dst: u8, raw: [u32; 3], literal: u32) -> Vec<u8> {
    [
        0xd4000000 | op << 16 | u32::from(dst),
        raw[0] | raw[1] << 9 | raw[2] << 18,
        literal,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
fn source(raw: u32, literal: u32, position: usize) -> Option<VectorSource> {
    Some(match raw {
        0..=107 | 126..=127 => VectorSource::Scalar(raw as u8),
        128..=192 => VectorSource::Immediate(raw - 128),
        193..=208 => VectorSource::Immediate((192i32 - raw as i32) as u32),
        240..=248 => VectorSource::Immediate(if position < 2 {
            [
                0x3800, 0xb800, 0x3c00, 0xbc00, 0x4000, 0xc000, 0x4400, 0xc400, 0x3118,
            ][(raw - 240) as usize]
        } else {
            FLOAT[(raw - 240) as usize]
        }),
        255 => VectorSource::Immediate(literal),
        256..=511 => VectorSource::Register((raw - 256) as u8),
        _ => return None,
    })
}
fn expected(op: u32, dst: u8, src: [VectorSource; 3]) -> Instruction {
    let [src0, src1, src2] = src;
    Instruction::VectorTernary {
        op: match op {
            883 => VectorTernaryOp::MadU32U16,
            885 => VectorTernaryOp::MadI32I16,
            _ => unreachable!(),
        },
        dst,
        src0,
        src1,
        src2,
    }
}

fn baseline(op: u32) -> [u32; 3] {
    [257, 258, if arity(op) == 2 { 128 } else { 259 }]
}
#[test]
fn every_active_selector_and_destination_boundary() {
    for op in OPS {
        for position in 0..arity(op) {
            for raw in 0..512 {
                for dst in [0, 5, 255] {
                    let mut fields = baseline(op);
                    fields[position] = raw;
                    let bytes = code(op, dst, fields, 0xffff);
                    let actual = decode_at(Gfx1030, &bytes, 0);
                    if let Some(value) = source(raw, 0xffff, position) {
                        let mut sources =
                            std::array::from_fn(|i| source(fields[i], 0xffff, i).unwrap());
                        sources[position] = value;
                        let actual = actual.unwrap();
                        assert_eq!(actual.instruction, expected(op, dst, sources));
                        assert_eq!(actual.byte_len, if raw == 255 { 12 } else { 8 });
                        assert_eq!(
                            decode_at(Gfx1030, &bytes[..actual.byte_len as usize], 0).unwrap(),
                            actual
                        );
                    } else {
                        assert!(
                            matches!(
                                actual.unwrap_err().kind,
                                DecodeErrorKind::UnsupportedForm { .. }
                            ),
                            "op={op} position={position} raw={raw}"
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn exact_opcode_scope_and_each_modifier_bit() {
    for op in OPS {
        let bytes = code(op, 5, baseline(op), 0);
        assert!(decode_at(Gfx1030, &bytes, 0).is_ok());
        for bit in (8..16).chain(59..64) {
            let mut modified = bytes.clone();
            modified[bit / 8] |= 1 << (bit % 8);
            assert!(matches!(
                decode_at(Gfx1030, &modified, 0).unwrap_err().kind,
                DecodeErrorKind::UnsupportedForm { .. }
            ));
        }
    }
    assert!(matches!(
        decode_at(Gfx1030, &code(346, 5, [257, 258, 259], 0), 0)
            .unwrap()
            .instruction,
        Instruction::VectorMadSad {
            op: xenora_decoder::VectorMadSadOp::SadU8,
            ..
        }
    ));
    // Nearby full-width/ternary opcodes are not admitted by a family catch-all.
    for op in [335, 336, 337, 340, 343, 350, 834, 840, 878, 880] {
        assert!(decode_at(Gfx1030, &code(op, 5, [257, 258, 259], 0), 0).is_err());
    }
}
#[test]
fn distinct_scalar_identity_budget_precedes_immediate_collapse() {
    // Exhaust all triples over two normal SGPRs, VCC, literal, inline and VGPR.
    for op in OPS {
        for a in [0, 105, 106, 107, 126, 127, 255, 129, 248, 511] {
            for b in [0, 105, 106, 107, 126, 127, 255, 129, 248, 511] {
                for c in [0, 105, 106, 107, 126, 127, 255, 129, 248, 511] {
                    let raw = [a, b, c];
                    let mut keys = std::collections::BTreeSet::new();
                    for value in raw {
                        if matches!(value, 0..=107 | 126..=127 | 255) {
                            keys.insert(value);
                        }
                    }
                    let actual = decode_at(Gfx1030, &code(op, 255, raw, 1), 0);
                    if keys.len() <= 2 {
                        let actual = actual.unwrap();
                        assert_eq!(
                            actual.instruction,
                            expected(
                                op,
                                255,
                                std::array::from_fn(|i| source(raw[i], 1, i).unwrap())
                            )
                        );
                        assert_eq!(actual.byte_len, if raw.contains(&255) { 12 } else { 8 });
                    } else {
                        match actual.unwrap_err().kind {
                            DecodeErrorKind::UnsupportedForm { detail, .. } => {
                                assert!(detail.contains("scalar-bus"))
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn shared_literals_all_positions_truncation_and_stream_boundaries() {
    for op in OPS {
        for mask in 0..(1 << arity(op)) {
            for literal in [0, 1, 0xffffffff, 0x80000000, 0x7fc12345, 0xbf810000] {
                let mut raw = baseline(op);
                for (index, value) in raw.iter_mut().enumerate().take(arity(op)) {
                    if mask & (1 << index) != 0 {
                        *value = 255;
                    }
                }
                let len = if mask == 0 { 8 } else { 12 };
                let bytes = code(op, 255, raw, literal);
                for cut in 0..len {
                    assert!(matches!(
                        decode_at(Gfx1030, &bytes[..cut], 0).unwrap_err().kind,
                        DecodeErrorKind::Truncated { .. }
                    ));
                }
                let mut stream = code(op, 0, baseline(op), 0)[..8].to_vec();
                stream.extend_from_slice(&bytes[..len]);
                stream.extend_from_slice(&0xbf810000u32.to_le_bytes());
                let actual = decode(Gfx1030, &stream).unwrap();
                assert_eq!(actual.len(), 3);
                assert_eq!((actual[1].offset, actual[1].byte_len), (8, len as u8));
                assert_eq!(
                    actual[1].instruction,
                    expected(
                        op,
                        255,
                        std::array::from_fn(|i| source(raw[i], literal, i).unwrap())
                    )
                );
                assert_eq!(actual[2].offset, 8 + len);
                assert_eq!(actual[2].instruction, Instruction::EndProgram);
            }
        }
    }
}
#[test]
fn mnemonics_display_and_existing_add_domain() {
    for (op, name) in OPS.into_iter().zip(["v_mad_u32_u16", "v_mad_i32_i16"]) {
        let raw = [106, 511, if arity(op) == 2 { 128 } else { 105 }];
        let actual = decode_at(Gfx1030, &code(op, 255, raw, 0), 0)
            .unwrap()
            .instruction;
        assert_eq!(actual.mnemonic(), name);
        assert_eq!(
            actual.to_string(),
            format!(
                "{name} v255, vcc_lo, v255{}",
                if arity(op) == 2 { "" } else { ", s105" }
            )
        );
    }
    // Existing VGPR/VGPR ADD keeps its representation; generic sources use the alias.
    let add = code(293, 5, [257, 258, 128], 0);
    assert_eq!(
        decode_at(Gfx1030, &add, 0).unwrap().instruction,
        Instruction::AddNoCarryU32 {
            dst: 5,
            src0: 1,
            src1: 2
        }
    );
    for raw in [[0, 258, 128], [257, 255, 128]] {
        assert!(matches!(
            decode_at(Gfx1030, &code(293, 5, raw, 0), 0)
                .unwrap()
                .instruction,
            Instruction::VectorBinaryE64 { .. }
        ));
    }
    assert!(decode_at(Gfx1030, &code(293, 5, [257, 258, 0], 0), 0).is_err());
}

#[test]
#[ignore = "requires native LLVM with gfx1030 target"]
fn llvm_exact_source_matrix() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut rows = Vec::new();
    for (op, name) in OPS.into_iter().zip(["v_mad_u32_u16", "v_mad_i32_i16"]) {
        for position in 0..3 {
            for (selector, text) in [
                (0, "s0"),
                (105, "s105"),
                (106, "vcc_lo"),
                (107, "vcc_hi"),
                (126, "exec_lo"),
                (127, "exec_hi"),
                (128, "0"),
                (192, "64"),
                (193, "-1"),
                (208, "-16"),
                (240, "0.5"),
                (241, "-0.5"),
                (242, "1.0"),
                (243, "-1.0"),
                (244, "2.0"),
                (245, "-2.0"),
                (246, "4.0"),
                (247, "-4.0"),
                (248, "0.15915494"),
                (255, "0x1234"),
                (256, "v0"),
                (511, "v255"),
            ] {
                let mut raw = [257, 258, 259];
                raw[position] = selector;
                let mut sources = ["v1", "v2", "v3"];
                sources[position] = text;
                rows.push((
                    format!("{name} v255, {}", sources.join(", ")),
                    code(op, 255, raw, 0x1234)[..if selector == 255 { 12 } else { 8 }].to_vec(),
                ));
            }
        }
        for mask in 1..8 {
            let mut raw = [257, 258, 259];
            let mut sources = ["v1", "v2", "v3"];
            for i in 0..3 {
                if mask & (1 << i) != 0 {
                    raw[i] = 255;
                    sources[i] = "0x1234";
                }
            }
            rows.push((
                format!("{name} v5, {}", sources.join(", ")),
                code(op, 5, raw, 0x1234),
            ));
        }
    }
    for (op, name) in [(883, "v_mad_u32_u16"), (885, "v_mad_i32_i16")] {
        for literal in [-32768i32, -17] {
            for mask in 1..8 {
                let mut raw = [257, 258, 259];
                let mut operands = ["v1".to_string(), "v2".to_string(), "v3".to_string()];
                for i in 0..3 {
                    if mask & (1 << i) != 0 {
                        raw[i] = 255;
                        operands[i] = literal.to_string();
                    }
                }
                rows.push((
                    format!("{name} v5, {}", operands.join(", ")),
                    code(op, 5, raw, literal as u32),
                ));
            }
        }
    }
    assert_eq!(rows.len(), 174);
    let llvm = std::env::var_os("LLVM_BIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/opt/homebrew/opt/llvm/bin".into())
        .join("llvm-mc");
    let mut child = Command::new(llvm)
        .args([
            "-triple=amdgcn-amd-amdhsa",
            "-mcpu=gfx1030",
            "-show-encoding",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let assembly = rows
        .iter()
        .map(|(line, _)| format!("{line}\n"))
        .collect::<String>();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(assembly.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    let encodings: Vec<Vec<u8>> = text
        .lines()
        .filter_map(|line| line.split_once("; encoding: ["))
        .map(|(_, tail)| {
            tail.trim_end_matches(']')
                .split(',')
                .map(|v| u8::from_str_radix(v.trim().trim_start_matches("0x"), 16).unwrap())
                .collect()
        })
        .collect();
    assert_eq!(encodings.len(), rows.len());
    for (actual, (line, expected)) in encodings.iter().zip(rows) {
        assert_eq!(actual, &expected, "{line}");
    }
}

#[test]
fn position_specific_literal_boundaries_and_full_accumulator() {
    for op in OPS {
        for literal in [0, 0x7fff, 0x8000, 0xffff, 0x10000, 0xffffffff] {
            for mask in 1..8 {
                let mut raw = [257, 258, 259];
                for (i, source) in raw.iter_mut().enumerate() {
                    if mask & (1 << i) != 0 {
                        *source = 255;
                    }
                }
                let result = decode_at(Gfx1030, &code(op, 5, raw, literal), 0);
                assert!(result.is_ok());
            }
        }
    }
}
