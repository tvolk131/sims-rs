use super::*;

#[test]
fn completed_delivery_rejects_pending_cross_owner_and_extra_fields() -> Result<()> {
    ensure!(completed_delivery(383, 88448, &Value::Null)?.is_none());
    let done = json!({"object":383,"pass":"Done","child":null,"pending":null});
    ensure!(completed_delivery(383, 88449, &done)?.unwrap().complete());
    ensure!(completed_delivery(383, 88448, &done).is_err());
    ensure!(completed_delivery(384, 88449, &done).is_err());
    let mut pending = done.clone();
    pending["pending"] = json!({"Position":{"object":383,"position":[0,0]}});
    ensure!(completed_delivery(383, 88449, &pending).is_err());
    let mut extra = done;
    extra["unknown_owner"] = true.into();
    ensure!(completed_delivery(383, 88449, &extra).is_err());
    Ok(())
}

#[test]
#[ignore = "requires private Goth native sources, complete profile proof and immutable installation"]
fn actual_mesh_and_ordinary_owners_reject_alias_record_cache_and_pending_mutations() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("mesh owner test worker panicked"))?
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let read = |p: &str| -> Result<Vec<u8>> { Ok(fs::read(root.join(p))?) };
    let value = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&read(p)?)?) };
    let cp = root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
    let refs: Value = serde_json::from_slice(&fs::read(cp.join("refs.json"))?)?;
    let saved: Saved = Store::open(cp.join("store"))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let source = value(
        "fixtures/private/goth-native-animation-rebase-20261028-first/native-before-complete-geometry-first.json",
    )?;
    let proto =
        value("fixtures/private/goth-resource-closure-20261028-first/prototypes-input.json")?;
    let profile = read("fixtures/private/goth-live-ordinary-20261028-first/profiles-first.json")?;
    let proof = value(
        "fixtures/private/goth-mesh-owners-20261029-first/ordinary-profile-proof-second.json",
    )?;
    let objects = value(
        "fixtures/private/goth-routing-owners-20261029-first/object-storage-before-first.json",
    )?;
    let providers = value(
        "fixtures/private/goth-stream-owners-20261028-first/resident-BHAV-comparison-fourth.json",
    )?;
    let index =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let checks = |s: &Saved| -> Result<usize> {
        let mut rows = super::super::meshes::evaluate(&source, s, &proto, &index)?;
        rows.extend(evaluate(
            &source, s, &profile, &proof, &objects, &providers, &index,
        )?);
        Ok(rows.iter().filter(|r| r["matched"] == false).count())
    };
    ensure!(
        checks(&saved)? == 0,
        "actual native mesh/ordinary baseline differs"
    );
    let vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
    for kind in 0..5 {
        let mut changed = saved.clone();
        if kind == 0 {
            let mut p = serde_json::to_value(&changed.assets.mesh_pool)?;
            let n = p["meshes"].as_array().unwrap().len();
            let entry = p["prototypes"]
                .as_object_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap();
            *entry = (((entry.as_u64().unwrap() as usize) + 1) % n).into();
            changed.assets.mesh_pool = serde_json::from_value(p)?;
            ensure!(checks(&changed)? == 1, "changed native mesh alias missed");
        } else {
            let mut p = serde_json::to_value(&changed.assets.ordinary)?;
            let original: sims_content::graphic_loading::Saved = serde_json::from_value(p.clone())?;
            ensure!(
                postcard::to_allocvec(&original)?
                    == postcard::to_allocvec(&changed.assets.ordinary)?,
                "ordinary test JSON codec loses native bits"
            );
            match kind {
                1 => {
                    let v = &mut p["graphics"]["383"]["binding"]["order"][0];
                    *v = (v.as_u64().unwrap() + 1).into();
                }
                2 => {
                    let v = &mut p["profiles"][0]["state"]["id"];
                    *v = (v.as_u64().unwrap() + 1).into();
                }
                3 => {
                    let v = &mut p["graphics"]["383"]["bound"]["skeleton"]["skin"]["animator"]["selection_cache"];
                    *v = (v.as_i64().unwrap() + 1).into();
                }
                4 => {
                    p["graphics"]["383"]["update"] =
                        json!({"object":383,"pass":"Done","child":null,"pending":null})
                }
                _ => unreachable!(),
            }
            changed.assets.ordinary = serde_json::from_value(p)?;
            if kind == 4 {
                ensure!(
                    checks(&changed).is_err(),
                    "invented baseline completed call was accepted"
                );
            } else {
                ensure!(
                    checks(&changed)? == if kind == 2 { 2 } else { 3 },
                    "ordinary changed owner missed: {kind}"
                );
            }
        }
        ensure!(
            postcard::to_allocvec(&changed.simulation.state.vm)? == vm,
            "read-only asset checker changed VM/RNG"
        );
    }
    Ok(())
}
