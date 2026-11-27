// SPDX-License-Identifier: GPL-3.0-or-later
//! Original expectations: AMD RDNA2 ISA 70648 section12.12 printed p184,
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! Opcode865 writes one VGPR lane selected by low five scalar bits and ignores
//! EXEC. Retain other lanes; absent destination has no present-lane effect under
//! this approved ABI. LLVM23.1.6 native ARM64 gfx1030: v1,s8,s9 encodes
//! [01,00,61,d7,08,12,00,02]. No production evaluator supplies expected values.
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const WAIT: u32 = 0xbf8c3f70;
const END: u32 = 0xbf810000;
const LENGTHS: [usize; 9] = [1, 15, 16, 17, 31, 32, 33, 63, 65];
const INDICES: [u32; 8] = [0, 1, 15, 16, 31, 32, 63, u32::MAX];
const FIXTURE: &[u8] = include_bytes!("../../../fixtures/rdna2/lane-write.bin");
fn write(dst: u32, data: u32, index: u32) -> [u32; 2] {
    [0xd7610000 | dst, 0x02000000 | data | index << 9]
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}
fn finish(mut w: Vec<u32>, dst: u32) -> Vec<u8> {
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, END]);
    bytes(&w)
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| [0x80000001, 0xffffffff, 0x7fffffff][i / 32 % 3] ^ ((i % 32) as u32 * 0x10203))
        .collect()
}
fn broadcast(data: &[u32]) -> Vec<u32> {
    data.chunks(32).flat_map(|w| vec![w[0]; w.len()]).collect()
}
fn changed(data: &[u32], index: u32, constant: Option<u32>) -> Vec<u32> {
    data.chunks(32)
        .flat_map(|wave| {
            wave.iter().enumerate().map(move |(lane, &old)| {
                if lane == (index % 32) as usize {
                    constant.unwrap_or(wave[0])
                } else {
                    old
                }
            })
        })
        .collect()
}
fn equal_masks(a: &[u32], b: &[u32]) -> Vec<u32> {
    a.chunks(32)
        .zip(b.chunks(32))
        .flat_map(|(a, b)| {
            let mask = a
                .iter()
                .zip(b)
                .enumerate()
                .fold(0, |m, (lane, (x, y))| m | (u32::from(x == y) << lane));
            vec![mask; a.len()]
        })
        .collect()
}
fn base(index: u32, constant: Option<u32>, exec: u32, dst: u32) -> Vec<u32> {
    let mut w = LOAD.to_vec();
    w.extend([WAIT, 0x7e080301]); // original input in v4
    if let Some(value) = constant {
        w.extend([0xbe8803ff, value]);
    } else {
        w.extend([0x7e040301, 0x7e100502, 0x7e040289]);
    } // capture v2 then overwrite it
    if dst != 1 {
        w.push(0x7e000301 | dst << 17);
    }
    w.extend([0xbe8903ff, index, 0xbefe03ff, exec]);
    w.extend(write(dst, 8, 9));
    w.push(0xbefe03c1);
    w
}
fn cases() -> Vec<(Vec<u8>, Vec<u32>, Vec<u32>)> {
    let mut cases = Vec::new();
    for len in LENGTHS {
        let data = input(len);
        for (n, index) in INDICES.into_iter().enumerate() {
            let constant = [0, 0x80000000, u32::MAX][n % 3];
            let dst = if index == 63 { 255 } else { 1 };
            for exec in [0, 0x80000002, u32::MAX] {
                cases.push((
                    finish(base(index, Some(constant), exec, dst), dst),
                    data.clone(),
                    changed(&data, index, Some(constant)),
                ));
            }
            cases.push((
                finish(base(index, None, 0x55555555, 1), 1),
                data.clone(),
                changed(&data, index, None),
            ));
            for value in [Some(constant), None] {
                let mut w = base(index, value, 0, dst);
                w.extend([0x7d840900 | dst, 0x7e06026a]); // compare written VGPR against saved v4
                let want = equal_masks(&changed(&data, index, value), &data);
                cases.push((finish(w, 3), data.clone(), want));
            }
        }
        // Read/write/read snapshot chain, then change both source SGPR and VGPR.
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0x7e100501, 0x7e040287, 0xbe890380]);
        w.extend(write(2, 8, 9));
        w.push(0x7e140502); // capture newly written v2 lane0 into s10
        w.extend([0xbe88038d, 0x7e020289]);
        w.extend(write(2, 8, 9));
        w.extend([0x7e06020a]);
        cases.push((finish(w.clone(), 3), data.clone(), broadcast(&data)));
        let want = (0..len).map(|i| if i % 32 == 0 { 13 } else { 7 }).collect();
        cases.push((finish(w.clone(), 2), data.clone(), want));
        w.push(0x7e140502);
        w.push(0x7e06020a);
        cases.push((finish(w, 3), data.clone(), vec![13; len]));
        // Last write wins only in its selected lane; other writes persist.
        for index in [0, 31] {
            let mut w = base(0, Some(11), 0, 1);
            w.extend([0xbe88038d, 0xbe8903ff, index, 0xbefe0380]);
            w.extend(write(1, 8, 9));
            w.push(0xbefe03c1);
            let mut want = changed(&data, 0, Some(11));
            for (i, v) in want.iter_mut().enumerate() {
                if i % 32 == index as usize {
                    *v = 13;
                }
            }
            cases.push((finish(w, 1), data.clone(), want));
        }
        // A write expression feeds a runtime branch join and then comparison replay.
        for taken in [false, true] {
            let mut w = base(0, Some(0), 0, 1);
            w.extend([
                0x7e040280,
                if taken { 0x7d840280 } else { 0x7d840281 },
                0xbf870001,
                0x7e020289,
                0x7d840501,
                0x7e06026a,
            ]);
            let want = data
                .chunks(32)
                .flat_map(|wave| vec![u32::from(taken); wave.len()])
                .collect();
            cases.push((finish(w, 3), data.clone(), want));
        }
        let want = data
            .chunks(32)
            .flat_map(|wave| (0..wave.len()).map(move |i| if i == 0 { wave[0] } else { 7 }))
            .collect();
        cases.push((FIXTURE.to_vec(), data, want));
    }
    cases
}
fn check(code: &[u8], data: &[u32], want: &[u32]) {
    xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap();
    assert_eq!(xenora_ref::execute(code, data).unwrap(), want);
}
fn rejects(code: &[u8], at: usize) {
    match xenora_shader::compile(xenora_shader::Architecture::Gfx1030, code).unwrap_err() {
        xenora_shader::CompileError::Decode(e) => assert_eq!(e.offset, at),
        xenora_shader::CompileError::Verify { offset, .. } => assert_eq!(offset, at),
        e => panic!("unexpected rejection {e:?}"),
    }
    assert_eq!(
        xenora_ref::execute(code, &[0x80000001; 33])
            .unwrap_err()
            .offset,
        at
    );
}
#[test]
fn llvm_words_and_fixture() {
    assert_eq!(write(1, 8, 9), [0xd7610001, 0x02001208]);
    assert_eq!(bytes(&write(1, 8, 9)), [1, 0, 0x61, 0xd7, 8, 0x12, 0, 2]);
    let mut w = LOAD.to_vec();
    w.extend([
        WAIT, 0x7e100501, 0x7e020289, 0x7e040287, 0xbe8903a0, 0xbefe0380,
    ]);
    w.extend(write(2, 8, 9));
    w.push(0xbefe03c1);
    assert_eq!(FIXTURE, finish(w, 2));
}
#[test]
fn independent_values_preservation_snapshots_and_replay() {
    assert_eq!(cases().len(), 504);
    for (code, data, want) in cases() {
        check(&code, &data, &want);
    }
}
#[test]
fn undefined_pending_destinations_and_scalar_sources_reject() {
    for exec in [0x80, 0xc1] {
        let mut w = vec![0xbe88038d, 0xbe890380, 0xbefe0300 | exec];
        w.extend(write(1, 8, 9));
        w.push(0xbefe03c1);
        rejects(&finish(w, 1), 12);
        // Existing destination content cannot make a pending load safe to modify.
        let mut w = vec![0x7e020287, 0xbe88038d, 0xbe890380];
        w.extend(LOAD);
        w.push(0xbefe0300 | exec);
        w.extend(write(1, 8, 9));
        w.extend([0xbefe03c1, WAIT]);
        rejects(&finish(w, 1), 24);
    }
    for missing in [8, 9] {
        let mut w = LOAD.to_vec();
        w.push(WAIT);
        w.push(if missing == 8 { 0xbe890380 } else { 0xbe88038d });
        w.extend(write(1, 8, 9));
        rejects(&finish(w, 1), 16);
    }
    let mut w = LOAD.to_vec();
    w.extend([WAIT, 0xbe88038d, 0x7e120501]);
    w.extend(write(1, 8, 9));
    rejects(&finish(w, 1), 20); // runtime index forbidden
    let mut w = LOAD.to_vec();
    w.extend([WAIT, 0xbe88038d, 0xbe890380]);
    w.extend(write(0, 8, 9));
    rejects(&finish(w, 1), 20);
}
#[test]
fn special_inline_vector_sources_and_modifiers_reject() {
    for bad in [
        106, 107, 124, 125, 126, 127, 128, 129, 160, 192, 193, 255, 256, 257, 511,
    ] {
        for src in [[bad, 9], [8, bad]] {
            let mut w = LOAD.to_vec();
            w.extend([WAIT, 0xbe88038d, 0xbe890380]);
            w.extend(write(1, src[0], src[1]));
            rejects(&finish(w, 1), 20);
        }
    }
    for bit in 8..16 {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe88038d, 0xbe890380]);
        let mut op = write(1, 8, 9);
        op[0] |= 1 << bit;
        w.extend(op);
        rejects(&finish(w, 1), 20);
    }
    for bit in 27..32 {
        let mut w = LOAD.to_vec();
        w.extend([WAIT, 0xbe88038d, 0xbe890380]);
        let mut op = write(1, 8, 9);
        op[1] |= 1 << bit;
        w.extend(op);
        rejects(&finish(w, 1), 20);
    }
}
#[test]
fn truncated_encoding_rejects() {
    let code = bytes(&write(1, 8, 9));
    for len in 1..8 {
        assert!(
            xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code[..len]).is_err()
        );
        assert!(xenora_ref::execute(&code[..len], &[0]).is_err());
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_lane_write_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (code, data, want) in cases() {
        let compiled = xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &code).unwrap();
        assert_eq!(
            gpu.run_words(&compiled.msl, xenora_shader::ENTRY_POINT, &data)
                .unwrap(),
            want
        );
    }
}
