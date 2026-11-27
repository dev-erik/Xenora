// SPDX-License-Identifier: GPL-3.0-or-later
//! Controlled CPU-only graph backend: no GPU work and no sleeps.
use std::sync::{Arc, Mutex};
use xenora_loader::graphics::*;
use xenora_loader::{GuestMemory, MemoryPermissions};

#[derive(Default)]
struct State {
    output: Option<GraphicsQueuedWorkOutput>,
    rejected: bool,
    drops: usize,
}
struct Ticket(Arc<Mutex<State>>);
impl Drop for Ticket {
    fn drop(&mut self) {
        self.0.lock().unwrap().drops += 1;
    }
}
impl GraphicsQueuedPendingWork for Ticket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.lock().unwrap().output.take().map(Ok)
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().expect("CPU result supplied")
    }
    fn reject_dependency(&mut self, _: GraphicsQueuePoint) {
        self.0.lock().unwrap().rejected = true;
    }
}
#[derive(Default)]
struct Backend {
    requests: Vec<GraphicsQueuedWorkRequest>,
    states: Vec<Arc<Mutex<State>>>,
}
struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
    device: GraphicsHandle,
    queues: [GraphicsHandle; 4],
    backend: Arc<Mutex<Backend>>,
}
impl Harness {
    fn new() -> Self {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queues = std::array::from_fn(|_| {
            ctx.create_command_queue(device, CommandListType::Direct)
                .unwrap()
        });
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 64, MemoryPermissions::READ_WRITE)
            .unwrap();
        let mut h = Self {
            ctx,
            memory,
            device,
            queues,
            backend: Arc::new(Mutex::new(Backend::default())),
        };
        h.install(true);
        h
    }
    fn install(&mut self, graph: bool) {
        let captured = self.backend.clone();
        let callback = move |request| -> Result<Box<dyn GraphicsQueuedPendingWork>, GraphicsError> {
            let state = Arc::new(Mutex::new(State::default()));
            let mut backend = captured.lock().unwrap();
            backend.requests.push(request);
            backend.states.push(state.clone());
            Ok(Box::new(Ticket(state)))
        };
        if graph {
            self.ctx
                .set_metal_graph_queued_backend(GraphicsTextureCapabilities::default(), callback);
        } else {
            self.ctx.set_metal_unaligned_buffer_region_queued_backend(
                GraphicsTextureCapabilities::default(),
                callback,
            );
        }
    }
    fn resource(&mut self, bytes: [u8; 4]) -> GraphicsHandle {
        let handle = self
            .ctx
            .create_committed_resource(
                self.device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: 4,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::COPY_DEST,
                None,
            )
            .unwrap();
        self.memory.write_bytes(0x1000, &bytes).unwrap();
        self.ctx
            .copy_guest_to_resource(handle, 0, 0x1000, 4, &self.memory)
            .unwrap();
        handle
    }
    fn list(&mut self) -> (GraphicsHandle, GraphicsHandle) {
        let allocator = self
            .ctx
            .create_command_allocator(self.device, CommandListType::Direct)
            .unwrap();
        (
            self.ctx
                .create_command_list(self.device, allocator, CommandListType::Direct)
                .unwrap(),
            allocator,
        )
    }
    fn copy_list(
        &mut self,
        source: GraphicsHandle,
        destination: GraphicsHandle,
        source_ready: bool,
    ) -> (GraphicsHandle, GraphicsHandle) {
        let (list, allocator) = self.list();
        if !source_ready {
            self.ctx
                .cmd_resource_barrier(
                    list,
                    source,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
        }
        self.ctx
            .cmd_copy_resource(list, destination, source)
            .unwrap();
        self.ctx
            .cmd_resource_barrier(
                list,
                destination,
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
        self.ctx.close_command_list(list).unwrap();
        (list, allocator)
    }
    fn submit(
        &mut self,
        queue: usize,
        list: GraphicsHandle,
        waits: &[GraphicsQueuePoint],
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.ctx
            .submit_command_list_after(self.queues[queue], list, waits, &self.memory)
    }
    fn point(&self, handle: GraphicsHandle) -> GraphicsQueuePoint {
        self.ctx.submission_point(handle).unwrap()
    }
    fn complete(&self, index: usize, bytes: &[u8]) {
        self.backend.lock().unwrap().states[index]
            .lock()
            .unwrap()
            .output = Some(GraphicsQueuedWorkOutput::Copy {
            layout: GraphicsCopyLayout::Buffer {
                byte_len: bytes.len(),
            },
            bytes: bytes.to_vec(),
        });
    }
    fn read(&mut self, handle: GraphicsHandle) -> [u8; 4] {
        self.ctx
            .copy_resource_to_guest(handle, 0, 0x1000, 4, &mut self.memory)
            .unwrap();
        let mut bytes = [0; 4];
        self.memory.read_bytes(0x1000, &mut bytes).unwrap();
        bytes
    }
    fn assert_source(&self, index: usize, point: GraphicsQueuePoint, waits: &[GraphicsQueuePoint]) {
        let backend = self.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[index] else {
            panic!("copy")
        };
        assert_eq!(request.waits, waits);
        let GraphicsCopyInput::Produced(version) = request.source else {
            panic!("must not use stale host snapshot")
        };
        assert_eq!(
            version,
            GraphicsProducedVersion {
                point,
                output: GraphicsProducedOutput::Copy(0)
            }
        );
    }
}

#[test]
fn cpu_graph_transitive_source_pending_same_queue_release_and_failure() {
    for fail in [false, true] {
        for consumer_first in [false, true] {
            let mut h = Harness::new();
            let seed = h.resource([17, 83, 149, 231]);
            let x = h.resource([1; 4]);
            let y = h.resource([2; 4]);
            let z = h.resource([3; 4]);
            let (la, aa) = h.copy_list(seed, x, false);
            let a = h.submit(0, la, &[]).unwrap();
            let pa = h.point(a);
            let (lb, ab) = h.copy_list(x, y, true);
            let b = h.submit(0, lb, &[]).unwrap();
            let pb = h.point(b);
            let (lc, ac) = h.copy_list(x, z, true);
            let c = h.submit(1, lc, &[pb]).unwrap();
            h.assert_source(2, pa, &[pa, pb]);
            assert_eq!(h.ctx.queued_copy_allocation_bytes(), 32); // Host16 + produced8 + produced8.
            h.complete(2, &[17, 83, 149, 231]);
            h.complete(1, &[17, 83, 149, 231]);
            h.complete(0, if fail { &[0] } else { &[17, 83, 149, 231] });
            if !consumer_first {
                h.ctx.wait_submission(a).unwrap();
                h.ctx.release_submission(a).unwrap();
                assert_eq!(h.backend.lock().unwrap().states[0].lock().unwrap().drops, 0);
            }
            assert_eq!(
                h.ctx.wait_submission(c).unwrap(),
                if fail {
                    GraphicsSubmissionStatus::Failed
                } else {
                    GraphicsSubmissionStatus::Complete
                }
            );
            if consumer_first {
                h.ctx.release_submission(a).unwrap();
            }
            h.ctx.release_submission(b).unwrap();
            assert_eq!(h.ctx.queued_copy_allocation_bytes(), 32);
            if fail {
                let backend = h.backend.lock().unwrap();
                assert!(backend.states[1].lock().unwrap().rejected);
                assert!(backend.states[2].lock().unwrap().rejected);
            }
            h.ctx.release_submission(c).unwrap();
            assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
            for state in &h.backend.lock().unwrap().states {
                assert_eq!(state.lock().unwrap().drops, 1);
            }
            if !fail {
                assert_eq!(h.read(z), [17, 83, 149, 231]);
            } else {
                for (list, allocator) in [(la, aa), (lb, ab), (lc, ac)] {
                    h.ctx.reset_command_list(list, allocator).unwrap();
                }
                let (retry, _) = h.copy_list(seed, x, false);
                let receipt = h.submit(0, retry, &[]).unwrap();
                let backend = h.backend.lock().unwrap();
                let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[3] else {
                    panic!("copy")
                };
                let GraphicsCopyInput::Host(bytes) = &request.source else {
                    panic!("fresh generation host")
                };
                assert_eq!(bytes.as_ref(), &[17, 83, 149, 231]);
                drop(backend);
                h.complete(3, &[17, 83, 149, 231]);
                h.ctx.wait_submission(receipt).unwrap();
                h.ctx.release_submission(receipt).unwrap();
            }
            assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
        }
    }
}

#[test]
fn cpu_graph_diamond_inherited_definition_is_not_a_conflict_in_either_wait_order() {
    for reverse in [false, true] {
        let mut h = Harness::new();
        let seed = h.resource([17, 83, 149, 231]);
        let x = h.resource([1; 4]);
        let y = h.resource([2; 4]);
        let z = h.resource([3; 4]);
        let out = h.resource([4; 4]);
        let (la, _) = h.copy_list(seed, x, false);
        let a = h.submit(0, la, &[]).unwrap();
        let pa = h.point(a);
        let (lb, _) = h.copy_list(x, y, true);
        let b = h.submit(0, lb, &[]).unwrap();
        let pb = h.point(b);
        let (lc, _) = h.copy_list(x, z, true);
        let c = h.submit(1, lc, &[pa]).unwrap();
        let pc = h.point(c);
        let (ld, _) = h.copy_list(x, out, true);
        let waits = if reverse { [pc, pb] } else { [pb, pc] };
        let d = h.submit(1, ld, &waits).unwrap();
        h.assert_source(3, pa, &[pa, pb, pc]);
        for i in 0..4 {
            h.complete(i, &[17, 83, 149, 231]);
        }
        h.ctx.wait_submission(d).unwrap();
        for receipt in [a, c, b, d] {
            h.ctx.release_submission(receipt).unwrap();
        }
        assert_eq!(h.read(out), [17, 83, 149, 231]);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    }
}

#[test]
fn cpu_graph_incomparable_equal_state_writers_reject_in_both_orders() {
    for reverse in [false, true] {
        let mut h = Harness::new();
        let seed = h.resource([17; 4]);
        let seed2 = h.resource([31; 4]);
        let x = h.resource([0; 4]);
        let out = h.resource([0; 4]);
        let (la, _) = h.copy_list(seed, x, false);
        let a = h.submit(0, la, &[]).unwrap();
        h.complete(0, &[17; 4]);
        h.ctx.wait_submission(a).unwrap();
        let (lb, _) = h.list();
        h.ctx
            .cmd_resource_barrier(
                lb,
                seed2,
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
        h.ctx
            .cmd_resource_barrier(
                lb,
                x,
                ResourceStates::COPY_SOURCE,
                ResourceStates::COPY_DEST,
            )
            .unwrap();
        h.ctx.cmd_copy_resource(lb, x, seed2).unwrap();
        h.ctx
            .cmd_resource_barrier(
                lb,
                x,
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
        h.ctx.close_command_list(lb).unwrap();
        let b = h.submit(1, lb, &[]).unwrap();
        h.complete(1, &[31; 4]);
        h.ctx.wait_submission(b).unwrap();
        let (lc, _) = h.copy_list(x, out, true);
        let waits = if reverse {
            [h.point(b), h.point(a)]
        } else {
            [h.point(a), h.point(b)]
        };
        let marker = h.ctx.create_device().unwrap();
        let charged = h.ctx.queued_copy_allocation_bytes();
        assert!(h.submit(0, lc, &waits).is_err());
        assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), charged);
        assert_eq!(h.backend.lock().unwrap().requests.len(), 2);
        h.ctx.release_submission(a).unwrap();
        h.ctx.release_submission(b).unwrap();
        assert_eq!(h.read(x), [31; 4]);
    }
}

#[test]
fn cpu_graph_sixteen_retained_nodes_then_atomic_capacity_and_pruning() {
    let mut h = Harness::new();
    let seed = h.resource([17, 83, 149, 231]);
    let mut receipts = Vec::new();
    assert_eq!(h.ctx.queued_submission_capacity(), MAX_QUEUED_GRAPH_NODES);
    for i in 0..MAX_QUEUED_GRAPH_NODES {
        let out = h.resource([0; 4]);
        let (list, _) = h.copy_list(seed, out, i != 0);
        receipts.push(h.submit(0, list, &[]).unwrap());
    }
    let out = h.resource([0; 4]);
    let (list, _) = h.copy_list(seed, out, true);
    let marker = h.ctx.create_device().unwrap();
    let charged = h.ctx.queued_copy_allocation_bytes();
    assert!(h.submit(0, list, &[]).is_err());
    assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
    assert_eq!(h.backend.lock().unwrap().requests.len(), 16);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), charged);
    for i in 0..16 {
        h.complete(i, &[17, 83, 149, 231]);
    }
    h.ctx.wait_submission(receipts[15]).unwrap();
    for receipt in &receipts[..15] {
        h.ctx.release_submission(*receipt).unwrap();
    }
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), charged);
    assert!(
        h.submit(0, list, &[]).is_err(),
        "released ancestors still consume capacity"
    );
    h.ctx.release_submission(receipts[15]).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    let next = h.submit(0, list, &[]).unwrap();
    assert_eq!(h.point(next).value, 17);
    h.complete(16, &[17, 83, 149, 231]);
    h.ctx.wait_submission(next).unwrap();
    h.ctx.release_submission(next).unwrap();
}

#[test]
fn cpu_graph_old_installer_restores_two_node_and_single_pending_queue_contract() {
    let mut h = Harness::new();
    h.install(false);
    assert_eq!(h.ctx.queued_submission_capacity(), 2);
    let seed = h.resource([17; 4]);
    let x = h.resource([0; 4]);
    let y = h.resource([0; 4]);
    let z = h.resource([0; 4]);
    let (la, _) = h.copy_list(seed, x, false);
    let a = h.submit(0, la, &[]).unwrap();
    let (lb, _) = h.copy_list(x, y, true);
    assert!(h.submit(0, lb, &[]).is_err());
    let b = h.submit(1, lb, &[h.point(a)]).unwrap();
    let (lc, _) = h.copy_list(y, z, true);
    assert!(h.submit(0, lc, &[h.point(b)]).is_err());
    assert_eq!(h.backend.lock().unwrap().requests.len(), 2);
    h.complete(0, &[17; 4]);
    h.complete(1, &[17; 4]);
    h.ctx.wait_submission(b).unwrap();
    h.ctx.release_submission(a).unwrap();
    h.ctx.release_submission(b).unwrap();
}

#[test]
fn cpu_graph_barrier_only_use_preserves_ancestor_output_and_retired_ticket_chain() {
    let mut h = Harness::new();
    let seed = h.resource([17, 83, 149, 231]);
    let seed2 = h.resource([31; 4]);
    let x = h.resource([1; 4]);
    let y = h.resource([2; 4]);
    let z = h.resource([3; 4]);
    let (la, aa) = h.copy_list(seed, x, false);
    let a = h.submit(0, la, &[]).unwrap();
    let pa = h.point(a);
    let (lb, ab) = h.list();
    h.ctx
        .cmd_resource_barrier(
            lb,
            x,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx
        .cmd_resource_barrier(
            lb,
            seed2,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.cmd_copy_resource(lb, y, seed2).unwrap();
    h.ctx
        .cmd_resource_barrier(
            lb,
            y,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(lb).unwrap();
    let b = h.submit(1, lb, &[pa]).unwrap();
    let pb = h.point(b);
    let (lc, ac) = h.copy_list(x, z, true);
    let c = h.submit(0, lc, &[pb]).unwrap();
    h.assert_source(2, pa, &[pa, pb]);
    for (list, allocator) in [(la, aa), (lb, ab), (lc, ac)] {
        h.ctx.reset_command_list(list, allocator).unwrap();
    }
    for resource in [seed, seed2, x, y, z] {
        h.ctx.destroy_resource(resource).unwrap();
    }
    h.complete(0, &[17, 83, 149, 231]);
    h.complete(1, &[31; 4]);
    h.complete(2, &[17, 83, 149, 231]);
    assert_eq!(
        h.ctx.wait_submission(c).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(a).unwrap();
    h.ctx.release_submission(b).unwrap();
    assert_eq!(h.backend.lock().unwrap().states[0].lock().unwrap().drops, 0);
    h.ctx.release_submission(c).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert!(
        h.ctx.destroy_resource(z).is_err(),
        "publication must not resurrect a destroyed destination"
    );
}

#[test]
fn cpu_graph_four_queue_failed_diamond_rejects_publication_and_recovers() {
    for reverse in [false, true] {
        let mut h = Harness::new();
        let seed = h.resource([17, 83, 149, 231]);
        let roots = [
            h.resource([1; 4]),
            h.resource([2; 4]),
            h.resource([3; 4]),
            h.resource([4; 4]),
        ];
        let (la, aa) = h.copy_list(seed, roots[0], false);
        let a = h.submit(0, la, &[]).unwrap();
        let pa = h.point(a);
        let (lb, ab) = h.copy_list(roots[0], roots[1], true);
        let b = h.submit(1, lb, &[pa]).unwrap();
        let (lc, ac) = h.copy_list(roots[0], roots[2], true);
        let c = h.submit(2, lc, &[pa]).unwrap();
        let (ld, ad) = h.copy_list(roots[0], roots[3], true);
        let waits = if reverse {
            [h.point(c), h.point(b)]
        } else {
            [h.point(b), h.point(c)]
        };
        let d = h.submit(3, ld, &waits).unwrap();
        h.assert_source(3, pa, &[pa, h.point(b), h.point(c)]);
        // Native-like descendants have complete results before the ancestor's
        // malformed output is discovered. None may become a published success.
        h.complete(0, &[17]);
        for i in 1..4 {
            h.complete(i, &[17, 83, 149, 231]);
        }
        assert_eq!(
            h.ctx.wait_submission(d).unwrap(),
            GraphicsSubmissionStatus::Failed
        );
        for receipt in [a, b, c, d] {
            assert_eq!(
                h.ctx.wait_submission(receipt).unwrap(),
                GraphicsSubmissionStatus::Failed
            );
            assert!(h.ctx.submission_copy_outcome(receipt).unwrap().is_none());
        }
        for (list, allocator) in [(la, aa), (lb, ab), (lc, ac), (ld, ad)] {
            h.ctx.reset_command_list(list, allocator).unwrap();
        }
        // Release ancestors first: descendants still pin their payload budget.
        let charged = h.ctx.queued_copy_allocation_bytes();
        for receipt in [a, b, c] {
            h.ctx.release_submission(receipt).unwrap();
        }
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), charged);
        h.ctx.release_submission(d).unwrap();
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        for state in &h.backend.lock().unwrap().states {
            assert_eq!(state.lock().unwrap().drops, 1);
        }
        // Original destination states survived failure: this COPY_DEST barrier
        // would reject if the failed graph had published its state transitions.
        for (i, root) in roots.into_iter().enumerate() {
            let out = h.resource([0; 4]);
            let (list, _) = h.copy_list(root, out, false);
            assert!(h.submit(i, list, &[pa]).is_err(), "old point must reject");
            let receipt = h.submit(i, list, &[]).unwrap();
            assert_eq!(
                h.point(receipt).value,
                2,
                "rejection must not consume a point"
            );
            let expected = [i as u8 + 1; 4];
            // Supply cleanup's result before assertions so a detected regression
            // cannot turn a useful assertion failure into a destructor panic.
            h.complete(4 + i, &expected);
            let (bytes, generation, old_generation) = {
                let backend = h.backend.lock().unwrap();
                let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[4 + i] else {
                    panic!("copy")
                };
                let GraphicsCopyInput::Host(bytes) = &request.source else {
                    panic!("host after recovery")
                };
                let GraphicsQueuedWorkRequest::Copy(old) = &backend.requests[0] else {
                    panic!("copy")
                };
                (bytes.to_vec(), request.generation, old.generation)
            };
            assert_eq!(bytes, expected);
            assert_eq!(generation, old_generation + 1);
            assert_eq!(
                h.ctx.wait_submission(receipt).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
            h.ctx.release_submission(receipt).unwrap();
            assert_eq!(h.read(out), expected);
        }
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}
