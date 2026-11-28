// SPDX-License-Identifier: GPL-3.0-or-later
use xenora_shader::{Architecture::Gfx1030, compile_graphics_pipeline};
#[test]
fn float_pixel_moves_keep_existing_provenance_and_lowering() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let original = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let mut e64: Vec<u8> = [
        0xd5810008u32,
        242 | 128 << 9 | 128 << 18,
        0xd5810009,
        128 | 128 << 9 | 128 << 18,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect();
    e64.extend_from_slice(&original[8..]);
    let a = compile_graphics_pipeline(Gfx1030, vs, original).unwrap();
    let b = compile_graphics_pipeline(Gfx1030, vs, &e64).unwrap();
    assert_eq!(a.shader.msl, b.shader.msl);
}
