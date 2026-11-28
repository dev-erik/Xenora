// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_decoder::{Architecture::Gfx1030, decode};
const OPS: &[(u32, &str)] = &[(0, "v_pk_mad_i16"), (9, "v_pk_mad_u16")];
fn code(op: u32, dst: u32, a: u32, b: u32, c: u32) -> Vec<u8> {
    [
        0xcc004000 | op << 16 | dst,
        0x18000000 | a | b << 9 | c << 18,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
#[test]
fn source_domains_lengths_and_modifiers() {
    for &(op, _) in OPS {
        for dst in [0, 5, 255] {
            for position in 0..3 {
                for source in 0..512 {
                    let mut src = [257, 258, 259];
                    src[position] = source;
                    assert_eq!(
                        decode(Gfx1030, &code(op, dst, src[0], src[1], src[2])).is_ok(),
                        source >= 256
                    );
                }
            }
        }
        let bytes = code(op, 255, 511, 511, 511);
        assert_eq!(decode(Gfx1030, &bytes).unwrap()[0].byte_len, 8);
        for cut in 1..8 {
            assert!(decode(Gfx1030, &bytes[..cut]).is_err());
        }
        for bit in (8..16).chain(59..64) {
            let mut changed = bytes.clone();
            let index = if bit < 32 { 0 } else { 4 };
            let w = u32::from_le_bytes(changed[index..index + 4].try_into().unwrap())
                ^ (1 << (bit % 32));
            changed[index..index + 4].copy_from_slice(&w.to_le_bytes());
            assert!(decode(Gfx1030, &changed).is_err(), "op={op} bit={bit}");
        }
    }
    for op in [14, 15, 127] {
        assert!(decode(Gfx1030, &code(op, 5, 257, 258, 259)).is_err());
    }
}
#[test]
#[ignore = "requires native LLVM"]
fn exact_llvm_packed_encodings() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let bin = std::env::var("LLVM_BIN").unwrap_or_else(|_| "/opt/homebrew/opt/llvm/bin".into());
    let mut rows = Vec::new();
    for &(op, name) in OPS {
        for (d, a, b, c) in [(5, 1, 2, 3), (255, 255, 255, 255), (0, 0, 1, 2)] {
            rows.push((
                format!("{name} v{d}, v{a}, v{b}, v{c}"),
                code(op, d, a + 256, b + 256, c + 256),
            ));
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
