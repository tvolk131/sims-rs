//! Retained viewer/service input comparison, separately from full admission.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved, completed_animations, completed_controls, completed_elapsed, completed_presentation,
    completed_viewer,
};
use std::{fs, path::Path};

fn same<T: Serialize>(name: &str, native: &T, rust: &T, rows: &mut Vec<Value>) -> Result<()> {
    let a = postcard::to_allocvec(native)?;
    let b = postcard::to_allocvec(rust)?;
    let mut row = json!({"field":name,"matched":a==b,
        "native_sha256":sims_fixtures::digest(&a),"rust_sha256":sims_fixtures::digest(&b),
        "first_byte_offset":a.iter().zip(&b).position(|(x,y)|x!=y)});
    if a != b {
        row["native"] = serde_json::to_value(native)?;
        row["rust"] = serde_json::to_value(rust)?;
    }
    rows.push(row);
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_frontend STORE REFS_JSON REF_FIELD NATIVE_UNCHANGED_INPUT_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    let viewer = completed_viewer::import(&source["viewer"], &source["lot"])?;
    let inputs = completed_presentation::import(&source["presentation"], &source["resident"])?;
    let view = &saved.view;
    let presenter = view
        .presenter
        .as_ref()
        .context("retained presenter absent")?;
    let notifications = view
        .notifications
        .as_ref()
        .context("retained notifications absent")?;
    // The caller separately verifies the original before/after payloads are
    // unchanged. Never relabel a before input as a native after observation.
    ensure!(
        source["proof"]["before_tick"] == 88448
            && source["proof"]["after_tick"] == 88449
            && source["proof"]["captured_payloads_equal"] == true,
        "native unchanged-input proof absent"
    );
    ensure!(
        [88448, 88449].contains(&saved.simulation.state.vm.tick),
        "unsupported boundary"
    );
    let mut rows = vec![];
    same("viewer/state", &viewer.view, &presenter.view, &mut rows)?;
    same(
        "viewer/camera",
        &Some(viewer.camera.clone()),
        &presenter.camera,
        &mut rows,
    )?;
    same(
        "viewer/inputs",
        &viewer.inputs,
        &presenter.inputs,
        &mut rows,
    )?;
    same(
        "commander/registry",
        &viewer.registry,
        &notifications.dispatch.registry,
        &mut rows,
    )?;
    same(
        "commander/fields",
        &viewer.fields,
        &notifications.dispatch.fields,
        &mut rows,
    )?;
    same(
        "presentation/viewport",
        &inputs.viewport,
        &view.viewport,
        &mut rows,
    )?;
    same(
        "presentation/properties",
        &Some(inputs.properties),
        &view.character_properties,
        &mut rows,
    )?;
    same(
        "presentation/visibility",
        &Some(inputs.visibility),
        &view.character_visibility,
        &mut rows,
    )?;
    same(
        "presentation/selection_override",
        &Some(viewer.person_selection_override),
        &view.person_selection_override,
        &mut rows,
    )?;
    ensure!(
        viewer.object_focus == 0 && inputs.native_focus == 0,
        "non-null native focus needs an identity mapping"
    );
    same(
        "presentation/observed_null_focus",
        &None::<u32>,
        &view.object_focus,
        &mut rows,
    )?;
    let control_cut = if saved.simulation.state.vm.tick == 88448 {
        "before"
    } else {
        "after"
    };
    let controls = completed_controls::import(&source["controls"][control_cut])?;
    let transport = view
        .transport
        .as_ref()
        .context("retained transport absent")?;
    same(
        "CPState/controls",
        &controls.controls,
        &transport.controls,
        &mut rows,
    )?;
    same(
        "transport/timing",
        &Some(controls.timing),
        &transport
            .timing
            .or_else(|| view.clock.as_ref().map(|s| s.fields.timing)),
        &mut rows,
    )?;
    if source["compare_elapsed"] == true {
        let clock = view
            .clock
            .as_ref()
            .context("retained elapsed caller absent")?;
        same(
            "elapsed/all_scalar_owners",
            &completed_elapsed::fields(&source["controls"][control_cut])?,
            &clock.fields,
            &mut rows,
        )?;
        let input = &source["controls"][control_cut];
        let now_bytes = completed_animations::bytes(
            input["now_raw"]
                .as_str()
                .context("clock now bytes absent")?,
        )?;
        let stopping = completed_animations::bytes(
            input["stopping_raw"]
                .as_str()
                .context("clock stopping byte absent")?,
        )?;
        ensure!(
            now_bytes.len() == 4 && stopping.len() == 1,
            "clock input extents differ"
        );
        let expected = (
            u32::from_le_bytes(now_bytes.try_into().unwrap()),
            stopping[0],
        );
        same(
            "elapsed/application_inputs",
            &expected,
            &(clock.now, clock.stopping),
            &mut rows,
        )?;
        same("elapsed/code_profile",&Some(sims_runtime::original_session::clock::CapturedCode::Goth88448DormantProtectedPoll20261029),
            &clock.captured_code,&mut rows)?;
        ensure!(
            clock.requests.is_empty() && clock.at_completed_update(),
            "retained elapsed caller boundary/requests differ"
        );
    }
    let differences = rows.iter().filter(|r| r["matched"] != true).count();
    let out = Path::new(&args[4]);
    fs::create_dir(out)?;
    let report = json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"retained viewer, Commander registry/fields and property/visibility inputs; this does not authenticate unbound input/publication services or all VM owners",
        "source_proof":source["proof"]});
    fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
    println!(
        "{}",
        json!({"comparisons":rows.len(),"differences":differences,"native_parity_ticks":0})
    );
    ensure!(differences == 0, "retained frontend components differ");
    Ok(())
}
