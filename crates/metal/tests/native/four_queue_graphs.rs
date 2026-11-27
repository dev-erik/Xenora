// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent literal byte oracles for the opt-in four-queue graph contract.
use xenora_metal::*;

fn buffer(cohort: &RenderCohort, point: &RenderPoint) -> RenderBufferOutput {
    let RenderCopyOutput::Buffer(output) = cohort.copy_output(point).unwrap() else {
        panic!("expected buffer output")
    };
    output
}

fn check(cohort: &mut RenderCohort, point: &RenderPoint, expected: &[u8]) {
    let OwnedCopyOutput::Buffer(bytes) = cohort.collect_copy(point).unwrap() else {
        panic!("expected buffer readback")
    };
    assert_eq!(bytes, expected);
}

#[test]
fn four_queue_chain_preserves_every_exact_version() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let gate = super::unaligned_copy::Gate::new(&gpu, &cohort, RenderQueue::Third);
    let a = cohort
        .submit_copy(
            RenderQueue::Third,
            QueuedCopyInput::HostBuffer(&[9, 47, 131, 255, 6, 82, 193]),
            &[],
        )
        .unwrap();
    let mut points = vec![a];
    for queue in [RenderQueue::First, RenderQueue::Fourth, RenderQueue::Second] {
        let previous = points.last().unwrap();
        let input = buffer(&cohort, previous);
        // Produced inputs must infer their dependency even without explicit waits.
        let point = cohort
            .submit_copy(queue, QueuedCopyInput::ProducedBuffer(input), &[])
            .unwrap();
        points.push(point);
    }
    gate.observe_then_release(&gpu, &cohort, RenderQueue::Second);
    for point in points.iter().rev() {
        check(&mut cohort, point, &[9, 47, 131, 255, 6, 82, 193]);
    }
}

#[test]
fn four_queue_fanout_fanin_waits_for_each_branch_and_preserves_inputs() {
    let gpu = NativeCompute::new().unwrap();
    for gated in [RenderQueue::Third, RenderQueue::Fourth] {
        for reverse_waits in [false, true] {
            let mut cohort = gpu.new_render_graph_cohort().unwrap();
            let gate = super::unaligned_copy::Gate::new(&gpu, &cohort, gated);
            let a = cohort
                .submit_copy(
                    RenderQueue::First,
                    QueuedCopyInput::HostBuffer(&[10, 20, 30, 40, 50, 60, 70, 80, 90]),
                    &[],
                )
                .unwrap();
            let av = buffer(&cohort, &a);
            let b = cohort
                .submit_buffer_region_copy(
                    RenderQueue::Third,
                    QueuedBufferRegionCopy {
                        source: QueuedCopyInput::HostBuffer(&[201, 202, 203]),
                        destination: QueuedCopyInput::ProducedBuffer(av.clone()),
                        source_offset: 0,
                        destination_offset: 1,
                        byte_len: 3,
                    },
                    &[],
                )
                .unwrap();
            let c = cohort
                .submit_buffer_region_copy(
                    RenderQueue::Fourth,
                    QueuedBufferRegionCopy {
                        source: QueuedCopyInput::HostBuffer(&[211, 212, 213]),
                        destination: QueuedCopyInput::ProducedBuffer(av),
                        source_offset: 0,
                        destination_offset: 5,
                        byte_len: 3,
                    },
                    &[],
                )
                .unwrap();
            let bv = buffer(&cohort, &b);
            let cv = buffer(&cohort, &c);
            let waits = if reverse_waits {
                [c.clone(), b.clone()]
            } else {
                [b.clone(), c.clone()]
            };
            let d = cohort
                .submit_buffer_region_copy(
                    RenderQueue::Second,
                    QueuedBufferRegionCopy {
                        source: QueuedCopyInput::ProducedBuffer(bv),
                        destination: QueuedCopyInput::ProducedBuffer(cv),
                        source_offset: 1,
                        destination_offset: 2,
                        byte_len: 3,
                    },
                    &waits,
                )
                .unwrap();
            gate.observe_then_release(&gpu, &cohort, RenderQueue::Second);
            check(&mut cohort, &d, &[10, 20, 201, 202, 203, 211, 212, 213, 90]);
            check(&mut cohort, &c, &[10, 20, 30, 40, 50, 211, 212, 213, 90]);
            check(&mut cohort, &b, &[10, 201, 202, 203, 50, 60, 70, 80, 90]);
            check(&mut cohort, &a, &[10, 20, 30, 40, 50, 60, 70, 80, 90]);
        }
    }
}

#[test]
fn four_queue_graph_retains_sixteen_nodes_and_recovers_after_capacity_rejection() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_graph_cohort().unwrap();
    let queues = [
        RenderQueue::First,
        RenderQueue::Second,
        RenderQueue::Third,
        RenderQueue::Fourth,
    ];
    let mut points = Vec::new();
    for queue in queues.into_iter().cycle().take(16) {
        points.push(
            cohort
                .submit_copy(
                    queue,
                    QueuedCopyInput::HostBuffer(&[219, 0, 73, 128, 5]),
                    &[],
                )
                .unwrap(),
        );
    }
    for queue in queues {
        assert!(
            cohort
                .submit_copy(queue, QueuedCopyInput::HostBuffer(&[255]), &[])
                .is_err()
        );
    }
    for point in points.iter().rev() {
        check(&mut cohort, point, &[219, 0, 73, 128, 5]);
    }
    cohort.reinitialize().unwrap();
    for queue in queues {
        let point = cohort
            .submit_copy(queue, QueuedCopyInput::HostBuffer(&[17, 209, 63]), &[])
            .unwrap();
        check(&mut cohort, &point, &[17, 209, 63]);
    }
}

#[test]
fn legacy_cohort_rejects_new_queues_without_consuming_two_admission_slots() {
    let gpu = NativeCompute::new().unwrap();
    let mut cohort = gpu.new_render_cohort().unwrap();
    for queue in [RenderQueue::Third, RenderQueue::Fourth] {
        assert!(
            cohort
                .submit_copy(queue, QueuedCopyInput::HostBuffer(&[99, 1, 203]), &[])
                .is_err()
        );
        assert!(
            cohort
                .submit_buffer_region_copy(
                    queue,
                    QueuedBufferRegionCopy {
                        source: QueuedCopyInput::HostBuffer(&[19, 23]),
                        destination: QueuedCopyInput::HostBuffer(&[41, 43, 47]),
                        source_offset: 0,
                        destination_offset: 1,
                        byte_len: 2,
                    },
                    &[]
                )
                .is_err()
        );
    }
    let a = cohort
        .submit_copy(
            RenderQueue::First,
            QueuedCopyInput::HostBuffer(&[7, 101, 239]),
            &[],
        )
        .unwrap();
    let b = cohort
        .submit_copy(
            RenderQueue::Second,
            QueuedCopyInput::HostBuffer(&[13, 107, 241]),
            &[],
        )
        .unwrap();
    for queue in [
        RenderQueue::First,
        RenderQueue::Second,
        RenderQueue::Third,
        RenderQueue::Fourth,
    ] {
        assert!(
            cohort
                .submit_copy(queue, QueuedCopyInput::HostBuffer(&[251]), &[])
                .is_err()
        );
    }
    check(&mut cohort, &b, &[13, 107, 241]);
    check(&mut cohort, &a, &[7, 101, 239]);
}

#[test]
fn four_queue_abort_drains_and_reinitialize_rejects_old_generation() {
    let gpu = NativeCompute::new().unwrap();
    for (producer_queue, consumer_queue) in [
        (RenderQueue::Third, RenderQueue::Fourth),
        (RenderQueue::Fourth, RenderQueue::Third),
    ] {
        let mut cohort = gpu.new_render_graph_cohort().unwrap();
        let producer = cohort
            .submit_copy(
                producer_queue,
                QueuedCopyInput::HostBuffer(&[31, 157, 0, 249, 83]),
                &[],
            )
            .unwrap();
        let old_source = buffer(&cohort, &producer);
        let consumer = cohort
            .submit_copy(
                consumer_queue,
                QueuedCopyInput::ProducedBuffer(old_source.clone()),
                &[],
            )
            .unwrap();
        let old_result = buffer(&cohort, &consumer);
        // Controlled host publication failure, not an injected GPU fault.
        // Leave both receipts uncollected so abort must drain their ancestry.
        cohort
            .abort_and_drain("independent controlled publication failure")
            .unwrap();
        for point in [&producer, &consumer] {
            assert!(cohort.collect_copy(point).is_err());
            assert!(cohort.copy_output(point).is_err());
            assert!(cohort.is_complete(point).is_err());
        }
        for queue in [RenderQueue::Third, RenderQueue::Fourth] {
            assert!(
                cohort
                    .submit_copy(queue, QueuedCopyInput::HostBuffer(&[1]), &[])
                    .is_err()
            );
        }
        cohort.reinitialize().unwrap();
        // Create new points first: stale points must not alias reused sequence
        // numbers, and rejected old inputs must not damage these fresh outputs.
        let fresh_third = cohort
            .submit_copy(
                RenderQueue::Third,
                QueuedCopyInput::HostBuffer(&[17, 233, 64, 9, 181]),
                &[],
            )
            .unwrap();
        let fresh_fourth = cohort
            .submit_copy(
                RenderQueue::Fourth,
                QueuedCopyInput::HostBuffer(&[229, 3, 128, 71, 45]),
                &[],
            )
            .unwrap();
        for point in [&producer, &consumer] {
            assert!(cohort.collect_copy(point).is_err());
            assert!(cohort.copy_output(point).is_err());
            assert!(cohort.is_complete(point).is_err());
            for queue in [RenderQueue::Third, RenderQueue::Fourth] {
                assert!(
                    cohort
                        .submit_copy(
                            queue,
                            QueuedCopyInput::HostBuffer(&[99]),
                            std::slice::from_ref(point)
                        )
                        .is_err()
                );
            }
        }
        for old_output in [old_source, old_result] {
            for queue in [RenderQueue::Third, RenderQueue::Fourth] {
                assert!(
                    cohort
                        .submit_copy(
                            queue,
                            QueuedCopyInput::ProducedBuffer(old_output.clone()),
                            &[]
                        )
                        .is_err()
                );
            }
        }
        check(&mut cohort, &fresh_fourth, &[229, 3, 128, 71, 45]);
        check(&mut cohort, &fresh_third, &[17, 233, 64, 9, 181]);
    }
}
