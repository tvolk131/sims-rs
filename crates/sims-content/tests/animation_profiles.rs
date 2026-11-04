//! Fixed Global animation-cache dependencies, including the swimming profiles.
use sims_content::{
    Index,
    vm::{self, NATIVE_ANIMATION_TABLES},
};
use sims_vm::{
    Program,
    animation::{NameTable, posture_name},
};
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);
impl Fixture {
    fn new(chunks: Vec<(u16, Vec<u8>)>) -> Self {
        let root = std::env::temp_dir().join(format!(
            "sims-animation-profiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let mut bytes = vec![0; 64];
        bytes[..13].copy_from_slice(b"IFF FILE 2.5:");
        for (id, data) in chunks {
            bytes.extend(b"STR#");
            bytes.extend((76 + data.len() as u32).to_be_bytes());
            bytes.extend(id.to_be_bytes());
            bytes.extend([0; 66]);
            bytes.extend(data);
        }
        fs::write(root.join("Global.iff"), bytes).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn strings(value: &str) -> Vec<u8> {
    let mut b = (-1i16).to_le_bytes().to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(value.bytes());
    b.push(0);
    b
}
fn lookup(program: &Program, special: i16, species: i16) -> Option<&str> {
    posture_name(27, special, species, 0, |table, i| match table {
        NameTable::Global(id) => program.modules[&program.globals]
            .strings
            .get(&id)?
            .get(i)
            .map(String::as_str),
        NameTable::Private(_) => None,
    })
}

#[test]
fn links_every_native_fixed_profile_and_hydrates_an_exact_old_catalog() {
    let temp = Fixture::new(
        NATIVE_ANIMATION_TABLES
            .into_iter()
            .map(|id| (id, strings(&format!("profile-{id}"))))
            .collect(),
    );
    let index = Index::scan(&temp.0).unwrap();
    let mut p = vm::load_program(&index, "Global.iff", "Global.iff").unwrap();
    assert_eq!(p.modules[&p.globals].strings.len(), 13);
    assert_eq!(lookup(&p, 0, 0), Some("profile-152"));
    assert_eq!(lookup(&p, 1, 0), Some("profile-158"));
    assert_eq!(lookup(&p, 1, 1), Some("profile-159"));
    let expected = p.fingerprint();
    for id in vm::PREVIOUSLY_UNLINKED_ANIMATION_TABLES {
        p.modules.get_mut("Global.iff").unwrap().strings.remove(&id);
    }
    assert_eq!(lookup(&p, 1, 0), Some("profile-150"));
    assert_eq!(
        vm::prepare_native_animation_tables(&index, &mut p).unwrap(),
        5
    );
    assert_eq!(p.fingerprint(), expected);
    assert_eq!(
        vm::prepare_native_animation_tables(&index, &mut p).unwrap(),
        0
    );
    assert_eq!(p.fingerprint(), expected);
}

#[test]
fn actual_absence_is_preserved_and_failed_preparation_cannot_publish_a_partial_cache() {
    let temp = Fixture::new(vec![
        (150, strings("normal")),
        (158, strings("swim")),
        (159, vec![0xff]),
    ]);
    let index = Index::scan(&temp.0).unwrap();
    // A fresh load must reject the present malformed fixed table too. It may
    // not silently omit159 and later report a native missing-resource case.
    assert!(vm::load_program(&index, "Global.iff", "Global.iff").is_err());
    let raw = fs::read(temp.0.join("Global.iff")).unwrap();
    use sha2::{Digest, Sha256};
    let mut p = Program {
        globals: "Global.iff".into(),
        ..Default::default()
    };
    p.modules.insert(
        "Global.iff".into(),
        sims_vm::Module {
            source: "Global.iff".into(),
            content_hash: format!("{:x}", Sha256::digest(raw)),
            ..Default::default()
        },
    );
    let before = p.fingerprint();
    assert!(
        vm::prepare_native_animation_tables(&index, &mut p)
            .unwrap_err()
            .to_string()
            .contains("159")
    );
    assert_eq!(p.fingerprint(), before);
    p.modules.get_mut("Global.iff").unwrap().content_hash = "wrong".into();
    let before = p.fingerprint();
    assert!(
        vm::prepare_native_animation_tables(&index, &mut p)
            .unwrap_err()
            .to_string()
            .contains("resource changed")
    );
    assert_eq!(p.fingerprint(), before);
    let absent = Fixture::new(vec![(150, strings("normal"))]);
    let index = Index::scan(&absent.0).unwrap();
    let mut p = vm::load_program(&index, "Global.iff", "Global.iff").unwrap();
    assert_eq!(
        vm::prepare_native_animation_tables(&index, &mut p).unwrap(),
        0
    );
    assert_eq!(lookup(&p, 1, 0), Some("normal"));
    assert!(!p.modules[&p.globals].strings.contains_key(&158));
}

#[test]
fn a_decoded_empty_special_entry_keeps_the_authentic_native_fallback() {
    let temp = Fixture::new(vec![(150, strings("normal")), (158, strings(""))]);
    let index = Index::scan(&temp.0).unwrap();
    let mut p = vm::load_program(&index, "Global.iff", "Global.iff").unwrap();
    assert_eq!(p.modules[&p.globals].strings[&158], [""]);
    assert_eq!(lookup(&p, 1, 0), Some("normal"));
    assert_eq!(
        vm::prepare_native_animation_tables(&index, &mut p).unwrap(),
        0
    );
    assert_eq!(p.modules[&p.globals].strings[&158], [""]);
    assert_eq!(lookup(&p, 1, 0), Some("normal"));
}
