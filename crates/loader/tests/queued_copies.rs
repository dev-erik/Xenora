// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU-only typed queued-copy contract. Fake callbacks perform no GPU work.
use std::sync::{Arc, Mutex};
use xenora_loader::graphics::*;
use xenora_loader::{GuestMemory, MemoryPermissions};

struct CpuTicket(Option<GraphicsQueuedWorkOutput>);
impl GraphicsQueuedPendingWork for CpuTicket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.take().map(Ok)
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().expect("CPU completion available")
    }
    fn reject_dependency(&mut self, _producer: GraphicsQueuePoint) {}
}

#[test]
fn cpu_queued_copy_typed_buffer_and_texture_publish_exact_bytes_without_presentation() {
    for layout in [
        GraphicsCopyLayout::Buffer { byte_len: 5 },
        GraphicsCopyLayout::Rgba8 { extent: (2, 1) },
    ] {
        let literal = match layout {
            GraphicsCopyLayout::TypedTexture(_)
            | GraphicsCopyLayout::Rgba8MipChain { .. }
            | GraphicsCopyLayout::Rgba8Array { .. } => {
                panic!("unexpected subresource layout in legacy copy test")
            }
            GraphicsCopyLayout::Buffer { .. } => vec![17, 83, 149, 231, 9],
            GraphicsCopyLayout::Rgba8 { .. } => vec![17, 83, 149, 231, 211, 43, 97, 127],
        };
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queue = ctx
            .create_command_queue(device, CommandListType::Direct)
            .unwrap();
        let allocator = ctx
            .create_command_allocator(device, CommandListType::Direct)
            .unwrap();
        let list = ctx
            .create_command_list(device, allocator, CommandListType::Direct)
            .unwrap();
        let desc = match layout {
            GraphicsCopyLayout::TypedTexture(_)
            | GraphicsCopyLayout::Rgba8MipChain { .. }
            | GraphicsCopyLayout::Rgba8Array { .. } => {
                panic!("unexpected subresource layout in legacy copy test")
            }
            GraphicsCopyLayout::Buffer { byte_len } => ResourceDesc {
                dimension: ResourceDimension::Buffer,
                width: byte_len as u64,
                height: 1,
                depth_or_array_size: 1,
                mip_levels: 1,
                format_rgba8: false,
            },
            GraphicsCopyLayout::Rgba8 {
                extent: (width, height),
            } => ResourceDesc {
                dimension: ResourceDimension::Texture2D,
                width: width as u64,
                height: height as u32,
                depth_or_array_size: 1,
                mip_levels: 1,
                format_rgba8: true,
            },
        };
        let source = ctx
            .create_committed_resource(device, desc.clone(), ResourceStates::COPY_DEST, None)
            .unwrap();
        let destination = ctx
            .create_committed_resource(device, desc, ResourceStates::COPY_DEST, None)
            .unwrap();
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 64, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory
            .map_region(0x2000, 64, MemoryPermissions::READ_WRITE)
            .unwrap();
        memory.write_bytes(0x1000, &literal).unwrap();
        ctx.copy_guest_to_resource(source, 0, 0x1000, literal.len(), &memory)
            .unwrap();
        ctx.cmd_resource_barrier(
            list,
            source,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
        ctx.cmd_copy_resource(list, destination, source).unwrap();
        ctx.cmd_resource_barrier(
            list,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
        ctx.close_command_list(list).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let expected = literal.clone();
        ctx.set_metal_copy_queued_backend(GraphicsTextureCapabilities::default(), move |request| {
            let GraphicsQueuedWorkRequest::Copy(copy) = request else {
                panic!("CPU copy dispatched as render");
            };
            assert_eq!(copy.layout, layout);
            assert_eq!(copy.output.point, copy.point);
            assert_eq!(copy.output.output, GraphicsProducedOutput::Copy(0));
            assert!(copy.waits.is_empty());
            match &copy.source {
                GraphicsCopyInput::Host(bytes) => assert_eq!(bytes.as_ref(), expected.as_slice()),
                _ => panic!("first copy must use committed host input"),
            }
            captured.lock().unwrap().push(copy);
            // Literal output is independent of any production copy implementation.
            Ok(Box::new(CpuTicket(Some(GraphicsQueuedWorkOutput::Copy {
                layout,
                bytes: expected.clone(),
            }))))
        });
        // Synchronous rendering API must reject before submitting any copy work.
        assert!(ctx.execute_command_list(queue, list, &memory).is_err());
        assert!(requests.lock().unwrap().is_empty());
        let receipt = ctx
            .submit_command_list(queue, list, &memory)
            .expect("typed queued backend must admit a valid whole-resource copy");
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert_eq!(
            ctx.wait_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        assert!(ctx.submission_outcome(receipt).unwrap().is_none());
        let outcome = ctx.submission_copy_outcome(receipt).unwrap().unwrap();
        assert_eq!(outcome.layout, layout);
        assert_eq!(outcome.bytes, literal);
        let presentation = ctx.last_presentation.lock().unwrap();
        assert_eq!(presentation.frame_index, 0);
        assert!(presentation.pixels.is_empty());
        drop(presentation);
        ctx.release_submission(receipt).unwrap();
        ctx.copy_resource_to_guest(destination, 0, 0x2000, literal.len(), &mut memory)
            .unwrap();
        let mut readback = vec![0; literal.len()];
        memory.read_bytes(0x2000, &mut readback).unwrap();
        assert_eq!(readback, literal);
    }
}

#[derive(Default)]
struct ControlledState {
    result: Option<Result<GraphicsQueuedWorkOutput, GraphicsError>>,
    rejected: Vec<GraphicsQueuePoint>,
}
struct ControlledTicket(Arc<Mutex<ControlledState>>);
impl GraphicsQueuedPendingWork for ControlledTicket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.lock().unwrap().result.take()
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().expect("release CPU ticket before wait")
    }
    fn reject_dependency(&mut self, producer: GraphicsQueuePoint) {
        self.0.lock().unwrap().rejected.push(producer);
    }
}
#[derive(Default)]
struct ControlledBackend {
    requests: Vec<GraphicsQueuedWorkRequest>,
    tickets: Vec<Arc<Mutex<ControlledState>>>,
}
struct CopyHarness {
    ctx: GraphicsContext,
    memory: GuestMemory,
    device: GraphicsHandle,
    queues: [GraphicsHandle; 2],
    allocators: [GraphicsHandle; 2],
    lists: [GraphicsHandle; 2],
    backend: Arc<Mutex<ControlledBackend>>,
}
impl CopyHarness {
    fn new() -> Self {
        let mut ctx = GraphicsContext::new();
        let device = ctx.create_device().unwrap();
        let queues = std::array::from_fn(|_| {
            ctx.create_command_queue(device, CommandListType::Direct)
                .unwrap()
        });
        let allocators = std::array::from_fn(|_| {
            ctx.create_command_allocator(device, CommandListType::Direct)
                .unwrap()
        });
        let lists = std::array::from_fn(|i| {
            ctx.create_command_list(device, allocators[i], CommandListType::Direct)
                .unwrap()
        });
        let backend = Arc::new(Mutex::new(ControlledBackend::default()));
        let captured = backend.clone();
        ctx.set_metal_copy_queued_backend(GraphicsTextureCapabilities::default(), move |request| {
            let state = Arc::new(Mutex::new(ControlledState::default()));
            let mut backend = captured.lock().unwrap();
            backend.requests.push(request);
            backend.tickets.push(state.clone());
            Ok(Box::new(ControlledTicket(state)))
        });
        let mut memory = GuestMemory::new();
        memory
            .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
            .unwrap();
        Self {
            ctx,
            memory,
            device,
            queues,
            allocators,
            lists,
            backend,
        }
    }
    fn resource(&mut self, texture: bool, bytes: &[u8]) -> GraphicsHandle {
        let resource = self
            .ctx
            .create_committed_resource(
                self.device,
                ResourceDesc {
                    dimension: if texture {
                        ResourceDimension::Texture2D
                    } else {
                        ResourceDimension::Buffer
                    },
                    width: if texture {
                        (bytes.len() / 4) as u64
                    } else {
                        bytes.len() as u64
                    },
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: texture,
                },
                ResourceStates::COPY_DEST,
                None,
            )
            .unwrap();
        self.memory.write_bytes(0x1000, bytes).unwrap();
        self.ctx
            .copy_guest_to_resource(resource, 0, 0x1000, bytes.len(), &self.memory)
            .unwrap();
        resource
    }
    fn record(
        &mut self,
        slot: usize,
        source: GraphicsHandle,
        destination: GraphicsHandle,
        source_state: ResourceStates,
        after: ResourceStates,
    ) {
        let list = self.lists[slot];
        if source_state != ResourceStates::COPY_SOURCE {
            self.ctx
                .cmd_resource_barrier(list, source, source_state, ResourceStates::COPY_SOURCE)
                .unwrap();
        }
        self.ctx
            .cmd_copy_resource(list, destination, source)
            .unwrap();
        if after != ResourceStates::COPY_DEST {
            self.ctx
                .cmd_resource_barrier(list, destination, ResourceStates::COPY_DEST, after)
                .unwrap();
        }
        self.ctx.close_command_list(list).unwrap();
    }
    fn submit(
        &mut self,
        slot: usize,
        waits: &[GraphicsQueuePoint],
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.ctx
            .submit_command_list_after(self.queues[slot], self.lists[slot], waits, &self.memory)
    }
    fn complete(&self, index: usize, output: GraphicsQueuedWorkOutput) {
        self.backend.lock().unwrap().tickets[index]
            .lock()
            .unwrap()
            .result = Some(Ok(output));
    }
    fn reset(&mut self, slot: usize) {
        self.ctx
            .reset_command_list(self.lists[slot], self.allocators[slot])
            .unwrap();
    }
    fn read(&mut self, handle: GraphicsHandle, count: usize) -> Vec<u8> {
        self.ctx
            .copy_resource_to_guest(handle, 0, 0x1000, count, &mut self.memory)
            .unwrap();
        let mut bytes = vec![0; count];
        self.memory.read_bytes(0x1000, &mut bytes).unwrap();
        bytes
    }
}
fn copy_bytes(bytes: &[u8]) -> GraphicsQueuedWorkOutput {
    GraphicsQueuedWorkOutput::Copy {
        layout: GraphicsCopyLayout::Buffer {
            byte_len: bytes.len(),
        },
        bytes: bytes.to_vec(),
    }
}

#[test]
fn cpu_copy_dependency_preserves_exact_version_and_both_collection_orders() {
    for consumer_first in [false, true] {
        let mut h = CopyHarness::new();
        let source = h.resource(false, &[17, 83, 149, 231, 9]);
        let middle = h.resource(false, &[1; 5]);
        let destination = h.resource(false, &[2; 5]);
        h.record(
            0,
            source,
            middle,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let a = h.submit(0, &[]).unwrap();
        let point = h.ctx.submission_point(a).unwrap();
        h.record(
            1,
            middle,
            destination,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_SOURCE,
        );
        assert!(h.submit(1, &[]).is_err());
        let b = h.submit(1, &[point]).unwrap();
        {
            let backend = h.backend.lock().unwrap();
            let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[1] else {
                panic!("copy expected");
            };
            assert_eq!(request.waits, [point]);
            assert!(
                matches!(request.source, GraphicsCopyInput::Produced(version) if version == (GraphicsProducedVersion { point, output: GraphicsProducedOutput::Copy(0) }))
            );
            assert_eq!(request.layout, GraphicsCopyLayout::Buffer { byte_len: 5 });
        }
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 48); // native padding: 4*8 + 2*8
        h.complete(0, copy_bytes(&[17, 83, 149, 231, 9]));
        h.complete(1, copy_bytes(&[17, 83, 149, 231, 9]));
        if !consumer_first {
            h.ctx.wait_submission(a).unwrap();
        }
        assert_eq!(
            h.ctx.wait_submission(b).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        h.ctx.release_submission(a).unwrap();
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 48);
        h.ctx.release_submission(b).unwrap();
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.read(middle, 5), [17, 83, 149, 231, 9]);
        assert_eq!(h.read(destination, 5), [17, 83, 149, 231, 9]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn cpu_copy_wrong_completion_kind_layout_or_length_rolls_back_bytes_and_states() {
    for failure in 0..3 {
        let mut h = CopyHarness::new();
        let source = h.resource(false, &[17, 83, 149, 231]);
        let destination = h.resource(false, &[9; 4]);
        h.record(
            0,
            source,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let receipt = h.submit(0, &[]).unwrap();
        h.complete(
            0,
            match failure {
                0 => GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                    color_layouts: None,
                    colors: vec![vec![17; 4]],
                    depth: None,
                }),
                1 => GraphicsQueuedWorkOutput::Copy {
                    layout: GraphicsCopyLayout::Rgba8 { extent: (1, 1) },
                    bytes: vec![17; 4],
                },
                _ => GraphicsQueuedWorkOutput::Copy {
                    layout: GraphicsCopyLayout::Buffer { byte_len: 4 },
                    bytes: vec![17; 3],
                },
            },
        );
        assert_eq!(
            h.ctx.wait_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Failed
        );
        assert!(h.ctx.submission_copy_outcome(receipt).unwrap().is_none());
        h.ctx.release_submission(receipt).unwrap();
        h.reset(0);
        // COPY_DEST->COPY_SOURCE must still be valid; probe source snapshots the
        // old destination bytes without overwriting them or using private fields.
        let probe = h.resource(false, &[0; 4]);
        h.record(
            0,
            destination,
            probe,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let recovered = h.submit(0, &[]).unwrap();
        {
            let backend = h.backend.lock().unwrap();
            let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[1] else {
                panic!("copy expected");
            };
            assert!(
                matches!(&request.source, GraphicsCopyInput::Host(bytes) if bytes.as_ref() == [9; 4])
            );
            let GraphicsQueuedWorkRequest::Copy(first) = &backend.requests[0] else {
                panic!("copy expected");
            };
            assert!(request.generation > first.generation);
        }
        h.complete(1, copy_bytes(&[9; 4]));
        h.ctx.wait_submission(recovered).unwrap();
        h.ctx.release_submission(recovered).unwrap();
        assert_eq!(h.read(probe, 4), [9; 4]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn cpu_cached_copy_success_is_rejected_after_copy_or_render_producer_publication_failure() {
    for render in [false, true] {
        let mut h = CopyHarness::new();
        let source = h.resource(render, &[17, 83, 149, 231]);
        let middle = h.resource(render, &[9; 4]);
        let destination = h.resource(render, &[7; 4]);
        if render {
            h.ctx
                .cmd_resource_barrier(
                    h.lists[0],
                    middle,
                    ResourceStates::COPY_DEST,
                    ResourceStates::RENDER_TARGET,
                )
                .unwrap();
            h.ctx
                .cmd_set_color_targets(h.lists[0], &[middle], &[[17, 83, 149, 231]])
                .unwrap();
            h.ctx
                .cmd_resource_barrier(
                    h.lists[0],
                    middle,
                    ResourceStates::RENDER_TARGET,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
            h.ctx.close_command_list(h.lists[0]).unwrap();
        } else {
            h.record(
                0,
                source,
                middle,
                ResourceStates::COPY_DEST,
                ResourceStates::COPY_SOURCE,
            );
        }
        let a = h.submit(0, &[]).unwrap();
        let point = h.ctx.submission_point(a).unwrap();
        h.record(
            1,
            middle,
            destination,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_SOURCE,
        );
        let b = h.submit(1, &[point]).unwrap();
        let layout = if render {
            GraphicsCopyLayout::Rgba8 { extent: (1, 1) }
        } else {
            GraphicsCopyLayout::Buffer { byte_len: 4 }
        };
        // Consumer has already returned success; rejection notification cannot
        // retract this cached result. Loader must still refuse its publication.
        h.complete(
            1,
            GraphicsQueuedWorkOutput::Copy {
                layout,
                bytes: vec![17, 83, 149, 231],
            },
        );
        h.complete(
            0,
            if render {
                GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                    color_layouts: None,
                    colors: vec![vec![17; 3]],
                    depth: None,
                })
            } else {
                GraphicsQueuedWorkOutput::Copy {
                    layout,
                    bytes: vec![17; 3],
                }
            },
        );
        assert_eq!(
            h.ctx.wait_submission(b).unwrap(),
            GraphicsSubmissionStatus::Failed
        );
        assert_eq!(
            h.ctx.wait_submission(a).unwrap(),
            GraphicsSubmissionStatus::Failed
        );
        assert!(
            h.backend.lock().unwrap().tickets[1]
                .lock()
                .unwrap()
                .rejected
                .contains(&point)
        );
        h.ctx.release_submission(a).unwrap();
        h.ctx.release_submission(b).unwrap();
        h.reset(0);
        h.reset(1);
        let probe = h.resource(render, &[0; 4]);
        h.record(
            0,
            destination,
            probe,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let c = h.submit(0, &[]).unwrap();
        {
            let backend = h.backend.lock().unwrap();
            let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[2] else {
                panic!("copy expected");
            };
            assert!(
                matches!(&request.source, GraphicsCopyInput::Host(bytes) if bytes.as_ref() == [7; 4])
            );
        }
        h.complete(
            2,
            GraphicsQueuedWorkOutput::Copy {
                layout,
                bytes: vec![7; 4],
            },
        );
        h.ctx.wait_submission(c).unwrap();
        h.ctx.release_submission(c).unwrap();
        assert_eq!(h.read(probe, 4), [7; 4]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn cpu_copy_retirement_survives_reset_destroy_and_does_not_resurrect_output() {
    let mut h = CopyHarness::new();
    let source = h.resource(false, &[17; 5]);
    let middle = h.resource(false, &[0; 5]);
    let dest = h.resource(false, &[0; 5]);
    // Reserve logical storage without allocating large host snapshots.
    for _ in 0..63 {
        h.ctx.create_depth_resource(h.device, (1024, 1024)).unwrap();
    }
    let buffer = |size| ResourceDesc {
        dimension: ResourceDimension::Buffer,
        width: size as u64,
        height: 1,
        depth_or_array_size: 1,
        mip_levels: 1,
        format_rgba8: false,
    };
    h.ctx
        .create_committed_resource(
            h.device,
            buffer(MAX_CONTEXT_RESOURCE_BYTES - 63 * 1024 * 1024 * 4 - 15),
            ResourceStates::COMMON,
            None,
        )
        .unwrap();
    h.record(
        0,
        source,
        middle,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    let a = h.submit(0, &[]).unwrap();
    let point = h.ctx.submission_point(a).unwrap();
    h.record(
        1,
        middle,
        dest,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_SOURCE,
    );
    let b = h.submit(1, &[point]).unwrap();
    assert!(
        h.ctx
            .copy_guest_to_resource(source, 0, 0x1000, 1, &h.memory)
            .is_err()
    );
    assert!(h.ctx.destroy_resource(middle).is_err());
    h.reset(0);
    h.reset(1);
    for resource in [source, middle, dest] {
        h.ctx.destroy_resource(resource).unwrap();
    }
    h.complete(0, copy_bytes(&[17; 5]));
    h.complete(1, copy_bytes(&[17; 5]));
    assert_eq!(
        h.ctx.wait_submission(b).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(a).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 48);
    assert!(
        h.ctx
            .create_committed_resource(h.device, buffer(6), ResourceStates::COMMON, None)
            .is_err()
    );
    h.ctx.release_submission(b).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.ctx
        .create_committed_resource(h.device, buffer(6), ResourceStates::COMMON, None)
        .unwrap();
    for resource in [source, middle, dest] {
        assert!(h.ctx.destroy_resource(resource).is_err());
    }
}

#[test]
fn cpu_copied_texture_load_uses_typed_native_version_and_render_completion() {
    let mut h = CopyHarness::new();
    let source = h.resource(true, &[17, 83, 149, 231, 211, 43, 97, 127]);
    let destination = h.resource(true, &[0; 8]);
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_DEST,
        ResourceStates::RENDER_TARGET,
    );
    let a = h.submit(0, &[]).unwrap();
    let point = h.ctx.submission_point(a).unwrap();
    h.ctx
        .cmd_set_color_targets_with_loads(h.lists[1], &[destination], &[GraphicsColorLoad::Load])
        .unwrap();
    h.ctx
        .cmd_resource_barrier(
            h.lists[1],
            destination,
            ResourceStates::RENDER_TARGET,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(h.lists[1]).unwrap();
    let b = h.submit(1, &[point]).unwrap();
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Render(request) = &backend.requests[1] else {
            panic!("render expected");
        };
        assert_eq!(request.extent, (2, 1));
        assert!(
            matches!(request.colors[0], GraphicsQueuedColorInput::Produced(version)
            if version == (GraphicsProducedVersion { point, output: GraphicsProducedOutput::Copy(0) }))
        );
    }
    h.complete(
        0,
        GraphicsQueuedWorkOutput::Copy {
            layout: GraphicsCopyLayout::Rgba8 { extent: (2, 1) },
            bytes: vec![17, 83, 149, 231, 211, 43, 97, 127],
        },
    );
    h.complete(
        1,
        GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
            color_layouts: None,
            colors: vec![vec![17, 83, 149, 231, 211, 43, 97, 127]],
            depth: None,
        }),
    );
    assert_eq!(
        h.ctx.wait_submission(b).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    assert!(h.ctx.submission_copy_outcome(b).unwrap().is_none());
    assert!(h.ctx.submission_outcome(b).unwrap().is_some());
    h.ctx.release_submission(a).unwrap();
    h.ctx.release_submission(b).unwrap();
    assert_eq!(h.read(destination, 8), [17, 83, 149, 231, 211, 43, 97, 127]);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 1);
}

#[test]
fn cpu_copy_old_backend_installers_revoke_capability_before_dispatch() {
    for installer in 0..4 {
        let mut h = CopyHarness::new();
        let source = h.resource(false, &[17; 5]);
        let destination = h.resource(false, &[9; 5]);
        h.record(
            0,
            source,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        match installer {
            0 => h
                .ctx
                .set_metal_queued_backend(|_| panic!("legacy queued copy dispatch")),
            1 => h.ctx.set_metal_produced_attachment_queued_backend(
                GraphicsTextureCapabilities::default(),
                |_| panic!("legacy attachment copy dispatch"),
            ),
            2 => h
                .ctx
                .set_metal_async_backend(|_| panic!("legacy async copy dispatch")),
            _ => h
                .ctx
                .set_metal_attachment_backend(|_, _, _, _, _| panic!("legacy sync copy dispatch")),
        }
        let marker = h.ctx.create_device().unwrap();
        assert!(h.submit(0, &[]).is_err());
        assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert!(h.backend.lock().unwrap().requests.is_empty());
    }
}

#[test]
fn cpu_copy_invalid_recorded_mix_and_state_do_not_dispatch_or_consume_receipt() {
    for invalid in 0..3 {
        let mut h = CopyHarness::new();
        let source = h.resource(false, &[17; 5]);
        let destination = h.resource(false, &[9; 5]);
        let list = h.lists[0];
        if invalid != 0 {
            h.ctx
                .cmd_resource_barrier(
                    list,
                    source,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
        }
        h.ctx.cmd_copy_resource(list, destination, source).unwrap();
        if invalid == 1 {
            h.ctx.cmd_copy_resource(list, destination, source).unwrap();
        }
        if invalid == 2 {
            h.ctx.cmd_set_viewport(list, 1, 1).unwrap();
        }
        h.ctx.close_command_list(list).unwrap();
        let marker = h.ctx.create_device().unwrap();
        assert!(h.submit(0, &[]).is_err());
        assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
        assert!(h.backend.lock().unwrap().requests.is_empty());
        h.reset(0);
        h.record(
            0,
            source,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let receipt = h.submit(0, &[]).unwrap();
        assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
        h.complete(0, copy_bytes(&[17; 5]));
        h.ctx.wait_submission(receipt).unwrap();
        h.ctx.release_submission(receipt).unwrap();
    }
}

#[test]
fn cpu_copy_publication_does_not_acquire_poisoned_presentation_lock() {
    let mut h = CopyHarness::new();
    let source = h.resource(false, &[17; 5]);
    let destination = h.resource(false, &[9; 5]);
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    let presentation = h.ctx.last_presentation.clone();
    assert!(
        std::panic::catch_unwind(move || {
            let _guard = presentation.lock().unwrap();
            panic!("controlled presentation lock poison");
        })
        .is_err()
    );
    let receipt = h.submit(0, &[]).unwrap();
    h.complete(0, copy_bytes(&[17; 5]));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 5), [17; 5]);
}

#[test]
fn cpu_pending_copy_geometry_cannot_be_materialized_from_stale_host_bytes() {
    for binding in 0..3 {
        let mut h = CopyHarness::new();
        let source = h.resource(false, &[0; 48]);
        let pending = h.resource(false, &[0; 48]);
        let vertices = h.resource(false, &[0; 48]);
        let colors = h.resource(false, &[0; 48]);
        let target = h.resource(true, &[0; 4]);
        let final_state = if binding == 2 {
            ResourceStates::INDEX_BUFFER
        } else {
            ResourceStates::VERTEX_AND_CONSTANT_BUFFER
        };
        h.record(0, source, pending, ResourceStates::COPY_DEST, final_state);
        let a = h.submit(0, &[]).unwrap();
        let point = h.ctx.submission_point(a).unwrap();
        let list = h.lists[1];
        h.ctx
            .cmd_resource_barrier(
                list,
                target,
                ResourceStates::COPY_DEST,
                ResourceStates::RENDER_TARGET,
            )
            .unwrap();
        h.ctx
            .cmd_set_color_targets(list, &[target], &[[0; 4]])
            .unwrap();
        h.ctx.cmd_set_viewport(list, 1, 1).unwrap();
        let pipeline = h
            .ctx
            .create_graphics_pipeline(
                h.device,
                include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin"),
                include_bytes!("../../../fixtures/rdna2/x9-pixel-arithmetic-ps.bin"),
            )
            .unwrap();
        h.ctx
            .cmd_set_graphics_pipeline(list, Some(pipeline))
            .unwrap();
        h.ctx
            .cmd_resource_barrier(
                list,
                vertices,
                ResourceStates::COPY_DEST,
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
            )
            .unwrap();
        h.ctx
            .cmd_resource_barrier(
                list,
                colors,
                ResourceStates::COPY_DEST,
                ResourceStates::VERTEX_AND_CONSTANT_BUFFER,
            )
            .unwrap();
        h.ctx
            .cmd_set_vertex_buffer(list, if binding == 0 { pending } else { vertices }, 0, 16)
            .unwrap();
        h.ctx
            .cmd_set_vertex_colors(list, if binding == 1 { pending } else { colors }, 0, 16)
            .unwrap();
        if binding == 2 {
            h.ctx.cmd_set_index_buffer(list, pending, 0, 57).unwrap();
            h.ctx
                .cmd_draw_indexed_instanced(list, 3, 1, 0, 0, 0)
                .unwrap();
        } else {
            h.ctx.cmd_draw_instanced(list, 3, 1, 0, 0).unwrap();
        }
        h.ctx.close_command_list(list).unwrap();
        assert!(h.submit(1, &[point]).is_err());
        assert_eq!(h.backend.lock().unwrap().requests.len(), 1);
        h.complete(0, copy_bytes(&[0; 48]));
        h.ctx.wait_submission(a).unwrap();
        h.ctx.release_submission(a).unwrap();
        // Once collected/released, the exact same recorded draw can use committed
        // host bytes. This proves the rejection was the in-flight version boundary.
        let b = h.submit(1, &[]).unwrap();
        assert_eq!(h.backend.lock().unwrap().requests.len(), 2);
        h.complete(
            1,
            GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                color_layouts: None,
                colors: vec![vec![0; 4]],
                depth: None,
            }),
        );
        h.ctx.wait_submission(b).unwrap();
        h.ctx.release_submission(b).unwrap();
    }
}

impl CopyHarness {
    fn record_clear(&mut self, slot: usize, target: GraphicsHandle) {
        let list = self.lists[slot];
        self.ctx
            .cmd_resource_barrier(
                list,
                target,
                ResourceStates::COPY_DEST,
                ResourceStates::RENDER_TARGET,
            )
            .unwrap();
        self.ctx
            .cmd_set_color_targets(list, &[target], &[[17, 83, 149, 231]])
            .unwrap();
        self.ctx
            .cmd_resource_barrier(
                list,
                target,
                ResourceStates::RENDER_TARGET,
                ResourceStates::COPY_SOURCE,
            )
            .unwrap();
        self.ctx.close_command_list(list).unwrap();
    }
}

#[test]
fn cpu_newer_independent_copy_never_suppresses_older_render_publication() {
    for copy_first in [false, true] {
        let mut h = CopyHarness::new();
        let target = h.resource(true, &[9; 4]);
        let source = h.resource(false, &[31, 47, 61, 79, 97]);
        let destination = h.resource(false, &[7; 5]);
        h.record_clear(0, target);
        let render = h.submit(0, &[]).unwrap();
        h.record(
            1,
            source,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        );
        let copy = h.submit(1, &[]).unwrap();
        {
            let backend = h.backend.lock().unwrap();
            let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[1] else {
                panic!("copy expected");
            };
            assert!(
                request.waits.is_empty(),
                "independent resources must not acquire a render dependency"
            );
        }
        h.complete(
            0,
            GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                color_layouts: None,
                colors: vec![vec![17, 83, 149, 231]],
                depth: None,
            }),
        );
        h.complete(1, copy_bytes(&[31, 47, 61, 79, 97]));
        if copy_first {
            assert_eq!(
                h.ctx.wait_submission(copy).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
            assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
            assert_eq!(
                h.ctx.wait_submission(render).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
        } else {
            assert_eq!(
                h.ctx.wait_submission(render).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
            assert_eq!(
                h.ctx.wait_submission(copy).unwrap(),
                GraphicsSubmissionStatus::Complete
            );
        }
        {
            let presentation = h.ctx.last_presentation.lock().unwrap();
            assert_eq!(presentation.frame_index, 1);
            assert_eq!((presentation.width, presentation.height), (1, 1));
            assert_eq!(presentation.pixels, [17, 83, 149, 231]);
        }
        h.ctx.release_submission(copy).unwrap();
        h.ctx.release_submission(render).unwrap();
        assert_eq!(h.read(destination, 5), [31, 47, 61, 79, 97]);
        assert_eq!(h.read(target, 4), [17, 83, 149, 231]);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.ctx.queued_color_allocation_bytes(), 0);
    }
}

#[test]
fn cpu_copy_backend_admission_error_preserves_storage_states_handle_and_point_for_retry() {
    let mut h = CopyHarness::new();
    let source = h.resource(false, &[17, 83, 149, 231, 9]);
    let destination = h.resource(false, &[7; 5]);
    let probe = h.resource(false, &[0; 5]);
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = attempts.clone();
    let captured = h.backend.clone();
    h.ctx
        .set_metal_copy_queued_backend(GraphicsTextureCapabilities::default(), move |request| {
            if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return Err(GraphicsError::BackendError(
                    "controlled precommit admission failure".into(),
                ));
            }
            let state = Arc::new(Mutex::new(ControlledState::default()));
            let mut backend = captured.lock().unwrap();
            backend.requests.push(request);
            backend.tickets.push(state.clone());
            Ok(Box::new(ControlledTicket(state)))
        });
    let marker = h.ctx.create_device().unwrap();
    assert!(h.submit(0, &[]).is_err());
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(h.ctx.queued_color_allocation_bytes(), 0);
    // A real second list proves old destination bytes and COPY_DEST survived.
    // Restore its state afterward so the unchanged original list can retry.
    let list = h.lists[1];
    h.ctx
        .cmd_resource_barrier(
            list,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.cmd_copy_resource(list, probe, destination).unwrap();
    h.ctx
        .cmd_resource_barrier(
            list,
            destination,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_DEST,
        )
        .unwrap();
    h.ctx
        .cmd_resource_barrier(
            list,
            probe,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(list).unwrap();
    let check = h.submit(1, &[]).unwrap();
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[0] else {
            panic!("copy expected");
        };
        assert!(
            matches!(&request.source, GraphicsCopyInput::Host(bytes) if bytes.as_ref() == [7; 5])
        );
    }
    h.complete(0, copy_bytes(&[7; 5]));
    h.ctx.wait_submission(check).unwrap();
    h.ctx.release_submission(check).unwrap();
    assert_eq!(h.read(probe, 5), [7; 5]);
    let retry = h.submit(0, &[]).unwrap();
    assert_eq!(h.ctx.submission_point(retry).unwrap().value, 1);
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[1] else {
            panic!("copy expected");
        };
        assert!(
            matches!(&request.source, GraphicsCopyInput::Host(bytes) if bytes.as_ref() == [17, 83, 149, 231, 9])
        );
    }
    h.complete(1, copy_bytes(&[17, 83, 149, 231, 9]));
    h.ctx.wait_submission(retry).unwrap();
    h.ctx.release_submission(retry).unwrap();
    assert_eq!(h.read(destination, 5), [17, 83, 149, 231, 9]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_shared_copy_render_payload_ceiling_rejects_before_dispatch_then_recovers() {
    // One 64 MiB host copy reserves 256 MiB including padded upload/scratch and
    // output. Resource storage + captured host snapshot + completion stays near
    // 256 MiB; never create a second expected buffer of that size.
    for copy_first in [false, true] {
        let mut h = CopyHarness::new();
        let desc = ResourceDesc {
            dimension: ResourceDimension::Buffer,
            width: 64 * 1024 * 1024,
            height: 1,
            depth_or_array_size: 1,
            mip_levels: 1,
            format_rgba8: false,
        };
        let source = h
            .ctx
            .create_committed_resource(h.device, desc.clone(), ResourceStates::COPY_SOURCE, None)
            .unwrap();
        let destination = h
            .ctx
            .create_committed_resource(h.device, desc, ResourceStates::COPY_DEST, None)
            .unwrap();
        let target = h.resource(true, &[9; 4]);
        h.record_clear(0, target);
        h.record(
            1,
            source,
            destination,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_SOURCE,
        );
        let (first_slot, second_slot) = if copy_first { (1, 0) } else { (0, 1) };
        let first = h.submit(first_slot, &[]).unwrap();
        let marker = h.ctx.create_device().unwrap();
        assert!(h.submit(second_slot, &[]).is_err());
        assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
        assert_eq!(h.backend.lock().unwrap().requests.len(), 1);
        assert_eq!(
            h.ctx.queued_copy_allocation_bytes(),
            if copy_first { 256 * 1024 * 1024 } else { 0 }
        );
        assert_eq!(
            h.ctx.queued_color_allocation_bytes(),
            if copy_first { 0 } else { 12 }
        );
        let completion = |copy| {
            if copy {
                GraphicsQueuedWorkOutput::Copy {
                    layout: GraphicsCopyLayout::Buffer {
                        byte_len: 64 * 1024 * 1024,
                    },
                    bytes: vec![0; 64 * 1024 * 1024],
                }
            } else {
                GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
                    color_layouts: None,
                    colors: vec![vec![17, 83, 149, 231]],
                    depth: None,
                })
            }
        };
        h.complete(0, completion(copy_first));
        h.ctx.wait_submission(first).unwrap();
        h.ctx.release_submission(first).unwrap();
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.ctx.queued_color_allocation_bytes(), 0);
        // Release captured host bytes too, keeping the test's peak bounded.
        h.backend.lock().unwrap().requests.clear();
        let second = h.submit(second_slot, &[]).unwrap();
        assert_eq!(h.ctx.submission_point(second).unwrap().value, 1);
        h.complete(1, completion(!copy_first));
        h.ctx.wait_submission(second).unwrap();
        h.ctx.release_submission(second).unwrap();
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.ctx.queued_color_allocation_bytes(), 0);
        assert_eq!(h.read(destination, 1), [0]);
        assert_eq!(h.read(target, 4), [17, 83, 149, 231]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 1);
    }
}

#[test]
fn cpu_cached_render_success_cannot_publish_after_copy_producer_publication_failure() {
    let mut h = CopyHarness::new();
    let source = h.resource(true, &[17, 83, 149, 231]);
    let target = h.resource(true, &[9; 4]);
    h.record(
        0,
        source,
        target,
        ResourceStates::COPY_DEST,
        ResourceStates::RENDER_TARGET,
    );
    let a = h.submit(0, &[]).unwrap();
    let point = h.ctx.submission_point(a).unwrap();
    h.ctx
        .cmd_set_color_targets_with_loads(h.lists[1], &[target], &[GraphicsColorLoad::Load])
        .unwrap();
    h.ctx
        .cmd_resource_barrier(
            h.lists[1],
            target,
            ResourceStates::RENDER_TARGET,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(h.lists[1]).unwrap();
    let b = h.submit(1, &[point]).unwrap();
    h.complete(
        1,
        GraphicsQueuedWorkOutput::Render(GraphicsPassOutput {
            color_layouts: None,
            colors: vec![vec![31, 47, 61, 79]],
            depth: None,
        }),
    );
    // Same byte count, wrong storage kind: cached downstream render is invalid.
    h.complete(
        0,
        GraphicsQueuedWorkOutput::Copy {
            layout: GraphicsCopyLayout::Buffer { byte_len: 4 },
            bytes: vec![17, 83, 149, 231],
        },
    );
    assert_eq!(
        h.ctx.wait_submission(b).unwrap(),
        GraphicsSubmissionStatus::Failed
    );
    assert_eq!(
        h.ctx.wait_submission(a).unwrap(),
        GraphicsSubmissionStatus::Failed
    );
    assert!(h.ctx.submission_outcome(b).unwrap().is_none());
    assert!(
        h.backend.lock().unwrap().tickets[1]
            .lock()
            .unwrap()
            .rejected
            .contains(&point)
    );
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    h.ctx.release_submission(a).unwrap();
    h.ctx.release_submission(b).unwrap();
    h.reset(0);
    h.reset(1);
    let probe = h.resource(true, &[0; 4]);
    h.record(
        0,
        target,
        probe,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    let c = h.submit(0, &[]).unwrap();
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[2] else {
            panic!("copy expected");
        };
        assert!(
            matches!(&request.source, GraphicsCopyInput::Host(bytes) if bytes.as_ref() == [9; 4])
        );
    }
    h.complete(
        2,
        GraphicsQueuedWorkOutput::Copy {
            layout: GraphicsCopyLayout::Rgba8 { extent: (1, 1) },
            bytes: vec![9; 4],
        },
    );
    h.ctx.wait_submission(c).unwrap();
    h.ctx.release_submission(c).unwrap();
    assert_eq!(h.read(probe, 4), [9; 4]);
}

#[test]
fn cpu_buffer_region_preserves_literal_destination_and_odd_tail_baseline() {
    let mut h = CopyHarness::new();
    h.ctx
        .set_metal_buffer_region_queued_backend(GraphicsTextureCapabilities::default(), |work| {
            let GraphicsQueuedWorkRequest::BufferRegion(request) = work else {
                panic!("CPU-only region callback received other work")
            };
            assert_eq!(request.source_byte_len, 5);
            assert_eq!(request.destination_byte_len, 9);
            assert_eq!(
                (
                    request.source_offset,
                    request.destination_offset,
                    request.byte_len
                ),
                (0, 4, 4)
            );
            assert!(request.waits.is_empty());
            assert_eq!(request.output.point, request.point);
            assert_eq!(request.output.output, GraphicsProducedOutput::Copy(0));
            let GraphicsCopyInput::Host(source) = request.source else {
                panic!("host source")
            };
            let GraphicsCopyInput::Host(destination) = request.destination else {
                panic!("host destination")
            };
            assert_eq!(&*source, &[17, 83, 149, 231, 9]);
            assert_eq!(&*destination, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
            Ok(Box::new(CpuTicket(Some(copy_bytes(&[
                1, 2, 3, 4, 17, 83, 149, 231, 99,
            ])))))
        });
    let source = h.resource(false, &[17, 83, 149, 231, 9]);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
    let list = h.lists[0];
    h.ctx
        .cmd_resource_barrier(
            list,
            source,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx
        .cmd_copy_buffer_region(list, destination, 4, source, 0, 4)
        .unwrap();
    h.ctx
        .cmd_resource_barrier(
            list,
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(list).unwrap();
    let receipt = h
        .submit(0, &[])
        .expect("valid aligned buffer region must be admitted");
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    assert!(h.ctx.submission_outcome(receipt).unwrap().is_none());
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 9), [1, 2, 3, 4, 17, 83, 149, 231, 99]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
}

impl CopyHarness {
    fn enable_regions(&mut self) {
        let captured = self.backend.clone();
        self.ctx.set_metal_buffer_region_queued_backend(
            GraphicsTextureCapabilities::default(),
            move |request| {
                let state = Arc::new(Mutex::new(ControlledState::default()));
                let mut backend = captured.lock().unwrap();
                backend.requests.push(request);
                backend.tickets.push(state.clone());
                Ok(Box::new(ControlledTicket(state)))
            },
        );
    }
    fn record_region(
        &mut self,
        slot: usize,
        source: GraphicsHandle,
        destination: GraphicsHandle,
        source_state: ResourceStates,
    ) {
        let list = self.lists[slot];
        if source_state != ResourceStates::COPY_SOURCE {
            self.ctx
                .cmd_resource_barrier(list, source, source_state, ResourceStates::COPY_SOURCE)
                .unwrap();
        }
        self.ctx
            .cmd_copy_buffer_region(list, destination, 4, source, 0, 4)
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
    }
}

#[test]
fn cpu_region_each_produced_input_exact_version_orders_and_failed_ancestor() {
    for destination_produced in [false, true] {
        for consumer_first in [false, true] {
            for producer_bad in [false, true] {
                let mut h = CopyHarness::new();
                h.enable_regions();
                let source = h.resource(false, &[17, 83, 149, 231, 9]);
                let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
                let seed_bytes: &[u8] = if destination_produced {
                    &[21, 22, 23, 24, 25, 26, 27, 28, 29]
                } else {
                    &[31, 47, 61, 79, 97]
                };
                let seed = h.resource(false, seed_bytes);
                let changed = if destination_produced {
                    destination
                } else {
                    source
                };
                h.record(
                    0,
                    seed,
                    changed,
                    ResourceStates::COPY_DEST,
                    if destination_produced {
                        ResourceStates::COPY_DEST
                    } else {
                        ResourceStates::COPY_SOURCE
                    },
                );
                let a = h.submit(0, &[]).unwrap();
                let point = h.ctx.submission_point(a).unwrap();
                h.record_region(
                    1,
                    source,
                    destination,
                    if destination_produced {
                        ResourceStates::COPY_DEST
                    } else {
                        ResourceStates::COPY_SOURCE
                    },
                );
                assert!(
                    h.submit(1, &[]).is_err(),
                    "pending input cannot fall back to stale host bytes"
                );
                assert_eq!(h.backend.lock().unwrap().requests.len(), 1);
                let producer_reservation = h.ctx.queued_copy_allocation_bytes();
                assert_eq!(
                    producer_reservation,
                    if destination_produced { 48 } else { 32 }
                );
                let b = h.submit(1, &[point]).unwrap();
                assert_eq!(
                    h.ctx.queued_copy_allocation_bytes() - producer_reservation,
                    if destination_produced { 40 } else { 48 },
                    "region charge includes only host uploads plus fresh destination and scratch"
                );
                {
                    let backend = h.backend.lock().unwrap();
                    let GraphicsQueuedWorkRequest::BufferRegion(request) = &backend.requests[1]
                    else {
                        panic!("region")
                    };
                    assert_eq!(request.waits, [point]);
                    assert_eq!(
                        (request.source_byte_len, request.destination_byte_len),
                        (5, 9)
                    );
                    let (produced, host) = if destination_produced {
                        (&request.destination, &request.source)
                    } else {
                        (&request.source, &request.destination)
                    };
                    let GraphicsCopyInput::Produced(version) = produced else {
                        panic!("exact producer required")
                    };
                    assert_eq!(
                        *version,
                        GraphicsProducedVersion {
                            point,
                            output: GraphicsProducedOutput::Copy(0)
                        }
                    );
                    let GraphicsCopyInput::Host(bytes) = host else {
                        panic!("independent host input")
                    };
                    assert_eq!(
                        bytes.as_ref(),
                        if destination_produced {
                            &[17, 83, 149, 231, 9][..]
                        } else {
                            &[1, 2, 3, 4, 5, 6, 7, 8, 99][..]
                        }
                    );
                }
                let expected: &[u8] = if destination_produced {
                    &[21, 22, 23, 24, 17, 83, 149, 231, 29]
                } else {
                    &[1, 2, 3, 4, 31, 47, 61, 79, 99]
                };
                h.complete(1, copy_bytes(expected)); // Cached success must still depend on publication.
                h.complete(
                    0,
                    if producer_bad {
                        copy_bytes(&[0])
                    } else {
                        copy_bytes(seed_bytes)
                    },
                );
                if !consumer_first {
                    h.ctx.wait_submission(a).unwrap();
                }
                assert_eq!(
                    h.ctx.wait_submission(b).unwrap(),
                    if producer_bad {
                        GraphicsSubmissionStatus::Failed
                    } else {
                        GraphicsSubmissionStatus::Complete
                    }
                );
                if producer_bad {
                    assert!(
                        !h.backend.lock().unwrap().tickets[1]
                            .lock()
                            .unwrap()
                            .rejected
                            .is_empty()
                    );
                    assert!(h.ctx.submission_copy_outcome(b).unwrap().is_none());
                    // Both original COPY_DEST states and original destination bytes survive.
                    h.ctx.release_submission(b).unwrap();
                    h.ctx.release_submission(a).unwrap();
                    h.reset(0);
                    h.reset(1);
                    let probe = h.resource(false, &[0; 9]);
                    h.record(
                        0,
                        destination,
                        probe,
                        ResourceStates::COPY_DEST,
                        ResourceStates::COPY_SOURCE,
                    );
                    let recovery = h.submit(0, &[]).unwrap();
                    let backend = h.backend.lock().unwrap();
                    let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[2] else {
                        panic!("copy")
                    };
                    let GraphicsCopyInput::Host(bytes) = &request.source else {
                        panic!("host rollback")
                    };
                    assert_eq!(bytes.as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
                    drop(backend);
                    h.complete(2, copy_bytes(&[1, 2, 3, 4, 5, 6, 7, 8, 99]));
                    h.ctx.wait_submission(recovery).unwrap();
                    h.ctx.release_submission(recovery).unwrap();
                } else {
                    h.ctx.wait_submission(a).unwrap();
                    h.ctx.release_submission(b).unwrap();
                    h.ctx.release_submission(a).unwrap();
                    assert_eq!(h.read(destination, 9), expected);
                }
                assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
                assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
            }
        }
    }
}

#[test]
fn cpu_region_bad_full_output_rolls_back_bytes_and_ordered_states() {
    for bad in [
        copy_bytes(&[0; 8]),
        GraphicsQueuedWorkOutput::Copy {
            layout: GraphicsCopyLayout::Rgba8 { extent: (1, 2) },
            bytes: vec![0; 8],
        },
    ] {
        let mut h = CopyHarness::new();
        h.enable_regions();
        let source = h.resource(false, &[17, 83, 149, 231, 9]);
        let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
        h.record_region(0, source, destination, ResourceStates::COPY_DEST);
        let receipt = h.submit(0, &[]).unwrap();
        h.complete(0, bad);
        assert_eq!(
            h.ctx.wait_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Failed
        );
        h.ctx.release_submission(receipt).unwrap();
        h.reset(0);
        h.record_region(0, source, destination, ResourceStates::COPY_DEST);
        let retry = h.submit(0, &[]).unwrap();
        {
            let backend = h.backend.lock().unwrap();
            let GraphicsQueuedWorkRequest::BufferRegion(request) = &backend.requests[1] else {
                panic!("region")
            };
            let GraphicsCopyInput::Host(bytes) = &request.destination else {
                panic!("host")
            };
            assert_eq!(bytes.as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
        }
        h.complete(1, copy_bytes(&[1, 2, 3, 4, 17, 83, 149, 231, 99]));
        h.ctx.wait_submission(retry).unwrap();
        h.ctx.release_submission(retry).unwrap();
        assert_eq!(h.read(destination, 9), [1, 2, 3, 4, 17, 83, 149, 231, 99]);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn cpu_region_retirement_retains_payload_until_release_without_resurrection() {
    let mut h = CopyHarness::new();
    h.enable_regions();
    let source = h.resource(false, &[17, 83, 149, 231, 9]);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
    h.record_region(0, source, destination, ResourceStates::COPY_DEST);
    let receipt = h.submit(0, &[]).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 64);
    assert!(h.ctx.destroy_resource(destination).is_err());
    h.reset(0);
    h.ctx.destroy_resource(source).unwrap();
    h.ctx.destroy_resource(destination).unwrap();
    h.complete(0, copy_bytes(&[1, 2, 3, 4, 17, 83, 149, 231, 99]));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 64);
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert!(h.ctx.destroy_resource(destination).is_err());
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_region_capability_downgrade_rejects_region_but_preserves_whole_copy() {
    let mut h = CopyHarness::new();
    h.enable_regions();
    let source = h.resource(false, &[17; 9]);
    let destination = h.resource(false, &[0; 9]);
    h.record_region(0, source, destination, ResourceStates::COPY_DEST);
    h.ctx
        .set_metal_copy_queued_backend(GraphicsTextureCapabilities::default(), |request| {
            let GraphicsQueuedWorkRequest::Copy(_) = request else {
                panic!("region reached old backend")
            };
            Ok(Box::new(CpuTicket(Some(copy_bytes(&[17; 9])))))
        });
    assert!(matches!(
        h.submit(0, &[]),
        Err(GraphicsError::NativeMetalRequired)
    ));
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.reset(0);
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_DEST,
        ResourceStates::COPY_SOURCE,
    );
    let receipt = h.submit(0, &[]).unwrap();
    assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
    h.ctx.wait_submission(receipt).unwrap();
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 9), [17; 9]);
}

#[test]
fn cpu_region_admission_failure_preserves_point_handle_budget_and_inputs() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut h = CopyHarness::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    h.ctx.set_metal_buffer_region_queued_backend(
        GraphicsTextureCapabilities::default(),
        move |work| {
            let GraphicsQueuedWorkRequest::BufferRegion(request) = work else {
                panic!("region")
            };
            assert_eq!(request.point.value, 1);
            let GraphicsCopyInput::Host(source) = request.source else {
                panic!("host")
            };
            let GraphicsCopyInput::Host(destination) = request.destination else {
                panic!("host")
            };
            assert_eq!(source.as_ref(), &[17, 83, 149, 231, 9]);
            assert_eq!(destination.as_ref(), &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
            if captured.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(GraphicsError::BackendError(
                    "controlled admission rejection".into(),
                ));
            }
            Ok(Box::new(CpuTicket(Some(copy_bytes(&[
                1, 2, 3, 4, 17, 83, 149, 231, 99,
            ])))))
        },
    );
    let source = h.resource(false, &[17, 83, 149, 231, 9]);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
    h.record_region(0, source, destination, ResourceStates::COPY_DEST);
    let marker = h.ctx.create_device().unwrap();
    assert!(h.submit(0, &[]).is_err());
    assert_eq!(h.ctx.create_device().unwrap().id, marker.id + 1);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    let receipt = h.submit(0, &[]).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 64);
    h.ctx.wait_submission(receipt).unwrap();
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 9), [1, 2, 3, 4, 17, 83, 149, 231, 99]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
}

#[test]
fn cpu_region_multiple_copies_and_invalid_ordered_states_reject_before_dispatch() {
    for (late_barrier, wrong_destination) in [(false, false), (true, false), (false, true)] {
        let mut h = CopyHarness::new();
        h.enable_regions();
        let source = h.resource(false, &[17; 9]);
        let destination = h.resource(false, &[0; 9]);
        let list = h.lists[0];
        if !late_barrier {
            h.ctx
                .cmd_resource_barrier(
                    list,
                    source,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
        }
        if wrong_destination {
            h.ctx
                .cmd_resource_barrier(
                    list,
                    destination,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
        }
        h.ctx
            .cmd_copy_buffer_region(list, destination, 4, source, 0, 4)
            .unwrap();
        if late_barrier {
            h.ctx
                .cmd_resource_barrier(
                    list,
                    source,
                    ResourceStates::COPY_DEST,
                    ResourceStates::COPY_SOURCE,
                )
                .unwrap();
        } else if !wrong_destination {
            h.ctx.cmd_copy_resource(list, destination, source).unwrap();
        }
        h.ctx.close_command_list(list).unwrap();
        assert!(h.submit(0, &[]).is_err());
        assert!(h.backend.lock().unwrap().requests.is_empty());
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    }
}

impl CopyHarness {
    fn guest_source(&mut self, address: u64, size: u64) -> GraphicsHandle {
        self.ctx
            .create_committed_resource(
                self.device,
                ResourceDesc {
                    dimension: ResourceDimension::Buffer,
                    width: size,
                    height: 1,
                    depth_or_array_size: 1,
                    mip_levels: 1,
                    format_rgba8: false,
                },
                ResourceStates::COPY_SOURCE,
                Some(address),
            )
            .unwrap()
    }
}

#[test]
fn cpu_guest_copy_snapshot_uses_submit_time_bytes_and_survives_source_destroy() {
    let mut h = CopyHarness::new();
    let source = h.guest_source(0x1800, 5);
    let destination = h.resource(false, &[91; 5]);
    h.memory.write_bytes(0x1800, &[1, 2, 3, 4, 5]).unwrap();
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_SOURCE,
    );
    let literal = [17, 29, 43, 61, 79];
    h.memory.write_bytes(0x1800, &literal).unwrap();
    let receipt = h.submit(0, &[]).unwrap();
    h.memory.write_bytes(0x1800, &[211; 5]).unwrap();
    h.reset(0);
    h.ctx.destroy_resource(source).unwrap();
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::Copy(request) = &backend.requests[0] else {
            panic!("whole copy")
        };
        let GraphicsCopyInput::Host(snapshot) = &request.source else {
            panic!("immutable host snapshot")
        };
        assert_eq!(&**snapshot, &literal);
        assert_eq!(request.layout, GraphicsCopyLayout::Buffer { byte_len: 5 });
    }
    h.complete(0, copy_bytes(&literal));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 5), literal);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_guest_copy_missing_or_unreadable_memory_rejects_without_consuming_admission() {
    for unreadable in [false, true] {
        let mut h = CopyHarness::new();
        let source = h.guest_source(0x6000, 5);
        let destination = h.resource(false, &[91; 5]);
        if unreadable {
            h.memory
                .map_region(
                    0x6000,
                    5,
                    MemoryPermissions {
                        read: false,
                        write: true,
                        execute: false,
                    },
                )
                .unwrap();
        }
        h.record(
            0,
            source,
            destination,
            ResourceStates::COPY_SOURCE,
            ResourceStates::COPY_SOURCE,
        ); // Does not read bytes.
        for _ in 0..3 {
            assert!(matches!(h.submit(0, &[]), Err(GraphicsError::Memory(_))));
        }
        assert!(h.backend.lock().unwrap().requests.is_empty());
        // Replace the memory map with a readable backing, preserving the same resource/list.
        h.memory = GuestMemory::new();
        h.memory
            .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
            .unwrap();
        h.memory
            .map_region_with_data(
                0x6000,
                5,
                MemoryPermissions::READ_ONLY,
                &[7, 11, 13, 17, 19],
            )
            .unwrap();
        let receipt = h.submit(0, &[]).unwrap();
        h.complete(0, copy_bytes(&[7, 11, 13, 17, 19]));
        assert_eq!(receipt.to_u64(), destination.to_u64() + 1);
        assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
        assert_eq!(
            h.ctx.wait_submission(receipt).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        h.ctx.release_submission(receipt).unwrap();
        assert_eq!(h.read(destination, 5), [7, 11, 13, 17, 19]);
    }
}

#[test]
fn cpu_guest_copy_invalid_completion_preserves_destination_bytes_and_state() {
    let mut h = CopyHarness::new();
    let source = h.guest_source(0x1800, 5);
    let destination = h.resource(false, &[91, 92, 93, 94, 95]);
    h.memory.write_bytes(0x1800, &[1, 2, 3, 4, 5]).unwrap();
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_SOURCE,
    );
    let receipt = h.submit(0, &[]).unwrap();
    h.complete(0, copy_bytes(&[1, 2, 3, 4]));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Failed
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    h.reset(0);
    h.ctx
        .cmd_resource_barrier(
            h.lists[0],
            destination,
            ResourceStates::COPY_DEST,
            ResourceStates::COPY_SOURCE,
        )
        .unwrap();
    h.ctx.close_command_list(h.lists[0]).unwrap();
    h.ctx
        .set_metal_backend(|_, _, _| panic!("CPU barrier-only list"));
    h.ctx
        .execute_command_list(h.queues[0], h.lists[0], &h.memory)
        .unwrap();
    assert_eq!(h.read(destination, 5), [91, 92, 93, 94, 95]);
}

#[test]
fn cpu_guest_copy_source_extension_does_not_admit_guest_destinations() {
    let mut h = CopyHarness::new();
    let guest = h.guest_source(0x1800, 16);
    let owned = h.resource(false, &[0; 16]);
    let texture = h.resource(true, &[0; 16]);
    let list = h.lists[0];
    assert!(h.ctx.cmd_copy_resource(list, guest, owned).is_err());
    assert!(
        h.ctx
            .cmd_copy_buffer_region(list, guest, 0, owned, 0, 4)
            .is_err()
    );
    assert!(
        h.ctx
            .cmd_copy_image(
                list,
                guest,
                GraphicsImageCopyLocation::Buffer {
                    offset: 0,
                    bytes_per_row: 4
                },
                texture,
                GraphicsImageCopyLocation::Rgba8 { origin: (0, 0) },
                (1, 1)
            )
            .is_err()
    );
    h.ctx.destroy_resource(guest).unwrap(); // No failed command retained it.
}

#[test]
fn cpu_guest_copy_backend_rejection_retries_with_fresh_snapshot() {
    let mut h = CopyHarness::new();
    let source = h.guest_source(0x1800, 5);
    let destination = h.resource(false, &[91; 5]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    h.ctx
        .set_metal_copy_queued_backend(Default::default(), move |request| {
            let GraphicsQueuedWorkRequest::Copy(copy) = request else {
                panic!("copy")
            };
            let GraphicsCopyInput::Host(bytes) = copy.source else {
                panic!("host")
            };
            let mut snapshots = captured.lock().unwrap();
            snapshots.push(bytes.to_vec());
            if snapshots.len() == 1 {
                return Err(GraphicsError::InvalidDescriptor);
            }
            Ok(Box::new(CpuTicket(Some(copy_bytes(&[17, 29, 43, 61, 79])))))
        });
    h.memory.write_bytes(0x1800, &[1, 2, 3, 4, 5]).unwrap();
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_SOURCE,
    );
    assert!(h.submit(0, &[]).is_err());
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.memory.write_bytes(0x1800, &[17, 29, 43, 61, 79]).unwrap();
    let receipt = h.submit(0, &[]).unwrap();
    assert_eq!(receipt.to_u64(), destination.to_u64() + 1);
    assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![vec![1, 2, 3, 4, 5], vec![17, 29, 43, 61, 79]]
    );
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(h.read(destination, 5), [17, 29, 43, 61, 79]);
}

#[test]
fn cpu_guest_copy_unreadable_tail_rejects_but_exact_odd_logical_span_succeeds() {
    let mut h = CopyHarness::new();
    let source = h.guest_source(0x6000, 5);
    let destination = h.resource(false, &[91; 5]);
    h.memory
        .map_region_with_data(0x6000, 4, MemoryPermissions::READ_ONLY, &[2, 3, 5, 7])
        .unwrap();
    h.memory
        .map_region(0x6004, 1, MemoryPermissions::NO_ACCESS)
        .unwrap();
    h.record(
        0,
        source,
        destination,
        ResourceStates::COPY_SOURCE,
        ResourceStates::COPY_SOURCE,
    );
    assert!(matches!(h.submit(0, &[]), Err(GraphicsError::Memory(_))));
    assert!(h.backend.lock().unwrap().requests.is_empty());
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.memory = GuestMemory::new();
    h.memory
        .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    h.memory
        .map_region_with_data(0x6000, 5, MemoryPermissions::READ_ONLY, &[2, 3, 5, 7, 11])
        .unwrap();
    let receipt = h.submit(0, &[]).unwrap();
    h.complete(0, copy_bytes(&[2, 3, 5, 7, 11]));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 5), [2, 3, 5, 7, 11]);
}

impl CopyHarness {
    fn enable_guest_regions(&mut self) {
        let captured = self.backend.clone();
        self.ctx.set_metal_unaligned_buffer_region_queued_backend(
            Default::default(),
            move |request| {
                let state = Arc::new(Mutex::new(ControlledState::default()));
                let mut backend = captured.lock().unwrap();
                backend.requests.push(request);
                backend.tickets.push(state.clone());
                Ok(Box::new(ControlledTicket(state)))
            },
        );
    }
    fn close_guest_region(
        &mut self,
        source: GraphicsHandle,
        destination: GraphicsHandle,
        source_offset: usize,
        destination_offset: usize,
        len: usize,
    ) {
        let list = self.lists[0];
        self.ctx
            .cmd_copy_buffer_region(
                list,
                destination,
                destination_offset,
                source,
                source_offset,
                len,
            )
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
    }
}

#[test]
fn cpu_guest_region_exact_end_snapshots_full_source_and_preserves_destination_prefix() {
    let mut h = CopyHarness::new();
    h.enable_guest_regions();
    let source = h.guest_source(0x1800, 9);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    h.memory.write_bytes(0x1800, &[31; 9]).unwrap();
    for (src, dst, len) in [
        (5, 6, 5),
        (4, 7, 5),
        (usize::MAX, 0, 1),
        (0, usize::MAX, 1),
        (0, 0, 0),
    ] {
        assert!(
            h.ctx
                .cmd_copy_buffer_region(h.lists[0], destination, dst, source, src, len)
                .is_err()
        );
    }
    h.close_guest_region(source, destination, 4, 6, 5);
    let literal = [17, 29, 43, 61, 79, 97, 113, 131, 149];
    h.memory.write_bytes(0x1800, &literal).unwrap();
    let receipt = h
        .submit(0, &[])
        .expect("guest-backed region source must be admitted");
    h.memory.write_bytes(0x1800, &[211; 9]).unwrap();
    h.reset(0);
    h.ctx.destroy_resource(source).unwrap();
    {
        let backend = h.backend.lock().unwrap();
        let GraphicsQueuedWorkRequest::BufferRegion(request) = &backend.requests[0] else {
            panic!("region")
        };
        assert_eq!(
            (
                request.source_byte_len,
                request.destination_byte_len,
                request.source_offset,
                request.destination_offset,
                request.byte_len
            ),
            (9, 11, 4, 6, 5)
        );
        assert!(
            matches!(&request.source,GraphicsCopyInput::Host(bytes) if bytes.as_ref()==literal)
        );
        assert!(
            matches!(&request.destination,GraphicsCopyInput::Host(bytes) if bytes.as_ref()==[1,2,3,4,5,6,7,8,9,10,11])
        );
    }
    // Entire padded source/destination, plus the existing unaligned patch parameters.
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 88); // 4*12 + 2*12 + 16
    h.complete(0, copy_bytes(&[1, 2, 3, 4, 5, 6, 79, 97, 113, 131, 149]));
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(
        h.read(destination, 11),
        [1, 2, 3, 4, 5, 6, 79, 97, 113, 131, 149]
    );
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
}

#[test]
fn cpu_guest_region_uncopied_unreadable_tail_rejects_full_snapshot_then_recovers() {
    let mut h = CopyHarness::new();
    h.enable_regions();
    let source = h.guest_source(0x6000, 9);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
    h.memory
        .map_region_with_data(0x6000, 4, MemoryPermissions::READ_ONLY, &[17, 29, 43, 61])
        .unwrap();
    h.memory
        .map_region(0x6004, 5, MemoryPermissions::NO_ACCESS)
        .unwrap();
    h.close_guest_region(source, destination, 0, 4, 4);
    assert!(matches!(h.submit(0, &[]), Err(GraphicsError::Memory(_))));
    assert!(h.backend.lock().unwrap().requests.is_empty());
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.memory = GuestMemory::new();
    h.memory
        .map_region(0x1000, 4096, MemoryPermissions::READ_WRITE)
        .unwrap();
    h.memory
        .map_region_with_data(
            0x6000,
            9,
            MemoryPermissions::READ_ONLY,
            &[17, 29, 43, 61, 79, 97, 113, 131, 149],
        )
        .unwrap();
    let receipt = h.submit(0, &[]).unwrap();
    h.complete(0, copy_bytes(&[1, 2, 3, 4, 17, 29, 43, 61, 99]));
    assert_eq!(receipt.to_u64(), destination.to_u64() + 1);
    assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 9), [1, 2, 3, 4, 17, 29, 43, 61, 99]);
}

#[test]
fn cpu_guest_region_backend_retry_refreshes_full_source_and_charges_backing_not_region() {
    let mut h = CopyHarness::new();
    let source = h.guest_source(0x1800, 17);
    let destination = h.resource(false, &[1, 2, 3, 4, 5, 6, 7, 8, 99]);
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let captured = snapshots.clone();
    h.ctx
        .set_metal_unaligned_buffer_region_queued_backend(Default::default(), move |work| {
            let GraphicsQueuedWorkRequest::BufferRegion(request) = work else {
                panic!("region")
            };
            let GraphicsCopyInput::Host(bytes) = request.source else {
                panic!("host")
            };
            let mut seen = captured.lock().unwrap();
            seen.push(bytes.to_vec());
            if seen.len() == 1 {
                return Err(GraphicsError::InvalidDescriptor);
            }
            assert_eq!(request.point.value, 1);
            Ok(Box::new(CpuTicket(Some(copy_bytes(&[
                1, 2, 3, 4, 77, 6, 7, 8, 99,
            ])))))
        });
    h.memory.write_bytes(0x1800, &[13; 17]).unwrap();
    h.close_guest_region(source, destination, 16, 4, 1);
    assert!(h.submit(0, &[]).is_err());
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    let mut next = [29; 17];
    next[16] = 77;
    h.memory.write_bytes(0x1800, &next).unwrap();
    let receipt = h.submit(0, &[]).unwrap();
    assert_eq!(receipt.to_u64(), destination.to_u64() + 1);
    assert_eq!(
        *snapshots.lock().unwrap(),
        vec![vec![13; 17], next.to_vec()]
    );
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 104); // full source20*2 + destination12*4 + parameters16, not copied1byte
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 9), [1, 2, 3, 4, 77, 6, 7, 8, 99]);
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
}

#[test]
fn cpu_guest_region_old_aligned_backend_rejects_unaligned_before_dispatch() {
    let mut h = CopyHarness::new();
    h.enable_regions();
    let source = h.guest_source(0x1800, 5);
    let destination = h.resource(false, &[1, 2, 3, 4, 5]);
    h.memory.write_bytes(0x1800, &[17, 29, 43, 61, 79]).unwrap();
    h.close_guest_region(source, destination, 1, 2, 1);
    assert!(matches!(
        h.submit(0, &[]),
        Err(GraphicsError::NativeMetalRequired)
    ));
    assert!(h.backend.lock().unwrap().requests.is_empty());
    assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    h.enable_guest_regions();
    let receipt = h.submit(0, &[]).unwrap();
    h.complete(0, copy_bytes(&[1, 2, 29, 4, 5]));
    assert_eq!(h.ctx.submission_point(receipt).unwrap().value, 1);
    assert_eq!(
        h.ctx.wait_submission(receipt).unwrap(),
        GraphicsSubmissionStatus::Complete
    );
    h.ctx.release_submission(receipt).unwrap();
    assert_eq!(h.read(destination, 5), [1, 2, 29, 4, 5]);
}
