//! Retained source geometry and cold restore. No VM tick or load pass executes.
use anyhow::{Result, ensure};
use serde_json::json;
use sims_runtime::original_session::completed_lot;
use std::{fs, path::Path};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 || (4..=5).contains(&args.len()) || args.len() == 7,
        "usage: completed_lot_geometry LOT NEW_OUTPUT_DIR [VM_STORE VM_REFS [ENVIRONMENT [REENTRY SYSTEMS]]]"
    );
    let input = serde_json::from_slice(&fs::read(&args[0])?)?;
    let imported = completed_lot::import(&input)?;
    let out = Path::new(&args[1]);
    fs::create_dir(out)?;
    let store = sims_fixtures::Store::create(out.join("store"))?;
    fs::write(
        out.join("empty-store-directories.json"),
        serde_json::to_vec_pretty(&json!(["cases", "pins", "runs", "tmp"]))?,
    )?;
    let reference = store.put_value(&imported)?;
    let cold: completed_lot::Imported = store.get_value(&reference)?;
    ensure!(cold == imported, "native geometry cold restore differs");
    if args.len() >= 4 {
        let refs: serde_json::Value = serde_json::from_slice(&fs::read(&args[3])?)?;
        let vm_ref = serde_json::from_value(refs["vm"].clone())?;
        let vm_store = sims_fixtures::Store::open(&args[2])?;
        let snapshot: sims_vm::Snapshot = vm_store.get_value(&vm_ref)?;
        ensure!(snapshot.state.tick == 88448, "physical VM epoch differs");
        let before = serde_json::to_vec(&snapshot)?;
        let physical = completed_lot::physical::import(&imported, &snapshot.state.world)?;
        ensure!(
            serde_json::to_vec(&snapshot)? == before,
            "physical borrowed validation mutated World"
        );
        let saved = store.put_value(&physical)?;
        let restored: completed_lot::physical::Imported = store.get_value(&saved)?;
        ensure!(restored == physical, "physical cold restore differs");
        let saved_world = store.put_value(&snapshot)?;
        let restored_world: sims_vm::Snapshot = store.get_value(&saved_world)?;
        ensure!(
            restored_world == snapshot,
            "physical source VM cold restore differs"
        );
        fs::write(
            out.join("world-refs.json"),
            serde_json::to_vec_pretty(&json!({"vm":saved_world}))?,
        )?;
        if args.len() >= 5 {
            let source = serde_json::from_slice(&fs::read(&args[4])?)?;
            let environment =
                completed_lot::environment::import(&imported, &source, &snapshot.state.world)?;
            ensure!(
                serde_json::to_vec(&snapshot)? == before,
                "room import changed VM"
            );
            let saved = store.put_value(&environment)?;
            let restored: completed_lot::environment::Imported = store.get_value(&saved)?;
            ensure!(
                restored == environment,
                "room environment cold restore differs"
            );
            fs::write(
                out.join("environment-ref.json"),
                serde_json::to_vec_pretty(&saved)?,
            )?;
            fs::write(
                out.join("environment-report.json"),
                serde_json::to_vec_pretty(&json!({"rooms":environment.manager.rooms.len(),
                "light_pool":environment.manager.settings.models,"local_light_cells":environment.manager.light_maps.iter().map(|m|m.cells.iter().filter(|c|**c!=[127;4]).count()).collect::<Vec<_>>(),
                "room_people":environment.manager.rooms.iter().map(|(&id,r)|(id,r.totals.people)).collect::<std::collections::BTreeMap<_,_>>(),
                "world_unchanged":true,"cold_equal":true,"native_parity_ticks":0,"complete_native_lot_admitted":false}))?,
            )?;
        }
        if args.len() == 7 {
            let source = serde_json::from_slice(&fs::read(&args[5])?)?;
            let parts = sims_runtime::conformance::completed_reentry::import_parts(source)?;
            let systems: serde_json::Value = serde_json::from_slice(&fs::read(&args[6])?)?;
            ensure!(
                systems["provenance"]["image_sha256"]
                    == imported.source["provenance"]["image_sha256"]
                    && systems["provenance"]["observation_sha256"]
                        == imported.source["provenance"]["observation_sha256"],
                "architecture system source epoch differs"
            );
            let (program, _) = sims_runtime::original_session::completed_world::program::join(
                parts.program,
                &systems["body_profiles"],
            )?;
            ensure!(
                program.fingerprint() == snapshot.program_hash,
                "architecture Program/source VM differs"
            );
            let architecture =
                completed_lot::architecture(&imported, &program, &snapshot.state.world)?;
            let saved = store.put_value(&architecture)?;
            let restored: sims_sim::architecture::Architecture = store.get_value(&saved)?;
            ensure!(
                restored == architecture && serde_json::to_vec(&snapshot)? == before,
                "architecture cold restore/VM differs"
            );
            fs::write(
                out.join("architecture-ref.json"),
                serde_json::to_vec_pretty(&saved)?,
            )?;
            fs::write(
                out.join("architecture-report.json"),
                serde_json::to_vec_pretty(&json!({"footprints":architecture.footprints.len(),
                "portals":architecture.footprints.values().filter(|f|f.portal).count(),"live_objects":snapshot.state.world.objects.len(),
                "world_unchanged":true,"cold_equal":true,"native_parity_ticks":0,"complete_native_lot_admitted":false}))?,
            )?;
        }
        fs::write(
            out.join("physical-ref.json"),
            serde_json::to_vec_pretty(&saved)?,
        )?;
        fs::write(
            out.join("physical-report.json"),
            serde_json::to_vec_pretty(&json!({"objects":physical.placement.links.next.len(),
            "memberships":physical.membership,"indicators":physical.indicators,"pools":physical.pools,
            "selected_person":physical.selected_person,"world_unchanged":true,"cold_equal":true,"native_parity_ticks":0}))?,
        )?;
    }
    fs::write(
        out.join("geometry-ref.json"),
        serde_json::to_vec_pretty(&reference)?,
    )?;
    fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&imported.comparison)?,
    )?;
    println!(
        "{}",
        json!({"source_geometry_equal":true,"backing":imported.lot.size,"active":imported.lot.active_extent(),
        "floors":imported.lot.floors.len(),"cold_equal":true,"native_parity_ticks":0,"complete_native_lot_admitted":false})
    );
    Ok(())
}
