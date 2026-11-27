//! Ties declared forms to checked-in fixtures. Most use LLVM mnemonics;
//! lane-read-e64-spec uses explicitly reviewed bytes, not an LLVM encoding oracle.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use xenora_decoder::{Architecture, SEMANTIC_FORMS, decode};
use xenora_isa::meta::{FORMS, identify};

#[test]
fn declared_semantic_forms_are_exactly_those_decoded_from_fixtures() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rdna2");
    let mut decoded = BTreeSet::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|extension| extension != "bin") {
            continue;
        }
        let binary = fs::read(&path).unwrap();
        for instruction in decode(Architecture::Gfx1030, &binary).unwrap() {
            let found = identify(&binary[instruction.offset..]).unwrap();
            assert_eq!(
                found.len,
                usize::from(instruction.byte_len),
                "{}",
                path.display()
            );
            decoded.insert((found.form.name, found.form.family));
        }
    }
    let declared: BTreeSet<_> = SEMANTIC_FORMS.into_iter().collect();
    assert!(
        decoded.is_subset(&declared),
        "{:?}",
        decoded.difference(&declared)
    );
    // Sampling now has an original LLVM-assembled graphics fixture as well.
    // Every declared form must be represented by the checked-in fixture corpus.
    assert_eq!(decoded, declared);
    for (name, family) in SEMANTIC_FORMS {
        assert!(
            FORMS
                .iter()
                .any(|form| form.name == name && form.family == family),
            "{name}"
        );
    }
}

#[test]
fn registry_forms_do_not_duplicate_hand_written_forms() {
    // A form declared in the semantic registry and also decoded by hand-written
    // code would have two implementations; the hand-written one would win.
    let unique: BTreeSet<_> = SEMANTIC_FORMS.into_iter().collect();
    assert_eq!(unique.len(), SEMANTIC_FORMS.len());
}
