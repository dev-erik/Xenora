// SPDX-License-Identifier: GPL-3.0-or-later
//! Compiler admission, wrapping lowering, initialization and condition-state checks.
use xenora_shader::{Architecture::Gfx1030, CompileError, compile};

fn program(
    op: u32,
    dst: u32,
    sources: [u32; 2],
    literal: u32,
    prefix: &[u32],
    suffix: &[u32],
) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(prefix);
    words.extend([
        0xd4000000 | op << 16 | dst,
        sources[0] | sources[1] << 9 | 128 << 18,
    ]);
    if sources.contains(&255) {
        words.push(literal);
    }
    words.extend(suffix);
    words.extend([
        0xbefe03c1,
        0xe0702000,
        0x80010000 | dst << 8,
        0xbbfd0000,
        0xbf810000,
    ]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn wrapping_unsigned_lowering_preserves_source_order_and_aliases() {
    for sources in [[257, 255], [255, 257], [255, 255], [257, 193]] {
        for literal in [0, 1, 0x80000000, 0x7fffffff, 0xffffffff] {
            for dst in [1, 2, 255] {
                let signed =
                    compile(Gfx1030, &program(895, dst, sources, literal, &[], &[])).unwrap();
                let unsigned =
                    compile(Gfx1030, &program(293, dst, sources, literal, &[], &[])).unwrap();
                assert!(signed.ir.contains("add.u32"));
                assert_eq!(signed.ir, unsigned.ir);
                assert_eq!(signed.msl, unsigned.msl);
            }
        }
    }
}

#[test]
fn source_initialization_and_partial_exec_retention() {
    for position in 0..2 {
        for source in [257, 128, 193, 240, 255, 8, 126] {
            let mut sources = [257; 2];
            sources[position] = source;
            for mask in [0xbefe0380, 0xbefe0381, 0xbefe03c1] {
                compile(
                    Gfx1030,
                    &program(895, 1, sources, 0x80000000, &[0xbe880381, mask], &[]),
                )
                .unwrap();
            }
        }
        for source in [259, 20, 106, 107, 127] {
            let mut sources = [257; 2];
            sources[position] = source;
            assert!(matches!(
                compile(Gfx1030, &program(895, 2, sources, 0, &[], &[])),
                Err(CompileError::Verify { .. })
            ));
        }
    }
    assert!(compile(Gfx1030, &program(895, 2, [257; 2], 0, &[0xbefe0381], &[])).is_err());
    assert!(compile(Gfx1030, &program(895, 0, [257; 2], 0, &[], &[])).is_err());
}

#[test]
fn condition_flags_remain_uninitialized_or_preserve_both_values() {
    for branch in [0xbf840000, 0xbf860000] {
        assert!(matches!(
            compile(Gfx1030, &program(895, 2, [257; 2], 0, &[], &[branch])),
            Err(CompileError::Verify { .. })
        ));
    }
    for flag in [false, true] {
        // A changed SCC reaches a forbidden v0 write.
        let suffix = if flag {
            vec![0xbf840001, 0xbf820001, 0x7e000280]
        } else {
            vec![0xbf840001, 0x7e000280]
        };
        compile(
            Gfx1030,
            &program(
                895,
                2,
                [257, 193],
                0,
                &[if flag { 0xbf068080 } else { 0xbf068180 }],
                &suffix,
            ),
        )
        .unwrap();
        // VCCZ: preserve explicit zero/nonzero VCC_LO across subtraction.
        let suffix = if flag {
            vec![0xbf860001, 0xbf820001, 0x7e000280]
        } else {
            vec![0xbf860001, 0x7e000280]
        };
        compile(
            Gfx1030,
            &program(
                895,
                2,
                [257, 193],
                0,
                &[if flag { 0xbeea0381 } else { 0xbeea0380 }],
                &suffix,
            ),
        )
        .unwrap();
    }
}
