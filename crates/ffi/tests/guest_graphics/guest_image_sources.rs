// SPDX-License-Identifier: GPL-3.0-or-later
//! Original guest stores and transfer descriptors; literal packed-byte expectations.
use super::*;

// Modes: legacy RGBA8, mip RGBA8, array RGBA8, typed uint, typed depth.
fn image_texture(g: &mut GuestScript, device: u64, mode: u8) -> u64 {
    match mode {
        0 => g.handle(6, &[H(device), I(4), I(4), I(1), I(COPY_DEST)]),
        1 => g.handle(6, &[H(device), I(8), I(8), I(3), I(COPY_DEST)]),
        2 => g.handle(51, &[H(device), I(8), I(8), I(3), I(2), I(COPY_DEST)]),
        _ => g.handle(
            53,
            &[
                H(device),
                I(8),
                I(8),
                I(3),
                I(2),
                I(if mode == 3 { 8 } else { 9 }),
                I(COPY_DEST),
            ],
        ),
    }
}
fn image_record(g: &mut GuestScript, mode: u8, l: u64, t: u64, s: u64, origin: u64) {
    if mode == 0 {
        g.ok(
            44,
            &[
                H(l),
                H(t),
                I(0),
                I(origin),
                I(origin),
                H(s),
                I(1),
                I(4),
                I(16),
                I(2),
                I(2),
            ],
        );
    } else {
        g.ok(
            if mode < 3 { 50 } else { 54 },
            &[
                H(l),
                H(t),
                I(0),
                I(if mode == 1 { 1 } else { (1 << 32) | 1 }),
                I(origin),
                I(origin),
                H(s),
                I(1),
                I(0),
                I(4),
                I(16),
                I(2),
                I(2),
            ],
        );
    }
}
fn source_bytes() -> Vec<u8> {
    let mut bytes = vec![0x6d; 41];
    bytes[4..12].copy_from_slice(&[1, 0, 0, 1, 255, 255, 255, 255]);
    bytes[20..28].copy_from_slice(&[0, 0, 0, 128, 69, 35, 193, 127]);
    bytes[40] = 0xe7;
    bytes
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_image_sources_pitched_snapshot_mips_slices_uint_depth_and_retirement() {
    guest_destinations_still_reject();
    for mode in 0..5 {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let q = g.handle(2, &[H(d)]);
        let a = g.handle(3, &[H(d)]);
        let l = g.handle(4, &[H(d), H(a)]);
        let mut initial = vec![0xa5; 8];
        initial.extend([0x11; 41]);
        initial.extend([0xa5; 8]);
        let backing = g.bytes(&initial);
        let s = g.handle(49, &[H(d), I(41), I(COPY_SOURCE), I(backing + 8)]);
        let t = image_texture(&mut g, d, mode);
        let size = match mode {
            0 => 64,
            1 => 336,
            _ => 672,
        };
        let prior = vec![0x93; size];
        let upload = g.bytes(&prior);
        g.ok(9, &[H(t), I(0), I(upload), I(size as u64)]);
        image_record(&mut g, mode, l, t, s, 1);
        g.barrier(l, t, COPY_DEST, COPY_SOURCE);
        g.ok(12, &[H(l)]);
        guest_runtime_write_bytes(&mut g, backing + 8, &source_bytes());
        let receipt = g.handle(35, &[H(q), H(l), I(0), I(0)]);
        guest_runtime_write_bytes(&mut g, backing + 8, &[0xee; 41]);
        g.ok(11, &[H(l), H(a)]);
        g.ok(26, &[H(s)]);
        g.call(32, &[H(receipt)], 0, Some(1));
        g.ok(33, &[H(receipt)]);
        let output = g.bytes(&vec![0xa5; size + 16]);
        g.ok(10, &[H(t), I(0), I(output + 8), I(size as u64)]);
        // 8x8 + 4x4 + 2x2 is 336 bytes per slice; selected mip starts at256.
        let base = match mode {
            0 => 0,
            1 => 256,
            _ => 592,
        };
        let mut expected = prior;
        expected[base + 20..base + 28].copy_from_slice(&[1, 0, 0, 1, 255, 255, 255, 255]);
        expected[base + 36..base + 44].copy_from_slice(&[0, 0, 0, 128, 69, 35, 193, 127]);
        check_original_guest_backend(
            g,
            true,
            true,
            &[(output, expected), (backing, vec![0xee; 41])],
            0,
            &[],
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_image_sources_exact_produced_destination_preserves_other_texels_and_subresources()
{
    for mode in 0..5 {
        for producer_first in [false, true] {
            let mut g = GuestScript::new();
            let d = g.handle(1, &[]);
            let q = [0, 1].map(|_| g.handle(2, &[H(d)]));
            let a = [0, 1].map(|_| g.handle(3, &[H(d)]));
            let l = a.map(|a| g.handle(4, &[H(d), H(a)]));
            let backing = g.bytes(&[0x42; 41]);
            let s = g.handle(49, &[H(d), I(41), I(COPY_SOURCE), I(backing)]);
            let t = image_texture(&mut g, d, mode);
            let size = match mode {
                0 => 64,
                1 => 336,
                _ => 672,
            };
            let prior = vec![0x93; size];
            let upload = g.bytes(&prior);
            g.ok(9, &[H(t), I(0), I(upload), I(size as u64)]);
            image_record(&mut g, mode, l[0], t, s, 0);
            g.ok(12, &[H(l[0])]);
            image_record(&mut g, mode, l[1], t, s, 1);
            g.barrier(l[1], t, COPY_DEST, COPY_SOURCE);
            g.ok(12, &[H(l[1])]);
            let producer = g.handle(35, &[H(q[0]), H(l[0]), I(0), I(0)]);
            g.call(35, &[H(q[1]), H(l[1]), I(0), I(0)], 2, Some(u64::MAX));
            guest_runtime_write_bytes(&mut g, backing, &source_bytes());
            let wait = guest_geometry_wait_record(&mut g, q[0], producer);
            let consumer = g.handle(35, &[H(q[1]), H(l[1]), I(wait), I(1)]);
            guest_runtime_write_bytes(&mut g, backing, &[0xee; 41]);
            for i in 0..2 {
                g.ok(11, &[H(l[i]), H(a[i])]);
            }
            g.ok(26, &[H(s)]);
            for r in if producer_first {
                [producer, consumer]
            } else {
                [consumer, producer]
            } {
                g.call(32, &[H(r)], 0, Some(1));
            }
            g.ok(33, &[H(consumer)]);
            g.ok(33, &[H(producer)]);
            let base = match mode {
                0 => 0,
                1 => 256,
                _ => 592,
            };
            let mut expected = prior;
            // Producer's non-overlapping top row and left texel must survive the patch.
            expected[base..base + 8].fill(0x42);
            expected[base + 16..base + 24].fill(0x42);
            expected[base + 20..base + 28].copy_from_slice(&[1, 0, 0, 1, 255, 255, 255, 255]);
            expected[base + 36..base + 44].copy_from_slice(&[0, 0, 0, 128, 69, 35, 193, 127]);
            let output = g.bytes(&vec![0xa5; size + 16]);
            g.ok(10, &[H(t), I(0), I(output + 8), I(size as u64)]);
            check_original_guest_backend(g, true, true, &[(output, expected)], 0, &[]);
        }
    }
}

fn guest_destinations_still_reject() {
    for mode in 0..5 {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let a = g.handle(3, &[H(d)]);
        let l = g.handle(4, &[H(d), H(a)]);
        let address = g.bytes(&[0x5a; 41]);
        let dest = g.handle(49, &[H(d), I(41), I(COPY_DEST), I(address)]);
        let source = image_texture(&mut g, d, mode);
        if mode == 0 {
            g.call(
                44,
                &[
                    H(l),
                    H(dest),
                    I(1),
                    I(4),
                    I(16),
                    H(source),
                    I(0),
                    I(1),
                    I(1),
                    I(2),
                    I(2),
                ],
                2,
                Some(u64::MAX),
            );
        } else {
            g.call(
                if mode < 3 { 50 } else { 54 },
                &[
                    H(l),
                    H(dest),
                    I(1),
                    I(0),
                    I(4),
                    I(16),
                    H(source),
                    I(0),
                    I(if mode == 1 { 1 } else { (1 << 32) | 1 }),
                    I(1),
                    I(1),
                    I(2),
                    I(2),
                ],
                2,
                Some(u64::MAX),
            );
        }
        g.ok(26, &[H(dest)]);
        g.ok(26, &[H(source)]);
        check_original_guest_backend(g, true, true, &[], 0, &[]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_image_sources_bc_runtime_capability_and_literal_blocks() {
    let gpu = xenora_metal::NativeCompute::new().expect("native Metal device");
    let formats = gpu.typed_copy_capabilities();
    drop(gpu);
    for (format, unit, pattern) in [
        (1u64, 8usize, vec![0, 248, 224, 7, 228, 27, 177, 78]),
        (
            2,
            16,
            vec![
                231, 17, 136, 198, 250, 136, 198, 250, 31, 0, 0, 248, 228, 27, 177, 78,
            ],
        ),
    ] {
        let supported = formats & (1 << format) != 0;
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let q = g.handle(2, &[H(d)]);
        let a = g.handle(3, &[H(d)]);
        let l = g.handle(4, &[H(d), H(a)]);
        let mut bytes = vec![0x11; unit * 3 + 1];
        bytes[unit..unit * 2].copy_from_slice(&pattern);
        let backing = g.bytes(&bytes);
        let s = g.handle(
            49,
            &[H(d), I(bytes.len() as u64), I(COPY_SOURCE), I(backing)],
        );
        // Single-block mip1 tail in slice1, with an uncopied odd source tail.
        // This checks exact BC bits and full-backing preservation, not multirow BC pitch.
        // 5x3: two blocks at mip0, one each at mip1 and mip2, two slices.
        let size = 8 * unit;
        let t = g.handle(53, &[H(d), I(5), I(3), I(3), I(2), I(format), I(COPY_DEST)]);
        let prior = vec![0x39; size];
        let upload = g.bytes(&prior);
        g.ok(9, &[H(t), I(0), I(upload), I(size as u64)]);
        g.ok(
            54,
            &[
                H(l),
                H(t),
                I(0),
                I((1 << 32) | 1),
                I(0),
                I(0),
                H(s),
                I(1),
                I(0),
                I(unit as u64),
                I((2 * unit) as u64),
                I(2),
                I(1),
            ],
        );
        g.barrier(l, t, COPY_DEST, COPY_SOURCE);
        g.ok(12, &[H(l)]);
        if supported {
            let r = g.handle(35, &[H(q), H(l), I(0), I(0)]);
            guest_runtime_write_bytes(&mut g, backing, &vec![0xee; bytes.len()]);
            g.ok(11, &[H(l), H(a)]);
            g.ok(26, &[H(s)]);
            g.call(32, &[H(r)], 0, Some(1));
            g.ok(33, &[H(r)]);
            let output = g.bytes(&vec![0xa5; size + 16]);
            g.ok(10, &[H(t), I(0), I(output + 8), I(size as u64)]);
            let mut expected = prior;
            expected[6 * unit..7 * unit].copy_from_slice(&pattern);
            check_original_guest_backend(g, true, true, &[(output, expected)], 0, &[]);
        } else {
            eprintln!(
                "BC format {format}: asserting runtime capability rejection, not native BC copy evidence"
            );
            g.call(35, &[H(q), H(l), I(0), I(0)], 2, Some(u64::MAX));
            check_original_guest_backend(g, true, true, &[], 0, &[]);
        }
    }
}
