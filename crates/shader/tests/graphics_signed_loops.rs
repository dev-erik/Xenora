// SPDX-License-Identifier: GPL-3.0-or-later
//! Original inline signed countdown admission and bit-preservation regressions.
use xenora_shader::{Architecture::Gfx1030, compile_graphics_pipeline};
struct Program {
    words: Vec<u32>,
    labels: std::collections::HashMap<&'static str, usize>,
    branches: Vec<(usize, &'static str)>,
}
impl Program {
    fn new() -> Self {
        Self {
            words: Vec::new(),
            labels: Default::default(),
            branches: Vec::new(),
        }
    }
    fn emit(&mut self, words: &[u32]) {
        self.words.extend_from_slice(words);
    }
    fn label(&mut self, name: &'static str) {
        assert!(self.labels.insert(name, self.words.len()).is_none());
    }
    fn branch(&mut self, conditional: bool, target: &'static str) {
        self.branches.push((self.words.len(), target));
        self.words
            .push(if conditional { 0xbf840000 } else { 0xbf820000 });
    }
    fn bytes(mut self) -> Vec<u8> {
        for (at, target) in self.branches {
            let delta = i16::try_from(self.labels[target] as isize - at as isize - 1).unwrap();
            self.words[at] |= u32::from(delta as u16);
        }
        self.words.into_iter().flat_map(u32::to_le_bytes).collect()
    }
}

fn compile(p: Program) -> Result<xenora_shader::CompiledGraphics, xenora_shader::CompileError> {
    let vertex: Vec<u8> = [0xf80008cfu32, 0x03020100, 0xbf810000]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    compile_graphics_pipeline(Gfx1030, &vertex, &p.bytes())
}
fn init(p: &mut Program) {
    p.emit(&[0x7e100280, 0x7e1202f2]);
}
fn export(p: &mut Program) {
    p.label("exports");
    p.emit(&[0xf800180f, 0x09080808, 0xbf810000]);
}
fn signed_loop(p: &mut Program, reversed: bool, signed_sub: bool) {
    p.label("header");
    // SOPC GT_I32(s32,0) or LT_I32(0,s32), followed by SCC0 exit.
    p.emit(&[if reversed { 0xbf042080 } else { 0xbf028020 }]);
    p.branch(true, "exit");
    p.emit(&[if signed_sub { 0x81a08120 } else { 0x80a08120 }]);
    p.branch(false, "header");
    p.label("exit");
}
fn min_signed(p: &mut Program, cap: u32, reversed: bool) {
    // SOP2 MIN_I32 s32,s24,literal (or reversed).
    p.emit(&[if reversed { 0x832018ff } else { 0x8320ff18 }, cap]);
}
fn assert_bound_error(
    result: Result<xenora_shader::CompiledGraphics, xenora_shader::CompileError>,
) {
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("trip count lacks a proven bound")
    );
}

#[test]
fn signed_positive_min_accepts_both_operands_and_sub_opcodes() {
    for reverse_min in [false, true] {
        for reverse_cmp in [false, true] {
            for signed_sub in [false, true] {
                let mut p = Program::new();
                init(&mut p);
                min_signed(&mut p, 256, reverse_min);
                signed_loop(&mut p, reverse_cmp, signed_sub);
                export(&mut p);
                let compiled = compile(p).expect("signed-positive bounded countdown");
                assert_eq!(compiled.required_uniform_mask, 1);
                assert_eq!(compiled.sampled_pairs, 0);
            }
        }
    }
}

#[test]
fn signed_constant_edges_and_oversized_positive_counts() {
    for value in [0xffffffff, 0x80000000, 0, 1, 256, 257, 0x7fffffff] {
        let mut p = Program::new();
        init(&mut p);
        p.emit(&[0xbea003ff, value]); // MOV s32,literal
        signed_loop(&mut p, false, true);
        export(&mut p);
        if matches!(value, 257 | 0x7fffffff) {
            assert_bound_error(compile(p));
        } else {
            assert_eq!(compile(p).unwrap().required_uniform_mask, 0);
        }
    }
}

#[test]
fn signed_negative_zero_trip_retains_exact_counter_bits() {
    for value in [0xffffffff, 0x80000000] {
        let mut p = Program::new();
        init(&mut p);
        p.emit(&[0xbea003ff, value]);
        signed_loop(&mut p, false, true);
        p.emit(&[0xbf06ff20, value]); // EQ_U32 actual exit bits,literal
        p.branch(true, "bad");
        p.branch(false, "exports");
        p.label("bad");
        p.emit(&[0xbea2031f]); // reachable only if counter was changed: read s31
        export(&mut p);
        assert_eq!(compile(p).unwrap().required_uniform_mask, 0);
    }
}

#[test]
fn negative_runtime_min_does_not_become_an_unsigned_zero_or_trip_bound() {
    for cap in [0xffffffff, 0x80000000, 256] {
        for rebound in [false, true] {
            let mut p = Program::new();
            init(&mut p);
            min_signed(&mut p, cap, false);
            signed_loop(&mut p, false, true);
            if rebound {
                p.emit(&[0x83a08420]);
            } // MIN_U32(s32,4)
            p.label("unsigned");
            p.emit(&[0xbf078020]);
            p.branch(true, "done");
            p.emit(&[0x80a08120]);
            p.branch(false, "unsigned");
            p.label("done");
            export(&mut p);
            if rebound {
                assert_eq!(compile(p).unwrap().required_uniform_mask, 1);
            } else {
                assert_bound_error(compile(p));
            }
        }
    }
}

#[test]
fn signed_min_257_and_unbounded_uniform_reject() {
    for bounded in [false, true] {
        let mut p = Program::new();
        init(&mut p);
        if bounded {
            min_signed(&mut p, 257, false);
        } else {
            p.emit(&[0xbea00318]);
        }
        signed_loop(&mut p, false, false);
        export(&mut p);
        assert_bound_error(compile(p));
    }
}

#[test]
fn signed_nonpositive_runtime_min_skips_body_but_preserves_input_dependency() {
    let mut p = Program::new();
    init(&mut p);
    min_signed(&mut p, 0xffffffff, true);
    p.label("header");
    p.emit(&[0xbf028020]);
    p.branch(true, "exit");
    p.emit(&[0xbea2031f, 0x81a08120]); // dead s31 read
    p.branch(false, "header");
    p.label("exit");
    export(&mut p);
    assert_eq!(compile(p).unwrap().required_uniform_mask, 1);
}

#[test]
fn nonpositive_runtime_min_prunes_sampling_without_pruning_liveout_reads() {
    for (cap, pairs) in [(u32::MAX, 0), (4, 1)] {
        let mut p = Program::new();
        init(&mut p);
        p.emit(&[0x7e040280, 0x7e060280]); // initialized sample coordinates v2,v3
        min_signed(&mut p, cap, false);
        p.label("header");
        p.emit(&[0xbf028020]);
        p.branch(true, "exit");
        p.emit(&[0xf09c0f08, 0x00400402]); // original pair0 image_sample_lz
        p.emit(&[0x81a08120]);
        p.branch(false, "header");
        p.label("exit");
        p.emit(&[0xbf061920]); // compare retained counter bits to independent s25
        p.branch(true, "exports");
        p.emit(&[0x7e1002f2]);
        export(&mut p);
        let compiled = compile(p).unwrap();
        assert_eq!(compiled.required_uniform_mask, 3);
        assert_eq!(compiled.sampled_pairs, pairs);
    }
}

#[test]
fn signed_greater_equal_zero_is_not_a_positive_guard() {
    let mut p = Program::new();
    init(&mut p);
    min_signed(&mut p, 4, false);
    p.label("header");
    p.emit(&[0xbf038020]); // GE_I32 permits zero -> -1; outside this contract
    p.branch(true, "exit");
    p.emit(&[0x81a08120]);
    p.branch(false, "header");
    p.label("exit");
    export(&mut p);
    assert!(
        compile(p)
            .unwrap_err()
            .to_string()
            .contains("loop header requires")
    );
}

#[test]
fn mixed_signed_outer_unsigned_child_rejects_at_child_bound() {
    let mut p = Program::new();
    init(&mut p);
    min_signed(&mut p, 4, false);
    p.emit(&[0x8321ff19, u32::MAX]); // retained s33=MIN_I32(s25,-1)
    p.label("header");
    p.emit(&[0xbf028020]);
    p.branch(true, "exit");
    let child_offset = p.words.len() * 4;
    p.label("child");
    p.emit(&[0xbf078021]);
    p.branch(true, "child_exit");
    p.emit(&[0x80a18121]);
    p.branch(false, "child");
    p.label("child_exit");
    p.emit(&[0x81a08120]);
    p.branch(false, "header");
    p.label("exit");
    export(&mut p);
    let error = compile(p).unwrap_err();
    assert!(
        matches!(error, xenora_shader::CompileError::GraphicsStage { error, .. }
        if matches!(*error, xenora_shader::CompileError::Verify { offset, reason }
            if offset == child_offset && reason == "loop trip count lacks a proven bound at most 256"))
    );
}

#[test]
fn original_shared_min_streams_are_bounded_in_both_domains() {
    for signed in [false, true] {
        for known in [false, true] {
            for diamond in [false, true] {
                let mut p = Program::new();
                init(&mut p);
                let opcode = if signed { 6u32 } else { 7 };
                let min = |dst: u32, a: u32, b: u32| {
                    0x80000000 | (opcode << 23) | (dst << 16) | (b << 8) | a
                };
                p.emit(&[min(32, 24, if known { 0x84 } else { 25 })]);
                for _ in 0..50 {
                    if diamond {
                        p.emit(&[min(33, 32, 26), min(34, 32, 27), min(32, 33, 34)]);
                    } else {
                        p.emit(&[min(32, 32, 32)]);
                    }
                }
                p.label("header");
                p.emit(&[if signed { 0xbf028020 } else { 0xbf078020 }]);
                p.branch(true, "exit");
                p.emit(&[0x80a08120]);
                p.branch(false, "header");
                p.label("exit");
                export(&mut p);
                assert!(p.words.len() * 4 <= 1024);
                if known {
                    assert_eq!(
                        compile(p).unwrap().required_uniform_mask,
                        if diamond { 13 } else { 1 }
                    );
                } else {
                    assert_bound_error(compile(p));
                }
            }
        }
    }
}

#[test]
fn scc1_zero_trip_unsigned_signed_and_runtime_caps_skip_body() {
    // Exercise both operand orders and zero/negative constants, plus runtime
    // MIN bounds proving no iterations without replacing the actual counter.
    for (compare, initial, runtime) in [
        (0xbf068020, 0u32, false),
        (0xbf062080, 0, false),
        (0xbf058020, 0, false),
        (0xbf032080, 0, false),
        (0xbf058020, u32::MAX, false),
        (0xbf032080, 0x80000000, false),
        (0xbf068020, 0, true),
        (0xbf058020, 0, true),
        (0xbf058020, u32::MAX, true),
        (0xbf032080, 0x80000000, true),
    ] {
        let mut p = Program::new();
        init(&mut p);
        if runtime {
            let unsigned = compare == 0xbf068020;
            p.emit(&[if unsigned { 0x83a0ff18 } else { 0x8320ff18 }, initial]);
        } else {
            p.emit(&[0xbea003ff, initial]);
        }
        p.label("header");
        p.emit(&[compare]);
        p.branch(true, "exit");
        *p.words.last_mut().unwrap() = 0xbf850000;
        p.emit(&[0xbea2031f]); // Poison: read uninitialized s31 if body executes.
        p.emit(&[0x80a08120]);
        p.branch(false, "header");
        p.label("exit");
        // SCC must remain true after the taken exit.
        p.branch(true, "bad");
        p.branch(false, "exports");
        p.label("bad");
        p.emit(&[0xbea2031f]);
        export(&mut p);
        let compiled = compile(p).expect("proven SCC1 zero trip skips poison and retains SCC");
        assert_eq!(compiled.required_uniform_mask, u8::from(runtime));
    }
}
