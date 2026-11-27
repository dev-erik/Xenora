//! Instruction identification against LLVM-assembled fixtures and hand-assembled words.

use std::fs;
use std::path::Path;

use xenora_isa::meta::{
    EncodingFamily, Extensions, FORMS, IdentifyError, VectorControl, identify, inventory, lookup,
};

fn words(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn name_of(values: &[u32]) -> (&'static str, usize) {
    let found = identify(&words(values)).unwrap();
    (found.form.name, found.len)
}

/// Expected mnemonics from assembly or explicitly annotated spec-derived words.
/// Raw data directives do not establish LLVM instruction encoding evidence.
fn source_mnemonics(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let instruction = line.split("//").next().unwrap().trim();
            if instruction.starts_with(".long ") {
                let (_, expected) = line
                    .split_once("// spec-instruction:")
                    .expect("raw instruction words require an explicit expected mnemonic");
                let expected = expected.trim();
                assert!(!expected.is_empty() && !expected.chars().any(char::is_whitespace));
                Some(expected)
            } else if instruction.is_empty()
                || instruction.starts_with('.')
                || instruction.ends_with(':')
            {
                None
            } else {
                Some(instruction)
            }
        })
        .map(|line| {
            let mnemonic = line.split_whitespace().next().unwrap().to_ascii_uppercase();
            mnemonic
                .trim_end_matches("_E32")
                .trim_end_matches("_E64")
                .to_owned()
        })
        .collect()
}

#[test]
fn fixtures_identify_with_exact_boundaries_and_mnemonics() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rdna2");
    let mut checked = 0;
    for entry in fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "bin") {
            continue;
        }
        let binary = fs::read(&path).unwrap();
        let source = fs::read_to_string(path.with_extension("s")).unwrap();
        let mut offset = 0;
        let mut identified = Vec::new();
        while offset < binary.len() {
            let found = identify(&binary[offset..])
                .unwrap_or_else(|error| panic!("{} at {offset:#x}: {error}", path.display()));
            identified.push(found.form.name.to_owned());
            offset += found.len;
        }
        assert_eq!(offset, binary.len(), "{}", path.display());
        assert_eq!(identified, source_mnemonics(&source), "{}", path.display());
        checked += 1;
    }
    assert!(checked >= 11, "only {checked} fixtures found");
}

#[test]
fn spec_annotations_are_checked_against_identified_bytes() {
    let expected =
        source_mnemonics(".long 0xd5820008, 0x101 // spec-instruction: V_READFIRSTLANE_B32");
    assert_eq!(expected, [name_of(&[0xd5820008, 0x101]).0]);
    let wrong = source_mnemonics(".long 0xd5820008, 0x101 // spec-instruction: V_READLANE_B32");
    assert_ne!(wrong, expected);
}

#[test]
fn hand_assembled_words_identify() {
    // SOPP s_endpgm (opcode 1), SOP1 s_mov_b32 s0, s1 (opcode 3).
    assert_eq!(name_of(&[0xbf81_0000]), ("S_ENDPGM", 4));
    assert_eq!(name_of(&[0xbe80_0301]), ("S_MOV_B32", 4));
    // VOP1 v_mov_b32 v0, v1: SRC0 = 256 + 1.
    assert_eq!(name_of(&[0x7e00_0301]), ("V_MOV_B32", 4));
    // VOP2 v_add_f32 v0, v1, v2: opcode 3 at bits 30:25, VSRC1 = 2 at bits 16:9.
    assert_eq!(name_of(&[0x0600_0501]), ("V_ADD_F32", 4));
    // VOP3 v_add_f32_e64 v0, v1, v2 (LLVM 23 encoding): opcode 259 = 256 + VOP2 opcode 3.
    assert_eq!(name_of(&[0xd503_0000, 0x0202_0501]), ("V_ADD_F32", 8));
}

#[test]
fn literal_sdwa_and_dpp_words_extend_the_length() {
    // v_mov_b32 v0, 0x12345678: SRC0 = 255 selects a literal.
    let literal = identify(&words(&[0x7e00_02ff, 0x1234_5678])).unwrap();
    assert_eq!((literal.form.name, literal.len), ("V_MOV_B32", 8));
    assert!(literal.trailer.literal);
    // s_add_u32 s0, s1, literal: SSRC1 = 255.
    assert_eq!(name_of(&[0x8000_ff01, 0x0000_1234]), ("S_ADD_U32", 8));
    // VOP3 with SRC2 = 255.
    let fma = identify(&words(&[0xd54b_0000, 0x03fe_0501, 0x3f80_0000]));
    assert!(matches!(fma, Ok(found) if found.len == 12 && found.trailer.literal));
    let sdwa = identify(&words(&[0x7e00_02f9, 0x0006_0601])).unwrap();
    assert_eq!(sdwa.trailer.vector_control, Some(VectorControl::Sdwa));
    assert_eq!(sdwa.len, 8);
    let dpp16 = identify(&words(&[0x7e00_02fa, 0x00ff_0001])).unwrap();
    assert_eq!(dpp16.trailer.vector_control, Some(VectorControl::Dpp16));
    let dpp8 = identify(&words(&[0x7e00_02ea, 0x00fa_c688])).unwrap();
    assert_eq!(
        dpp8.trailer.vector_control,
        Some(VectorControl::Dpp8 {
            fetch_inactive: true
        })
    );
}

#[test]
fn vop3_interpolation_attribute_fields_never_select_a_literal() {
    // AMD RDNA 2 ISA guide section 10.4.2: in the VOP3 interpolation forms SRC0
    // holds {attr_word, attr_chan, attr} bit-fields (XML OPR_ATTR), so SRC0 = 255
    // is attr 63, channel w (and attr_word 0) and the instruction is 8 bytes.
    // LLVM 23.1.6 agrees (#235).
    for (first, name) in [
        (0xd600_0000, "V_INTERP_P1_F32"),
        (0xd601_0000, "V_INTERP_P2_F32"),
        (0xd602_0000, "V_INTERP_MOV_F32"),
        (0xd742_0000, "V_INTERP_P1LL_F16"),
        (0xd743_0000, "V_INTERP_P1LV_F16"),
        (0xd75a_0000, "V_INTERP_P2_F16"),
    ] {
        let found = identify(&words(&[first, 0x0000_00ff, 0x1234_5678])).unwrap();
        assert_eq!((found.form.name, found.len), (name, 8));
        assert!(!found.trailer.literal, "{name}");
    }
    // A general VOP3 source with the same selector still takes the literal.
    let add = identify(&words(&[0xd503_0000, 0x0000_00ff, 0x1234_5678])).unwrap();
    assert_eq!((add.form.name, add.len), ("V_ADD_F32", 12));
    // So does a VGPR-only source (LLVM reports these 12 bytes with an invalid operand).
    let permlane = identify(&words(&[0xd777_0000, 0x0000_00ff, 0x1234_5678])).unwrap();
    assert_eq!((permlane.form.name, permlane.len), ("V_PERMLANE16_B32", 12));
}

#[test]
fn truncated_and_unassigned_inputs_are_rejected() {
    assert_eq!(identify(&[]), Err(IdentifyError::Truncated { needed: 4 }));
    assert_eq!(
        identify(&[0x00, 0x00, 0x81]),
        Err(IdentifyError::Truncated { needed: 4 })
    );
    // VOP3 first dword alone.
    assert_eq!(
        identify(&words(&[0xd503_0000])),
        Err(IdentifyError::Truncated { needed: 8 })
    );
    // Literal selected but absent.
    assert_eq!(
        identify(&words(&[0x7e00_02ff])),
        Err(IdentifyError::Truncated { needed: 8 })
    );
    // SOPP opcodes are 7 bits; this table has no SOPP opcode 127.
    assert!(lookup(0xbf80_0000 | (127 << 16)).is_none());
    assert_eq!(
        identify(&words(&[0xbfff_0000])),
        Err(IdentifyError::Unassigned)
    );
}

#[test]
fn extensions_outside_a_forms_metadata_are_rejected() {
    let without_sdwa = FORMS
        .iter()
        .find(|form| {
            form.family == EncodingFamily::Vop1 && !form.extensions.contains(Extensions::SDWA)
        })
        .expect("a VOP1 form without SDWA");
    let word = (without_sdwa.bits as u32) | 0xf9;
    assert_eq!(
        identify(&words(&[word, 0])),
        Err(IdentifyError::InvalidExtension("sdwa"))
    );
}

#[test]
fn every_form_identifies_as_itself() {
    for form in &FORMS {
        assert_eq!(lookup(form.bits).map(|found| found.name), Some(form.name));
        assert_eq!(
            lookup(form.bits).unwrap().family,
            form.family,
            "{}",
            form.name
        );
    }
    let total: usize = inventory().iter().map(|(_, count)| count).sum();
    assert_eq!(total, FORMS.len());
}

#[test]
fn documented_form_counts_match_the_generated_table() {
    let count = FORMS.len();
    let count = format!("{},{:03}", count / 1000, count % 1000);
    let families = EncodingFamily::ALL.len();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, claim) in [
        (
            "docs/research/rdna2.md",
            format!("{count} forms in {families} encoding families"),
        ),
        (
            "CHANGELOG.md",
            format!("{count} base forms across {families} encoding families"),
        ),
    ] {
        let text = fs::read_to_string(root.join(file)).unwrap();
        assert!(
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(&claim),
            "{file} must state \"{claim}\""
        );
    }
}

#[test]
fn first_word_opcode_table_agrees_with_full_metadata() {
    use xenora_isa::{BUFFER_LOAD_DWORD, S_ENDPGM, V_ADD_F32, V_ADD_NC_U32, V_MOV_B32};
    for (opcode, name, family) in [
        (
            BUFFER_LOAD_DWORD,
            "BUFFER_LOAD_DWORD",
            EncodingFamily::Mubuf,
        ),
        (S_ENDPGM, "S_ENDPGM", EncodingFamily::Sopp),
        (V_ADD_F32, "V_ADD_F32", EncodingFamily::Vop2),
        (V_ADD_NC_U32, "V_ADD_NC_U32", EncodingFamily::Vop3),
        (V_MOV_B32, "V_MOV_B32", EncodingFamily::Vop1),
    ] {
        let form = FORMS
            .iter()
            .find(|form| form.name == name && form.family == family)
            .unwrap();
        assert!(opcode.matches(form.bits as u32), "{name}");
    }
}
