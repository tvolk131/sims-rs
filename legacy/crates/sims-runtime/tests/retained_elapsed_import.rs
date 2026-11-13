//! Fresh-baseline control/elapsed source import; no complete Session admission.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::{Store, readset::Readset};
use sims_runtime::original_session::{
    application_parent, clock, completed_application, completed_controls, completed_elapsed,
    transport,
};
use sims_vm::scheduler::clock::{Fields, Frame, Inputs, Request};
use std::{io::Write, path::Path};

#[test]
#[ignore = "private source-pinned 33 completed observations and original caller trace"]
fn fresh_baseline_controls_and_elapsed_resume_are_source_bound() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join(
        "fixtures/private/goth-completed-continuation-20261030-third/results/native-sequence",
    );
    let encoded = std::fs::read(source.join("capture-result.json"))?;
    ensure!(
        sims_fixtures::digest(&encoded)
            == "4139107d1965925bf3ee5f44fa524d5d4b1fae22bfdb617edfffb69d33bcd393",
        "original elapsed source seal differs"
    );
    let seal: Value = serde_json::from_slice(&encoded)?;
    let destination =
        std::env::var_os("SIMS_RETAINED_ELAPSED_IMPORT_OUTPUT").map(std::path::PathBuf::from);
    let output = destination
        .as_ref()
        .map(|p| Store::create(p.with_extension("store")))
        .transpose()?;
    let mut rows = Vec::new();
    let mut baseline = None;
    for index in 0..33 {
        let directory = source.join(format!("completed-{index:02}"));
        let encoded = std::fs::read(directory.join("observation.json.gz"))?;
        let hash = sims_fixtures::digest(&encoded);
        ensure!(
            seal["observations"][index]["sha256"] == hash,
            "elapsed observation identity differs"
        );
        let state: Value = serde_json::from_reader(flate2::read::GzDecoder::new(&encoded[..]))?;
        let raw = std::fs::read(directory.join("acknowledged.mem"))?;
        let memory = Readset::parse(&raw)?;
        let controls = completed_controls::captured::import(&state, &memory)?;
        let fields = completed_elapsed::captured::fields(&state, &memory)?;
        ensure!(
            controls.timing == fields.timing,
            "elapsed/control counter source differs"
        );
        if index == 0 {
            let (transport, clock) = completed_elapsed::captured::resume_baseline(&state, &memory)?;
            ensure!(
                !transport.pending()
                    && transport.timing.is_none()
                    && transport.controls == controls.controls
                    && clock.pending()
                    && clock.at_completed_update()
                    && clock.fields == fields
                    && clock.now == 584466
                    && clock.stopping == 0
                    && clock.captured_code
                        == Some(clock::CapturedCode::Goth89073DormantProtectedPoll20261031),
                "fresh native elapsed caller was not retained exactly"
            );
            let parent_source=std::fs::read(root.join("fixtures/private/goth-elapsed-admission-20261031-first/application-parent-source-second.json"))?;
            let application =
                completed_application::captured::import(&state, &memory, &parent_source)?;
            let parent = application.runtime_parent(&clock)?;
            ensure!(
                parent.awaiting_elapsed()
                    && !parent.at_elapsed_return()
                    && parent.fields() == &application.fields,
                "source parent continuation differs"
            );
            ensure!(
                parent.post_clock_timer() == Some(application.timer)
                    && application.timer
                        == sims_vm::scheduler::clock::Timer {
                            elapsed: 607,
                            interval: 1000,
                            last: 584114,
                            limit: 0
                        },
                "actual application return timer was not retained"
            );
            let parent_reference = output.as_ref().map(|s| s.put_value(&parent)).transpose()?;
            if let Some(store) = &output {
                let cold: application_parent::State =
                    store.get_value(parent_reference.as_ref().unwrap())?;
                ensure!(cold == parent, "durable application parent differs");
            }
            let mut wrong_profile = clock.clone();
            wrong_profile.captured_code =
                Some(clock::CapturedCode::Goth88448DormantProtectedPoll20261029);
            ensure!(
                application.runtime_parent(&wrong_profile).is_err(),
                "fresh parent accepted the older source profile"
            );
            let mut wrong = serde_json::to_value(&clock)?;
            wrong["call"] = Value::Null;
            let wrong: clock::State = serde_json::from_value(wrong)?;
            ensure!(
                application.runtime_parent(&wrong).is_err(),
                "fresh parent accepted a returned elapsed child"
            );
            let mut altered = parent_source.clone();
            altered.push(b' ');
            ensure!(
                completed_application::captured::import(&state, &memory, &altered).is_err(),
                "unqualified parent source version admitted"
            );
            let value = serde_json::to_value((&transport, &clock))?;
            let cold: (transport::State, clock::State) = serde_json::from_value(value.clone())?;
            ensure!(
                cold == (transport.clone(), clock.clone()),
                "elapsed named codec changed owners"
            );
            let reference = output
                .as_ref()
                .map(|s| s.put_value(&(&transport, &clock)))
                .transpose()?;
            if let Some(store) = &output {
                let cold: (transport::State, clock::State) =
                    store.get_value(reference.as_ref().unwrap())?;
                ensure!(
                    cold == (transport, clock.clone()),
                    "durable elapsed codec changed owners"
                );
            }
            let frame: Frame = serde_json::from_value(value[1]["call"]["frame"].clone())?;
            baseline = Some((fields.clone(), frame, clock.now, reference.clone()));
            let mut rejected = 0;
            for fault in 0..8 {
                let mut bad = state.clone();
                match fault {
                    0 => bad["tick"] = json!(89074),
                    1 => bad["readset"]["sha256"] = json!("0".repeat(64)),
                    2 => bad["domains"]["controls"]["simulator"] = json!(0),
                    3 => bad["domains"]["controls"]["raw"] = json!("00"),
                    4 => bad["domains"]["controls"]["requests"] = json!([{}]),
                    5 => bad["barrier"]["raw_g_before"] = json!("00"),
                    6 => bad["identity"]["thread_id"] = json!(241),
                    _ => {
                        bad["physical_baseline"]["archive"]["compressed_sha256"] =
                            json!("0".repeat(64))
                    }
                }
                ensure!(
                    completed_elapsed::captured::resume_baseline(&bad, &memory).is_err(),
                    "altered native elapsed source admitted: fault {fault}"
                );
                rejected += 1;
            }
            ensure!(
                completed_controls::import(&state["domains"]["controls"]).is_err(),
                "old fixed qualification accepted the fresh source"
            );
            rows.push(json!({"index":index,"tick":state["tick"],"source_sha256":hash,
                "controls":controls,"fields":fields,"baseline_owners":reference,"application_parent":parent_reference,
                "application_source":application.source,"rejected_source_mutations":rejected}));
        } else {
            ensure!(
                completed_elapsed::captured::resume_baseline(&state, &memory).is_err(),
                "later state manufactured a fresh initial caller"
            );
            rows.push(json!({"index":index,"tick":state["tick"],"source_sha256":hash,"controls":controls,"fields":fields}));
        }
    }
    let (mut fields, mut frame, now, reference) =
        baseline.context("baseline elapsed owner absent")?;
    let mut input = Inputs {
        now,
        stopping: 0,
        step_word: 0,
    };
    let mut completed = 0;
    let mut queued = 0;
    for _ in 0..128 {
        match frame
            .advance(&mut fields, &mut input)
            .map_err(anyhow::Error::msg)?
        {
            Some(Request::ProcessRequests) => {
                queued += 1;
                frame
                    .acknowledge(Request::ProcessRequests, 0)
                    .map_err(anyhow::Error::msg)?;
            }
            Some(Request::UpdateLot) => {
                completed += 1;
                ensure!(
                    completed <= 8,
                    "elapsed caller manufactured a ninth child without another outer frame"
                );
                let expected: Fields = serde_json::from_value(rows[completed]["fields"].clone())?;
                ensure!(
                    fields == expected,
                    "source-retained elapsed fields differ before child {completed}"
                );
                frame
                    .acknowledge(Request::UpdateLot, 0)
                    .map_err(anyhow::Error::msg)?;
                let cold: Frame = serde_json::from_slice(&serde_json::to_vec(&frame)?)?;
                ensure!(cold == frame, "elapsed frame codec changed continuation");
                frame = cold;
            }
            None => break,
            other => anyhow::bail!("unexpected source-retained elapsed service {other:?}"),
        }
    }
    ensure!(
        frame.complete() && completed == 8 && queued == 9 && input.step_word == 0,
        "source-retained elapsed return cadence differs"
    );
    let result = json!({"schema":1,"kind":"source_bound_fresh_controls_elapsed_import","native_parity_credit":0,
        "rows":rows,"baseline_owners":reference,"completed_child_requests":completed,"empty_request_services":queued,
        "returns_after_tick":89081,"next_child_without_outer_frame":false,"final_frame":frame,
        "qualification":"Actual 33-cut CPState/elapsed owners, exact baseline stack/CPU/code, constructorless counter transfer, stored caller codec, eight existing-frame child requests and return. No lot execution or complete Session/outer caller admission."});
    if let Some(path) = destination {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&serde_json::to_vec_pretty(&result)?)?;
    }
    Ok(())
}
