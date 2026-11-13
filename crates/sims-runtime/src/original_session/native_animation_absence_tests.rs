use super::*;
use sims_content::Resource;
use sims_formats::animation::{Animation, Samples};
use sims_vm::program::AnimationAsset;

fn index(entries: &[(&str, Vec<u8>)]) -> Index {
    Index {
        root: "absent-install".into(),
        resources: entries
            .iter()
            .map(|(name, bytes)| Resource {
                locator: (*name).into(),
                name: (*name).into(),
                source: "absent-install".into(),
                offset: 0,
                size: bytes.len() as u64,
                unpacked_size: bytes.len() as u64,
                archived: false,
                owned: Some(bytes.clone().into()),
            })
            .collect(),
        archives: 0,
        diagnostics: Vec::new(),
    }
}

fn proofs(index: &Index) -> BTreeMap<String, String> {
    let names = ["missing".into()].into();
    let report =
        sims_content::assets::load_named_native_animations(index, &mut Program::default(), &names)
            .unwrap();
    assert_eq!(report.confirmed_absent.len(), 1);
    report.confirmed_absent
}

fn asset() -> AnimationAsset {
    AnimationAsset {
        source: "synthetic".into(),
        content_hash: "synthetic".into(),
        samples_source: "synthetic".into(),
        samples_hash: "synthetic".into(),
        clip: Animation {
            name: "missing".into(),
            skill: "missing".into(),
            duration_ms: 1000.,
            distance: 0.,
            moving: false,
            translation_count: 0,
            rotation_count: 0,
            frames: 30,
            motions: Vec::new(),
            events: Vec::new(),
        },
        samples: Samples {
            translations: Vec::new(),
            rotations: Vec::new(),
        },
    }
}

#[test]
fn native_animation_absences_restore_before_snapshot_fingerprint_validation() {
    let index = index(&[("inventory.cmx.bcf", vec![0; 12])]);
    let proofs = proofs(&index);
    let program = Program {
        native_animation_absences: proofs.clone(),
        ..Default::default()
    };
    let vm = sims_vm::Vm::new(sims_vm::World::default(), 1);
    let snapshot = sims_vm::Snapshot::capture(&program, &vm);
    let mut restored = Program::default();
    assert!(snapshot.clone().restore(&restored).is_err());
    restore_native_animation_absences(&index, &mut restored, &proofs).unwrap();
    assert_eq!(restored.native_animation_absences, proofs);
    assert_eq!(restored.fingerprint(), program.fingerprint());
    assert_eq!(snapshot.restore(&restored).unwrap(), vm);
}

#[test]
fn native_animation_absences_restore_rejects_changed_inventory_or_proof() {
    let original = index(&[("inventory.cmx.bcf", vec![0; 12])]);
    let expected = proofs(&original);
    let changed = index(&[
        ("inventory.cmx.bcf", vec![0; 12]),
        ("additional.cmx.bcf", vec![0; 12]),
    ]);
    assert_ne!(proofs(&changed), expected);
    let mut program = Program::default();
    assert!(restore_native_animation_absences(&changed, &mut program, &expected).is_err());
    assert!(program.native_animation_absences.is_empty());
    let mut tampered = expected.clone();
    tampered.insert("missing".into(), "0".repeat(64));
    assert!(restore_native_animation_absences(&original, &mut program, &tampered).is_err());
    assert!(program.native_animation_absences.is_empty());
}

#[test]
fn native_animation_absences_restore_rejects_loaded_and_retained_conflicts() {
    let index = index(&[("inventory.cmx.bcf", vec![0; 12])]);
    let expected = proofs(&index);
    let mut program = Program {
        animations: [("missing".into(), asset())].into(),
        ..Default::default()
    };
    assert!(restore_native_animation_absences(&index, &mut program, &expected).is_err());
    assert!(program.native_animation_absences.is_empty());
    program.animations.clear();
    program.native_animation_absences = [("other".into(), "a".repeat(64))].into();
    assert!(restore_native_animation_absences(&index, &mut program, &expected).is_err());
    assert!(restore_native_animation_absences(&index, &mut program, &BTreeMap::new()).is_err());
}

#[test]
fn native_animation_absences_restore_rejects_incomplete_metadata_scan() {
    let original = index(&[("inventory.cmx.bcf", vec![0; 12])]);
    let expected = proofs(&original);
    let malformed = index(&[("inventory.cmx.bcf", vec![0; 9])]);
    let mut program = Program::default();
    assert!(restore_native_animation_absences(&malformed, &mut program, &expected).is_err());
    assert!(program.native_animation_absences.is_empty());
    restore_native_animation_absences(&malformed, &mut program, &BTreeMap::new()).unwrap();
}

#[test]
fn native_animation_absences_decoder_rejects_duplicates_and_malformed_proofs() {
    for json in [
        format!(
            r#"{{"missing":"{}","missing":"{}"}}"#,
            "a".repeat(64),
            "a".repeat(64)
        ),
        format!(r#"{{"Missing":"{}"}}"#, "a".repeat(64)),
        format!(r#"{{"":"{}"}}"#, "a".repeat(64)),
        format!(r#"{{"missing":"{}"}}"#, "A".repeat(64)),
        r#"{"missing":"short"}"#.into(),
    ] {
        let mut deserializer = serde_json::Deserializer::from_str(&json);
        assert!(
            native_animation_absences(&mut deserializer).is_err(),
            "{json}"
        );
    }
    let json = format!(r#"{{"missing":"{}"}}"#, "a".repeat(64));
    let mut deserializer = serde_json::Deserializer::from_str(&json);
    assert_eq!(
        native_animation_absences(&mut deserializer).unwrap(),
        [("missing".into(), "a".repeat(64))].into()
    );
}
