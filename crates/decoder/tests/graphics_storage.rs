// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal gfx1030 encodings checked with native ARM64 LLVM 23.1.6.
//! Public AMD RDNA2 Tables 41/42 and 100/101; no emulator-derived fixtures.
use xenora_decoder::{Architecture::Gfx1030, Instruction, decode};
const RGBA: [u8; 8] = [0x08, 0x1f, 0x20, 0xf0, 0x20, 0x08, 0x00, 0x00];
const UINT: [u8; 8] = [0x08, 0x11, 0x20, 0xf0, 0x20, 0x08, 0x00, 0x00];

#[test]
fn plain_store_decodes_unsigned_xy_and_source_data_without_a_sampler() {
    for (bytes, dmask) in [(RGBA, 15), (UINT, 1)] {
        let decoded = decode(Gfx1030, &bytes).unwrap();
        assert_eq!(
            decoded[0].instruction,
            Instruction::ImageStore2D {
                data: 8,
                coord: 32,
                resource: 0,
                dmask,
            }
        );
        assert_eq!(decoded[0].byte_len, 8);
        assert!(decoded[0].to_string().contains("image_store"));
        assert!(decoded[0].to_string().contains("v[32:33]"));
        assert!(decoded[0].to_string().contains("dim:SQ_RSRC_IMG_2D unorm"));
    }
}

#[test]
fn store_rejects_other_dimensions_masks_modifiers_sampler_and_short_streams() {
    for bit in [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 25, 53, 54, 55, 56, 57, 58, 59,
        60, 61, 62, 63,
    ] {
        let mut bytes = RGBA;
        bytes[bit / 8] ^= 1 << (bit % 8);
        assert!(decode(Gfx1030, &bytes).is_err(), "bit {bit}");
    }
    for mask in 0..16 {
        let mut bytes = RGBA;
        bytes[1] = 0x10 | mask;
        assert_eq!(decode(Gfx1030, &bytes).is_ok(), mask == 1 || mask == 15);
    }
    for len in 1..8 {
        assert!(decode(Gfx1030, &RGBA[..len]).is_err());
    }
}

#[test]
fn store_source_spans_never_wrap_registers() {
    let mut bytes = RGBA;
    bytes[4] = 254;
    bytes[5] = 252;
    assert!(decode(Gfx1030, &bytes).is_ok());
    for data in [253, 254, 255] {
        bytes[5] = data;
        assert!(decode(Gfx1030, &bytes).is_err());
    }
    let mut bytes = UINT;
    bytes[4] = 254;
    bytes[5] = 255;
    assert!(decode(Gfx1030, &bytes).is_ok());
    bytes[4] = 255;
    assert!(decode(Gfx1030, &bytes).is_err());
    bytes = RGBA;
    bytes[6] = 25; // s100 cannot hold an eight-word descriptor.
    assert!(decode(Gfx1030, &bytes).is_err());
}
