use super::*;
use sims_fixtures::Store;
use std::{fs, path::Path};
#[test]
#[ignore = "private actual Goth source caches, current managed cuts and immutable installation"]
fn actual_source_caches_reject_changed_noise_names_stream_floor_and_settings() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(run)?
        .join()
        .map_err(|_| anyhow::anyhow!("source cache contract worker panicked"))?
}
fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = |p: &str| -> Result<Vec<u8>> { Ok(fs::read(root.join(p))?) };
    let read = |p: &str| -> Result<Value> { Ok(serde_json::from_slice(&bytes(p)?)?) };
    let proto =
        bytes("fixtures/private/goth-resource-closure-20261028-first/prototypes-input.json")?;
    let glob = bytes("fixtures/private/goth-asset-host-20261028-first/global-second.json")?;
    let cloth = bytes("fixtures/private/goth-asset-host-20261028-first/clothing-first.json")?;
    let cache_proof =
        read("fixtures/private/goth-source-cache-contract-20261029-first/cache-proof-second.json")?;
    let clothing_proof = read(
        "fixtures/private/goth-session-contract-20261029-first/clothing-unchanged-fifth.json",
    )?;
    let physical = read(
        "fixtures/private/goth-supplemental-owners-20261029-first/physical-input-first/before.json",
    )?;
    let index =
        Index::scan(&root.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    let store = Store::open(
        root.join("fixtures/private/goth-editor-settings-20261029-first/session-first/store"),
    )?;
    let refs =
        read("fixtures/private/goth-editor-settings-20261029-first/session-first/refs.json")?;
    let saved: Saved = store.get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let original = postcard::to_allocvec(&saved.simulation.state.vm)?;
    let check = |state: &Saved, proof: &Value| {
        evaluate(
            state,
            &index,
            Inputs {
                prototypes: &proto,
                global: &glob,
                clothing: &cloth,
                cache_proof: proof,
                clothing_proof: &clothing_proof,
                physical: &physical,
            },
        )
    };
    ensure!(
        check(&saved, &cache_proof)?["difference_count"] == 0,
        "native source cache comparison failed"
    );
    for kind in 0..5 {
        let mut wrong = saved.clone();
        match kind {
            0 => {
                let mut v = serde_json::to_value(&wrong.assets.skeletons)?;
                let catalog = v["catalogs"]
                    .as_object_mut()
                    .context("catalog absent")?
                    .values_mut()
                    .next()
                    .context("empty catalogs")?;
                let record = &mut catalog["resources"][0]["wiggle"][0]["records"][0][0];
                *record = (record.as_i64().context("retained noise absent")? ^ 1).into();
                wrong.assets.skeletons = serde_json::from_value(v)?;
            }
            1 => {
                let mut v = serde_json::to_value(&wrong.assets.clothing)?;
                let first = &mut v["names"]["bodies"][0][0];
                *first = (first.as_u64().context("clothing name absent")? ^ 1).into();
                wrong.assets.clothing = serde_json::from_value(v)?;
            }
            2 => {
                let mut v = serde_json::to_value(&wrong.assets.autonomy_resources)?;
                let cursor = &mut v["components"]["entries"]["1"]["stream"]["position"];
                *cursor = (cursor.as_u64().context("Global stream cursor absent")? ^ 1).into();
                wrong.assets.autonomy_resources = serde_json::from_value(v)?;
            }
            3 => {
                let mut v = serde_json::to_value(&wrong.assets.floors)?;
                let floor = v
                    .as_object_mut()
                    .context("floor bindings absent")?
                    .values_mut()
                    .next()
                    .context("empty floors")?;
                floor["category"] = (floor["category"]
                    .as_u64()
                    .context("floor category absent")?
                    ^ 1)
                .into();
                wrong.assets.floors = serde_json::from_value(v)?;
            }
            _ => wrong.assets.initializer_settings.fade_duration += 1,
        }
        ensure!(
            check(&wrong, &cache_proof)?["difference_count"] == 1,
            "source cache mutation{kind} was accepted or failed elsewhere"
        );
    }
    let mut bad = cache_proof.clone();
    bad["fields"][0]["before_after_equal"] = false.into();
    ensure!(
        check(&saved, &bad).is_err(),
        "changed original cache proof accepted"
    );
    ensure!(
        postcard::to_allocvec(&saved.simulation.state.vm)? == original,
        "read-only source cache audit changed VM/RNG"
    );
    Ok(())
}
