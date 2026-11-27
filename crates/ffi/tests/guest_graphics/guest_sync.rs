// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE guests exercising explicit full-root synchronization on native Metal.
use super::*;

fn sync_handles(g: &mut GuestScript, handles: &[u64]) -> u64 {
    let table = g.bytes(&vec![0; handles.len() * 8]);
    for (index, handle) in handles.iter().enumerate() {
        g.copy_result(*handle, table + index as u64 * 8);
    }
    table
}

fn sync_guarded(g: &mut GuestScript, bytes: &[u8]) -> u64 {
    let mut guarded = vec![0xa5; 8];
    guarded.extend_from_slice(bytes);
    guarded.extend_from_slice(&[0xa5; 8]);
    g.bytes(&guarded)
}

// Preserve intermediate guest bytes before later successful exports can overwrite
// evidence of an erroneous write. Include both original guards in the snapshot.
fn sync_snapshot(g: &mut GuestScript, address: u64, size: usize) -> u64 {
    let snapshot = g.bytes(&vec![0; size + 16]);
    for offset in (0..=size + 8).step_by(8) {
        g.copy_result(address + offset as u64, snapshot + offset as u64);
    }
    g.copy_result(address + size as u64 + 8, snapshot + size as u64 + 8);
    snapshot
}

fn sync_retire(g: &mut GuestScript, queue: u64, list: u64, allocator: u64) {
    g.ok(12, &[H(list)]);
    let receipt = g.handle(35, &[H(queue), H(list), I(0), I(0)]);
    g.call(32, &[H(receipt)], 0, Some(1));
    g.ok(33, &[H(receipt)]);
    g.ok(11, &[H(list), H(allocator)]);
}

// Legacy RGBA8, mip RGBA8, array RGBA8, array uint, array depth, odd buffer.
fn sync_root(g: &mut GuestScript, device: u64, mode: u8) -> u64 {
    match mode {
        0 | 1 => g.handle(
            6,
            &[
                H(device),
                I(4),
                I(4),
                I(if mode == 0 { 1 } else { 3 }),
                I(COPY_DEST),
            ],
        ),
        2 => g.handle(51, &[H(device), I(4), I(4), I(3), I(2), I(COPY_DEST)]),
        3 | 4 => g.handle(
            53,
            &[
                H(device),
                I(4),
                I(4),
                I(3),
                I(2),
                I(if mode == 3 { 8 } else { 9 }),
                I(COPY_DEST),
            ],
        ),
        5 => g.handle(5, &[H(device), I(37), I(COPY_DEST)]),
        _ => unreachable!(),
    }
}

fn sync_literals(mode: u8) -> Vec<u8> {
    if mode == 5 {
        return vec![0x39; 37];
    }
    // Six explicit subresource literals. Depth values are finite positive FP32;
    // uint includes high bits. Packing is slice-major, then 4x4, 2x2, 1x1.
    let words = match mode {
        3 => [
            [1, 0, 0, 128],
            [255, 255, 255, 255],
            [69, 35, 193, 127],
            [17, 34, 51, 68],
            [254, 255, 255, 255],
            [0, 0, 0, 128],
        ],
        4 => [
            [0, 0, 0, 62],
            [0, 0, 128, 62],
            [0, 0, 0, 63],
            [0, 0, 64, 63],
            [0, 0, 128, 63],
            [0, 0, 0, 0],
        ],
        _ => [
            [17, 31, 47, 255],
            [53, 71, 89, 107],
            [127, 149, 173, 199],
            [211, 19, 37, 59],
            [79, 101, 131, 157],
            [181, 193, 223, 239],
        ],
    };
    let counts = match mode {
        0 => &[16][..],
        1 => &[16, 4, 1][..],
        _ => &[16, 4, 1, 16, 4, 1][..],
    };
    let mut bytes = Vec::new();
    for (word, count) in words.iter().zip(counts) {
        bytes.extend(word.repeat(*count));
    }
    bytes
}

fn sync_patch(g: &mut GuestScript, list: u64, root: u64, source: u64, mode: u8) {
    match mode {
        0 => g.ok(
            44,
            &[
                H(list),
                H(root),
                I(0),
                I(1),
                I(0),
                H(source),
                I(1),
                I(0),
                I(4),
                I(1),
                I(2),
            ],
        ),
        1..=4 => g.ok(
            if mode < 3 { 50 } else { 54 },
            &[
                H(list),
                H(root),
                I(0),
                I(if mode == 1 { 1 } else { (1 << 32) | 1 }),
                I(1),
                I(0),
                H(source),
                I(1),
                I(0),
                I(0),
                I(4),
                I(1),
                I(2),
            ],
        ),
        5 => g.ok(43, &[H(list), H(root), I(7), H(source), I(0), I(8)]),
        _ => unreachable!(),
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sync_import_snapshot_partial_copy_mips_arrays_uint_depth() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let roots: Vec<_> = (0..6).map(|mode| sync_root(&mut g, d, mode)).collect();
    let mut backing = Vec::new();
    let mut expected = Vec::new();
    let mut reads = Vec::new();
    for (mode, root) in roots.iter().enumerate() {
        let bytes = sync_literals(mode as u8);
        let address = sync_guarded(&mut g, &bytes);
        g.ok(75, &[H(*root), I(address + 8)]);
        backing.push(address);
        expected.push(bytes);
    }
    let table = sync_handles(&mut g, &roots);
    g.ok(76, &[I(table), I(6), I(1)]);
    for (address, bytes) in backing.iter().zip(&expected) {
        guest_runtime_write_bytes(&mut g, address + 8, &vec![0xee; bytes.len()]);
    }
    // Exactly representable depth bits also exercise raw uint and RGBA bytes.
    let patch = [0, 0, 128, 62, 0, 0, 64, 63];
    let source = g.handle(5, &[H(d), I(8), I(COPY_DEST)]);
    let data = g.bytes(&patch);
    g.ok(9, &[H(source), I(0), I(data), I(8)]);
    g.barrier(l, source, COPY_DEST, COPY_SOURCE);
    for (mode, root) in roots.iter().enumerate() {
        sync_patch(&mut g, l, *root, source, mode as u8);
        g.barrier(l, *root, COPY_DEST, COPY_SOURCE);
        sync_retire(&mut g, q, l, a);
        // These offsets are independently counted in the packed root layout.
        let offsets: &[usize] = match mode {
            0 => &[4, 20],
            1 => &[68, 76],
            2..=4 => &[152, 160],
            _ => &[7, 11],
        };
        for (row, offset) in offsets.iter().enumerate() {
            expected[mode][*offset..*offset + 4].copy_from_slice(&patch[row * 4..row * 4 + 4]);
        }
    }
    for (address, bytes) in backing.iter().zip(&expected) {
        let snapshot = sync_snapshot(&mut g, *address, bytes.len());
        reads.push((snapshot, vec![0xee; bytes.len()]));
    }
    g.ok(76, &[I(table), I(6), I(0)]);
    for ((address, bytes), root) in backing.into_iter().zip(expected).zip(roots) {
        reads.push((address, bytes.clone()));
        let out = sync_guarded(&mut g, &vec![0; bytes.len()]);
        g.ok(10, &[H(root), I(0), I(out + 8), I(bytes.len() as u64)]);
        reads.push((out, bytes));
    }
    check_original_guest_backend(g, true, true, &reads, 0, &[]);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sync_pending_and_retained_receipt_export_write_nothing() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let src = g.handle(5, &[H(d), I(16), I(COPY_DEST)]);
    let dst = g.handle(5, &[H(d), I(16), I(COPY_DEST)]);
    let literal = [
        3, 17, 29, 43, 61, 79, 97, 113, 131, 151, 173, 193, 211, 229, 241, 251,
    ];
    let source = sync_guarded(&mut g, &literal);
    let dest = sync_guarded(&mut g, &[0x72; 16]);
    g.ok(75, &[H(src), I(source + 8)]);
    g.ok(75, &[H(dst), I(dest + 8)]);
    let input = sync_handles(&mut g, &[src]);
    let output = sync_handles(&mut g, &[dst]);
    g.ok(76, &[I(input), I(1), I(1)]);
    g.barrier(l, src, COPY_DEST, COPY_SOURCE);
    g.ok(42, &[H(l), H(dst), H(src)]);
    sync_retire(&mut g, q, l, a);
    // Publish COPY_SOURCE before the tested submission, so rejection cannot be
    // explained by an uncommitted barrier. Establish that idle export succeeds.
    g.ok(76, &[I(input), I(1), I(0)]);
    guest_runtime_write_bytes(&mut g, source + 8, &[0x5c; 16]);
    g.ok(42, &[H(l), H(dst), H(src)]);
    g.barrier(l, dst, COPY_DEST, COPY_SOURCE);
    g.ok(12, &[H(l)]);
    let r = g.handle(35, &[H(q), H(l), I(0), I(0)]);
    g.ok(11, &[H(l), H(a)]);
    g.call(76, &[I(output), I(1), I(0)], 2, Some(u64::MAX));
    let pending = sync_snapshot(&mut g, dest, 16);
    g.call(76, &[I(input), I(1), I(0)], 2, Some(u64::MAX));
    let pending_source = sync_snapshot(&mut g, source, 16);
    g.call(32, &[H(r)], 0, Some(1));
    g.call(76, &[I(output), I(1), I(0)], 2, Some(u64::MAX));
    let retained = sync_snapshot(&mut g, dest, 16);
    g.call(76, &[I(input), I(1), I(0)], 2, Some(u64::MAX));
    let retained_source = sync_snapshot(&mut g, source, 16);
    g.ok(33, &[H(r)]);
    g.ok(76, &[I(input), I(1), I(0)]);
    g.ok(76, &[I(output), I(1), I(0)]);
    check_original_guest_backend(
        g,
        true,
        true,
        &[
            (pending, vec![0x72; 16]),
            (retained, vec![0x72; 16]),
            (pending_source, vec![0x5c; 16]),
            (retained_source, vec![0x5c; 16]),
            (dest, literal.to_vec()),
            (source, literal.to_vec()),
        ],
        0,
        &[],
    );
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sync_bind_rebind_destroy_never_implicitly_transfer() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let mut reads = Vec::new();
    let scratch_source = g.handle(5, &[H(d), I(8), I(COPY_SOURCE)]);
    let scratch_dest = g.handle(5, &[H(d), I(8), I(COPY_DEST)]);
    for mode in 0..6 {
        let root = sync_root(&mut g, d, mode);
        let bytes = sync_literals(mode);
        let old = sync_guarded(&mut g, &vec![0x19; bytes.len()]);
        let new = sync_guarded(&mut g, &vec![0x73; bytes.len()]);
        let upload = g.bytes(&bytes);
        g.ok(9, &[H(root), I(0), I(upload), I(bytes.len() as u64)]);
        g.ok(75, &[H(root), I(old + 8)]);
        g.ok(75, &[H(root), I(new + 8)]);
        g.barrier(l, root, COPY_DEST, COPY_SOURCE);
        g.ok(42, &[H(l), H(scratch_dest), H(scratch_source)]);
        sync_retire(&mut g, q, l, a);
        let out = sync_guarded(&mut g, &vec![0; bytes.len()]);
        g.ok(10, &[H(root), I(0), I(out + 8), I(bytes.len() as u64)]);
        reads.push((out, bytes.clone()));
        let untouched = sync_snapshot(&mut g, new, bytes.len());
        reads.push((untouched, vec![0x73; bytes.len()]));
        let table = sync_handles(&mut g, &[root]);
        g.ok(76, &[I(table), I(1), I(0)]);
        let exported = sync_snapshot(&mut g, new, bytes.len());
        reads.push((exported, bytes.clone()));
        guest_runtime_write_bytes(&mut g, new + 8, &vec![0x91; bytes.len()]);
        g.ok(26, &[H(root)]);
        reads.push((old, vec![0x19; bytes.len()]));
        reads.push((new, vec![0x91; bytes.len()]));
    }
    check_original_guest_backend(g, true, true, &reads, 0, &[]);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sync_repeated_import_copy_export_sustains_roundtrips() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let q = g.handle(2, &[H(d)]);
    let a = g.handle(3, &[H(d)]);
    let l = g.handle(4, &[H(d), H(a)]);
    let src = sync_root(&mut g, d, 0);
    let dst = sync_root(&mut g, d, 0);
    let source = sync_guarded(&mut g, &[0; 64]);
    let dest = sync_guarded(&mut g, &[0; 64]);
    g.ok(75, &[H(src), I(source + 8)]);
    g.ok(75, &[H(dst), I(dest + 8)]);
    let input = sync_handles(&mut g, &[src]);
    let output = sync_handles(&mut g, &[dst]);
    let mut reads = Vec::new();
    // More than the 16 retained-receipt limit, reusing roots, queue and allocator.
    for frame in 0..24u8 {
        let mut bytes = sync_literals(0);
        bytes[0..4].copy_from_slice(&[frame, 0x81, 0x43, 0xfe]);
        bytes[60..64].copy_from_slice(&[0x17, frame, 0xab, 0x80]);
        guest_runtime_write_bytes(&mut g, source + 8, &bytes);
        guest_runtime_write_bytes(&mut g, dest + 8, &[0x66; 64]);
        g.ok(76, &[I(input), I(1), I(1)]);
        guest_runtime_write_bytes(&mut g, source + 8, &[0xdd; 64]);
        if frame != 0 {
            g.barrier(l, dst, COPY_SOURCE, COPY_DEST);
        }
        g.barrier(l, src, COPY_DEST, COPY_SOURCE);
        g.ok(42, &[H(l), H(dst), H(src)]);
        g.barrier(l, dst, COPY_DEST, COPY_SOURCE);
        g.barrier(l, src, COPY_SOURCE, COPY_DEST);
        sync_retire(&mut g, q, l, a);
        g.ok(76, &[I(output), I(1), I(0)]);
        let snapshot = sync_snapshot(&mut g, dest, 64);
        reads.push((snapshot, bytes));
    }
    reads.push((source, vec![0xdd; 64]));
    check_original_guest_backend(g, true, true, &reads, 0, &[]);
}
