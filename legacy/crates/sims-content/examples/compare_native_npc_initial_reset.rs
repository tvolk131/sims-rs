//! Compare the composed original NPC constructor reset, with real Init/Main.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_vm::{
    person_factory::{
        ConstructionReset, ConstructionResetRequest as R, InitialResetRequest as Request,
        PostConstructRequest, PostConstruction,
    },
    *,
};
use std::collections::BTreeMap;

fn fields(v: &Value) -> BTreeMap<i16, i16> {
    v.as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, v)| (i as i16, v.as_i64().unwrap() as i16))
        .collect()
}

/// The retained oracle predates dialog/catalog metadata and extra string banks.
/// Check every original exported value (including whole-IFF hashes), allowing
/// only these audited additions. The live Program is never changed, nor are the
/// native fixture bytes, their digest, or any expected execution results.
fn validate_resource_export(linked: &Value, original: &Value) -> Result<()> {
    let mut projected = linked.clone();
    for (key, expected) in [
        ("neighbor_resource_flags", Value::Null),
        ("resource_storage", Value::Null),
        ("text_language", json!(1)),
    ] {
        ensure!(original.get(key).is_none(), "oracle export schema changed");
        ensure!(
            projected.as_object_mut().unwrap().remove(key) == Some(expected),
            "new factory context differs: {key}"
        );
    }
    let modules = projected["modules"]
        .as_object_mut()
        .context("linked modules absent")?;
    let ui = modules
        .remove("GameData_ALL_Recursive/UIText.iff")
        .context("additional UI text module absent")?;
    ensure!(
        ui["source"] == "GameData_ALL_Recursive/UIText.iff"
            && ui["semiglobal"].is_null()
            && ui["headline_catalog"].is_null()
    );
    for (key, value) in ui.as_object().context("UI module absent")? {
        if !matches!(
            key.as_str(),
            "source" | "content_hash" | "strings" | "semiglobal" | "headline_catalog"
        ) {
            ensure!(
                value.as_object().is_some_and(|v| v.is_empty())
                    || value.as_array().is_some_and(|v| v.is_empty()),
                "additional UI module has non-text content: {key}"
            );
        }
    }
    for (source, module) in modules {
        let old = original["modules"]
            .get(source)
            .context("unexpected factory module")?;
        let map = module.as_object_mut().context("factory module absent")?;
        for key in ["catalog_names", "dialog_marked"] {
            ensure!(
                old.get(key).is_none() && map.remove(key).is_some(),
                "factory metadata export changed: {key}"
            );
        }
        let strings = map
            .get_mut("strings")
            .and_then(Value::as_object_mut)
            .context("strings absent")?;
        let extras: Vec<_> = strings
            .keys()
            .filter(|key| old["strings"].get(*key).is_none())
            .cloned()
            .collect();
        for key in extras {
            let allowed = if source.ends_with("::NPC_TravelingSalesman.iff") {
                ["131", "132", "301", "305"].contains(&key.as_str())
            } else {
                key == "305"
            };
            ensure!(
                allowed,
                "unaudited added factory string bank: {source}:{key}"
            );
            strings.remove(&key);
        }
    }
    ensure!(
        projected == *original,
        "oracle resources no longer match linked program"
    );
    Ok(())
}

fn state(vm: &Vm, thread: ThreadId) -> Value {
    let o = &vm.world.objects[&1];
    json!({"data":o.data.values().copied().collect::<Vec<_>>(),
        "person":o.person.as_ref().unwrap().values().copied().collect::<Vec<_>>(),
        "motives":o.motives,"previous":o.motive_update.as_ref().unwrap().previous,
        "temps":o.temps,"attributes":o.attributes,"countdown":o.idle.as_ref().unwrap().countdown,
        "global_rng":vm.rng_state,"crt":vm.crt_rng_state.unwrap(),
        "last_decay":o.motive_update.as_ref().unwrap().last_decay,
        "frames":vm.threads[&thread].stack.iter().map(|f| json!({"bhav":f.location.bhav,
            "pc":f.location.instruction,"stack_object":f.stack_object,"args":f.args,"locals":f.locals,
            "offset":f.storage_offset})).collect::<Vec<_>>()})
}

fn run(
    vm: &mut Vm,
    p: &Program,
    thread: ThreadId,
    trace: &mut Vec<Value>,
    callback: bool,
    restore: bool,
) -> Result<()> {
    for _ in 0..20_000 {
        if callback && vm.threads[&thread].callback.is_none() {
            return Ok(());
        }
        let r = vm.run_slice(p, thread, 1).map_err(anyhow::Error::msg)?;
        for t in r.trace {
            let source = if t.location.bhav < 4096 {
                "Global.iff"
            } else if t.location.bhav >= 8192 {
                "PersonGlobals.iff"
            } else {
                r"People\TemplatePerson.iff"
            };
            trace.push(json!([
                t.tick,
                source,
                t.location.bhav,
                t.location.instruction,
                t.depth
            ]));
        }
        match &vm.threads[&thread].status {
            Status::Ready => {}
            Status::Sleeping { .. } if !callback || vm.threads[&thread].callback.is_none() => {
                return Ok(());
            }
            status => anyhow::bail!("unexpected NPC reset status {status:?}"),
        }
        if restore {
            *vm = Snapshot::capture(p, vm)
                .restore(p)
                .map_err(anyhow::Error::msg)?;
        }
    }
    anyhow::bail!("NPC reset instruction budget exhausted")
}

fn main() -> Result<()> {
    let with_factory = std::env::args().any(|a| a == "--factory");
    let with_post = std::env::args().any(|a| a == "--post");
    let bytes = std::fs::read("cache/research/sims-exe/oracle/person-factory-resources.json")?;
    let resources: Value = serde_json::from_slice(&bytes)?;
    let report: Value = serde_json::from_slice(&std::fs::read(if with_factory {
        "cache/research/sims-exe/oracle/registered-person-factory.json"
    } else if with_post {
        "cache/research/sims-exe/oracle/person-postconstruct.json"
    } else {
        "cache/research/sims-exe/oracle/npc-initial-reset.json"
    })?)?;
    ensure!(
        report["capture_sha256"]
            == "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    ensure!(
        report["resources_sha256"] == format!("{:x}", Sha256::digest(&bytes)),
        "native probe used different exported resources"
    );
    let index = sims_content::Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let p = sims_content::vm::load_program(&index, "NPC_TravelingSalesman.iff", "Global.iff")?;
    // The export goes through Value (f32 -> f64) before JSON encoding. Use
    // the same round trip here so decimal parsing cannot create false deltas.
    let linked: Value = serde_json::from_slice(&serde_json::to_vec(&serde_json::to_value(&p)?)?)?;
    validate_resource_export(&linked, &resources["program"])?;
    let tuning = motives::Tuning::from_program(&p).map_err(anyhow::Error::msg)?;
    let globals: BTreeMap<i16, i16> = serde_json::from_value(report["context"]["globals"].clone())?;
    let mut total = 0;
    for row in report["cases"].as_array().context("native rows")? {
        for restored in [false, true] {
            let c = &row["input"];
            let n = &row["original"];
            let before = &n["before"];
            let mut object = Object::construct_person(
                Definition {
                    source: resources["private"].as_str().unwrap().into(),
                    object: 128,
                    guid: 0x44a86a6c,
                    slots: p.modules[resources["private"].as_str().unwrap()].object_definition_words
                        [&128][20] as u16,
                },
                &p,
                if with_post { 0 } else { 5 },
            )
            .map_err(anyhow::Error::msg)?;
            object.data = fields(&before["data"]);
            object.person = Some(fields(&before["person"]));
            let mut people = person::WorldState {
                neighbor_slots: Some(vec![Some(1), None, None]),
                object_order: vec![1],
                person_order: vec![1],
                lot_zoning: serde_json::from_value(report["context"]["lot_zoning"].clone())?,
                ..Default::default()
            };
            let revision = c["revision"].as_i64().unwrap() as i32;
            let mut neighbor: BTreeMap<_, _> = (0..256)
                .map(|i| (i, if revision > 0 { i * 2 } else { 0 }))
                .collect();
            neighbor.insert(61, 0);
            people.neighbors.insert(
                1,
                person::Neighbor {
                    guid: if c["existing"] == true {
                        0x44a86a6c
                    } else {
                        123
                    },
                    person_revision: revision,
                    person: neighbor,
                    ..Default::default()
                },
            );
            let mut world = World {
                globals: globals.clone(),
                person: Some(people),
                actions: Some(action::WorldState::default()),
                refresh: Some(refresh::RefreshState::default()),
                ..Default::default()
            };
            world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .insert(1, Default::default());
            world.positions.insert(
                1,
                autonomy::Placement {
                    x: -16,
                    y: -16,
                    level: 1,
                },
            );
            world
                .refresh
                .as_mut()
                .unwrap()
                .draw
                .insert(1, refresh::DrawState::constructed(1));
            world.objects.insert(1, object);
            let mut vm = Vm::new(world, c["seed"].as_u64().unwrap());
            vm.crt_rng_state = Some(c["crt"].as_u64().unwrap() as u32);
            if with_factory {
                total += registered(&mut vm, &p, &tuning, row, restored)?;
                continue;
            }
            if with_post {
                let mut post = PostConstruction::begin(&vm, &p, 1).map_err(anyhow::Error::msg)?;
                while let Some(request) = post.next(&mut vm).map_err(anyhow::Error::msg)? {
                    if restored {
                        post = serde_json::from_slice(&serde_json::to_vec(&post)?)?;
                        vm = Snapshot::capture(&p, &vm)
                            .restore(&p)
                            .map_err(anyhow::Error::msg)?;
                        ensure!(post.next(&mut vm).map_err(anyhow::Error::msg)? == Some(request));
                    }
                    match request {
                        PostConstructRequest::AutonomyConstants => anyhow::bail!(
                            "NPC oracle requires the source-bound process resource provider; no supplied completion"
                        ),
                        PostConstructRequest::BindAnimator => {
                            person_reset::reload_bound_profile(&mut vm, &p, 1)
                                .map_err(anyhow::Error::msg)?
                        }
                        // Same supplied boundaries as the native probe: base
                        // storage, initial scores and cached profile resource.
                        PostConstructRequest::Base
                        | PostConstructRequest::InitialScores
                        | PostConstructRequest::RefreshResourceProfile => {}
                    }
                    post.complete(request).map_err(anyhow::Error::msg)?;
                }
                let expected = &n["post"];
                ensure!(
                    json!(
                        vm.world.objects[&1]
                            .person
                            .as_ref()
                            .unwrap()
                            .values()
                            .copied()
                            .collect::<Vec<_>>()
                    ) == expected["person"]
                );
                ensure!(
                    json!(vm.crt_rng_state) == expected["crt"]
                        && json!(vm.rng_state) == expected["global_rng"]
                );
                ensure!(json!(vm.world.objects[&1].slots.len()) == expected["slot_count"]);
                ensure!(vm.world.objects[&1].headline == Some(Default::default()));
                ensure!(vm.world.objects[&1].routing_slots.is_none());
                for (slot, height) in expected["slots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .skip(1)
                    .zip(person::BUILTIN_SLOT_HEIGHTS)
                {
                    ensure!(slot[5] == height);
                }
                ensure!(expected["headline"][2] == 2f32.to_bits());
                ensure!(expected["headline"][4] == 0 && expected["headline"][13] == 0);
            }
            let mut reset = ConstructionReset::begin(
                &mut vm,
                1,
                construction::ObjectDate {
                    year: 1997,
                    month: 1,
                    day: 1,
                },
            )
            .map_err(anyhow::Error::msg)?;
            let mut trace = vec![];
            while let Some(request) = reset.next(&mut vm, &p).map_err(anyhow::Error::msg)? {
                if restored {
                    reset = serde_json::from_slice(&serde_json::to_vec(&reset)?)?;
                    vm = Snapshot::capture(&p, &vm)
                        .restore(&p)
                        .map_err(anyhow::Error::msg)?;
                    ensure!(reset.next(&mut vm, &p).map_err(anyhow::Error::msg)? == Some(request));
                }
                match request {
                    R::Script(thread) => {
                        let callback = vm.threads[&thread].callback.is_some();
                        run(&mut vm, &p, thread, &mut trace, callback, restored)?;
                        continue;
                    }
                    R::Service(Request::ReloadAppearance) => {
                        person_reset::reload_bound_profile(&mut vm, &p, 1)
                            .map_err(anyhow::Error::msg)?
                    }
                    R::Service(Request::AttachNeighbor(mode)) => {
                        let person = vm
                            .world
                            .objects
                            .get_mut(&1)
                            .unwrap()
                            .person
                            .as_mut()
                            .unwrap();
                        vm.world
                            .person
                            .as_mut()
                            .unwrap()
                            .attach_person_neighbor(person, 0x44a86a6c, mode)
                            .map_err(anyhow::Error::msg)?;
                    }
                    R::ImmediatePersonUpdate { object, tick } => {
                        ensure!(object == 1 && tick == 0);
                        vm.finish_object_update(1, true)
                            .map_err(anyhow::Error::msg)?;
                        let o = vm.world.objects.get_mut(&1).unwrap();
                        let mut person = [0; 256];
                        for (&i, &value) in o.person.as_ref().unwrap() {
                            person[i as usize] = value;
                        }
                        o.motive_update.as_mut().unwrap().finish(
                            o.motives.as_mut().unwrap(),
                            &mut person,
                            o.data[&34] != 0,
                            tick,
                            vm.world.globals[&0],
                            &tuning,
                            None,
                            &o.motive_changes,
                        );
                        o.person.as_mut().unwrap().insert(27, person[27]);
                    }
                    R::Service(Request::RandomizeTraits) => {
                        vm.randomize_person_traits(1).map_err(anyhow::Error::msg)?
                    }
                    R::Service(Request::LoadScores) => {
                        ensure!(
                            person_loading::load_score_curves(&vm.world, &p, 1)
                                .map_err(anyhow::Error::msg)?
                                .len()
                                == 9
                        );
                    }
                    R::Service(Request::ClearAnimator | Request::IsResidential) => {}
                    other => anyhow::bail!("unexpected constructor boundary {other:?}"),
                }
                reset
                    .complete(
                        &mut vm,
                        &p,
                        request,
                        (request == R::Service(Request::IsResidential)).then_some(true),
                    )
                    .map_err(anyhow::Error::msg)?;
            }
            let actual = state(&vm, vm.object_threads[&1]);
            if actual != n["after"] {
                std::fs::write(
                    "cache/npc-state-mismatch.json",
                    serde_json::to_vec_pretty(
                        &json!({"actual":actual,"expected":n["after"],"trace":trace,"native_trace":n["trace"]}),
                    )?,
                )?;
                anyhow::bail!(
                    "NPC state mismatch {c} restored={restored}; see cache/npc-state-mismatch.json"
                );
            }
            assert_eq!(actual, n["after"], "{c} restored={restored}");
            assert_eq!(
                json!(trace),
                n["trace"],
                "NPC trace {c} restored={restored}"
            );
            assert_eq!(
                Snapshot::capture(&p, &vm)
                    .restore(&p)
                    .map_err(anyhow::Error::msg)?,
                vm
            );
            total += trace.len();
            println!(
                "matched {c}, restored={restored}, {} instructions",
                trace.len()
            );
        }
    }
    let count = report["cases"].as_array().unwrap().len();
    println!(
        "{count} composed native NPC constructors match, post={with_post}, factory={with_factory}, direct and restored; {total} compared instructions"
    );
    Ok(())
}

fn registered(
    vm: &mut Vm,
    p: &Program,
    tuning: &motives::Tuning,
    row: &Value,
    restored: bool,
) -> Result<usize> {
    use sims_vm::person_factory::registered::{Factory, Request as F};
    let definition = vm.world.objects.remove(&1).unwrap().definition.unwrap();
    vm.world.positions.remove(&1);
    vm.world.refresh.as_mut().unwrap().draw.remove(&1);
    vm.world.actions.as_mut().unwrap().actors.remove(&1);
    vm.world.objects.insert(2, Object::default());
    let people = vm.world.person.as_mut().unwrap();
    people.object_order = vec![2];
    people.person_order.clear();
    if row["input"]["cached"] == true {
        people.resource_instances.insert(definition.guid, vec![]);
    }
    let mut factory = Factory::begin(
        vm,
        p,
        definition,
        construction::ObjectDate {
            year: 1997,
            month: 1,
            day: 1,
        },
        [0, 0],
    )
    .map_err(anyhow::Error::msg)?
    .context("factory rejected resource")?;
    ensure!(factory.object() == 1);
    let mut trace = vec![];
    let mut signals = vec![];
    let (mut dirty, mut flags, mut active) = (false, 0u32, false);
    while let Some(request) = factory.next(vm, p).map_err(anyhow::Error::msg)? {
        if restored {
            let bytes = serde_json::to_vec(&(Snapshot::capture(p, vm), &factory))?;
            let (saved, operation): (Snapshot, Factory) = serde_json::from_slice(&bytes)?;
            *vm = saved.restore(p).map_err(anyhow::Error::msg)?;
            factory = operation;
            ensure!(factory.next(vm, p).map_err(anyhow::Error::msg)? == Some(request));
        }
        let mut residential = None;
        match request {
            F::Registered(id) => {
                ensure!(id == 1 && !dirty);
                dirty = true;
                let native = &row["original"]["order"][1];
                ensure!(
                    json!(
                        vm.world.objects[&id]
                            .data
                            .values()
                            .copied()
                            .collect::<Vec<_>>()
                    ) == native["data"]
                );
                ensure!(
                    json!(vm.world.person.as_ref().unwrap().object_order)
                        == native["registration"]["order"]
                );
                ensure!(
                    json!(vm.world.person.as_ref().unwrap().person_order)
                        == native["registration"]["people"]
                );
            }
            F::Post(PostConstructRequest::AutonomyConstants) => anyhow::bail!(
                "NPC oracle requires the source-bound process resource provider; no supplied completion"
            ),
            F::Post(PostConstructRequest::Base) => vm
                .prepare_person_base_postconstruction(p, 1)
                .map_err(anyhow::Error::msg)?,
            F::Post(PostConstructRequest::BindAnimator)
            | F::Reset(R::Service(Request::ReloadAppearance)) => {
                person_reset::reload_bound_profile(vm, p, 1).map_err(anyhow::Error::msg)?;
            }
            F::Post(PostConstructRequest::InitialScores) => {
                let actual = sims_vm::person_loading::initial_score_curves();
                for (curve, native) in actual
                    .iter()
                    .zip(row["original"]["initial_scores"].as_array().unwrap())
                {
                    ensure!(json!(curve.motive) == native["header"][4]);
                    ensure!(
                        json!(curve.points.iter().flatten().copied().collect::<Vec<_>>())
                            == native["points"]
                    );
                }
            }
            F::Post(PostConstructRequest::RefreshResourceProfile) => {}
            F::Reset(R::Script(thread)) => {
                let callback = vm.threads[&thread].callback.is_some();
                run(vm, p, thread, &mut trace, callback, restored)?;
                continue;
            }
            F::Reset(R::Service(Request::AttachNeighbor(mode))) => {
                let person = vm
                    .world
                    .objects
                    .get_mut(&1)
                    .unwrap()
                    .person
                    .as_mut()
                    .unwrap();
                vm.world
                    .person
                    .as_mut()
                    .unwrap()
                    .attach_person_neighbor(person, 0x44a86a6c, mode)
                    .map_err(anyhow::Error::msg)?;
            }
            F::Reset(R::ImmediatePersonUpdate { object, tick }) => {
                ensure!(object == 1 && tick == 0);
                vm.finish_object_update(1, true)
                    .map_err(anyhow::Error::msg)?;
                let o = vm.world.objects.get_mut(&1).unwrap();
                let mut person = [0; 256];
                for (&i, &value) in o.person.as_ref().unwrap() {
                    person[i as usize] = value;
                }
                o.motive_update.as_mut().unwrap().finish(
                    o.motives.as_mut().unwrap(),
                    &mut person,
                    o.data[&34] != 0,
                    tick,
                    vm.world.globals[&0],
                    tuning,
                    None,
                    &o.motive_changes,
                );
                o.person.as_mut().unwrap().insert(27, person[27]);
            }
            F::Reset(R::Service(Request::RandomizeTraits)) => {
                vm.randomize_person_traits(1).map_err(anyhow::Error::msg)?
            }
            F::Reset(R::Service(Request::IsResidential)) => residential = Some(true),
            F::Reset(R::Service(Request::ClearAnimator)) => {}
            F::Reset(R::Service(Request::LoadScores)) => {
                ensure!(
                    person_loading::load_score_curves(&vm.world, p, 1)
                        .map_err(anyhow::Error::msg)?
                        .len()
                        == 9
                );
            }
            F::Activate(id) => {
                ensure!(id == 1 && !active);
                active = true;
                flags |= 0x80;
            }
            F::Created(id) => {
                ensure!(active);
                signals.push([0xe2, id]);
            }
            other => anyhow::bail!("unexpected factory service {other:?}"),
        }
        factory
            .complete(vm, p, request, residential)
            .map_err(anyhow::Error::msg)?;
    }
    let n = &row["original"];
    let actual = state(vm, vm.object_threads[&1]);
    if actual != n["after"] {
        std::fs::write(
            "cache/registered-person-mismatch.json",
            serde_json::to_vec_pretty(&json!({"actual":actual,"native":n["after"]}))?,
        )?;
        anyhow::bail!(
            "registered person state mismatch; see cache/registered-person-mismatch.json"
        );
    }
    ensure!(
        json!(trace) == n["trace"],
        "registered person trace mismatch"
    );
    let registry = vm.world.person.as_ref().unwrap();
    ensure!(json!(registry.object_order) == n["registry"]["order"]);
    ensure!(json!(registry.person_order) == n["registry"]["people"]);
    ensure!(
        json!(
            registry
                .resource_instances
                .get(&0x44a86a6c)
                .cloned()
                .unwrap_or_default()
        ) == n["registry"]["instances"]
    );
    ensure!(json!(u8::from(dirty)) == n["registry"]["dirty"] && json!(flags) == n["native_flags"]);
    let scheduler = u32::from(vm.world.objects[&1].idle.as_ref().unwrap().countdown as u16)
        | (u32::from(active) << 16);
    ensure!(json!(scheduler) == n["scheduler"] && json!(signals) == n["signals"]);
    println!(
        "registered factory {:?}, restored={restored}: {} instructions match",
        row["input"],
        trace.len()
    );
    Ok(trace.len())
}
