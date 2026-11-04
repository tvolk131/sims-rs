use super::*;
use std::{fs, path::Path};
fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read(name: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        root()
            .join("fixtures/private/goth-asset-host-20261028-first")
            .join(name),
    )?)?)
}

#[test]
#[ignore = "actual private stopped Global resident cache"]
fn retained_global_routines_keep_lazy_records_absent_and_reject_conflicts_atomically() -> Result<()>
{
    let source = read("global-second.json")?;
    let index =
        Index::scan(&root().join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let native = format!(
        "native-provider:{:08x}",
        source["provider"]["pointer"].as_u64().unwrap()
    );
    let locator = "GameData_ALL_Recursive/Global/Global.far::Global.iff";
    let mut program = Program {
        globals: native.clone(),
        modules: BTreeMap::from([(
            native.clone(),
            sims_vm::Module {
                source: native.clone(),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let rows = source["graph"]["maps"][0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["type_raw"] == u32::from_be_bytes(*b"BHAV"))
        .unwrap()["inner"]["nodes"]
        .as_array()
        .unwrap();
    let count = rows
        .iter()
        .filter(|r| r["handle"] != 0 && r["key_variant"] == 0)
        .count();
    assert_eq!(
        import_resident_global_routines(&source, &index, locator, &mut program)?,
        count
    );
    assert_eq!(
        program.modules[&native].routines[&424]
            .body
            .instructions
            .len(),
        2
    );
    for row in rows.iter().filter(|r| r["handle"] == 0) {
        assert!(
            !program.modules[&native]
                .routines
                .contains_key(&(row["key_id"].as_u64().unwrap() as u16))
        );
    }
    let fingerprint = program.fingerprint();
    assert_eq!(
        import_resident_global_routines(&source, &index, locator, &mut program)?,
        0
    );
    assert_eq!(program.fingerprint(), fingerprint);
    *program
        .modules
        .get_mut(&native)
        .unwrap()
        .routine_versions
        .get_mut(&424)
        .unwrap() ^= 1;
    let before = program.fingerprint();
    assert!(import_resident_global_routines(&source, &index, locator, &mut program).is_err());
    assert_eq!(program.fingerprint(), before);
    Ok(())
}
#[test]
#[ignore = "actual private stopped asset-owner source"]
fn captured_clothing_is_retained_and_missing_or_contradictory_names_fail() -> Result<()> {
    let input = read("clothing-first.json")?;
    let cache = clothing::import(&input)?;
    let value = serde_json::to_value(&cache)?;
    ensure!(
        value["retained"] == true
            && value["names"]["bodies"].as_array().unwrap().len() == 2186
            && value["names"]["textures"]["names"]
                .as_array()
                .unwrap()
                .len()
                == 4431,
        "current clothing contents lost"
    );
    let mut missing = input.clone();
    missing["reads"]
        .as_array_mut()
        .unwrap()
        .retain(|r| r["label"] != "clothing.bodies.used");
    ensure!(
        clothing::import(&missing).is_err(),
        "missing names enumerated instead"
    );
    let mut changed = input;
    changed["caches"]["bodies"]["values"][0] = "00".into();
    ensure!(
        clothing::import(&changed).is_err(),
        "contradictory names accepted"
    );
    Ok(())
}
#[test]
#[ignore = "actual private joined asset owners"]
fn source_pool_identity_preserves_bounds_and_rejects_conflicting_native_aliases() -> Result<()> {
    // Named deserialization of the full captured checkpoint exceeds the test
    // harness's small worker stack. Keep that fixture load in a bounded worker;
    // this changes no production loader or simulation behavior.
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(source_pool_identity)?
        .join()
        .map_err(|_| anyhow::anyhow!("captured asset fixture worker panicked"))?
}
fn source_pool_identity() -> Result<()> {
    let refs: Value = serde_json::from_slice(&fs::read(
        root().join("fixtures/private/goth-asset-host-20261028-first/executed-third/refs.json"),
    )?)?;
    let store = sims_fixtures::Store::open(
        root().join("fixtures/private/goth-asset-host-20261028-first/executed-third/store"),
    )?;
    let joined: Joined = store.get_value(&serde_json::from_value(refs["assets"].clone())?)?;
    let native: Vec<u32> = serde_json::from_value(joined.source["native_meshes"].clone())?;
    let (same, maps, owners) = pool(&[(&joined.mesh_pool, &native), (&joined.mesh_pool, &native)])?;
    ensure!(
        same == serde_json::to_value(&joined.mesh_pool)? && maps[0] == maps[1] && owners == native,
        "shared native cache was recalculated/deduplicated by contents"
    );
    let mut conflicting = joined.mesh_pool.clone();
    conflicting.meshes[0].stamps[0] ^= 1;
    ensure!(
        pool(&[(&joined.mesh_pool, &native), (&conflicting, &native)]).is_err(),
        "conflicting generation alias accepted"
    );
    let mut distinct = native.clone();
    for p in &mut distinct {
        *p += 0x10000000;
    }
    let value = serde_json::to_value(&joined.mesh_pool)?;
    let other: MeshPool = exact(json!({"meshes":value["meshes"],"prototypes":{}}))?;
    let (both, _, owners) = pool(&[(&joined.mesh_pool, &native), (&other, &distinct)])?;
    ensure!(
        both["meshes"].as_array().unwrap().len() == 28 && owners.len() == 28,
        "distinct native owners merged by matching contents"
    );
    Ok(())
}
#[test]
#[ignore = "actual private stopped Global owner and immutable asset corpus"]
fn retained_global_restores_current_cursor_and_rejects_incomplete_maps() -> Result<()> {
    let v = read("global-second.json")?;
    let index = Index::scan(&root().join("game-data/cabinet-extracted"))?;
    let locator = "GameData_ALL_Recursive/Global/Global.far::Global.iff";
    let owner = global::import(&v, &index, locator)?;
    let value = serde_json::to_value(&owner)?;
    ensure!(
        value["components"]["entries"]["1"]["stream"]["position"] == 16238
            && value["components"]["handles"].as_object().unwrap().len() == 47
            && value["events"].as_array().unwrap().is_empty(),
        "current Global owner rebuilt"
    );
    let mut missing = v.clone();
    missing["graph"]["resident_handles"]
        .as_array_mut()
        .unwrap()
        .pop();
    ensure!(
        global::import(&missing, &index, locator).is_err(),
        "missing resident allocation accepted"
    );
    let mut alias = v.clone();
    alias["graph"]["maps"][0]["nodes"][0]["inner"]["pointer"] = 0.into();
    ensure!(
        global::import(&alias, &index, locator).is_err(),
        "unbound inner hash owner accepted"
    );
    let mut topology = v;
    topology["graph"]["maps"][0]["declared_entry_count_raw"] = 0.into();
    ensure!(
        global::import(&topology, &index, locator).is_err(),
        "unknown hash map rebuilt"
    );
    Ok(())
}
