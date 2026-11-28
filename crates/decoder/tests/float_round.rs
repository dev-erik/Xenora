// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, decode};
fn code(byte: u32, wide: bool, dst: u32, source: u32) -> Vec<u8> {
    let words = if wide {
        vec![0xd4000000 | (384 + byte) << 16 | dst, 0x02010000 | source]
    } else {
        vec![0x7e000000 | dst << 17 | byte << 9 | source]
    };
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn source_domains_registers_lengths_and_modifiers() {
    for byte in [14, 33, 34, 35, 36] {
        for wide in [false, true] {
            for dst in [0, 127, 128, 255] {
                for source in 0..512 {
                    assert_eq!(
                        decode(Gfx1030, &code(byte, wide, dst, source)).is_ok(),
                        source >= 256
                    );
                }
                let bytes = code(byte, wide, dst, 511);
                for cut in 1..bytes.len() {
                    assert!(decode(Gfx1030, &bytes[..cut]).is_err());
                }
                if wide {
                    for bit in (8..16).chain(41..64) {
                        let mut changed = bytes.clone();
                        changed[bit / 8] ^= 1 << (bit % 8);
                        assert!(decode(Gfx1030, &changed).is_err());
                    }
                } else {
                    let mut changed = bytes.clone();
                    changed[2] ^= 1;
                    assert!(decode(Gfx1030, &changed).is_err());
                }
            }
        }
    }
}
#[test]
#[ignore = "requires native LLVM"]
fn exact_llvm_round_encodings() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let bin = std::env::var("LLVM_BIN").unwrap_or_else(|_| "/opt/homebrew/opt/llvm/bin".into());
    let mut rows = Vec::new();
    for byte in [14, 33, 34, 35, 36] {
        for wide in [false, true] {
            for (dst, src) in [(0, 0), (127, 128), (128, 127), (255, 255)] {
                rows.push((
                    format!(
                        "{}_{} v{dst}, v{src}",
                        match byte {
                            14 => "v_cvt_off_f32_i4",
                            33 => "v_trunc_f32",
                            34 => "v_ceil_f32",
                            35 => "v_rndne_f32",
                            _ => "v_floor_f32",
                        },
                        if wide { "e64" } else { "e32" }
                    ),
                    code(byte, wide, dst, src + 256),
                ));
            }
        }
    }
    let mut child = Command::new(std::path::Path::new(&bin).join("llvm-mc"))
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            rows.iter()
                .map(|r| format!("{}\n", r.0))
                .collect::<String>()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let actual: Vec<Vec<u8>> = text
        .lines()
        .filter_map(|l| l.split_once("; encoding: ["))
        .map(|(_, r)| {
            r.trim_end_matches(']')
                .split(',')
                .map(|x| u8::from_str_radix(x.trim().trim_start_matches("0x"), 16).unwrap())
                .collect()
        })
        .collect();
    assert_eq!(actual.len(), rows.len());
    for (got, (_, want)) in actual.iter().zip(rows) {
        assert_eq!(got, &want);
    }
}
