// SPDX-License-Identifier: GPL-3.0-or-later
//! Compiler admission tests; arithmetic outputs are independently checked elsewhere.
use xenora_shader::{Architecture::Gfx1030, CompileError, compile};
fn program(op: u32, dst: u32, sources: [u32; 3], prefix: &[u32]) -> Vec<u8> {
    let mut words = vec![0xe0302000, 0x80000100, 0xbf8c3f70];
    words.extend(prefix);
    words.extend([
        0xd4000000 | op << 16 | dst,
        sources[0] | sources[1] << 9 | sources[2] << 18,
    ]);
    if sources.contains(&255) {
        words.push(if sources[..2].contains(&255) {
            0xffff
        } else {
            0xffffffff
        });
    }
    words.push(0xbefe03c1);
    words.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
#[test]
fn sources_aliases_and_inactive_destinations() {
    for op in [883, 885] {
        for position in 0..3 {
            for source in [257, 128, 193, 240, 255, 8] {
                if position < 2 && source == 240 {
                    continue;
                }
                let mut src = [257; 3];
                src[position] = source;
                let code = program(op, 1, src, &[0xbe880381]);
                compile(Gfx1030, &code).unwrap();
                // Partial EXEC requires retention of the initialized old destination.
                compile(Gfx1030, &program(op, 1, src, &[0xbe880381, 0xbefe0381])).unwrap();
            }
            for source in [259, 20, 106, 107] {
                let mut src = [257; 3];
                src[position] = source;
                assert!(matches!(
                    compile(Gfx1030, &program(op, 2, src, &[])),
                    Err(CompileError::Verify { .. })
                ));
            }
        }
        assert!(compile(Gfx1030, &program(op, 2, [257; 3], &[0xbefe0381])).is_err());
        assert!(compile(Gfx1030, &program(op, 0, [257; 3], &[])).is_err());
    }
}
