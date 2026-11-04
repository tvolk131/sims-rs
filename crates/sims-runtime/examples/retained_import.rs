//! Read-only diagnostic at the original ObjM reader's return, before controller
//! reconciliation and post-load callbacks. It uses the production reader.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_content::{
    Index, character_loading::Reader, house_startup::Incoming, object_catalog::Catalog,
    saves::House,
};
use sims_runtime::original_session::View;
use std::{fs::File, path::Path, sync::Arc};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: retained_import CONTENT_ROOT SOURCE STARTUP_REPORT SKELETON_CACHE NEW_OUTPUT"
    );
    let output = Path::new(&args[4]);
    ensure!(
        output
            .parent()
            .context("output parent")?
            .canonicalize()?
            .starts_with(Path::new("cache").canonicalize()?),
        "private output required"
    );
    let file = File::create_new(output)?;
    let index = Index::scan(Path::new(&args[0]))?;
    let house = House::load(&index, &args[1])?;
    let report: Value = serde_json::from_reader(File::open(&args[2])?)?;
    let incoming: Incoming = serde_json::from_value(report["incoming"].clone())?;
    let mut catalog = Catalog::household(
        Arc::new(index.clone()),
        &house.neighborhood_source,
        Default::default(),
    )?;
    let controllers = catalog.global_controllers(&index)?;
    let mut program = house.link_program(&index)?;
    sims_content::catalog::link_controllers(&index, &mut program, &controllers)?;
    program.resource_storage = Some(catalog.resource_storage(&index)?);
    program.neighbor_resource_flags = Some(
        catalog
            .definitions
            .guids
            .iter()
            .map(|(&guid, &i)| (guid, catalog.definitions.entries[i].flags))
            .collect(),
    );
    let guids = incoming
        .vm
        .world
        .objects
        .values()
        .filter_map(|o| o.definition.as_ref().map(|d| d.guid))
        .collect::<Vec<_>>();
    sims_content::assets::load_object_guids(&index, &mut program, &guids)?;
    let mut reader = Reader::new(
        &index,
        &house,
        sims_sim::pose::animator::Settings {
            fade_enabled: true,
            fade_duration: 200,
            sampling_config: 1,
        },
        [0.; 3],
        incoming.vm.crt_rng_state.context("incoming CRT")?,
        View::default(),
    )?;
    reader.skeletons = serde_json::from_reader(File::open(&args[3])?)?;
    reader.skeletons.validate(&index)?;
    let vm = incoming.prepare_read(
        &house,
        &program,
        &controllers,
        |_, _, _| Ok(()),
        &mut |_, _| Err("incoming asset service unavailable".into()),
    )?;
    let mut execution =
        house.import_saved_execution_with_person_reader(&mut program, &mut reader, vm)?;
    let imported = json!({"state":state(&execution.vm, reader.crt_state),"vm":execution.vm});
    //43fc02: successful ObjM read ->442b9f reconciliation ->443cb3 grid
    //repair. Do not select the resident family or execute post-load visits.
    execution.reconcile_global_controllers(&program, &controllers)?;
    let grid = execution.repair_object_grid(&house)?;
    execution.restore_placement_links(&house, &grid)?;
    let reconciled = json!({"state":state(&execution.vm, reader.crt_state),"vm":execution.vm});
    let architecture = sims_sim::architecture::Architecture::from_original(
        house.lot_geometry(&grid.floors)?,
        &program,
    )
    .map_err(anyhow::Error::msg)?;
    architecture.publish_room_context(&mut execution.vm.world);
    let mut membership = sims_vm::person_loading::RoomMembership::constructed(&execution.vm.world);
    let mut effects = vec![];
    let callbacks = execution.update_loaded_rooms(
        &mut program,
        &architecture,
        &mut membership,
        0,
        &mut effects,
        &mut |_, _, _| Err("room callback needs its native service owner".into()),
    )?;
    let rooms = json!({"state":state(&execution.vm, reader.crt_state),"vm":execution.vm,
        "callbacks":callbacks,"membership":membership,"effects":effects});
    serde_json::to_writer(
        file,
        &json!({"objm_read_return":imported,"objects_read_return":reconciled,"rooms_return":rooms}),
    )?;
    eprintln!(
        "{} objects after reconciliation; reader CRT {}",
        execution.vm.world.objects.len(),
        reader.crt_state
    );
    Ok(())
}

fn state(vm: &sims_vm::Vm, crt: u32) -> Value {
    // Same explicit gameplay contract as newbie_parity. Keep the complete VM
    // separately for investigating fields excluded from that comparator.
    json!({
        "tick":vm.tick,"global_rng":vm.rng_state,"lot_rng":vm.lot_rng_state,
        // During deserialization the character host owns the process CRT.
        // Session publishes it back to the VM when reader.into_live returns.
        "crt_rng":crt,"globals":vm.world.globals,
        "actions":vm.world.actions,"positions":vm.world.positions,
        "objects":vm.world.objects.iter().map(|(&id,o)| json!({
            "id":id,"guid":o.definition.as_ref().map(|d|d.guid),
            "data":o.data,"temps":o.temps,"attributes":o.attributes,
            "registry_word":o.idle.as_ref().map(|i|i.registry_word()),
            "person":o.person,"motives":o.motives,"motive_update":o.motive_update,
            "thread":vm.object_threads.get(&id).map(|id| &vm.threads[id])
        })).collect::<Vec<_>>()
    })
}
