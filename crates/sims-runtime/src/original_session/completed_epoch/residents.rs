//! Exhaustive retained Resident owner classification at an actual native caller.
//! Native value fields still require their named independent source comparers.
use super::super::completed_geometry::{bytes, u};
use super::{Context, Result, Saved, Value, ensure, json};
use std::collections::BTreeSet;
#[cfg(test)]
#[path = "residents/tests.rs"]
mod tests;

fn boundary(saved: &Saved, proof: &Value) -> Result<()> {
    ensure!(
        proof["schema"] == 1
            && proof["status"] == "actual_native_completed_main_thread_callers_qualified"
            && proof["source_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e"
            && proof["parent_closure_sha256"]
                == "bcbced9059a201f08ab04b68ea432d1adc2cbab3a0ef098646e8056f85944e50"
            && proof["target_executed"] == false
            && proof["changed_return_rejected"] == true,
        "native completed caller qualification differs"
    );
    let ids: Vec<i16> = serde_json::from_value(proof["native_object_ids"].clone())?;
    ensure!(
        proof["native_bytecode_sha256"]
            == "7ce70152aabfb406f076c31860b0c33e99bb332578f615bdf7e4a25f8a572231"
            && proof["animation_yields"]
                == json!([{"object":16,"thread":16,"request_epoch":1,"tick":88448},{"object":18,"thread":18,"request_epoch":2,"tick":88448},{"object":21,"thread":21,"request_epoch":3,"tick":88448}]),
        "native waiting animation request epoch differs"
    );
    ensure!(
        ids.len() == 344
            && ids.iter().copied().collect::<BTreeSet<_>>()
                == saved
                    .simulation
                    .state
                    .vm
                    .world
                    .objects
                    .keys()
                    .copied()
                    .collect(),
        "native caller object ownership differs"
    );
    let anchors = [
        ("callee_saved_stopwatch", "ac66e209"),
        ("callee_return", "75da4600"),
        ("callee_saved_outer_ESI", "e865e209"),
        ("outer_clock_saved_app_EBP", "04fe8100"),
        ("outer_clock_return", "b81a4d00"),
        ("app_frame_EH_state", "ffffffff"),
        ("app_frame_return", "63754f00"),
        ("frame_dispatch_return", "7c5e5d00"),
    ];
    let cuts = proof["cuts"]
        .as_array()
        .context("native caller cuts absent")?;
    ensure!(cuts.len() == 2, "native caller cut cohort differs");
    for (cut, (name, tick)) in cuts.iter().zip([("before", 88448), ("after", 88449)]) {
        ensure!(
            cut["cut"] == name
                && cut["tick"] == tick
                && cut["registers"] == json!({"eip":0x46d51d,"esp":0x81fc4c,"ebp":0x81fc80}),
            "native main-thread caller point differs"
        );
        let rows = cut["anchors"]
            .as_array()
            .context("native stack anchors absent")?;
        ensure!(
            rows.len() == anchors.len(),
            "native stack anchor cohort differs"
        );
        for ((name, raw), row) in anchors.iter().zip(rows) {
            ensure!(
                row["name"] == *name && row["raw"] == *raw,
                "native source return/stack owner differs"
            );
        }
    }
    let rows = proof["code"]
        .as_array()
        .context("native boundary code absent")?;
    ensure!(rows.len() == 4, "native boundary code cohort differs");
    for (rows, cut) in rows
        .chunks_exact(2)
        .zip(["completed_tick_frame", "registry_update"])
    {
        for (row, side) in rows.iter().zip(["before", "after"]) {
            let (address, count, hash) = if cut == "completed_tick_frame" {
                (
                    0x46d345,
                    480,
                    "86b48a6aa3fdc9eb2f0306405e64f0ddf2bcd129b537159bb76b1aa082671502",
                )
            } else {
                (
                    0x46d553,
                    324,
                    "370ddd86ca475047c5e6faaed023c4b28a3ab8a6d45415bff839a572f3f53ba9",
                )
            };
            ensure!(
                row["name"] == cut
                    && row["cut"] == side
                    && row["address"] == address
                    && row["bytes"] == count
                    && row["sha256"] == hash,
                "native caller code witness differs"
            );
        }
    }
    Ok(())
}
pub fn evaluate(saved: &Saved, source: &Value, proof: &Value) -> Result<Value> {
    boundary(saved, proof)?;
    let sim = &saved.simulation.state;
    ensure!(
        source["tick"] == sim.vm.tick && [88448, 88449].contains(&sim.vm.tick),
        "Resident source cut differs"
    );
    let actors = source["resident"]["actors"]
        .as_array()
        .context("native resident actors absent")?;
    ensure!(
        actors.len() == 3 && sim.characters.keys().copied().eq([16, 18, 21]),
        "Resident native/source cohort differs"
    );
    let keys: BTreeSet<_> = [
        "character",
        "controls",
        "evaluated_tick",
        "context",
        "source",
        "animation_yield",
        "profile_dirty",
        "skill_profile",
        "ordinary_update",
        "person_update",
        "displayed_animation",
        "native_walk",
        "native_turn",
        "physical_movement",
        "refresh_tick",
    ]
    .into_iter()
    .collect();
    let mut rows = vec![];
    let mut inventory = vec![];
    for (&id, resident) in &sim.characters {
        let a = actors
            .iter()
            .find(|a| a["object"] == id)
            .context("native Resident absent")?;
        let ar = bytes(a, "animator_raw")?;
        ensure!(
            ar.len() == 0x2a0
                && ar[0x150] == 0
                && ar[0xd8] == 0
                && u(&ar, 0x188)? == 0
                && u(&ar, 0x190)? == 0,
            "active native motion/profile requires its own caller classification"
        );
        let value = serde_json::to_value(resident)?;
        ensure!(
            value
                .as_object()
                .context("Resident is not an object")?
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == keys,
            "Resident field inventory changed"
        );
        let mut fields = vec![];
        for (field, comparison) in [
            (
                "character",
                "characters/all native animator,bones,hierarchy,head,rig,model,placement,censor,clock,visibility; completed-call children below",
            ),
            (
                "controls",
                "characters/native motion,anchors,generation,events; retained path and initializer source extensions",
            ),
            ("evaluated_tick", "characters/native last-clock boundary"),
            ("context", "characters/complete original property context"),
            ("source", "immutable-source/full live skeleton metadata"),
            ("profile_dirty", "characters/native animator+d8"),
            (
                "displayed_animation",
                "person-labels/original person+18c strings",
            ),
        ] {
            fields.push(json!({"field":field,"kind":"native_value_comparison_obligation","comparison":comparison,"retained_sha256":sims_fixtures::digest(&postcard::to_allocvec(&value[field])?)}));
        }
        let expected = if sim.vm.tick == 88448 {
            None
        } else {
            Some((
                id as u64,
                match id {
                    16 => 1,
                    18 => 2,
                    21 => 3,
                    _ => unreachable!(),
                },
                88448u64,
            ))
        };
        let matched = resident.animation_yield == expected;
        fields.push(json!({"field":"animation_yield","kind":"source_bound_waiting_animation_request_epoch","matched":matched,"expected":expected,"actual":resident.animation_yield}));
        rows.push(json!({"object":id,"field":"animation_yield","kind":"source_bound_waiting_animation_request_epoch","matched":matched,"expected":expected,"actual":resident.animation_yield}));
        for field in [
            "skill_profile",
            "ordinary_update",
            "person_update",
            "native_walk",
            "native_turn",
            "physical_movement",
            "refresh_tick",
        ] {
            let matched = value[field].is_null();
            fields.push(json!({"field":field,"kind":"unpublished_Rust_synchronous_call","matched":matched,"native_persistent_work_absence_claimed":false}));
            rows.push(json!({"object":id,"field":field,"matched":matched,"kind":"qualified_replacement_call_epoch"}));
        }
        for (field, v) in [
            ("character.pass", &value["character"]["pass"]),
            (
                "character.skin.handler",
                &value["character"]["skin"]["handler"],
            ),
            (
                "character.skin.animator.pass",
                &value["character"]["skin"]["animator"]["pass"],
            ),
            (
                "controls.movement.pass",
                &value["controls"]["movement"]["pass"],
            ),
        ] {
            rows.push(json!({"object":id,"field":field,"kind":"qualified_replacement_child_call_epoch","matched":v.is_null(),"native_persistent_work_absence_claimed":false}));
        }
        ensure!(
            fields.len() == 15,
            "Resident field classification incomplete"
        );
        inventory.push(json!({"object":id,"fields":fields}));
    }
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":sim.vm.tick,"inventory":inventory,"comparisons":rows,"difference_count":differences,"native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All15 fields per Resident classified; transient Rust latches/children require the actual original completed C-stack point at both cuts. Native seven value owners remain explicit fixed-comparer obligations. Script frames, queues/events and lifetime/deletion are not declared absent."}),
    )
}
