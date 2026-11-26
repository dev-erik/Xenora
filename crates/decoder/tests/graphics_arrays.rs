// SPDX-License-Identifier: GPL-3.0-or-later
//! Original literal gfx1030 bytes; AMD RDNA2 Tables 41, 43 and 100/101.
//! Checked with native ARM64 LLVM 23.1.6 llvm-mc -show-encoding.
use xenora_decoder::{Architecture::Gfx1030, Instruction, decode};

const ARRAY: [u8; 8] = [0x28, 0x0f, 0x9c, 0xf0, 0x02, 0x08, 0x40, 0x00];

#[test]
fn array_dimension_has_three_float_coordinates_without_legacy_da() {
    let decoded = decode(Gfx1030, &ARRAY).unwrap();
    assert_eq!(
        decoded[0].instruction,
        Instruction::ImageSampleLzArray {
            dst: 8,
            coord: 2,
            resource: 0,
            sampler: 8,
        }
    );
    assert_eq!(decoded[0].byte_len, 8);
    assert!(decoded[0].to_string().contains("v[2:4]"));
    assert!(decoded[0].to_string().contains("dim:SQ_RSRC_IMG_2D_ARRAY"));
    let mut two_d = ARRAY;
    two_d[0] = 0x08;
    assert!(matches!(
        decode(Gfx1030, &two_d).unwrap()[0].instruction,
        Instruction::ImageSampleLz { coord: 2, .. }
    ));
}

#[test]
fn array_rejects_each_unsupported_modifier_dimension_and_truncation() {
    for bit in [
        1, 2, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 25, 58, 59, 60, 61, 62, 63,
    ] {
        let mut bytes = ARRAY;
        bytes[bit / 8] ^= 1 << (bit % 8);
        assert!(decode(Gfx1030, &bytes).is_err(), "bit {bit}");
    }
    for dimension in [0, 2, 3, 4, 6, 7] {
        let mut bytes = ARRAY;
        bytes[0] = dimension << 3;
        assert!(decode(Gfx1030, &bytes).is_err());
    }
    for length in 1..8 {
        assert!(decode(Gfx1030, &ARRAY[..length]).is_err());
    }
}

#[test]
fn array_register_spans_fit_without_wrapping() {
    let mut bytes = ARRAY;
    bytes[4] = 253;
    bytes[5] = 252;
    assert!(decode(Gfx1030, &bytes).is_ok());
    for coord in [254, 255] {
        bytes[4] = coord;
        assert!(decode(Gfx1030, &bytes).is_err());
    }
    bytes[4] = 253;
    for dst in [253, 254, 255] {
        bytes[5] = dst;
        assert!(decode(Gfx1030, &bytes).is_err());
    }
}
