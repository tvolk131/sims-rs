//! Retained Object fields, independently imported at each actual source cut.
//! Supplemental routing/slot geometry/score owners have separate gates. This
//! diagnostic must never turn those omissions into full Session admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{Saved, completed_object_storage};
use sims_vm::{Object, ObjectId};
use std::{collections::BTreeMap, fs, path::Path};

fn projection(object: &Object) -> Object {
    let mut value = object.clone();
    // Explicit separate native-owner comparisons, never absent native data.
    value.native_motive_scores = None;
    value.native_slot_geometry = None;
    value.routing_slots = None;
    // Replacement adapter bookkeeping has no historical native timestamp.
    value.last_update = None;
    if let Some(idle) = &mut value.idle {
        idle.last_run = None;
    }
    if let Some(headline) = &mut value.headline {
        headline.last_update = None;
    }
    value
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 6,
        "usage: completed_tick_object_storage STORE REFS_JSON FIELD NATIVE_OBJECT_JSON JOIN_REPORT NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    ensure!(
        source["schema"] == 1
            && source["tick"] == saved.simulation.state.vm.tick
            && source["executes_target_code"] == false
            && source["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && source["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "object storage source cut/epoch differs"
    );
    let inputs: Vec<completed_object_storage::Input> =
        serde_json::from_value(source["objects"].clone())?;
    let native = completed_object_storage::import(&inputs, "source-object-comparison-unpublished")?;
    let report: Value = serde_json::from_slice(&fs::read(&args[4])?)?;
    let aliases = report["provider_aliases"]
        .as_object()
        .context("provider aliases absent")?;
    let actual = &saved.simulation.state.vm.world.objects;
    ensure!(
        native.keys().eq(actual.keys()),
        "object-storage cohort differs"
    );
    for object in native.values().chain(actual.values()) {
        let definition = object.definition.as_ref().context("definition absent")?;
        // Both imported Object stores deliberately retain the qualified native
        // provider key. The Program resolves it through this descriptor map;
        // rewriting only the reference to a FAR path would create a false diff.
        aliases
            .get(&definition.source)
            .and_then(Value::as_str)
            .context("provider descriptor absent")?;
    }
    let mut differences = Vec::new();
    let mut expected_projection = BTreeMap::<ObjectId, Object>::new();
    let mut actual_projection = BTreeMap::<ObjectId, Object>::new();
    for (&id, expected) in &native {
        let expected = projection(expected);
        let actual = projection(&actual[&id]);
        if expected != actual {
            let a = serde_json::to_value(&expected)?;
            let b = serde_json::to_value(&actual)?;
            let keys: std::collections::BTreeSet<_> = a
                .as_object()
                .unwrap()
                .keys()
                .chain(b.as_object().unwrap().keys())
                .collect();
            for key in keys {
                if a.get(key) != b.get(key) {
                    differences.push(
                        json!({"object":id,"field":key,"native":a.get(key),"rust":b.get(key)}),
                    );
                }
            }
        }
        expected_projection.insert(id, expected);
        actual_projection.insert(id, actual);
    }
    let a = postcard::to_allocvec(&expected_projection)?;
    let b = postcard::to_allocvec(&actual_projection)?;
    let report = json!({"schema":1,"objects":native.len(),"difference_count":differences.len(),
        "differences":differences,"native_sha256":sims_fixtures::digest(&a),
        "rust_sha256":sims_fixtures::digest(&b),"native_parity_ticks":0,
        "complete_native_Session_admitted":false,
        "separate_native_owner_gates":["native_motive_scores","native_slot_geometry","routing_slots"],
        "replacement_adapter_bookkeeping":["last_update","idle.last_run","headline.last_update"],
        "scope":"Object storage import fields at the actual cut; separate native owners and whole Session admission remain required"});
    let out = Path::new(&args[5]);
    fs::create_dir(out)?;
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"objects":native.len(),"differences":report["difference_count"]})
    );
    ensure!(
        a == b && report["difference_count"] == 0,
        "retained Object storage differs"
    );
    Ok(())
}
