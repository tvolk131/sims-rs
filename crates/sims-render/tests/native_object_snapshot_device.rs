use anyhow::{Context, Result, ensure};
use sims_content::{Index, object_catalog::Catalog, object_resources::Overlay};
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::edit_history::snapshot::{
    self, CaptureView, References, Snapshot, lifecycle::Destroy, live::View,
};
use std::{cell::RefCell, sync::Arc};

#[test]
#[ignore = "private original Goth/Mashuga houses; real capture, catalogue identity and headless pixels"]
fn original_house_snapshot_destruction_preserves_objects_and_pixels() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_OBJECT_SNAPSHOT_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Arc::new(Index::scan(&root.join("game-data/cabinet-extracted"))?);
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let neighborhood = format!(
            "{}/Neighborhood.iff",
            saved
                .source
                .split('/')
                .next()
                .context("house source directory absent")?
        );
        let catalog = RefCell::new(Catalog::household(
            index.clone(),
            &neighborhood,
            Overlay::default(),
        )?);
        let catalog_identity = catalog.borrow().identity().map_err(anyhow::Error::msg)?;
        let resolve = |d: &sims_vm::Definition| catalog.borrow_mut().live_dialog_resource(d);
        let mut session = Session::restore(&index, saved)?;
        let mut device = Device::restore(
            &session,
            serde_json::from_value(wrapper["camera_device"].clone())?,
        )?;
        let pixels = software::render(device.scene());
        let before = session.simulation.clone();
        let mut expected = before.clone();
        let mut targets = vec![];
        for multipart in [false, true] {
            let (&id, _) = before
                .vm
                .world
                .objects
                .iter()
                .find(|(id, o)| {
                    o.person.is_none()
                        && o.definition.is_some()
                        && o.data.contains_key(&1)
                        && before
                            .native_lot
                            .as_ref()
                            .unwrap()
                            .placement
                            .links
                            .capture_raw(&before.vm.world, **id)
                            .is_ok_and(|d| d.on_lot)
                        && if multipart {
                            o.multitile.as_ref().is_some_and(|m| m.head.is_some())
                                && sims_vm::multitile::dynamic(
                                    &before.vm.world,
                                    &session.program,
                                    **id,
                                ) == Ok(false)
                        } else {
                            o.multitile.is_none()
                        }
                })
                .context("required on-lot snapshot target absent")?;
            targets.push((multipart, id));
        }
        // This explicitly constructed subsystem starts with counter1. It does
        // not infer an absent historical undo catalogue from the imported lot.
        let mut refs = References::new(1);
        let mut snapshots = vec![];
        let mut rows = vec![];
        for (multipart, id) in targets {
            let normalized = if multipart {
                before.vm.world.objects[&id]
                    .multitile
                    .as_ref()
                    .unwrap()
                    .head
                    .unwrap_or(id)
            } else {
                id
            };
            let expected_position = expected
                .native_lot
                .as_ref()
                .unwrap()
                .placement
                .links
                .capture(&mut expected.vm.world, normalized)
                .map_err(anyhow::Error::msg)?;
            for copy in 0..2 {
                let mut saved = Snapshot::default();
                let sim = &mut session.simulation;
                let mut view = View {
                    world: &mut sim.vm.world,
                    program: &session.program,
                    links: &sim.native_lot.as_ref().unwrap().placement.links,
                    definition: &resolve,
                };
                ensure!(
                    snapshot::capture(&mut saved, &mut refs, id, &mut view)
                        .map_err(anyhow::Error::msg)?
                );
                ensure!(
                    saved.position == expected_position.position
                        && saved.level == Some(expected_position.level)
                        && saved.slot == Some(i32::from(expected_position.slot as i16))
                );
                ensure!(refs.script_id(&saved).map_err(anyhow::Error::msg)? == normalized);
                let handle = view
                    .world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .handle(normalized)
                    .context("native lifetime handle absent")?;
                ensure!(refs.object(&saved).map_err(anyhow::Error::msg)? == Some(handle));
                ensure!(
                    saved.facing
                        == Some(i32::from(
                            CaptureView::facing(&view, handle).map_err(anyhow::Error::msg)?
                        ))
                );
                rows.push(serde_json::json!({"multipart":multipart,"requested":id,"normalized":normalized,"handle":handle,"copy":copy,"snapshot":saved}));
                snapshots.push(saved);
            }
        }
        ensure!(
            session.simulation == expected,
            "snapshot capture changed state beyond native physical normalization"
        );
        ensure!(refs.records.len() == 2 && refs.records.values().all(|r| r.count == 2));
        let bytes =
            serde_json::to_vec(&(session.save(), device.save(&session)?, &refs, &snapshots))?;
        let (saved, saved_device, cold_refs, cold_snapshots): (
            Saved,
            _,
            References,
            Vec<Snapshot>,
        ) = serde_json::from_slice(&bytes)?;
        session = Session::restore(&index, saved)?;
        device = Device::restore(&session, saved_device)?;
        refs = cold_refs;
        snapshots = cold_snapshots;
        refs.validate().map_err(anyhow::Error::msg)?;
        ensure!(session.simulation == expected && software::render(device.scene()) == pixels);
        for saved in &snapshots {
            let mut release = Destroy::default();
            release = serde_json::from_slice(&serde_json::to_vec(&release)?)?;
            let sim = &mut session.simulation;
            let mut view = View {
                world: &mut sim.vm.world,
                program: &session.program,
                links: &sim.native_lot.as_ref().unwrap().placement.links,
                definition: &resolve,
            };
            ensure!(
                release
                    .advance(saved, &mut refs, &mut view)
                    .map_err(anyhow::Error::msg)?
                    .is_none()
                    && release.complete(),
                "on-lot snapshot requested object deletion"
            );
        }
        ensure!(refs.records.is_empty() && refs.order.is_empty() && session.simulation == expected);
        device.refresh(&session)?;
        let after = software::render(device.scene());
        ensure!(after == pixels);
        ensure!(catalog.borrow().identity().map_err(anyhow::Error::msg)? == catalog_identity);
        after.save(out.join(format!("{lot}.png")))?;
        reports.push(serde_json::json!({"lot":lot,"tick":session.simulation.vm.tick,"captures":rows,"catalog_identity":catalog_identity,"catalog_unchanged":true,
            "vm_matches_native_capture_only":true,"snapshot_destructor_executed":true,"references_released_without_object_deletion":true,"cold_restore_pixels_equal":true,"pixels_unchanged":true}));
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"cases":reports,"native_execution":false,
        "scope":"Actual original-lot snapshot capture via registry lifetime handles and retained installed definitions. Shared references survive Rust save/restore and48bbf7 destruction releases them without deleting on-lot objects. No object movement, purchase or complete Buy-mode binding claimed."}),
        )?,
    )?;
    Ok(())
}
