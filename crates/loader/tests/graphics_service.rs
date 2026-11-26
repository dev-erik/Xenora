// SPDX-License-Identifier: GPL-3.0-or-later

//! CPU-only graphics service envelope, memory, and transaction tests. No GPU work.

use xenora_loader::graphics::{GraphicsContext, GraphicsHandle, ResourceStates};
use xenora_loader::graphics_service::graphics_call_v1;
use xenora_loader::{ExitReason, GuestMemory, MemoryPermissions};

const DESC: u64 = 0x1000;
const OUT: u64 = 0x2000;
const DATA: u64 = 0x3000;
const SENTINEL: u64 = 0x9182_7364_5546_3728;
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const PS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");

fn record(opcode: u64, args: &[u64]) -> [u64; 16] {
    assert!(args.len() <= 14);
    let mut words = [0; 16];
    words[0] = 1;
    words[1] = opcode;
    words[2..2 + args.len()].copy_from_slice(args);
    words
}

struct Harness {
    ctx: GraphicsContext,
    memory: GuestMemory,
}

impl Harness {
    fn new() -> Self {
        let mut memory = GuestMemory::new();
        for (address, size) in [(DESC, 128), (OUT, 8), (DATA, 4096)] {
            memory
                .map_region(address, size, MemoryPermissions::READ_WRITE)
                .unwrap();
        }
        Self {
            ctx: GraphicsContext::new(),
            memory,
        }
    }

    fn prepare(&mut self, words: [u64; 16]) {
        let bytes: Vec<_> = words.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.memory.write_bytes(DESC, &bytes).unwrap();
        self.memory
            .write_bytes(OUT, &SENTINEL.to_le_bytes())
            .unwrap();
    }

    fn output(&self) -> u64 {
        let mut bytes = [0; 8];
        self.memory.read_bytes(OUT, &mut bytes).unwrap();
        u64::from_le_bytes(bytes)
    }

    fn call(&mut self, opcode: u64, args: &[u64]) -> Result<u64, ExitReason> {
        self.prepare(record(opcode, args));
        graphics_call_v1(&mut self.ctx, &mut self.memory, DESC, 128, OUT)
    }

    fn ok(&mut self, opcode: u64, args: &[u64]) -> u64 {
        assert_eq!(self.call(opcode, args), Ok(0), "opcode {opcode}");
        self.output()
    }

    fn setup_list(&mut self) -> (u64, u64, u64, u64) {
        let device = self.ok(1, &[]);
        let queue = self.ok(2, &[device]);
        let allocator = self.ok(3, &[device]);
        let list = self.ok(4, &[device, allocator]);
        (device, queue, allocator, list)
    }
}

#[test]
fn cpu_invalid_envelopes_and_reserved_words_preserve_result() {
    let mut h = Harness::new();
    let valid = record(1, &[]);
    for bytes in [0, 127, 129, u64::MAX] {
        h.prepare(valid);
        assert_eq!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, bytes, OUT),
            Ok(1)
        );
        assert_eq!(h.output(), SENTINEL);
    }
    for (index, value) in [
        (0, 0),
        (0, 2),
        (1, 0),
        (1, 67), // First unassigned opcode after shader array bindings.
        (1, u64::MAX),
        (2, 1),
        (15, 1),
    ] {
        let mut invalid = valid;
        invalid[index] = value;
        h.prepare(invalid);
        assert_eq!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
            Ok(1)
        );
        assert_eq!(h.output(), SENTINEL);
    }
    let actual = h.ok(1, &[]);
    let expected = GraphicsContext::new().create_device().unwrap().to_u64();
    assert_eq!(actual, expected, "bad envelopes must not consume a handle");
}

#[test]
fn cpu_high_bits_and_noncanonical_fields_are_not_silently_truncated() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let buffer = h.ok(5, &[device, 48, u64::from(ResourceStates::INDEX_BUFFER.0)]);
    for (opcode, args) in [
        (5, vec![device, 48, 1u64 << 32]),
        (20, vec![list, buffer, 0, (1u64 << 32) | 42]),
        (28, vec![list, (1u64 << 32) | u64::from(1.0f32.to_bits())]),
        (22, vec![list, 0, 2, 0, 15, 0, 0, 0, 0, 0, 0, 0]),
    ] {
        assert_eq!(h.call(opcode, &args), Ok(2), "opcode {opcode}");
        assert_eq!(h.output(), SENTINEL);
    }
}

#[test]
fn cpu_result_preflight_faults_do_not_consume_handles_or_write_prefixes() {
    for layout in 0..4 {
        let mut h = Harness::new();
        let destination = 0x9000;
        match layout {
            0 => {} // Entirely unmapped.
            1 => h
                .memory
                .map_region_with_data(destination, 8, MemoryPermissions::READ_ONLY, &[71; 8])
                .unwrap(),
            2 | 3 => {
                h.memory
                    .map_region_with_data(destination, 4, MemoryPermissions::READ_WRITE, &[71; 4])
                    .unwrap();
                if layout == 2 {
                    h.memory
                        .map_region_with_data(
                            destination + 4,
                            4,
                            MemoryPermissions::READ_ONLY,
                            &[73; 4],
                        )
                        .unwrap();
                }
            }
            _ => unreachable!(),
        }
        h.prepare(record(1, &[]));
        assert!(matches!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, destination),
            Err(ExitReason::MemoryFault(_))
        ));
        if layout != 0 {
            let mut prefix = [0; 4];
            h.memory.read_bytes(destination, &mut prefix).unwrap();
            assert_eq!(prefix, [71; 4]);
        }
        let actual = h.ok(1, &[]);
        let expected = GraphicsContext::new().create_device().unwrap().to_u64();
        assert_eq!(actual, expected, "output preflight layout {layout}");
    }
}

#[test]
fn cpu_shader_memory_and_translation_failures_leave_output_unchanged() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    h.memory.write_bytes(DATA, VS).unwrap();
    h.memory.write_bytes(DATA + 256, PS).unwrap();
    assert!(matches!(
        h.call(
            8,
            &[device, 0x9000, VS.len() as u64, DATA + 256, PS.len() as u64]
        ),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.output(), SENTINEL);
    // Readable but malformed bytes exercise translation failure separately.
    h.memory.write_bytes(DATA + 512, &[0xff; 4]).unwrap();
    assert_eq!(
        h.call(8, &[device, DATA, VS.len() as u64, DATA + 512, 4]),
        Ok(2)
    );
    assert_eq!(h.output(), SENTINEL);
    let pipeline = h.ok(
        8,
        &[device, DATA, VS.len() as u64, DATA + 256, PS.len() as u64],
    );
    h.ok(27, &[pipeline]);
}

#[test]
fn cpu_later_invalid_draw_preserves_prior_presentation_without_dispatch() {
    let mut h = Harness::new();
    let (device, queue, allocator, list) = h.setup_list();
    h.ok(17, &[list, 2, 2]);
    h.ok(12, &[list]);
    assert_eq!(h.ok(25, &[queue, list]), 0);
    let before = h.ctx.last_presentation.lock().unwrap().clone();
    h.ok(11, &[list, allocator]);
    let buffer = h.ok(
        5,
        &[
            device,
            48,
            u64::from(ResourceStates::VERTEX_AND_CONSTANT_BUFFER.0),
        ],
    );
    h.ok(18, &[list, buffer, 0, 16]);
    h.ok(23, &[list, 3, 1, 0, 0]);
    h.ok(23, &[list, 3, 1, 3, 0]); // Second draw fetches beyond the 48-byte buffer.
    h.ok(12, &[list]);
    // CPU-only callback: invalid recorded input must never reach the backend.
    h.ctx
        .set_metal_backend(|_, _, _| panic!("invalid service submission dispatched"));
    assert_eq!(h.call(25, &[queue, list]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    let after = h.ctx.last_presentation.lock().unwrap();
    assert_eq!(after.pixels, before.pixels);
    assert_eq!(
        (after.width, after.height, after.frame_index, after.backend),
        (
            before.width,
            before.height,
            before.frame_index,
            before.backend
        )
    );
}

#[test]
fn cpu_upload_readback_overlap_and_recorded_resource_lifetime() {
    let mut h = Harness::new();
    let (device, queue, allocator, list) = h.setup_list();
    let resource = h.ok(5, &[device, 8, u64::from(ResourceStates::COPY_DEST.0)]);
    h.memory
        .write_bytes(DATA, &[2, 3, 5, 7, 11, 13, 17, 19])
        .unwrap();
    h.ok(9, &[resource, 0, DATA, 8]);
    // Upload memory failure must preserve the bytes already stored.
    assert!(matches!(
        h.call(9, &[resource, 0, 0x9000, 8]),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.output(), SENTINEL);
    h.ok(
        13,
        &[
            list,
            resource,
            u64::from(ResourceStates::COPY_DEST.0),
            u64::from(ResourceStates::COPY_SOURCE.0),
        ],
    );
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    assert_eq!(h.call(26, &[resource]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.memory.write_bytes(DATA + 128, &[99; 24]).unwrap();
    h.ok(10, &[resource, 0, DATA + 136, 8]);
    let mut actual = [0; 24];
    h.memory.read_bytes(DATA + 128, &mut actual).unwrap();
    assert_eq!(&actual[..8], &[99; 8]);
    assert_eq!(&actual[8..16], &[2, 3, 5, 7, 11, 13, 17, 19]);
    assert_eq!(&actual[16..], &[99; 8]);
    // The result pointer overlaps the readback destination: neither may change.
    h.prepare(record(10, &[resource, 0, DATA + 136, 8]));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA + 140),
        Ok(2)
    );
    let mut after = [0; 24];
    h.memory.read_bytes(DATA + 128, &mut after).unwrap();
    assert_eq!(after, actual);
    h.ok(11, &[list, allocator]);
    h.ok(26, &[resource]);
    assert_eq!(h.call(10, &[resource, 0, DATA + 136, 8]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    // Public handle decoding remains round-trippable without inspecting internals.
    assert_eq!(GraphicsHandle::from_u64(resource).to_u64(), resource);
}

#[test]
fn cpu_guest_object_budget_rejects_and_recovers_after_destruction() {
    let mut h = Harness::new();
    let (device, _, _, _) = h.setup_list();
    let mut last = 0;
    for _ in 0..4092 {
        last = h.ok(5, &[device, 1, 0]);
    }
    assert_eq!(h.call(1, &[]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(26, &[last]);
    let replacement = h.ok(5, &[device, 1, 0]);
    assert_ne!(replacement, last);
}

#[test]
fn cpu_guest_command_budget_rejects_without_recording_and_reset_reclaims_it() {
    let mut h = Harness::new();
    let (_, queue, allocator, list) = h.setup_list();
    for _ in 0..4096 {
        h.ok(17, &[list, 1, 1]);
    }
    assert_eq!(h.call(17, &[list, 2, 2]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    let frame = h.ctx.last_presentation.lock().unwrap().clone();
    assert_eq!((frame.width, frame.height), (1, 1));
    h.ok(11, &[list, allocator]);
    h.ok(17, &[list, 2, 2]);
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    let frame = h.ctx.last_presentation.lock().unwrap().clone();
    assert_eq!((frame.width, frame.height), (2, 2));
}

#[test]
fn cpu_readback_address_overflow_is_a_memory_fault() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    let resource = h.ok(5, &[device, 8, u64::from(ResourceStates::COPY_SOURCE.0)]);
    assert!(matches!(
        h.call(10, &[resource, 0, u64::MAX - 3, 8]),
        Err(ExitReason::MemoryFault(
            xenora_loader::MemoryError::Overflow
        ))
    ));
    assert_eq!(h.output(), SENTINEL);
}

#[test]
fn cpu_consumed_inputs_may_alias_the_result_slot() {
    let mut h = Harness::new();
    h.prepare(record(1, &[]));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DESC),
        Ok(0)
    );
    let mut handle = [0; 8];
    h.memory.read_bytes(DESC, &mut handle).unwrap();
    let device = u64::from_le_bytes(handle);
    let queue = h.ok(2, &[device]);
    let allocator = h.ok(3, &[device]);
    let list = h.ok(4, &[device, allocator]);
    let resource = h.ok(5, &[device, 8, u64::from(ResourceStates::COPY_DEST.0)]);
    let payload = [2, 3, 5, 7, 11, 13, 17, 19];
    h.memory.write_bytes(DATA, &payload).unwrap();
    h.prepare(record(9, &[resource, 0, DATA, 8]));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA),
        Ok(0)
    );
    let mut overwritten = [1; 8];
    h.memory.read_bytes(DATA, &mut overwritten).unwrap();
    assert_eq!(overwritten, [0; 8]);
    h.ok(
        13,
        &[
            list,
            resource,
            u64::from(ResourceStates::COPY_DEST.0),
            u64::from(ResourceStates::COPY_SOURCE.0),
        ],
    );
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    h.ok(10, &[resource, 0, DATA + 16, 8]);
    let mut actual = [0; 8];
    h.memory.read_bytes(DATA + 16, &mut actual).unwrap();
    assert_eq!(actual, payload);
}

#[test]
fn cpu_vertex_offset_overflow_does_not_record_or_change_result() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let resource = h.ok(
        5,
        &[
            device,
            48,
            u64::from(ResourceStates::VERTEX_AND_CONSTANT_BUFFER.0),
        ],
    );
    for (offset, stride) in [(u64::MAX, 16), (16, u64::MAX)] {
        assert_eq!(h.call(18, &[list, resource, offset, stride]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    // Destroy succeeds only if neither failed binding recorded a reference.
    h.ok(26, &[resource]);
}

#[test]
fn cpu_color_load_records_validate_before_recording() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let target = h.ok(
        6,
        &[device, 1, 1, 1, u64::from(ResourceStates::RENDER_TARGET.0)],
    );
    for (mode, clear) in [(2u64, 0u64), (1, 1), (0, 1 << 32)] {
        let bytes: Vec<_> = [target, mode, clear]
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect();
        h.memory.write_bytes(DATA, &bytes).unwrap();
        assert_eq!(h.call(29, &[list, 1, DATA]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    for count in [0, 9, u64::MAX] {
        assert_eq!(h.call(29, &[list, count, DATA]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    assert!(matches!(
        h.call(29, &[list, 1, DATA + 4096 - 16]),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.output(), SENTINEL);
    h.ok(26, &[target]);
}

#[test]
fn cpu_color_load_command_preserves_prior_clear_and_uses_recording_budget() {
    let mut h = Harness::new();
    let (device, queue, allocator, list) = h.setup_list();
    let target = h.ok(
        6,
        &[device, 1, 1, 1, u64::from(ResourceStates::RENDER_TARGET.0)],
    );
    let clear: Vec<_> = [target, 0u64, 0xff0b0703]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    h.memory.write_bytes(DATA, &clear).unwrap();
    h.ok(29, &[list, 1, DATA]);
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    h.ok(11, &[list, allocator]);
    let load: Vec<_> = [target, 1u64, 0]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    h.memory.write_bytes(DATA, &load).unwrap();
    for _ in 0..4096 {
        h.ok(29, &[list, 1, DATA]);
    }
    assert_eq!(h.call(29, &[list, 1, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(12, &[list]);
    h.ok(25, &[queue, list]);
    assert_eq!(
        h.ctx.last_presentation.lock().unwrap().pixels,
        [3, 7, 11, 255]
    );
    assert_eq!(h.call(26, &[target]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(26, &[target]);
}

#[test]
fn cpu_invalid_later_color_load_record_retains_no_targets() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let a = h.ok(
        6,
        &[device, 1, 1, 1, u64::from(ResourceStates::RENDER_TARGET.0)],
    );
    let b = h.ok(
        6,
        &[device, 1, 1, 1, u64::from(ResourceStates::RENDER_TARGET.0)],
    );
    let bytes: Vec<_> = [a, 1, 0, b, 1, 7]
        .into_iter()
        .flat_map(u64::to_le_bytes)
        .collect();
    h.memory.write_bytes(DATA, &bytes).unwrap();
    assert_eq!(h.call(29, &[list, 2, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(26, &[a]);
    h.ok(26, &[b]);
}

#[test]
fn cpu_async_guest_receipts_preflight_outputs_and_cache_terminal_status() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use xenora_loader::graphics::{GraphicsError, GraphicsPassOutput, GraphicsPendingSubmission};
    type Completion = Option<Result<GraphicsPassOutput, GraphicsError>>;
    struct Controlled(Arc<Mutex<Completion>>);
    impl GraphicsPendingSubmission for Controlled {
        fn poll(&mut self) -> Completion {
            self.0.lock().unwrap().take()
        }
        fn wait(&mut self) -> Result<GraphicsPassOutput, GraphicsError> {
            self.poll()
                .expect("test releases completion before wait or drop")
        }
    }
    let mut h = Harness::new();
    let (_, queue, _, list) = h.setup_list();
    h.ok(17, &[list, 1, 1]);
    h.ok(12, &[list]);
    let result = Arc::new(Mutex::new(None));
    let submitted = Arc::new(AtomicUsize::new(0));
    let completion = result.clone();
    let counter = submitted.clone();
    h.ctx.set_metal_async_backend(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Controlled(completion.clone())))
    });
    h.prepare(record(30, &[queue, list]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(submitted.load(Ordering::SeqCst), 0);
    let receipt = h.ok(30, &[queue, list]);
    assert_eq!(submitted.load(Ordering::SeqCst), 1);
    assert_eq!(h.ok(31, &[receipt]), 0);
    assert_eq!(h.call(33, &[receipt]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    assert_eq!(h.call(30, &[queue, list]), Ok(2));
    *result.lock().unwrap() = Some(Ok(GraphicsPassOutput {
        color_layouts: None,
        colors: vec![vec![7, 11, 13, 255]],
        depth: None,
    }));
    for opcode in [31, 32] {
        h.prepare(record(opcode, &[receipt]));
        assert!(matches!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
            Err(ExitReason::MemoryFault(_))
        ));
        assert!(result.lock().unwrap().is_some());
        assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    }
    assert_eq!(h.ok(32, &[receipt]), 1);
    assert_eq!(h.ok(31, &[receipt]), 1);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 1);
    h.prepare(record(33, &[receipt]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.ok(31, &[receipt]), 1);
    h.ok(33, &[receipt]);
    assert_eq!(h.call(31, &[receipt]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    let receipt = h.ok(30, &[queue, list]);
    *result.lock().unwrap() = Some(Err(GraphicsError::BackendError(
        "controlled completion failure".into(),
    )));
    assert_eq!(h.ok(32, &[receipt]), 2);
    assert_eq!(h.ok(31, &[receipt]), 2);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 1);
    assert_eq!(
        h.ctx.last_presentation.lock().unwrap().pixels,
        [7, 11, 13, 255]
    );
    h.ok(33, &[receipt]);
}

#[test]
fn cpu_sampled_table_validates_last_record_before_recording_or_retaining() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let texture = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    for (word, value) in [(6, 2), (7, 1), (6, 1), (9, 1u64 << 32)] {
        let mut table = [0u64; 10];
        table[0] = texture;
        table[word] = value;
        if word == 6 && value == 1 {
            table[8] = f32::NAN.to_bits() as u64;
        }
        let bytes: Vec<_> = table.into_iter().flat_map(u64::to_le_bytes).collect();
        h.memory.write_bytes(DATA, &bytes).unwrap();
        assert_eq!(h.call(34, &[list, DATA]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    // Rejected tables must not retain the otherwise valid first resource.
    h.ok(26, &[texture]);
    h.memory.write_bytes(DATA, &[0; 80]).unwrap();
    h.ok(34, &[list, DATA]);
    h.ok(11, &[list, allocator]);
}

#[test]
fn cpu_sampled_table_memory_and_output_preflight_do_not_record() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let texture = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let mut table = [0u64; 10];
    table[1] = texture;
    let bytes: Vec<_> = table.into_iter().flat_map(u64::to_le_bytes).collect();
    h.memory.write_bytes(DATA, &bytes).unwrap();
    h.prepare(record(34, &[list, DATA]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    assert!(matches!(
        h.call(34, &[list, DATA + 4096 - 72]),
        Err(ExitReason::MemoryFault(_))
    ));
    h.ok(26, &[texture]);
}

#[test]
fn cpu_sampled_table_result_alias_consumes_both_handles_before_output() {
    for result_offset in [0, 8] {
        let mut h = Harness::new();
        let (device, _, allocator, list) = h.setup_list();
        let a = h.ok(
            6,
            &[
                device,
                1,
                1,
                1,
                ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
            ],
        );
        let b = h.ok(
            6,
            &[
                device,
                1,
                1,
                1,
                ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
            ],
        );
        let mut table = [0u64; 10];
        table[0] = a;
        table[1] = b;
        let bytes: Vec<_> = table.into_iter().flat_map(u64::to_le_bytes).collect();
        h.memory.write_bytes(DATA, &bytes).unwrap();
        h.prepare(record(34, &[list, DATA]));
        assert_eq!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA + result_offset),
            Ok(0)
        );
        let mut actual = [0; 80];
        h.memory.read_bytes(DATA, &mut actual).unwrap();
        let mut expected = bytes;
        expected[result_offset as usize..result_offset as usize + 8].fill(0);
        assert_eq!(actual.as_slice(), expected.as_slice());
        // Both original handles were recorded, even the one overwritten by result0.
        assert_eq!(h.call(26, &[a]), Ok(2));
        assert_eq!(h.call(26, &[b]), Ok(2));
        h.ok(11, &[list, allocator]);
        h.ok(26, &[a]);
        h.ok(26, &[b]);
    }
}

#[test]
fn cpu_sampled_table_invalid_second_texture_is_atomic_at_command_budget() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let a = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let buffer = h.ok(
        5,
        &[device, 16, ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64],
    );
    h.memory.write_bytes(DATA, &[0; 80]).unwrap();
    for _ in 0..4095 {
        h.ok(34, &[list, DATA]);
    }
    // A valid buffer handle is still an invalid texture; a stale handle also fails.
    for second in [buffer, u64::MAX] {
        let mut table = [0u64; 10];
        table[0] = a;
        table[1] = second;
        let bytes: Vec<_> = table.into_iter().flat_map(u64::to_le_bytes).collect();
        h.memory.write_bytes(DATA, &bytes).unwrap();
        assert_eq!(h.call(34, &[list, DATA]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    // Neither rejected table retained its valid first handle or consumed a command.
    h.ok(26, &[a]);
    h.ok(26, &[buffer]);
    h.memory.write_bytes(DATA, &[0; 80]).unwrap();
    h.ok(34, &[list, DATA]); // Exactly command4096.
    assert_eq!(h.call(34, &[list, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(11, &[list, allocator]);
    h.ok(34, &[list, DATA]);
}

// CPU-only queued service adapter: completion is immediate and literal; no GPU.
struct ServiceQueuedTicket;
impl xenora_loader::graphics::GraphicsQueuedPendingSubmission for ServiceQueuedTicket {
    fn poll(
        &mut self,
    ) -> Option<
        Result<xenora_loader::graphics::GraphicsPassOutput, xenora_loader::graphics::GraphicsError>,
    > {
        Some(self.wait())
    }
    fn wait(
        &mut self,
    ) -> Result<xenora_loader::graphics::GraphicsPassOutput, xenora_loader::graphics::GraphicsError>
    {
        Ok(xenora_loader::graphics::GraphicsPassOutput {
            color_layouts: None,
            colors: vec![vec![7, 11, 13, 255]],
            depth: None,
        })
    }
    fn reject_dependency(&mut self, _: xenora_loader::graphics::GraphicsQueuePoint) {}
}

type QueuedServiceRequests =
    std::sync::Arc<std::sync::Mutex<Vec<xenora_loader::graphics::GraphicsQueuedSubmissionRequest>>>;
type QueuedServiceHarness = (Harness, QueuedServiceRequests, u64, u64, u64, u64);

fn queued_service_harness() -> QueuedServiceHarness {
    let mut h = Harness::new();
    let (device, first_queue, allocator, first_list) = h.setup_list();
    let second_queue = h.ok(2, &[device]);
    let second_list = h.ok(4, &[device, allocator]);
    for list in [first_list, second_list] {
        let target = h.ok(
            6,
            &[device, 1, 1, 1, ResourceStates::RENDER_TARGET.0 as u64],
        );
        h.memory.write_u64(DATA, target).unwrap();
        h.memory.write_u64(DATA + 8, 0xff0d0b07).unwrap();
        h.ok(15, &[list, 1, DATA]);
        h.ok(12, &[list]);
    }
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = requests.clone();
    h.ctx.set_metal_queued_backend(move |request| {
        capture.lock().unwrap().push(request);
        Ok(Box::new(ServiceQueuedTicket))
    });
    (
        h,
        requests,
        first_queue,
        first_list,
        second_queue,
        second_list,
    )
}

#[test]
fn cpu_queued_service_preflights_result_and_entire_wait_table_before_submit() {
    let (mut h, requests, qa, la, qb, lb) = queued_service_harness();
    for (opcode, args) in [(35, vec![qa, la, 0, 0]), (36, vec![u64::MAX])] {
        h.prepare(record(opcode, &args));
        assert!(matches!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
            Err(ExitReason::MemoryFault(_))
        ));
        assert_eq!(h.output(), SENTINEL);
    }
    h.memory
        .map_region_with_data(0x5000, 8, MemoryPermissions::READ_ONLY, &[71; 8])
        .unwrap();
    h.prepare(record(35, &[qa, la, 0, 0]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x5000),
        Err(ExitReason::MemoryFault(_))
    ));
    assert!(requests.lock().unwrap().is_empty());
    let a = h.ok(35, &[qa, la, 0, 0]);
    assert_eq!(h.ok(36, &[a]), 1); // Failed output preflight consumed no point.
    let tail = DATA + 4096 - 16;
    h.memory.write_u64(tail, qa).unwrap();
    h.memory.write_u64(tail + 8, 1).unwrap();
    assert!(matches!(
        h.call(35, &[qb, lb, tail, 2]),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.output(), SENTINEL);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(h.ctx.last_presentation.lock().unwrap().frame_index, 0);
    h.memory.write_u64(DATA, qa).unwrap();
    h.memory.write_u64(DATA + 8, 1).unwrap();
    let b = h.ok(35, &[qb, lb, DATA, 1]);
    assert_eq!(h.ok(36, &[b]), 1);
    h.ok(32, &[b]);
    h.ok(33, &[a]);
    h.ok(33, &[b]);
}

#[test]
fn cpu_queued_service_invalid_second_wait_and_reserved_words_are_atomic() {
    let (mut h, requests, qa, la, qb, lb) = queued_service_harness();
    let a = h.ok(35, &[qa, la, 0, 0]);
    for (second_queue, second_value) in [(qa, 1), (qa, 2), (qb, 1), (u64::MAX, 1)] {
        for (offset, word) in [qa, 1, second_queue, second_value].into_iter().enumerate() {
            h.memory.write_u64(DATA + offset as u64 * 8, word).unwrap();
        }
        assert_eq!(h.call(35, &[qb, lb, DATA, 2]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
    for args in [
        [qb, lb, DATA, 0],
        [qb, lb, DATA, 3],
        [qb, lb, DATA, u64::MAX],
    ] {
        assert_eq!(h.call(35, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    for (opcode, args, unused) in [(35, vec![qb, lb, DATA, 1], 6), (36, vec![a], 3)] {
        let mut words = record(opcode, &args);
        words[unused] = 1;
        h.prepare(words);
        assert_eq!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
            Ok(1)
        );
        assert_eq!(h.output(), SENTINEL);
    }
    h.memory.write_u64(DATA, qa).unwrap();
    h.memory.write_u64(DATA + 8, 1).unwrap();
    let b = h.ok(35, &[qb, lb, DATA, 1]);
    assert_eq!(requests.lock().unwrap().len(), 2);
    h.ok(32, &[b]);
    h.ok(33, &[a]);
    assert_eq!(h.call(36, &[a]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(33, &[b]);
}

#[test]
fn cpu_queued_service_wait_payload_may_alias_receipt_result_after_consumption() {
    let (mut h, requests, qa, la, qb, lb) = queued_service_harness();
    let a = h.ok(35, &[qa, la, 0, 0]);
    h.memory.write_u64(DATA, qa).unwrap();
    h.memory.write_u64(DATA + 8, 1).unwrap();
    h.prepare(record(35, &[qb, lb, DATA, 1]));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA),
        Ok(0)
    );
    let b = h.memory.read_u64(DATA).unwrap();
    assert_ne!(b, qa);
    assert_eq!(h.memory.read_u64(DATA + 8).unwrap(), 1);
    assert_eq!(
        requests.lock().unwrap()[1].waits,
        [xenora_loader::graphics::GraphicsQueuePoint {
            queue: GraphicsHandle::from_u64(qa),
            value: 1,
        }]
    );
    h.ok(32, &[b]);
    h.ok(33, &[a]);
    h.ok(33, &[b]);
}

#[test]
fn cpu_guest_view_create_destroy_and_atomic_table_preflight() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let backing = h.ok(
        6,
        &[
            device,
            4,
            4,
            3,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    for (base, count, format) in [
        (0, 0, 0),
        (2, 2, 0),
        (u32::MAX as u64, 2, 0),
        (0, 1, 2),
        (1u64 << 32, 1, 0),
    ] {
        assert_eq!(h.call(37, &[device, backing, base, count, format]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    h.prepare(record(37, &[device, backing, 1, 2, 1]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    let view = h.ok(37, &[device, backing, 1, 2, 1]);
    assert_eq!(h.call(26, &[backing]), Ok(2));
    let mut table = [0u64; 10];
    table[0] = view;
    table[1] = backing;
    h.memory
        .write_bytes(
            DATA,
            &table
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(h.call(39, &[list, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(38, &[view]); // Invalid second entry did not record or retain first.
    let view = h.ok(37, &[device, backing, 1, 2, 0]);
    table[0] = view;
    table[1] = 0;
    h.memory
        .write_bytes(
            DATA,
            &table
                .into_iter()
                .flat_map(u64::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    h.prepare(record(39, &[list, DATA]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    assert!(matches!(
        h.call(39, &[list, DATA + 4096 - 72]),
        Err(ExitReason::MemoryFault(_))
    ));
    h.ok(39, &[list, DATA]);
    assert_eq!(h.call(38, &[view]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(38, &[view]);
    h.ok(26, &[backing]);
}

#[test]
fn cpu_guest_views_share_object_budget_and_bindings_share_command_budget() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let backing = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let mut first = 0;
    // Four setup objects plus one backing leave exactly4091 view objects.
    for index in 0..4091 {
        let view = h.ok(37, &[device, backing, 0, 1, 0]);
        if index == 0 {
            first = view;
        }
    }
    assert_eq!(h.call(37, &[device, backing, 0, 1, 0]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(38, &[first]);
    h.ok(37, &[device, backing, 0, 1, 1]);
    h.memory.write_bytes(DATA, &[0; 80]).unwrap();
    for _ in 0..4096 {
        h.ok(39, &[list, DATA]);
    }
    assert_eq!(h.call(39, &[list, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(11, &[list, allocator]);
    h.ok(39, &[list, DATA]);
}

#[test]
fn cpu_guest_typed_bc_creation_footprint_and_compressed_transfer_roundtrip() {
    for (format, length) in [(1, 48usize), (2, 96), (3, 30)] {
        let mut h = Harness::new();
        let (device, queue, allocator, list) = h.setup_list();
        for args in [
            [device, 0, 5, 3, format, ResourceStates::COPY_DEST.0 as u64],
            [device, 5, 5, 4, format, ResourceStates::COPY_DEST.0 as u64],
            [device, 5, 5, 3, 4, ResourceStates::COPY_DEST.0 as u64],
        ] {
            assert_eq!(h.call(40, &args), Ok(2));
            assert_eq!(h.output(), SENTINEL);
        }
        let args = [device, 5, 5, 3, format, ResourceStates::COPY_DEST.0 as u64];
        h.prepare(record(40, &args));
        assert!(matches!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
            Err(ExitReason::MemoryFault(_))
        ));
        let texture = h.ok(40, &args);
        let bytes: Vec<_> = (0..length).map(|i| (255 - i) as u8).collect();
        h.memory.write_bytes(DATA, &bytes).unwrap();
        h.ok(9, &[texture, 0, DATA, length as u64]);
        assert_eq!(h.call(9, &[texture, length as u64, DATA, 1]), Ok(2));
        h.ok(
            13,
            &[
                list,
                texture,
                ResourceStates::COPY_DEST.0 as u64,
                ResourceStates::COPY_SOURCE.0 as u64,
            ],
        );
        h.ok(12, &[list]);
        h.ok(25, &[queue, list]);
        h.memory.write_bytes(DATA, &vec![0; length]).unwrap();
        h.ok(10, &[texture, 0, DATA, length as u64]);
        let mut actual = vec![0; length];
        h.memory.read_bytes(DATA, &mut actual).unwrap();
        assert_eq!(actual, bytes);
        h.ok(11, &[list, allocator]);
        h.ok(26, &[texture]);
    }
}

#[test]
fn cpu_guest_typed_bc_view_formats_and_create_envelope_are_checked() {
    let mut h = Harness::new();
    let (device, _, _, _) = h.setup_list();
    for (format, valid_views) in [(1, [2, 3]), (2, [4, 5]), (3, [6, 6])] {
        let texture = h.ok(
            40,
            &[
                device,
                4,
                4,
                1,
                format,
                ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
            ],
        );
        for view_format in 0..=7 {
            let status = h.call(37, &[device, texture, 0, 1, view_format]);
            if valid_views.contains(&view_format) {
                assert_eq!(status, Ok(0));
                let view = h.output();
                h.ok(38, &[view]);
            } else {
                assert_eq!(status, Ok(2));
                assert_eq!(h.output(), SENTINEL);
            }
        }
        h.ok(26, &[texture]);
    }
    let mut words = record(
        40,
        &[device, 4, 4, 1, 1, ResourceStates::COPY_DEST.0 as u64],
    );
    words[8] = 1;
    h.prepare(words);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    for field in [2, 3, 4, 5] {
        let mut args = [device, 4, 4, 1, 1, ResourceStates::COPY_DEST.0 as u64];
        args[field] |= 1u64 << 32;
        assert_eq!(h.call(40, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
}

#[test]
fn cpu_uniform_guest_binding_preflights_all_bytes_and_copies_before_result_alias() {
    use xenora_loader::graphics::{GraphicsPassOutput, GraphicsTextureCapabilities};
    let mut h = Harness::new();
    let (device, queue, _, list) = h.setup_list();
    let words = [0x11223344u32, 2, 3, 4, 5, 6, 7, 0x8899aabb];
    h.memory
        .write_bytes(
            DATA,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    h.prepare(record(41, &[list, DATA, 1]));
    // Result aliases the first two words. Binding must have copied the full input.
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA),
        Ok(0)
    );
    assert!(matches!(
        h.call(41, &[list, DATA + 4096 - 16, 1]),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.call(41, &[list, DATA, 2]), Ok(2));
    assert_eq!(h.call(41, &[list, DATA, 0]), Ok(2));
    let pipeline = h
        .ctx
        .create_graphics_pipeline(GraphicsHandle::from_u64(device), VS, PS)
        .unwrap();
    let vb = h.ok(5, &[device, 48, 1]);
    let list_handle = GraphicsHandle::from_u64(list);
    h.ctx
        .cmd_set_graphics_pipeline(list_handle, Some(pipeline))
        .unwrap();
    h.ctx
        .cmd_set_vertex_buffer(list_handle, GraphicsHandle::from_u64(vb), 0, 16)
        .unwrap();
    h.ctx.cmd_draw_instanced(list_handle, 3, 1, 0, 0).unwrap();
    h.ctx.set_metal_uniform_attachment_backend(
        GraphicsTextureCapabilities::default(),
        move |draws, extent, _, _, _| {
            assert_eq!(draws[0].uniforms, Some(words));
            Ok(GraphicsPassOutput {
                color_layouts: None,
                colors: vec![[1, 2, 3, 255].repeat(extent.0 * extent.1)],
                depth: None,
            })
        },
    );
    h.ctx.close_command_list(list_handle).unwrap();
    h.ctx
        .execute_command_list(GraphicsHandle::from_u64(queue), list_handle, &h.memory)
        .unwrap();
    assert_eq!(h.call(41, &[list, 0, 0]), Ok(2));
}

#[test]
fn cpu_uniform_opcode_budget_and_result_preflight_do_not_record_failed_bindings() {
    for layout in 0..3 {
        let mut h = Harness::new();
        let (_, _, allocator, list) = h.setup_list();
        h.memory.write_bytes(DATA, &[17; 32]).unwrap();
        for _ in 0..4095 {
            h.ok(41, &[list, DATA, 1]);
        }
        let bad_result = 0x9000;
        if layout == 1 {
            h.memory
                .map_region_with_data(bad_result, 8, MemoryPermissions::READ_ONLY, &[71; 8])
                .unwrap();
        } else if layout == 2 {
            h.memory
                .map_region_with_data(bad_result, 4, MemoryPermissions::READ_WRITE, &[71; 4])
                .unwrap();
            h.memory
                .map_region_with_data(bad_result + 4, 4, MemoryPermissions::READ_ONLY, &[73; 4])
                .unwrap();
        }
        h.prepare(record(41, &[list, 0, 0]));
        assert!(matches!(
            graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, bad_result),
            Err(ExitReason::MemoryFault(_))
        ));
        assert_eq!(h.output(), SENTINEL);
        if layout != 0 {
            let mut prefix = [0; 4];
            h.memory.read_bytes(bad_result, &mut prefix).unwrap();
            assert_eq!(prefix, [71; 4]);
        }
        h.ok(41, &[list, DATA, 1]); // Exactly4096: failed preflight consumed no command.
        assert_eq!(h.call(41, &[list, 0, 0]), Ok(2));
        assert_eq!(h.output(), SENTINEL);
        assert_eq!(h.call(17, &[list, 1, 1]), Ok(2)); // Same shared recording budget.
        h.ok(11, &[list, allocator]);
        h.ok(41, &[list, 0, 0]); // Reset refunds the recording budget.
    }
}

#[test]
fn cpu_copy_opcode_pins_both_resources_until_list_reset() {
    let mut h = Harness::new();
    let (device, queue, allocator, list) = h.setup_list();
    let source = h.ok(5, &[device, 37, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 37, ResourceStates::COPY_DEST.0 as u64]);
    h.ok(42, &[list, destination, source]);
    assert_eq!(h.call(26, &[source]), Ok(2));
    assert_eq!(h.call(26, &[destination]), Ok(2));
    h.ok(12, &[list]);
    assert_eq!(h.call(42, &[list, destination, source]), Ok(2));
    // A recorded copy cannot silently succeed through the software render path.
    assert_eq!(h.call(25, &[queue, list]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(26, &[source]);
    h.ok(26, &[destination]);
}

#[test]
fn cpu_copy_opcode_rejects_aliases_and_equal_byte_count_different_layouts() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let source = h.ok(5, &[device, 24, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 23, ResourceStates::COPY_DEST.0 as u64]);
    let texture = h.ok(6, &[device, 3, 2, 1, ResourceStates::COPY_DEST.0 as u64]);
    let other_texture = h.ok(6, &[device, 2, 3, 1, ResourceStates::COPY_SOURCE.0 as u64]);
    for args in [
        [list, source, source],
        [list, destination, source],
        [list, texture, source],
        [list, texture, other_texture],
        [list, destination, 0],
    ] {
        assert_eq!(h.call(42, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    // Rejected recordings must pin nothing.
    for resource in [source, destination, texture, other_texture] {
        h.ok(26, &[resource]);
    }
}

#[test]
fn cpu_copy_opcode_result_preflight_and_reserved_words_do_not_record() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let source = h.ok(5, &[device, 5, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 5, ResourceStates::COPY_DEST.0 as u64]);
    h.prepare(record(42, &[list, destination, source]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
        Err(ExitReason::MemoryFault(_))
    ));
    let mut invalid = record(42, &[list, destination, source]);
    invalid[5] = 1;
    h.prepare(invalid);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    h.ok(26, &[source]);
    h.ok(26, &[destination]);
}

#[test]
fn cpu_copy_opcode_shares_command_budget_and_result_alias_is_safe() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let source = h.ok(5, &[device, 5, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 5, ResourceStates::COPY_DEST.0 as u64]);
    for _ in 0..4095 {
        h.ok(42, &[list, destination, source]);
    }
    h.prepare(record(42, &[list, destination, source]));
    // Result overlaps the source handle in the already consumed descriptor.
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DESC + 32),
        Ok(0)
    );
    assert_eq!(h.call(42, &[list, destination, source]), Ok(2));
    assert_eq!(h.call(17, &[list, 1, 1]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(42, &[list, destination, source]);
}

#[test]
fn cpu_buffer_region_opcode_records_unequal_backings_and_pins_until_reset() {
    let mut h = Harness::new();
    let (device, queue, allocator, list) = h.setup_list();
    let source = h.ok(5, &[device, 16, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 21, ResourceStates::COPY_DEST.0 as u64]);
    // CopyBufferRegion(list, destination, destination_offset, source, source_offset, bytes).
    h.ok(43, &[list, destination, 8, source, 4, 8]);
    assert_eq!(h.call(26, &[source]), Ok(2));
    assert_eq!(h.call(26, &[destination]), Ok(2));
    h.ok(12, &[list]);
    assert_eq!(h.call(43, &[list, destination, 8, source, 4, 8]), Ok(2));
    assert_eq!(h.call(25, &[queue, list]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(26, &[source]);
    h.ok(26, &[destination]);
}

#[test]
fn cpu_buffer_region_opcode_rejects_padding_overflow_and_alias_atomically() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let source = h.ok(5, &[device, 17, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 21, ResourceStates::COPY_DEST.0 as u64]);
    for args in [
        [list, destination, 0, source, 0, 0],
        [list, destination, 20, source, 0, 4],
        [list, destination, 0, source, 16, 4],
        [list, destination, u64::MAX - 3, source, 0, 8],
        [list, destination, 0, source, u64::MAX - 3, 8],
        [list, source, 0, source, 4, 4],
    ] {
        assert_eq!(h.call(43, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    h.ok(26, &[source]);
    h.ok(26, &[destination]);
}

#[test]
fn cpu_buffer_region_opcode_preflights_result_and_shares_recording_budget() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let source = h.ok(5, &[device, 17, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 21, ResourceStates::COPY_DEST.0 as u64]);
    let args = [list, destination, 0, source, 0, 4];
    h.prepare(record(43, &args));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
        Err(ExitReason::MemoryFault(_))
    ));
    let mut invalid = record(43, &args);
    invalid[8] = 1;
    h.prepare(invalid);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    for _ in 0..4095 {
        h.ok(43, &args);
    }
    h.prepare(record(43, &args));
    // Alias a consumed source handle with the output word.
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DESC + 40),
        Ok(0)
    );
    assert_eq!(h.call(43, &args), Ok(2));
    assert_eq!(h.call(17, &[list, 1, 1]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(43, &args);
}

#[test]
fn cpu_image_copy_opcode_records_three_directions_and_pins_both_inputs() {
    for direction in 0..3 {
        let mut h = Harness::new();
        let (device, queue, allocator, list) = h.setup_list();
        let source = if direction == 1 {
            h.ok(5, &[device, 24, ResourceStates::COPY_SOURCE.0 as u64])
        } else {
            h.ok(6, &[device, 4, 3, 1, ResourceStates::COPY_SOURCE.0 as u64])
        };
        let destination = if direction == 2 {
            h.ok(5, &[device, 27, ResourceStates::COPY_DEST.0 as u64])
        } else {
            h.ok(6, &[device, 5, 4, 1, ResourceStates::COPY_DEST.0 as u64])
        };
        let (dk, da, db) = if direction == 2 {
            (1, 4, 12)
        } else {
            (0, 2, 1)
        };
        let (sk, sa, sb) = if direction == 1 {
            (1, 4, 12)
        } else {
            (0, 1, 1)
        };
        let args = [list, destination, dk, da, db, source, sk, sa, sb, 2, 2];
        h.ok(44, &args);
        assert_eq!(h.call(26, &[source]), Ok(2));
        assert_eq!(h.call(26, &[destination]), Ok(2));
        h.ok(12, &[list]);
        assert_eq!(h.call(44, &args), Ok(2));
        assert_eq!(h.call(25, &[queue, list]), Ok(2));
        h.ok(11, &[list, allocator]);
        h.ok(26, &[source]);
        h.ok(26, &[destination]);
    }
}

#[test]
fn cpu_image_copy_opcode_rejects_layout_bounds_pitch_alias_and_overflow_atomically() {
    let mut h = Harness::new();
    let (device, _, _, list) = h.setup_list();
    let source = h.ok(6, &[device, 4, 3, 1, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(6, &[device, 5, 4, 1, ResourceStates::COPY_DEST.0 as u64]);
    let buffer = h.ok(5, &[device, 27, ResourceStates::COPY_DEST.0 as u64]);
    for args in [
        [list, destination, 0, 2, 1, source, 0, 1, 1, 0, 2],
        [list, destination, 0, 2, 1, source, 0, 1, 1, 2, 0],
        [list, destination, 0, 4, 1, source, 0, 1, 1, 2, 2],
        [list, destination, 0, 2, 1, source, 0, 3, 1, 2, 2],
        [list, destination, 0, u64::MAX, 1, source, 0, 1, 1, 2, 2],
        [list, destination, 0, 2, 1, source, 0, 1, 1, u64::MAX, 2],
        [list, source, 0, 0, 0, source, 0, 1, 1, 2, 2],
        [list, destination, 1, 0, 8, source, 0, 1, 1, 2, 2],
        [list, destination, 2, 0, 0, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, 1, 12, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, 4, 10, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, 4, 4, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, 4, 16, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, 4, 16388, source, 0, 1, 1, 2, 2],
        [list, buffer, 1, u64::MAX - 3, 12, source, 0, 1, 1, 2, 2],
    ] {
        assert_eq!(h.call(44, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    for resource in [source, destination, buffer] {
        h.ok(26, &[resource]);
    }
}

#[test]
fn cpu_image_copy_opcode_preflight_alias_and_shared_command_budget() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let source = h.ok(6, &[device, 4, 3, 1, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(6, &[device, 5, 4, 1, ResourceStates::COPY_DEST.0 as u64]);
    let args = [list, destination, 0, 2, 1, source, 0, 1, 1, 2, 2];
    h.prepare(record(44, &args));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x9000),
        Err(ExitReason::MemoryFault(_))
    ));
    let mut invalid = record(44, &args);
    invalid[13] = 1;
    h.prepare(invalid);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    for _ in 0..4095 {
        h.ok(44, &args);
    }
    h.prepare(record(44, &args));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DESC + 56),
        Ok(0)
    );
    assert_eq!(h.call(44, &args), Ok(2));
    assert_eq!(h.call(17, &[list, 1, 1]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(44, &args);
}

#[test]
fn cpu_buffer_region_opcode_records_arbitrary_logical_bytes() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let source = h.ok(5, &[device, 17, ResourceStates::COPY_SOURCE.0 as u64]);
    let destination = h.ok(5, &[device, 21, ResourceStates::COPY_DEST.0 as u64]);
    for (source_offset, destination_offset, count) in [(0, 1, 4), (1, 0, 4), (0, 0, 5), (16, 20, 1)]
    {
        h.ok(
            43,
            &[
                list,
                destination,
                destination_offset,
                source,
                source_offset,
                count,
            ],
        );
    }
    assert_eq!(h.call(26, &[source]), Ok(2));
    assert_eq!(h.call(26, &[destination]), Ok(2));
    h.ok(12, &[list]);
    h.ok(11, &[list, allocator]);
    h.ok(26, &[source]);
    h.ok(26, &[destination]);
}

#[test]
fn cpu_guest_child_view_preflight_ranges_and_independent_lifetime() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    let root = h.ok(
        6,
        &[
            device,
            8,
            8,
            4,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let parent = h.ok(37, &[device, root, 1, 2, 0]);
    for args in [
        [device, parent, 0, 0, 0],
        [device, parent, 2, 1, 0],
        [device, parent, 1, 2, 0],
        [device, parent, u32::MAX as u64, 1, 0],
        [device, parent, 0, 1u64 << 32, 0],
        [device, parent, 1u64 << 32, 1, 0],
        [device, parent, 0, 1, 6],
        [device, parent, 0, 1, 1u64 << 32],
        [device, parent, 0, 1, 2],
        [device, root, 0, 1, 0],
        [0, parent, 0, 1, 0],
    ] {
        assert_eq!(h.call(45, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    let mut reserved = record(45, &[device, parent, 1, 1, 1]);
    reserved[7] = 1;
    h.prepare(reserved);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    h.prepare(record(45, &[device, parent, 1, 1, 1]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    h.memory
        .map_region_with_data(0x5000, 8, MemoryPermissions::READ_ONLY, &[0; 8])
        .unwrap();
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x5000),
        Err(ExitReason::MemoryFault(_))
    ));
    let child = h.ok(45, &[device, parent, 1, 1, 1]);
    assert_eq!(
        child,
        parent + 1,
        "invalid children must not consume handles"
    );
    h.ok(38, &[parent]);
    assert_eq!(h.call(26, &[root]), Ok(2));
    // Parent is no longer required to create a deeper independent child.
    let leaf = h.ok(45, &[device, child, 0, 1, 0]);
    h.ok(38, &[child]);
    assert_eq!(h.call(26, &[root]), Ok(2));
    h.ok(38, &[leaf]);
    h.ok(26, &[root]);
}

#[test]
fn cpu_guest_child_views_share_global_object_budget() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    let root = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let parent = h.ok(37, &[device, root, 0, 1, 0]);
    let mut first = 0;
    for _ in 0..4093 {
        let child = h.ok(45, &[device, parent, 0, 1, 0]);
        if first == 0 {
            first = child;
        }
    }
    assert_eq!(h.call(45, &[device, parent, 0, 1, 0]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(38, &[first]);
    h.ok(45, &[device, parent, 0, 1, 0]);
}

#[test]
fn cpu_guest_child_view_formats_follow_root_family() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    for (storage, formats) in [(0, [0, 1]), (1, [2, 3]), (2, [4, 5]), (3, [6, 6])] {
        let root = h.ok(
            40,
            &[
                device,
                8,
                4,
                4,
                storage,
                ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
            ],
        );
        let parent = h.ok(37, &[device, root, 1, 3, formats[1]]);
        for format in 0..=7 {
            let result = h.call(45, &[device, parent, 1, 2, format]);
            if formats.contains(&format) {
                assert_eq!(result, Ok(0));
                let child = h.output();
                h.ok(38, &[child]);
            } else {
                assert_eq!(result, Ok(2));
                assert_eq!(h.output(), SENTINEL);
            }
        }
        h.ok(38, &[parent]);
        h.ok(26, &[root]);
    }
}

#[test]
fn cpu_guest_sampler_create_preflight_and_malformed_descriptors_consume_no_handles() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    let max = f32::MAX.to_bits() as u64;
    for args in [
        [0, u32::MAX as u64, 0, max],
        [device, 1u64 << 32, 0, max],
        [device, 0, 1u64 << 32, max],
        [device, 0, 0, 1u64 << 32],
        [device, 0, f32::NAN.to_bits() as u64, max],
        [device, 0, 0, f32::INFINITY.to_bits() as u64],
        [device, 0, (-1.0f32).to_bits() as u64, max],
        [device, 0, 2.0f32.to_bits() as u64, 1.0f32.to_bits() as u64],
    ] {
        assert_eq!(h.call(46, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    let mut words = record(46, &[device, u32::MAX as u64, 0, max]);
    words[6] = 1;
    h.prepare(words);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    h.prepare(record(46, &[device, u32::MAX as u64, 0, max]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    h.memory
        .map_region_with_data(0x5000, 8, MemoryPermissions::READ_ONLY, &[0; 8])
        .unwrap();
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, 0x5000),
        Err(ExitReason::MemoryFault(_))
    ));
    let sampler = h.ok(46, &[device, u32::MAX as u64, 0, max]);
    assert_eq!(sampler, device + 1);
    h.ok(47, &[sampler]);
    assert_eq!(h.call(47, &[sampler]), Ok(2));
    assert_eq!(h.call(47, &[device]), Ok(2));
}

#[test]
fn cpu_guest_sampler_handle_table_preflight_and_snapshot_lifetimes() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let root = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let view = h.ok(37, &[device, root, 0, 1, 0]);
    let sampler = h.ok(46, &[device, 0, 0, f32::MAX.to_bits() as u64]);
    let table = |words: [u64; 4]| {
        words
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>()
    };
    h.memory
        .write_bytes(DATA, &table([view, 0, sampler, root]))
        .unwrap();
    assert_eq!(h.call(48, &[list, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(38, &[view]); // Bad last handle did not retain first view.
    let view = h.ok(37, &[device, root, 0, 1, 0]);
    h.memory
        .write_bytes(DATA, &table([view, 0, sampler, 0]))
        .unwrap();
    h.prepare(record(48, &[list, DATA]));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    assert!(matches!(
        h.call(48, &[list, DATA + 4096 - 24]),
        Err(ExitReason::MemoryFault(_))
    ));
    // Result/table overlap is safe because the complete immutable table is read first.
    h.prepare(record(48, &[list, DATA]));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA + 24),
        Ok(0)
    );
    h.ok(47, &[sampler]);
    assert_eq!(h.call(38, &[view]), Ok(2));
    h.memory.write_bytes(DATA, &table([0; 4])).unwrap();
    h.ok(48, &[list, DATA]);
    assert_eq!(h.call(38, &[view]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(38, &[view]);
    h.ok(26, &[root]);
}

#[test]
fn cpu_guest_sampler_objects_and_handle_bindings_obey_shared_budgets() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let mut first = 0;
    for _ in 0..4092 {
        let sampler = h.ok(46, &[device, u32::MAX as u64, 0, f32::MAX.to_bits() as u64]);
        if first == 0 {
            first = sampler;
        }
    }
    assert_eq!(h.call(46, &[device, 0, 0, 0]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(47, &[first]);
    h.ok(46, &[device, 0, 0, 0]);
    h.memory.write_bytes(DATA, &[0; 32]).unwrap();
    for _ in 0..4096 {
        h.ok(48, &[list, DATA]);
    }
    assert_eq!(h.call(48, &[list, DATA]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
    h.ok(11, &[list, allocator]);
    h.ok(48, &[list, DATA]);
}

#[test]
fn cpu_guest_sampler_handle_bindings_preserve_independent_null_slots() {
    let mut h = Harness::new();
    let (device, _, allocator, list) = h.setup_list();
    let root = h.ok(
        6,
        &[
            device,
            1,
            1,
            1,
            ResourceStates::PIXEL_SHADER_RESOURCE.0 as u64,
        ],
    );
    let view = h.ok(37, &[device, root, 0, 1, 0]);
    let sampler = h.ok(46, &[device, u32::MAX as u64, 0, f32::MAX.to_bits() as u64]);
    for mask in 0..16 {
        let words: [u64; 4] = std::array::from_fn(|i| {
            if mask & (1 << i) == 0 {
                0
            } else if i < 2 {
                view
            } else {
                sampler
            }
        });
        h.memory
            .write_bytes(
                DATA,
                &words
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        h.ok(48, &[list, DATA]); // Presence is validated against used pairs only on an actual draw.
    }
    h.ok(47, &[sampler]);
    assert_eq!(h.call(38, &[view]), Ok(2));
    h.ok(11, &[list, allocator]);
    h.ok(38, &[view]);
    h.ok(26, &[root]);
}

#[test]
fn cpu_guest_backed_buffer_creation_checks_envelope_address_and_output_before_handle() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    for args in [
        [device, 0, 0, DATA],
        [device, 5, 1u64 << 32, DATA],
        [device, 5, 0, u64::MAX - 3],
        [0, 5, 0, DATA],
    ] {
        assert_eq!(h.call(49, &args), Ok(2));
        assert_eq!(h.output(), SENTINEL);
    }
    let mut words = record(
        49,
        &[device, 5, ResourceStates::COPY_SOURCE.0 as u64, 0x9000],
    );
    words[6] = 1;
    h.prepare(words);
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT),
        Ok(1)
    );
    assert_eq!(h.output(), SENTINEL);
    h.prepare(record(
        49,
        &[device, 5, ResourceStates::COPY_SOURCE.0 as u64, 0x9000],
    ));
    assert!(matches!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, OUT + 1),
        Err(ExitReason::MemoryFault(_))
    ));
    let resource = h.ok(
        49,
        &[device, 5, ResourceStates::COPY_SOURCE.0 as u64, 0x9000],
    );
    assert_eq!(
        resource,
        device + 1,
        "creation validates address span but does not read unmapped memory"
    );
    h.ok(26, &[resource]);
}

#[test]
fn cpu_guest_backed_source_records_without_reading_and_submission_reports_memory_fault() {
    let mut h = Harness::new();
    let (device, queue, _, list) = h.setup_list();
    let source = h.ok(
        49,
        &[device, 5, ResourceStates::COPY_SOURCE.0 as u64, 0x9000],
    );
    let destination = h.ok(5, &[device, 5, ResourceStates::COPY_DEST.0 as u64]);
    h.ok(42, &[list, destination, source]);
    h.ok(12, &[list]);
    h.ctx
        .set_metal_copy_queued_backend(Default::default(), |_| {
            panic!("memory fault must precede backend")
        });
    assert!(matches!(
        h.call(30, &[queue, list]),
        Err(ExitReason::MemoryFault(_))
    ));
    assert_eq!(h.output(), SENTINEL);
}

#[test]
fn cpu_guest_backed_creation_result_can_alias_future_source_bytes() {
    let mut h = Harness::new();
    let device = h.ok(1, &[]);
    h.memory.write_bytes(DATA, &[0; 8]).unwrap();
    h.prepare(record(
        49,
        &[device, 8, ResourceStates::COPY_SOURCE.0 as u64, DATA],
    ));
    assert_eq!(
        graphics_call_v1(&mut h.ctx, &mut h.memory, DESC, 128, DATA),
        Ok(0)
    );
    let source = h.memory.read_u64(DATA).unwrap();
    let queue = h.ok(2, &[device]);
    let allocator = h.ok(3, &[device]);
    let list = h.ok(4, &[device, allocator]);
    let destination = h.ok(5, &[device, 8, ResourceStates::COPY_DEST.0 as u64]);
    h.ctx
        .set_metal_copy_queued_backend(Default::default(), move |request| {
            let xenora_loader::graphics::GraphicsQueuedWorkRequest::Copy(copy) = request else {
                panic!("copy")
            };
            let xenora_loader::graphics::GraphicsCopyInput::Host(bytes) = copy.source else {
                panic!("host")
            };
            assert_eq!(&*bytes, &source.to_le_bytes());
            Err(xenora_loader::graphics::GraphicsError::InvalidDescriptor) // Inspect admission only; no pending native work.
        });
    h.ok(42, &[list, destination, source]);
    h.ok(12, &[list]);
    assert_eq!(h.call(30, &[queue, list]), Ok(2));
    assert_eq!(h.output(), SENTINEL);
}
