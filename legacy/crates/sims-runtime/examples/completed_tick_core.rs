//! Read-only diagnostic projection. This must not be used as a whole-state gate.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::Saved;
use std::{collections::BTreeMap, fs, path::Path};
fn words(values: impl IntoIterator<Item = i16>) -> Vec<u16> {
    values.into_iter().map(|n| n as u16).collect()
}
fn cells(values: &BTreeMap<i16, i16>, count: usize) -> Result<Vec<u16>> {
    (0..count)
        .map(|i| {
            values
                .get(&(i as i16))
                .copied()
                .with_context(|| format!("missing native storage cell{i}"))
                .map(|v| v as u16)
        })
        .collect()
}
fn project(saved: &Saved, owners: bool) -> Result<Value> {
    let simulation = &saved.simulation.state;
    let vm = &simulation.vm;
    let world = &vm.world;
    let registry = world
        .scheduler_storage
        .as_ref()
        .context("registry absent")?;
    let mut objects = BTreeMap::new();
    for (&id, obj) in &world.objects {
        let thread = &vm.threads[&vm.object_threads[&id]];
        let frames:Vec<_>=thread.stack.iter().map(|f|json!({"source":f.location.source,"code_owner":f.code_owner,"tree":f.location.bhav,"node":f.location.instruction,"stack_object":f.stack_object as u16,"continuation":f.continuation,"args":words(f.args.iter().copied()),"locals":words(f.locals.iter().copied()),"offset":f.storage_offset})).collect();
        objects.insert(id.to_string(),json!({"registry_word":registry.word(world,id).map_err(anyhow::Error::msg)?,"data":cells(&obj.data,72)?,"temps":words(obj.temps),"attributes":words(obj.attributes.iter().copied()),"last_result":thread.last_result,"frames":frames,"person":obj.person.as_ref().map(|p|cells(p,256)).transpose()?,"motives":obj.motives,"previous_motives":obj.motive_update.as_ref().map(|m|m.previous)}));
    }
    let characters:BTreeMap<_,_>=simulation.characters.iter().map(|(&id,r)|(id.to_string(),json!({"clock":r.character.clock as u32,"engaged":r.controls.engaged as u32,"animation_events":r.controls.animation_events as u32}))).collect();
    let mut value = json!({"tick":vm.tick,"rng":[vm.rng_state,vm.lot_rng_state.context("lot RNG absent")?,vm.crt_rng_state.context("CRT RNG absent")?],"globals":cells(&world.globals,64)?,"objects":objects,"characters":characters});
    if owners {
        // Complete fixed inventory from completed_world's exhaustive join;
        // the source cannot choose a smaller inventory to obtain agreement.
        // Serialize each value explicitly: World's compatibility serializer
        // intentionally omits some empty/default fields in old saves.
        value["systems"] = json!({
            "autonomy_constants": world.autonomy_constants,
            "map_size": world.map_size,
            "registry_ui_disabled": world.registry_ui_disabled,
            "magic_history": world.magic_history,
            "registry_order": world.registry_order,
            "indicator_queues": world.indicator_queues,
            "camera_rotation": world.camera_rotation,
            "special_effects": world.special_effects,
            "resource_storage": world.resource_storage,
            "changed_resources": world.changed_resources,
            "image_cache": world.image_cache,
            "ui_owner": world.ui_owner,
            "dialogs": world.dialogs,
            "fires": world.fires,
            "positions": world.positions,
            "room_outdoors": world.room_outdoors,
            "room_half_tiles": world.room_half_tiles,
            "budget": world.budget,
            "globals": world.globals,
            "tree_advertisements": world.tree_advertisements,
            "neighborhood": world.neighborhood,
            "refresh": world.refresh,
            "actions": world.actions
        });
        let autonomy = world
            .autonomy_constants
            .as_ref()
            .context("autonomy owner absent")?;
        value["systems"]["autonomy_constants"]["input_evidence"] =
            serde_json::to_value(&autonomy.input_evidence)?;
        value["systems"]["autonomy_constants"]["observation"] =
            serde_json::to_value(&autonomy.observation)?;
        value["systems"]["indicator_queues"]["dirty"] = json!(world.indicator_queues.dirty);
        value["systems"]["image_cache"]["entries"] = json!(world.image_cache.entries.iter().map(|e| json!({"guid":e.guid,"neighbor":e.neighbor,"resource":e.resource,"images":e.images,"dirty":e.dirty})).collect::<Vec<_>>());
        value["person_world"] =
            serde_json::to_value(world.person.as_ref().context("person World absent")?)?;
    }
    Ok(value)
}
fn differences(a: &Value, b: &Value, path: &str, count: &mut usize, first: &mut Vec<Value>) {
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for k in keys {
                if !a.contains_key(k) || !b.contains_key(k) {
                    *count += 1;
                    if first.len() < 100 {
                        first.push(json!({"path":format!("{path}/{k}"), "missing_from":if !a.contains_key(k) {"native"} else {"rust"}}));
                    }
                    continue;
                }
                differences(&a[k], &b[k], &format!("{path}/{k}"), count, first)
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(a, b, &format!("{path}/{i}"), count, first)
            }
        }
        _ => {
            *count += 1;
            if first.len() < 100 {
                first.push(json!({"path":path,"native":a,"rust":b}));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_null_is_not_equal_and_details_do_not_limit_count() {
        let mut count = 0;
        let mut first = vec![];
        differences(
            &json!({"owner":null}),
            &json!({}),
            "",
            &mut count,
            &mut first,
        );
        assert_eq!(count, 1);
        differences(
            &json!(vec![0; 200]),
            &json!(vec![1; 200]),
            "",
            &mut count,
            &mut first,
        );
        assert_eq!(count, 201);
        assert_eq!(first.len(), 100);
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5 || (args.len() == 6 && args[5] == "--owners"),
        "usage: completed_tick_core STORE REFS_JSON REF_FIELD NATIVE_CORE_JSON NEW_DIR [--owners]"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let rust = project(&saved, args.len() == 6)?;
    let native: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let mut count = 0;
    let mut first = vec![];
    differences(&native, &rust, "", &mut count, &mut first);
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    fs::write(out.join("rust-core.json"), serde_json::to_vec(&rust)?)?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"scope":"integer/frame/character-clock diagnostic; not complete state or native parity admission","native_parity_ticks":0,"difference_count":count,"first_differences":first}),
        )?,
    )?;
    println!("{}", json!({"differences":count,"native_parity_ticks":0}));
    ensure!(count == 0, "retained core components differ");
    Ok(())
}
