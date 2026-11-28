// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, Instruction, decode};
fn code(dst: u32, source: u32, literal: u32) -> Vec<u8> {
    let mut words = vec![0xd54f0000 | dst, source | 258 << 9 | 259 << 18];
    if source == 255 {
        words.push(literal);
    }
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn lengths_and_truncation() {
    for source in [8, 255, 257, 511] {
        for dst in [0, 1, 255] {
            let bytes = code(dst, source, 0x12345678);
            let decoded = decode(Gfx1030, &bytes).unwrap();
            assert_eq!(usize::from(decoded[0].byte_len), bytes.len());
            assert!(matches!(
                decoded[0].instruction,
                Instruction::VectorTernary {
                    op: xenora_decoder::VectorTernaryOp::AlignbyteB32,
                    ..
                }
            ));
            for cut in 1..bytes.len() {
                assert!(decode(Gfx1030, &bytes[..cut]).is_err());
            }
        }
    }
}
#[test]
#[ignore = "requires native LLVM"]
fn exact_llvm_alignbyte_encodings() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let rows = [
        ("v_alignbyte_b32 v2, v1, v2, v3", 2, 257, 0),
        ("v_alignbyte_b32 v255, s8, v2, v3", 255, 8, 0),
        ("v_alignbyte_b32 v3, 0x12345678, v2, v3", 3, 255, 0x12345678),
        ("v_alignbyte_b32 v3, -1, v2, v3", 3, 193, 0),
    ];
    let llvm_bin =
        std::env::var("LLVM_BIN").unwrap_or_else(|_| "/opt/homebrew/opt/llvm/bin".into());
    let mut child = Command::new(std::path::Path::new(&llvm_bin).join("llvm-mc"))
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
    let text = rows
        .iter()
        .map(|r| format!("{}\n", r.0))
        .collect::<String>();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let actual: Vec<Vec<u8>> = text
        .lines()
        .filter_map(|line| line.split_once("; encoding: ["))
        .map(|(_, tail)| {
            tail.trim_end_matches(']')
                .split(',')
                .map(|v| u8::from_str_radix(v.trim().trim_start_matches("0x"), 16).unwrap())
                .collect()
        })
        .collect();
    assert_eq!(actual.len(), rows.len());
    for (bytes, (_, dst, source, literal)) in actual.iter().zip(rows) {
        assert_eq!(bytes, &code(dst, source, literal));
    }
}

#[test]
fn each_source_selector_and_shared_literal() {
    for position in 0..3 {
        for selector in 0..512u32 {
            let mut sources = [257, 258, 259];
            sources[position] = selector;
            let mut words = vec![0xd54f00ff, sources[0] | sources[1] << 9 | sources[2] << 18];
            if selector == 255 {
                words.push(0xabfe7e01);
            }
            let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
            assert_eq!(
                decode(Gfx1030, &bytes).is_ok(),
                matches!(selector, 0..=107 | 126..=208 | 240..=248 | 255..=511),
                "position={position} selector={selector}"
            );
        }
    }
    let words = [0xd54f00ffu32, 255 | 255 << 9 | 255 << 18, 0x12345678];
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    assert_eq!(decode(Gfx1030, &bytes).unwrap()[0].byte_len, 12);
    let words = [0xd54f00ffu32, 8 | 9 << 9 | 10 << 18];
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    assert!(
        decode(Gfx1030, &bytes).is_err(),
        "three distinct scalar inputs exceed the checked contract"
    );
}
