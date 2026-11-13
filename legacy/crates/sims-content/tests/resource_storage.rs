//! Private original neighborhood TATT checks, using unchanged native decoder output.
#[path = "support/catalog.rs"]
mod historical_catalog;
use sims_vm::resource_storage::State;
use std::{fs, path::PathBuf};

#[test]
#[ignore = "requires original resources and private native_resource_storage.py capture"]
fn original_catalog_and_neighborhood_storage_match_native() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = sims_content::Index::scan(&root.join("game-data/cabinet-extracted")).unwrap();
    let catalog = historical_catalog::resource_storage(&index).unwrap();
    assert_eq!(catalog.len(), 11);
    assert!(catalog.iter().all(|d| !d.reset_per_lot));
    let native: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("cache/research/sims-exe/oracle/resource-storage.json")).unwrap(),
    )
    .unwrap();
    let mut results = vec![];
    for row in native["cases"].as_array().unwrap() {
        let folder = row["source"].as_str().unwrap();
        let file = if folder == "UserData" {
            "House05.iff"
        } else {
            "House09.iff"
        };
        let house =
            sims_content::saves::House::load(&index, &format!("{folder}/Houses/{file}")).unwrap();
        let saved = house.resource_storage.as_ref().unwrap();
        let mut state = State::construct(&catalog);
        // Native fixture's sentinel detects short-row tail zeroing.
        state
            .records
            .iter_mut()
            .find(|r| r.guid == 0x8963d7c1)
            .unwrap()
            .values
            .fill(99);
        let consumed = state.load(saved);
        let help = state.records.iter().find(|r| r.guid == 0x8963d7c1).unwrap();
        assert_eq!(
            serde_json::to_value(help).unwrap(),
            row["original"]["records"][0]
        );
        assert_eq!(
            consumed as u64,
            row["original"]["consumed"].as_u64().unwrap()
        );
        assert!(
            state
                .records
                .iter()
                .filter(|r| r.guid != 0x8963d7c1)
                .all(|r| r.values.iter().all(|v| *v == 0))
        );
        results.push(serde_json::json!({"source":folder,"saved":saved,"loaded":state}));
    }
    // Repeated compatibility checks must preserve earlier evidence.
    let output = root.join(
        std::env::var("SIMS_RESOURCE_STORAGE_EVIDENCE")
            .unwrap_or_else(|_| format!("cache/resource-storage-test-{}.json", std::process::id())),
    );
    use std::io::Write;
    fs::File::create_new(&output)
        .unwrap()
        .write_all(
            &serde_json::to_vec_pretty(
                &serde_json::json!({"catalog":catalog,"neighborhoods":results}),
            )
            .unwrap(),
        )
        .unwrap();
    println!("Evidence: {}", output.display());
}
