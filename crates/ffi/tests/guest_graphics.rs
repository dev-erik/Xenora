// SPDX-License-Identifier: GPL-3.0-or-later
//! Original PE32+ guest commands through GraphicsCallV1 and the production adapter.

use std::sync::Arc;
use xenora_ffi::graphics_backend::{
    install_async_pipeline_backend, install_pipeline_backend, install_queued_pipeline_backend,
};
use xenora_loader::cpu::ExitReason;
use xenora_loader::graphics::GraphicsBackendId;
use xenora_loader::imports::ImportRegistry;
use xenora_loader::pe::{
    IMAGE_FILE_MACHINE_AMD64, IMAGE_SCN_CNT_CODE, IMAGE_SCN_MEM_EXECUTE, IMAGE_SCN_MEM_READ,
    IMAGE_SCN_MEM_WRITE, PE32_PLUS_MAGIC,
};
use xenora_loader::services::{MockClock, ServicesState, register_standard_services};
use xenora_loader::startup::{StartupLayout, run_synthetic_startup};

const BASE: u64 = 0x140000000;
const TEXT_RVA: u32 = 0x1000;
const DATA_RVA: u32 = 0x21000;
const COPY_DEST: u64 = 0x400;
const COPY_SOURCE: u64 = 0x800;
const RENDER_TARGET: u64 = 4;
const PIXEL_RESOURCE: u64 = 0x80;
const VERTEX_BUFFER: u64 = 1;
const CLEAR: [u8; 4] = [17, 31, 47, 255];

#[derive(Clone, Copy)]
enum Arg {
    Immediate(u64),
    // Guest address of an earlier import's handle result, not a host handle.
    Result(u64),
}
use Arg::{Immediate as I, Result as H};

struct CallCheck {
    opcode: u64,
    status: u64,
    result: u64,
    expected_status: u64,
    // None requires a nonzero returned handle; Some checks an exact result.
    expected_result: Option<u64>,
}

struct GuestScript {
    code: Vec<u8>,
    data: Vec<u8>,
    calls: Vec<CallCheck>,
}

impl GuestScript {
    fn new() -> Self {
        let mut data = vec![0; 0x100];
        put32(&mut data, 0, DATA_RVA + 0x40);
        put32(&mut data, 12, DATA_RVA + 0x80);
        put32(&mut data, 16, DATA_RVA + 0x60);
        put64(&mut data, 0x40, u64::from(DATA_RVA + 0xa0));
        put64(&mut data, 0x60, u64::from(DATA_RVA + 0xa0));
        let dll = b"xenora_fixture.dll\0";
        data[0x80..0x80 + dll.len()].copy_from_slice(dll);
        let symbol = b"\0\0GraphicsCallV1\0";
        data[0xa0..0xa0 + symbol.len()].copy_from_slice(symbol);
        Self {
            // Win64 home area and alignment; startup enters with RSP % 16 == 8.
            code: vec![0x48, 0x83, 0xec, 0x28],
            data,
            calls: Vec::new(),
        }
    }

    fn bytes(&mut self, bytes: &[u8]) -> u64 {
        self.data.resize(self.data.len().next_multiple_of(8), 0);
        let address = BASE + u64::from(DATA_RVA) + self.data.len() as u64;
        self.data.extend_from_slice(bytes);
        address
    }

    fn mov_imm(&mut self, prefix: [u8; 2], value: u64) {
        self.code.extend_from_slice(&prefix);
        self.code.extend_from_slice(&value.to_le_bytes());
    }

    fn store_rax(&mut self, address: u64) {
        self.mov_imm([0x49, 0xba], address); // mov r10, imm64
        self.code.extend_from_slice(&[0x49, 0x89, 0x02]); // mov [r10], rax
    }

    fn copy_result(&mut self, from: u64, to: u64) {
        self.mov_imm([0x48, 0xb8], from); // mov rax, imm64
        self.code.extend_from_slice(&[0x48, 0x8b, 0x00]); // mov rax, [rax]
        self.store_rax(to);
    }

    fn call(&mut self, opcode: u64, args: &[Arg], status: u64, result: Option<u64>) -> u64 {
        assert!(args.len() <= 14);
        let mut words = [0u64; 16];
        words[0] = 1;
        words[1] = opcode;
        for (slot, arg) in args.iter().enumerate() {
            if let I(value) = arg {
                words[slot + 2] = *value;
            }
        }
        let record: Vec<_> = words.into_iter().flat_map(u64::to_le_bytes).collect();
        let descriptor = self.bytes(&record);
        let output = self.bytes(&u64::MAX.to_le_bytes());
        let status_address = self.bytes(&u64::MAX.to_le_bytes());
        for (slot, arg) in args.iter().enumerate() {
            if let H(address) = arg {
                self.copy_result(*address, descriptor + (slot as u64 + 2) * 8);
            }
        }
        self.mov_imm([0x48, 0xb9], descriptor); // RCX
        self.mov_imm([0x48, 0xba], 128); // RDX
        self.mov_imm([0x49, 0xb8], output); // R8
        self.mov_imm([0x49, 0xb9], 0); // R9 reserved
        let next = BASE + u64::from(TEXT_RVA) + self.code.len() as u64 + 6;
        let iat = BASE + u64::from(DATA_RVA) + 0x60;
        let displacement = i32::try_from(iat as i64 - next as i64).unwrap();
        self.code.extend_from_slice(&[0xff, 0x15]); // call [rip + IAT displacement]
        self.code.extend_from_slice(&displacement.to_le_bytes());
        self.store_rax(status_address);
        self.calls.push(CallCheck {
            opcode,
            status: status_address,
            result: output,
            expected_status: status,
            expected_result: result,
        });
        output
    }

    fn ok(&mut self, opcode: u64, args: &[Arg]) {
        self.call(opcode, args, 0, Some(0));
    }

    fn handle(&mut self, opcode: u64, args: &[Arg]) -> u64 {
        self.call(opcode, args, 0, None)
    }

    fn barrier(&mut self, list: u64, resource: u64, before: u64, after: u64) {
        self.ok(13, &[H(list), H(resource), I(before), I(after)]);
    }

    fn targets(&mut self, list: u64, targets: &[u64]) {
        let records = self.bytes(&vec![0; targets.len() * 16]);
        for (slot, target) in targets.iter().enumerate() {
            self.copy_result(*target, records + slot as u64 * 16);
            let offset = usize::try_from(records - BASE - u64::from(DATA_RVA)).unwrap();
            put64(
                &mut self.data,
                offset + slot * 16 + 8,
                u64::from(u32::from_le_bytes(CLEAR)),
            );
        }
        self.ok(15, &[H(list), I(targets.len() as u64), I(records)]);
    }

    fn load_targets(&mut self, list: u64, targets: &[u64]) {
        let records = self.bytes(&vec![0; targets.len() * 24]);
        for (slot, target) in targets.iter().enumerate() {
            self.copy_result(*target, records + slot as u64 * 24);
            let offset = usize::try_from(records - BASE - u64::from(DATA_RVA)).unwrap();
            put64(&mut self.data, offset + slot * 24 + 8, 1);
        }
        self.ok(29, &[H(list), I(targets.len() as u64), I(records)]);
    }

    fn finish(&mut self) -> Vec<u8> {
        self.code.extend_from_slice(&[0x48, 0x83, 0xc4, 0x28, 0xf4]);
        let text_size = self.code.len().next_multiple_of(0x200);
        let data_size = self.data.len().next_multiple_of(0x200);
        assert!(text_size <= (DATA_RVA - TEXT_RVA) as usize);
        let text_file = 0x400;
        let data_file = text_file + text_size;
        let mut pe = vec![0; data_file + data_size];
        pe[..2].copy_from_slice(b"MZ");
        put32(&mut pe, 0x3c, 0x80);
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        put16(&mut pe, coff, IMAGE_FILE_MACHINE_AMD64);
        put16(&mut pe, coff + 2, 2);
        put16(&mut pe, coff + 16, 240);
        put16(&mut pe, coff + 18, 0x22); // Executable, large-address-aware.
        let opt = coff + 20;
        put16(&mut pe, opt, PE32_PLUS_MAGIC);
        put32(&mut pe, opt + 16, TEXT_RVA);
        put64(&mut pe, opt + 24, BASE);
        put32(&mut pe, opt + 32, 0x1000);
        put32(&mut pe, opt + 36, 0x200);
        put32(
            &mut pe,
            opt + 56,
            (DATA_RVA + data_size as u32).next_multiple_of(0x1000),
        );
        put32(&mut pe, opt + 60, 0x400);
        put32(&mut pe, opt + 108, 16);
        put32(&mut pe, opt + 120, DATA_RVA);
        put32(&mut pe, opt + 124, 40);
        for (index, (name, rva, offset, size, flags)) in [
            (
                b".text".as_slice(),
                TEXT_RVA,
                text_file,
                text_size,
                IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_EXECUTE,
            ),
            (
                b".data".as_slice(),
                DATA_RVA,
                data_file,
                data_size,
                0x40 | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let section = opt + 240 + index * 40;
            pe[section..section + name.len()].copy_from_slice(name);
            put32(&mut pe, section + 8, size as u32);
            put32(&mut pe, section + 12, rva);
            put32(&mut pe, section + 16, size as u32);
            put32(&mut pe, section + 20, offset as u32);
            put32(&mut pe, section + 36, flags);
        }
        pe[text_file..text_file + self.code.len()].copy_from_slice(&self.code);
        pe[data_file..data_file + self.data.len()].copy_from_slice(&self.data);
        pe
    }
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn floats(values: &[[f32; 4]]) -> Vec<u8> {
    values
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn guest_import_selects_sampled_and_mrt_pipelines_with_retained_depth() {
    run_guest_scene(false);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn guest_import_submits_and_waits_for_native_rendering() {
    run_guest_scene(true);
}

fn run_guest_scene(asynchronous: bool) {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queue = guest.handle(2, &[H(device)]);
    let allocator = guest.handle(3, &[H(device)]);
    let list = guest.handle(4, &[H(device), H(allocator)]);
    let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let colors = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let texture = guest.handle(6, &[H(device), I(1), I(1), I(1), I(COPY_DEST)]);
    let target = guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]);
    let depth = guest.handle(7, &[H(device), I(4), I(3)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin");
    let vs_addr = guest.bytes(vs);
    let ps_addr = guest.bytes(ps);
    let sample = guest.handle(
        8,
        &[
            H(device),
            I(vs_addr),
            I(vs.len() as u64),
            I(ps_addr),
            I(ps.len() as u64),
        ],
    );
    let uv = guest.bytes(&floats(&[[0.5, 0.5, 0.0, 1.0]; 3]));
    guest.ok(9, &[H(colors), I(0), I(uv), I(48)]);
    let mut readbacks = Vec::new();
    for frame in 0..2 {
        guest.ok(11, &[H(list), H(allocator)]);
        let z = if frame == 0 { 0.25 } else { 0.75 };
        let positions = guest.bytes(&floats(&[
            [-1.0, -1.0, z, 1.0],
            [3.0, -1.0, z, 1.0],
            [-1.0, 3.0, z, 1.0],
        ]));
        let texel = if frame == 0 {
            [200, 40, 60, 255]
        } else {
            [40, 120, 200, 255]
        };
        let pixels = guest.bytes(&texel);
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        guest.ok(9, &[H(texture), I(0), I(pixels), I(4)]);
        guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
        if frame == 0 {
            guest.barrier(list, colors, COPY_DEST, VERTEX_BUFFER);
        } else {
            guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
        }
        guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
        guest.ok(14, &[H(list), H(sample)]);
        if frame == 0 {
            guest.targets(list, &[target]);
        } else {
            guest.load_targets(list, &[target]);
        }
        guest.ok(
            16,
            &[
                H(list),
                H(depth),
                I(frame),
                I(if frame == 0 {
                    u64::from(1f32.to_bits())
                } else {
                    0
                }),
            ],
        );
        guest.ok(17, &[H(list), I(4), I(3)]);
        guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
        guest.ok(19, &[H(list), H(colors), I(0), I(16)]);
        guest.ok(
            21,
            &[H(list), H(texture), I(u64::from(u32::MAX)), I(0), I(0)],
        );
        let scissor = if frame == 0 { [1, 0, 0, 2, 3] } else { [0; 5] };
        guest.ok(
            22,
            &[
                H(list),
                I(2),
                I(1),
                I(0),
                I(15),
                I(0),
                I(0),
                I(scissor[0]),
                I(scissor[1]),
                I(scissor[2]),
                I(scissor[3]),
                I(scissor[4]),
            ],
        );
        guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
        guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
        guest.barrier(list, vb, VERTEX_BUFFER, COPY_DEST);
        guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
        guest.ok(12, &[H(list)]);
        if asynchronous {
            let submission = guest.handle(30, &[H(queue), H(list)]);
            guest.call(32, &[H(submission)], 0, Some(1));
            guest.call(31, &[H(submission)], 0, Some(1));
            guest.ok(33, &[H(submission)]);
        } else {
            guest.call(25, &[H(queue), H(list)], 0, Some(1));
        }
        let readback = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(target), I(0), I(readback + 8), I(48)]);
        readbacks.push(readback);
    }
    // References survive execution, including the selected pipeline and sampled texture.
    // Semantic errors preserve the result sentinel under the service contract.
    guest.call(26, &[H(texture)], 2, Some(u64::MAX));
    guest.call(27, &[H(sample)], 2, Some(u64::MAX));
    guest.ok(11, &[H(list), H(allocator)]);
    guest.ok(27, &[H(sample)]);
    // A distinct translated pipeline overlays the same preserved color target.
    let overlay_vs = include_bytes!("../../../fixtures/rdna2/x9-forward-vs.bin");
    let overlay_ps = include_bytes!("../../../fixtures/rdna2/x9-forward-ps.bin");
    let overlay_va = guest.bytes(overlay_vs);
    let overlay_pa = guest.bytes(overlay_ps);
    let overlay = guest.handle(
        8,
        &[
            H(device),
            I(overlay_va),
            I(overlay_vs.len() as u64),
            I(overlay_pa),
            I(overlay_ps.len() as u64),
        ],
    );
    guest.ok(14, &[H(list), H(overlay)]);
    guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
    guest.load_targets(list, &[target]);
    guest.ok(17, &[H(list), I(4), I(3)]);
    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
    guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
    guest.ok(
        22,
        &[
            H(list),
            I(0),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(1),
            I(1),
            I(2),
            I(1),
        ],
    );
    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
    guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
    guest.barrier(list, vb, VERTEX_BUFFER, COPY_DEST);
    guest.ok(12, &[H(list)]);
    if asynchronous {
        let submission = guest.handle(30, &[H(queue), H(list)]);
        guest.call(32, &[H(submission)], 0, Some(1));
        guest.call(31, &[H(submission)], 0, Some(1));
        guest.ok(33, &[H(submission)]);
    } else {
        guest.call(25, &[H(queue), H(list)], 0, Some(1));
    }
    let overlay_readback = guest.bytes(&[0xa5; 64]);
    guest.ok(10, &[H(target), I(0), I(overlay_readback + 8), I(48)]);
    guest.ok(11, &[H(list), H(allocator)]);
    guest.ok(27, &[H(overlay)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-mrt-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let mrt = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );
    let targets: Vec<_> = (0..8)
        .map(|_| guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]))
        .collect();
    guest.ok(14, &[H(list), H(mrt)]);
    guest.targets(list, &targets);
    guest.ok(17, &[H(list), I(4), I(3)]);
    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
    guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
    for target in &targets {
        guest.barrier(list, *target, RENDER_TARGET, COPY_SOURCE);
    }
    guest.ok(12, &[H(list)]);
    if asynchronous {
        let submission = guest.handle(30, &[H(queue), H(list)]);
        guest.call(32, &[H(submission)], 0, Some(1));
        guest.call(31, &[H(submission)], 0, Some(1));
        guest.ok(33, &[H(submission)]);
    } else {
        guest.call(25, &[H(queue), H(list)], 0, Some(1));
    }
    let mut mrt_readbacks = Vec::new();
    for target in &targets {
        let output = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(*target), I(0), I(output + 8), I(48)]);
        mrt_readbacks.push(output);
    }
    guest.ok(11, &[H(list), H(allocator)]);
    guest.ok(27, &[H(mrt)]);
    for resource in targets
        .into_iter()
        .chain([vb, colors, texture, target, depth])
    {
        guest.ok(26, &[H(resource)]);
    }
    let pe = guest.finish();
    let services = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    if asynchronous {
        install_async_pipeline_backend(&mut services.graphics.lock().unwrap()).unwrap();
    } else {
        install_pipeline_backend(&mut services.graphics.lock().unwrap()).unwrap();
    }
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, services.clone());
    let result = run_synthetic_startup(
        &pe,
        StartupLayout {
            image_base: BASE,
            stack_top: 0x500000000,
            tls_base: 0x300000000,
        },
        imports,
        100_000,
    )
    .unwrap();
    assert_eq!(result.execution.exit_reason, ExitReason::Halt);
    assert_eq!(result.bound_imports, 1);
    let memory = &result.image.memory;
    for (index, call) in guest.calls.iter().enumerate() {
        assert_eq!(
            memory.read_u64(call.status).unwrap(),
            call.expected_status,
            "call {index}, opcode {}",
            call.opcode
        );
        let value = memory.read_u64(call.result).unwrap();
        if let Some(expected) = call.expected_result {
            assert_eq!(
                value, expected,
                "call {index}, opcode {} result",
                call.opcode
            );
        } else {
            assert_ne!(value, 0, "call {index} did not return a handle");
        }
    }
    let read = |address| {
        let mut bytes = [0; 64];
        memory.read_bytes(address, &mut bytes).unwrap();
        assert_eq!(&bytes[..8], &[0xa5; 8]);
        assert_eq!(&bytes[56..], &[0xa5; 8]);
        bytes[8..56].to_vec()
    };
    for (frame, address) in readbacks.into_iter().enumerate() {
        let bytes = read(address);
        for (index, pixel) in bytes.as_chunks::<4>().0.iter().enumerate() {
            let expected = match (frame, index % 4 < 2) {
                (_, true) => [100, 40, 60, 255],
                (1, false) => [20, 120, 200, 255],
                _ => CLEAR,
            };
            assert_eq!(*pixel, expected, "sample frame {frame}, pixel {index}");
        }
    }
    for (index, pixel) in read(overlay_readback).as_chunks::<4>().0.iter().enumerate() {
        let expected = if index / 4 == 1 && (1..3).contains(&(index % 4)) {
            [128, 0, 255, 255]
        } else if index % 4 < 2 {
            [100, 40, 60, 255]
        } else {
            [20, 120, 200, 255]
        };
        assert_eq!(*pixel, expected, "overlay pixel {index}");
    }
    let palette = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 255, 255, 255],
    ];
    for (address, color) in mrt_readbacks.into_iter().zip(palette) {
        assert_eq!(read(address), color.repeat(12));
    }
    let graphics = services.graphics.lock().unwrap();
    let presentation = graphics.last_presentation.lock().unwrap();
    assert_eq!(presentation.backend, Some(GraphicsBackendId::MetalNative));
    assert_eq!(presentation.frame_index, 4);
    assert_eq!((presentation.width, presentation.height), (4, 3));
    assert_eq!(presentation.pixels, [0, 0, 0, 255].repeat(12));
}

// Shared only by the additional original PE scenes below.
fn finish_guest_submission(
    guest: &mut GuestScript,
    queue: u64,
    list: u64,
    asynchronous: bool,
    draws: u64,
) {
    guest.ok(12, &[H(list)]);
    if asynchronous {
        let receipt = guest.handle(30, &[H(queue), H(list)]);
        guest.call(32, &[H(receipt)], 0, Some(1));
        guest.call(31, &[H(receipt)], 0, Some(1));
        guest.ok(33, &[H(receipt)]);
    } else {
        guest.call(25, &[H(queue), H(list)], 0, Some(draws));
    }
}

fn check_original_guest_scene(
    guest: GuestScript,
    asynchronous: bool,
    readbacks: &[(u64, Vec<u8>)],
    frames: u64,
    final_pixels: &[u8],
) {
    check_original_guest_backend(guest, asynchronous, false, readbacks, frames, final_pixels);
}

fn check_original_guest_backend(
    mut guest: GuestScript,
    asynchronous: bool,
    queued: bool,
    readbacks: &[(u64, Vec<u8>)],
    frames: u64,
    final_pixels: &[u8],
) {
    let pe = guest.finish();
    let services = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    if queued {
        install_queued_pipeline_backend(&mut services.graphics.lock().unwrap()).unwrap();
    } else if asynchronous {
        install_async_pipeline_backend(&mut services.graphics.lock().unwrap()).unwrap();
    } else {
        install_pipeline_backend(&mut services.graphics.lock().unwrap()).unwrap();
    }
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, services.clone());
    let result = run_synthetic_startup(
        &pe,
        StartupLayout {
            image_base: BASE,
            stack_top: 0x500000000,
            tls_base: 0x300000000,
        },
        imports,
        100_000,
    )
    .unwrap();
    assert_eq!(result.execution.exit_reason, ExitReason::Halt);
    assert_eq!(result.bound_imports, 1);
    for (index, call) in guest.calls.iter().enumerate() {
        assert_eq!(
            result.image.memory.read_u64(call.status).unwrap(),
            call.expected_status,
            "call {index}, opcode {}",
            call.opcode
        );
        let value = result.image.memory.read_u64(call.result).unwrap();
        match call.expected_result {
            Some(expected) => assert_eq!(
                value, expected,
                "call {index}, opcode {} result",
                call.opcode
            ),
            None => assert_ne!(value, 0, "call {index} missing handle"),
        }
    }
    for (address, expected) in readbacks {
        let mut bytes = vec![0; expected.len() + 16];
        result
            .image
            .memory
            .read_bytes(*address, &mut bytes)
            .unwrap();
        assert_eq!(&bytes[..8], &[0xa5; 8]);
        assert_eq!(&bytes[bytes.len() - 8..], &[0xa5; 8]);
        assert_eq!(
            &bytes[8..bytes.len() - 8],
            expected.as_slice(),
            "guest readback {address:#x}"
        );
    }
    let graphics = services.graphics.lock().unwrap();
    let presentation = graphics.last_presentation.lock().unwrap();
    // Copy-only submissions must not manufacture a presentation.
    let expected_backend = (frames != 0).then_some(GraphicsBackendId::MetalNative);
    assert_eq!(presentation.backend, expected_backend);
    assert_eq!(presentation.frame_index, frames);
    assert_eq!(presentation.pixels, final_pixels);
}

fn guest_varying_pipeline(guest: &mut GuestScript, device: u64) -> u64 {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
    let vertex = guest.bytes(vs);
    let pixel = guest.bytes(ps);
    guest.handle(
        8,
        &[
            H(device),
            I(vertex),
            I(vs.len() as u64),
            I(pixel),
            I(ps.len() as u64),
        ],
    )
}

fn padded_float4_records(offset: usize, stride: usize, values: &[[f32; 4]]) -> Vec<u8> {
    let mut bytes = vec![0xa5; offset + stride * values.len()];
    for (index, value) in values.iter().enumerate() {
        let start = offset + index * stride;
        bytes[start..start + 16].copy_from_slice(&floats(&[*value]));
    }
    bytes
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_index_views_base_vertex_padding_failure_and_recovery() {
    for asynchronous in [false, true] {
        for format in [57, 42] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queue = guest.handle(2, &[H(device)]);
            let allocator = guest.handle(3, &[H(device)]);
            let list = guest.handle(4, &[H(device), H(allocator)]);
            let pipeline = guest_varying_pipeline(&mut guest, device);
            let positions = padded_float4_records(
                16,
                32,
                &[
                    [-1., -1., 0., 1.],
                    [3., -1., 0., 1.],
                    [-1., 3., 0., 1.],
                    [8., 8., 0., 1.], // decoy reached if base -1 is omitted
                ],
            );
            let vb = guest.handle(5, &[H(device), I(positions.len() as u64), I(COPY_DEST)]);
            let position_address = guest.bytes(&positions);
            guest.ok(
                9,
                &[H(vb), I(0), I(position_address), I(positions.len() as u64)],
            );
            let colors = guest.handle(5, &[H(device), I(104), I(COPY_DEST)]);
            let index_size = if format == 57 { 2 } else { 4 };
            let ib = guest.handle(5, &[H(device), I(9 * index_size), I(COPY_DEST)]);
            let target = guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]);
            let mut readbacks = Vec::new();
            for phase in 0..3 {
                if phase != 0 {
                    guest.ok(11, &[H(list), H(allocator)]);
                }
                // Failed blue draws must not partially replace the prior red frame.
                let color = match phase {
                    0 => [1., 0., 0., 1.],
                    1 => [0., 0., 1., 1.],
                    _ => [0., 1., 0., 1.],
                };
                let color_bytes =
                    padded_float4_records(8, 24, &[color, color, color, [0., 0., 1., 1.]]);
                let color_address = guest.bytes(&color_bytes);
                guest.ok(9, &[H(colors), I(0), I(color_address), I(104)]);
                // View offset skips two entries; start index skips the third.
                // Effective indices [3,1,2,3,1,2] plus base -1 select [2,0,1] twice.
                let indices = [
                    65535u32,
                    65535,
                    65535,
                    3,
                    1,
                    2,
                    3,
                    1,
                    if phase == 1 { 65535 } else { 2 },
                ];
                let index_bytes: Vec<u8> = if format == 57 {
                    indices
                        .iter()
                        .flat_map(|i| (*i as u16).to_le_bytes())
                        .collect()
                } else {
                    indices.iter().flat_map(|i| i.to_le_bytes()).collect()
                };
                let index_address = guest.bytes(&index_bytes);
                guest.ok(
                    9,
                    &[H(ib), I(0), I(index_address), I(index_bytes.len() as u64)],
                );
                if phase == 0 {
                    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                }
                if phase != 0 {
                    guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
                }
                guest.barrier(list, colors, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, ib, COPY_DEST, 2); // INDEX_BUFFER
                guest.targets(list, &[target]);
                guest.ok(14, &[H(list), H(pipeline)]);
                guest.ok(17, &[H(list), I(4), I(3)]);
                guest.ok(18, &[H(list), H(vb), I(16), I(32)]);
                guest.ok(19, &[H(list), H(colors), I(8), I(24)]);
                guest.ok(20, &[H(list), H(ib), I(2 * index_size), I(format)]);
                guest.ok(24, &[H(list), I(6), I(1), I(1), I(u64::MAX), I(0)]);
                guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
                guest.barrier(list, colors, VERTEX_BUFFER, COPY_DEST);
                guest.barrier(list, ib, 2, COPY_DEST);
                if phase == 1 {
                    guest.ok(12, &[H(list)]);
                    // A malformed final index fails preparation: no receipt or frame.
                    guest.call(
                        if asynchronous { 30 } else { 25 },
                        &[H(queue), H(list)],
                        2,
                        Some(u64::MAX),
                    );
                } else {
                    finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                }
                let output = guest.bytes(&[0xa5; 64]);
                guest.ok(10, &[H(target), I(0), I(output + 8), I(48)]);
                readbacks.push((
                    output,
                    if phase == 2 {
                        [0, 255, 0, 255].repeat(12)
                    } else {
                        [255, 0, 0, 255].repeat(12)
                    },
                ));
            }
            check_original_guest_scene(
                guest,
                asynchronous,
                &readbacks,
                2,
                &[0, 255, 0, 255].repeat(12),
            );
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_alpha_additive_masks_and_replace_have_literal_rgba_results() {
    for asynchronous in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let pipeline = guest_varying_pipeline(&mut guest, device);
        let positions = floats(&[[-1., -1., 0., 1.], [3., -1., 0., 1.], [-1., 3., 0., 1.]]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let position_address = guest.bytes(&positions);
        guest.ok(9, &[H(vb), I(0), I(position_address), I(48)]);
        let values = [
            [0., 0., 254. / 255., 63. / 255.], // Replace background
            [254. / 255., 0., 0., 0.5],        // fractional source alpha
            [0., 64. / 255., 0., 32. / 255.],  // Additive
            [1. / 255., 0., 1. / 255., 0.5],   // Alpha blend RGB only, preserving destination A
            [1., 32. / 255., 1., 95. / 255.],  // Replace G/A only
            [17. / 255., 31. / 255., 47. / 255., 63. / 255.], // restore full Replace
        ];
        let records: Vec<[f32; 4]> = values.into_iter().flat_map(|value| [value; 3]).collect();
        let color_bytes = floats(&records);
        let colors = guest.handle(5, &[H(device), I(color_bytes.len() as u64), I(COPY_DEST)]);
        let color_address = guest.bytes(&color_bytes);
        guest.ok(
            9,
            &[
                H(colors),
                I(0),
                I(color_address),
                I(color_bytes.len() as u64),
            ],
        );
        let target = guest.handle(6, &[H(device), I(6), I(3), I(1), I(RENDER_TARGET)]);
        guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
        guest.barrier(list, colors, COPY_DEST, VERTEX_BUFFER);
        guest.targets(list, &[target]);
        guest.ok(14, &[H(list), H(pipeline)]);
        guest.ok(17, &[H(list), I(6), I(3)]);
        guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
        for (draw, (blend, mask, x, width)) in [
            (0, 15, 0, 6),
            (1, 15, 0, 4),
            (2, 15, 1, 3),
            (1, 7, 2, 1),
            (0, 10, 3, 1),
            (0, 15, 4, 1),
        ]
        .into_iter()
        .enumerate()
        {
            guest.ok(19, &[H(list), H(colors), I(draw as u64 * 48), I(16)]);
            guest.ok(
                22,
                &[
                    H(list),
                    I(0),
                    I(0),
                    I(blend),
                    I(mask),
                    I(0),
                    I(0),
                    I(1),
                    I(x),
                    I(0),
                    I(width),
                    I(3),
                ],
            );
            guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
        }
        guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
        finish_guest_submission(&mut guest, queue, list, asynchronous, 6);
        let output = guest.bytes(&[0xa5; 88]);
        guest.ok(10, &[H(target), I(0), I(output + 8), I(72)]);
        // Alpha RGB: 254*.5=127; alpha: 255*.5+63*.5=159.
        // Additive adds G=64 and A=32 without saturation. The next RGB-only
        // alpha draw uses Sa=.5 despite masked alpha: (1+127)/2=64,
        // G=64/2=32, and stored destination alpha stays 191.
        // Separate columns preserve additive-only, masked-alpha, G/A-only,
        // restored Replace/full-mask, and original fractional-destination results.
        let row = [
            127, 0, 127, 159, 127, 64, 127, 191, 64, 32, 64, 191, 127, 32, 127, 95, 17, 31, 47, 63,
            0, 0, 254, 63,
        ];
        let expected = row.repeat(3);
        check_original_guest_scene(
            guest,
            asynchronous,
            &[(output, expected.clone())],
            1,
            &expected,
        );
    }
}

fn sampled_table(
    guest: &mut GuestScript,
    textures: [Option<u64>; 2],
    mips: [Option<u32>; 2],
    malformed: bool,
) -> u64 {
    let mut words = [0u64; 10];
    for (slot, mip) in mips.into_iter().enumerate() {
        if let Some(mip) = mip {
            words[2 + slot * 4] = 1;
            words[3 + slot * 4] = u64::from(mip);
        }
    }
    if malformed {
        words[6] = 2;
    } // noncanonical sampler1-present, after a valid first record
    let address = guest.bytes(
        &words
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    for (slot, texture) in textures.into_iter().enumerate() {
        if let Some(handle) = texture {
            guest.copy_result(handle, address + slot as u64 * 8);
        }
    }
    address
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_multislot_pairs_rebind_upload_and_retire_independently() {
    for asynchronous in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let va = guest.bytes(vs);
        let mut pipelines = Vec::new();
        for ps in [
            include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin").as_slice(),
            include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin").as_slice(),
            include_bytes!("../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin").as_slice(),
        ] {
            let pa = guest.bytes(ps);
            pipelines.push(guest.handle(
                8,
                &[
                    H(device),
                    I(va),
                    I(vs.len() as u64),
                    I(pa),
                    I(ps.len() as u64),
                ],
            ));
        }
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let attributes = guest.bytes(&floats(&[
            [0.5, 0.5, 0., 1.],
            [0.5, 0.5, 0., 1.],
            [0.5, 0.5, 0., 1.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
            [0., 0., 1., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(attributes), I(96)]);
        let textures = [
            guest.handle(6, &[H(device), I(2), I(2), I(2), I(COPY_DEST)]),
            guest.handle(6, &[H(device), I(2), I(2), I(2), I(COPY_DEST)]),
        ];
        let target = guest.handle(6, &[H(device), I(8), I(3), I(1), I(RENDER_TARGET)]);
        let mut readbacks = Vec::new();
        for frame in 0..2 {
            if frame > 0 {
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let (a0, a1, b0, b1) = if frame == 0 {
                (
                    [18, 31, 47, 255],
                    [72, 83, 97, 255],
                    [110, 127, 139, 211],
                    [152, 167, 181, 199],
                )
            } else {
                (
                    [20, 33, 49, 253],
                    [74, 85, 99, 251],
                    [112, 129, 141, 209],
                    [154, 169, 183, 197],
                )
            };
            for (texture, base, mip) in [(textures[0], a0, a1), (textures[1], b0, b1)] {
                let mut pixels = base.repeat(4);
                pixels.extend_from_slice(&mip);
                let address = guest.bytes(&pixels);
                guest.ok(9, &[H(texture), I(0), I(address), I(20)]);
                guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
            }
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(8), I(3)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            for (draw, (program, x, width)) in
                [(0, 0, 8), (0, 1, 7), (1, 2, 5), (2, 3, 3), (0, 4, 1)]
                    .into_iter()
                    .enumerate()
            {
                guest.ok(14, &[H(list), H(pipelines[program])]);
                guest.ok(
                    19,
                    &[H(list), H(cb), I(if program == 1 { 48 } else { 0 }), I(16)],
                );
                let (bound, mips) = match program {
                    0 => (
                        [Some(textures[0]), Some(textures[1])],
                        if draw == 1 {
                            [Some(1), Some(0)]
                        } else {
                            [Some(0), Some(1)]
                        },
                    ),
                    _ => ([None, None], [None, None]),
                };
                let table = sampled_table(&mut guest, bound, mips, false);
                guest.ok(34, &[H(list), I(table)]);
                if draw == 0 {
                    // Invalid last record must not partially replace the original texture table.
                    let invalid = sampled_table(
                        &mut guest,
                        [Some(textures[1]), Some(textures[0])],
                        mips,
                        true,
                    );
                    guest.call(34, &[H(list), I(invalid)], 2, Some(u64::MAX));
                }
                if program == 2 {
                    // Existing opcode21 continues to bind slot0 after clearing the new tables.
                    guest.ok(21, &[H(list), H(textures[0]), I(1), I(0), I(0)]);
                }
                guest.ok(
                    22,
                    &[
                        H(list),
                        I(0),
                        I(0),
                        I(0),
                        I(15),
                        I(0),
                        I(0),
                        I(1),
                        I(x),
                        I(0),
                        I(width),
                        I(3),
                    ],
                );
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            }
            for texture in textures {
                guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
            }
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                for resource in [textures[0], textures[1], vb, cb] {
                    guest.ok(26, &[H(resource)]);
                }
                for pipeline in &pipelines {
                    guest.ok(27, &[H(*pipeline)]);
                }
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 5);
            }
            let (normal, swapped, legacy) = if frame == 0 {
                ([72, 127, 47, 199], [18, 167, 97, 211], [36, 83, 97, 255])
            } else {
                ([74, 129, 49, 197], [20, 169, 99, 209], [37, 85, 99, 251])
            };
            let row = [
                normal,
                swapped,
                [0, 0, 255, 255],
                legacy,
                normal,
                legacy,
                [0, 0, 255, 255],
                swapped,
            ];
            let expected = row.repeat(3).into_iter().flatten().collect::<Vec<_>>();
            let output = guest.bytes(&[0xa5; 112]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(96)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_scene(guest, asynchronous, &readbacks, 2, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_two_queues_sample_produced_versions_and_retire() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
    let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
    let lists = allocators.map(|allocator| guest.handle(4, &[H(device), H(allocator)]));
    let producer = guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]);
    let target = guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]);
    let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let positions = guest.bytes(&floats(&[
        [-1., -1., 0., 1.],
        [3., -1., 0., 1.],
        [-1., 3., 0., 1.],
    ]));
    let attributes = guest.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
    guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
    guest.ok(9, &[H(cb), I(0), I(attributes), I(48)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-sampled-arithmetic-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let pipeline = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );
    let mut readbacks = Vec::new();
    for (frame, color) in [[200u8, 40, 60, 255], [80, 120, 160, 255]]
        .into_iter()
        .enumerate()
    {
        if frame > 0 {
            guest.barrier(lists[0], producer, PIXEL_RESOURCE, RENDER_TARGET);
            guest.barrier(lists[1], target, COPY_SOURCE, RENDER_TARGET);
        } else {
            guest.barrier(lists[1], vb, COPY_DEST, VERTEX_BUFFER);
            guest.barrier(lists[1], cb, COPY_DEST, VERTEX_BUFFER);
        }
        let mut record = [0u8; 16];
        record[8..12].copy_from_slice(&color);
        let clear = guest.bytes(&record);
        guest.copy_result(producer, clear);
        guest.ok(15, &[H(lists[0]), I(1), I(clear)]);
        guest.barrier(lists[0], producer, RENDER_TARGET, PIXEL_RESOURCE);
        guest.ok(12, &[H(lists[0])]);
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(a)], 0, Some(frame as u64 + 1));
        guest.targets(lists[1], &[target]);
        guest.ok(17, &[H(lists[1]), I(4), I(3)]);
        guest.ok(14, &[H(lists[1]), H(pipeline)]);
        guest.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
        guest.ok(19, &[H(lists[1]), H(cb), I(0), I(16)]);
        guest.ok(
            21,
            &[
                H(lists[1]),
                H(producer),
                I(0xffffffff),
                I(0),
                I(u64::from(f32::MAX.to_bits())),
            ],
        );
        guest.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let wait = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], wait);
        guest.copy_result(value, wait + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for i in 0..2 {
            guest.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        if frame == 1 {
            for resource in [producer, vb, cb] {
                guest.ok(26, &[H(resource)]);
            }
            guest.ok(27, &[H(pipeline)]);
        }
        guest.call(32, &[H(b)], 0, Some(1));
        guest.call(31, &[H(a)], 0, Some(1));
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let output = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(target), I(0), I(output + 8), I(48)]);
        readbacks.push((
            output,
            [color[0] / 2, color[1], color[2], color[3]].repeat(12),
        ));
    }
    let final_pixels = readbacks.last().unwrap().1.clone();
    check_original_guest_backend(guest, true, true, &readbacks, 4, &final_pixels);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_texture_views_rebase_mips_decode_srgb_and_retire() {
    for asynchronous in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let texture = guest.handle(6, &[H(device), I(4), I(4), I(3), I(COPY_DEST)]);
        // Same storage, differing interpretation and relative mip origin.
        let linear = guest.handle(37, &[H(device), H(texture), I(1), I(2), I(0)]);
        let srgb = guest.handle(37, &[H(device), H(texture), I(1), I(2), I(1)]);
        let last = guest.handle(37, &[H(device), H(texture), I(2), I(1), I(0)]);
        guest.call(
            37,
            &[H(device), H(texture), I(2), I(2), I(0)],
            2,
            Some(u64::MAX),
        );
        guest.call(26, &[H(texture)], 2, Some(u64::MAX));
        let target = guest.handle(6, &[H(device), I(5), I(3), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let attributes = guest.bytes(&floats(&[
            [0.25, 0.25, 0., 1.],
            [0.25, 0.25, 0., 1.],
            [0.25, 0.25, 0., 1.],
            [0.75, 0.75, 0., 1.],
            [0.75, 0.75, 0., 1.],
            [0.75, 0.75, 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(attributes), I(96)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for frame in 0..2 {
            let (first, decoded, spatial, mip2) = if frame == 0 {
                (
                    [128u8, 64, 192, 128],
                    [55, 13, 134, 128],
                    [16, 32, 48, 96],
                    [40, 120, 200, 64],
                )
            } else {
                (
                    [64, 128, 32, 192],
                    [13, 55, 4, 192],
                    [80, 160, 240, 32],
                    [200, 80, 40, 128],
                )
            };
            let mut bytes = [200u8, 10, 20, 255].repeat(16);
            bytes.extend(
                [first, [8, 24, 40, 56], [72, 88, 104, 120], spatial]
                    .into_iter()
                    .flatten(),
            );
            bytes.extend(mip2);
            let upload = guest.bytes(&bytes);
            guest.ok(9, &[H(texture), I(0), I(upload), I(84)]);
            guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(5), I(3)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            for (column, (view, mip)) in
                [(linear, 0), (srgb, 0), (linear, 1), (last, 0), (linear, 0)]
                    .into_iter()
                    .enumerate()
            {
                guest.ok(
                    19,
                    &[H(list), H(cb), I(if column == 4 { 48 } else { 0 }), I(16)],
                );
                let table = sampled_table(&mut guest, [Some(view), None], [Some(mip), None], false);
                guest.ok(39, &[H(list), I(table)]);
                if column == 0 {
                    // A malformed final sampler must not replace the earlier view binding.
                    let bad = sampled_table(&mut guest, [Some(srgb), None], [Some(0), None], true);
                    guest.call(39, &[H(list), I(bad)], 2, Some(u64::MAX));
                }
                guest.ok(
                    22,
                    &[
                        H(list),
                        I(0),
                        I(0),
                        I(0),
                        I(15),
                        I(0),
                        I(0),
                        I(1),
                        I(column as u64),
                        I(0),
                        I(1),
                        I(3),
                    ],
                );
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            }
            guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            guest.call(38, &[H(linear)], 2, Some(u64::MAX));
            if asynchronous && frame == 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                for view in [linear, srgb, last] {
                    guest.ok(38, &[H(view)]);
                }
                for resource in [texture, vb, cb] {
                    guest.ok(26, &[H(resource)]);
                }
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 5);
                guest.ok(11, &[H(list), H(allocator)]);
                if frame == 1 {
                    for view in [linear, srgb, last] {
                        guest.ok(38, &[H(view)]);
                    }
                    guest.ok(26, &[H(texture)]);
                }
            }
            let row = [first, decoded, mip2, mip2, spatial];
            let expected = row.repeat(3).into_iter().flatten().collect::<Vec<_>>();
            let output = guest.bytes(&[0xa5; 76]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(60)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_scene(guest, asynchronous, &readbacks, 2, &final_pixels);
    }
}

// Original endpoint/selector packing, independent of production BC decoders.
fn original_bc_block(
    format: u64,
    color0: u16,
    color1: u16,
    alpha0: u8,
    alpha1: u8,
    spatial: bool,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    if format == 2 {
        bytes.extend([alpha0, alpha1]);
        let selectors = if spatial { 1u64 << 45 } else { 0 };
        bytes.extend_from_slice(&selectors.to_le_bytes()[..6]);
    }
    bytes.extend(color0.to_le_bytes());
    bytes.extend(color1.to_le_bytes());
    bytes.extend((if spatial { 1u32 << 30 } else { 0 }).to_le_bytes());
    bytes
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_bc_textures_rebase_decode_and_retire() {
    for format in [1u64, 2] {
        for asynchronous in [false, true] {
            let linear_format = if format == 1 { 2 } else { 4 };
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queue = guest.handle(2, &[H(device)]);
            let allocator = guest.handle(3, &[H(device)]);
            let list = guest.handle(4, &[H(device), H(allocator)]);
            let texture = guest.handle(40, &[H(device), I(8), I(8), I(3), I(format), I(COPY_DEST)]);
            // Same storage, differing interpretation and relative mip origin.
            let linear = guest.handle(37, &[H(device), H(texture), I(1), I(2), I(linear_format)]);
            let srgb = guest.handle(
                37,
                &[H(device), H(texture), I(1), I(2), I(linear_format + 1)],
            );
            let last = guest.handle(37, &[H(device), H(texture), I(2), I(1), I(linear_format)]);
            guest.call(
                37,
                &[H(device), H(texture), I(2), I(2), I(linear_format)],
                2,
                Some(u64::MAX),
            );
            guest.call(26, &[H(texture)], 2, Some(u64::MAX));
            let target = guest.handle(6, &[H(device), I(5), I(3), I(1), I(RENDER_TARGET)]);
            let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
            let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
            let positions = guest.bytes(&floats(&[
                [-1., -1., 0., 1.],
                [3., -1., 0., 1.],
                [-1., 3., 0., 1.],
            ]));
            let attributes = guest.bytes(&floats(&[
                [0.25, 0.25, 0., 1.],
                [0.25, 0.25, 0., 1.],
                [0.25, 0.25, 0., 1.],
                [0.75, 0.75, 0., 1.],
                [0.75, 0.75, 0., 1.],
                [0.75, 0.75, 0., 1.],
            ]));
            guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
            guest.ok(9, &[H(cb), I(0), I(attributes), I(96)]);
            let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
            let ps = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
            let va = guest.bytes(vs);
            let pa = guest.bytes(ps);
            let pipeline = guest.handle(
                8,
                &[
                    H(device),
                    I(va),
                    I(vs.len() as u64),
                    I(pa),
                    I(ps.len() as u64),
                ],
            );
            let mut readbacks = Vec::new();
            for frame in 0..2 {
                let alpha = if format == 1 {
                    255
                } else if frame == 0 {
                    128
                } else {
                    192
                };
                let spatial_alpha = if format == 1 { 255 } else { 64 };
                let (first, decoded, spatial, mip2, endpoint, other, tail) = if frame == 0 {
                    (
                        [132, 130, 132, alpha],
                        [59, 57, 59, alpha],
                        [0, 255, 0, spatial_alpha],
                        [255, 0, 0, 255],
                        0x8410,
                        0x07e0,
                        0xf800,
                    )
                } else {
                    (
                        [66, 65, 66, alpha],
                        [14, 13, 14, alpha],
                        [0, 0, 255, spatial_alpha],
                        [0, 255, 0, 255],
                        0x4208,
                        0x001f,
                        0x07e0,
                    )
                };
                // Four base blocks, then one block per selected mip. Selector15 uses
                // endpoint1; selector5 uses endpoint0. Expectations are literal pixels.
                let mut bytes = original_bc_block(format, 0xf800, 0, 255, 255, false).repeat(4);
                bytes.extend(original_bc_block(
                    format,
                    endpoint,
                    other,
                    alpha,
                    spatial_alpha,
                    true,
                ));
                bytes.extend(original_bc_block(format, tail, 0, 255, 255, false));
                let upload = guest.bytes(&bytes);
                guest.ok(9, &[H(texture), I(0), I(upload), I(bytes.len() as u64)]);
                guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
                if frame == 0 {
                    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                    guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
                } else {
                    guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
                }
                guest.targets(list, &[target]);
                guest.ok(17, &[H(list), I(5), I(3)]);
                guest.ok(14, &[H(list), H(pipeline)]);
                guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
                for (column, (view, mip)) in
                    [(linear, 0), (srgb, 0), (linear, 1), (last, 0), (linear, 0)]
                        .into_iter()
                        .enumerate()
                {
                    guest.ok(
                        19,
                        &[H(list), H(cb), I(if column == 4 { 48 } else { 0 }), I(16)],
                    );
                    let table =
                        sampled_table(&mut guest, [Some(view), None], [Some(mip), None], false);
                    guest.ok(39, &[H(list), I(table)]);
                    if column == 0 {
                        // A malformed final sampler must not replace the earlier view binding.
                        let bad =
                            sampled_table(&mut guest, [Some(srgb), None], [Some(0), None], true);
                        guest.call(39, &[H(list), I(bad)], 2, Some(u64::MAX));
                    }
                    guest.ok(
                        22,
                        &[
                            H(list),
                            I(0),
                            I(0),
                            I(0),
                            I(15),
                            I(0),
                            I(0),
                            I(1),
                            I(column as u64),
                            I(0),
                            I(1),
                            I(3),
                        ],
                    );
                    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
                }
                guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
                guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
                guest.call(38, &[H(linear)], 2, Some(u64::MAX));
                if asynchronous && frame == 1 {
                    guest.ok(12, &[H(list)]);
                    let receipt = guest.handle(30, &[H(queue), H(list)]);
                    guest.ok(11, &[H(list), H(allocator)]);
                    for view in [linear, srgb, last] {
                        guest.ok(38, &[H(view)]);
                    }
                    for resource in [texture, vb, cb] {
                        guest.ok(26, &[H(resource)]);
                    }
                    guest.ok(27, &[H(pipeline)]);
                    guest.call(32, &[H(receipt)], 0, Some(1));
                    guest.ok(33, &[H(receipt)]);
                } else {
                    finish_guest_submission(&mut guest, queue, list, asynchronous, 5);
                    guest.ok(11, &[H(list), H(allocator)]);
                    if frame == 1 {
                        for view in [linear, srgb, last] {
                            guest.ok(38, &[H(view)]);
                        }
                        guest.ok(26, &[H(texture)]);
                    }
                }
                let row = [first, decoded, mip2, mip2, spatial];
                let expected = row.repeat(3).into_iter().flatten().collect::<Vec<_>>();
                let output = guest.bytes(&[0xa5; 76]);
                guest.ok(10, &[H(target), I(0), I(output + 8), I(60)]);
                readbacks.push((output, expected));
            }
            let final_pixels = readbacks.last().unwrap().1.clone();
            check_original_guest_scene(guest, asynchronous, &readbacks, 2, &final_pixels);
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_runtime_uniforms_change_geometry_and_color_without_recompile() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let target = guest.handle(6, &[H(device), I(8), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        // Centered half-width rectangle. Runtime s24 shifts it exactly one half.
        let positions = guest.bytes(&floats(&[
            [-0.5, -1., 0., 1.],
            [0.5, -1., 0., 1.],
            [-0.5, 1., 0., 1.],
            [-0.5, 1., 0., 1.],
            [0.5, -1., 0., 1.],
            [0.5, 1., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(96)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-runtime-uniform-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for (frame, (geometry, color)) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
            .into_iter()
            .enumerate()
        {
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(8), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            let words = [geometry, color, 0, 0, 0, 0, 0, 0];
            let address = guest.bytes(
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            guest.ok(41, &[H(list), I(address), I(1)]);
            // Noncanonical later binding must not overwrite the valid snapshot.
            guest.call(41, &[H(list), I(address), I(2)], 2, Some(u64::MAX));
            guest.ok(23, &[H(list), I(6), I(1), I(0), I(0)]);
            guest.ok(41, &[H(list), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == 3 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(26, &[H(vb)]);
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let tint = if color == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 255, 0, 255]
            };
            let mut row = if geometry == 0 {
                tint.repeat(4)
            } else {
                CLEAR.repeat(4)
            };
            row.extend(if geometry == 0 {
                CLEAR.repeat(4)
            } else {
                tint.repeat(4)
            });
            let expected = row.repeat(2);
            let output = guest.bytes(&[0xa5; 80]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(64)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 4, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_color_load_preserves_producer_pixels_across_frames() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
    let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
    let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
    let target = guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]);
    let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let positions = guest.bytes(&floats(&[
        [-1., -1., 0., 1.],
        [3., -1., 0., 1.],
        [-1., 3., 0., 1.],
    ]));
    guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let pipeline = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );
    let mut readbacks = Vec::new();
    for (frame, color) in [[17u8, 31, 47, 63], [71, 89, 101, 127]]
        .into_iter()
        .enumerate()
    {
        if frame > 0 {
            guest.barrier(lists[0], target, COPY_SOURCE, RENDER_TARGET);
        } else {
            guest.barrier(lists[1], vb, COPY_DEST, VERTEX_BUFFER);
        }
        let mut record = [0; 16];
        record[8..12].copy_from_slice(&color);
        let clear = guest.bytes(&record);
        guest.copy_result(target, clear);
        guest.ok(15, &[H(lists[0]), I(1), I(clear)]);
        guest.ok(12, &[H(lists[0])]);
        let first = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(first)], 0, Some(frame as u64 + 1));
        guest.load_targets(lists[1], &[target]);
        guest.ok(17, &[H(lists[1]), I(4), I(3)]);
        guest.ok(14, &[H(lists[1]), H(pipeline)]);
        guest.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
        guest.ok(
            22,
            &[
                H(lists[1]),
                I(0),
                I(0),
                I(0),
                I(15),
                I(0),
                I(0),
                I(1),
                I(0),
                I(0),
                I(1),
                I(3),
            ],
        );
        guest.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let wait = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], wait);
        guest.copy_result(value, wait + 8);
        let second = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for i in 0..2 {
            guest.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        if frame == 1 {
            guest.ok(26, &[H(vb)]);
            guest.ok(27, &[H(pipeline)]);
        }
        guest.call(32, &[H(second)], 0, Some(1));
        guest.call(31, &[H(first)], 0, Some(1));
        guest.ok(33, &[H(first)]);
        guest.ok(33, &[H(second)]);
        let output = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(target), I(0), I(output + 8), I(48)]);
        let expected: Vec<u8> = (0..12)
            .flat_map(|i| if i % 4 == 0 { [255, 0, 0, 255] } else { color })
            .collect();
        readbacks.push((output, expected));
    }
    let final_pixels = readbacks.last().unwrap().1.clone();
    check_original_guest_backend(guest, true, true, &readbacks, 4, &final_pixels);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_depth_versions_preserve_readonly_update_and_clear() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
    let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
    let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
    let targets = [0, 1].map(|_| guest.handle(6, &[H(device), I(4), I(2), I(1), I(RENDER_TARGET)]));
    let depth = guest.handle(7, &[H(device), I(4), I(2)]);
    let vb = guest.handle(5, &[H(device), I(192), I(COPY_DEST)]);
    let positions: Vec<[f32; 4]> = [0.25, 0.75, 0.5, 0.625]
        .into_iter()
        .flat_map(|z| [[-1., -1., z, 1.], [3., -1., z, 1.], [-1., 3., z, 1.]])
        .collect();
    let bytes = guest.bytes(&floats(&positions));
    guest.ok(9, &[H(vb), I(0), I(bytes), I(192)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let pipeline = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );
    let mut readbacks = Vec::new();
    for phase in 0..4 {
        if phase == 0 {
            guest.barrier(lists[0], vb, COPY_DEST, VERTEX_BUFFER);
        } else {
            guest.barrier(lists[0], depth, 0x20, 0x10);
            guest.barrier(lists[1], targets[1], COPY_SOURCE, RENDER_TARGET);
        }
        guest.targets(lists[0], &[targets[0]]);
        guest.ok(
            16,
            &[
                H(lists[0]),
                H(depth),
                I(u64::from(phase == 1 || phase == 2)),
                I(if phase == 1 || phase == 2 {
                    0
                } else if phase == 0 {
                    1f32.to_bits() as u64
                } else {
                    0.875f32.to_bits() as u64
                }),
            ],
        );
        if phase == 0 || phase == 2 {
            guest.ok(17, &[H(lists[0]), I(4), I(2)]);
            guest.ok(14, &[H(lists[0]), H(pipeline)]);
            guest.ok(18, &[H(lists[0]), H(vb), I(0), I(16)]);
            if phase == 0 {
                for half in 0..2 {
                    guest.ok(
                        22,
                        &[
                            H(lists[0]),
                            I(8),
                            I(1),
                            I(0),
                            I(15),
                            I(0),
                            I(0),
                            I(1),
                            I(half * 2),
                            I(0),
                            I(2),
                            I(2),
                        ],
                    );
                    guest.ok(23, &[H(lists[0]), I(3), I(1), I(half * 3), I(0)]);
                }
            } else {
                guest.ok(
                    22,
                    &[
                        H(lists[0]),
                        I(2),
                        I(1),
                        I(0),
                        I(15),
                        I(0),
                        I(0),
                        I(0),
                        I(0),
                        I(0),
                        I(0),
                        I(0),
                    ],
                );
                guest.ok(23, &[H(lists[0]), I(3), I(1), I(6), I(0)]);
            }
        }
        guest.barrier(lists[0], depth, 0x10, 0x20);
        guest.ok(12, &[H(lists[0])]);
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(a)], 0, Some(phase + 1));
        guest.targets(lists[1], &[targets[1]]);
        guest.ok(16, &[H(lists[1]), H(depth), I(1), I(0)]);
        guest.ok(17, &[H(lists[1]), I(4), I(2)]);
        guest.ok(14, &[H(lists[1]), H(pipeline)]);
        guest.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
        guest.ok(
            22,
            &[
                H(lists[1]),
                I(2),
                I(0),
                I(0),
                I(15),
                I(0),
                I(0),
                I(0),
                I(0),
                I(0),
                I(0),
                I(0),
            ],
        );
        guest.ok(
            23,
            &[
                H(lists[1]),
                I(3),
                I(1),
                I(if phase == 1 || phase == 2 { 9 } else { 6 }),
                I(0),
            ],
        );
        guest.barrier(lists[1], targets[1], RENDER_TARGET, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let wait = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], wait);
        guest.copy_result(value, wait + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for i in 0..2 {
            guest.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        if phase == 3 {
            for resource in [depth, vb] {
                guest.ok(26, &[H(resource)]);
            }
            guest.ok(27, &[H(pipeline)]);
        }
        guest.call(32, &[H(b)], 0, Some(1));
        guest.call(31, &[H(a)], 0, Some(1));
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let output = guest.bytes(&[0xa5; 48]);
        guest.ok(10, &[H(targets[1]), I(0), I(output + 8), I(32)]);
        let expected: Vec<u8> = (0..8)
            .flat_map(|i| {
                if phase == 3 || (phase < 2 && i % 4 >= 2) {
                    [255, 0, 0, 255]
                } else {
                    CLEAR
                }
            })
            .collect();
        readbacks.push((output, expected));
    }
    let final_pixels = readbacks.last().unwrap().1.clone();
    check_original_guest_backend(guest, true, true, &readbacks, 8, &final_pixels);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_scalar_arithmetic_observes_result_and_carry() {
    // Literal result/flag oracles, independent of the production scalar evaluator.
    // Reuse one compiled pipeline across wrap, no-wrap and deliberate mismatches.
    let cases = [
        ([0xffff_ffffu32, 1, 0, 0], true, [255, 255, 0, 255]),
        ([1, 2, 1, 3], true, [255, 0, 0, 255]),
        ([0xffff_ffff, 1, 1, 1], false, [0, 255, 0, 255]),
        ([0x7fc0_0000, 0, 0, 0x7fc0_0000], true, [255, 0, 0, 255]),
        ([0, 0, 1, 1], false, [0, 0, 0, 255]),
    ];
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let target = guest.handle(6, &[H(device), I(8), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [1., -1., 0., 1.],
            [-1., 1., 0., 1.],
            [-1., 1., 0., 1.],
            [1., -1., 0., 1.],
            [1., 1., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(96)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-scalar-add-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-scalar-add-u32-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for (frame, (inputs, left, tint)) in cases.into_iter().enumerate() {
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(8), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            let mut words = [0u32; 8];
            words[..4].copy_from_slice(&inputs);
            let address = guest.bytes(
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            guest.ok(41, &[H(list), I(address), I(1)]);
            guest.ok(23, &[H(list), I(6), I(1), I(0), I(0)]);
            // Recorded draw retains its values after subsequent binding changes.
            guest.ok(41, &[H(list), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == cases.len() - 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(26, &[H(vb)]);
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let mut row = if left {
                tint.repeat(4)
            } else {
                CLEAR.repeat(4)
            };
            row.extend(if left {
                CLEAR.repeat(4)
            } else {
                tint.repeat(4)
            });
            let expected = row.repeat(2);
            let output = guest.bytes(&[0xa5; 80]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(64)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(
            guest,
            asynchronous,
            queued,
            &readbacks,
            cases.len() as u64,
            &final_pixels,
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_copy_buffer_chain_preserves_37_bytes_without_presentation() {
    const BYTES: [u8; 37] = [
        0, 255, 128, 1, 17, 83, 149, 231, 2, 254, 127, 64, 3, 9, 27, 81, 243, 7, 11, 13, 19, 23,
        29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 89, 97,
    ];
    // Each order uses only two receipts; this is not a three-node concurrent chain.
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
        let lists = [
            guest.handle(4, &[H(device), H(allocators[0])]),
            guest.handle(4, &[H(device), H(allocators[1])]),
        ];
        let source = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
        let intermediate = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
        let destination = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
        let input = guest.bytes(&BYTES);
        guest.ok(9, &[H(source), I(0), I(input), I(37)]);
        // Record everything before submitting, including the consumer's source state.
        guest.barrier(lists[0], source, COPY_DEST, COPY_SOURCE);
        guest.ok(42, &[H(lists[0]), H(intermediate), H(source)]);
        guest.barrier(lists[0], intermediate, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[0])]);
        guest.ok(42, &[H(lists[1]), H(destination), H(intermediate)]);
        guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let producer = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(producer)], 0, Some(1));
        let dependency = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], dependency);
        guest.copy_result(value, dependency + 8);
        let consumer = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        guest.ok(26, &[H(source)]);
        guest.ok(26, &[H(intermediate)]);
        let order = if producer_first {
            [producer, consumer]
        } else {
            [consumer, producer]
        };
        for receipt in order {
            // Opcode 32 returns Complete (1), not a draw count.
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        guest.ok(33, &[H(producer)]);
        guest.ok(33, &[H(consumer)]);
        let output = guest.bytes(&[0xa5; 53]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(37)]);
        check_original_guest_backend(guest, true, true, &[(output, BYTES.to_vec())], 0, &[]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_render_to_copy_preserves_pixels_without_extra_frame() {
    guest_queued_texture_copy_pair(0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_copy_to_sample_preserves_spatial_pixels() {
    guest_queued_texture_copy_pair(1);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_copy_to_color_load_preserves_untouched_pixels() {
    guest_queued_texture_copy_pair(2);
}

// Mode 0: render -> copy; 1: copy -> sample; 2: copy -> ColorLoad.
// The guest submits both receipts before waiting. Native gated tests establish
// physical overlap; this guest test does not assume a particular GPU schedule.
fn guest_queued_texture_copy_pair(mode: u8) {
    const TEXELS: [u8; 24] = [
        17, 83, 149, 231, 211, 43, 97, 127, 3, 251, 61, 19, 89, 5, 239, 67, 157, 199, 7, 181, 253,
        109, 31, 255,
    ];
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
        let lists = [
            guest.handle(4, &[H(device), H(allocators[0])]),
            guest.handle(4, &[H(device), H(allocators[1])]),
        ];
        let source_state = if mode == 0 { RENDER_TARGET } else { COPY_DEST };
        let source = guest.handle(6, &[H(device), I(3), I(2), I(1), I(source_state)]);
        let copied = guest.handle(6, &[H(device), I(3), I(2), I(1), I(COPY_DEST)]);
        let target = if mode == 1 {
            guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)])
        } else {
            copied
        };
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        let vs: &[u8] = if mode == 1 {
            include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin")
        } else {
            include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin")
        };
        let ps: &[u8] = if mode == 1 {
            include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin")
        } else {
            include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin")
        };
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let render_list = lists[usize::from(mode != 0)];
        guest.barrier(render_list, vb, COPY_DEST, VERTEX_BUFFER);
        let mut attributes = None;
        if mode == 0 {
            guest.targets(render_list, &[source]);
        } else {
            let input = guest.bytes(&TEXELS);
            guest.ok(9, &[H(source), I(0), I(input), I(24)]);
            guest.barrier(lists[0], source, COPY_DEST, COPY_SOURCE);
            guest.ok(42, &[H(lists[0]), H(copied), H(source)]);
            guest.barrier(
                lists[0],
                copied,
                COPY_DEST,
                if mode == 1 {
                    PIXEL_RESOURCE
                } else {
                    RENDER_TARGET
                },
            );
            if mode == 1 {
                guest.targets(render_list, &[target]);
                // One constant coordinate per pixel avoids deriving the oracle
                // from interpolation or an implementation-side texture decoder.
                let values: Vec<[f32; 4]> = (0..6)
                    .flat_map(|pixel| {
                        [[
                            (pixel % 3) as f32 / 3. + 1. / 6.,
                            (pixel / 3) as f32 / 2. + 0.25,
                            0.,
                            1.,
                        ]; 3]
                    })
                    .collect();
                let bytes = floats(&values);
                let cb = guest.handle(5, &[H(device), I(bytes.len() as u64), I(COPY_DEST)]);
                let data = guest.bytes(&bytes);
                guest.ok(9, &[H(cb), I(0), I(data), I(bytes.len() as u64)]);
                guest.barrier(render_list, cb, COPY_DEST, VERTEX_BUFFER);
                attributes = Some(cb);
                guest.ok(
                    21,
                    &[
                        H(render_list),
                        H(copied),
                        I(0xffffffff),
                        I(0),
                        I(u64::from(f32::MAX.to_bits())),
                    ],
                );
            } else {
                guest.load_targets(render_list, &[target]);
            }
        }
        guest.ok(17, &[H(render_list), I(3), I(2)]);
        guest.ok(14, &[H(render_list), H(pipeline)]);
        guest.ok(18, &[H(render_list), H(vb), I(0), I(16)]);
        for pixel in 0..if mode == 1 { 6 } else { 1 } {
            if let Some(cb) = attributes {
                guest.ok(19, &[H(render_list), H(cb), I(pixel * 48), I(16)]);
            }
            let (x, y, height) = if mode == 1 {
                (pixel % 3, pixel / 3, 1)
            } else {
                (0, 0, 2)
            };
            guest.ok(
                22,
                &[
                    H(render_list),
                    I(0),
                    I(0),
                    I(0),
                    I(15),
                    I(0),
                    I(0),
                    I(1),
                    I(x),
                    I(y),
                    I(1),
                    I(height),
                ],
            );
            guest.ok(23, &[H(render_list), I(3), I(1), I(0), I(0)]);
        }
        if mode == 0 {
            guest.barrier(lists[0], source, RENDER_TARGET, COPY_SOURCE);
            guest.ok(42, &[H(lists[1]), H(copied), H(source)]);
            guest.barrier(lists[1], copied, COPY_DEST, COPY_SOURCE);
        } else {
            guest.barrier(render_list, target, RENDER_TARGET, COPY_SOURCE);
        }
        for list in lists {
            guest.ok(12, &[H(list)]);
        }
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(a)], 0, Some(1));
        let dependency = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], dependency);
        guest.copy_result(value, dependency + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        for resource in [source, vb] {
            guest.ok(26, &[H(resource)]);
        }
        if let Some(cb) = attributes {
            guest.ok(26, &[H(cb)]);
            guest.ok(26, &[H(copied)]);
        }
        guest.ok(27, &[H(pipeline)]);
        for receipt in if producer_first { [a, b] } else { [b, a] } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let mut expected = if mode == 0 {
            CLEAR.repeat(6)
        } else {
            TEXELS.to_vec()
        };
        if mode != 1 {
            expected[..4].copy_from_slice(&[255, 0, 0, 255]);
            expected[12..16].copy_from_slice(&[255, 0, 0, 255]);
        }
        let output = guest.bytes(&[0xa5; 40]);
        guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
        // Exactly one render in either direction; the copy must not advance frames.
        check_original_guest_backend(
            guest,
            true,
            true,
            &[(output, expected.clone())],
            1,
            &expected,
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_buffer_region_pending_source_preserves_destination() {
    guest_queued_buffer_region_pair(true);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_buffer_region_pending_destination_preserves_prior_version() {
    guest_queued_buffer_region_pair(false);
}

fn guest_queued_buffer_region_pair(pending_source: bool) {
    const SOURCE: [u8; 16] = [
        91, 73, 55, 37, 255, 2, 129, 4, 17, 83, 149, 231, 13, 29, 61, 127,
    ];
    const PRIOR: [u8; 21] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
        119, 120, 121,
    ];
    const EXPECTED: [u8; 21] = [
        101, 102, 103, 104, 105, 106, 107, 108, 255, 2, 129, 4, 17, 83, 149, 231, 117, 118, 119,
        120, 121,
    ];
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
        let lists = [
            guest.handle(4, &[H(device), H(allocators[0])]),
            guest.handle(4, &[H(device), H(allocators[1])]),
        ];
        let source = guest.handle(5, &[H(device), I(16), I(COPY_DEST)]);
        let destination = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        let seed_bytes = if pending_source {
            SOURCE.as_slice()
        } else {
            PRIOR.as_slice()
        };
        let seed = guest.handle(5, &[H(device), I(seed_bytes.len() as u64), I(COPY_DEST)]);
        let seed_data = guest.bytes(seed_bytes);
        guest.ok(
            9,
            &[H(seed), I(0), I(seed_data), I(seed_bytes.len() as u64)],
        );
        let (host_resource, host_bytes) = if pending_source {
            (destination, PRIOR.as_slice())
        } else {
            (source, SOURCE.as_slice())
        };
        let host_data = guest.bytes(host_bytes);
        guest.ok(
            9,
            &[
                H(host_resource),
                I(0),
                I(host_data),
                I(host_bytes.len() as u64),
            ],
        );
        guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
        let produced = if pending_source { source } else { destination };
        guest.ok(42, &[H(lists[0]), H(produced), H(seed)]);
        if pending_source {
            guest.barrier(lists[0], source, COPY_DEST, COPY_SOURCE);
        } else {
            // Preserving the destination reads its pending prior version even
            // though its resource state remains COPY_DEST across both copies.
            guest.barrier(lists[1], source, COPY_DEST, COPY_SOURCE);
        }
        guest.ok(12, &[H(lists[0])]);
        // Original guest opcode: list, destination, destination offset,
        // source, source offset, count. Both lists close before either submit.
        guest.ok(
            43,
            &[H(lists[1]), H(destination), I(8), H(source), I(4), I(8)],
        );
        guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(a)], 0, Some(1));
        let dependency = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], dependency);
        guest.copy_result(value, dependency + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
        // No waits until both receipts exist; only two concurrent nodes.
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        guest.ok(26, &[H(seed)]);
        guest.ok(26, &[H(source)]);
        for receipt in if producer_first { [a, b] } else { [b, a] } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let output = guest.bytes(&[0xa5; 37]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(21)]);
        check_original_guest_backend(guest, true, true, &[(output, EXPECTED.to_vec())], 0, &[]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_image_texture_rectangle() {
    guest_queued_image_pair(0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_image_pitched_buffer_to_texture() {
    guest_queued_image_pair(1);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_image_texture_to_pitched_buffer() {
    guest_queued_image_pair(2);
}

fn guest_queued_image_pair(direction: u8) {
    const PIXELS: [u8; 16] = [
        1, 41, 81, 121, 2, 42, 82, 122, 3, 43, 83, 123, 4, 44, 84, 124,
    ];
    const PITCHED: [u8; 20] = [
        1, 41, 81, 121, 2, 42, 82, 122, 251, 252, 253, 254, 3, 43, 83, 123, 4, 44, 84, 124,
    ];
    const PRIOR: [u8; 16] = [
        101, 111, 121, 131, 102, 112, 122, 132, 103, 113, 123, 133, 104, 114, 124, 134,
    ];
    const BUFFER_PRIOR: [u8; 33] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
        119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133,
    ];
    const BUFFER_EXPECTED: [u8; 33] = [
        101, 102, 103, 104, 1, 41, 81, 121, 2, 42, 82, 122, 113, 114, 115, 116, 117, 118, 119, 120,
        121, 122, 123, 124, 3, 43, 83, 123, 4, 44, 84, 124, 133,
    ];
    const RECT_EXPECTED: [u8; 16] = [
        2, 42, 82, 122, 102, 112, 122, 132, 4, 44, 84, 124, 104, 114, 124, 134,
    ];
    let source_bytes = if direction == 1 {
        PITCHED.as_slice()
    } else {
        PIXELS.as_slice()
    };
    let destination_bytes = if direction == 2 {
        BUFFER_PRIOR.as_slice()
    } else {
        PRIOR.as_slice()
    };
    let expected = match direction {
        0 => RECT_EXPECTED.as_slice(),
        1 => PIXELS.as_slice(),
        _ => BUFFER_EXPECTED.as_slice(),
    };
    for pending_source in [false, true] {
        for producer_first in [false, true] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
            let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
            let lists = [
                guest.handle(4, &[H(device), H(allocators[0])]),
                guest.handle(4, &[H(device), H(allocators[1])]),
            ];
            let create = |guest: &mut GuestScript, buffer: bool, len: usize| {
                if buffer {
                    guest.handle(5, &[H(device), I(len as u64), I(COPY_DEST)])
                } else {
                    guest.handle(6, &[H(device), I(2), I(2), I(1), I(COPY_DEST)])
                }
            };
            let source = create(&mut guest, direction == 1, source_bytes.len());
            let destination = create(&mut guest, direction == 2, destination_bytes.len());
            let seed_bytes = if pending_source {
                source_bytes
            } else {
                destination_bytes
            };
            let seed_is_buffer = if pending_source {
                direction == 1
            } else {
                direction == 2
            };
            let seed = create(&mut guest, seed_is_buffer, seed_bytes.len());
            let data = guest.bytes(seed_bytes);
            guest.ok(9, &[H(seed), I(0), I(data), I(seed_bytes.len() as u64)]);
            let (host, bytes) = if pending_source {
                (destination, destination_bytes)
            } else {
                (source, source_bytes)
            };
            let data = guest.bytes(bytes);
            guest.ok(9, &[H(host), I(0), I(data), I(bytes.len() as u64)]);
            guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
            guest.ok(
                42,
                &[
                    H(lists[0]),
                    H(if pending_source { source } else { destination }),
                    H(seed),
                ],
            );
            guest.barrier(
                lists[usize::from(!pending_source)],
                source,
                COPY_DEST,
                COPY_SOURCE,
            );
            guest.ok(12, &[H(lists[0])]);
            let (source_kind, source_a, source_b) = match direction {
                0 => (0, 1, 0),
                1 => (1, 0, 12),
                _ => (0, 0, 0),
            };
            let (destination_kind, destination_a, destination_b) = if direction == 2 {
                (1, 4, 20)
            } else {
                (0, 0, 0)
            };
            guest.ok(
                44,
                &[
                    H(lists[1]),
                    H(destination),
                    I(destination_kind),
                    I(destination_a),
                    I(destination_b),
                    H(source),
                    I(source_kind),
                    I(source_a),
                    I(source_b),
                    I(if direction == 0 { 1 } else { 2 }),
                    I(2),
                ],
            );
            guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
            guest.ok(12, &[H(lists[1])]);
            let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
            let value = guest.call(36, &[H(a)], 0, Some(1));
            let dependency = guest.bytes(&[0; 16]);
            guest.copy_result(queues[0], dependency);
            guest.copy_result(value, dependency + 8);
            let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
            for index in 0..2 {
                guest.ok(11, &[H(lists[index]), H(allocators[index])]);
            }
            guest.ok(26, &[H(seed)]);
            guest.ok(26, &[H(source)]);
            for receipt in if producer_first { [a, b] } else { [b, a] } {
                guest.call(32, &[H(receipt)], 0, Some(1));
            }
            guest.ok(33, &[H(a)]);
            guest.ok(33, &[H(b)]);
            let output = guest.bytes(&vec![0xa5; expected.len() + 16]);
            guest.ok(
                10,
                &[
                    H(destination),
                    I(0),
                    I(output + 8),
                    I(expected.len() as u64),
                ],
            );
            check_original_guest_backend(guest, true, true, &[(output, expected.to_vec())], 0, &[]);
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_counted_uniform_loop_changes_trip_counts() {
    let cases = [
        (0u32, 0u8),
        (1, 64),
        (2, 128),
        (3, 191),
        (4, 255),
        (9, 255),
        (0xffff_ffff, 255),
        (0, 0),
    ];
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let target = guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-loop-vgpr-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for (frame, (count, gray)) in cases.into_iter().enumerate() {
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(3), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            let words = [count, 0, 0, 0, 0, 0, 0, 0];
            let address = guest.bytes(
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            guest.ok(41, &[H(list), I(address), I(1)]);
            guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            guest.ok(41, &[H(list), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == cases.len() - 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(26, &[H(vb)]);
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let output = guest.bytes(&[0xa5; 40]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
            readbacks.push((output, [gray, gray, gray, 255].repeat(6)));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 8, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_nested_uniform_loops_reinitialize_inner_each_outer_iteration() {
    // Literal n*m/16 UNORM oracles; original shader MIN bounds both inputs.
    let cases = [
        (0u32, 4u32, 0u8),
        (4, 0, 0),
        (1, 1, 16),
        (1, 4, 64),
        (2, 3, 96),
        (3, 3, 143),
        (4, 4, 255),
        (9, 2, 128),
        (2, 0xffff_ffff, 128),
        (0, 0, 0),
    ];
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let target = guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-nested-loop-vgpr-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for (frame, (outer, inner, gray)) in cases.into_iter().enumerate() {
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(3), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            let words = [outer, inner, 0, 0, 0, 0, 0, 0];
            let address = guest.bytes(
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            guest.ok(41, &[H(list), I(address), I(1)]);
            guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            guest.ok(41, &[H(list), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == cases.len() - 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(26, &[H(vb)]);
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let output = guest.bytes(&[0xa5; 40]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
            readbacks.push((output, [gray, gray, gray, 255].repeat(6)));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 10, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_unaligned_region_pending_source() {
    guest_queued_unaligned_buffer_regions(true);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_unaligned_region_pending_destination() {
    guest_queued_unaligned_buffer_regions(false);
}

fn guest_queued_unaligned_buffer_regions(pending_source: bool) {
    // Cross-word patch: source bytes 3..10 replace destination bytes 5..12.
    guest_queued_unaligned_buffer_region_case(
        pending_source,
        3,
        5,
        7,
        &[
            101, 102, 103, 104, 105, 37, 255, 2, 129, 4, 17, 83, 113, 114, 115, 116, 117, 118, 119,
            120, 121,
        ],
    );
    // One byte at each logical end: destination has a one-byte final word.
    guest_queued_unaligned_buffer_region_case(
        pending_source,
        15,
        20,
        1,
        &[
            101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117,
            118, 119, 120, 127,
        ],
    );
}

fn guest_queued_unaligned_buffer_region_case(
    pending_source: bool,
    source_offset: u64,
    destination_offset: u64,
    byte_len: u64,
    expected: &[u8],
) {
    const SOURCE: [u8; 16] = [
        91, 73, 55, 37, 255, 2, 129, 4, 17, 83, 149, 231, 13, 29, 61, 127,
    ];
    const PRIOR: [u8; 21] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118,
        119, 120, 121,
    ];
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
        let lists = [
            guest.handle(4, &[H(device), H(allocators[0])]),
            guest.handle(4, &[H(device), H(allocators[1])]),
        ];
        let source = guest.handle(5, &[H(device), I(16), I(COPY_DEST)]);
        let destination = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        let seed_bytes = if pending_source {
            SOURCE.as_slice()
        } else {
            PRIOR.as_slice()
        };
        let seed = guest.handle(5, &[H(device), I(seed_bytes.len() as u64), I(COPY_DEST)]);
        let seed_data = guest.bytes(seed_bytes);
        guest.ok(
            9,
            &[H(seed), I(0), I(seed_data), I(seed_bytes.len() as u64)],
        );
        let (host_resource, host_bytes) = if pending_source {
            (destination, PRIOR.as_slice())
        } else {
            (source, SOURCE.as_slice())
        };
        let host_data = guest.bytes(host_bytes);
        guest.ok(
            9,
            &[
                H(host_resource),
                I(0),
                I(host_data),
                I(host_bytes.len() as u64),
            ],
        );
        guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
        let produced = if pending_source { source } else { destination };
        guest.ok(42, &[H(lists[0]), H(produced), H(seed)]);
        if pending_source {
            guest.barrier(lists[0], source, COPY_DEST, COPY_SOURCE);
        } else {
            // Preserving the destination reads its pending prior version even
            // though its resource state remains COPY_DEST across both copies.
            guest.barrier(lists[1], source, COPY_DEST, COPY_SOURCE);
        }
        guest.ok(12, &[H(lists[0])]);
        // Original guest opcode: list, destination, destination offset,
        // source, source offset, count. Both lists close before either submit.
        guest.ok(
            43,
            &[
                H(lists[1]),
                H(destination),
                I(destination_offset),
                H(source),
                I(source_offset),
                I(byte_len),
            ],
        );
        guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let value = guest.call(36, &[H(a)], 0, Some(1));
        let dependency = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], dependency);
        guest.copy_result(value, dependency + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
        // No waits until both receipts exist; only two concurrent nodes.
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        guest.ok(26, &[H(seed)]);
        guest.ok(26, &[H(source)]);
        for receipt in if producer_first { [a, b] } else { [b, a] } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let output = guest.bytes(&[0xa5; 37]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(21)]);
        check_original_guest_backend(guest, true, true, &[(output, expected.to_vec())], 0, &[]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_graph_diamond_preserves_versions() {
    for reverse in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators: Vec<_> = (0..4).map(|_| guest.handle(3, &[H(device)])).collect();
        let lists: Vec<_> = allocators
            .iter()
            .map(|a| guest.handle(4, &[H(device), H(*a)]))
            .collect();
        let seed = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        let a = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        let b = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        let c = guest.handle(5, &[H(device), I(21), I(COPY_DEST)]);
        for (resource, bytes) in [
            (
                seed,
                [
                    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21,
                ],
            ),
            (
                b,
                [
                    101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
                    117, 118, 119, 120, 121,
                ],
            ),
            (
                c,
                [
                    201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216,
                    217, 218, 219, 220, 221,
                ],
            ),
        ] {
            let data = guest.bytes(&bytes);
            guest.ok(9, &[H(resource), I(0), I(data), I(21)]);
        }
        guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
        guest.ok(42, &[H(lists[0]), H(a), H(seed)]);
        guest.barrier(lists[0], a, COPY_DEST, COPY_SOURCE);
        guest.ok(43, &[H(lists[1]), H(b), I(8), H(a), I(0), I(4)]);
        guest.barrier(lists[1], b, COPY_DEST, COPY_SOURCE);
        guest.ok(43, &[H(lists[2]), H(c), I(0), H(a), I(17), I(4)]);
        guest.ok(43, &[H(lists[3]), H(c), I(8), H(b), I(8), I(4)]);
        guest.barrier(lists[3], c, COPY_DEST, COPY_SOURCE);
        for list in &lists {
            guest.ok(12, &[H(*list)]);
        }
        let ra = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let av = guest.call(36, &[H(ra)], 0, Some(1));
        let wa = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], wa);
        guest.copy_result(av, wa + 8);
        let rb = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wa), I(1)]);
        let rc = guest.handle(35, &[H(queues[0]), H(lists[2]), I(wa), I(1)]);
        let bv = guest.call(36, &[H(rb)], 0, Some(1));
        let cv = guest.call(36, &[H(rc)], 0, Some(2));
        let wd = guest.bytes(&[0; 48]);
        for (offset, queue, value) in if reverse {
            [(0, queues[0], cv), (16, queues[1], bv)]
        } else {
            [(0, queues[1], bv), (16, queues[0], cv)]
        } {
            guest.copy_result(queue, wd + offset);
            guest.copy_result(value, wd + offset + 8);
        }
        guest.copy_result(queues[0], wd + 32);
        guest.copy_result(av, wd + 40);
        let rd = guest.handle(35, &[H(queues[1]), H(lists[3]), I(wd), I(3)]);
        for i in 0..4 {
            guest.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        guest.ok(26, &[H(seed)]);
        guest.ok(26, &[H(a)]);
        guest.ok(26, &[H(b)]);
        for receipt in [rd, rc, rb, ra] {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        for receipt in [ra, rb, rc, rd] {
            guest.ok(33, &[H(receipt)]);
        }
        let output = guest.bytes(&[0xa5; 37]);
        guest.ok(10, &[H(c), I(0), I(output + 8), I(21)]);
        let expected = vec![
            18, 19, 20, 21, 205, 206, 207, 208, 1, 2, 3, 4, 213, 214, 215, 216, 217, 218, 219, 220,
            221,
        ];
        check_original_guest_backend(guest, true, true, &[(output, expected)], 0, &[]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_queued_graph_reads_transitive_ancestor() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
    let allocators: Vec<_> = (0..3).map(|_| guest.handle(3, &[H(device)])).collect();
    let lists: Vec<_> = allocators
        .iter()
        .map(|a| guest.handle(4, &[H(device), H(*a)]))
        .collect();
    let seed = guest.handle(5, &[H(device), I(5), I(COPY_DEST)]);
    let resources: Vec<_> = (0..3)
        .map(|_| guest.handle(5, &[H(device), I(5), I(COPY_DEST)]))
        .collect();
    let data = guest.bytes(&[17, 53, 127, 193, 251]);
    guest.ok(9, &[H(seed), I(0), I(data), I(5)]);
    guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
    guest.ok(42, &[H(lists[0]), H(resources[0]), H(seed)]);
    guest.barrier(lists[0], resources[0], COPY_DEST, COPY_SOURCE);
    guest.ok(42, &[H(lists[1]), H(resources[1]), H(resources[0])]);
    guest.ok(42, &[H(lists[2]), H(resources[2]), H(resources[0])]);
    guest.barrier(lists[2], resources[2], COPY_DEST, COPY_SOURCE);
    for list in &lists {
        guest.ok(12, &[H(*list)]);
    }
    let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
    let ap = guest.call(36, &[H(a)], 0, Some(1));
    let wa = guest.bytes(&[0; 16]);
    guest.copy_result(queues[0], wa);
    guest.copy_result(ap, wa + 8);
    let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wa), I(1)]);
    let bp = guest.call(36, &[H(b)], 0, Some(1));
    let wb = guest.bytes(&[0; 16]);
    guest.copy_result(queues[1], wb);
    guest.copy_result(bp, wb + 8);
    // Both explicit and implicit edges name B only; A must be found transitively.
    let c = guest.handle(35, &[H(queues[1]), H(lists[2]), I(wb), I(1)]);
    for i in 0..3 {
        guest.ok(11, &[H(lists[i]), H(allocators[i])]);
    }
    for resource in [seed, resources[0], resources[1]] {
        guest.ok(26, &[H(resource)]);
    }
    for receipt in [c, b, a] {
        guest.call(32, &[H(receipt)], 0, Some(1));
    }
    for receipt in [a, b, c] {
        guest.ok(33, &[H(receipt)]);
    }
    let output = guest.bytes(&[0xa5; 21]);
    guest.ok(10, &[H(resources[2]), I(0), I(output + 8), I(5)]);
    check_original_guest_backend(
        guest,
        true,
        true,
        &[(output, vec![17, 53, 127, 193, 251])],
        0,
        &[],
    );
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_render_graph_loads_transitive_color_and_depth() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
    let allocators = [0, 1, 2].map(|_| guest.handle(3, &[H(device)]));
    let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
    let target = guest.handle(6, &[H(device), I(4), I(2), I(1), I(RENDER_TARGET)]);
    let other = guest.handle(6, &[H(device), I(4), I(2), I(1), I(RENDER_TARGET)]);
    let depth = guest.handle(7, &[H(device), I(4), I(2)]);
    let vb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
    let positions = guest.bytes(&floats(&[
        [-1., -1., 0.5, 1.],
        [3., -1., 0.5, 1.],
        [-1., 3., 0.5, 1.],
        [-1., -1., 0.125, 1.],
        [3., -1., 0.125, 1.],
        [-1., 3., 0.125, 1.],
    ]));
    guest.ok(9, &[H(vb), I(0), I(positions), I(96)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let pipeline = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );

    // A produces distinctive color and depth; neither exists in committed host storage yet.
    let prior = [29u8, 73, 151, 199];
    let mut record = [0; 16];
    record[8..12].copy_from_slice(&prior);
    let clear = guest.bytes(&record);
    guest.copy_result(target, clear);
    guest.ok(15, &[H(lists[0]), I(1), I(clear)]);
    guest.ok(
        16,
        &[H(lists[0]), H(depth), I(0), I(0.25f32.to_bits() as u64)],
    );
    guest.barrier(lists[0], vb, COPY_DEST, VERTEX_BUFFER);
    guest.ok(12, &[H(lists[0])]);
    let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
    let ap = guest.call(36, &[H(a)], 0, Some(1));
    let wa = guest.bytes(&[0; 16]);
    guest.copy_result(queues[0], wa);
    guest.copy_result(ap, wa + 8);

    // B defines a depth state without binding depth or replacing A's output version.
    guest.targets(lists[1], &[other]);
    guest.barrier(lists[1], depth, 0x10, 0x20);
    guest.barrier(lists[1], other, RENDER_TARGET, COPY_SOURCE);
    guest.ok(12, &[H(lists[1])]);
    let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wa), I(1)]);
    let bp = guest.call(36, &[H(b)], 0, Some(1));
    let wb = guest.bytes(&[0; 16]);
    guest.copy_result(queues[1], wb);
    guest.copy_result(bp, wb + 8);

    // C has only B as its explicit and implicit parent. Both loaded versions are A's.
    guest.load_targets(lists[2], &[target]);
    guest.ok(16, &[H(lists[2]), H(depth), I(1), I(0)]);
    guest.ok(17, &[H(lists[2]), I(4), I(2)]);
    guest.ok(14, &[H(lists[2]), H(pipeline)]);
    guest.ok(18, &[H(lists[2]), H(vb), I(0), I(16)]);
    // Less, no depth writes: .5 fails against .25 everywhere.
    guest.ok(
        22,
        &[
            H(lists[2]),
            I(2),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
        ],
    );
    guest.ok(23, &[H(lists[2]), I(3), I(1), I(0), I(0)]);
    // .125 passes, but only in the first column. A zero/uninitialized depth rejects it.
    guest.ok(
        22,
        &[
            H(lists[2]),
            I(2),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(0),
            I(0),
            I(1),
            I(2),
        ],
    );
    guest.ok(23, &[H(lists[2]), I(3), I(1), I(3), I(0)]);
    guest.barrier(lists[2], target, RENDER_TARGET, COPY_SOURCE);
    guest.ok(12, &[H(lists[2])]);
    let c = guest.handle(35, &[H(queues[1]), H(lists[2]), I(wb), I(1)]);
    // All three render receipts remain outstanding until descendant-first collection.
    for receipt in [c, b, a] {
        guest.call(32, &[H(receipt)], 0, Some(1));
    }
    for receipt in [a, b, c] {
        guest.ok(33, &[H(receipt)]);
    }
    let output = guest.bytes(&[0xa5; 48]);
    let other_output = guest.bytes(&[0xa5; 48]);
    guest.ok(10, &[H(target), I(0), I(output + 8), I(32)]);
    guest.ok(10, &[H(other), I(0), I(other_output + 8), I(32)]);
    let expected = vec![
        255, 0, 0, 255, 29, 73, 151, 199, 29, 73, 151, 199, 29, 73, 151, 199, 255, 0, 0, 255, 29,
        73, 151, 199, 29, 73, 151, 199, 29, 73, 151, 199,
    ];
    check_original_guest_backend(
        guest,
        true,
        true,
        &[
            (output, expected.clone()),
            (other_output, [17, 31, 47, 255].repeat(8)),
        ],
        3,
        &expected,
    );
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_conditional_loop_break_uses_current_uniforms() {
    // Literal accumulated eighths; stop matches the live counter before update.
    let cases = [
        (0u32, 2u32, 0u8),
        (4, 4, 0),
        (4, 3, 32),
        (4, 2, 64),
        (4, 1, 96),
        (4, 0, 128),
        (1, 1, 0),
        (1, 0, 32),
        (0xffff_ffff, 0xffff_ffff, 128),
        (2, 3, 64),
        (0, 0, 0),
        (9, 2, 64),
    ];
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let target = guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let positions = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-loop-break-before-ps.bin");
        let va = guest.bytes(vs);
        let pa = guest.bytes(ps);
        let pipeline = guest.handle(
            8,
            &[
                H(device),
                I(va),
                I(vs.len() as u64),
                I(pa),
                I(ps.len() as u64),
            ],
        );
        let mut readbacks = Vec::new();
        for (frame, (outer, inner, gray)) in cases.into_iter().enumerate() {
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(3), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            let words = [outer, inner, 0, 0, 0, 0, 0, 0];
            let address = guest.bytes(
                &words
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            guest.ok(41, &[H(list), I(address), I(1)]);
            guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            guest.ok(41, &[H(list), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == cases.len() - 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(26, &[H(vb)]);
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                guest.ok(11, &[H(list), H(allocator)]);
            }
            let output = guest.bytes(&[0xa5; 40]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
            readbacks.push((output, [gray, gray, gray, 255].repeat(6)));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 12, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_signed_countdowns_preserve_negative_bits_and_exact_256() {
    // The final four rows deliberately disagree with the independently supplied
    // count, final-bit, or SCC expectations. Both fixtures run through each real adapter.
    let cases = [
        (
            [0xffffffffu32, 0, 0xffffffff, 0, 0, 0, 0, 0],
            [255u8, 255, 255, 255],
        ),
        (
            [0x80000000, 0, 0x80000000, 0, 0, 0, 0, 0],
            [255, 255, 255, 255],
        ),
        ([0, 0, 0, 0, 0, 0, 0, 0], [255, 255, 255, 255]),
        ([1, 1, 0, 0, 0, 0, 0, 0], [255, 255, 255, 255]),
        ([256, 256, 0, 0, 0, 0, 0, 0], [255, 255, 255, 255]),
        ([0x7fffffff, 256, 0, 0, 0, 0, 0, 0], [255, 255, 255, 255]),
        ([256, 255, 0, 0, 0, 0, 0, 0], [0, 255, 255, 255]),
        ([0xffffffff, 0, 0, 0, 0, 0, 0, 0], [255, 0, 255, 255]),
        ([1, 1, 0, 1, 0, 0, 0, 0], [255, 255, 0, 255]),
        ([1, 1, 0, 0, 1, 0, 0, 0], [255, 255, 0, 255]),
    ];
    for ps in [
        include_bytes!("../../../fixtures/rdna2/x9-loop-signed-count-ps.bin").as_slice(),
        include_bytes!("../../../fixtures/rdna2/x9-loop-signed-reversed-ps.bin").as_slice(),
    ] {
        for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queue = guest.handle(2, &[H(device)]);
            let allocator = guest.handle(3, &[H(device)]);
            let list = guest.handle(4, &[H(device), H(allocator)]);
            let target = guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)]);
            let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
            let positions = guest.bytes(&floats(&[
                [-1., -1., 0., 1.],
                [3., -1., 0., 1.],
                [-1., 3., 0., 1.],
            ]));
            guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
            let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
            let va = guest.bytes(vs);
            let pa = guest.bytes(ps);
            let pipeline = guest.handle(
                8,
                &[
                    H(device),
                    I(va),
                    I(vs.len() as u64),
                    I(pa),
                    I(ps.len() as u64),
                ],
            );
            let mut readbacks = Vec::new();
            for (frame, (words, pixel)) in cases.into_iter().enumerate() {
                if frame == 0 {
                    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                } else {
                    guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
                }
                guest.targets(list, &[target]);
                guest.ok(17, &[H(list), I(3), I(2)]);
                guest.ok(14, &[H(list), H(pipeline)]);
                guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
                let address = guest.bytes(
                    &words
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
                guest.ok(41, &[H(list), I(address), I(1)]);
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
                guest.ok(41, &[H(list), I(0), I(0)]);
                guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
                if asynchronous && frame == cases.len() - 1 {
                    guest.ok(12, &[H(list)]);
                    let receipt = guest.handle(30, &[H(queue), H(list)]);
                    guest.ok(11, &[H(list), H(allocator)]);
                    guest.ok(26, &[H(vb)]);
                    guest.ok(27, &[H(pipeline)]);
                    guest.call(32, &[H(receipt)], 0, Some(1));
                    guest.ok(33, &[H(receipt)]);
                } else {
                    finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                    guest.ok(11, &[H(list), H(allocator)]);
                }
                let output = guest.bytes(&[0xa5; 40]);
                guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
                readbacks.push((output, pixel.repeat(6)));
            }
            let final_pixels = readbacks.last().unwrap().1.clone();
            check_original_guest_backend(
                guest,
                asynchronous,
                queued,
                &readbacks,
                10,
                &final_pixels,
            );
        }
    }
}

fn guest_geometry_mrt_pipeline(guest: &mut GuestScript, device: u64) -> u64 {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-scene-varying-mrt-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    )
}
fn guest_geometry_wait_record(guest: &mut GuestScript, queue: u64, receipt: u64) -> u64 {
    let value = guest.call(36, &[H(receipt)], 0, None);
    let wait = guest.bytes(&[0; 16]);
    guest.copy_result(queue, wait);
    guest.copy_result(value, wait + 8);
    wait
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_produced_geometry_positions_colors_indices_and_retirement() {
    for role in 0..3 {
        for format in [57u64, 42] {
            for descendant_first in [false, true] {
                let mut guest = GuestScript::new();
                let device = guest.handle(1, &[]);
                let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
                let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
                let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
                let pipeline = guest_geometry_mrt_pipeline(&mut guest, device);
                let p = padded_float4_records(
                    3,
                    23,
                    &[
                        [-1., -1., 0.25, 1.],
                        [3., -1., 0.25, 1.],
                        [-1., 3., 0.25, 1.],
                    ],
                );
                let c = padded_float4_records(5, 21, &[[0., 1., 0., 1.]; 3]);
                let width = if format == 57 { 2 } else { 4 };
                let mut indices = vec![0xa5; width];
                for value in [65535u32, 65535, 65535, 3, 1, 2] {
                    indices.extend_from_slice(&value.to_le_bytes()[..width]);
                }
                let bytes = [p, c, indices];
                let resources = bytes
                    .each_ref()
                    .map(|b| guest.handle(5, &[H(device), I(b.len() as u64), I(COPY_DEST)]));
                for (i, b) in bytes.iter().enumerate() {
                    let data = if i == role {
                        guest.bytes(&vec![0; b.len()])
                    } else {
                        guest.bytes(b)
                    };
                    guest.ok(9, &[H(resources[i]), I(0), I(data), I(b.len() as u64)]);
                }
                let seed = guest.handle(5, &[H(device), I(bytes[role].len() as u64), I(COPY_DEST)]);
                let data = guest.bytes(&bytes[role]);
                guest.ok(9, &[H(seed), I(0), I(data), I(bytes[role].len() as u64)]);
                let targets = [
                    guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]),
                    guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]),
                ];
                guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
                guest.ok(42, &[H(lists[0]), H(resources[role]), H(seed)]);
                guest.barrier(
                    lists[0],
                    resources[role],
                    COPY_DEST,
                    if role == 2 { 2 } else { VERTEX_BUFFER },
                );
                guest.ok(12, &[H(lists[0])]);
                let producer = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
                let wait = guest_geometry_wait_record(&mut guest, queues[0], producer);
                for (i, r) in resources.iter().enumerate() {
                    if i != role {
                        guest.barrier(
                            lists[1],
                            *r,
                            COPY_DEST,
                            if i == 2 { 2 } else { VERTEX_BUFFER },
                        );
                    }
                }
                guest.targets(lists[1], &targets);
                guest.ok(17, &[H(lists[1]), I(4), I(3)]);
                guest.ok(14, &[H(lists[1]), H(pipeline)]);
                guest.ok(18, &[H(lists[1]), H(resources[0]), I(3), I(23)]);
                guest.ok(19, &[H(lists[1]), H(resources[1]), I(5), I(21)]);
                guest.ok(
                    20,
                    &[
                        H(lists[1]),
                        H(resources[2]),
                        I((3 * width) as u64),
                        I(format),
                    ],
                );
                guest.ok(24, &[H(lists[1]), I(3), I(1), I(1), I(u64::MAX), I(0)]);
                for target in targets {
                    guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
                }
                guest.ok(12, &[H(lists[1])]);
                let consumer = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
                for i in 0..2 {
                    guest.ok(11, &[H(lists[i]), H(allocators[i])]);
                }
                for resource in [seed, resources[0], resources[1], resources[2]] {
                    guest.ok(26, &[H(resource)]);
                }
                guest.ok(27, &[H(pipeline)]);
                for receipt in if descendant_first {
                    [consumer, producer]
                } else {
                    [producer, consumer]
                } {
                    guest.call(32, &[H(receipt)], 0, Some(1));
                }
                for receipt in [producer, consumer] {
                    guest.ok(33, &[H(receipt)]);
                }
                let a = guest.bytes(&[0xa5; 64]);
                let b = guest.bytes(&[0xa5; 64]);
                guest.ok(10, &[H(targets[0]), I(0), I(a + 8), I(48)]);
                guest.ok(10, &[H(targets[1]), I(0), I(b + 8), I(48)]);
                let expected = [0, 255, 0, 255].repeat(12);
                check_original_guest_backend(
                    guest,
                    true,
                    true,
                    &[(a, expected.clone()), (b, [0, 0, 255, 255].repeat(12))],
                    1,
                    &expected,
                );
            }
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_produced_geometry_failure_preserves_mrt_depth_and_recovery() {
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
    let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
    let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
    let pipeline = guest_geometry_mrt_pipeline(&mut guest, device);
    let targets = [
        guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]),
        guest.handle(6, &[H(device), I(4), I(3), I(1), I(RENDER_TARGET)]),
    ];
    let depth = guest.handle(7, &[H(device), I(4), I(3)]);
    let positions = floats(&[
        [-1., -1., 0.25, 1.],
        [3., -1., 0.25, 1.],
        [-1., 3., 0.25, 1.],
        [-1., -1., 0.5, 1.],
        [3., -1., 0.5, 1.],
        [-1., 3., 0.5, 1.],
        [-1., -1., 0.125, 1.],
        [3., -1., 0.125, 1.],
        [-1., 3., 0.125, 1.],
    ]);
    let red = floats(&[[1., 0., 0., 1.]; 9]);
    let blue = floats(&[[0., 0., 1., 1.]; 9]);
    let resources = [&positions, &red, &blue].map(|bytes| {
        let resource = guest.handle(5, &[H(device), I(bytes.len() as u64), I(COPY_DEST)]);
        let address = guest.bytes(bytes);
        guest.ok(9, &[H(resource), I(0), I(address), I(bytes.len() as u64)]);
        resource
    });
    let seed = guest.handle(5, &[H(device), I(12), I(COPY_DEST)]);
    let ib = guest.handle(5, &[H(device), I(12), I(COPY_DEST)]);
    let bad = guest.bytes(
        &[0u32, 1, 999]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    guest.ok(9, &[H(seed), I(0), I(bad), I(12)]);
    let mut readbacks = Vec::new();
    // Baseline .25 writes red/green and retained depth .25.
    for resource in resources {
        guest.barrier(lists[1], resource, COPY_DEST, VERTEX_BUFFER);
    }
    guest.targets(lists[1], &targets);
    guest.ok(16, &[H(lists[1]), H(depth), I(0), I(1f32.to_bits() as u64)]);
    guest.ok(17, &[H(lists[1]), I(4), I(3)]);
    guest.ok(14, &[H(lists[1]), H(pipeline)]);
    guest.ok(18, &[H(lists[1]), H(resources[0]), I(0), I(16)]);
    guest.ok(19, &[H(lists[1]), H(resources[1]), I(0), I(16)]);
    guest.ok(
        22,
        &[
            H(lists[1]),
            I(2),
            I(1),
            I(0),
            I(15),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
        ],
    );
    guest.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
    for target in targets {
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
    }
    finish_guest_submission(&mut guest, queues[1], lists[1], true, 1);
    guest.ok(11, &[H(lists[1]), H(allocators[1])]);
    // A copy supplies an invalid final index; submit both before waiting.
    guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
    guest.ok(42, &[H(lists[0]), H(ib), H(seed)]);
    guest.barrier(lists[0], ib, COPY_DEST, 2);
    guest.ok(12, &[H(lists[0])]);
    let producer = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
    let wait = guest_geometry_wait_record(&mut guest, queues[0], producer);
    for target in targets {
        guest.barrier(lists[1], target, COPY_SOURCE, RENDER_TARGET);
    }
    guest.load_targets(lists[1], &targets);
    guest.ok(16, &[H(lists[1]), H(depth), I(0), I(1f32.to_bits() as u64)]);
    guest.ok(17, &[H(lists[1]), I(4), I(3)]);
    guest.ok(14, &[H(lists[1]), H(pipeline)]);
    guest.ok(18, &[H(lists[1]), H(resources[0]), I(0), I(16)]);
    guest.ok(19, &[H(lists[1]), H(resources[2]), I(0), I(16)]);
    guest.ok(
        22,
        &[
            H(lists[1]),
            I(2),
            I(1),
            I(0),
            I(15),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
        ],
    );
    guest.ok(23, &[H(lists[1]), I(3), I(1), I(3), I(0)]);
    guest.ok(20, &[H(lists[1]), H(ib), I(0), I(42)]);
    guest.ok(24, &[H(lists[1]), I(3), I(1), I(0), I(3), I(0)]);
    for target in targets {
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
    }
    guest.ok(12, &[H(lists[1])]);
    let failed = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
    guest.call(32, &[H(producer)], 0, Some(1));
    guest.call(32, &[H(failed)], 0, Some(2));
    for receipt in [producer, failed] {
        guest.ok(33, &[H(receipt)]);
    }
    guest.ok(11, &[H(lists[0]), H(allocators[0])]);
    guest.ok(11, &[H(lists[1]), H(allocators[1])]);
    for (target, color) in [
        (targets[0], [255, 0, 0, 255]),
        (targets[1], [0, 255, 0, 255]),
    ] {
        let out = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(target), I(0), I(out + 8), I(48)]);
        readbacks.push((out, color.repeat(12)));
    }
    // .5 must still fail against .25. Then .125 passes in the first column;
    // this detects a failed clear to1 or a silently zeroed depth snapshot.
    for target in targets {
        guest.barrier(lists[1], target, COPY_SOURCE, RENDER_TARGET);
    }
    guest.load_targets(lists[1], &targets);
    guest.ok(16, &[H(lists[1]), H(depth), I(1), I(0)]);
    guest.ok(17, &[H(lists[1]), I(4), I(3)]);
    guest.ok(14, &[H(lists[1]), H(pipeline)]);
    guest.ok(18, &[H(lists[1]), H(resources[0]), I(0), I(16)]);
    guest.ok(19, &[H(lists[1]), H(resources[2]), I(0), I(16)]);
    guest.ok(
        22,
        &[
            H(lists[1]),
            I(2),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
            I(0),
        ],
    );
    guest.ok(23, &[H(lists[1]), I(3), I(1), I(3), I(0)]);
    guest.ok(
        22,
        &[
            H(lists[1]),
            I(2),
            I(1),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(0),
            I(0),
            I(1),
            I(3),
        ],
    );
    guest.ok(23, &[H(lists[1]), I(3), I(1), I(6), I(0)]);
    for target in targets {
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
    }
    finish_guest_submission(&mut guest, queues[1], lists[1], true, 2);
    let a = [
        0, 0, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
    ]
    .repeat(3);
    let b = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
    ]
    .repeat(3);
    for (target, pixels) in [(targets[0], a.clone()), (targets[1], b)] {
        let out = guest.bytes(&[0xa5; 64]);
        guest.ok(10, &[H(target), I(0), I(out + 8), I(48)]);
        readbacks.push((out, pixels));
    }
    check_original_guest_backend(guest, true, true, &readbacks, 2, &a);
}

fn guest_nested_view_pipeline(guest: &mut GuestScript, device: u64) -> u64 {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    )
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_nested_views_compose_nonzero_bases_and_srgb() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let texture = guest.handle(6, &[H(device), I(8), I(8), I(4), I(COPY_DEST)]);
        // Opcode45 order matches37: device,parent,relative base,count,format.
        // An sRGB parent does not silently override its linear child's format.
        let parent = guest.handle(37, &[H(device), H(texture), I(1), I(3), I(1)]);
        let linear = guest.handle(45, &[H(device), H(parent), I(1), I(2), I(0)]);
        let srgb = guest.handle(45, &[H(device), H(parent), I(1), I(2), I(1)]);
        let last = guest.handle(45, &[H(device), H(linear), I(1), I(1), I(0)]);
        let last_srgb = guest.handle(45, &[H(device), H(srgb), I(1), I(1), I(1)]);
        guest.ok(38, &[H(parent)]);
        guest.call(26, &[H(texture)], 2, Some(u64::MAX));
        let target = guest.handle(6, &[H(device), I(7), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let p = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let uv = guest.bytes(&floats(&[
            [0.25, 0.25, 0., 1.],
            [0.25, 0.25, 0., 1.],
            [0.25, 0.25, 0., 1.],
            [0.75, 0.75, 0., 1.],
            [0.75, 0.75, 0., 1.],
            [0.75, 0.75, 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(uv), I(96)]);
        let pipeline = guest_nested_view_pipeline(&mut guest, device);
        let mut readbacks = Vec::new();
        for frame in 0..2 {
            let (top, decoded, bottom, tail, tail_decoded) = if frame == 0 {
                (
                    [128u8, 64, 192, 128],
                    [55, 13, 134, 128],
                    [16, 32, 48, 96],
                    [64, 128, 32, 192],
                    [13, 55, 4, 192],
                )
            } else {
                (
                    [64, 128, 32, 192],
                    [13, 55, 4, 192],
                    [80, 160, 240, 32],
                    [128, 64, 192, 128],
                    [55, 13, 134, 128],
                )
            };
            // Root mips0/1 are decoys. Child0 is root2 (2x2), child1=root3.
            let mut bytes = [201u8, 9, 21, 255].repeat(64);
            bytes.extend([5u8, 203, 17, 255].repeat(16));
            bytes.extend([top, [8, 24, 40, 56], [72, 88, 104, 120], bottom].as_flattened());
            bytes.extend(tail);
            let upload = guest.bytes(&bytes);
            guest.ok(9, &[H(texture), I(0), I(upload), I(340)]);
            guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(7), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            for (column, (view, mip)) in [
                (linear, 0),
                (srgb, 0),
                (linear, 1),
                (last, 0),
                (linear, 0),
                (last_srgb, 0),
                (srgb, 1),
            ]
            .into_iter()
            .enumerate()
            {
                guest.ok(
                    19,
                    &[H(list), H(cb), I(if column == 4 { 48 } else { 0 }), I(16)],
                );
                let table = sampled_table(&mut guest, [Some(view), None], [Some(mip), None], false);
                guest.ok(39, &[H(list), I(table)]);
                guest.ok(
                    22,
                    &[
                        H(list),
                        I(0),
                        I(0),
                        I(0),
                        I(15),
                        I(0),
                        I(0),
                        I(1),
                        I(column as u64),
                        I(0),
                        I(1),
                        I(2),
                    ],
                );
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            }
            guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                for view in [linear, srgb, last, last_srgb] {
                    guest.ok(38, &[H(view)]);
                }
                for resource in [texture, vb, cb] {
                    guest.ok(26, &[H(resource)]);
                }
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 7);
                guest.ok(11, &[H(list), H(allocator)]);
                if frame == 1 {
                    for view in [linear, srgb, last, last_srgb] {
                        guest.ok(38, &[H(view)]);
                    }
                    guest.ok(26, &[H(texture)]);
                }
            }
            let expected = [top, decoded, tail, tail, bottom, tail_decoded, tail_decoded]
                .as_flattened()
                .repeat(2);
            let output = guest.bytes(&[0xa5; 72]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(56)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 2, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_nested_views_parent_bounds_and_independent_root_lifetime() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let texture = guest.handle(6, &[H(device), I(8), I(8), I(4), I(COPY_DEST)]);
        let parent = guest.handle(37, &[H(device), H(texture), I(1), I(2), I(0)]);
        // Both ranges fit the root after base composition, but not the parent.
        for (base, count, format) in [
            (2, 1, 0),
            (1, 2, 0),
            (0, 0, 0),
            (u64::from(u32::MAX), 1, 0),
            (0, 1, 2),
        ] {
            guest.call(
                45,
                &[H(device), H(parent), I(base), I(count), I(format)],
                2,
                Some(u64::MAX),
            );
        }
        let child = guest.handle(45, &[H(device), H(parent), I(1), I(1), I(0)]);
        let grandchild = guest.handle(45, &[H(device), H(child), I(0), I(1), I(1)]);
        guest.ok(38, &[H(parent)]);
        guest.call(26, &[H(texture)], 2, Some(u64::MAX));
        guest.call(
            45,
            &[H(device), H(parent), I(0), I(1), I(0)],
            2,
            Some(u64::MAX),
        );
        guest.ok(38, &[H(child)]);
        // Only the grandchild remains: it independently protects the root.
        guest.call(26, &[H(texture)], 2, Some(u64::MAX));
        let mut bytes = [9u8, 201, 33, 255].repeat(64);
        bytes.extend([17u8, 29, 211, 255].repeat(16));
        bytes.extend([128u8, 64, 192, 128].repeat(4));
        bytes.extend([3u8, 7, 11, 255]);
        let upload = guest.bytes(&bytes);
        guest.ok(9, &[H(texture), I(0), I(upload), I(340)]);
        let target = guest.handle(6, &[H(device), I(3), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let p = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let uv = guest.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
        guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(uv), I(48)]);
        let pipeline = guest_nested_view_pipeline(&mut guest, device);
        guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
        guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
        guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
        guest.targets(list, &[target]);
        guest.ok(17, &[H(list), I(3), I(2)]);
        guest.ok(14, &[H(list), H(pipeline)]);
        guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
        guest.ok(19, &[H(list), H(cb), I(0), I(16)]);
        let table = sampled_table(&mut guest, [Some(grandchild), None], [Some(0), None], false);
        guest.ok(39, &[H(list), I(table)]);
        guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
        guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
        guest.call(38, &[H(grandchild)], 2, Some(u64::MAX));
        if asynchronous {
            guest.ok(12, &[H(list)]);
            let receipt = guest.handle(30, &[H(queue), H(list)]);
            guest.ok(11, &[H(list), H(allocator)]);
            guest.ok(38, &[H(grandchild)]);
            guest.ok(26, &[H(texture)]);
            guest.ok(26, &[H(vb)]);
            guest.ok(26, &[H(cb)]);
            guest.ok(27, &[H(pipeline)]);
            guest.call(32, &[H(receipt)], 0, Some(1));
            guest.ok(33, &[H(receipt)]);
        } else {
            finish_guest_submission(&mut guest, queue, list, false, 1);
            guest.ok(11, &[H(list), H(allocator)]);
            guest.ok(38, &[H(grandchild)]);
            guest.ok(26, &[H(texture)]);
        }
        let expected = [55, 13, 134, 128].repeat(6);
        let output = guest.bytes(&[0xa5; 40]);
        guest.ok(10, &[H(target), I(0), I(output + 8), I(24)]);
        check_original_guest_backend(
            guest,
            asynchronous,
            queued,
            &[(output, expected.clone())],
            1,
            &expected,
        );
    }
}

fn guest_sampler_handle_table(
    guest: &mut GuestScript,
    views: [Option<u64>; 2],
    samplers: [Option<u64>; 2],
) -> u64 {
    let table = guest.bytes(&[0; 32]);
    for (slot, handle) in views.into_iter().chain(samplers).enumerate() {
        if let Some(handle) = handle {
            guest.copy_result(handle, table + slot as u64 * 8);
        }
    }
    table
}

struct GuestSamplerScene {
    device: u64,
    queue: u64,
    allocators: [u64; 2],
    lists: [u64; 2],
    views: [u64; 2],
    narrow: u64,
    targets: [u64; 2],
    pipeline: u64,
    vb: u64,
    cb: u64,
}
fn guest_sampler_scene(guest: &mut GuestScript) -> GuestSamplerScene {
    let device = guest.handle(1, &[]);
    let queue = guest.handle(2, &[H(device)]);
    let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
    let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
    let textures = [0, 1].map(|_| guest.handle(6, &[H(device), I(4), I(4), I(3), I(COPY_DEST)]));
    let views = textures.map(|t| guest.handle(37, &[H(device), H(t), I(1), I(2), I(0)]));
    let narrow = guest.handle(37, &[H(device), H(textures[1]), I(1), I(1), I(0)]);
    // Original all-four-pairs PS: A/s1.r, B/s0.g, A/s0.b, B/s1.a.
    for (texture, mip0, mip1) in [
        (textures[0], [18u8, 31, 47, 255], [72, 83, 97, 255]),
        (textures[1], [110u8, 127, 139, 211], [152, 167, 181, 199]),
    ] {
        let mut bytes = [3u8, 5, 7, 11].repeat(16); // root mip0 is a decoy
        bytes.extend(mip0.repeat(4));
        bytes.extend(mip1);
        let address = guest.bytes(&bytes);
        guest.ok(9, &[H(texture), I(0), I(address), I(84)]);
        guest.barrier(lists[0], texture, COPY_DEST, PIXEL_RESOURCE);
    }
    let targets = [0, 1].map(|_| guest.handle(6, &[H(device), I(2), I(2), I(1), I(RENDER_TARGET)]));
    let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let positions = guest.bytes(&floats(&[
        [-1., -1., 0., 1.],
        [3., -1., 0., 1.],
        [-1., 3., 0., 1.],
    ]));
    let attributes = guest.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
    guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
    guest.ok(9, &[H(cb), I(0), I(attributes), I(48)]);
    guest.barrier(lists[0], vb, COPY_DEST, VERTEX_BUFFER);
    guest.barrier(lists[0], cb, COPY_DEST, VERTEX_BUFFER);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-multislot-ps.bin");
    let va = guest.bytes(vs);
    let pa = guest.bytes(ps);
    let pipeline = guest.handle(
        8,
        &[
            H(device),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
        ],
    );
    GuestSamplerScene {
        device,
        queue,
        allocators,
        lists,
        views,
        narrow,
        targets,
        pipeline,
        vb,
        cb,
    }
}
fn guest_sampler_draw_setup(
    guest: &mut GuestScript,
    scene: &GuestSamplerScene,
    list: u64,
    target: u64,
) {
    guest.targets(list, &[target]);
    guest.ok(17, &[H(list), I(2), I(2)]);
    guest.ok(14, &[H(list), H(scene.pipeline)]);
    guest.ok(18, &[H(list), H(scene.vb), I(0), I(16)]);
    guest.ok(19, &[H(list), H(scene.cb), I(0), I(16)]);
}
fn guest_sampler_column(guest: &mut GuestScript, list: u64, column: u64) {
    guest.ok(
        22,
        &[
            H(list),
            I(0),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(column),
            I(0),
            I(1),
            I(2),
        ],
    );
    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sampler_handles_snapshot_across_draws_lists_and_retirement() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let scene = guest_sampler_scene(&mut guest);
        let zero = guest.handle(46, &[H(scene.device), I(0), I(0), I(1f32.to_bits() as u64)]);
        let one = guest.handle(46, &[H(scene.device), I(1), I(0), I(1f32.to_bits() as u64)]);
        // No explicit mip: clamp sample_lz to relative mip1 through min/maxLOD.
        let clamped = guest.handle(
            46,
            &[
                H(scene.device),
                I(u32::MAX as u64),
                I(1f32.to_bits() as u64),
                I(1f32.to_bits() as u64),
            ],
        );
        for index in 0..2 {
            let list = scene.lists[index];
            guest_sampler_draw_setup(&mut guest, &scene, list, scene.targets[index]);
            for column in 0..2 {
                let pair = match (index, column) {
                    (0, 0) | (1, 1) => [Some(zero), Some(one)],
                    (0, 1) => [Some(one), Some(zero)],
                    _ => [Some(clamped), Some(zero)],
                };
                let table = guest_sampler_handle_table(&mut guest, scene.views.map(Some), pair);
                guest.ok(48, &[H(list), I(table)]);
                guest_sampler_column(&mut guest, list, column);
            }
            guest.barrier(list, scene.targets[index], RENDER_TARGET, COPY_SOURCE);
        }
        // Recorded commands own values, not these handles. Both lists already
        // contain zero/clamped snapshots when their handles disappear.
        guest.ok(47, &[H(zero)]);
        guest.ok(47, &[H(clamped)]);
        let mut readbacks = Vec::new();
        for index in 0..2 {
            let list = scene.lists[index];
            if asynchronous && index == 0 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(scene.queue), H(list)]);
                guest.ok(47, &[H(one)]); // destruction after submit is permitted
                guest.ok(11, &[H(list), H(scene.allocators[index])]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, scene.queue, list, asynchronous, 2);
                guest.ok(11, &[H(list), H(scene.allocators[index])]);
                if index == 0 {
                    guest.ok(47, &[H(one)]);
                }
            }
            let expected = if index == 0 {
                [72, 127, 47, 199, 18, 167, 97, 211].repeat(2)
            } else {
                [18, 167, 97, 211, 72, 127, 47, 199].repeat(2)
            };
            let output = guest.bytes(&[0xa5; 32]);
            guest.ok(10, &[H(scene.targets[index]), I(0), I(output + 8), I(16)]);
            readbacks.push((output, expected));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 2, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_sampler_handles_invalid_second_is_atomic_and_lod_is_view_relative() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let scene = guest_sampler_scene(&mut guest);
        let zero = guest.handle(46, &[H(scene.device), I(0), I(0), I(1f32.to_bits() as u64)]);
        let one = guest.handle(46, &[H(scene.device), I(1), I(0), I(1f32.to_bits() as u64)]);
        let stale = guest.handle(46, &[H(scene.device), I(0), I(0), I(0)]);
        guest.ok(47, &[H(stale)]);
        let list = scene.lists[0];
        guest_sampler_draw_setup(&mut guest, &scene, list, scene.targets[0]);
        let good =
            guest_sampler_handle_table(&mut guest, scene.views.map(Some), [Some(zero), Some(one)]);
        guest.ok(48, &[H(list), I(good)]);
        guest_sampler_column(&mut guest, list, 0);
        // Valid changed sampler0 followed by stale sampler1 must not partially
        // replace the prior table. Column1 remains identical to column0.
        let bad =
            guest_sampler_handle_table(&mut guest, scene.views.map(Some), [Some(one), Some(stale)]);
        guest.call(48, &[H(list), I(bad)], 2, Some(u64::MAX));
        guest_sampler_column(&mut guest, list, 1);
        guest.barrier(list, scene.targets[0], RENDER_TARGET, COPY_SOURCE);
        finish_guest_submission(&mut guest, scene.queue, list, asynchronous, 2);
        guest.ok(11, &[H(list), H(scene.allocators[0])]);
        let expected = [72, 127, 47, 199].repeat(4);
        let mut readbacks = Vec::new();
        let before = guest.bytes(&[0xa5; 32]);
        guest.ok(10, &[H(scene.targets[0]), I(0), I(before + 8), I(16)]);
        readbacks.push((before, expected.clone()));
        // A valid sampler handle can still be invalid for a particular pair:
        // root has mip2, but narrow view has only relative mip0. Descriptor
        // binding is valid; draw preparation rejects relative mip1 atomically.
        guest.barrier(list, scene.targets[0], COPY_SOURCE, RENDER_TARGET);
        guest_sampler_draw_setup(&mut guest, &scene, list, scene.targets[0]);
        let narrow = guest_sampler_handle_table(
            &mut guest,
            [Some(scene.views[0]), Some(scene.narrow)],
            [Some(zero), Some(one)],
        );
        guest.ok(48, &[H(list), I(narrow)]);
        guest_sampler_column(&mut guest, list, 0);
        guest.barrier(list, scene.targets[0], RENDER_TARGET, COPY_SOURCE);
        guest.ok(12, &[H(list)]);
        guest.call(
            if asynchronous { 30 } else { 25 },
            &[H(scene.queue), H(list)],
            2,
            Some(u64::MAX),
        );
        guest.ok(11, &[H(list), H(scene.allocators[0])]);
        let after = guest.bytes(&[0xa5; 32]);
        guest.ok(10, &[H(scene.targets[0]), I(0), I(after + 8), I(16)]);
        readbacks.push((after, expected.clone()));
        guest.ok(47, &[H(zero)]);
        guest.ok(47, &[H(one)]);
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 1, &expected);
    }
}

// Emit runtime stores, not edits to the PE's initial data. The final overlapping
// word writes an odd logical tail exactly, without touching either guard.
fn guest_runtime_write_bytes(guest: &mut GuestScript, address: u64, bytes: &[u8]) {
    assert!(bytes.len() >= 8);
    for offset in (0..=bytes.len() - 8).step_by(8) {
        guest.mov_imm(
            [0x48, 0xb8],
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()),
        );
        guest.store_rax(address + offset as u64);
    }
    if !bytes.len().is_multiple_of(8) {
        let offset = bytes.len() - 8;
        guest.mov_imm(
            [0x48, 0xb8],
            u64::from_le_bytes(bytes[offset..].try_into().unwrap()),
        );
        guest.store_rax(address + offset as u64);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_backed_copy_snapshots_at_submission_then_retires() {
    const SUBMITTED: [u8; 37] = [
        3, 251, 17, 129, 64, 7, 223, 41, 155, 2, 199, 83, 237, 11, 29, 47, 61, 79, 97, 113, 131,
        149, 167, 181, 197, 211, 227, 241, 5, 19, 37, 53, 71, 89, 107, 127, 151,
    ];
    const AFTER: [u8; 37] = [
        202, 4, 188, 6, 174, 8, 160, 10, 146, 12, 132, 14, 118, 16, 104, 18, 90, 20, 76, 22, 62,
        24, 48, 26, 34, 28, 20, 30, 6, 32, 248, 34, 234, 36, 220, 38, 206,
    ];
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
        let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
        let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
        let mut initial = vec![0xa5; 8];
        initial.extend([0x11; 37]);
        initial.extend([0xa5; 8]);
        let backing = guest.bytes(&initial);
        let source = guest.handle(49, &[H(device), I(37), I(COPY_SOURCE), I(backing + 8)]);
        let intermediate = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
        let destination = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
        guest.ok(42, &[H(lists[0]), H(intermediate), H(source)]);
        guest.barrier(lists[0], intermediate, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[0])]);
        guest.ok(42, &[H(lists[1]), H(destination), H(intermediate)]);
        guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        // A was present at creation and recording. Only B is submitted.
        guest_runtime_write_bytes(&mut guest, backing + 8, &SUBMITTED);
        let producer = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        // C is written by actual guest instructions immediately after submit.
        guest_runtime_write_bytes(&mut guest, backing + 8, &AFTER);
        let wait = guest_geometry_wait_record(&mut guest, queues[0], producer);
        let consumer = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        guest.ok(26, &[H(source)]);
        guest.ok(26, &[H(intermediate)]);
        for receipt in if producer_first {
            [producer, consumer]
        } else {
            [consumer, producer]
        } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        for receipt in [producer, consumer] {
            guest.ok(33, &[H(receipt)]);
        }
        let output = guest.bytes(&[0xa5; 53]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(37)]);
        check_original_guest_backend(
            guest,
            true,
            true,
            &[(output, SUBMITTED.to_vec()), (backing, AFTER.to_vec())],
            0,
            &[],
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_backed_copy_forbidden_roles_and_failed_submit_preserve_destination() {
    const PRIOR: [u8; 37] = [
        1, 5, 9, 13, 17, 21, 25, 29, 33, 37, 41, 45, 49, 53, 57, 61, 65, 69, 73, 77, 81, 85, 89,
        93, 97, 101, 105, 109, 113, 117, 121, 125, 129, 133, 137, 141, 145,
    ];
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queue = guest.handle(2, &[H(device)]);
    let allocator = guest.handle(3, &[H(device)]);
    let list = guest.handle(4, &[H(device), H(allocator)]);
    let input = guest.bytes(&[0x73; 37]);
    let source = guest.handle(49, &[H(device), I(37), I(COPY_SOURCE), I(input)]);
    let mut guarded = vec![0xa5; 8];
    guarded.extend([0x39; 37]);
    guarded.extend([0xa5; 8]);
    let forbidden_memory = guest.bytes(&guarded);
    let forbidden_destination = guest.handle(
        49,
        &[H(device), I(37), I(COPY_DEST), I(forbidden_memory + 8)],
    );
    let seed = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
    let destination = guest.handle(5, &[H(device), I(37), I(COPY_DEST)]);
    let bytes = guest.bytes(&PRIOR);
    guest.ok(9, &[H(seed), I(0), I(bytes), I(37)]);
    guest.barrier(list, seed, COPY_DEST, COPY_SOURCE);
    guest.ok(42, &[H(list), H(destination), H(seed)]);
    guest.barrier(list, destination, COPY_DEST, COPY_SOURCE);
    finish_guest_submission(&mut guest, queue, list, true, 0);
    guest.ok(11, &[H(list), H(allocator)]);
    // Creation and recording deliberately require no backing read. This range
    // is unmapped; discard the recording without trying to submit it.
    let unmapped = guest.handle(49, &[H(device), I(37), I(COPY_SOURCE), I(0x600000000)]);
    guest.ok(42, &[H(list), H(destination), H(unmapped)]);
    guest.ok(11, &[H(list), H(allocator)]);
    guest.ok(26, &[H(unmapped)]);
    guest.call(
        49,
        &[H(device), I(37), I(COPY_SOURCE), I(u64::MAX - 16)],
        2,
        Some(u64::MAX),
    );
    // Whole/region copies may only target owned storage. Image transfers
    // still reject guest-backed storage in either role, even for in-bounds ranges.
    guest.call(
        42,
        &[H(list), H(forbidden_destination), H(seed)],
        2,
        Some(u64::MAX),
    );
    guest.call(
        42,
        &[H(list), H(forbidden_destination), H(source)],
        2,
        Some(u64::MAX),
    );
    // Guest-backed region sources are covered by original_guest_backed_region_.
    guest.call(
        43,
        &[H(list), H(forbidden_destination), I(0), H(seed), I(0), I(4)],
        2,
        Some(u64::MAX),
    );
    let texture = guest.handle(6, &[H(device), I(1), I(1), I(1), I(COPY_DEST)]);
    guest.call(
        44,
        &[
            H(list),
            H(texture),
            I(0),
            I(0),
            I(0),
            H(source),
            I(1),
            I(0),
            I(4),
            I(1),
            I(1),
        ],
        2,
        Some(u64::MAX),
    );
    guest.call(
        44,
        &[
            H(list),
            H(forbidden_destination),
            I(1),
            I(0),
            I(4),
            H(texture),
            I(0),
            I(0),
            I(0),
            I(1),
            I(1),
        ],
        2,
        Some(u64::MAX),
    );
    // A valid earlier whole copy must not publish when a later barrier is
    // invalid. This is validation failure evidence, not a GPU/device fault.
    guest.barrier(list, destination, COPY_SOURCE, COPY_DEST);
    guest.ok(42, &[H(list), H(destination), H(source)]);
    guest.barrier(list, source, COPY_DEST, COPY_SOURCE); // actual source state is COPY_SOURCE
    guest.barrier(list, destination, COPY_DEST, COPY_SOURCE);
    guest.ok(12, &[H(list)]);
    guest.call(35, &[H(queue), H(list), I(0), I(0)], 2, Some(u64::MAX));
    guest.ok(11, &[H(list), H(allocator)]);
    let output = guest.bytes(&[0xa5; 53]);
    guest.ok(10, &[H(destination), I(0), I(output + 8), I(37)]);
    guest.ok(26, &[H(source)]);
    guest.ok(26, &[H(forbidden_destination)]);
    check_original_guest_backend(
        guest,
        true,
        true,
        &[(output, PRIOR.to_vec()), (forbidden_memory, vec![0x39; 37])],
        0,
        &[],
    );
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_backed_region_submission_snapshot_preserves_borders_and_odd_tail() {
    const SUBMITTED: [u8; 13] = [3, 251, 17, 129, 64, 7, 223, 41, 155, 2, 199, 83, 237];
    const AFTER: [u8; 13] = [
        201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
    ];
    const PRIOR: [u8; 13] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    ];
    // Independent full-destination literals: interior odd patch, then a patch
    // ending exactly at both logical source and destination ends.
    for (source_offset, destination_offset, byte_len, expected) in [
        (
            1,
            3,
            7,
            [101, 102, 103, 251, 17, 129, 64, 7, 223, 41, 111, 112, 113],
        ),
        (
            8,
            8,
            5,
            [101, 102, 103, 104, 105, 106, 107, 108, 155, 2, 199, 83, 237],
        ),
    ] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let mut initial = vec![0xa5; 8];
        initial.extend([0x11; 13]);
        initial.extend([0xa5; 8]);
        let backing = guest.bytes(&initial);
        let source = guest.handle(49, &[H(device), I(13), I(COPY_SOURCE), I(backing + 8)]);
        let destination = guest.handle(5, &[H(device), I(13), I(COPY_DEST)]);
        let prior = guest.bytes(&PRIOR);
        guest.ok(9, &[H(destination), I(0), I(prior), I(13)]);
        guest.ok(
            43,
            &[
                H(list),
                H(destination),
                I(destination_offset),
                H(source),
                I(source_offset),
                I(byte_len),
            ],
        );
        guest.barrier(list, destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(list)]);
        // The backing differs at creation/recording, submission, and collection.
        guest_runtime_write_bytes(&mut guest, backing + 8, &SUBMITTED);
        let receipt = guest.handle(35, &[H(queue), H(list), I(0), I(0)]);
        guest_runtime_write_bytes(&mut guest, backing + 8, &AFTER);
        guest.ok(11, &[H(list), H(allocator)]);
        guest.ok(26, &[H(source)]);
        guest.call(32, &[H(receipt)], 0, Some(1));
        guest.ok(33, &[H(receipt)]);
        let output = guest.bytes(&[0xa5; 29]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(13)]);
        check_original_guest_backend(
            guest,
            true,
            true,
            &[(output, expected.to_vec()), (backing, AFTER.to_vec())],
            0,
            &[],
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_backed_region_pending_destination_version_and_retirement() {
    const SUBMITTED: [u8; 13] = [3, 251, 17, 129, 64, 7, 223, 41, 155, 2, 199, 83, 237];
    const AFTER: [u8; 13] = [
        201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
    ];
    const PRIOR: [u8; 13] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    ];
    const EXPECTED: [u8; 13] = [101, 102, 103, 251, 17, 129, 64, 7, 223, 41, 111, 112, 113];
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [0, 1].map(|_| guest.handle(2, &[H(device)]));
        let allocators = [0, 1].map(|_| guest.handle(3, &[H(device)]));
        let lists = allocators.map(|a| guest.handle(4, &[H(device), H(a)]));
        let mut initial = vec![0xa5; 8];
        initial.extend([0x11; 13]);
        initial.extend([0xa5; 8]);
        let backing = guest.bytes(&initial);
        let source = guest.handle(49, &[H(device), I(13), I(COPY_SOURCE), I(backing + 8)]);
        let seed = guest.handle(5, &[H(device), I(13), I(COPY_DEST)]);
        let destination = guest.handle(5, &[H(device), I(13), I(COPY_DEST)]);
        let prior = guest.bytes(&PRIOR);
        guest.ok(9, &[H(seed), I(0), I(prior), I(13)]);
        // Stale committed destination cannot supply the expected untouched edges.
        let stale = guest.bytes(&[10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22]);
        guest.ok(9, &[H(destination), I(0), I(stale), I(13)]);
        guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
        guest.ok(42, &[H(lists[0]), H(destination), H(seed)]);
        guest.ok(12, &[H(lists[0])]);
        guest.ok(
            43,
            &[H(lists[1]), H(destination), I(3), H(source), I(1), I(7)],
        );
        guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let producer = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let wait = guest_geometry_wait_record(&mut guest, queues[0], producer);
        guest_runtime_write_bytes(&mut guest, backing + 8, &SUBMITTED);
        let consumer = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        guest_runtime_write_bytes(&mut guest, backing + 8, &AFTER);
        for index in 0..2 {
            guest.ok(11, &[H(lists[index]), H(allocators[index])]);
        }
        guest.ok(26, &[H(source)]);
        guest.ok(26, &[H(seed)]);
        for receipt in if producer_first {
            [producer, consumer]
        } else {
            [consumer, producer]
        } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        for receipt in [producer, consumer] {
            guest.ok(33, &[H(receipt)]);
        }
        let output = guest.bytes(&[0xa5; 29]);
        guest.ok(10, &[H(destination), I(0), I(output + 8), I(13)]);
        check_original_guest_backend(
            guest,
            true,
            true,
            &[(output, EXPECTED.to_vec()), (backing, AFTER.to_vec())],
            0,
            &[],
        );
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_r8_mips_nested_views_changed_uploads_and_retirement() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        let textures =
            [0, 1].map(|_| guest.handle(40, &[H(device), I(5), I(3), I(3), I(3), I(COPY_DEST)]));
        let views = textures.map(|t| guest.handle(37, &[H(device), H(t), I(0), I(3), I(6)]));
        let parent = guest.handle(37, &[H(device), H(textures[0]), I(1), I(2), I(6)]);
        let child = guest.handle(45, &[H(device), H(parent), I(1), I(1), I(6)]);
        guest.ok(38, &[H(parent)]);
        guest.call(26, &[H(textures[0])], 2, Some(u64::MAX));
        let target = guest.handle(6, &[H(device), I(6), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let p = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let uv = guest.bytes(&floats(&[
            [0.1, 0.1, 0., 1.],
            [0.1, 0.1, 0., 1.],
            [0.1, 0.1, 0., 1.],
            [0.9, 0.9, 0., 1.],
            [0.9, 0.9, 0., 1.],
            [0.9, 0.9, 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(uv), I(96)]);
        let pipeline = guest_nested_view_pipeline(&mut guest, device);
        let mut readbacks = Vec::new();
        for frame in 0..2 {
            let (a, b, red) = if frame == 0 {
                (
                    [
                        11u8, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 171, 193, 99,
                    ],
                    [
                        61u8, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 201, 211, 219,
                    ],
                    [11u8, 61, 25, 171, 99, 219],
                )
            } else {
                (
                    [
                        31u8, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 121, 143, 149,
                    ],
                    [
                        81u8, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 151, 161, 169,
                    ],
                    [31u8, 81, 45, 121, 149, 169],
                )
            };
            for (texture, bytes) in [(textures[0], a), (textures[1], b)] {
                let address = guest.bytes(&bytes);
                guest.ok(9, &[H(texture), I(0), I(address), I(18)]);
                guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
            }
            if frame == 0 {
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(6), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            for (column, (view, mip)) in [
                (views[0], 0),
                (views[1], 0),
                (views[0], 0),
                (views[0], 1),
                (child, 0),
                (views[1], 2),
            ]
            .into_iter()
            .enumerate()
            {
                guest.ok(
                    19,
                    &[H(list), H(cb), I(if column == 2 { 48 } else { 0 }), I(16)],
                );
                let table = sampled_table(&mut guest, [Some(view), None], [Some(mip), None], false);
                guest.ok(39, &[H(list), I(table)]);
                guest.ok(
                    22,
                    &[
                        H(list),
                        I(0),
                        I(0),
                        I(0),
                        I(15),
                        I(0),
                        I(0),
                        I(1),
                        I(column as u64),
                        I(0),
                        I(1),
                        I(2),
                    ],
                );
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            }
            for texture in textures {
                guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
            }
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if asynchronous && frame == 1 {
                guest.ok(12, &[H(list)]);
                let receipt = guest.handle(30, &[H(queue), H(list)]);
                guest.ok(11, &[H(list), H(allocator)]);
                for view in [views[0], views[1], child] {
                    guest.ok(38, &[H(view)]);
                }
                for resource in [textures[0], textures[1], vb, cb] {
                    guest.ok(26, &[H(resource)]);
                }
                guest.ok(27, &[H(pipeline)]);
                guest.call(32, &[H(receipt)], 0, Some(1));
                guest.ok(33, &[H(receipt)]);
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 6);
                guest.ok(11, &[H(list), H(allocator)]);
                if frame == 1 {
                    for view in [views[0], views[1], child] {
                        guest.ok(38, &[H(view)]);
                    }
                    for texture in textures {
                        guest.ok(26, &[H(texture)]);
                    }
                }
            }
            let row: Vec<_> = red.into_iter().flat_map(|r| [r, 0, 0, 255]).collect();
            let output = guest.bytes(&[0xa5; 64]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(48)]);
            readbacks.push((output, row.repeat(2)));
        }
        let final_pixels = readbacks.last().unwrap().1.clone();
        check_original_guest_backend(guest, asynchronous, queued, &readbacks, 2, &final_pixels);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_r8_validation_relative_lod_and_legacy_capability_rejection() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queue = guest.handle(2, &[H(device)]);
        let allocator = guest.handle(3, &[H(device)]);
        let list = guest.handle(4, &[H(device), H(allocator)]);
        guest.call(
            40,
            &[H(device), I(0), I(3), I(1), I(3), I(COPY_DEST)],
            2,
            Some(u64::MAX),
        );
        guest.call(
            40,
            &[H(device), I(5), I(3), I(4), I(3), I(COPY_DEST)],
            2,
            Some(u64::MAX),
        );
        let texture = guest.handle(40, &[H(device), I(5), I(3), I(3), I(3), I(COPY_DEST)]);
        let narrow = guest.handle(37, &[H(device), H(texture), I(1), I(1), I(6)]);
        for format in [0, 1, 2, 3, 4, 5] {
            guest.call(
                37,
                &[H(device), H(texture), I(0), I(1), I(format)],
                2,
                Some(u64::MAX),
            );
        }
        guest.call(
            45,
            &[H(device), H(narrow), I(1), I(1), I(6)],
            2,
            Some(u64::MAX),
        );
        let bytes = guest.bytes(&[
            11u8, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 171, 193, 99,
        ]);
        guest.ok(9, &[H(texture), I(0), I(bytes), I(18)]);
        // Upload overflow must reject atomically, not overwrite the valid mip.
        let bad = guest.bytes(&[250; 19]);
        guest.call(9, &[H(texture), I(0), I(bad), I(19)], 2, Some(u64::MAX));
        let target = guest.handle(6, &[H(device), I(2), I(2), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let p = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        let uv = guest.bytes(&floats(&[[0.25, 0.5, 0., 1.]; 3]));
        guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
        guest.ok(9, &[H(cb), I(0), I(uv), I(48)]);
        let pipeline = guest_nested_view_pipeline(&mut guest, device);
        let mut readbacks = Vec::new();
        for invalid in [false, true] {
            if !invalid {
                guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
                guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
            } else {
                guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
            }
            guest.targets(list, &[target]);
            guest.ok(17, &[H(list), I(2), I(2)]);
            guest.ok(14, &[H(list), H(pipeline)]);
            guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
            guest.ok(19, &[H(list), H(cb), I(0), I(16)]);
            let table = sampled_table(
                &mut guest,
                [Some(narrow), None],
                [Some(u32::from(invalid)), None],
                false,
            );
            guest.ok(39, &[H(list), I(table)]);
            guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
            guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
            if invalid {
                guest.ok(12, &[H(list)]);
                guest.call(
                    if asynchronous { 30 } else { 25 },
                    &[H(queue), H(list)],
                    2,
                    Some(u64::MAX),
                );
            } else {
                finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
            }
            guest.ok(11, &[H(list), H(allocator)]);
            let output = guest.bytes(&[0xa5; 32]);
            guest.ok(10, &[H(target), I(0), I(output + 8), I(16)]);
            readbacks.push((output, [171, 0, 0, 255].repeat(4)));
        }
        check_original_guest_backend(
            guest,
            asynchronous,
            queued,
            &readbacks,
            1,
            &[171, 0, 0, 255].repeat(4),
        );
    }
    // Explicit legacy registration must revoke typed R8 capability. The panic
    // callback is a dispatch guard, not simulated GPU execution evidence.
    let mut guest = GuestScript::new();
    let device = guest.handle(1, &[]);
    let queue = guest.handle(2, &[H(device)]);
    let allocator = guest.handle(3, &[H(device)]);
    let list = guest.handle(4, &[H(device), H(allocator)]);
    // Creation is backend-independent. Only execution checks the downgraded
    // renderer's format capabilities. Use whole-texture slot0, not view APIs.
    let texture = guest.handle(40, &[H(device), I(5), I(3), I(3), I(3), I(COPY_DEST)]);
    let texels = guest.bytes(&[128; 18]);
    guest.ok(9, &[H(texture), I(0), I(texels), I(18)]);
    let target = guest.handle(6, &[H(device), I(1), I(1), I(1), I(RENDER_TARGET)]);
    let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
    let p = guest.bytes(&floats(&[
        [-1., -1., 0., 1.],
        [3., -1., 0., 1.],
        [-1., 3., 0., 1.],
    ]));
    let uv = guest.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
    guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
    guest.ok(9, &[H(cb), I(0), I(uv), I(48)]);
    let pipeline = guest_nested_view_pipeline(&mut guest, device);
    guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
    guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
    guest.targets(list, &[target]);
    guest.ok(17, &[H(list), I(1), I(1)]);
    guest.ok(14, &[H(list), H(pipeline)]);
    guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
    guest.ok(19, &[H(list), H(cb), I(0), I(16)]);
    guest.ok(21, &[H(list), H(texture), I(0), I(0), I(0)]);
    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
    guest.ok(12, &[H(list)]);
    guest.call(25, &[H(queue), H(list)], 2, Some(u64::MAX));
    let pe = guest.finish();
    let services = Arc::new(ServicesState::new(Arc::new(MockClock::new(0))));
    {
        let mut graphics = services.graphics.lock().unwrap();
        install_pipeline_backend(&mut graphics).unwrap();
        graphics.set_metal_backend(|_, _, _| panic!("legacy R8 draw dispatched renderer"));
    }
    let mut imports = ImportRegistry::new();
    register_standard_services(&mut imports, services.clone());
    let result = run_synthetic_startup(
        &pe,
        StartupLayout {
            image_base: BASE,
            stack_top: 0x500000000,
            tls_base: 0x300000000,
        },
        imports,
        100_000,
    )
    .unwrap();
    assert_eq!(result.execution.exit_reason, ExitReason::Halt);
    for call in guest.calls {
        assert_eq!(
            result.image.memory.read_u64(call.status).unwrap(),
            call.expected_status
        );
        if let Some(expected) = call.expected_result {
            assert_eq!(result.image.memory.read_u64(call.result).unwrap(), expected);
        }
    }
    let graphics = services.graphics.lock().unwrap();
    let presentation = graphics.last_presentation.lock().unwrap();
    assert_eq!(presentation.frame_index, 0);
    assert!(presentation.pixels.is_empty());
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_mip_transfers_texture_to_texture() {
    guest_mip_transfer_pair(0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_mip_transfers_buffer_to_texture() {
    guest_mip_transfer_pair(1);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_mip_transfers_texture_to_buffer() {
    guest_mip_transfer_pair(2);
}

fn guest_mip_transfer_pair(direction: u8) {
    const PIXELS: [u8; 72] = [
        1, 41, 81, 121, 2, 42, 82, 122, 3, 43, 83, 123, 4, 44, 84, 124, 5, 45, 85, 125, 6, 46, 86,
        126, 7, 47, 87, 127, 8, 48, 88, 128, 9, 49, 89, 129, 10, 50, 90, 130, 11, 51, 91, 131, 12,
        52, 92, 132, 13, 53, 93, 133, 14, 54, 94, 134, 15, 55, 95, 135, 71, 81, 91, 101, 72, 82,
        92, 102, 201, 211, 221, 231,
    ];
    const PITCHED: [u8; 12] = [231, 232, 233, 234, 9, 19, 29, 39, 10, 20, 30, 40];
    const PRIOR: [u8; 72] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 31, 41,
        51, 61, 32, 42, 52, 62, 33, 43, 53, 63,
    ];
    const BUFFER_PRIOR: [u8; 13] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    ];
    const BUFFER_EXPECTED: [u8; 13] = [101, 102, 103, 104, 71, 81, 91, 101, 72, 82, 92, 102, 113];
    const RECT_EXPECTED: [u8; 72] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 31, 41,
        51, 61, 32, 42, 52, 62, 72, 82, 92, 102,
    ];
    const UPLOAD_EXPECTED: [u8; 72] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 9, 19, 29,
        39, 10, 20, 30, 40, 33, 43, 53, 63,
    ];
    let source_bytes = if direction == 1 {
        PITCHED.as_slice()
    } else {
        PIXELS.as_slice()
    };
    let destination_bytes = if direction == 2 {
        BUFFER_PRIOR.as_slice()
    } else {
        PRIOR.as_slice()
    };
    let expected = match direction {
        0 => RECT_EXPECTED.as_slice(),
        1 => UPLOAD_EXPECTED.as_slice(),
        _ => BUFFER_EXPECTED.as_slice(),
    };
    for pending_source in [false, true] {
        let mut expected = expected.to_vec();
        if direction == 1 && !pending_source {
            // Consumer replaces only pixel zero. Pixel one must survive from
            // the producer repair, not the poisoned host destination snapshot.
            expected[64..68].copy_from_slice(&[32, 42, 52, 62]);
        }
        for producer_first in [false, true] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
            let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
            let lists = [
                guest.handle(4, &[H(device), H(allocators[0])]),
                guest.handle(4, &[H(device), H(allocators[1])]),
            ];
            let create = |guest: &mut GuestScript, buffer: bool, len: usize| {
                if buffer {
                    guest.handle(5, &[H(device), I(len as u64), I(COPY_DEST)])
                } else {
                    guest.handle(6, &[H(device), I(5), I(3), I(3), I(COPY_DEST)])
                }
            };
            let source = create(&mut guest, direction == 1, source_bytes.len());
            let destination = create(&mut guest, direction == 2, destination_bytes.len());
            let seed_bytes = if pending_source {
                source_bytes
            } else {
                destination_bytes
            };
            let seed_is_buffer = if pending_source {
                direction == 1
            } else {
                direction == 2
            };
            let seed = create(&mut guest, seed_is_buffer, seed_bytes.len());
            let data = guest.bytes(seed_bytes);
            guest.ok(9, &[H(seed), I(0), I(data), I(seed_bytes.len() as u64)]);
            let (host, bytes) = if pending_source {
                (destination, destination_bytes)
            } else {
                (source, source_bytes)
            };
            let data = guest.bytes(bytes);
            guest.ok(9, &[H(host), I(0), I(data), I(bytes.len() as u64)]);
            guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
            let changed = if pending_source { source } else { destination };
            // Initialize untouched levels independently; producer updates mip one only.
            let mut poisoned = seed_bytes.to_vec();
            if seed_is_buffer {
                poisoned.fill(0xee);
            } else {
                poisoned[60..68].fill(0xee);
            }
            let initial = guest.bytes(&poisoned);
            guest.ok(
                9,
                &[H(changed), I(0), I(initial), I(seed_bytes.len() as u64)],
            );
            if seed_is_buffer {
                guest.ok(42, &[H(lists[0]), H(changed), H(seed)]);
            } else {
                guest.ok(
                    50,
                    &[
                        H(lists[0]),
                        H(changed),
                        I(0),
                        I(1),
                        I(0),
                        I(0),
                        H(seed),
                        I(0),
                        I(1),
                        I(0),
                        I(0),
                        I(2),
                        I(1),
                    ],
                );
            }
            guest.barrier(
                lists[usize::from(!pending_source)],
                source,
                COPY_DEST,
                COPY_SOURCE,
            );
            guest.ok(12, &[H(lists[0])]);
            let (source_kind, source_a, source_b) = match direction {
                0 => (0, 1, 0),
                1 => (1, 4, 12),
                _ => (0, 0, 0),
            };
            let (destination_kind, destination_a, destination_b) = if direction == 2 {
                (1, 4, 20)
            } else {
                (0, 0, 0)
            };
            guest.ok(
                50,
                &[
                    H(lists[1]),
                    H(destination),
                    I(destination_kind),
                    I(if direction == 2 {
                        0
                    } else if direction == 0 {
                        2
                    } else {
                        1
                    }),
                    I(destination_a),
                    I(destination_b),
                    H(source),
                    I(source_kind),
                    I(if direction == 1 { 0 } else { 1 }),
                    I(source_a),
                    I(source_b),
                    I(if direction == 0 || (direction == 1 && !pending_source) {
                        1
                    } else {
                        2
                    }),
                    I(1),
                ],
            );
            guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
            guest.ok(12, &[H(lists[1])]);
            let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
            let value = guest.call(36, &[H(a)], 0, Some(1));
            let dependency = guest.bytes(&[0; 16]);
            guest.copy_result(queues[0], dependency);
            guest.copy_result(value, dependency + 8);
            let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
            for index in 0..2 {
                guest.ok(11, &[H(lists[index]), H(allocators[index])]);
            }
            guest.ok(26, &[H(seed)]);
            guest.ok(26, &[H(source)]);
            for receipt in if producer_first { [a, b] } else { [b, a] } {
                guest.call(32, &[H(receipt)], 0, Some(1));
            }
            guest.ok(33, &[H(a)]);
            guest.ok(33, &[H(b)]);
            let output = guest.bytes(&vec![0xa5; expected.len() + 16]);
            guest.ok(
                10,
                &[
                    H(destination),
                    I(0),
                    I(output + 8),
                    I(expected.len() as u64),
                ],
            );
            check_original_guest_backend(guest, true, true, &[(output, expected.to_vec())], 0, &[]);
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_transfers_texture_to_texture() {
    guest_array_transfer_pair(0);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_transfers_buffer_to_texture() {
    guest_array_transfer_pair(1);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_transfers_texture_to_buffer() {
    guest_array_transfer_pair(2);
}

fn guest_array_transfer_pair(direction: u8) {
    const PIXELS: [u8; 144] = [
        1, 41, 81, 121, 2, 42, 82, 122, 3, 43, 83, 123, 4, 44, 84, 124, 5, 45, 85, 125, 6, 46, 86,
        126, 7, 47, 87, 127, 8, 48, 88, 128, 9, 49, 89, 129, 10, 50, 90, 130, 11, 51, 91, 131, 12,
        52, 92, 132, 13, 53, 93, 133, 14, 54, 94, 134, 15, 55, 95, 135, 71, 81, 91, 101, 72, 82,
        92, 102, 201, 211, 221, 231, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39,
        39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39,
        39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 39, 17,
        53, 127, 193, 29, 71, 131, 197, 203, 213, 223, 233,
    ];
    const PITCHED: [u8; 12] = [231, 232, 233, 234, 9, 19, 29, 39, 10, 20, 30, 40];
    const PRIOR: [u8; 144] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 31, 41,
        51, 61, 32, 42, 52, 62, 33, 43, 53, 63, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
    ];
    const BUFFER_PRIOR: [u8; 13] = [
        101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    ];
    const BUFFER_EXPECTED: [u8; 13] = [101, 102, 103, 104, 17, 53, 127, 193, 29, 71, 131, 197, 113];
    const RECT_EXPECTED: [u8; 144] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 31, 41,
        51, 61, 32, 42, 52, 62, 33, 43, 53, 63, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 29, 71, 131, 197,
    ];
    const UPLOAD_EXPECTED: [u8; 144] = [
        101, 1, 21, 241, 102, 2, 22, 242, 103, 3, 23, 243, 104, 4, 24, 244, 105, 5, 25, 245, 106,
        6, 26, 246, 107, 7, 27, 247, 108, 8, 28, 248, 109, 9, 29, 249, 110, 10, 30, 250, 111, 11,
        31, 251, 112, 12, 32, 252, 113, 13, 33, 253, 114, 14, 34, 254, 115, 15, 35, 255, 31, 41,
        51, 61, 32, 42, 52, 62, 33, 43, 53, 63, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89, 89,
        89, 9, 19, 29, 39, 10, 20, 30, 40, 89, 89, 89, 89,
    ];
    let source_bytes = if direction == 1 {
        PITCHED.as_slice()
    } else {
        PIXELS.as_slice()
    };
    let destination_bytes = if direction == 2 {
        BUFFER_PRIOR.as_slice()
    } else {
        PRIOR.as_slice()
    };
    let expected = match direction {
        0 => RECT_EXPECTED.as_slice(),
        1 => UPLOAD_EXPECTED.as_slice(),
        _ => BUFFER_EXPECTED.as_slice(),
    };
    for pending_source in [false, true] {
        let mut expected = expected.to_vec();
        if direction == 1 && !pending_source {
            // Consumer replaces only pixel zero. Pixel one must survive from
            // the producer repair, not the poisoned host destination snapshot.
            expected[136..140].copy_from_slice(&[89, 89, 89, 89]);
        }
        for producer_first in [false, true] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
            let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
            let lists = [
                guest.handle(4, &[H(device), H(allocators[0])]),
                guest.handle(4, &[H(device), H(allocators[1])]),
            ];
            let create = |guest: &mut GuestScript, buffer: bool, len: usize| {
                if buffer {
                    guest.handle(5, &[H(device), I(len as u64), I(COPY_DEST)])
                } else {
                    guest.handle(51, &[H(device), I(5), I(3), I(3), I(2), I(COPY_DEST)])
                }
            };
            let source = create(&mut guest, direction == 1, source_bytes.len());
            let destination = create(&mut guest, direction == 2, destination_bytes.len());
            let seed_bytes = if pending_source {
                source_bytes
            } else {
                destination_bytes
            };
            let seed_is_buffer = if pending_source {
                direction == 1
            } else {
                direction == 2
            };
            let seed = create(&mut guest, seed_is_buffer, seed_bytes.len());
            let data = guest.bytes(seed_bytes);
            guest.ok(9, &[H(seed), I(0), I(data), I(seed_bytes.len() as u64)]);
            let (host, bytes) = if pending_source {
                (destination, destination_bytes)
            } else {
                (source, source_bytes)
            };
            let data = guest.bytes(bytes);
            guest.ok(9, &[H(host), I(0), I(data), I(bytes.len() as u64)]);
            guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
            let changed = if pending_source { source } else { destination };
            // Initialize untouched levels independently; producer updates mip one only.
            let mut poisoned = seed_bytes.to_vec();
            if seed_is_buffer {
                poisoned.fill(0xee);
            } else {
                poisoned[132..140].fill(0xee);
            }
            let initial = guest.bytes(&poisoned);
            guest.ok(
                9,
                &[H(changed), I(0), I(initial), I(seed_bytes.len() as u64)],
            );
            if seed_is_buffer {
                guest.ok(42, &[H(lists[0]), H(changed), H(seed)]);
            } else {
                guest.ok(
                    50,
                    &[
                        H(lists[0]),
                        H(changed),
                        I(0),
                        I((1u64 << 32) | 1),
                        I(0),
                        I(0),
                        H(seed),
                        I(0),
                        I((1u64 << 32) | 1),
                        I(0),
                        I(0),
                        I(2),
                        I(1),
                    ],
                );
            }
            guest.barrier(
                lists[usize::from(!pending_source)],
                source,
                COPY_DEST,
                COPY_SOURCE,
            );
            guest.ok(12, &[H(lists[0])]);
            let (source_kind, source_a, source_b) = match direction {
                0 => (0, 1, 0),
                1 => (1, 4, 12),
                _ => (0, 0, 0),
            };
            let (destination_kind, destination_a, destination_b) = if direction == 2 {
                (1, 4, 20)
            } else {
                (0, 0, 0)
            };
            guest.ok(
                50,
                &[
                    H(lists[1]),
                    H(destination),
                    I(destination_kind),
                    I(if direction == 2 {
                        0
                    } else if direction == 0 {
                        (1u64 << 32) | 2
                    } else {
                        (1u64 << 32) | 1
                    }),
                    I(destination_a),
                    I(destination_b),
                    H(source),
                    I(source_kind),
                    I(if direction == 1 { 0 } else { (1u64 << 32) | 1 }),
                    I(source_a),
                    I(source_b),
                    I(if direction == 0 || (direction == 1 && !pending_source) {
                        1
                    } else {
                        2
                    }),
                    I(1),
                ],
            );
            guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
            guest.ok(12, &[H(lists[1])]);
            let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
            let value = guest.call(36, &[H(a)], 0, Some(1));
            let dependency = guest.bytes(&[0; 16]);
            guest.copy_result(queues[0], dependency);
            guest.copy_result(value, dependency + 8);
            let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(dependency), I(1)]);
            for index in 0..2 {
                guest.ok(11, &[H(lists[index]), H(allocators[index])]);
            }
            guest.ok(26, &[H(seed)]);
            guest.ok(26, &[H(source)]);
            for receipt in if producer_first { [a, b] } else { [b, a] } {
                guest.call(32, &[H(receipt)], 0, Some(1));
            }
            guest.ok(33, &[H(a)]);
            guest.ok(33, &[H(b)]);
            let output = guest.bytes(&vec![0xa5; expected.len() + 16]);
            guest.ok(
                10,
                &[
                    H(destination),
                    I(0),
                    I(output + 8),
                    I(expected.len() as u64),
                ],
            );
            check_original_guest_backend(guest, true, true, &[(output, expected.to_vec())], 0, &[]);
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_views_slices_nested_mips_snapshots_and_retirement() {
    guest_array_view_snapshots(false);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_ranges_nested_snapshots_and_retirement() {
    guest_array_view_snapshots(true);
}

fn guest_array_view_snapshots(ranges: bool) {
    for layers in [1u64, 2] {
        for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queue = guest.handle(2, &[H(device)]);
            let allocator = guest.handle(3, &[H(device)]);
            let list = guest.handle(4, &[H(device), H(allocator)]);
            let texture = guest.handle(51, &[H(device), I(5), I(3), I(3), I(layers), I(COPY_DEST)]);
            // Opcode45 order matches37: device,parent,relative base,count,format.
            // An sRGB parent does not silently override its linear child's format.
            let (parent, first_slice) = if ranges {
                let range = guest.handle(
                    57,
                    &[H(device), H(texture), I(0), I(layers), I(0), I(3), I(1)],
                );
                let child = guest.handle(
                    58,
                    &[H(device), H(range), I(layers - 1), I(1), I(1), I(2), I(1)],
                );
                let parent = guest.handle(59, &[H(device), H(child), I(0), I(0), I(2), I(1)]);
                let first = guest.handle(59, &[H(device), H(range), I(0), I(1), I(1), I(0)]);
                // Range parents are descriptors, not lifetime dependencies of children.
                guest.ok(60, &[H(range)]);
                guest.ok(60, &[H(child)]);
                (parent, first)
            } else {
                (
                    guest.handle(
                        52,
                        &[H(device), H(texture), I(layers - 1), I(1), I(2), I(1)],
                    ),
                    guest.handle(52, &[H(device), H(texture), I(0), I(1), I(1), I(0)]),
                )
            };
            let linear = guest.handle(45, &[H(device), H(parent), I(0), I(2), I(0)]);
            let srgb = guest.handle(45, &[H(device), H(parent), I(0), I(2), I(1)]);
            let last = guest.handle(45, &[H(device), H(linear), I(1), I(1), I(0)]);
            let last_srgb = guest.handle(45, &[H(device), H(srgb), I(1), I(1), I(1)]);
            guest.ok(38, &[H(parent)]);
            guest.call(26, &[H(texture)], 2, Some(u64::MAX));
            let target = guest.handle(6, &[H(device), I(8), I(2), I(1), I(RENDER_TARGET)]);
            let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
            let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
            let p = guest.bytes(&floats(&[
                [-1., -1., 0., 1.],
                [3., -1., 0., 1.],
                [-1., 3., 0., 1.],
            ]));
            let uv = guest.bytes(&floats(&[
                [0.25, 0.25, 0., 1.],
                [0.25, 0.25, 0., 1.],
                [0.25, 0.25, 0., 1.],
                [0.75, 0.75, 0., 1.],
                [0.75, 0.75, 0., 1.],
                [0.75, 0.75, 0., 1.],
            ]));
            guest.ok(9, &[H(vb), I(0), I(p), I(48)]);
            guest.ok(9, &[H(cb), I(0), I(uv), I(96)]);
            let pipeline = guest_nested_view_pipeline(&mut guest, device);
            let mut readbacks = Vec::new();
            for frame in 0..2 {
                let (top, decoded, bottom, tail, tail_decoded) = if frame == 0 {
                    (
                        [128u8, 64, 192, 128],
                        [55, 13, 134, 128],
                        [16, 32, 48, 96],
                        [64, 128, 32, 192],
                        [13, 55, 4, 192],
                    )
                } else {
                    (
                        [64, 128, 32, 192],
                        [13, 55, 4, 192],
                        [80, 160, 240, 32],
                        [128, 64, 192, 128],
                        [55, 13, 134, 128],
                    )
                };
                // Selected slice: mip0 decoy; mip1 two distinct pixels; mip2 tail.
                let mut bytes = Vec::new();
                if layers == 2 {
                    bytes.extend([7u8, 17, 27, 255].repeat(18));
                }
                bytes.extend([201u8, 9, 21, 255].repeat(15));
                bytes.extend(top);
                bytes.extend(bottom);
                bytes.extend(tail);
                let upload = guest.bytes(&bytes);
                guest.ok(9, &[H(texture), I(0), I(upload), I(72 * layers)]);
                guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
                if frame == 0 {
                    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                    guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
                } else {
                    guest.barrier(list, target, COPY_SOURCE, RENDER_TARGET);
                }
                guest.targets(list, &[target]);
                guest.ok(17, &[H(list), I(8), I(2)]);
                guest.ok(14, &[H(list), H(pipeline)]);
                guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
                for (column, (view, mip)) in [
                    (linear, 0),
                    (srgb, 0),
                    (linear, 1),
                    (last, 0),
                    (linear, 0),
                    (last_srgb, 0),
                    (srgb, 1),
                    (first_slice, 0),
                ]
                .into_iter()
                .enumerate()
                {
                    guest.ok(
                        19,
                        &[H(list), H(cb), I(if column == 4 { 48 } else { 0 }), I(16)],
                    );
                    let table =
                        sampled_table(&mut guest, [Some(view), None], [Some(mip), None], false);
                    guest.ok(39, &[H(list), I(table)]);
                    guest.ok(
                        22,
                        &[
                            H(list),
                            I(0),
                            I(0),
                            I(0),
                            I(15),
                            I(0),
                            I(0),
                            I(1),
                            I(column as u64),
                            I(0),
                            I(1),
                            I(2),
                        ],
                    );
                    guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
                }
                guest.barrier(list, texture, PIXEL_RESOURCE, COPY_DEST);
                guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
                if asynchronous && frame == 1 {
                    guest.ok(12, &[H(list)]);
                    let receipt = guest.handle(30, &[H(queue), H(list)]);
                    guest.ok(11, &[H(list), H(allocator)]);
                    for view in [linear, srgb, last, last_srgb, first_slice] {
                        guest.ok(38, &[H(view)]);
                    }
                    for resource in [texture, vb, cb] {
                        guest.ok(26, &[H(resource)]);
                    }
                    guest.ok(27, &[H(pipeline)]);
                    guest.call(32, &[H(receipt)], 0, Some(1));
                    guest.ok(33, &[H(receipt)]);
                } else {
                    finish_guest_submission(&mut guest, queue, list, asynchronous, 8);
                    guest.ok(11, &[H(list), H(allocator)]);
                    if frame == 1 {
                        for view in [linear, srgb, last, last_srgb, first_slice] {
                            guest.ok(38, &[H(view)]);
                        }
                        guest.ok(26, &[H(texture)]);
                    }
                }
                let expected = [
                    top,
                    decoded,
                    tail,
                    tail,
                    bottom,
                    tail_decoded,
                    tail_decoded,
                    if layers == 2 { [7, 17, 27, 255] } else { top },
                ]
                .as_flattened()
                .repeat(2);
                let output = guest.bytes(&[0xa5; 80]);
                guest.ok(10, &[H(target), I(0), I(output + 8), I(64)]);
                readbacks.push((output, expected));
            }
            let final_pixels = readbacks.last().unwrap().1.clone();
            check_original_guest_backend(guest, asynchronous, queued, &readbacks, 2, &final_pixels);
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_views_pending_copy_sample_and_retirement() {
    guest_array_view_pending(false);
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_array_ranges_pending_copy_sample_and_retirement() {
    guest_array_view_pending(true);
}

fn guest_array_view_pending(ranges: bool) {
    for producer_first in [false, true] {
        let mut guest = GuestScript::new();
        let device = guest.handle(1, &[]);
        let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
        let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
        let lists = [
            guest.handle(4, &[H(device), H(allocators[0])]),
            guest.handle(4, &[H(device), H(allocators[1])]),
        ];
        let root = guest.handle(51, &[H(device), I(5), I(3), I(3), I(2), I(COPY_DEST)]);
        let source = guest.handle(5, &[H(device), I(8), I(COPY_DEST)]);
        let poison = guest.bytes(&[0xee; 144]);
        guest.ok(9, &[H(root), I(0), I(poison), I(144)]);
        let bytes = guest.bytes(&[17, 53, 127, 193, 29, 71, 131, 197]);
        guest.ok(9, &[H(source), I(0), I(bytes), I(8)]);
        let view = if ranges {
            let range = guest.handle(57, &[H(device), H(root), I(1), I(1), I(0), I(3), I(0)]);
            let child = guest.handle(58, &[H(device), H(range), I(0), I(1), I(1), I(1), I(0)]);
            guest.ok(60, &[H(range)]);
            let view = guest.handle(59, &[H(device), H(child), I(0), I(0), I(1), I(0)]);
            guest.ok(60, &[H(child)]);
            view
        } else {
            let parent = guest.handle(52, &[H(device), H(root), I(1), I(0), I(3), I(0)]);
            let view = guest.handle(45, &[H(device), H(parent), I(1), I(1), I(0)]);
            guest.ok(38, &[H(parent)]);
            view
        };
        guest.barrier(lists[0], source, COPY_DEST, COPY_SOURCE);
        guest.ok(
            50,
            &[
                H(lists[0]),
                H(root),
                I(0),
                I((1u64 << 32) | 1),
                I(0),
                I(0),
                H(source),
                I(1),
                I(0),
                I(0),
                I(8),
                I(2),
                I(1),
            ],
        );
        guest.barrier(lists[0], root, COPY_DEST, PIXEL_RESOURCE);
        guest.ok(12, &[H(lists[0])]);
        let target = guest.handle(6, &[H(device), I(2), I(1), I(1), I(RENDER_TARGET)]);
        let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
        let cb = guest.handle(5, &[H(device), I(96), I(COPY_DEST)]);
        let pos = guest.bytes(&floats(&[
            [-1., -1., 0., 1.],
            [3., -1., 0., 1.],
            [-1., 3., 0., 1.],
        ]));
        guest.ok(9, &[H(vb), I(0), I(pos), I(48)]);
        let uv = guest.bytes(&floats(&[
            [0.25, 0.5, 0., 1.],
            [0.25, 0.5, 0., 1.],
            [0.25, 0.5, 0., 1.],
            [0.75, 0.5, 0., 1.],
            [0.75, 0.5, 0., 1.],
            [0.75, 0.5, 0., 1.],
        ]));
        guest.ok(9, &[H(cb), I(0), I(uv), I(96)]);
        let pipeline = guest_nested_view_pipeline(&mut guest, device);
        guest.barrier(lists[1], vb, COPY_DEST, VERTEX_BUFFER);
        guest.barrier(lists[1], cb, COPY_DEST, VERTEX_BUFFER);
        guest.targets(lists[1], &[target]);
        guest.ok(17, &[H(lists[1]), I(2), I(1)]);
        guest.ok(14, &[H(lists[1]), H(pipeline)]);
        guest.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
        let table = sampled_table(&mut guest, [Some(view), None], [Some(0), None], false);
        guest.ok(39, &[H(lists[1]), I(table)]);
        for x in 0..2 {
            guest.ok(19, &[H(lists[1]), H(cb), I(x * 48), I(16)]);
            guest.ok(
                22,
                &[
                    H(lists[1]),
                    I(0),
                    I(0),
                    I(0),
                    I(15),
                    I(0),
                    I(0),
                    I(1),
                    I(x),
                    I(0),
                    I(1),
                    I(1),
                ],
            );
            guest.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
        }
        guest.barrier(lists[1], target, RENDER_TARGET, COPY_SOURCE);
        guest.ok(12, &[H(lists[1])]);
        let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
        let point = guest.call(36, &[H(a)], 0, Some(1));
        let wait = guest.bytes(&[0; 16]);
        guest.copy_result(queues[0], wait);
        guest.copy_result(point, wait + 8);
        let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
        for i in 0..2 {
            guest.ok(11, &[H(lists[i]), H(allocators[i])]);
        }
        guest.ok(38, &[H(view)]);
        for r in [root, source, vb, cb] {
            guest.ok(26, &[H(r)]);
        }
        guest.ok(27, &[H(pipeline)]);
        for receipt in if producer_first { [a, b] } else { [b, a] } {
            guest.call(32, &[H(receipt)], 0, Some(1));
        }
        guest.ok(33, &[H(a)]);
        guest.ok(33, &[H(b)]);
        let out = guest.bytes(&[0xa5; 24]);
        guest.ok(10, &[H(target), I(0), I(out + 8), I(8)]);
        let expected = vec![17, 53, 127, 193, 29, 71, 131, 197];
        check_original_guest_backend(guest, true, true, &[(out, expected.clone())], 1, &expected);
    }
}

// (guest format code, storage unit bytes, full two-layer bytes, source offset,
// destination offset, literal selected texel/block). No layout helper oracle.
fn guest_typed_copy_cases() -> Vec<(u64, usize, usize, usize, usize, Vec<u8>)> {
    vec![
        (0, 4, 144, 64, 140, vec![17, 53, 127, 193]),
        (1, 8, 64, 16, 48, vec![0, 248, 224, 7, 228, 27, 177, 78]),
        (
            2,
            16,
            128,
            32,
            96,
            vec![
                231, 17, 136, 198, 250, 136, 198, 250, 31, 0, 0, 248, 228, 27, 177, 78,
            ],
        ),
        (3, 1, 36, 16, 35, vec![127]),
        (4, 2, 72, 32, 70, vec![17, 193]),
        (5, 4, 144, 64, 140, vec![17, 53, 127, 193]),
        (6, 8, 288, 128, 280, vec![0, 188, 0, 64, 0, 128, 53, 126]),
        (7, 4, 144, 64, 140, vec![69, 35, 193, 127]),
        (8, 4, 144, 64, 140, vec![1, 0, 0, 1]),
        (9, 4, 144, 64, 140, vec![69, 35, 193, 127]),
    ]
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_transfers_texture_to_texture() {
    guest_typed_copy_pair(0);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_transfers_buffer_to_texture() {
    guest_typed_copy_pair(1);
}
#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_transfers_texture_to_buffer() {
    guest_typed_copy_pair(2);
}
fn guest_typed_copy_pair(direction: u8) {
    for (format, unit, size, source_offset, destination_offset, pattern) in guest_typed_copy_cases()
    {
        for pending_source in [false, true] {
            for producer_first in [false, true] {
                let compressed = format == 1 || format == 2;
                let mut source_bytes = vec![0x39; if direction == 1 { 2 * unit } else { size }];
                let start = if direction == 1 { unit } else { source_offset };
                source_bytes[start..start + unit].copy_from_slice(&pattern);
                let prior = vec![0xa5; if direction == 2 { 2 * unit + 1 } else { size }];
                let mut expected = prior.clone();
                let patch = if direction == 2 {
                    unit
                } else {
                    destination_offset
                };
                expected[patch..patch + unit].copy_from_slice(&pattern);
                let mut guest = GuestScript::new();
                let device = guest.handle(1, &[]);
                let queues = [guest.handle(2, &[H(device)]), guest.handle(2, &[H(device)])];
                let allocators = [guest.handle(3, &[H(device)]), guest.handle(3, &[H(device)])];
                let lists = [
                    guest.handle(4, &[H(device), H(allocators[0])]),
                    guest.handle(4, &[H(device), H(allocators[1])]),
                ];
                let create = |g: &mut GuestScript, buffer: bool, len: usize| {
                    if buffer {
                        g.handle(5, &[H(device), I(len as u64), I(COPY_DEST)])
                    } else {
                        g.handle(
                            53,
                            &[H(device), I(5), I(3), I(3), I(2), I(format), I(COPY_DEST)],
                        )
                    }
                };
                let source = create(&mut guest, direction == 1, source_bytes.len());
                let destination = create(&mut guest, direction == 2, prior.len());
                let (seed_bytes, seed_buffer) = if pending_source {
                    (&source_bytes, direction == 1)
                } else {
                    (&prior, direction == 2)
                };
                let seed = create(&mut guest, seed_buffer, seed_bytes.len());
                let changed = if pending_source { source } else { destination };
                let upload = guest.bytes(seed_bytes);
                guest.ok(9, &[H(seed), I(0), I(upload), I(seed_bytes.len() as u64)]);
                let mut poisoned = seed_bytes.clone();
                // Producer changes all bytes for buffers, and a whole mip1 for textures.
                // Destination poison is in mip0, outside the consumer's mip1/mip2 patch.
                let seed_mip = if pending_source { 1 } else { 0 };
                let seed_slice = if pending_source { 0 } else { 1 };
                let (range_start, range_len) = if seed_buffer {
                    (0, seed_bytes.len())
                } else if pending_source {
                    (
                        if compressed {
                            if format == 1 { 16 } else { 32 }
                        } else {
                            15 * unit
                        },
                        if compressed { unit } else { 2 * unit },
                    )
                } else {
                    (size / 2, if compressed { 2 * unit } else { 15 * unit })
                };
                poisoned[range_start..range_start + range_len].fill(0xee);
                let initial = guest.bytes(&poisoned);
                guest.ok(9, &[H(changed), I(0), I(initial), I(poisoned.len() as u64)]);
                let (host, bytes) = if pending_source {
                    (destination, &prior)
                } else {
                    (source, &source_bytes)
                };
                let upload = guest.bytes(bytes);
                guest.ok(9, &[H(host), I(0), I(upload), I(bytes.len() as u64)]);
                guest.barrier(lists[0], seed, COPY_DEST, COPY_SOURCE);
                if seed_buffer {
                    guest.ok(42, &[H(lists[0]), H(changed), H(seed)]);
                } else {
                    let sub = (seed_slice << 32) | seed_mip;
                    guest.ok(
                        54,
                        &[
                            H(lists[0]),
                            H(changed),
                            I(0),
                            I(sub),
                            I(0),
                            I(0),
                            H(seed),
                            I(0),
                            I(sub),
                            I(0),
                            I(0),
                            I(if pending_source { 2 } else { 5 }),
                            I(if pending_source { 1 } else { 3 }),
                        ],
                    );
                }
                guest.barrier(
                    lists[usize::from(!pending_source)],
                    source,
                    COPY_DEST,
                    COPY_SOURCE,
                );
                guest.ok(12, &[H(lists[0])]);
                guest.ok(
                    54,
                    &[
                        H(lists[1]),
                        H(destination),
                        I(u64::from(direction == 2)),
                        I(if direction == 2 {
                            0
                        } else {
                            (1u64 << 32) | if compressed { 1 } else { 2 }
                        }),
                        I(if direction == 2 { unit as u64 } else { 0 }),
                        I(if direction == 2 { (3 * unit) as u64 } else { 0 }),
                        H(source),
                        I(u64::from(direction == 1)),
                        I(if direction == 1 { 0 } else { 1 }),
                        I(if direction == 1 {
                            unit as u64
                        } else {
                            u64::from(!compressed)
                        }),
                        I(if direction == 1 { (3 * unit) as u64 } else { 0 }),
                        I(if compressed { 2 } else { 1 }),
                        I(1),
                    ],
                );
                guest.barrier(lists[1], destination, COPY_DEST, COPY_SOURCE);
                guest.ok(12, &[H(lists[1])]);
                let a = guest.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
                let point = guest.call(36, &[H(a)], 0, Some(1));
                let waits = guest.bytes(&[0; 16]);
                guest.copy_result(queues[0], waits);
                guest.copy_result(point, waits + 8);
                let b = guest.handle(35, &[H(queues[1]), H(lists[1]), I(waits), I(1)]);
                for i in 0..2 {
                    guest.ok(11, &[H(lists[i]), H(allocators[i])]);
                }
                guest.ok(26, &[H(seed)]);
                guest.ok(26, &[H(source)]);
                for receipt in if producer_first { [a, b] } else { [b, a] } {
                    guest.call(32, &[H(receipt)], 0, Some(1));
                }
                guest.ok(33, &[H(a)]);
                guest.ok(33, &[H(b)]);
                let output = guest.bytes(&vec![0xa5; expected.len() + 16]);
                guest.ok(
                    10,
                    &[
                        H(destination),
                        I(0),
                        I(output + 8),
                        I(expected.len() as u64),
                    ],
                );
                check_original_guest_backend(guest, true, true, &[(output, expected)], 0, &[]);
            }
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_roles_sample_matrix_three_adapters() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        for array in [false, true] {
            let mut guest = GuestScript::new();
            let device = guest.handle(1, &[]);
            let queue = guest.handle(2, &[H(device)]);
            let producer_queue = if queued {
                Some(guest.handle(2, &[H(device)]))
            } else {
                None
            };
            let allocator = guest.handle(3, &[H(device)]);
            let list = guest.handle(4, &[H(device), H(allocator)]);
            let vb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
            let cb = guest.handle(5, &[H(device), I(48), I(COPY_DEST)]);
            let positions = guest.bytes(&floats(&[
                [-1., -1., 0.5, 1.],
                [3., -1., 0.5, 1.],
                [-1., 3., 0.5, 1.],
            ]));
            let uv = guest.bytes(&floats(&[[0.5, 0.5, 0., 1.]; 3]));
            guest.ok(9, &[H(vb), I(0), I(positions), I(48)]);
            guest.ok(9, &[H(cb), I(0), I(uv), I(48)]);
            let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
            let ps = include_bytes!("../../../fixtures/rdna2/x9-view-sample-ps.bin");
            let va = guest.bytes(vs);
            let pa = guest.bytes(ps);
            let mut readbacks = Vec::new();
            for (index, (format, view_code, width, bytes, types, expected)) in [
                (0, 0, 1, vec![17, 53, 127, 193], 0, vec![17, 53, 127, 193]),
                (1, 2, 4, vec![255, 255, 0, 0, 0, 0, 0, 0], 0, vec![255; 4]),
                (
                    2,
                    4,
                    4,
                    vec![17, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 0, 0],
                    0,
                    vec![255, 255, 255, 17],
                ),
                (3, 6, 1, vec![128], 0, vec![128, 0, 0, 255]),
                (4, 7, 1, vec![17, 193], 0, vec![17, 193, 0, 255]),
                (5, 8, 1, vec![17, 53, 127, 193], 0, vec![127, 53, 17, 193]),
                (
                    6,
                    10,
                    1,
                    vec![0, 52, 0, 56, 0, 58, 0, 60],
                    0,
                    vec![64, 128, 191, 255],
                ),
                (7, 11, 1, vec![0, 0, 128, 62], 0, vec![64, 0, 0, 255]),
                (
                    8,
                    12,
                    1,
                    vec![69, 35, 193, 127],
                    257,
                    vec![69, 35, 193, 127],
                ),
                (9, 13, 1, vec![0, 0, 64, 63], 2, vec![191, 0, 0, 255]),
            ]
            .into_iter()
            .enumerate()
            {
                eprintln!(
                    "typed sample adapter async={asynchronous} queued={queued} array={array} format={format} first_call={}",
                    guest.calls.len()
                );
                let root_width = if array { width.max(2) } else { width };
                let mut packed = if array {
                    vec![0; bytes.len() * if width == 4 { 4 } else { 10 }]
                } else {
                    bytes.clone()
                };
                if array {
                    let offset = packed.len() - bytes.len();
                    packed[offset..].copy_from_slice(&bytes);
                }
                let texture = guest.handle(
                    53,
                    &[
                        H(device),
                        I(root_width),
                        I(root_width),
                        I(if array { 2 } else { 1 }),
                        I(if array { 2 } else { 0 }),
                        I(format),
                        I(COPY_DEST),
                    ],
                );
                let data = guest.bytes(&packed);
                let producer = if queued {
                    let source = guest.handle(
                        53,
                        &[
                            H(device),
                            I(root_width),
                            I(root_width),
                            I(if array { 2 } else { 1 }),
                            I(if array { 2 } else { 0 }),
                            I(format),
                            I(COPY_DEST),
                        ],
                    );
                    guest.ok(9, &[H(source), I(0), I(data), I(packed.len() as u64)]);
                    let poison = guest.bytes(&vec![0; packed.len()]);
                    guest.ok(9, &[H(texture), I(0), I(poison), I(packed.len() as u64)]);
                    let pq = producer_queue.unwrap();
                    let alloc = guest.handle(3, &[H(device)]);
                    let pl = guest.handle(4, &[H(device), H(alloc)]);
                    guest.barrier(pl, source, COPY_DEST, COPY_SOURCE);
                    let sub = if array { (1u64 << 32) | 1 } else { 0 };
                    let span = if array { root_width / 2 } else { root_width };
                    guest.ok(
                        54,
                        &[
                            H(pl),
                            H(texture),
                            I(0),
                            I(sub),
                            I(0),
                            I(0),
                            H(source),
                            I(0),
                            I(sub),
                            I(0),
                            I(0),
                            I(span),
                            I(span),
                        ],
                    );
                    guest.barrier(pl, texture, COPY_DEST, PIXEL_RESOURCE);
                    guest.ok(12, &[H(pl)]);
                    let receipt = guest.handle(35, &[H(pq), H(pl), I(0), I(0)]);
                    let point = guest.call(36, &[H(receipt)], 0, Some(index as u64 + 1));
                    let wait = guest.bytes(&[0; 16]);
                    guest.copy_result(pq, wait);
                    guest.copy_result(point, wait + 8);
                    guest.ok(11, &[H(pl), H(alloc)]);
                    guest.ok(26, &[H(source)]);
                    Some((receipt, wait))
                } else {
                    guest.ok(9, &[H(texture), I(0), I(data), I(packed.len() as u64)]);
                    None
                };
                let view = if array {
                    guest.handle(52, &[H(device), H(texture), I(1), I(1), I(1), I(view_code)])
                } else {
                    guest.handle(37, &[H(device), H(texture), I(0), I(1), I(view_code)])
                };
                let pipeline = guest.handle(
                    55,
                    &[
                        H(device),
                        I(va),
                        I(vs.len() as u64),
                        I(pa),
                        I(ps.len() as u64),
                        I(types),
                    ],
                );
                let target = guest.handle(
                    53,
                    &[
                        H(device),
                        I(1),
                        I(1),
                        I(1),
                        I(0),
                        I(if types == 257 { 8 } else { 0 }),
                        I(RENDER_TARGET),
                    ],
                );
                let mut record = [0u64; 8];
                record[3] = u64::from(types == 257);
                let records = guest.bytes(
                    &record
                        .into_iter()
                        .flat_map(u64::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
                guest.copy_result(target, records);
                guest.ok(56, &[H(list), I(1), I(records)]);
                if index == 0 {
                    guest.barrier(list, vb, COPY_DEST, VERTEX_BUFFER);
                    guest.barrier(list, cb, COPY_DEST, VERTEX_BUFFER);
                }
                if !queued {
                    guest.barrier(list, texture, COPY_DEST, PIXEL_RESOURCE);
                }
                guest.ok(14, &[H(list), H(pipeline)]);
                guest.ok(17, &[H(list), I(1), I(1)]);
                guest.ok(18, &[H(list), H(vb), I(0), I(16)]);
                guest.ok(19, &[H(list), H(cb), I(0), I(16)]);
                let table = sampled_table(&mut guest, [Some(view), None], [Some(0), None], false);
                guest.ok(39, &[H(list), I(table)]);
                guest.ok(23, &[H(list), I(3), I(1), I(0), I(0)]);
                guest.barrier(list, target, RENDER_TARGET, COPY_SOURCE);
                if let Some((producer, wait)) = producer {
                    guest.ok(12, &[H(list)]);
                    let consumer = guest.handle(35, &[H(queue), H(list), I(wait), I(1)]);
                    guest.call(32, &[H(consumer)], 0, Some(1));
                    guest.call(32, &[H(producer)], 0, Some(1));
                    guest.ok(33, &[H(consumer)]);
                    guest.ok(33, &[H(producer)]);
                } else {
                    finish_guest_submission(&mut guest, queue, list, asynchronous, 1);
                }
                guest.ok(11, &[H(list), H(allocator)]);
                guest.ok(38, &[H(view)]);
                guest.ok(26, &[H(texture)]);
                guest.ok(27, &[H(pipeline)]);
                let output = guest.bytes(&[0xa5; 20]);
                guest.ok(10, &[H(target), I(0), I(output + 8), I(4)]);
                readbacks.push((output, expected));
            }
            eprintln!("executing typed sample async={asynchronous} queued={queued} array={array}");
            check_original_guest_backend(
                guest,
                asynchronous,
                queued,
                &readbacks,
                9,
                &[191, 0, 0, 255],
            );
        }
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_roles_seven_attachments_depth_write_reject() {
    for (asynchronous, queued) in [(false, false), (true, false), (true, true)] {
        let mut g = GuestScript::new();
        let d = g.handle(1, &[]);
        let q = g.handle(2, &[H(d)]);
        let a = g.handle(3, &[H(d)]);
        let l = g.handle(4, &[H(d), H(a)]);
        let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
        let cb = g.handle(5, &[H(d), I(96), I(COPY_DEST)]);
        let pos = g.bytes(&floats(&[
            [-1., -1., 0.5, 1.],
            [3., -1., 0.5, 1.],
            [-1., 3., 0.5, 1.],
        ]));
        let mut colors = vec![[0.25, 0.5, 0.75, 1.]; 3];
        colors.extend([[1., 0., 0., 1.]; 3]);
        let ca = g.bytes(&floats(&colors));
        g.ok(9, &[H(vb), I(0), I(pos), I(48)]);
        g.ok(9, &[H(cb), I(0), I(ca), I(96)]);
        let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
        let ps = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
        let va = g.bytes(vs);
        let pa = g.bytes(ps);
        let mut reads = Vec::new();
        for (i, (format, expected)) in [
            (3, vec![64]),
            (4, vec![64, 128]),
            (0, vec![64, 128, 191, 255]),
            (5, vec![191, 128, 64, 255]),
            (6, vec![0, 52, 0, 56, 0, 58, 0, 60]),
            (7, vec![0, 0, 128, 62]),
            (8, vec![0, 0, 128, 62]),
        ]
        .into_iter()
        .enumerate()
        {
            let t = g.handle(
                53,
                &[H(d), I(1), I(1), I(1), I(0), I(format), I(RENDER_TARGET)],
            );
            let z = g.handle(7, &[H(d), I(1), I(1)]);
            let pipeline = g.handle(
                55,
                &[
                    H(d),
                    I(va),
                    I(vs.len() as u64),
                    I(pa),
                    I(ps.len() as u64),
                    I(if format == 8 { 256 } else { 0 }),
                ],
            );
            let mut words = [0u64; 8];
            words[3] = u64::from(format == 8);
            let records = g.bytes(
                &words
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            );
            g.copy_result(t, records);
            g.ok(56, &[H(l), I(1), I(records)]);
            g.ok(16, &[H(l), H(z), I(0), I(u64::from(1f32.to_bits()))]);
            if i == 0 {
                g.barrier(l, vb, COPY_DEST, VERTEX_BUFFER);
                g.barrier(l, cb, COPY_DEST, VERTEX_BUFFER);
            }
            g.ok(14, &[H(l), H(pipeline)]);
            g.ok(17, &[H(l), I(1), I(1)]);
            g.ok(18, &[H(l), H(vb), I(0), I(16)]);
            g.ok(
                22,
                &[
                    H(l),
                    I(2),
                    I(1),
                    I(0),
                    I(15),
                    I(0),
                    I(0),
                    I(0),
                    I(0),
                    I(0),
                    I(0),
                    I(0),
                ],
            );
            for offset in [0, 48] {
                g.ok(19, &[H(l), H(cb), I(offset), I(16)]);
                g.ok(23, &[H(l), I(3), I(1), I(0), I(0)]);
            }
            g.barrier(l, t, RENDER_TARGET, COPY_SOURCE);
            finish_guest_submission(&mut g, q, l, asynchronous, 2);
            g.ok(11, &[H(l), H(a)]);
            g.ok(27, &[H(pipeline)]);
            g.ok(26, &[H(z)]);
            let out = g.bytes(&vec![0xa5; expected.len() + 16]);
            g.ok(10, &[H(t), I(0), I(out + 8), I(expected.len() as u64)]);
            reads.push((out, expected));
        }
        check_original_guest_backend(g, asynchronous, queued, &reads, 1, &[64, 128, 191, 255]);
    }
}

#[test]
#[cfg_attr(not(feature = "gpu-tests"), ignore = "requires native Metal")]
fn original_guest_typed_roles_pending_load_and_rendered_depth_sample() {
    let mut g = GuestScript::new();
    let d = g.handle(1, &[]);
    let queues = [g.handle(2, &[H(d)]), g.handle(2, &[H(d)])];
    let alloc = [g.handle(3, &[H(d)]), g.handle(3, &[H(d)])];
    let lists = [
        g.handle(4, &[H(d), H(alloc[0])]),
        g.handle(4, &[H(d), H(alloc[1])]),
    ];
    let color = g.handle(53, &[H(d), I(2), I(1), I(1), I(0), I(7), I(RENDER_TARGET)]);
    let rgba = g.handle(6, &[H(d), I(2), I(1), I(1), I(RENDER_TARGET)]);
    let depth = g.handle(7, &[H(d), I(2), I(1)]);
    let view = g.handle(37, &[H(d), H(depth), I(0), I(1), I(13)]);
    let vb = g.handle(5, &[H(d), I(48), I(COPY_DEST)]);
    let cb = g.handle(5, &[H(d), I(96), I(COPY_DEST)]);
    let positions = g.bytes(&floats(&[
        [-1., -1., 0.25, 1.],
        [3., -1., 0.25, 1.],
        [-1., 3., 0.25, 1.],
    ]));
    let mut values = vec![[0.5, 0., 0., 1.]; 3];
    values.extend([[0.75, 0.5, 0., 1.]; 3]);
    let values = g.bytes(&floats(&values));
    g.ok(9, &[H(vb), I(0), I(positions), I(48)]);
    g.ok(9, &[H(cb), I(0), I(values), I(96)]);
    let vs = include_bytes!("../../../fixtures/rdna2/x9-varying-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-varying-ps.bin");
    let sample = include_bytes!("../../../fixtures/rdna2/x9-scene-sampled-mrt-ps.bin");
    let va = g.bytes(vs);
    let pa = g.bytes(ps);
    let sa = g.bytes(sample);
    let producer_pipeline = g.handle(
        55,
        &[
            H(d),
            I(va),
            I(vs.len() as u64),
            I(pa),
            I(ps.len() as u64),
            I(0),
        ],
    );
    let consumer_pipeline = g.handle(
        55,
        &[
            H(d),
            I(va),
            I(vs.len() as u64),
            I(sa),
            I(sample.len() as u64),
            I(2),
        ],
    );
    let clear = [0u64, 0, 0, 0, 0.125f64.to_bits(), 0, 0, 0];
    let records = g.bytes(
        &clear
            .into_iter()
            .flat_map(u64::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    g.copy_result(color, records);
    g.ok(56, &[H(lists[0]), I(1), I(records)]);
    g.ok(
        16,
        &[H(lists[0]), H(depth), I(0), I(u64::from(1f32.to_bits()))],
    );
    g.barrier(lists[0], vb, COPY_DEST, VERTEX_BUFFER);
    g.barrier(lists[0], cb, COPY_DEST, VERTEX_BUFFER);
    g.ok(14, &[H(lists[0]), H(producer_pipeline)]);
    g.ok(17, &[H(lists[0]), I(2), I(1)]);
    g.ok(18, &[H(lists[0]), H(vb), I(0), I(16)]);
    g.ok(19, &[H(lists[0]), H(cb), I(0), I(16)]);
    g.ok(
        22,
        &[
            H(lists[0]),
            I(2),
            I(1),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(1),
            I(0),
            I(1),
            I(1),
        ],
    );
    g.ok(23, &[H(lists[0]), I(3), I(1), I(0), I(0)]);
    g.barrier(lists[0], depth, 0x10, PIXEL_RESOURCE);
    g.ok(12, &[H(lists[0])]);
    let records = g.bytes(&[0; 128]);
    g.copy_result(rgba, records);
    g.copy_result(color, records + 64);
    // Second attachment loads the producer version. MRT1 is BRGA, whose red
    // channel is the depth sample's missing blue channel, exactly zero.
    let load = g.bytes(&1u64.to_le_bytes());
    g.copy_result(load, records + 72);
    g.ok(56, &[H(lists[1]), I(2), I(records)]);
    g.ok(14, &[H(lists[1]), H(consumer_pipeline)]);
    g.ok(17, &[H(lists[1]), I(2), I(1)]);
    g.ok(18, &[H(lists[1]), H(vb), I(0), I(16)]);
    g.ok(19, &[H(lists[1]), H(cb), I(48), I(16)]);
    let table = sampled_table(&mut g, [Some(view), None], [Some(0), None], false);
    g.ok(39, &[H(lists[1]), I(table)]);
    g.ok(
        22,
        &[
            H(lists[1]),
            I(0),
            I(0),
            I(0),
            I(15),
            I(0),
            I(0),
            I(1),
            I(1),
            I(0),
            I(1),
            I(1),
        ],
    );
    g.ok(23, &[H(lists[1]), I(3), I(1), I(0), I(0)]);
    g.barrier(lists[1], color, RENDER_TARGET, COPY_SOURCE);
    g.barrier(lists[1], rgba, RENDER_TARGET, COPY_SOURCE);
    g.ok(12, &[H(lists[1])]);
    let a = g.handle(35, &[H(queues[0]), H(lists[0]), I(0), I(0)]);
    let point = g.call(36, &[H(a)], 0, Some(1));
    let wait = g.bytes(&[0; 16]);
    g.copy_result(queues[0], wait);
    g.copy_result(point, wait + 8);
    let b = g.handle(35, &[H(queues[1]), H(lists[1]), I(wait), I(1)]);
    for i in 0..2 {
        g.ok(11, &[H(lists[i]), H(alloc[i])]);
    }
    g.ok(38, &[H(view)]);
    for r in [depth, vb, cb] {
        g.ok(26, &[H(r)]);
    }
    for p in [producer_pipeline, consumer_pipeline] {
        g.ok(27, &[H(p)]);
    }
    g.call(32, &[H(b)], 0, Some(1));
    g.call(32, &[H(a)], 0, Some(1));
    g.ok(33, &[H(b)]);
    g.ok(33, &[H(a)]);
    let out_color = g.bytes(&[0xa5; 24]);
    let out_rgba = g.bytes(&[0xa5; 24]);
    g.ok(10, &[H(color), I(0), I(out_color + 8), I(8)]);
    g.ok(10, &[H(rgba), I(0), I(out_rgba + 8), I(8)]);
    let pixels = vec![0, 0, 0, 0, 64, 0, 0, 255];
    check_original_guest_backend(
        g,
        true,
        true,
        &[
            (out_color, vec![0, 0, 0, 62, 0, 0, 0, 0]),
            (out_rgba, pixels.clone()),
        ],
        1,
        &pixels,
    );
}

#[path = "guest_graphics/attachment_transactions.rs"]
mod attachment_transactions;

#[path = "guest_graphics/attachment_composition.rs"]
mod attachment_composition;

#[path = "guest_graphics/array_sampling.rs"]
mod array_sampling;
#[path = "guest_graphics/storage_views.rs"]
mod storage_views;
