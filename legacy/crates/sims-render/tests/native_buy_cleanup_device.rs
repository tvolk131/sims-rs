use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_render::{camera_device::Device, software};
use sims_runtime::original_session::{Saved, Session};
use sims_sim::edit_history::{Store, tool};
use sims_sim::object_catalog::{
    definition::Catalog,
    move_tool::{
        Groups, cleanup,
        dispatch::{Dispatch, Service},
        hover,
    },
};

struct View<'a>(&'a sims_vm::World);
impl cleanup::View for View<'_> {
    fn disposable(&self, _: u32) -> Result<bool, String> {
        Err("empty tool cannot query a held object".into())
    }
    fn script_id(&self, _: u32) -> Result<i16, String> {
        Err("empty tool cannot delete a held object".into())
    }
    fn clear_payload_cost(&mut self, _: u32) -> Result<(), String> {
        Err("empty tool has no payload".into())
    }
}
impl hover::View for View<'_> {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Ok((id > 0 && self.0.objects.contains_key(&id)).then_some(id as u32))
    }
    fn flags(&self, id: u32) -> Result<u32, String> {
        self.0
            .objects
            .get(&(id as i16))
            .and_then(|o| o.native_flags)
            .ok_or("native flags absent".into())
    }
}

#[test]
#[ignore = "private original saved Goth/Mashuga houses; headless software rendering"]
fn real_house_move_cleanup_preserves_vm_execution_and_restores_nested_flags_and_pixels()
-> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join(std::env::var("SIMS_BUY_CLEANUP_OUTPUT")?);
    ensure!(out.starts_with(root.join("cache")) && !out.exists());
    std::fs::create_dir_all(&out)?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reports = vec![];
    for lot in ["goth", "mashuga"] {
        let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join(
            format!("cache/original-ui-timer-dispatch-iced/{lot}/resumed.json"),
        ))?)?;
        let saved: Saved = serde_json::from_value(wrapper["session"].clone())?;
        let initial = Session::restore(&index, saved)?;
        let mut targets = vec![];
        for multipart in [false, true] {
            let (&id, _) = initial
                .simulation
                .vm
                .world
                .objects
                .iter()
                .find(|(id, o)| {
                    o.person.is_none()
                        && o.definition.is_some()
                        && o.native_flags.is_some()
                        && if multipart {
                            o.multitile.as_ref().is_some_and(|m| m.next.is_some())
                                && sims_vm::multitile::dynamic(
                                    &initial.simulation.vm.world,
                                    &initial.program,
                                    **id,
                                ) == Ok(false)
                        } else {
                            o.multitile.is_none()
                        }
                })
                .context("house lacks required ordinary/multipart target")?;
            targets.push((multipart, id));
        }
        for (multipart, id) in targets {
            let mut s = Session::restore(&index, initial.save())?;
            let low = (s.simulation.vm.world.objects[&id].native_flags.unwrap() & 0x1f) | 0x11;
            s.simulation
                .update_native_low_flags(&s.program, id, low)
                .map_err(anyhow::Error::msg)?;
            let mut device = Device::restore(
                &s,
                serde_json::from_value(wrapper["camera_device"].clone())?,
            )?;
            let pixels = software::render(device.scene());
            let before = s.simulation.clone();
            let mut expected = before.clone();
            expected
                .update_native_low_flags(&s.program, id, low & 0xe)
                .map_err(anyhow::Error::msg)?;
            let mut direct = before.clone();
            let mut direct_hover = id;
            let mut transfer = hover::Transfer::new(0, 0);
            direct
                .advance_move_tool_hover(&s.program, &mut direct_hover, &mut transfer)
                .map_err(anyhow::Error::msg)?;
            ensure!(direct_hover == 0 && transfer.complete() && direct == expected);
            let mut state = cleanup::State::constructor_fields(1);
            state.hover = id;
            // Constructed tool-command fixture, not inferred from the imported
            // household. Empty CancelAfter and ref2->1 execute real history
            // code without supplying any virtual object command body.
            let mut history = Store::default();
            history.construct_command(7).map_err(anyhow::Error::msg)?;
            history.acquire(7).map_err(anyhow::Error::msg)?;
            history.acquire(7).map_err(anyhow::Error::msg)?;
            state.payload = Some(7);
            let mut call = Dispatch::activate(u32::MAX);
            // The concrete cursor's Root+40==0 path is intentional. This
            // test binds flag invalidation, not a window cursor implementation.
            let input = cleanup::Inputs {
                registry: 1,
                window: 0,
                key_function: 0,
                control_state: 0,
            };
            let stop = call
                .advance(
                    &mut state,
                    &mut View(&s.simulation.vm.world),
                    input,
                    &Groups::default(),
                    &Catalog::default(),
                    0,
                )
                .map_err(anyhow::Error::msg)?;
            ensure!(stop == Some(Service::External(cleanup::Request::StopPayload(7))));
            let mut child = tool::Call::new(cleanup::Request::StopPayload(7))
                .map_err(anyhow::Error::msg)?
                .context("missing history bridge")?;
            ensure!(child.finish(&mut call).is_err());
            (child, history, call, state) =
                serde_json::from_slice(&serde_json::to_vec(&(&child, &history, &call, &state))?)?;
            ensure!(
                child
                    .advance(&mut history)
                    .map_err(anyhow::Error::msg)?
                    .is_none()
            );
            child.finish(&mut call).map_err(anyhow::Error::msg)?;
            let request = call
                .advance(
                    &mut state,
                    &mut View(&s.simulation.vm.world),
                    input,
                    &Groups::default(),
                    &Catalog::default(),
                    0,
                )
                .map_err(anyhow::Error::msg)?
                .context("hover child absent")?;
            ensure!(matches!(request, Service::Flags(_)) && state.hover == id);
            let bytes = serde_json::to_vec(&(s.save(), device.save(&s)?, &call, &state, &history))?;
            let (saved, saved_device, mut cold_call, mut cold_state, mut cold_history): (
                Saved,
                _,
                Dispatch,
                cleanup::State,
                Store,
            ) = serde_json::from_slice(&bytes)?;
            s = Session::restore(&index, saved)?;
            device = Device::restore(&s, saved_device)?;
            ensure!(software::render(device.scene()) == pixels);
            ensure!(
                cold_call
                    .advance(
                        &mut cold_state,
                        &mut View(&s.simulation.vm.world),
                        input,
                        &Groups::default(),
                        &Catalog::default(),
                        0
                    )
                    .map_err(anyhow::Error::msg)?
                    == Some(request)
            );
            let Service::Flags(r) = request else {
                anyhow::bail!("unexpected external child");
            };
            s.simulation
                .update_native_low_flags(&s.program, r.object as i16, r.low)
                .map_err(anyhow::Error::msg)?;
            cold_call
                .acknowledge(request, cleanup::Reply::Done)
                .map_err(anyhow::Error::msg)?;
            (cold_call, cold_state) =
                serde_json::from_slice(&serde_json::to_vec(&(&cold_call, &cold_state))?)?;
            let release = cold_call
                .advance(
                    &mut cold_state,
                    &mut View(&s.simulation.vm.world),
                    input,
                    &Groups::default(),
                    &Catalog::default(),
                    0,
                )
                .map_err(anyhow::Error::msg)?;
            ensure!(release == Some(Service::External(cleanup::Request::ReleasePayload(7))));
            let mut child = tool::Call::new(cleanup::Request::ReleasePayload(7))
                .map_err(anyhow::Error::msg)?
                .context("missing release bridge")?;
            (child, cold_history, cold_call, cold_state) = serde_json::from_slice(
                &serde_json::to_vec(&(&child, &cold_history, &cold_call, &cold_state))?,
            )?;
            ensure!(
                child
                    .advance(&mut cold_history)
                    .map_err(anyhow::Error::msg)?
                    .is_none()
            );
            ensure!(
                cold_history
                    .command(7)
                    .map_err(anyhow::Error::msg)?
                    .references
                    == 1
            );
            child.finish(&mut cold_call).map_err(anyhow::Error::msg)?;
            ensure!(
                cold_call
                    .advance(
                        &mut cold_state,
                        &mut View(&s.simulation.vm.world),
                        input,
                        &Groups::default(),
                        &Catalog::default(),
                        0
                    )
                    .map_err(anyhow::Error::msg)?
                    .is_none()
            );
            ensure!(cold_call.complete() && cold_state.hover == 0);
            ensure!(
                s.simulation == expected,
                "composed cleanup differs from actual low-flag child"
            );
            let changed: Vec<_> = s
                .simulation
                .vm
                .world
                .objects
                .iter()
                .filter_map(|(&id, object)| {
                    (object.native_flags != before.vm.world.objects[&id].native_flags).then_some(id)
                })
                .collect();
            ensure!(!changed.is_empty() && (!multipart || changed.len() > 1));
            // Normalize exactly the allowed writes; all VM frames, stacks,
            // clocks, RNG, motives, queues, routing and world links must match.
            let mut normalized = s.simulation.clone();
            normalized.vm.world.refresh = before.vm.world.refresh.clone();
            for (&id, object) in &mut normalized.vm.world.objects {
                let old = before.vm.world.objects[&id].native_flags;
                ensure!(object.native_flags.unwrap_or(0) & !0x1f == old.unwrap_or(0) & !0x1f);
                object.native_flags = old;
            }
            ensure!(
                normalized == before,
                "cleanup changed simulation beyond low flags and draw invalidation"
            );
            device.refresh(&s)?;
            let after = software::render(device.scene());
            let saved = s.save();
            let saved_device = device.save(&s)?;
            s = Session::restore(
                &index,
                serde_json::from_slice(&serde_json::to_vec(&saved)?)?,
            )?;
            device = Device::restore(
                &s,
                serde_json::from_slice(&serde_json::to_vec(&saved_device)?)?,
            )?;
            ensure!(s.simulation == expected && software::render(device.scene()) == after);
            let name = format!("{lot}-{}", if multipart { "multipart" } else { "ordinary" });
            after.save(out.join(format!("{name}.png")))?;
            reports.push(json!({"lot":lot,"multipart":multipart,"target":id,"changed_objects":changed,
                "tick":s.simulation.vm.tick,"vm_execution_unchanged":true,"cold_restore_pixels_equal":true,
                "pixels_unchanged":after==pixels,"cursor_window_present":false,
                "empty_cancel_after_executed":true,"release_reference_count":1,
                "history_fixture":"constructed ref2 tool command; no object virtual bodies supplied"}));
        }
    }
    std::fs::write(
        out.join("report.json"),
        serde_json::to_vec_pretty(&json!({"cases":reports,"native_execution":false,
        "scope":"Composed activation/deactivation/reset/hover plus actual empty cancellation and reference release on real households. Actual multipart low-flag invalidation; no window cursor, object deletion, purchase, or full Buy-mode binding."}))?,
    )?;
    Ok(())
}
