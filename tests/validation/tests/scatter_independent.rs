// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent DS_PERMUTE acceptance. No AMD hardware validation.
//! Independent source: public AMD RDNA2 ISA 70648, printed p205, DS_PERMUTE_B32.
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! P205 prose permits any colliding writer; pseudocode/comment names the highest
//! source lane. Membership tests below express the former, separate deterministic
//! policy tests express this project's selected highest-active-writer policy.
//! FullWavesRequired, static EXEC, source/destination snapshots, distinct LGKM wait.
//! DS word0 = 0xd8000000 | (178 << 18) | offset under the agreed gfx1030 layout.
//! Table94 supplies word1 ADDR[7:0], DATA0[15:8], DATA1[23:16], VDST[31:24].
//! Its printed word0 OP/GDS positions disagree with that gfx1030 layout; no native
//! LLVM verification is claimed here. Zero DATA1/reserved bits are the project's
//! admitted subset; encoding assertions below use literal independently derived bytes.
//! No compiler/reference evaluator supplies expected values.

const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const VM: u32 = 0xbf8c3f70;
const LGKM: u32 = 0xbf8cc07f;
const VS: u32 = 0xbbfd0000;
const END: u32 = 0xbf810000;
const SENTINEL: u32 = 0xa55a5aa5;

struct Program {
    name: String,
    code: Vec<u8>,
    input: Vec<u32>,
    highest_policy: Vec<u32>,
    // One candidate for retention/no-writer/unique-writer, several for collision.
    eligible_values: Vec<Vec<u32>>,
}
struct ScatterCase {
    source: Vec<u32>,
    addresses: [u32; 32],
    offset: u16,
    exec: u32,
    dst: u32,
    prior: Vec<u32>,
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| [0x80001000, 0xfffffe00, 0x7fffff00][i / 32] + (i % 32) as u32)
        .collect()
}
fn scatter(dst: u32, address: u32, data: u32, offset: u16) -> [u32; 2] {
    [
        0xdac80000 | u32::from(offset),
        dst << 24 | data << 8 | address,
    ]
}
fn finish(mut words: Vec<u32>, dst: u32) -> Vec<u8> {
    words.extend([0xe0702000, 0x80010000 | dst << 8, VS, END]);
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn address_vector(words: &mut Vec<u32>, addresses: &[u32; 32]) {
    words.push(0x7e040280); // initialize v2 before partial-EXEC updates
    for (lane, &address) in addresses.iter().enumerate() {
        words.extend([0xbefe03ff, 1u32 << lane, 0x7e0402ff, address]);
    }
    words.push(0xbefe03c1);
}
fn body(case: &ScatterCase) -> Vec<u32> {
    let mut words = LOAD.to_vec();
    words.extend([VM, 0x7e0602ff, SENTINEL]);
    address_vector(&mut words, &case.addresses);
    words.extend([0xbefe03ff, case.exec]);
    words.extend(scatter(case.dst, 2, 1, case.offset));
    words.extend([LGKM, 0xbefe03c1]);
    words
}
fn eligible_writers(
    addresses: &[u32; 32],
    offset: u16,
    exec: u32,
    destination: usize,
) -> Vec<usize> {
    // Invert the routing relation: examine candidate writers for this destination.
    // Widen before addition. Discarded high bits make 32-bit overflow immaterial
    // to the low seven address bits used by the selected Wave32 contract.
    addresses
        .iter()
        .enumerate()
        .filter_map(|(writer, &address)| {
            let active = exec & (1u32 << writer) != 0;
            let byte_address = u64::from(address) + u64::from(offset);
            let target = (byte_address / 4) % 32;
            (active && target == destination as u64).then_some(writer)
        })
        .collect()
}
fn answers(case: &ScatterCase) -> (Vec<Vec<u32>>, Vec<u32>) {
    let mut membership = Vec::new();
    let mut policy = Vec::new();
    for (index, &old) in case.prior.iter().enumerate() {
        let lane = index % 32;
        if case.exec & (1u32 << lane) == 0 {
            membership.push(vec![old]);
            policy.push(old);
            continue;
        }
        let writers = eligible_writers(&case.addresses, case.offset, case.exec, lane);
        let wave = index / 32 * 32;
        let values: Vec<_> = writers
            .iter()
            .map(|&writer| case.source[wave + writer])
            .collect();
        // Membership never selects a winner. Highest selection is a separate step.
        membership.push(if values.is_empty() { vec![0] } else { values });
        policy.push(
            writers
                .last()
                .map_or(0, |&writer| case.source[wave + writer]),
        );
    }
    (membership, policy)
}
fn base_cases() -> Vec<ScatterCase> {
    let mut out = Vec::new();
    for pattern in 0..12 {
        for (mask_index, exec) in [u32::MAX, u32::MAX, 0, 0x80000002, 0x55555555, 0x80000000]
            .into_iter()
            .enumerate()
        {
            let source = input([32, 64, 96][(pattern + mask_index) % 3]);
            let offset = [0, 0, 0, 1, 3, 4, 124, 127, 128, 65535, 0, 0][pattern];
            let addresses = std::array::from_fn(|lane| match pattern {
                0 => lane as u32 * 4,              // identity, unique writers
                1 => (31 - lane) as u32 * 4,       // reverse, unique writers
                2 => ((lane + 1) % 32) as u32 * 4, // rotate, unique writers
                3 => lane as u32 * 4 + 3,          // carry from low address bits
                4 => lane as u32 * 4 + 1,
                5 => (lane / 2) as u32 * 4, // paired collisions and holes
                6 => (lane % 4) as u32 * 4, // eight writers per target
                7 => 0,                     // broadcast scatter + offset
                8 => lane as u32 * 4,       // complete-wave wrap
                9 => [u32::MAX, 0xfffffffcu32, 1, 127][lane % 4],
                10 => 0, // highest lane31 wins if active
                _ => [0, 0, 12, 4][lane % 4] + (lane / 4) as u32 * 16,
            });
            let dst = [3, 1, 2][(pattern + mask_index) % 3];
            let prior = match dst {
                1 => source.clone(),
                2 => (0..source.len()).map(|i| addresses[i % 32]).collect(),
                _ => vec![SENTINEL; source.len()],
            };
            out.push(ScatterCase {
                source,
                addresses,
                offset,
                exec,
                dst,
                prior,
            });
        }
    }
    out
}
fn base_programs() -> Vec<Program> {
    base_cases()
        .into_iter()
        .enumerate()
        .map(|(n, case)| {
            let (eligible_values, highest_policy) = answers(&case);
            let mut words = body(&case);
            // Two sequential scatters with independent original data/address are legal.
            // Complete each separately; a repeated LGKM wait is harmless.
            if n == 0 {
                words.extend(scatter(case.dst, 2, 1, case.offset));
                words.extend([LGKM, LGKM]);
            }
            // Copy the result first, then overwrite BOTH original data/address VGPRs.
            // This also covers dst==data and dst==address without destroying output.
            words.extend([0x7e080300 | case.dst, 0x7e020280, 0x7e040280]);
            Program {
                name: format!("scatter {n}"),
                code: finish(words, 4),
                input: case.source,
                highest_policy,
                eligible_values,
            }
        })
        .collect()
}
fn singleton(values: &[u32]) -> Vec<Vec<u32>> {
    values.iter().map(|&value| vec![value]).collect()
}
fn composition_programs() -> Vec<Program> {
    let mut out = Vec::new();
    for len in [32, 64, 96] {
        let data = input(len);
        let case = ScatterCase {
            source: data.clone(),
            addresses: std::array::from_fn(|i| ((i + 1) % 32) as u32 * 4),
            offset: 0,
            exec: u32::MAX,
            dst: 3,
            prior: vec![SENTINEL; len],
        };
        let (_, scattered) = answers(&case);
        for mode in 0..3 {
            // Generate the equivalent rotation compactly: lane*4 + OFFSET4.
            // Literal per-lane address writes expand excessively under VCC replay.
            let mut words = LOAD.to_vec();
            words.extend([VM, 0x7e0602ff, SENTINEL]);
            if mode == 2 {
                // Direct input byte addresses group four consecutive writers.
                // The marker bases are multiples of128, so +4 targets lanes1..8.
                words.extend(scatter(3, 1, 1, 4));
            } else {
                words.push(0x34040282); // input low5 bits = lane, shifted to bytes
                words.extend(scatter(3, 2, 1, 4));
            }
            words.push(LGKM);
            let (dst, highest_policy) = match mode {
                0 => {
                    // Gather destination lane+1 reverses this unique scatter.
                    // MBCNT(-1,0) gives the logical lane; shift to byte units.
                    // Avoid a second 32-lane literal initialization exceeding
                    // the fixture's 1024-byte admission bound.
                    words.extend([0xd7650002, 0x020100c1, 0x34040482]);
                    words.extend([0x7e080280, 0xdacc0004, 0x04000302, LGKM]);
                    (4, data.clone())
                }
                1 => {
                    words.extend([0xbe89039f, 0xd7600008, 0x02001303, 0x7e080208]);
                    let expected = scattered
                        .as_chunks::<32>()
                        .0
                        .iter()
                        .flat_map(|wave| vec![wave[31]; 32])
                        .collect();
                    (4, expected)
                }
                _ => {
                    words.extend([0x7d8406ff, 0x80001003, 0x7e08026a]);
                    // Highest of writers0..3 reaches lane1. Only wave0 contains
                    // this literal marker; later waves have distinct high bits.
                    let expected = (0..len).map(|i| if i < 32 { 2 } else { 0 }).collect();
                    (4, expected)
                }
            };
            words.extend([0x7e020280, 0x7e040280, 0x7e060280]);
            out.push(Program {
                name: format!("composition length{len} mode{mode}"),
                code: finish(words, dst),
                input: data.clone(),
                eligible_values: singleton(&highest_policy),
                highest_policy,
            });
        }
    }
    out
}
fn gather_scatter_gather_programs() -> Vec<Program> {
    let mut out = Vec::new();
    for len in [32, 64, 96] {
        for gather_address in [false, true] {
            let data = input(len);
            let mut w = LOAD.to_vec();
            w.extend([VM, 0x7e060280, 0x7e080280, 0x7e0a0280, 0x7e0c0280]);
            w.extend([0xd7650002, 0x020100c1, 0x34040482]);
            if gather_address {
                // Reverse the address vector: source lane i routes to 31-i,
                // then OFFSET4 advances its destination once more.
                w.extend([0xd8d47c1f, 0x05000002, LGKM]);
                w.extend(scatter(4, 5, 1, 4));
            } else {
                // Pair-swap source data before an independent rotate scatter.
                w.extend([0xd8d4041f, 0x03000001, LGKM]);
                w.extend(scatter(4, 2, 3, 4));
            }
            w.extend([LGKM, 0xd8d4401f, 0x06000004, LGKM]);
            // The outer gather must retain the entire nested expression's
            // snapshots despite later overwrites of data, addresses and results.
            w.extend([0x7e020280, 0x7e040280, 0x7e060280, 0x7e080280, 0x7e0a0280]);
            let mut highest_policy = Vec::new();
            for wave in data.as_chunks::<32>().0 {
                // Materialize each stage independently; do not collapse the
                // chain to an inverse routing formula or call either evaluator.
                let gathered_data: Vec<_> = if gather_address {
                    wave.to_vec()
                } else {
                    wave.as_chunks::<2>()
                        .0
                        .iter()
                        .flat_map(|pair| [pair[1], pair[0]])
                        .collect()
                };
                let gathered_addresses: Vec<_> = if gather_address {
                    (0..32).rev().map(|lane| lane * 4).collect()
                } else {
                    (0..32).map(|lane| lane * 4).collect()
                };
                let mut scattered = [0; 32];
                for (source, &address) in gathered_addresses.iter().enumerate() {
                    let destination = ((address + 4) / 4) % 32;
                    scattered[destination] = gathered_data[source];
                }
                highest_policy.extend_from_slice(&scattered[16..]);
                highest_policy.extend_from_slice(&scattered[..16]);
            }
            out.push(Program {
                name: format!("gather-scatter-gather length{len} address{gather_address}"),
                code: finish(w, 6),
                input: data,
                eligible_values: singleton(&highest_policy),
                highest_policy,
            });
        }
    }
    out
}

fn programs() -> Vec<Program> {
    base_programs()
        .into_iter()
        .chain(composition_programs())
        .chain(gather_scatter_gather_programs())
        .collect()
}
fn compile(code: &[u8]) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    xenora_shader::compile_with_options(
        xenora_shader::Architecture::Gfx1030,
        code,
        xenora_shader::CompileOptions {
            wave32_launch: xenora_shader::Wave32Launch::FullWavesRequired,
        },
    )
}
fn reference(code: &[u8], input: &[u32]) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_options(
        code,
        input,
        xenora_ref::ExecuteOptions {
            wave32_launch: xenora_ref::Wave32Launch::FullWavesRequired,
        },
    )
}
fn assert_membership(p: &Program, output: &[u32]) {
    assert_eq!(output.len(), p.eligible_values.len(), "{}", p.name);
    for (lane, (&word, candidates)) in output.iter().zip(&p.eligible_values).enumerate() {
        assert!(
            candidates.contains(&word),
            "{} lane{lane}: {word:x} not in {candidates:x?}",
            p.name
        );
    }
}
#[test]
fn public_example_and_literal_encoding_anchors() {
    assert_eq!(scatter(3, 2, 1, 0), [0xdac80000, 0x03000102]);
    assert_eq!(
        scatter(3, 2, 1, 0)
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        [0, 0, 0xc8, 0xda, 2, 1, 0, 3]
    );
    let mut c = ScatterCase {
        source: (1..=32).collect(),
        addresses: [0; 32],
        offset: 0,
        exec: 0xf,
        dst: 3,
        prior: vec![SENTINEL; 32],
    };
    c.addresses[..4].copy_from_slice(&[0, 0, 12, 4]);
    let (membership, highest) = answers(&c);
    assert_eq!(&highest[..4], &[2, 4, 0, 3]);
    assert_eq!(membership[0], [1, 2]); // prose permits A or B; policy selects B
    c.exec = 0xa;
    assert_eq!(&answers(&c).1[..4], &[SENTINEL, 4, SENTINEL, 0]);
    assert_eq!(base_programs().len(), 72);
    assert_eq!(composition_programs().len(), 9);
    assert_eq!(gather_scatter_gather_programs().len(), 6);
    assert_eq!(programs().len(), 87);
}
#[test]
fn exact_uncontested_and_collision_eligible_results() {
    for p in programs() {
        compile(&p.code).unwrap_or_else(|e| panic!("{}: {e}", p.name));
        let output = reference(&p.code, &p.input).unwrap();
        assert_membership(&p, &output);
        for (lane, candidates) in p.eligible_values.iter().enumerate() {
            if candidates.len() == 1 {
                assert_eq!(output[lane], candidates[0], "{} lane{lane}", p.name);
            }
        }
    }
}
#[test]
fn project_highest_active_writer_policy_separate_from_spec_membership() {
    for p in base_programs() {
        assert_eq!(
            reference(&p.code, &p.input).unwrap(),
            p.highest_policy,
            "{}",
            p.name
        );
    }
}
fn invalid_programs() -> Vec<(&'static str, Vec<u8>)> {
    let mut out = Vec::new();
    let mut ready = LOAD.to_vec();
    ready.extend([VM, 0x7e040280, 0x7e060280]);
    let mut pending = ready.clone();
    pending.extend(scatter(3, 2, 1, 0));
    for wait in [None, Some(VM), Some(0xbc7d0000), Some(VS)] {
        let mut w = pending.clone();
        if let Some(wait) = wait {
            w.push(wait);
        }
        out.push(("DS output before LGKM", finish(w, 3)));
    }
    for (name, tail) in [
        ("pending old destination overwrite", vec![0x7e060280, LGKM]),
        ("pending destination compare", vec![0x7d840303, LGKM]),
        (
            "pending destination readlane",
            vec![0xbe890380, 0xd7600008, 0x02001303, LGKM],
        ),
        ("pending destination vector read", vec![0x7e080303, LGKM]),
        ("pending DS at END", vec![]),
        (
            "overlapping DS destinations",
            vec![0xdac80000, 0x04000102, LGKM],
        ),
        (
            "reissue pending destination",
            vec![0xdac80000, 0x03000102, LGKM],
        ),
    ] {
        let mut w = pending.clone();
        w.extend(tail);
        out.push((name, finish(w, 1)));
    }
    let mut w = LOAD.to_vec();
    w.push(LGKM);
    out.push(("LGKM does not release buffer load", finish(w, 1)));
    let mut w = ready.clone();
    w.extend([0xe0702000, 0x80010100, LGKM, END]);
    out.push((
        "LGKM does not release store",
        w.into_iter().flat_map(u32::to_le_bytes).collect(),
    ));
    for (first, second) in [
        (0xdac90000, 0x03000102),
        (0xdaca0000, 0x03000102),
        (0xdac80000, 0x03010102),
    ] {
        let mut w = ready.clone();
        w.extend([first, second, LGKM]);
        out.push(("reserved/GDS/DATA1", finish(w, 3)));
    }
    for (dst, addr, src) in [(0, 2, 1), (3, 4, 1), (3, 2, 4)] {
        let mut w = ready.clone();
        w.extend(scatter(dst, addr, src, 0));
        w.push(LGKM);
        out.push(("reserved dst or uninitialized source", finish(w, 3)));
    }
    let mut w = LOAD.to_vec();
    w.extend([0x7e040280, 0x7e060280]);
    w.extend(scatter(3, 2, 1, 0));
    w.extend([VM, LGKM]);
    out.push(("pending source load", finish(w, 3)));
    out
}
#[test]
fn readiness_and_wait_domains_reject() {
    // Positive control ensures an unsupported opcode cannot make all negatives pass.
    let p = base_programs().remove(0);
    compile(&p.code).unwrap();
    reference(&p.code, &p.input).unwrap();
    for (name, code) in invalid_programs() {
        assert!(compile(&code).is_err(), "compiler: {name}");
        assert!(reference(&code, &input(32)).is_err(), "reference: {name}");
    }
}
#[test]
fn nested_scatter_data_and_address_replay_reject() {
    // Contract: at most one scatter per expression replay path, including both
    // data and address dependencies. Separate sequential independent scatters
    // are covered by native program0. Both negative programs are far below the
    // code-size cap; there is no large-input or giant-source confounder.
    let positive = base_programs().remove(0);
    compile(&positive.code).unwrap();
    for (address, data) in [(2, 3), (3, 1)] {
        let mut w = LOAD.to_vec();
        w.extend([VM, 0x7e040280, 0x7e060280]);
        w.extend(scatter(3, 2, 1, 0));
        w.push(LGKM);
        w.extend(scatter(4, address, data, 0));
        w.push(LGKM);
        let code = finish(w, 4);
        assert!(code.len() < 1024);
        assert!(
            compile(&code).is_err(),
            "nested scatter data{data} address{address}"
        );
        // This is a compiler replay-admission constraint, not AMD semantics.
        // The byte-oriented reference may evaluate it without expression replay.
    }
}
#[test]
fn collision_membership_accepts_a_nonhighest_writer_but_policy_does_not() {
    let p = base_programs().remove(60); // all32 active writers target lane0
    let mut alternative = p.highest_policy.clone();
    let lowest = p.eligible_values[0][0];
    assert_ne!(lowest, p.highest_policy[0]);
    alternative[0] = lowest;
    assert_membership(&p, &alternative); // AMD prose permits this result
    assert_ne!(alternative, p.highest_policy); // project policy does not
    assert!(!p.eligible_values[0].contains(&SENTINEL));
    assert_eq!(p.eligible_values[1], vec![0]); // no writer is exactly zero
}
#[test]
fn full_wave_launch_contract() {
    let p = base_programs().remove(0);
    let compiled = compile(&p.code).unwrap();
    for count in [0, 1, 31, 33, 65, 4097] {
        assert!(compiled.validate_launch(count).is_err());
        assert!(reference(&p.code, &vec![0; count]).is_err());
    }
    for count in [32, 64, 96, 4096] {
        compiled.validate_launch(count).unwrap();
    }
    assert!(xenora_shader::compile(xenora_shader::Architecture::Gfx1030, &p.code).is_err());
    assert!(xenora_ref::execute(&p.code, &p.input).is_err());
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_scatter_membership_and_project_policy() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (n, p) in programs().into_iter().enumerate() {
        let compiled = compile(&p.code).unwrap();
        let output = gpu.run_compiled_words(&compiled, &p.input).unwrap();
        assert_membership(&p, &output);
        assert_eq!(output, p.highest_policy, "project policy: {}", p.name);
        if [0, 83, 86].contains(&n) {
            compiled.validate_launch(p.input.len()).unwrap();
            for width in [2, 17, 64] {
                let output = gpu
                    .run_words_with_group_width(
                        &compiled.msl,
                        xenora_shader::ENTRY_POINT,
                        &p.input,
                        width,
                    )
                    .unwrap()
                    .output;
                assert_membership(&p, &output);
                assert_eq!(output, p.highest_policy, "{} group width{width}", p.name);
            }
        }
    }
    // 87 distinct native programs; 9 additional width checks =96 dispatches.
}
