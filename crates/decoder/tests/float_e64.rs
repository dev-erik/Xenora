// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, decode};
fn code(op: u32, dst: u32, a: u32, b: u32) -> Vec<u8> {
    [0xd4000000 | op << 16 | dst, a | b << 9 | 128 << 18]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect()
}
#[test]
fn source_domains_modifiers_and_lengths() {
    for op in [259, 264] {
        for dst in [0, 1, 255] {
            for position in 0..2 {
                for source in 0..512 {
                    let mut src = [257, 258];
                    src[position] = source;
                    let bytes = code(op, dst, src[0], src[1]);
                    assert_eq!(decode(Gfx1030, &bytes).is_ok(), source >= 256);
                }
            }
        }
        let bytes = code(op, 255, 511, 511);
        assert_eq!(decode(Gfx1030, &bytes).unwrap()[0].byte_len, 8);
        for cut in 1..8 {
            assert!(decode(Gfx1030, &bytes[..cut]).is_err());
        }
        for bit in 8..16 {
            let mut bytes = code(op, 1, 257, 258);
            let first = u32::from_le_bytes(bytes[..4].try_into().unwrap()) | 1 << bit;
            bytes[..4].copy_from_slice(&first.to_le_bytes());
            assert!(decode(Gfx1030, &bytes).is_err());
        }
        for tail in [
            257 | 258 << 9,
            257 | 258 << 9 | 129 << 18,
            257 | 258 << 9 | 128 << 18 | 1 << 27,
            257 | 258 << 9 | 128 << 18 | 1 << 31,
        ] {
            let mut bytes = code(op, 1, 257, 258);
            bytes[4..].copy_from_slice(&u32::to_le_bytes(tail));
            assert!(decode(Gfx1030, &bytes).is_err());
        }
    }
}
#[test]
#[ignore = "requires native LLVM"]
fn exact_llvm_float_encodings() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let rows = [
        ("v_add_f32_e64 v2, v1, v2", 259, 2, 257, 258),
        ("v_mul_f32_e64 v255, v255, v1", 264, 255, 511, 257),
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
    for (bytes, (_, op, dst, a, b)) in actual.iter().zip(rows) {
        assert_eq!(bytes, &code(op, dst, a, b));
    }
}
