//! Bounded diagnostics for actual observed scalar/stack fields. A match here is
//! deliberately not full native boundary, renderer or Session qualification.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
pub mod animation;
pub mod pose;
pub mod physical;
pub mod routes;
pub mod sequence;

fn signed(v: &Value) -> Result<i16> {
    let n = v.as_u64().context("native unsigned short absent")?;
    Ok(u16::try_from(n)? as i16)
}
fn shorts(v: &Value) -> Result<Vec<i16>> {
    v.as_array()
        .context("native short vector absent")?
        .iter()
        .map(signed)
        .collect()
}
pub fn compare(native: &Value, simulation: &sims_sim::Simulation) -> Result<Value> {
    let rows = native["snapshot"]["objects"]
        .as_array()
        .context("native objects absent")?;
    let mut by_id = BTreeMap::new();
    for row in rows {
        let id: i16 = row["id"]
            .as_i64()
            .context("native object id absent")?
            .try_into()?;
        ensure!(
            by_id.insert(id, row).is_none(),
            "duplicate native object id"
        );
    }
    ensure!(
        by_id.keys().eq(simulation.vm.world.objects.keys()),
        "native/Rust object domains differ"
    );
    let mut compared = 0usize;
    let mut differences = 0usize;
    let mut first = Vec::new();
    let mut check = |path: String, expected: Value, actual: Value| {
        compared += 1;
        if expected != actual {
            differences += 1;
            if first.len() < 50 {
                first.push(json!({"path":path,"native":expected,"rust":actual}));
            }
        }
    };
    check(
        "/tick".into(),
        native["snapshot"]["tick"].clone(),
        json!(simulation.vm.tick),
    );
    check(
        "/rng/global".into(),
        native["snapshot"]["global_rng"].clone(),
        json!(simulation.vm.rng_state),
    );
    check(
        "/rng/lot".into(),
        native["snapshot"]["lot_rng"].clone(),
        json!(simulation.vm.lot_rng_state),
    );
    check(
        "/rng/crt".into(),
        native["snapshot"]["crt"]["state"].clone(),
        json!(simulation.vm.crt_rng_state),
    );
    let mut raw_object_words = 0;
    let mut mapped_object_words = 0;
    let mut raw_person_words = 0;
    let mut mapped_person_words = 0;
    let mut framed = BTreeSet::new();
    for (&id, row) in &by_id {
        let object = &simulation.vm.world.objects[&id];
        let data = shorts(&row["data"])?;
        raw_object_words += data.len();
        for (&field, &value) in &object.data {
            ensure!(field >= 0, "negative stored object data key");
            let expected = *data
                .get(field as usize)
                .context("stored object data exceeds observed native extent")?;
            mapped_object_words += 1;
            check(
                format!("/objects/{id}/data/{field}"),
                json!(expected),
                json!(value),
            );
        }
        check(
            format!("/objects/{id}/attributes"),
            json!(shorts(&row["attributes"])?),
            json!(object.attributes),
        );
        check(
            format!("/objects/{id}/temps"),
            json!(shorts(&row["temps"])?),
            json!(object.temps),
        );
        if let Some(person) = &object.person {
            let data = shorts(&row["person"]["data"])?;
            raw_person_words += data.len();
            for (&field, &value) in person {
                ensure!(field >= 0, "negative stored person data key");
                let expected = *data
                    .get(field as usize)
                    .context("stored person data exceeds observed native extent")?;
                mapped_person_words += 1;
                check(
                    format!("/objects/{id}/person/{field}"),
                    json!(expected),
                    json!(value),
                );
            }
            check(
                format!("/objects/{id}/motives"),
                row["person"]["motives_bits"].clone(),
                json!(object.motives),
            );
            check(
                format!("/objects/{id}/previous_motives"),
                row["person"]["previous_motives_bits"].clone(),
                json!(object.motive_update.as_ref().map(|s| s.previous)),
            );
        } else {
            ensure!(row.get("person").is_none(), "native person absent in Rust");
        }
        let thread_id = *simulation
            .vm
            .object_threads
            .get(&id)
            .context("object thread absent")?;
        let thread = simulation
            .vm
            .threads
            .get(&thread_id)
            .context("mapped thread absent")?;
        let frames = row["frames"]
            .as_array()
            .context("native active frames absent")?;
        check(
            format!("/objects/{id}/frames/length"),
            json!(frames.len()),
            json!(thread.stack.len()),
        );
        for (i, (a, b)) in frames.iter().zip(&thread.stack).enumerate() {
            framed.insert(id);
            for (name, expected, actual) in [
                ("tree", a["tree"].clone(), json!(b.location.bhav)),
                ("node", a["node"].clone(), json!(b.location.instruction)),
                (
                    "continuation",
                    a["continuation"].clone(),
                    json!(b.continuation),
                ),
                (
                    "stack_object",
                    a["stack_object"].clone(),
                    json!(b.stack_object),
                ),
                ("args", json!(shorts(&a["args"])?), json!(b.args)),
                ("locals", json!(shorts(&a["locals"])?), json!(b.locals)),
            ] {
                check(format!("/objects/{id}/frames/{i}/{name}"), expected, actual);
            }
        }
    }
    Ok(
        json!({"schema":1,"kind":"observed_scalar_stack_diagnostic","native_parity_credit":0,
        "compared":compared,"differences":differences,"first_differences":first,
        "object_words":{"observed":raw_object_words,"mapped":mapped_object_words},
        "person_words":{"observed":raw_person_words,"mapped":mapped_person_words},"objects_with_frames":framed.len(),
        "qualification":"Selected observed fields only. Source versions, full thread/wait state, physical links, routes, animation, geometry and other host ownership require their own comparisons."}),
    )
}
