// SPDX-License-Identifier: GPL-3.0-or-later
//! Independent instruction words for scalar conditional admission and CFG checks.
use std::collections::HashMap;
use xenora_shader::{
    Architecture::Gfx1030, CompileError, CompiledGraphics, GraphicsPipelineTypes,
    GraphicsPipelineTypes4, compile_graphics_pipeline_typed, compile_graphics_pipeline_typed4,
};
const VS: &[u8] = include_bytes!("../../../fixtures/rdna2/x9-constant-vs.bin");
const UNDEFINED: &str = "graphics scalar local is uninitialized";
const SCC: &str = "SCC is uninitialized";

#[derive(Default)]
struct Program {
    words: Vec<u32>,
    labels: HashMap<usize, usize>,
    branches: Vec<(usize, usize)>,
    next_label: usize,
}
impl Program {
    fn emit(&mut self, words: &[u32]) {
        self.words.extend_from_slice(words);
    }
    fn label_id(&mut self) -> usize {
        let id = self.next_label;
        self.next_label += 1;
        id
    }
    fn label(&mut self, id: usize) {
        assert!(self.labels.insert(id, self.words.len()).is_none());
    }
    fn branch(&mut self, conditional: bool, target: usize) {
        self.branches.push((self.words.len(), target));
        self.words
            .push(if conditional { 0xbf840000 } else { 0xbf820000 });
    }
    fn literal(&mut self, dst: u32, bits: u32) {
        self.emit(&[mov(dst, 255), bits]);
    }
    fn flag(&mut self, value: bool) {
        self.emit(&[if value { 0xbf068080 } else { 0xbf068180 }]);
    }
    // A wrong compile-time flag reaches a forbidden pixel v0 write.
    fn expect_flag(&mut self, value: bool) {
        let good = self.label_id();
        if value {
            let bad = self.label_id();
            self.branch(true, bad);
            self.branch(false, good);
            self.label(bad);
        } else {
            self.branch(true, good);
        }
        self.emit(&[0x7e000280]);
        self.label(good);
    }
    fn expect_word(&mut self, reg: u32, bits: u32) {
        self.emit(&[0xbf06ff00 | reg, bits]);
        self.expect_flag(true);
    }
    fn counted(&mut self, counter: u32, signed: bool, body: impl FnOnce(&mut Self, usize)) {
        let header = self.label_id();
        let exit = self.label_id();
        self.label(header);
        self.emit(&[(if signed { 0xbf028000 } else { 0xbf078000 }) | counter]);
        self.branch(true, exit);
        body(self, exit);
        self.emit(&[0x80808100 | counter << 16 | counter]); // SUB_U32 counter,counter,1
        self.branch(false, header);
        self.label(exit);
    }
    fn bytes(mut self) -> Vec<u8> {
        self.emit(&[0x7e100280, 0xf800180f, 0x08080808, 0xbf810000]);
        for (at, target) in self.branches {
            let delta = i16::try_from(self.labels[&target] as isize - at as isize - 1).unwrap();
            self.words[at] |= u32::from(delta as u16);
        }
        self.words.into_iter().flat_map(u32::to_le_bytes).collect()
    }
}
fn mov(dst: u32, src: u32) -> u32 {
    0xbe800300 | dst << 16 | src
}
fn cmov(dst: u32, src: u32) -> u32 {
    0xbe800500 | dst << 16 | src
}
fn cmovk(dst: u32, value: i16) -> u32 {
    0xb1000000 | dst << 16 | u32::from(value as u16)
}
fn bitset(dst: u32, index: u32, value: bool) -> u32 {
    0xbe800000 | dst << 16 | (if value { 29 } else { 27 }) << 8 | index
}
fn bitcmp(src: u32, index: u32, value: bool) -> u32 {
    0xbf000000 | (if value { 13 } else { 12 }) << 16 | index << 8 | src
}
fn compile(p: Program) -> Result<CompiledGraphics, CompileError> {
    compile_graphics_pipeline_typed(Gfx1030, VS, &p.bytes(), GraphicsPipelineTypes::default())
}
fn rejects(p: Program, message: &str) {
    let error = compile(p).unwrap_err();
    assert!(
        error.to_string().contains(message),
        "expected {message}: {error}"
    );
}

#[test]
fn known_conditions_preserve_lazy_initialization_and_flags() {
    compile(Program::default()).unwrap();
    for immediate in [false, true] {
        let mut fresh = Program::default();
        fresh.flag(true);
        fresh.emit(&[if immediate {
            cmovk(32, -1)
        } else {
            cmov(32, 193)
        }]);
        fresh.expect_flag(true);
        fresh.expect_word(32, u32::MAX);
        compile(fresh).unwrap();

        let mut false_undefined = Program::default();
        false_undefined.flag(false);
        false_undefined.emit(&[if immediate {
            cmovk(32, -1)
        } else {
            cmov(32, 33)
        }]);
        false_undefined.expect_flag(false);
        compile(false_undefined).unwrap();

        let mut still_undefined = Program::default();
        still_undefined.flag(false);
        still_undefined.emit(&[
            if immediate {
                cmovk(32, -1)
            } else {
                cmov(32, 33)
            },
            mov(34, 32),
        ]);
        rejects(still_undefined, UNDEFINED);

        let mut unset = Program::default();
        unset.emit(&[if immediate {
            cmovk(32, 0)
        } else {
            cmov(32, 128)
        }]);
        rejects(unset, SCC);
    }
    for source in [32, 33] {
        let mut p = Program::default();
        p.flag(true);
        p.emit(&[cmov(32, source)]);
        rejects(p, UNDEFINED);
        let mut p = Program::default();
        p.flag(false);
        p.emit(&[cmov(32, source)]);
        compile(p).unwrap();
    }
    let mut p = Program::default();
    p.literal(32, 0x89abcdef);
    p.flag(false);
    p.emit(&[cmov(32, 33), cmovk(32, 0)]);
    p.expect_flag(false);
    p.expect_word(32, 0x89abcdef);
    compile(p).unwrap();
}

#[test]
fn cmovk_sign_extension_and_defined_aliases() {
    for (immediate, expected) in [
        (0, 0),
        (1, 1),
        (32767, 32767),
        (-32768, 0xffff8000),
        (-1, 0xffffffff),
    ] {
        let mut p = Program::default();
        p.flag(true);
        p.emit(&[cmovk(105, immediate)]);
        p.expect_flag(true);
        p.expect_word(105, expected);
        p.flag(true);
        p.emit(&[cmov(105, 105)]);
        p.expect_word(105, expected);
        compile(p).unwrap();
    }
}

#[test]
fn runtime_condition_requires_both_alternatives_and_tracks_exact_uniforms() {
    for mode in 0..3 {
        let mut p = Program::default();
        p.literal(32, 7);
        match mode {
            0 => p.flag(false),
            1 => p.flag(true),
            _ => p.emit(&[bitcmp(24, 128, true)]),
        }
        p.emit(&[cmov(32, 31)]);
        assert_eq!(
            compile(p).unwrap().required_uniform_mask,
            [0, 128, 129][mode]
        );
    }
    for (old_defined, source_defined) in
        [(false, true), (true, false), (false, false), (true, true)]
    {
        let mut p = Program::default();
        if old_defined {
            p.literal(32, 7);
        }
        if source_defined {
            p.literal(33, 9);
        }
        p.emit(&[bitcmp(24, 128, true), cmov(32, 33)]);
        if old_defined && source_defined {
            assert_eq!(compile(p).unwrap().required_uniform_mask, 1);
        } else {
            rejects(p, UNDEFINED);
        }
    }
    let mut p = Program::default();
    p.emit(&[bitcmp(24, 128, true), cmovk(32, -1)]);
    rejects(p, UNDEFINED);
    let mut p = Program::default();
    p.literal(32, 7);
    p.emit(&[bitcmp(24, 128, true), cmovk(32, -1)]);
    assert_eq!(compile(p).unwrap().required_uniform_mask, 1);
}

#[test]
fn invalid_source_and_destination_classes_reject_even_when_false() {
    for src in [0, 23] {
        let mut p = Program::default();
        p.flag(false);
        p.emit(&[cmov(32, src)]);
        rejects(p, "conditional move source must be a uniform or local");
    }
    for dst in [0, 23, 24, 31] {
        for op in [cmov(dst, 128), cmovk(dst, 0)] {
            let mut p = Program::default();
            p.flag(false);
            p.emit(&[op]);
            rejects(p, "destination must be a local");
        }
    }
    for src in [106, 107, 124, 126, 127] {
        let mut p = Program::default();
        p.flag(false);
        p.emit(&[cmov(32, src)]);
        rejects(p, "expected a normal SGPR");
    }
}

#[test]
fn bit_updates_keep_scc_and_capture_old_destination_before_alias_writes() {
    for value in [false, true] {
        for flag in [None, Some(false), Some(true)] {
            let mut p = Program::default();
            p.literal(32, 31);
            if let Some(flag) = flag {
                p.flag(flag);
            }
            p.emit(&[bitset(32, 32, value)]);
            if let Some(flag) = flag {
                p.expect_flag(flag);
            } else {
                let target = p.label_id();
                p.branch(true, target);
                p.label(target);
                rejects(p, SCC);
                continue;
            }
            p.expect_word(32, if value { 0x8000001f } else { 31 });
            compile(p).unwrap();
        }
        for index in [0u32, 31, 32, 33, 63, u32::MAX] {
            let mut p = Program::default();
            p.literal(32, if value { 0 } else { u32::MAX });
            p.literal(33, index);
            p.emit(&[bitset(32, 33, value)]);
            let mask = 1u32 << (index % 32);
            p.expect_word(32, if value { mask } else { !mask });
            compile(p).unwrap();
        }
        for initialize_old in [false, true] {
            let mut p = Program::default();
            if initialize_old {
                p.literal(32, 0);
            }
            p.emit(&[bitset(32, 33, value)]);
            rejects(p, UNDEFINED);
        }
        // Isolate an undefined old destination from an undefined index.
        let mut p = Program::default();
        p.emit(&[bitset(32, 128, value)]);
        rejects(p, UNDEFINED);
        let mut p = Program::default();
        p.emit(&[mov(32, 24), bitset(32, 25, value)]);
        assert_eq!(compile(p).unwrap().required_uniform_mask, 3);
    }
}

#[test]
fn bit_comparisons_initialize_replace_scc_and_preserve_sources() {
    for value in [false, true] {
        for bit in 0..32u32 {
            let mut p = Program::default();
            p.literal(32, 1 << bit);
            p.literal(33, bit + 32);
            p.emit(&[bitcmp(32, 33, value)]);
            p.expect_flag(value);
            p.expect_word(32, 1 << bit);
            p.expect_word(33, bit + 32);
            p.emit(&[bitcmp(128, 33, value)]);
            p.expect_flag(!value);
            compile(p).unwrap();
        }
        let mut p = Program::default();
        p.emit(&[bitcmp(24, 25, value)]);
        assert_eq!(compile(p).unwrap().required_uniform_mask, 3);
        let mut p = Program::default();
        p.emit(&[bitcmp(32, 128, value)]);
        rejects(p, UNDEFINED);
    }
}

#[test]
fn zero_trip_skips_undefined_body_but_does_not_define_exit_locals() {
    for signed in [false, true] {
        let mut p = Program::default();
        p.literal(32, 0);
        p.counted(32, signed, |p, _| {
            p.emit(&[bitset(33, 34, true), cmov(35, 36), bitcmp(37, 38, true)])
        });
        p.expect_flag(false);
        assert_eq!(compile(p).unwrap().required_uniform_mask, 0);
        let mut p = Program::default();
        p.literal(32, 0);
        p.counted(32, signed, |p, _| {
            p.flag(true);
            p.emit(&[cmovk(33, 1)]);
        });
        p.emit(&[mov(34, 33)]);
        rejects(p, UNDEFINED);
    }
}

#[test]
fn runtime_diamond_retains_maybe_undefined_and_merges_defined_values() {
    for initialized in [false, true] {
        let mut p = Program::default();
        if initialized {
            p.literal(32, 7);
        }
        p.emit(&[bitcmp(24, 128, true)]);
        let no = p.label_id();
        let join = p.label_id();
        p.branch(true, no);
        p.flag(true);
        p.emit(&[cmovk(32, 9)]);
        p.branch(false, join);
        p.label(no);
        p.flag(false);
        p.emit(&[cmov(32, 33)]);
        p.label(join);
        p.emit(&[mov(34, 32), cmov(34, 129)]); // SCC and local both join here.
        if initialized {
            assert_eq!(compile(p).unwrap().required_uniform_mask, 1);
        } else {
            rejects(p, UNDEFINED);
        }
    }
}

#[test]
fn bounded_loop_body_updates_and_bitcompare_breaks_accept() {
    for signed in [false, true] {
        let mut p = Program::default();
        p.literal(32, 3);
        p.literal(34, 31);
        p.counted(32, signed, |p, exit| {
            p.emit(&[
                bitset(34, 34, true),
                bitcmp(32, 128, true),
                cmov(34, 25),
                cmovk(34, -1),
            ]);
            p.emit(&[bitcmp(24, 128, true)]);
            p.branch(true, exit);
        });
        p.emit(&[mov(35, 34)]);
        assert_eq!(compile(p).unwrap().required_uniform_mask, 3);
    }
}

#[test]
fn any_new_counter_write_rejects_even_false_or_inside_child() {
    for nested in [false, true] {
        for operation in [
            bitset(32, 128, false),
            bitset(32, 128, true),
            cmov(32, 128),
            cmovk(32, 0),
        ] {
            let mut p = Program::default();
            p.literal(32, 2);
            p.counted(32, false, |p, _| {
                if nested {
                    p.literal(33, 2);
                    p.counted(33, false, |p, _| {
                        p.flag(false);
                        p.emit(&[operation]);
                    });
                } else {
                    p.flag(false);
                    p.emit(&[operation]);
                }
            });
            rejects(
                p,
                "loop counter requires exactly one current-value decrement",
            );
        }
    }
    let mut p = Program::default();
    p.literal(32, 2);
    p.literal(34, 7);
    p.counted(32, false, |p, _| {
        p.literal(33, 2);
        p.counted(33, false, |p, _| {
            p.flag(false);
            p.emit(&[cmov(34, 35)]);
        });
    });
    compile(p).unwrap();
}

#[test]
fn conditional_and_bit_writes_invalidate_carried_bounds_unless_reclamped() {
    for operation in [
        bitset(34, 137, true),
        bitset(34, 137, false),
        cmov(34, 35),
        cmovk(34, 512),
    ] {
        for clamp in [false, true] {
            let mut p = Program::default();
            p.literal(32, 2);
            p.literal(34, 2);
            p.literal(35, 512);
            p.counted(32, false, |p, _| {
                if clamp {
                    p.emit(&[0x83800000 | 33 << 16 | 130 << 8 | 34]);
                }
                // MIN_U32 s33,s34,2
                else {
                    p.emit(&[mov(33, 34)]);
                }
                p.counted(33, false, |p, _| p.emit(&[mov(36, 128)]));
                p.flag(true);
                p.emit(&[operation]);
            });
            if clamp {
                compile(p).unwrap();
            } else {
                rejects(p, "loop trip count lacks a proven bound");
            }
        }
    }
}

#[test]
fn bitcompare_does_not_replace_or_interrupt_countdown_guard() {
    for replace in [false, true] {
        let mut p = Program::default();
        p.literal(32, 2);
        let header = p.label_id();
        let exit = p.label_id();
        p.label(header);
        p.emit(&[if replace {
            bitcmp(32, 128, true)
        } else {
            0xbf078020
        }]);
        if !replace {
            p.emit(&[bitcmp(32, 128, true)]);
        }
        p.branch(true, exit);
        p.emit(&[0x80a08120]);
        p.branch(false, header);
        p.label(exit);
        rejects(
            p,
            if replace {
                "loop header requires unsigned counter"
            } else {
                "loop header requires adjacent SCC0 exit"
            },
        );
    }
}

#[test]
fn extended_descriptors_cannot_be_conditionally_written() {
    for dst in [32u32, 55, 60, 68, 71] {
        for operation in [
            bitset(dst, 128, false),
            bitset(dst, 128, true),
            cmov(dst, 24),
            cmovk(dst, 0),
        ] {
            let mut p = Program::default();
            p.flag(false);
            p.emit(&[operation]);
            let error = compile_graphics_pipeline_typed4(
                Gfx1030,
                VS,
                &p.bytes(),
                GraphicsPipelineTypes4::default(),
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("extended descriptor words are reserved"),
                "{error}"
            );
        }
    }
    let mut p = Program::default();
    p.literal(72, 7);
    p.flag(false);
    p.emit(&[cmov(72, 73), cmovk(72, -1)]);
    p.expect_word(72, 7);
    let compiled = compile_graphics_pipeline_typed4(
        Gfx1030,
        VS,
        &p.bytes(),
        GraphicsPipelineTypes4::default(),
    )
    .unwrap();
    assert_eq!(compiled.required_uniform_mask, 0);
}

#[test]
fn true_body_definition_is_local_and_false_body_source_stays_unread() {
    let mut p = Program::default();
    p.literal(32, 2);
    p.literal(34, 7);
    p.counted(32, false, |p, _| {
        p.flag(true);
        p.emit(&[cmovk(33, 9), mov(35, 33)]);
        p.flag(false);
        p.emit(&[cmov(34, 36)]);
    });
    compile(p).unwrap();
    // Reading a possibly unwritten loop local after the header exit is unsafe.
    let mut p = Program::default();
    p.emit(&[0x83a08218]); // MIN_U32 s32,s24,2: may be zero.
    p.counted(32, false, |p, _| {
        p.flag(true);
        p.emit(&[cmovk(33, 9)]);
    });
    p.emit(&[mov(35, 33)]);
    rejects(p, UNDEFINED);
}
