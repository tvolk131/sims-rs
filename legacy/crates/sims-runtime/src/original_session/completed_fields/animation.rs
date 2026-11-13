//! Current layer/clock comparison. Only unpublished Rust handle identities are
//! normalized; native floating values compare as IEEE-754 single bits.
use super::*;
use crate::original_session::completed_animations;
use sims_fixtures::readset::Readset;

pub(super) fn float_bits(value: &mut Value) {
    match value {
        Value::Number(n) if n.is_f64() => {
            *value = json!({"f32_bits":(n.as_f64().unwrap()as f32).to_bits()})
        }
        Value::Array(a) => a.iter_mut().for_each(float_bits),
        Value::Object(o) => o.values_mut().for_each(float_bits),
        _ => (),
    }
}

fn canonical(animator: &sims_sim::pose::animator::Animator, control: u16) -> Result<Value> {
    let mut value = serde_json::to_value(animator)?;
    let mut ids = BTreeMap::new();
    for (ordinal, id) in animator.destruction_layers().iter().enumerate() {
        if let Some(id) = id {
            ensure!(
                ids.insert(id.0, ordinal as u64 + 1).is_none(),
                "shared animator slot needs an explicit identity profile"
            );
        }
    }
    let mut next = 5;
    for layer in animator.layers() {
        if let std::collections::btree_map::Entry::Vacant(entry) = ids.entry(layer.id.0) {
            entry.insert(next);
            next += 1;
        }
    }
    let remap = |value: &mut Value| -> Result<()> {
        if value.is_null() {
            return Ok(());
        }
        let id = value.as_u64().context("animator layer handle absent")?;
        *value = json!(
            *ids.get(&id)
                .context("animator slot references unobserved layer")?
        );
        Ok(())
    };
    for layer in value["layers"]
        .as_array_mut()
        .context("animator layers absent")?
    {
        remap(&mut layer["id"])?;
    }
    for slot in value["slots"]
        .as_array_mut()
        .context("animator slots absent")?
    {
        remap(slot)?;
    }
    remap(&mut value["blend"])?;
    // next_id is an unpublished adapter allocation counter, not an observed
    // native scalar. Full cold-state equality still checks it separately.
    value.as_object_mut().unwrap().remove("next_id");
    // Native arithmetic control is process-owned, not an Animator byte field.
    value["fp_control"] = json!(control);
    float_bits(&mut value);
    Ok(value)
}

pub fn compare(
    state: &Value,
    memory: &Readset<'_>,
    simulation: &sims_sim::Simulation,
    epoch: &str,
) -> Result<Value> {
    ensure!(
        state["tick"] == simulation.vm.tick && state["readset"]["sha256"] == memory.source_sha256(),
        "animator comparison epoch differs"
    );
    let controls: Vec<_> = state["identity"]["hardware_registers"]
        .as_str()
        .context("animator CPU state absent")?
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("FCW="))
        .collect();
    ensure!(
        controls.len() == 1,
        "animator CPU control missing/ambiguous"
    );
    let control = u16::from_str_radix(controls[0], 16)?;
    let source = &state["domains"]["animations"];
    crate::original_session::completed_skeletons::captured::reads(&source["payload"], memory)?;
    let imported = completed_animations::import_with_resident_lists(
        &serde_json::from_value::<Vec<completed_animations::AssetInput>>(
            source["payload"]["assets"].clone(),
        )?,
        &serde_json::from_value::<Vec<completed_animations::CharacterInput>>(
            source["characters"].clone(),
        )?,
        epoch,
        &source["payload"]["resident_lists"],
        &state["characters"]["event_cursors"],
        memory,
    )?;
    ensure!(
        imported.characters.keys().eq(simulation.characters.keys()),
        "animator actor domain differs"
    );
    let mut differences = Vec::new();
    let mut rows = Vec::new();
    for (&id, native) in &imported.characters {
        let rust = &simulation.characters[&id];
        ensure!(
            rust.character.skin.animator.fp_control() == control,
            "Rust animator arithmetic control differs"
        );
        let a = canonical(&native.animator, control)?;
        let b = canonical(&rust.character.skin.animator, control)?;
        for (key, expected) in a.as_object().unwrap() {
            if expected != &b[key] {
                differences.push(json!({"object":id,"field":key,"native":expected,"rust":b[key]}));
            }
        }
        for (key, a, b) in [
            ("clock", native.clock, rust.character.clock),
            ("engaged", native.engaged, rust.controls.engaged),
            (
                "animation_events",
                native.animation_events,
                rust.controls.animation_events,
            ),
        ] {
            if a != b {
                differences.push(json!({"object":id,"field":key,"native":a,"rust":b}));
            }
        }
        rows.push(json!({"object":id,"native":a,"rust":b}));
    }
    Ok(
        json!({"schema":1,"kind":"observed_animator_diagnostic","native_parity_credit":0,
        "normalization":"Role-bound unpublished LayerIds; next_id excluded; captured process FCW bound separately; f32 values compared by bits.",
        "differences":differences,"actors":rows,
        "qualification":"Animator/layer/clock fields only. Skeleton hierarchy, skin sampling, geometry, routes and full Session are separate."}),
    )
}
