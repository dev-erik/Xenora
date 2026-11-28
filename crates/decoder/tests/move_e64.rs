// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, Instruction, decode};
fn code(dst: u32, source: u32, literal: u32) -> Vec<u8> {
    let mut words = vec![0xd5810000 | dst, source | 128 << 9 | 128 << 18];
    if source == 255 {
        words.push(literal);
    }
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn selectors_register_boundaries_and_lengths() {
    for dst in [0, 1, 255] {
        for source in 0..512 {
            let bytes = code(dst, source, 0x7fa12345);
            let result = decode(Gfx1030, &bytes);
            let accepted = matches!(source,0..=107|126..=208|240..=248|255..=511);
            assert_eq!(result.is_ok(), accepted, "src={source}");
            if let Ok(result) = result {
                assert_eq!(result.len(), 1);
                assert_eq!(usize::from(result[0].byte_len), bytes.len());
                assert!(
                    matches!(result[0].instruction,Instruction::MoveB32{dst:d,..} if u32::from(d)==dst)
                );
                for cut in 1..bytes.len() {
                    assert!(decode(Gfx1030, &bytes[..cut]).is_err());
                }
            }
        }
    }
}
#[test]
fn modifiers_and_unused_operands_are_rejected() {
    for bit in (8..16).chain(27..32) {
        let mut bytes = code(2, 257, 0);
        let index = if bit >= 27 { 4 } else { 0 };
        let word = u32::from_le_bytes(bytes[index..index + 4].try_into().unwrap()) | 1 << bit;
        bytes[index..index + 4].copy_from_slice(&word.to_le_bytes());
        assert!(decode(Gfx1030, &bytes).is_err());
    }
    for shift in [9, 18] {
        for unused in [0, 129, 255, 511] {
            let mut bytes = code(2, 257, 0);
            let word = (257 | 128 << 9 | 128 << 18) & !(511 << shift) | unused << shift;
            bytes[4..8].copy_from_slice(&u32::to_le_bytes(word));
            assert!(decode(Gfx1030, &bytes).is_err());
        }
    }
}
#[test]
#[ignore = "requires native LLVM"]
fn exact_llvm_move_encodings() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let rows = [
        ("v_mov_b32_e64 v2, v1", 2, 257, 0),
        ("v_mov_b32_e64 v255, s8", 255, 8, 0),
        ("v_mov_b32_e64 v3, 0x7fa12345", 3, 255, 0x7fa12345),
        ("v_mov_b32_e64 v3, -1", 3, 193, 0),
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
