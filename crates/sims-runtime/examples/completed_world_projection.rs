//! Private original-input model join; deliberately not NativeHousehold admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_runtime::{
    conformance::completed_reentry,
    original_session::{completed_person_world, completed_world},
};
use sims_vm::{
    retained_completed::{CompletedAdapterEpoch, IdentityMapping, ObjectThreadBinding},
    retained_epoch::{FreshUnpublishedRequestEpoch, VmProjection},
};
use std::{fs, path::Path};

fn read(path: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 4,
        "usage: completed_world_projection REENTRY PERSON_WORLD SYSTEMS NEW_OUTPUT_DIR"
    );
    let parts = completed_reentry::import_parts(read(&args[0])?)?;
    let p = read(&args[1])?;
    let person = completed_person_world::import(
        p["person_fields"].clone(),
        serde_json::from_value(p["native_relationship_orders"].clone())?,
        p["retained_external_family_owners"].clone(),
        serde_json::from_value(p["family_delivery"].clone())?,
    )?;
    let systems = read(&args[2])?;
    ensure!(
        systems["provenance"]["image_sha256"] == parts.provenance["image_sha256"]
            && systems["provenance"]["observation_sha256"]
                == parts.provenance["observation_sha256"]
            && systems["delivery"]["epoch"] == parts.source_epoch,
        "completed model source epoch differs"
    );
    for receipt in systems["reads"]
        .as_array()
        .context("system read receipts absent")?
    {
        let raw = sims_runtime::original_session::completed_animations::bytes(
            receipt["raw"].as_str().context("system raw bytes absent")?,
        )?;
        ensure!(
            receipt["bytes"] == raw.len() && receipt["sha256"] == sims_fixtures::digest(&raw),
            "system receipt differs"
        );
    }
    let (program, profiles) =
        completed_world::program::join(parts.program, &systems["body_profiles"])?;
    let world = completed_world::join(
        systems["fields"].clone(),
        completed_world::Owners {
            objects: parts.objects,
            scheduler: parts.registry,
            person: person.state,
            body_profiles: profiles,
        },
    )?;
    let source_epoch = parts.source_epoch;
    let threads = parts
        .entries
        .iter()
        .map(|e| IdentityMapping {
            source_owner: e.source_owner.clone(),
            adapter: e.adapter_thread,
        })
        .collect();
    let next_thread = parts
        .entries
        .iter()
        .map(|e| e.adapter_thread)
        .max()
        .context("no retained threads")?
        + 1;
    let bindings = parts
        .entries
        .iter()
        .map(|e| {
            let object = e.object.context("completed object association absent")?;
            let sims_vm::Status::NativeRegistryReentry(owner) = &e.controls.status else {
                anyhow::bail!("completed lifecycle needs its qualified native reentry owner")
            };
            let definition = world.objects[&object]
                .definition
                .as_ref()
                .context("lifecycle definition absent")?;
            Ok(ObjectThreadBinding {
                object,
                thread: Some(e.adapter_thread),
                lifecycle: Some(sims_vm::lifecycle::Lifecycle {
                    owner: definition.source.clone(),
                    definition: definition.object,
                    init: owner.evidence().cached_init,
                    main: owner.evidence().cached_main,
                    phase: sims_vm::lifecycle::Phase::Main,
                    thread: e.adapter_thread,
                    run_init: owner.closed_group_init_permission(),
                }),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let projection: VmProjection = serde_json::from_value(systems["projection"].clone())?;
    let world_bytes = serde_json::to_vec(&world)?;
    let vm = sims_vm::Vm::from_retained_completed_epoch(
        &program,
        world,
        projection,
        parts.entries,
        bindings,
        CompletedAdapterEpoch {
            fresh: FreshUnpublishedRequestEpoch {
                source_key: source_epoch,
                accepted_commands: 0,
                published_thread_handles: 0,
                published_request_handles: 0,
            },
            threads,
            requests: vec![],
            next_thread,
            next_request: 1,
        },
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        serde_json::to_vec(&vm.world)? == world_bytes,
        "completed VM construction altered supplied World"
    );
    let before = sims_vm::Snapshot::capture(&program, &vm);
    let out = Path::new(&args[3]);
    fs::create_dir(out)?;
    // Named MessagePack supports the existing named/optional World fields;
    // positional Postcard cannot encode serde-skipped field positions safely.
    let store = sims_fixtures::Store::create(out.join("model-store"))?;
    let saved = store.put_value(&before)?;
    let restored: sims_vm::Snapshot = store.get_value(&saved)?;
    let restored = restored.restore(&program).map_err(anyhow::Error::msg)?;
    ensure!(restored == vm, "completed VM model cold restore differs");
    let program_saved = store.put_value(&program)?;
    fs::write(
        out.join("model-refs.json"),
        serde_json::to_vec_pretty(&json!({"vm": saved, "program": program_saved}))?,
    )?;
    let report = json!({"status": "model_assembled", "tick": vm.tick, "objects": vm.world.objects.len(),
        "threads": vm.threads.len(), "world_fields": 27, "constructors_executed": 0,
        "ticks_executed": 0, "native_parity_ticks": 0, "cold_vm_equal": true,
        "lazy_bank_demands": systems["body_profiles"]["demands"],
        "complete_native_Session_qualified": false,
        "remaining": ["partial resident Program and current lazy resource/provider provenance",
            "process initialization provenance; current captured loaded fastpath only", "complete Simulation and live Host composition",
            "new pair baseline qualification", "ordered native one-tick trace and strict warm/cold comparison"]});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"status": report["status"], "objects": report["objects"],
        "threads": report["threads"], "cold_vm_equal": report["cold_vm_equal"],
        "native_parity_ticks": 0, "complete_native_Session_qualified": false,
        "report": out.join("report.json")})
    );
    Ok(())
}
