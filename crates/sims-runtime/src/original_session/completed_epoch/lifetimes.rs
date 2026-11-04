//! Native deferred-removal ownership plus source-qualified synchronous callers.
use super::{Context, Result, Saved, Value, ensure, json, same};
use std::collections::BTreeSet;

pub fn evaluate(saved: &Saved, proof: &Value) -> Result<Value> {
    ensure!(
        proof["schema"] == 1
            && proof["status"] == "native_completed_registry_and_synchronous_retirement_qualified"
            && proof["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && proof["decoded_pc_payload_sha256"]
                == "abd3d875cda21f17fa0139261a4d9a39a7243a65a172b322bcf58450cbe0fbd0"
            && proof["negative_same_id_pointer_replacement"]["rejected"] == true,
        "native lifetime proof absent/different"
    );
    let functions = proof["inactive_functions"]
        .as_array()
        .context("native destructor coverage absent")?;
    ensure!(
        functions.len() == 4
            && functions
                .iter()
                .all(|r| r["instruction_visits"] == 0 && r["live_function_executed"] == false),
        "native deletion/destructor path was reached"
    );
    let cuts = proof["cuts"]
        .as_array()
        .context("native lifetime cuts absent")?;
    ensure!(cuts.len() == 2, "native lifetime two-cut ownership differs");
    let s = &saved.simulation.state;
    let native = cuts
        .iter()
        .find(|r| r["tick"] == s.vm.tick)
        .context("native lifetime current cut absent")?;
    ensure!(
        native["deferred_removals"] == 0
            && native["main_thread_at_completed_return"] == true
            && native["synchronous_deletion_and_retirement_in_flight"] == false,
        "native retirement is not quiescent"
    );
    let r = &native["registry"];
    let handles: Vec<Option<u32>> = serde_json::from_value(r["handles"].clone())?;
    let words: Vec<u32> = serde_json::from_value(r["words"].clone())?;
    let mappings = r["native_live_pointers"]
        .as_array()
        .context("native registry live pointers absent")?;
    ensure!(
        handles.len() == 386
            && words.len() == 670
            && mappings.len() == 344
            && r["last_handle"] == 386,
        "native registry complete used extents differ"
    );
    let ids: BTreeSet<u32> = mappings
        .iter()
        .map(|r| {
            r["object"]
                .as_u64()
                .map(|n| n as u32)
                .context("native lifetime object absent")
        })
        .collect::<Result<_>>()?;
    let pointers: BTreeSet<u64> = mappings
        .iter()
        .map(|r| {
            r["pointer"]
                .as_u64()
                .context("native lifetime pointer absent")
        })
        .collect::<Result<_>>()?;
    ensure!(
        ids.len() == 344
            && pointers.len() == 344
            && mappings
                .iter()
                .all(|r| r["replacement_lifetime"] == r["object"]),
        "native current lifetime aliases differ"
    );
    ensure!(
        handles
            .iter()
            .enumerate()
            .all(|(i, v)| *v == ids.contains(&((i + 1) as u32)).then_some((i + 1) as u32)),
        "fresh replacement lifetime namespace differs"
    );
    let expected =
        sims_vm::scheduler::storage::Storage::from_retained(&s.vm.world, handles, words, 386)
            .map_err(anyhow::Error::msg)?;
    let actual =
        s.vm.world
            .scheduler_storage
            .as_ref()
            .context("current lifetime registry absent")?;
    let mut rows = vec![];
    same(
        "registry/full_native_used_slots_words_and_live_aliases",
        &expected,
        actual,
        &mut rows,
    )?;
    let actual_ids: BTreeSet<_> =
        s.vm.world
            .objects
            .keys()
            .map(|&id| u32::try_from(id).map_err(anyhow::Error::from))
            .collect::<Result<_>>()?;
    same(
        "registry/all_native_current_Object_IDs",
        &ids,
        &actual_ids,
        &mut rows,
    )?;
    let quiescent = [
        ("simulation/removals", s.removals.is_empty()),
        ("simulation/deletion", s.deletion.is_empty()),
        (
            "simulation/retired_characters",
            s.retired_characters.is_empty(),
        ),
        ("simulation/retired_graphics", s.retired_graphics.is_empty()),
        (
            "assets/person_retirement_services",
            saved.assets.retirements.is_empty(),
        ),
        (
            "assets/ordinary_retirement_services",
            saved.assets.ordinary.retirements.is_empty(),
        ),
    ];
    for (name, empty) in quiescent {
        same(name, &true, &empty, &mut rows)?;
    }
    let people: BTreeSet<_> =
        s.vm.world
            .person
            .as_ref()
            .context("native lifetime people absent")?
            .person_order
            .iter()
            .copied()
            .collect();
    let resident_people: BTreeSet<_> = s.characters.keys().copied().collect();
    same(
        "characters/only_current_native_persons",
        &people,
        &resident_people,
        &mut rows,
    )?;
    let asset_people: BTreeSet<_> = saved.assets.characters.keys().copied().collect();
    same(
        "characters/live_host_aliases",
        &people,
        &asset_people,
        &mut rows,
    )?;
    let graphics: BTreeSet<_> = saved.assets.ordinary.graphics.keys().copied().collect();
    same(
        "ordinary/current_simulation_host_aliases",
        &s.retained_ordinary_graphics,
        &graphics,
        &mut rows,
    )?;
    for (&id, g) in &saved.assets.ordinary.graphics {
        let graphic = serde_json::to_value(g)?;
        let owner: i16 = serde_json::from_value(graphic["owner"]["graphic"].clone())?;
        ensure!(
            actual.handle(id) == Some(u32::try_from(id)?),
            "ordinary graphic lost its qualified current registry lifetime"
        );
        same(
            &format!("ordinary/{id}/current_registry_lifetime"),
            &id,
            &owner,
            &mut rows,
        )?;
    }
    ensure!(
        people.len() == 3 && graphics.len() == 2,
        "native live character cohort differs"
    );
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":s.vm.tick,"comparisons":rows,"comparison_count":rows.len(),"difference_count":differences,
        "native_live_objects":344,"native_pointer_slots":386,"native_used_words":670,"complete_native_Session_admitted":false,"native_parity_ticks":0,
        "source_proof":proof["source_report_sha256"],"scope":"Full actual current Registry values and pointer alias graph; original real removal queue empty; synchronous deletion/destructor wrapper absence requires the qualified original completed C-stack and inactive native paths. Current character/ordinary lifetime aliases remain live. Full geometry/prototype aliases and every Thread/value owner remain mandatory separate comparers."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    #[test]
    #[ignore = "private captured completed Goth source owners"]
    fn original_lifetime_owners_reject_pending_retirement_wrong_words_and_callers() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let checkpoint =
            root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
        let store = sims_fixtures::Store::open(checkpoint.join("store"))?;
        let proof: Value = serde_json::from_slice(&fs::read(
            root.join("fixtures/private/goth-native-lifetimes-20261029-first/proof-second.json"),
        )?)?;
        for (index, name) in [(0, "refs.json"), (1, "managed-endpoint.json")] {
            let refs: Value = serde_json::from_slice(&fs::read(checkpoint.join(name))?)?;
            let saved: Saved =
                store.get_value(&serde_json::from_value(refs["session"].clone())?)?;
            let original = postcard::to_allocvec(&saved.simulation.state.vm)?;
            ensure!(
                evaluate(&saved, &proof)?["difference_count"] == 0,
                "original native lifetime positive failed"
            );
            let mut bad = saved.clone();
            bad.simulation
                .state
                .removals
                .push(sims_vm::removal::Pending {
                    target: 16,
                    suppress_reset: false,
                });
            ensure!(
                evaluate(&bad, &proof)?["difference_count"] == 1,
                "extra pending native removal admitted"
            );
            let mut bad = saved.clone();
            bad.simulation.state.retired_graphics.insert(
                383,
                sims_sim::removal::graphic::Retired {
                    definition: saved.simulation.state.vm.world.objects[&383]
                        .definition
                        .clone()
                        .context("ordinary source definition absent")?,
                    registry_lifetime: Some(383),
                    tick: saved.simulation.state.vm.tick,
                    asset_service_started: false,
                },
            );
            ensure!(
                evaluate(&bad, &proof)?["difference_count"] == 1,
                "unqualified retired graphic admitted"
            );
            let mut bad = proof.clone();
            let word = bad["cuts"][index]["registry"]["words"][669]
                .as_u64()
                .unwrap();
            bad["cuts"][index]["registry"]["words"][669] = (word ^ 1).into();
            ensure!(
                evaluate(&saved, &bad)?["difference_count"] == 1,
                "wrong original retained word tail admitted"
            );
            let mut bad = proof.clone();
            bad["cuts"][index]["main_thread_at_completed_return"] = false.into();
            ensure!(
                evaluate(&saved, &bad).is_err(),
                "active native synchronous caller admitted"
            );
            let mut bad = proof.clone();
            bad["inactive_functions"][0]["instruction_visits"] = 1.into();
            ensure!(
                evaluate(&saved, &bad).is_err(),
                "reached native deletion call admitted"
            );
            ensure!(
                postcard::to_allocvec(&saved.simulation.state.vm)? == original,
                "native lifetime audit changed VM/RNG"
            );
        }
        Ok(())
    }
}
