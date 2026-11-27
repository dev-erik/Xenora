// SPDX-License-Identifier: GPL-3.0-or-later

use xenora_shader::{Architecture, CompileError, compile};

const WAVE32_CONTROL: &[u8] = include_bytes!("../../../fixtures/rdna2/add.bin");

#[test]
fn upper_exec_write_does_not_change_wave32_lowering() {
    // s_mov_b32 exec_hi, 1. Wave width is part of the compute ABI.
    let mut stream = vec![0x81, 0x03, 0xff, 0xbe];
    stream.extend_from_slice(WAVE32_CONTROL);
    let with_upper_write = compile(Architecture::Gfx1030, &stream).unwrap();
    let control = compile(Architecture::Gfx1030, WAVE32_CONTROL).unwrap();
    assert_eq!(with_upper_write.msl, control.msl);
}

#[test]
fn unqualified_cross_lane_permute_requires_explicit_wave32_launch() {
    assert!(compile(Architecture::Gfx1030, WAVE32_CONTROL).is_ok());

    // ds_bpermute_b32 v2, v0, v1 assembled for gfx1030 with
    // +wavefrontsize64. These instruction bytes also occur in Wave32; the
    // assembler flag supplies no launch metadata in this raw stream. Decoding
    // is now supported, but the default ABI must still reject its execution.
    let mut stream = vec![0, 0, 0xcc, 0xda, 0, 1, 0, 2];
    stream.extend_from_slice(WAVE32_CONTROL);
    assert!(matches!(
        compile(Architecture::Gfx1030, &stream),
        Err(CompileError::Verify {
            offset: 0,
            reason: "gathers require full Wave32 launch"
        })
    ));
}
