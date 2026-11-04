#![cfg(feature = "private-fixtures")]

//! Explicit actual raw1 callback body on a fresh asset-backed Goth Session.
//! Source-bound initial text/capacity; not the exact postload prefix or GUI cadence.
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use sims_content::Index;
use sims_runtime::original_session::{
    Options, Session,
    window_update::{ApplicationFamily, Binding, CountedBinding, Progress, StorageProfile},
};
use sims_vm::family_label::{Fields, NumericRequest, Request};
use std::{fs, path::PathBuf, time::Instant};
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn logical(family: ApplicationFamily) -> Binding {
    Binding {
        fields: Fields {
            owner: 100,
            state: Some(101),
            label: Some(102),
        },
        application_family: family,
        counted_text: [(
            101,
            CountedBinding {
                bytes: vec![],
                capacity: Some(256),
                profile: Some(StorageProfile::CompleteCollection191Inline256Observed20261025),
            },
        )]
        .into(),
        label_text: [(102, vec![])].into(),
    }
}
#[test]
#[ignore = "private original assets and retained source-bound startup callback evidence"]
fn actual_goth_explicit_callback_before_poll_preserves_every_service_and_ready_cold_cut()
-> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("family-window startup proof panicked")
}
fn cold(warm: &Session, index: &Index, out: &std::path::Path, cut: usize) -> Result<Session> {
    let before = serde_json::to_vec(&warm.save())?;
    let managed = serde_json::to_vec(&warm.save_managed()?)?;
    let restored = match Session::restore(index, serde_json::from_slice(&managed)?) {
        Ok(s) => s,
        Err(e) => {
            fs::write(out.join("first-failure-managed.json"), &managed)?;
            return Err(e);
        }
    };
    let after = serde_json::to_vec(&restored.save())?;
    if before != after {
        fs::write(out.join("first-failure-warm.json"), &before)?;
        fs::write(out.join("first-failure-cold.json"), &after)?;
        anyhow::bail!("family window full state/RNG differs at cut{cut}");
    }
    ensure!(
        managed == serde_json::to_vec(&restored.save_managed()?)?,
        "managed resave differs at cut{cut}"
    );
    ensure!(
        before == serde_json::to_vec(&warm.save())?,
        "passive save mutated cut{cut}"
    );
    Ok(restored)
}
fn pair(warm: &Session, resumed: &Session, out: &std::path::Path, cut: usize) -> Result<()> {
    let a = serde_json::to_vec(&warm.save())?;
    let b = serde_json::to_vec(&resumed.save())?;
    if a != b {
        fs::write(out.join("first-failure-warm-execution.json"), a)?;
        fs::write(out.join("first-failure-cold-execution.json"), b)?;
        anyhow::bail!("warm/cold execution state differs at cut{cut}");
    }
    Ok(())
}
/// Authored guard fixture, published by ordinary pump_dialogs. Its text/record
/// is not asserted to be an original captured modal or a managed-save fixture.
fn displayed_modal_guard(base: &Session, family: i16) -> Result<()> {
    let untouched = serde_json::to_vec(&base.save())?;
    let mut s = base.fork_for_exploration()?;
    ensure!(
        s.simulation.vm.world.dialogs.records.is_empty() && !s.ui_wait_pending(),
        "modal guard requires an ordinary clean presentation boundary"
    );
    let mut location = None;
    'modules: for (source, module) in &s.program.modules {
        for (&bhav, routine) in &module.routines {
            if bhav < 4096 {
                continue;
            }
            for (instruction, ins) in routine.body.instructions.iter().enumerate() {
                if ins.opcode == 0x24
                    && sims_vm::dialog::presentation::supported(ins.operands[5])
                    && ins.operands[7] & 128 == 0
                {
                    location = Some((source.clone(), bhav, instruction, ins.operands));
                    break 'modules;
                }
            }
        }
    }
    let (source, bhav, instruction, operands) =
        location.context("no existing supported modal Dialog instruction")?;
    let caller = *s
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .unwrap()
        .person_order
        .first()
        .context("modal fixture caller absent")?;
    let buttons = sims_vm::dialog::presentation::buttons(operands[5], [0; 3])
        .into_iter()
        .map(|button| sims_vm::dialog::Button {
            label: "Authored guard fixture".into(),
            result: button.result,
        })
        .collect();
    s.simulation
        .vm
        .world
        .dialogs
        .records
        .push(sims_vm::dialog::Record {
            presentation: Some(Default::default()),
            id: 1,
            caller,
            origin: sims_vm::Location {
                source: source.clone(),
                bhav,
                instruction,
            },
            source,
            operands,
            stack_object: caller,
            title: "Authored modal guard fixture".into(),
            body: "No original modal appearance or protocol claim".into(),
            buttons,
            icon_object: None,
            job_icon: None,
            bitmap_icon: None,
            global_bitmap_icon: None,
            portrait: None,
            status: 0,
        });
    s.simulation.vm.world.ui_owner.dialog_owners.push(caller);
    ensure!(
        s.simulation.vm.pump_dialogs().map_err(anyhow::Error::msg)? == Some(1),
        "ordinary pump did not publish modal"
    );
    let dialogs = serde_json::to_vec(&s.simulation.vm.world.dialogs)?;
    let mut expected = s.simulation.clone();
    expected
        .vm
        .world
        .person
        .as_mut()
        .unwrap()
        .resolve_family_friend_count(family, expected.vm.world.autonomy_constants.as_ref())
        .map_err(anyhow::Error::msg)?;
    s.bind_family_window(logical(ApplicationFamily::Family(family)))?;
    s.begin_family_window_update(1)?;
    for _ in 0..160 {
        match s.advance_family_window_update(1)? {
            Progress::Complete => break,
            Progress::Pending => (),
            Progress::Service(request) => s.service_family_window_request(&request)?,
        }
    }
    ensure!(
        !s.family_window_pending(),
        "displayed-modal callback did not finish"
    );
    ensure!(
        s.simulation.vm.world.dialogs.modal == Some(1) && !s.ui_wait_pending(),
        "callback consumed modal or started UIWait"
    );
    ensure!(
        dialogs == serde_json::to_vec(&s.simulation.vm.world.dialogs)?,
        "callback changed displayed modal"
    );
    ensure!(
        serde_json::to_vec(&expected)? == serde_json::to_vec(&s.simulation)?,
        "modal callback changed simulation beyond getter"
    );
    ensure!(
        untouched == serde_json::to_vec(&base.save())?,
        "guard fixture mutated original branch"
    );
    Ok(())
}
fn run() -> Result<()> {
    let repo =
        PathBuf::from(std::env::var_os("SIMS_FCNS_REPO").context("SIMS_FCNS_REPO required")?);
    let assets =
        PathBuf::from(std::env::var_os("SIMS_FCNS_ASSETS").context("SIMS_FCNS_ASSETS required")?);
    let immutable = PathBuf::from(
        std::env::var_os("SIMS_FCNS_IMMUTABLE").context("SIMS_FCNS_IMMUTABLE required")?,
    );
    let out = PathBuf::from(
        std::env::var_os("SIMS_FAMILY_WINDOW_OUT").context("SIMS_FAMILY_WINDOW_OUT required")?,
    );
    fs::create_dir_all(&out)?;
    let input: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/family-window-startup.json"))?;
    let bytes = fs::read(repo.join(input["audit_source"].as_str().unwrap()))?;
    ensure!(
        hash(&bytes) == input["audit_sha256"].as_str().unwrap(),
        "native caller evidence changed"
    );
    ensure!(
        input["calls"][0]["dirty"] == 1
            && input["calls"][1]["dirty"] == 0
            && input["calls"][0]["family"] == 5
            && input["calls"][0]["count"] == 0
            && input["raw_argument"] == 1,
        "startup source event differs"
    );
    let observed = &input["actual_observation"];
    let verification = fs::read(repo.join(observed["verification_source"].as_str().unwrap()))?;
    ensure!(
        hash(&verification) == observed["verification_sha256"].as_str().unwrap(),
        "actual observation verification changed"
    );
    let capture = fs::read(repo.join(observed["capture_source"].as_str().unwrap()))?;
    ensure!(
        hash(&capture) == observed["capture_sha256"].as_str().unwrap(),
        "actual callback capture changed"
    );
    let entry: serde_json::Value = serde_json::from_slice(
        capture
            .split(|b| *b == b'\n')
            .nth(4)
            .context("windowentry missing")?,
    )?;
    ensure!(
        entry["kind"] == "window_entry"
            && entry["cpstate"]["length"] == 0
            && entry["cpstate"]["text_with_nul"] == "00"
            && entry["label"]["text"]["bytes"] == ""
            && observed["summary"]["cpstate_capacity"] == 256
            && observed["summary"]["buffer_identity_preserved"] == true
            && observed["summary"]["callback_return_al"] == 0,
        "observed text/storage profile differs"
    );
    let index = Index::scan(&assets)?;
    let cold_index = Index::scan(&immutable)?;
    ensure!(
        cold_index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
        "cold index has starting saves"
    );
    let mut warm = Session::load(&index, "UserData/Houses/House05.iff", Options::default())?;
    let mut report = serde_json::json!({"scope":"Actual raw1 callback-body on ordinary fresh Goth; observed empty text and explicit256-capacity profile; no exact native prefix/cadence/pixel/tick claim","strict_original_ticks_added":0,"source_event":input,"cuts":[]});
    let result = (|| -> Result<()> {
        let family = warm
            .house
            .household
            .as_ref()
            .context("Goth household absent")?
            .id as i16;
        ensure!(
            family == 5
                && warm
                    .simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .unwrap()
                    .controlled_family
                    == family,
            "Goth application family source differs"
        );
        let metadata = &warm
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .unwrap()
            .family_metadata[&family];
        ensure!(
            metadata.dirty && metadata.friend_count == Some(0),
            "ordinary load does not reach dirty1/count0 source boundary"
        );
        ensure!(
            warm.host.presentation.ui_wait.is_none() && warm.host.presentation.clock.is_none(),
            "UI polling already owned"
        );
        displayed_modal_guard(&warm, family)?;
        report["authored_displayed_modal_guard"] = serde_json::json!({"allowed":true,"answer_or_UIWait":false,"scope":"ordinary pump guard fixture only, not actual native modal/cold/pixel proof"});
        let mut expected = warm.simulation.clone();
        expected
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .resolve_family_friend_count(family, expected.vm.world.autonomy_constants.as_ref())
            .map_err(anyhow::Error::msg)?;
        let expected_simulation = serde_json::to_vec(&expected)?;
        warm.bind_family_window(logical(ApplicationFamily::Family(family)))?;
        let initial_rng = (
            warm.simulation.vm.tick,
            warm.simulation.vm.rng_state,
            warm.simulation.vm.lot_rng_state,
            warm.simulation.vm.crt_rng_state,
        );
        let initial_buffer = warm
            .host
            .presentation
            .family_window
            .as_ref()
            .unwrap()
            .counted_buffer(101)
            .context("bound buffer absent")?;
        warm.begin_family_window_update(input["raw_argument"].as_u64().unwrap() as u32)?;
        ensure!(
            warm.interface_update_pending(),
            "family window did not aggregate UI ownership"
        );
        let gate_before = serde_json::to_vec(&warm.save())?;
        let tick_error = warm
            .step()
            .err()
            .context("pending callback leaked a tick")?;
        let poll_error = warm
            .begin_ui_wait(None)
            .err()
            .context("pending callback leaked UIWait")?;
        let portrait_error = warm
            .begin_ui_portrait()
            .err()
            .context("pending callback leaked portrait work")?;
        ensure!(
            tick_error
                .to_string()
                .contains("interface update must return")
                && poll_error
                    .to_string()
                    .contains("UI wait poll overlaps synchronous work")
                && portrait_error
                    .to_string()
                    .contains("UI portrait overlaps synchronous work"),
            "caller rejected for an unrelated precondition"
        );
        ensure!(
            gate_before == serde_json::to_vec(&warm.save())?,
            "rejected caller mutated retained state"
        );
        let mut cuts = 0usize;
        let mut calls = 0usize;
        let mut temporary_buffers = vec![];
        let started = Instant::now();
        let mut restored = cold(&warm, &cold_index, &out, cuts)?;
        for _ in 0..160 {
            ensure!(
                started.elapsed().as_secs() < 900,
                "bounded callback proof exceeded900s"
            );
            let progress = warm.advance_family_window_update(1)?;
            ensure!(
                progress == restored.advance_family_window_update(1)?,
                "warm/cold progress differs"
            );
            pair(&warm, &restored, &out, cuts)?;
            cuts += 1;
            restored = cold(&warm, &cold_index, &out, cuts)?;
            report["cuts"].as_array_mut().unwrap().push(serde_json::json!({"cut":cuts,"phase":"advance","progress":format!("{progress:?}"),"state_sha256":hash(&serde_json::to_vec(&warm.save())?)}));
            match progress {
                Progress::Complete => break,
                Progress::Pending => (),
                Progress::Service(request) => {
                    if let Request::Window(sims_vm::family_label::WindowRequest::Temporary {
                        source,
                    }) = &request
                    {
                        ensure!(
                            source.buffer == initial_buffer,
                            "actual returned buffer identity changed"
                        );
                        temporary_buffers.push(source.buffer);
                    }

                    if matches!(request, Request::Numeric(NumericRequest::Friends { .. })) {
                        calls += 1;
                    }
                    warm.service_family_window_request(&request)?;
                    restored.service_family_window_request(&request)?;
                    pair(&warm, &restored, &out, cuts)?;
                    let ready = serde_json::to_vec(&warm.save())?;
                    ensure!(
                        warm.service_family_window_request(&request).is_err(),
                        "ready service executed twice"
                    );
                    ensure!(
                        ready == serde_json::to_vec(&warm.save())?,
                        "duplicate service changed state"
                    );
                    cuts += 1;
                    restored = cold(&warm, &cold_index, &out, cuts)?;
                    report["cuts"].as_array_mut().unwrap().push(serde_json::json!({"cut":cuts,"phase":"service-ready","request":request,"state_sha256":hash(&ready)}));
                }
            }
            ensure!(
                initial_rng
                    == (
                        warm.simulation.vm.tick,
                        warm.simulation.vm.rng_state,
                        warm.simulation.vm.lot_rng_state,
                        warm.simulation.vm.crt_rng_state
                    ),
                "callback changed tick/RNG"
            );
            ensure!(
                warm.host.presentation.ui_wait.is_none() && warm.host.presentation.clock.is_none(),
                "callback invoked UI poll"
            );
        }
        ensure!(
            !warm.family_window_pending() && calls == 2,
            "observed changed-text branch did not complete two getters"
        );
        let backend = warm.host.presentation.family_window.as_ref().unwrap();
        ensure!(
            backend.label(102) == Some(b"0".as_slice()) && backend.destruction_order() == [2, 1],
            "text/temp outcome differs"
        );
        ensure!(
            expected_simulation == serde_json::to_vec(&warm.simulation)?,
            "callback altered simulation beyond ordinary family getter"
        );
        ensure!(
            initial_rng
                == (
                    warm.simulation.vm.tick,
                    warm.simulation.vm.rng_state,
                    warm.simulation.vm.lot_rng_state,
                    warm.simulation.vm.crt_rng_state
                ),
            "final callback changed tick/RNG"
        );
        report["complete_cold_cuts"] = cuts.into();
        report["friends_services"] = calls.into();
        ensure!(
            backend.result() == Some(0) && temporary_buffers.len() == 2,
            "actual callback return/temp count differs"
        );
        report["normalized_temporary_buffers"] = serde_json::json!(temporary_buffers);
        report["capacity_source"] = serde_json::json!(
            "actual CPState+23e u16=256; unsupported growth remains pending/error"
        );

        report["final_state_sha256"] = hash(&serde_json::to_vec(&warm.save())?).into();
        Ok(())
    })();
    report["outcome"] = serde_json::json!(if result.is_ok() { "pass" } else { "failed" });
    if let Err(e) = &result {
        report["first_error"] = format!("{e:#}").into();
        fs::write(
            out.join("first-failure-state.json"),
            serde_json::to_vec(&warm.save())?,
        )?;
    }
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    result
}
