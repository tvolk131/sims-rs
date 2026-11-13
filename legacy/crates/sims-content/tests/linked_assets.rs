use sims_content::{Index, assets};
use sims_vm::{Module, Program};
use std::{collections::BTreeMap, fs};
struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn fixture() -> Temp {
    // SystemTime may have microsecond resolution: parallel tests can start in
    // the same clock sample. A per-process counter keeps their trees isolated.
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "sims-linked-assets-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(p.join("a")).unwrap();
    fs::create_dir_all(p.join("b")).unwrap();
    Temp(p)
}
fn catalog(name: &str) -> Vec<u8> {
    let mut b = vec![];
    for n in [0u32, 0, 1] {
        b.extend(n.to_le_bytes());
    }
    for s in [name, "samples"] {
        b.push(s.len() as u8);
        b.extend(s.as_bytes());
    }
    for n in [33f32.to_bits(), 0, 0, 1, 0, 1] {
        b.extend(n.to_le_bytes());
    }
    b.push(4);
    b.extend(b"ROOT");
    for n in [1u32, 33f32.to_bits(), 1, 0, 0, 0, 0, 0] {
        b.extend(n.to_le_bytes());
    }
    b
}
fn program() -> Program {
    Program {
        globals: "g".into(),
        modules: BTreeMap::from([(
            "g".into(),
            Module {
                source: "g".into(),
                strings: BTreeMap::from([(
                    156,
                    vec!["Actual-Clip".into(), "absent-unused".into()],
                )]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    }
}
#[test]
fn embedded_names_dependency_locality_and_conflicting_providers() {
    let t = fixture();
    fs::write(
        t.0.join("a/unrelated-filename.cmx.bcf"),
        catalog("Actual-Clip"),
    )
    .unwrap();
    fs::write(t.0.join("a/samples.cfp"), [126, 126, 126]).unwrap();
    fs::write(t.0.join("b/samples.cfp"), [127, 126, 126]).unwrap();
    let index = Index::scan(&t.0).unwrap();
    assert!(index.find("samples.cfp").is_err());
    let mut p = program();
    let report = assets::load_linked_animations(&index, &mut p).unwrap();
    assert_eq!(report.loaded_clips, 1);
    assert_eq!(report.unresolved, ["absent-unused"]);
    let clip = &p.animations["actual-clip"];
    assert_eq!(clip.samples.translations, [[0.; 3]]);
    assert_eq!(clip.samples_source, "a/samples.cfp");
    assert_eq!(clip.content_hash.len(), 64);
    fs::write(t.0.join("b/different-name.cmx.bcf"), catalog("Actual-Clip")).unwrap();
    let e = assets::load_linked_animations(&Index::scan(&t.0).unwrap(), &mut program())
        .unwrap_err()
        .to_string();
    assert!(e.contains("conflicting animation"), "{e}");
    // Identical duplicates are accepted without inventing expansion override precedence.
    fs::write(t.0.join("b/samples.cfp"), [126, 126, 126]).unwrap();
    assert_eq!(
        assets::load_linked_animations(&Index::scan(&t.0).unwrap(), &mut program())
            .unwrap()
            .loaded_clips,
        1
    );
}
#[test]
fn corrupt_catalogs_are_reported_and_required_sample_failures_propagate() {
    let t = fixture();
    fs::write(t.0.join("a/broken.cmx.bcf"), [0; 9]).unwrap();
    let r = assets::load_linked_animations(&Index::scan(&t.0).unwrap(), &mut program()).unwrap();
    assert_eq!(r.catalog_errors.len(), 1);
    assert_eq!(r.unresolved.len(), 2);
    fs::write(t.0.join("a/valid.cmx.bcf"), catalog("Actual-Clip")).unwrap();
    assert!(assets::load_linked_animations(&Index::scan(&t.0).unwrap(), &mut program()).is_err());
    fs::write(t.0.join("a/samples.cfp"), [255]).unwrap();
    assert!(assets::load_linked_animations(&Index::scan(&t.0).unwrap(), &mut program()).is_err());
}
