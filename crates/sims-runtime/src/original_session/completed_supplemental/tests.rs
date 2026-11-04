use super::*;
use crate::original_session::Saved;
use sims_fixtures::Store;
use std::{fs, path::Path};

#[test]
#[ignore = "private actual original supplemental readsets and managed Goth checkpoint"]
fn actual_owners_and_corruptions_preserve_the_vm() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(actual)?
        .join()
        .map_err(|_| anyhow::anyhow!("supplemental fixture worker panicked"))?
}
fn actual() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let input: Value = serde_json::from_slice(&fs::read(
        root.join("fixtures/private/goth-supplemental-owners-20261029-first/before-first.json"),
    )?)?;
    let checkpoint = root.join("fixtures/private/goth-routing-owners-20261029-first/session-first");
    let refs: Value = serde_json::from_slice(&fs::read(checkpoint.join("refs.json"))?)?;
    let saved: Saved = Store::open(checkpoint.join("store"))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let vm = &saved.simulation.state.vm;
    let original = postcard::to_allocvec(vm)?;
    let owners = import(&input, &vm.world)?;
    ensure!(
        owners.scores.len() == 3
            && owners.geometry.len() == 344
            && owners.geometry.values().map(Vec::len).sum::<usize>() == 160
            && owners.head_names.len() == 3
            && owners.opaque_slot_words == 157,
        "actual supplemental cohort lost"
    );
    ensure!(
        owners.body_profiles == vm.world.body_profiles,
        "actual BodyStrings values differ"
    );
    for change in 0..5 {
        let mut bad = input.clone();
        match change {
            0 => {
                bad["people"].as_array_mut().unwrap().pop();
            }
            1 => bad["people"][0]["data"]["curves"][0]["points_raw"] = "00".into(),
            2 => bad["body_profiles"]["definitions"]
                .as_object_mut()
                .unwrap()
                .clear(),
            3 => {
                let row = bad["objects"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|o| o["object"] == 21)
                    .unwrap();
                row["head_name"] = "ROOT".into();
            }
            _ => bad["provenance"]["replay_report_sha256"] = "wrong".into(),
        }
        ensure!(
            import(&bad, &vm.world).is_err(),
            "supplemental corruption{change} admitted"
        );
    }
    ensure!(
        postcard::to_allocvec(vm)? == original,
        "supplemental parser changed VM/RNG"
    );
    Ok(())
}
