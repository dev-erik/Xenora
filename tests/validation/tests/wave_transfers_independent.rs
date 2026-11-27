// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent expected values with compiler/reference acceptance and an ignored
//! native harness. No AMD hardware validation is claimed.
//! Public AMD RDNA2 70648: printed pp186-187 (PERMLANE) and pp206-209 (DS),
//! independently read from /private/tmp/clamp-rdna2.txt.
//! https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content
//! Contract: FullWavesRequired; SWIZZLE/PERMLANE full EXEC; BPERMUTE static EXEC.
//! Literal transfer encodings follow the public format tables; e64 READFIRST
//! explicitly has no LLVM encoding verification.
//! Nonzero-mask rotate is rejected: printed p207 examples disagree with p209.
//! Both descriptions remain recorded below as evidence of the conflict; neither
//! is used as an accepted lowering for those offsets.
const LGKM: u32 = 0xbf8cc07f;
const SENTINEL: u32 = 0xa5a55a5a;
#[derive(Clone, Debug)]
enum Transfer {
    BPermute {
        offset: u16,
    },
    Swizzle {
        offset: u16,
    },
    Permlane {
        cross: bool,
        selectors: [u32; 2],
        fi: bool,
        bc: bool,
    },
}
#[derive(Debug)]
struct Case {
    operation: Transfer,
    source: Vec<u32>,
    addresses: Vec<u32>,
    prior: Vec<u32>,
    exec: u32,
    alias_source: bool,
    alias_address: bool,
    expected: Vec<u32>,
}
fn input(len: usize) -> Vec<u32> {
    (0..len)
        .map(|i| [0x80001000, 0xfffffe00, 0x7fffff00][i / 32] + (i % 32) as u32)
        .collect()
}
fn swizzle_lane(offset: u16, lane: usize) -> usize {
    if offset >= 0xe000 {
        // Five-bit reversal, written as weighted contributions rather than
        // a host bit-reverse intrinsic. AMD's examples pin the reversal width.
        let reversed: usize = (0..5)
            .map(|bit| ((lane / (1 << bit)) % 2) * (1 << (4 - bit)))
            .sum();
        let kept: Vec<_> = (0..5).filter(|bit| offset & (1 << bit) != 0).collect();
        (reversed / (1 << kept.len())) | (lane & usize::from(offset & 31))
    } else if offset >= 0xc000 {
        // Work modulo32 before retaining masked original lane bits. This agrees
        // with the documented wrap examples at either rotation boundary.
        let amount = i32::from((offset / 32) % 32);
        let delta = if offset & 0x400 == 0 { amount } else { -amount };
        let rotated = (lane as i32 + delta).rem_euclid(32) as usize;
        let keep = usize::from(offset & 31);
        (lane & keep) | (rotated & (31 ^ keep))
    } else if offset >= 0x8000 {
        let choice = (usize::from(offset) / (1 << (2 * (lane % 4)))) % 4;
        lane / 4 * 4 + choice
    } else {
        // Per-bit truth table for ((lane AND and_mask) OR or_mask) XOR xor_mask.
        (0..5).fold(0, |result, bit| {
            let from_lane = lane & (1 << bit) != 0 && offset & (1 << bit) != 0;
            let force = offset & (1 << (bit + 5)) != 0;
            let invert = offset & (1 << (bit + 10)) != 0;
            result | (usize::from((from_lane || force) != invert) << bit)
        })
    }
}
fn selected(operation: &Transfer, lane: usize, address: u32) -> usize {
    match *operation {
        Transfer::BPermute { offset } => {
            ((u64::from(address) + u64::from(offset)) / 4 % 32) as usize
        }
        Transfer::Swizzle { offset } => swizzle_lane(offset, lane),
        Transfer::Permlane {
            cross, selectors, ..
        } => {
            let column = lane % 16;
            let selected = (selectors[column / 8] / 16u32.pow((column % 8) as u32)) % 16;
            let row = if cross { 1 - lane / 16 } else { lane / 16 };
            row * 16 + selected as usize
        }
    }
}
fn expected(
    operation: &Transfer,
    source: &[u32],
    addresses: &[u32],
    prior: &[u32],
    exec: u32,
) -> Vec<u32> {
    source
        .as_chunks::<32>()
        .0
        .iter()
        .enumerate()
        .flat_map(|(wave, words)| {
            (0..32).map(move |lane| {
                let index = wave * 32 + lane;
                if matches!(operation, Transfer::BPermute { .. }) && exec & (1 << lane) == 0 {
                    return prior[index];
                }
                let from = selected(operation, lane, addresses[index]);
                if matches!(operation, Transfer::BPermute { .. }) && exec & (1 << from) == 0 {
                    0
                } else {
                    words[from]
                }
            })
        })
        .collect()
}
fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    // BPERMUTE: byte-low-bit discard, offset carry, wrap31->0, large addresses,
    // broadcast, reverse, repeated-wave separation, zero-source and retain-dest.
    for (n, offset) in [0, 1, 3, 4, 124, 127, 128, 255, 65535]
        .into_iter()
        .enumerate()
    {
        for exec in [0, 0x80000002, 0x55555555, u32::MAX] {
            let len = [32, 64, 96][n % 3];
            let source = input(len);
            let addresses: Vec<u32> = (0..len)
                .map(|i| match n % 3 {
                    0 => (31 - i % 32) as u32 * 4,
                    1 => [0, 1, 3, 4, 124, 127, 128, u32::MAX][i % 8],
                    _ => 60,
                })
                .collect();
            let alias_source = n % 3 == 1;
            let alias_address = n % 3 == 2;
            let prior = if alias_source {
                source.clone()
            } else if alias_address {
                addresses.clone()
            } else {
                vec![SENTINEL; len]
            };
            let operation = Transfer::BPermute { offset };
            let want = expected(&operation, &source, &addresses, &prior, exec);
            cases.push(Case {
                operation,
                source,
                addresses,
                prior,
                exec,
                alias_source,
                alias_address,
                expected: want,
            });
        }
    }
    // SWIZZLE: bitmask identity/reversals/each broadcast; quad modes including
    // upper ignored bits; rotate both directions/all keep bits; every FFT mask.
    let mut offsets = vec![
        0x001f, 0x041f, 0x201f, 0x401f, 0x7c1f, 0x1fff, 0x0000, 0x8000, 0x80e4, 0x801b, 0x8055,
        0xbfe4,
    ];
    offsets.extend((0..32).map(|lane| lane << 5));
    for mask in [0, 1, 3, 7, 15, 31] {
        for amount in [0, 1, 15, 31] {
            offsets.extend([0xc000 | (amount << 5) | mask, 0xc400 | (amount << 5) | mask]);
        }
    }
    offsets.extend(0xe000..=0xe01f);
    offsets.extend([0xe3ff, 0xffff]);
    for (n, offset) in offsets.into_iter().enumerate() {
        let len = [32, 64, 96][n % 3];
        let source = input(len);
        let addresses = vec![0; len];
        let alias_source = n % 2 == 0;
        let prior = if alias_source {
            source.clone()
        } else {
            vec![SENTINEL; len]
        };
        let operation = Transfer::Swizzle { offset };
        let want = expected(&operation, &source, &addresses, &prior, u32::MAX);
        cases.push(Case {
            operation,
            source,
            addresses,
            prior,
            exec: u32::MAX,
            alias_source,
            alias_address: false,
            expected: want,
        });
    }
    // PERMLANE: both rows, every nibble position and both selector halves,
    // identity/reverse/rotate/broadcast, all FI/BC bits under full EXEC.
    for cross in [false, true] {
        for (n, selectors) in [
            [0x76543210, 0xfedcba98],
            [0x89abcdef, 0x01234567],
            [0x87654321, 0x0fedcba9],
            [0, 0],
            [u32::MAX, u32::MAX],
        ]
        .into_iter()
        .enumerate()
        {
            for (fi, bc) in [(false, false), (true, false), (false, true), (true, true)] {
                let len = [32, 64, 96][n % 3];
                let source = input(len);
                let addresses = vec![0; len];
                let alias_source = fi;
                let prior = if alias_source {
                    source.clone()
                } else {
                    vec![SENTINEL; len]
                };
                let operation = Transfer::Permlane {
                    cross,
                    selectors,
                    fi,
                    bc,
                };
                let want = expected(&operation, &source, &addresses, &prior, u32::MAX);
                cases.push(Case {
                    operation,
                    source,
                    addresses,
                    prior,
                    exec: u32::MAX,
                    alias_source,
                    alias_address: false,
                    expected: want,
                });
            }
        }
    }
    cases
}
// Integration recipes: load/VM wait -> first gather/LGKM if DS -> overwrite
// original source and address registers -> second gather/LGKM if DS -> compare
// result to an independently constructed marker -> capture lane31/read scalar.
#[derive(Debug)]
struct ChainCase {
    first: Transfer,
    second: Transfer,
    source: Vec<u32>,
    after_first: Vec<u32>,
    after_second: Vec<u32>,
    equality_to_original: Vec<u32>,
    read_lane31: Vec<u32>,
}
fn chains() -> Vec<ChainCase> {
    let mut cases = Vec::new();
    for len in [32, 64, 96] {
        for first in [
            Transfer::Swizzle { offset: 0x7c1f },
            Transfer::Permlane {
                cross: true,
                selectors: [0x76543210, 0xfedcba98],
                fi: false,
                bc: false,
            },
        ] {
            let second = Transfer::BPermute { offset: 4 };
            let source = input(len);
            let addresses: Vec<_> = (0..len).map(|i| (i % 32) as u32 * 4).collect();
            let prior = vec![SENTINEL; len];
            let after_first = expected(&first, &source, &addresses, &prior, u32::MAX);
            let after_second = expected(&second, &after_first, &addresses, &prior, u32::MAX);
            let equality_to_original = source
                .as_chunks::<32>()
                .0
                .iter()
                .zip(after_second.as_chunks::<32>().0.iter())
                .flat_map(|(a, b)| {
                    let mask = a
                        .iter()
                        .zip(b)
                        .enumerate()
                        .fold(0, |m, (i, (x, y))| m | (u32::from(x == y) << i));
                    vec![mask; 32]
                })
                .collect();
            let read_lane31 = after_second
                .as_chunks::<32>()
                .0
                .iter()
                .flat_map(|wave| vec![wave[31]; 32])
                .collect();
            cases.push(ChainCase {
                first,
                second,
                source,
                after_first,
                after_second,
                equality_to_original,
                read_lane31,
            });
        }
    }
    cases
}
// Negative integration cases, each required for both DS forms:
// - read/store/compare/readlane pending DS destination before LGKM;
// - VM wait (bf8c3f70 or bc7d0000) and VS wait do not release DS;
// - LGKM does not release a pending buffer load or store;
// - write or reissue into pending DS destination, including an initialized old value;
// - outstanding transfer at END; repeat LGKM is harmless after completion;
// - invalid DS GDS/reserved/data1/address fields; wrong source/destination readiness;
// - SWIZZLE/PERMLANE partial or empty EXEC, unknown/runtime/special selectors;
// - unsupported modifier bits; runtime-branch-region transfers;
// - FullWavesRequired rejects empty/partial/oversized launches before GPU.
#[test]
fn documented_examples_and_pseudocode_anchors_only() {
    assert_eq!(LGKM.to_le_bytes(), [0x7f, 0xc0, 0x8c, 0xbf]);
    assert_eq!(
        (0..8).map(|i| swizzle_lane(0xe000, i)).collect::<Vec<_>>(),
        [0, 16, 8, 24, 4, 20, 12, 28]
    );
    assert_eq!(
        (0..8).map(|i| swizzle_lane(0xe010, i)).collect::<Vec<_>>(),
        [0, 8, 4, 12, 2, 10, 6, 14]
    );
    // P207 lists one-based hexadecimal data. Subtract one to get lane indices.
    let p207 = [1, 0, 3, 6, 5, 4, 7, 10];
    let p209: Vec<_> = (0..8).map(|i| swizzle_lane(0xc021, i)).collect();
    assert_ne!(p207.as_slice(), p209.as_slice());
    assert_eq!((p207[0], p209[0]), (1, 0));
    // P209 expression is retained solely to demonstrate the conflict.
    assert_eq!(
        (0..8).map(|i| swizzle_lane(0xc021, i)).collect::<Vec<_>>(),
        [0, 3, 2, 5, 4, 7, 6, 9]
    );
    assert_eq!(swizzle_lane(0xc020, 31), 0);
    assert_eq!(swizzle_lane(0xc420, 0), 31);
    assert_eq!(
        (0..8).map(|i| swizzle_lane(0x801b, i)).collect::<Vec<_>>(),
        [3, 2, 1, 0, 7, 6, 5, 4]
    );
    let source: Vec<_> = (1..=32).collect();
    let mut addresses = vec![0; 32];
    addresses[..4].copy_from_slice(&[0, 0, 12, 4]);
    let out = expected(
        &Transfer::BPermute { offset: 0 },
        &source,
        &addresses,
        &[SENTINEL; 32],
        0xa,
    );
    assert_eq!(&out[..4], &[SENTINEL, 0, SENTINEL, 2]);
}
#[test]
fn prepared_corpus_is_internal_consistent_not_production_acceptance() {
    let cases = cases();
    assert_eq!(cases.len(), 202);
    for case in cases {
        assert_eq!(case.source.len(), case.expected.len());
        assert_eq!(case.addresses.len(), case.prior.len());
        assert!(!(case.alias_source && case.alias_address));
        if let Transfer::Permlane { fi, bc, .. } = &case.operation {
            let _ = (*fi, *bc);
            assert_eq!(case.exec, u32::MAX);
        }
        for i in 0..case.source.len() {
            if matches!(case.operation, Transfer::BPermute { .. })
                && case.exec & (1 << (i % 32)) == 0
            {
                assert_eq!(case.expected[i], case.prior[i]);
            }
        }
    }
    let chains = chains();
    assert_eq!(chains.len(), 6);
    for chain in chains {
        assert!(matches!(
            chain.first,
            Transfer::Swizzle { .. } | Transfer::Permlane { .. }
        ));
        assert!(matches!(chain.second, Transfer::BPermute { .. }));
        assert_eq!(chain.source.len(), chain.after_first.len());
        assert_eq!(chain.source.len(), chain.after_second.len());
        assert_eq!(chain.source.len(), chain.equality_to_original.len());
        assert_eq!(chain.source.len(), chain.read_lane31.len());
    }
}

// Actual compiler/reference/native acceptance. Nonzero-mask rotate is excluded
// from positive/native cases and explicitly rejected pending clarification.
const LOAD: [u32; 2] = [0xe0302000, 0x80000100];
const VM: u32 = 0xbf8c3f70;
struct Program {
    code: Vec<u8>,
    data: Vec<u32>,
    want: Vec<u32>,
    full: bool,
}
fn finish(mut w: Vec<u32>, dst: u32) -> Vec<u8> {
    w.extend([0xe0702000, 0x80010000 | dst << 8, 0xbbfd0000, 0xbf810000]);
    w.into_iter().flat_map(u32::to_le_bytes).collect()
}
fn transfer(w: &mut Vec<u32>, op: &Transfer, dst: u32, src: u32, addr: u32) {
    match *op {
        Transfer::BPermute { offset } => {
            w.extend([0xdacc0000 | u32::from(offset), dst << 24 | src << 8 | addr])
        }
        Transfer::Swizzle { offset } => w.extend([0xd8d40000 | u32::from(offset), dst << 24 | src]),
        Transfer::Permlane {
            cross,
            selectors,
            fi,
            bc,
        } => {
            w.extend([0xbe8803ff, selectors[0], 0xbe8903ff, selectors[1]]);
            w.extend([
                if cross { 0xd7780000 } else { 0xd7770000 }
                    | dst
                    | u32::from(fi) << 11
                    | u32::from(bc) << 12,
                256 | src | 8 << 9 | 9 << 18,
            ]);
        }
    }
}
fn complete(w: &mut Vec<u32>, op: &Transfer) {
    if !matches!(op, Transfer::Permlane { .. }) {
        w.push(LGKM);
    }
}
fn disputed(op: &Transfer) -> bool {
    matches!(op, Transfer::Swizzle{offset} if (0xc000..0xe000).contains(offset) && offset & 31 != 0)
}
fn program(case: Case) -> Program {
    let mut w = LOAD.to_vec();
    w.push(VM);
    w.extend([0x7e040280, 0x7e0602ff, SENTINEL]);
    // Materialize original address patterns by lane, without relying on a
    // production-derived expression or changing the fixed buffer ABI.
    for (lane, &address) in case.addresses.iter().take(32).enumerate() {
        w.extend([0xbefe03ff, 1u32 << lane, 0x7e0402ff, address]);
    }
    w.extend([0xbefe03ff, case.exec]);
    let dst = if case.alias_source {
        1
    } else if case.alias_address {
        2
    } else {
        3
    };
    transfer(&mut w, &case.operation, dst, 1, 2);
    complete(&mut w, &case.operation);
    w.push(0xbefe03c1);
    Program {
        code: finish(w, dst),
        data: case.source,
        want: case.expected,
        full: true,
    }
}
fn programs() -> Vec<Program> {
    let mut out: Vec<_> = cases()
        .into_iter()
        .filter(|c| !disputed(&c.operation))
        .map(program)
        .collect();
    // Nested gather, destructive later source overwrite, then comparison or
    // lane31 capture. The first transfer's source must remain a snapshot.
    for chain in chains() {
        for mode in 0..3 {
            let mut w = LOAD.to_vec();
            w.extend([VM, 0x7e060280, 0x7e080280]);
            transfer(&mut w, &chain.first, 3, 1, 2);
            complete(&mut w, &chain.first);
            // MBCNT yields lane; multiply by four for the byte address.
            w.extend([0xd7650002, 0x020100c1, 0x34040482]);
            transfer(&mut w, &chain.second, 4, 3, 2);
            complete(&mut w, &chain.second);
            let (dst, want) = match mode {
                0 => (4, chain.after_second.clone()),
                1 => {
                    w.extend([0x7d840304, 0x7e0a026a]);
                    (5, chain.equality_to_original.clone())
                }
                _ => {
                    w.extend([0xbe89039f, 0xd7600008, 0x02001304, 0x7e080280, 0x7e0a0208]);
                    (5, chain.read_lane31.clone())
                }
            };
            w.extend([0x7e020280, 0x7e060280, 0x7e040280]);
            out.push(Program {
                code: finish(w, dst),
                data: chain.source.clone(),
                want,
                full: true,
            });
        }
    }
    // Literal p207 FFT examples, converted from one-based hexadecimal values.
    // These expectations do not call swizzle_lane or another reference model.
    for (offset, indices) in [
        (
            0xe000,
            [
                0, 16, 8, 24, 4, 20, 12, 28, 2, 18, 10, 26, 6, 22, 14, 30, 1, 17, 9, 25, 5, 21, 13,
                29, 3, 19, 11, 27, 7, 23, 15, 31,
            ],
        ),
        (
            0xe010,
            [
                0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15, 16, 24, 20, 28, 18, 26, 22,
                30, 17, 25, 21, 29, 19, 27, 23, 31,
            ],
        ),
        (
            0xe01f,
            [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,
                23, 24, 25, 26, 27, 28, 29, 30, 31,
            ],
        ),
    ] {
        let data = input(96);
        let want = data
            .as_chunks::<32>()
            .0
            .iter()
            .flat_map(|wave| indices.map(|i| wave[i]))
            .collect();
        let mut w = LOAD.to_vec();
        w.extend([VM, 0x7e060280]);
        transfer(&mut w, &Transfer::Swizzle { offset }, 3, 1, 2);
        w.push(LGKM);
        out.push(Program {
            code: finish(w, 3),
            data,
            want,
            full: true,
        });
    }
    out
}
fn compile(p: &Program) -> Result<xenora_shader::CompiledShader, xenora_shader::CompileError> {
    xenora_shader::compile_with_options(
        xenora_shader::Architecture::Gfx1030,
        &p.code,
        xenora_shader::CompileOptions {
            wave32_launch: if p.full {
                xenora_shader::Wave32Launch::FullWavesRequired
            } else {
                xenora_shader::Wave32Launch::PartialAllowed
            },
        },
    )
}
fn reference(p: &Program) -> Result<Vec<u32>, xenora_ref::ReferenceError> {
    xenora_ref::execute_with_options(
        &p.code,
        &p.data,
        xenora_ref::ExecuteOptions {
            wave32_launch: if p.full {
                xenora_ref::Wave32Launch::FullWavesRequired
            } else {
                xenora_ref::Wave32Launch::PartialAllowed
            },
        },
    )
}
#[test]
fn compiler_and_reference_transfers_snapshots_and_replay() {
    for (n, p) in programs().iter().enumerate() {
        compile(p).unwrap_or_else(|e| panic!("program {n}: {e}"));
        assert_eq!(
            reference(p).unwrap_or_else(|e| panic!("program {n}: {e}")),
            p.want,
            "program {n}"
        );
    }
}
#[test]
fn nonzero_mask_rotate_rejects_pending_spec_clarification() {
    for c in cases().into_iter().filter(|c| disputed(&c.operation)) {
        let p = program(c);
        assert!(
            compile(&p).is_err(),
            "nonzero-mask rotate compiler admission"
        );
        assert!(
            reference(&p).is_err(),
            "nonzero-mask rotate reference admission"
        );
    }
}

// READFIRSTLANE e64: AMD public XML opcode386 + Table82 bit layout.
// Literal bytes, NOT native LLVM evidence (LLVM does not support this form).
// Zero unused fields are a project admission subset, not an AMD requirement
// for canonical padding. No AMD hardware validation has been performed.
fn e64_program(selected: usize, exec: u32, len: usize, full: bool, boundary: bool) -> Program {
    let data = input(len);
    let src = if boundary { 255 } else { 1 };
    let dst = if boundary { 105 } else { 8 };
    let mut w = LOAD.to_vec();
    w.push(VM);
    if boundary {
        w.push(0x7ffe0301);
    }
    w.extend([
        0xbefe03ff,
        exec,
        0xd5820000 | dst,
        256 | src,
        0xbefe03c1,
        0x7e000280 | src << 17,
        0x7e060200 | dst,
    ]);
    let want = data
        .chunks(32)
        .flat_map(|wave| vec![wave[selected]; wave.len()])
        .collect();
    Program {
        code: finish(w, 3),
        data,
        want,
        full,
    }
}
fn e64_programs() -> Vec<Program> {
    let mut out: Vec<_> = (0..32)
        .map(|lane| {
            e64_program(
                lane,
                (1 << lane) | 0x80000000,
                [32, 64, 96][lane % 3],
                true,
                lane == 31,
            )
        })
        .collect();
    out.push(e64_program(0, 0, 96, true, false));
    for (mask, len) in [(0, 1), (1, 33), (u32::MAX, 65)] {
        out.push(e64_program(0, mask, len, false, false));
    }
    out
}
#[test]
fn e64_literal_encoding_and_readfirst_outputs() {
    assert_eq!(
        include_bytes!("../../../fixtures/rdna2/lane-read-e64-spec.bin").as_slice(),
        e64_program(0, 0, 96, true, false).code
    );
    assert_eq!(
        [0xd5820008u32, 0x101]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        [8, 0, 0x82, 0xd5, 1, 1, 0, 0]
    );
    assert_eq!(
        [0xd5820069u32, 0x1ff]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
        [0x69, 0, 0x82, 0xd5, 0xff, 1, 0, 0]
    );
    for (n, p) in e64_programs().iter().enumerate() {
        compile(p).unwrap_or_else(|e| panic!("e64 program {n}: {e}"));
        assert_eq!(reference(p).unwrap(), p.want, "e64 program {n}");
    }
}
#[test]
fn e64_project_zero_padding_and_modifier_rejections() {
    let base = e64_program(0, 1, 32, true, false);
    // Locate literal encoding rather than assume instruction-array indices.
    let at = base
        .code
        .windows(4)
        .position(|w| w == [8, 0, 0x82, 0xd5])
        .unwrap();
    let mut mutations = Vec::new();
    for bit in 8..16 {
        mutations.push((0, 1u32 << bit));
    }
    for bit in 9..32 {
        mutations.push((4, 1u32 << bit));
    }
    for (word, mask) in mutations {
        let mut p = Program {
            code: base.code.clone(),
            data: base.data.clone(),
            want: vec![],
            full: true,
        };
        let start = at + word;
        let value = u32::from_le_bytes(p.code[start..start + 4].try_into().unwrap()) | mask;
        p.code[start..start + 4].copy_from_slice(&value.to_le_bytes());
        assert!(compile(&p).is_err(), "word {word}, mask {mask:x}");
        assert!(reference(&p).is_err(), "word {word}, mask {mask:x}");
    }
}
#[test]
#[ignore = "lead coordinates native Metal with API and shader validation"]
fn native_wave_transfers_independent_outputs() {
    assert_eq!(std::env::var("MTL_DEBUG_LAYER").as_deref(), Ok("1"));
    assert_eq!(std::env::var("MTL_SHADER_VALIDATION").as_deref(), Ok("1"));
    let gpu = xenora_metal::NativeCompute::new().unwrap();
    for (n, p) in native_programs().into_iter().enumerate() {
        let compiled = compile(&p).unwrap();
        assert_eq!(
            gpu.run_compiled_words(&compiled, &p.data).unwrap(),
            p.want,
            "program {n}"
        );
    }
}
fn negative_programs() -> Vec<(&'static str, Program)> {
    let mut out = Vec::new();
    let mut add = |name, w, dst| {
        out.push((
            name,
            Program {
                code: finish(w, dst),
                data: input(32),
                want: vec![],
                full: true,
            },
        ))
    };
    for op in [
        Transfer::BPermute { offset: 0 },
        Transfer::Swizzle { offset: 0x401f },
    ] {
        let mut start = LOAD.to_vec();
        start.extend([VM, 0x7e040280, 0x7e060280]);
        transfer(&mut start, &op, 3, 1, 2);
        for wait in [None, Some(VM), Some(0xbc7d0000), Some(0xbbfd0000)] {
            let mut w = start.clone();
            if let Some(wait) = wait {
                w.push(wait);
            }
            add("unreleased DS store", w, 3);
        }
        for (name, tail) in [
            ("pending DS move", vec![0x7e080303, LGKM]),
            ("pending DS compare", vec![0x7d840303, LGKM]),
            (
                "pending DS readlane",
                vec![0xbe890380, 0xd7600008, 0x02001303, LGKM],
            ),
            ("overwrite pending initialized dst", vec![0x7e060280, LGKM]),
            ("pending DS at END", vec![]),
        ] {
            let mut w = start.clone();
            w.extend(tail);
            add(name, w, 1);
        }
        let mut w = start.clone();
        transfer(&mut w, &op, 4, 1, 2);
        w.push(LGKM);
        add("overlapping DS", w, 1);
        let mut w = LOAD.to_vec();
        w.extend([0x7e040280, 0x7e060280]);
        transfer(&mut w, &op, 3, 1, 2);
        w.extend([VM, LGKM]);
        add("pending buffer source", w, 3);
        for bit in [16, 17] {
            let mut w = start.clone();
            let at = w.len() - 2;
            w[at] |= 1 << bit;
            w.push(LGKM);
            add("DS reserved/GDS", w, 3);
        }
        let mut w = start.clone();
        let at = w.len() - 1;
        w[at] |= 1 << 16;
        w.push(LGKM);
        add("DS unused data1", w, 3);
    }
    let mut w = LOAD.to_vec();
    w.push(LGKM);
    add("LGKM does not release buffer load", w, 1);
    let mut w = LOAD.to_vec();
    w.extend([VM, 0xe0702000, 0x80010100, LGKM, 0xbf810000]);
    out.push((
        "LGKM does not release store",
        Program {
            code: w.into_iter().flat_map(u32::to_le_bytes).collect(),
            data: input(32),
            want: vec![],
            full: true,
        },
    ));
    for cross in [false, true] {
        for selector in [10, 106, 128, 256] {
            let mut w = LOAD.to_vec();
            w.extend([
                VM,
                0x7e060280,
                0xbe890380,
                if cross { 0xd7780003 } else { 0xd7770003 },
                256 | 1 | selector << 9 | 9 << 18,
            ]);
            out.push((
                "unknown/special/inline/vector selector",
                Program {
                    code: finish(w, 3),
                    data: input(32),
                    want: vec![],
                    full: true,
                },
            ));
        }
        let mut w = LOAD.to_vec();
        w.extend([
            VM,
            0x7e060280,
            0x7e100501,
            0xbe890380,
            if cross { 0xd7780003 } else { 0xd7770003 },
            0x00241101,
        ]);
        out.push((
            "runtime scalar selector",
            Program {
                code: finish(w, 3),
                data: input(32),
                want: vec![],
                full: true,
            },
        ));
    }
    for op in [
        Transfer::Swizzle { offset: 0x401f },
        Transfer::Permlane {
            cross: false,
            selectors: [0, 0],
            fi: true,
            bc: true,
        },
        Transfer::Permlane {
            cross: true,
            selectors: [0, 0],
            fi: false,
            bc: false,
        },
    ] {
        for mask in [0, 0x80000001] {
            let mut w = LOAD.to_vec();
            w.extend([VM, 0x7e060280, 0xbefe03ff, mask]);
            transfer(&mut w, &op, 3, 1, 2);
            complete(&mut w, &op);
            w.push(0xbefe03c1);
            out.push((
                "nonfull EXEC",
                Program {
                    code: finish(w, 3),
                    data: input(32),
                    want: vec![],
                    full: true,
                },
            ));
        }
    }
    out
}
#[test]
fn pending_ds_wait_domains_and_scalar_admission_reject() {
    for (n, (name, p)) in negative_programs().iter().enumerate() {
        assert!(compile(p).is_err(), "compiler case {n}: {name}");
        assert!(reference(p).is_err(), "reference case {n}: {name}");
    }
}
#[test]
fn graphics_rejects_even_unreachable_transfers_and_lgkm() {
    let vs = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
    let ps = include_bytes!("../../../fixtures/rdna2/x9-constant-ps.bin");
    xenora_shader::compile_graphics(xenora_shader::Architecture::Gfx1030, vs, ps).unwrap();
    for words in [
        vec![0xdacc0000, 0x03000102],
        vec![0xd8d4401f, 0x03000001],
        vec![0xd7770003, 0x00241101],
        vec![0xd7780003, 0x00241101],
        vec![LGKM],
    ] {
        for skipped in [false, true] {
            let mut prefix = Vec::new();
            if skipped {
                prefix.push(0xbf820000 | words.len() as u32);
            }
            prefix.extend(&words);
            let prefix: Vec<u8> = prefix.into_iter().flat_map(u32::to_le_bytes).collect();
            for vertex in [false, true] {
                let mut code = prefix.clone();
                code.extend(if vertex { vs.as_slice() } else { ps.as_slice() });
                let err = xenora_shader::compile_graphics(
                    xenora_shader::Architecture::Gfx1030,
                    if vertex { &code } else { vs },
                    if vertex { ps } else { &code },
                )
                .unwrap_err();
                assert!(
                    err.to_string().contains("full Wave32 compute contract"),
                    "{err}"
                );
            }
        }
    }
}
fn native_programs() -> Vec<Program> {
    // 81 pairwise direct cases + all21 nested/FFT examples +36 e64 =138.
    programs()
        .into_iter()
        .enumerate()
        .filter(|(n, _)| *n >= 162 || n % 2 == (n / 4) % 2)
        .map(|(_, p)| p)
        .chain(e64_programs())
        .collect()
}
#[test]
fn frozen_acceptance_counts() {
    assert_eq!(programs().len(), 183);
    assert_eq!(e64_programs().len(), 36);
    assert_eq!(native_programs().len(), 138);
    assert_eq!(negative_programs().len(), 46);
}
