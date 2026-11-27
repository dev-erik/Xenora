// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent submission-boundary checks. Mock completions are not GPU evidence.
use std::sync::{Arc, Mutex};
use xenora_loader::graphics::*;
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{GuestMemory, MemoryPermissions};
use xenora_texture::TextureFormat;

const RECORD: u64 = 0x1000;
const RESULT: u64 = 0x2000;
const SCRATCH: u64 = 0x3000;
const BACKING: u64 = 0x8000;
const SENTINEL: u64 = 0x123456789abcdef0;
// Four distinct uint words: >2^24, all ones, sign bit, NaN-looking bits.
const ROW0: [u8; 8] = [1, 0, 0, 1, 255, 255, 255, 255];
const ROW1: [u8; 8] = [0, 0, 0, 128, 69, 35, 193, 127];
fn submitted() -> Vec<u8> {
    let mut b = vec![0x6d; 41];
    b[4..12].copy_from_slice(&ROW0);
    b[20..28].copy_from_slice(&ROW1);
    b[40] = 0xe7;
    b
}
#[derive(Clone, Copy, Debug)]
enum Case {
    Legacy,
    Mip,
    Array,
    Uint,
    Depth,
}
const CASES: [Case; 5] = [
    Case::Legacy,
    Case::Mip,
    Case::Array,
    Case::Uint,
    Case::Depth,
];
impl Case {
    fn size(self) -> usize {
        match self {
            Self::Legacy => 64,
            Self::Mip => 336,
            _ => 672,
        }
    }
    fn base(self) -> usize {
        match self {
            Self::Legacy => 0,
            Self::Mip => 256,
            _ => 592,
        }
    }
    fn sub(self) -> u64 {
        match self {
            Self::Legacy => 0,
            Self::Mip => 1,
            _ => (1 << 32) | 1,
        }
    }
    fn opcode(self) -> u64 {
        match self {
            Self::Legacy => 44,
            Self::Uint | Self::Depth => 54,
            _ => 50,
        }
    }
    fn layout(self) -> GraphicsCopyLayout {
        match self {
            Self::Legacy => GraphicsCopyLayout::Rgba8 { extent: (4, 4) },
            Self::Mip => GraphicsCopyLayout::Rgba8MipChain {
                extent: (8, 8),
                mip_levels: 3,
            },
            Self::Array => GraphicsCopyLayout::Rgba8Array {
                extent: (8, 8),
                mip_levels: 3,
                array_layers: 2,
            },
            Self::Uint | Self::Depth => {
                GraphicsCopyLayout::TypedTexture(GraphicsTypedTextureDesc {
                    format: if matches!(self, Self::Depth) {
                        TextureFormat::Depth32Float
                    } else {
                        TextureFormat::R32Uint
                    },
                    extent: (8, 8),
                    mip_levels: 3,
                    array_layers: Some(2),
                })
            }
        }
    }
    fn location(self) -> GraphicsImageCopyLocation {
        match self {
            Self::Legacy => GraphicsImageCopyLocation::Rgba8 { origin: (1, 1) },
            Self::Uint | Self::Depth => GraphicsImageCopyLocation::Texture {
                mip: 1,
                slice: 1,
                origin: (1, 1),
            },
            _ => GraphicsImageCopyLocation::Rgba8Mip {
                mip: 1,
                slice: u32::from(matches!(self, Self::Array)),
                origin: (1, 1),
            },
        }
    }
    fn expected(self, prior: &[u8]) -> Vec<u8> {
        let mut b = prior.to_vec();
        // Selected mip is 4x4: row1/column1 = 20; row2/column1 = 36.
        b[self.base() + 20..self.base() + 28].copy_from_slice(&ROW0);
        b[self.base() + 36..self.base() + 44].copy_from_slice(&ROW1);
        b
    }
}
struct Ticket(Arc<Mutex<Option<GraphicsQueuedWorkOutput>>>);
impl GraphicsQueuedPendingWork for Ticket {
    fn poll(&mut self) -> Option<Result<GraphicsQueuedWorkOutput, GraphicsError>> {
        self.0.lock().unwrap().take().map(Ok)
    }
    fn wait(&mut self) -> Result<GraphicsQueuedWorkOutput, GraphicsError> {
        self.poll().unwrap_or_else(|| {
            Err(GraphicsError::BackendError(
                "mock completion not supplied".into(),
            ))
        })
    }
    fn reject_dependency(&mut self, _: GraphicsQueuePoint) {}
}
#[derive(Default)]
struct Captured {
    requests: Vec<GraphicsQueuedImageCopyRequest>,
    completions: Vec<Arc<Mutex<Option<GraphicsQueuedWorkOutput>>>>,
    reject_next: bool,
}
struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
    captured: Arc<Mutex<Captured>>,
    device: u64,
    queues: [u64; 2],
    allocators: [u64; 2],
    lists: [u64; 2],
}
impl Harness {
    fn memory() -> GuestMemory {
        let mut m = GuestMemory::new();
        for (a, n) in [(RECORD, 128), (RESULT, 8), (SCRATCH, 4096)] {
            m.map_region(a, n, MemoryPermissions::READ_WRITE).unwrap();
        }
        m
    }
    fn new() -> Self {
        Self::with_formats(0x3ff)
    }
    fn with_formats(formats: u16) -> Self {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let backend = captured.clone();
        let mut ctx = GraphicsContext::new();
        ctx.set_metal_typed_copy_queued_backend(
            Default::default(),
            GraphicsTypedCopyCapabilities { formats },
            move |work| {
                let GraphicsQueuedWorkRequest::ImageCopy(r) = work else {
                    panic!("image copy expected")
                };
                let mut b = backend.lock().unwrap();
                b.requests.push(r);
                if std::mem::take(&mut b.reject_next) {
                    return Err(GraphicsError::InvalidDescriptor);
                }
                let done = Arc::new(Mutex::new(None));
                b.completions.push(done.clone());
                Ok(Box::new(Ticket(done)))
            },
        );
        let mut h = Self {
            ctx,
            memory: Self::memory(),
            captured,
            device: 0,
            queues: [0; 2],
            allocators: [0; 2],
            lists: [0; 2],
        };
        h.device = h.ok(1, &[]);
        for i in 0..2 {
            h.queues[i] = h.ok(2, &[h.device]);
            h.allocators[i] = h.ok(3, &[h.device]);
            h.lists[i] = h.ok(4, &[h.device, h.allocators[i]]);
        }
        h
    }
    fn call(&mut self, op: u64, args: &[u64]) -> u64 {
        let mut words = [0u64; 16];
        words[0] = 1;
        words[1] = op;
        words[2..2 + args.len()].copy_from_slice(args);
        let bytes: Vec<_> = words.into_iter().flat_map(u64::to_le_bytes).collect();
        self.memory.write_bytes(RECORD, &bytes).unwrap();
        self.memory
            .write_bytes(RESULT, &SENTINEL.to_le_bytes())
            .unwrap();
        graphics_call_v1(&mut self.ctx, &mut self.memory, RECORD, 128, RESULT).unwrap()
    }
    fn ok(&mut self, op: u64, args: &[u64]) -> u64 {
        assert_eq!(self.call(op, args), 0, "opcode {op}");
        self.memory.read_u64(RESULT).unwrap()
    }
    fn source(&mut self) -> u64 {
        self.ok(49, &[self.device, 41, 0x800, BACKING])
    }
    fn texture(&mut self, c: Case, bytes: &[u8]) -> u64 {
        let t = match c {
            Case::Legacy => self.ok(6, &[self.device, 4, 4, 1, 0x400]),
            Case::Mip => self.ok(6, &[self.device, 8, 8, 3, 0x400]),
            Case::Array => self.ok(51, &[self.device, 8, 8, 3, 2, 0x400]),
            Case::Uint | Case::Depth => self.ok(
                53,
                &[
                    self.device,
                    8,
                    8,
                    3,
                    2,
                    if matches!(c, Case::Depth) { 9 } else { 8 },
                    0x400,
                ],
            ),
        };
        self.memory.write_bytes(SCRATCH, bytes).unwrap();
        self.ok(9, &[t, 0, SCRATCH, bytes.len() as u64]);
        t
    }
    fn record(&mut self, c: Case, i: usize, s: u64, t: u64, finish: bool) {
        let l = self.lists[i];
        if matches!(c, Case::Mip) {
            self.ctx
                .cmd_copy_image_mips(
                    GraphicsHandle::from_u64(l),
                    GraphicsHandle::from_u64(t),
                    c.location(),
                    GraphicsHandle::from_u64(s),
                    GraphicsImageCopyLocation::Buffer {
                        offset: 4,
                        bytes_per_row: 16,
                    },
                    (2, 2),
                )
                .unwrap();
        } else if matches!(c, Case::Legacy) {
            self.ok(44, &[l, t, 0, 1, 1, s, 1, 4, 16, 2, 2]);
        } else {
            self.ok(c.opcode(), &[l, t, 0, c.sub(), 1, 1, s, 1, 0, 4, 16, 2, 2]);
        }
        if finish {
            self.ok(13, &[l, t, 0x400, 0x800]);
        }
        self.ok(12, &[l]);
    }
    fn submit(
        &mut self,
        i: usize,
        waits: &[GraphicsQueuePoint],
    ) -> Result<GraphicsHandle, GraphicsError> {
        self.ctx.submit_command_list_after(
            GraphicsHandle::from_u64(self.queues[i]),
            GraphicsHandle::from_u64(self.lists[i]),
            waits,
            &self.memory,
        )
    }
    fn complete(&mut self, i: usize, c: Case, bytes: Vec<u8>) {
        *self.captured.lock().unwrap().completions[i].lock().unwrap() =
            Some(GraphicsQueuedWorkOutput::Copy {
                layout: c.layout(),
                bytes,
            });
    }
    fn read(&mut self, t: u64, size: usize) -> Vec<u8> {
        self.ok(10, &[t, 0, SCRATCH, size as u64]);
        let mut b = vec![0; size];
        self.memory.read_bytes(SCRATCH, &mut b).unwrap();
        b
    }
    fn finish(&mut self, r: GraphicsHandle) {
        assert_eq!(
            self.ctx.wait_submission(r).unwrap(),
            GraphicsSubmissionStatus::Complete
        );
        self.ctx.release_submission(r).unwrap();
    }
}

#[test]
fn guest_image_sources_capture_whole_logical_buffer_at_submit_and_survive_remap_destroy() {
    for c in CASES {
        let mut h = Harness::new();
        h.memory
            .map_region_with_data(BACKING, 41, MemoryPermissions::READ_WRITE, &[0x11; 41])
            .unwrap();
        let s = h.source();
        let prior = vec![0x93; c.size()];
        let t = h.texture(c, &prior);
        h.record(c, 0, s, t, true);
        h.memory.write_bytes(BACKING, &submitted()).unwrap();
        let r = h.submit(0, &[]).unwrap();
        h.memory.write_bytes(BACKING, &[0xee; 41]).unwrap();
        // Replace the mapping at exactly the same guest address while work is pending.
        h.memory = Harness::memory();
        h.memory
            .map_region_with_data(BACKING, 41, MemoryPermissions::READ_ONLY, &[0xcc; 41])
            .unwrap();
        h.ok(11, &[h.lists[0], h.allocators[0]]);
        h.ok(26, &[s]);
        {
            let b = h.captured.lock().unwrap();
            let q = &b.requests[0];
            assert_eq!(q.source_layout, GraphicsCopyLayout::Buffer { byte_len: 41 });
            assert_eq!(q.destination_layout, c.layout());
            assert_eq!(q.destination_location, c.location());
            assert_eq!(
                q.source_location,
                GraphicsImageCopyLocation::Buffer {
                    offset: 4,
                    bytes_per_row: 16
                }
            );
            assert_eq!(q.extent, (2, 2));
            let GraphicsCopyInput::Host(bytes) = &q.source else {
                panic!("immutable source")
            };
            assert_eq!(bytes.as_ref(), submitted());
            let GraphicsCopyInput::Host(bytes) = &q.destination else {
                panic!("owned prior")
            };
            assert_eq!(bytes.as_ref(), prior);
        }
        let expected = c.expected(&prior);
        h.complete(0, c, expected.clone());
        h.finish(r);
        assert_eq!(h.read(t, c.size()), expected);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
}

#[test]
fn guest_image_sources_uncopied_unreadable_tail_is_atomic_and_read_only_retry_is_fresh() {
    for c in CASES {
        let mut h = Harness::new();
        let s = h.source();
        let prior = vec![0x93; c.size()];
        let t = h.texture(c, &prior);
        // Every byte addressed by the rectangle is readable; only unused byte40 faults.
        h.memory
            .map_region_with_data(BACKING, 40, MemoryPermissions::READ_ONLY, &[0x11; 40])
            .unwrap();
        h.memory
            .map_region(BACKING + 40, 1, MemoryPermissions::NO_ACCESS)
            .unwrap();
        h.record(c, 0, s, t, true);
        for _ in 0..2 {
            assert!(matches!(h.submit(0, &[]), Err(GraphicsError::Memory(_))));
        }
        assert!(h.captured.lock().unwrap().requests.is_empty());
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        h.memory = Harness::memory();
        h.memory
            .map_region_with_data(BACKING, 41, MemoryPermissions::READ_ONLY, &submitted())
            .unwrap();
        let r = h.submit(0, &[]).unwrap();
        assert_eq!(r.to_u64(), t + 1);
        assert_eq!(h.ctx.submission_point(r).unwrap().value, 1);
        {
            let b = h.captured.lock().unwrap();
            let GraphicsCopyInput::Host(src) = &b.requests[0].source else {
                panic!("snapshot")
            };
            assert_eq!(src.as_ref(), submitted());
            let GraphicsCopyInput::Host(dst) = &b.requests[0].destination else {
                panic!("unchanged prior")
            };
            assert_eq!(dst.as_ref(), prior);
        }
        let expected = c.expected(&prior);
        h.complete(0, c, expected.clone());
        h.finish(r);
        assert_eq!(h.read(t, c.size()), expected);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    }
}

#[test]
fn guest_image_sources_backend_rejection_recaptures_and_refunds() {
    for c in CASES {
        let mut h = Harness::new();
        let s = h.source();
        let prior = vec![0x93; c.size()];
        let t = h.texture(c, &prior);
        h.memory
            .map_region_with_data(BACKING, 41, MemoryPermissions::READ_WRITE, &[0x11; 41])
            .unwrap();
        h.record(c, 0, s, t, true);
        h.captured.lock().unwrap().reject_next = true;
        assert!(h.submit(0, &[]).is_err());
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        h.memory.write_bytes(BACKING, &submitted()).unwrap();
        let r = h.submit(0, &[]).unwrap();
        assert_eq!(h.ctx.submission_point(r).unwrap().value, 1);
        {
            let b = h.captured.lock().unwrap();
            assert_eq!(b.requests.len(), 2);
            for (request, expected) in b.requests.iter().zip([vec![0x11; 41], submitted()]) {
                let GraphicsCopyInput::Host(bytes) = &request.source else {
                    panic!("snapshot")
                };
                assert_eq!(bytes.as_ref(), expected);
            }
        }
        let expected = c.expected(&prior);
        h.complete(0, c, expected.clone());
        h.finish(r);
        assert_eq!(h.read(t, c.size()), expected);
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    }
}

#[test]
fn guest_image_sources_exact_pending_destination_requires_wait_and_keeps_prior_version() {
    for c in CASES {
        for producer_first in [false, true] {
            let mut h = Harness::new();
            let s = h.source();
            let prior = vec![0x93; c.size()];
            let t = h.texture(c, &prior);
            h.memory
                .map_region_with_data(BACKING, 41, MemoryPermissions::READ_WRITE, &[0x42; 41])
                .unwrap();
            let l = h.lists[0];
            if matches!(c, Case::Legacy) {
                h.ok(44, &[l, t, 0, 0, 0, s, 1, 4, 16, 2, 2]);
            } else {
                h.ok(c.opcode(), &[l, t, 0, c.sub(), 0, 0, s, 1, 0, 4, 16, 2, 2]);
            }
            h.ok(12, &[l]);
            let a = h.submit(0, &[]).unwrap();
            let point = h.ctx.submission_point(a).unwrap();
            h.record(c, 1, s, t, true);
            assert!(h.submit(1, &[]).is_err());
            assert_eq!(h.captured.lock().unwrap().requests.len(), 1);
            h.memory.write_bytes(BACKING, &submitted()).unwrap();
            let b = h.submit(1, &[point]).unwrap();
            {
                let captured = h.captured.lock().unwrap();
                let q = &captured.requests[1];
                assert_eq!(q.waits, [point]);
                let GraphicsCopyInput::Produced(version) = q.destination else {
                    panic!("exact Produced prior")
                };
                assert_eq!(
                    version,
                    GraphicsProducedVersion {
                        point,
                        output: GraphicsProducedOutput::Copy(0)
                    }
                );
                let GraphicsCopyInput::Host(bytes) = &q.source else {
                    panic!("guest source")
                };
                assert_eq!(bytes.as_ref(), submitted());
            }
            for i in 0..2 {
                h.ok(11, &[h.lists[i], h.allocators[i]]);
            }
            h.ok(26, &[s]);
            let mut produced = prior.clone();
            produced[c.base()..c.base() + 8].fill(0x42);
            produced[c.base() + 16..c.base() + 24].fill(0x42);
            let expected = c.expected(&produced);
            h.complete(0, c, produced);
            h.complete(1, c, expected.clone());
            for r in if producer_first { [a, b] } else { [b, a] } {
                assert_eq!(
                    h.ctx.wait_submission(r).unwrap(),
                    GraphicsSubmissionStatus::Complete
                );
            }
            h.ctx.release_submission(b).unwrap();
            h.ctx.release_submission(a).unwrap();
            assert_eq!(h.read(t, c.size()), expected);
            assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
        }
    }
}

#[test]
fn guest_image_sources_do_not_enable_guest_destinations() {
    for c in CASES {
        let mut h = Harness::new();
        let guest = h.ok(49, &[h.device, 41, 0x400, BACKING]);
        let t = h.texture(c, &vec![0x93; c.size()]);
        let l = h.lists[0];
        let status = if matches!(c, Case::Legacy) {
            h.call(44, &[l, guest, 1, 4, 16, t, 0, 1, 1, 2, 2])
        } else {
            h.call(
                c.opcode(),
                &[l, guest, 1, 0, 4, 16, t, 0, c.sub(), 1, 1, 2, 2],
            )
        };
        assert_eq!(status, 2);
        assert_eq!(h.memory.read_u64(RESULT).unwrap(), SENTINEL);
        h.ok(26, &[guest]);
        assert!(h.captured.lock().unwrap().requests.is_empty());
    }
}

#[test]
fn guest_image_sources_typed_capability_denial_precedes_dispatch() {
    for (case, bit) in [(Case::Uint, 8), (Case::Depth, 9)] {
        let mut h = Harness::with_formats(0x3ff & !(1 << bit));
        h.memory
            .map_region_with_data(BACKING, 41, MemoryPermissions::READ_ONLY, &submitted())
            .unwrap();
        let source = h.source();
        let destination = h.texture(case, &vec![0x93; case.size()]);
        h.record(case, 0, source, destination, true);
        assert!(matches!(
            h.submit(0, &[]),
            Err(GraphicsError::NativeMetalRequired)
        ));
        let captured = h.captured.lock().unwrap();
        assert!(captured.requests.is_empty());
        assert!(captured.completions.is_empty());
        assert_eq!(h.ctx.queued_copy_allocation_bytes(), 0);
    }
}
