use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{Index, saved_avatar::BoundAvatar, saves::House};
use sims_sim::pose::{
    appearance::{Execution, Input},
    appearance_selection::{Request, Selection, State},
};
use sims_vm::{Object, Vm, World};

/// Fresh profile selection and asset binding, before geometry refresh or saved
/// animator installation. This intentionally does not claim a live household.
#[test]
#[ignore = "requires private original character assets"]
fn original_profiles_select_decode_and_replace_real_appearance_records() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        for resident in &house
            .household
            .as_ref()
            .context("household absent")?
            .residents
        {
            let id = resident.object as i16;
            let definition = storage.world.objects[&id]
                .definition
                .clone()
                .context("definition absent")?;
            let module = &program.modules[&definition.source];
            let words = &module.object_definition_words[&definition.object];
            let profile = &module.strings[&(words[19] as u16)];
            let flags = words[0xcc / 2] as u16;
            let object =
                Object::construct_person(definition, &program, 0).map_err(anyhow::Error::msg)?;
            let mut world = World::default();
            world.objects.insert(id, object);
            let mut vm = Vm::new(world, 1);
            sims_vm::person_reset::reload_bound_profile(&mut vm, &program, id)
                .map_err(anyhow::Error::msg)?;
            let person = vm
                .world
                .objects
                .get_mut(&id)
                .unwrap()
                .person
                .as_mut()
                .unwrap();
            let mut state = State::constructed([23, 47, 91]);
            let input =
                json!({"profile":profile,"person":person,"cache":state.cached,"flags":flags});
            let mut selection = Selection::begin(&mut state, person, profile, flags, false)
                .map_err(anyhow::Error::msg)?;
            let mut bound = BoundAvatar::empty(&index, &profile[0], id as u64)?;
            let mut events: Vec<Value> = selection
                .lookups
                .iter()
                .map(|n| json!({"profile":n}))
                .collect();
            while let Some(request) = selection.next(&state, &bound.records) {
                let mut restored: Selection =
                    serde_json::from_slice(&serde_json::to_vec(&selection)?)?;
                ensure!(restored.next(&state, &bound.records) == Some(request.clone()));
                selection = restored;
                match request {
                    Request::Bind {record,descriptor} => {
                        events.push(json!({"bind":{"record":record,"name":descriptor.name,"texture":descriptor.texture}}));
                        let dirty = bound.apply_record(&index, record, Execution::bind(Input::Replace {
                            name:descriptor.name,texture:descriptor.texture,
                        }))?;
                        ensure!(dirty.is_empty(), "fresh profile replaced geometry");
                    }
                    Request::TextureNotification { first } => events.push(json!({"texture_notification":if first {"4eb0cc"} else {"4eb0eb"},"guid":0x12345678u32})),
                    Request::Tint(tint) => events.push(json!({"tint":tint})),
                }
                selection.acknowledge().map_err(anyhow::Error::msg)?;
            }
            ensure!(selection.is_complete());
            ensure!(bound.part_ranges.len() == 4 && bound.avatar.parts.len() >= 4);
            let original = json!({"person":person.values().copied().collect::<Vec<_>>(),
                "cache":state.cached,"dirty":state.dirty,"special_dirty":state.special_dirty,"skin_dirty":state.skin_dirty,
                "records":bound.records.entries.iter().map(|r| json!({"bound":r.binding.is_some(),"name":r.name,"texture":r.texture})).collect::<Vec<_>>()});
            let records = bound.records.clone();
            let before_parts = bound.avatar.parts.len();
            // A successful replacement must allocate a new identity before
            // removing the old one, with the other record ranges still intact.
            let head = bound.records.head;
            let old = bound.records.entries[head].binding.unwrap();
            let dirty = bound.apply_record(&index, head, Execution::bind(Input::Current))?;
            ensure!(dirty == vec![id as u64]);
            ensure!(bound.records.entries[head].binding.unwrap().id != old.id);
            ensure!(!bound.part_ranges.contains_key(&old.id));
            ensure!(bound.avatar.parts.len() == before_parts && bound.part_ranges.len() == 4);
            let mut covered = vec![false; before_parts];
            for range in bound.part_ranges.values() {
                for part in range.clone() {
                    ensure!(!covered[part]);
                    covered[part] = true;
                }
            }
            ensure!(covered.iter().all(|b| *b));
            let dirty = bound.apply_record(&index, head, Execution::clear())?;
            ensure!(dirty == vec![id as u64] && bound.records.entries[head].binding.is_none());
            ensure!(bound.records.entries[head].name == records.entries[head].name);
            bound.apply_record(&index, head, Execution::bind(Input::Current))?;
            ensure!(bound.avatar.parts.len() == before_parts);
            let state = bound.snapshot();
            let restored = BoundAvatar::restore(&index, &profile[0], id as u64, &state)?;
            ensure!(restored.snapshot() == state && restored.part_ranges == bound.part_ranges);
            ensure!(restored.avatar.sources == bound.avatar.sources);
            ensure!(
                restored
                    .avatar
                    .parts
                    .iter()
                    .zip(&bound.avatar.parts)
                    .all(|(a, b)| a.mesh.name == b.mesh.name && a.texture.rgba == b.texture.rgba)
            );

            report.push(json!({"source":source,"object":id,"name":resident.name,"input":input,
                "events":events,"original":original,"records":records,"parts":before_parts,"sources":bound.avatar.sources}));
        }
    }
    ensure!(report.len() == 7);
    std::fs::write(
        root.join("cache/original-appearance-selection.json"),
        serde_json::to_vec(&report)?,
    )?;
    Ok(())
}

#[test]
#[ignore = "requires private native original-profile comparison"]
fn original_profile_selections_match_the_native_engine() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rust: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/original-appearance-selection.json"),
    )?)?;
    let native: Vec<Value> = serde_json::from_slice(&std::fs::read(
        root.join("cache/native-original-appearance-selection.json"),
    )?)?;
    ensure!(rust.len() == 7 && native.len() == 7);
    for (rust, native) in rust.iter().zip(&native) {
        ensure!(rust["input"] == native["input"]);
        ensure!(
            rust["events"] == native["events"],
            "{} appearance service order differs",
            rust["name"]
        );
        ensure!(
            rust["original"] == native["original"],
            "{} appearance state differs",
            rust["name"]
        );
    }
    Ok(())
}
