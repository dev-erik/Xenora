//! Ties the generated semantic registry tables to the independent XML metadata table.

use xenora_isa::meta::{EncodingFamily, FORMS as META_FORMS};
use xenora_isa::semantics::{CarryIn, FORM_IDS, FORMS, GENERATED_SCHEMA, SCHEMA, SccEffect};
use xenora_isa::{ScalarAluOp, ScalarCompareOp, ScalarUnaryOp, VectorAluOp, VectorUnaryOp};

#[test]
fn generated_tables_match_the_schema_and_the_metadata_forms() {
    assert_eq!(GENERATED_SCHEMA, SCHEMA);
    assert_eq!(FORM_IDS.len(), FORMS.len());
    for (row, (name, family)) in FORMS.iter().zip(FORM_IDS) {
        assert_eq!((row.name, row.family), (name, family));
        let meta = META_FORMS
            .iter()
            .find(|form| form.name == row.name && form.family == row.family)
            .unwrap_or_else(|| panic!("{} is not an identified form", row.name));
        assert_eq!(meta.opcode, row.opcode, "{}", row.name);
    }
}

/// Every word of `family` that the XML metadata identifies selects an operation
/// exactly when the registry declares that form, and no 32-bit word of another
/// family selects one.
fn dispatch_selects_exactly(family: EncodingFamily, select: impl Fn(u32) -> Option<&'static str>) {
    let declared: Vec<_> = FORMS.iter().filter(|row| row.family == family).collect();
    let mut selected = 0;
    for meta in META_FORMS.iter().filter(|form| form.family == family) {
        match select(meta.bits as u32) {
            Some(name) => {
                assert_eq!(name, meta.name);
                selected += 1;
            }
            None => assert!(
                declared.iter().all(|row| row.name != meta.name),
                "{} is declared but not dispatched",
                meta.name
            ),
        }
    }
    assert_eq!(selected, declared.len());
    for meta in META_FORMS
        .iter()
        .filter(|form| form.family != family && form.bytes == 4)
    {
        assert_eq!(select(meta.bits as u32), None, "{}", meta.name);
    }
}

#[test]
fn scalar_alu_dispatch_selects_exactly_the_registry_sop2_forms() {
    for op in ScalarAluOp::ALL {
        let row = op.semantics();
        assert_eq!(row.family, EncodingFamily::Sop2);
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(op.reads_scc(), row.carry_in == CarryIn::Scc);
        assert_eq!(op.writes_scc(), row.scc != SccEffect::Unchanged);
    }
    dispatch_selects_exactly(EncodingFamily::Sop2, |word| {
        ScalarAluOp::from_word(word).map(|op| op.semantics().name)
    });
}

#[test]
fn scalar_compare_dispatch_selects_exactly_the_registry_sopc_forms() {
    for op in ScalarCompareOp::ALL {
        let row = op.semantics();
        assert_eq!(row.family, EncodingFamily::Sopc);
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(row.relation, Some(op.relation()));
    }
    dispatch_selects_exactly(EncodingFamily::Sopc, |word| {
        ScalarCompareOp::from_word(word).map(|op| op.semantics().name)
    });
}

#[test]
fn scalar_unary_dispatch_selects_exactly_the_registry_sop1_forms() {
    for op in ScalarUnaryOp::ALL {
        let row = op.semantics();
        assert_eq!(row.family, EncodingFamily::Sop1);
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(op.writes_scc(), row.scc != SccEffect::Unchanged);
    }
    // s_mov_b32 (opcode 3) stays hand-written, so it must not be dispatched here.
    dispatch_selects_exactly(EncodingFamily::Sop1, |word| {
        ScalarUnaryOp::from_word(word).map(|op| op.semantics().name)
    });
}

#[test]
fn vector_alu_dispatch_selects_exactly_the_registry_vop2_forms() {
    for op in VectorAluOp::ALL {
        let row = op.semantics();
        assert_eq!(row.family, EncodingFamily::Vop2);
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(op.assembly_name(), format!("{}_e32", op.mnemonic()));
        assert_eq!(op.swaps_sources(), row.swap_sources);
        assert_eq!(row.scc, SccEffect::Unchanged);
    }
    // The hand-written VOP2 forms (v_and_b32, v_add_f32, ...) must not be
    // dispatched here.
    dispatch_selects_exactly(EncodingFamily::Vop2, |word| {
        VectorAluOp::from_word(word).map(|op| op.semantics().name)
    });
}

#[test]
fn scalar_pack_halfwords_and_scc() {
    // Four distinct halfwords expose source reversal and wrong-half selection.
    for (opcode, expected) in [
        (0x32, 0x3210_cdef),
        (0x33, 0x7654_cdef),
        (0x34, 0x7654_89ab),
    ] {
        let op = ScalarAluOp::from_word(0x8000_0000 | (opcode << 23)).unwrap();
        assert!(!op.reads_scc());
        assert_eq!(op.semantics().scc, SccEffect::Unchanged);
        for scc in [false, true] {
            assert_eq!(op.evaluate(0x89ab_cdef, 0x7654_3210, scc), (expected, None));
            assert_eq!(op.evaluate(0, 0, scc), (0, None));
            assert_eq!(op.evaluate(u32::MAX, u32::MAX, scc), (u32::MAX, None));
        }
    }
}

#[test]
fn vector_unary_dispatch_and_value_contract() {
    assert_eq!(VectorUnaryOp::ALL.len(), 5);
    for op in VectorUnaryOp::ALL {
        let row = op.semantics();
        assert_eq!(row.family, EncodingFamily::Vop1);
        assert_eq!(row.name.to_ascii_lowercase(), op.mnemonic());
        assert_eq!(op.assembly_name(), format!("{}_e32", op.mnemonic()));
        assert_eq!(row.scc, SccEffect::Unchanged);
        assert_eq!(row.carry_in, CarryIn::None);
    }
    dispatch_selects_exactly(EncodingFamily::Vop1, |word| {
        VectorUnaryOp::from_word(word).map(|op| op.semantics().name)
    });
    use VectorUnaryOp::*;
    for (input, expected) in [
        (0, [0xffff_ffff, 0, 0xffff_ffff, 0xffff_ffff, 0xffff_ffff]),
        (0xffff_ffff, [0, 0xffff_ffff, 0, 0, 0xffff_ffff]),
        (1, [0xffff_fffe, 0x8000_0000, 31, 0, 31]),
        (0x8000_0000, [0x7fff_ffff, 1, 0, 31, 1]),
        (0x4000_0000, [0xbfff_ffff, 2, 1, 30, 1]),
        (0xffff_fffe, [1, 0x7fff_ffff, 0, 1, 31]),
        (0xffff_0000, [0x0000_ffff, 0x0000_ffff, 0, 16, 16]),
    ] {
        for (op, value) in [NotB32, BfrevB32, FfbhU32, FfblB32, FfbhI32]
            .into_iter()
            .zip(expected)
        {
            assert_eq!(op.evaluate(input), value, "{op:?}({input:08x})");
        }
    }
    assert_eq!(BfrevB32.evaluate(0x0123_4567), 0xe6a2_c480);
}
