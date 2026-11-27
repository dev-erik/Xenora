// SPDX-License-Identifier: GPL-3.0-or-later
//! Original guest coverage for the opt-in four-queue graph increment.
//! Literal copy expectations; GPU evidence requires actually running this test.
use super::*;

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_four_queue_chain_keeps_mapping_after_successful_drain() {
    let mut g = GuestScript::new();
    let device = g.handle(1, &[]);
    let queues: Vec<_> = (0..5).map(|_| g.handle(2, &[H(device)])).collect();
    let allocators: Vec<_> = (0..5).map(|_| g.handle(3, &[H(device)])).collect();
    let lists: Vec<_> = allocators
        .iter()
        .map(|a| g.handle(4, &[H(device), H(*a)]))
        .collect();
    let seed = g.handle(5, &[H(device), I(16), I(COPY_DEST)]);
    let literal = [
        3, 17, 129, 251, 29, 43, 61, 79, 97, 113, 131, 151, 173, 193, 211, 229,
    ];
    let data = g.bytes(&literal);
    g.ok(9, &[H(seed), I(0), I(data), I(16)]);
    let roots: Vec<_> = (0..5)
        .map(|_| g.handle(5, &[H(device), I(16), I(COPY_DEST)]))
        .collect();
    for (root, byte) in roots.iter().zip([0x31, 0x41, 0x51, 0x61, 0x71]) {
        let data = g.bytes(&[byte; 16]);
        g.ok(9, &[H(*root), I(0), I(data), I(16)]);
    }
    g.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
    g.ok(42, &[H(lists[0]), H(roots[0]), H(seed)]);
    g.barrier(lists[0], roots[0], COPY_DEST, COPY_SOURCE);
    for i in 1..4 {
        // Each consumer must use its predecessor's produced bytes, including
        // the untouched prefix retained from that predecessor's input version.
        g.ok(
            43,
            &[H(lists[i]), H(roots[i]), I(4), H(roots[i - 1]), I(0), I(12)],
        );
        g.barrier(lists[i], roots[i], COPY_DEST, COPY_SOURCE);
    }
    g.ok(42, &[H(lists[4]), H(roots[4]), H(roots[3])]);
    g.barrier(lists[4], roots[4], COPY_DEST, COPY_SOURCE);
    for list in &lists {
        g.ok(12, &[H(*list)]);
    }
    let mut receipts = Vec::new();
    let mut wait = 0;
    for i in 0..4 {
        let receipt = g.handle(
            35,
            &[H(queues[i]), H(lists[i]), I(wait), I(u64::from(i != 0))],
        );
        let point = g.call(36, &[H(receipt)], 0, Some(1));
        wait = g.bytes(&[0; 16]);
        g.copy_result(queues[i], wait);
        g.copy_result(point, wait + 8);
        receipts.push(receipt);
    }
    // Four retained nodes are below the sixteen-node cap. Only the new queue
    // exceeds admission; a rejection must leave both list and result intact.
    g.call(
        35,
        &[H(queues[4]), H(lists[4]), I(wait), I(1)],
        2,
        Some(u64::MAX),
    );
    let retry = g.handle(35, &[H(queues[0]), H(lists[4]), I(wait), I(1)]);
    g.call(36, &[H(retry)], 0, Some(2));
    receipts.push(retry);
    for i in 0..5 {
        g.ok(11, &[H(lists[i]), H(allocators[i])]);
    }
    for resource in [seed, roots[0], roots[1], roots[2]] {
        g.ok(26, &[H(resource)]);
    }
    // Wait and release descendants first, while ancestors still have receipts.
    for receipt in receipts.iter().rev() {
        g.call(32, &[H(*receipt)], 0, Some(1));
        g.ok(33, &[H(*receipt)]);
    }
    let output = g.bytes(&[0xa5; 32]);
    g.ok(10, &[H(roots[4]), I(0), I(output + 8), I(16)]);

    // Successful drainage retains the generation and its four queue mappings.
    // A fifth queue still rejects; the same list retries on an existing queue.
    // This checks existing-queue points, not the unobservable rejected queue's
    // next point: proving that requires a separate generation-recovery test.
    g.barrier(lists[4], roots[4], COPY_SOURCE, COPY_DEST);
    g.ok(42, &[H(lists[4]), H(roots[4]), H(roots[3])]);
    g.barrier(lists[4], roots[4], COPY_DEST, COPY_SOURCE);
    g.ok(12, &[H(lists[4])]);
    g.call(
        35,
        &[H(queues[4]), H(lists[4]), I(0), I(0)],
        2,
        Some(u64::MAX),
    );
    let reused = g.handle(35, &[H(queues[1]), H(lists[4]), I(0), I(0)]);
    g.call(36, &[H(reused)], 0, Some(2));
    g.ok(11, &[H(lists[4]), H(allocators[4])]);
    g.ok(26, &[H(roots[3])]);
    g.call(32, &[H(reused)], 0, Some(1));
    g.ok(33, &[H(reused)]);
    let final_output = g.bytes(&[0xa5; 32]);
    g.ok(10, &[H(roots[4]), I(0), I(final_output + 8), I(16)]);
    // Three successive right-shifted 12-byte copies preserve these prefixes;
    // no production decoder, graph traversal or reference computes the oracle.
    let expected = vec![
        0x61, 0x61, 0x61, 0x61, 0x51, 0x51, 0x51, 0x51, 0x41, 0x41, 0x41, 0x41, 3, 17, 129, 251,
    ];
    check_original_guest_backend(
        g,
        true,
        true,
        &[(output, expected.clone()), (final_output, expected)],
        0,
        &[],
    );
}
