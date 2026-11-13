//! Structurally compose the retained completed cut. This does not certify the
//! native epoch's next tick, execute a constructor, or substitute a starting save.
use anyhow::{Context, Result, ensure};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sims_content::{Index, saves::House};
use sims_fixtures::Store;
use sims_runtime::{
    original_session::{
        self, Options, RegistryScheduler, Session, View, completed_baseline,
        completed_presentation::HostOwners, completed_viewer,
    },
};
use std::{collections::BTreeMap, fs, path::Path, sync::Arc};
fn value(root: &Path, config: &Value, key: &str) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        root.join(
            config[key]
                .as_str()
                .context("completed Session input absent")?,
        ),
    )?)?)
}
fn stored<T: DeserializeOwned>(root: &Path, config: &Value, key: &str, field: &str) -> Result<T> {
    let dir = root.join(
        config[key]
            .as_str()
            .context("completed Session store absent")?,
    );
    let refs: Value = serde_json::from_slice(&fs::read(dir.join("refs.json"))?)?;
    Store::open(dir.join("store"))?.get_value(&serde_json::from_value(refs[field].clone())?)
}
pub(crate) fn retained_program(root:&Path,config:&Value,owned:&sims_content::owned_resources::Neighborhood,immutable:&Index)->Result<(sims_vm::Program,BTreeMap<String,String>,String,Value)>{
    original_session::completed_replay::ProgramInputs::read(config,|locator|Ok(serde_json::from_slice(&fs::read(root.join(locator))?)?))?.link(owned,immutable)
}
fn diagnostic_step(session:&mut Session,trace:&mut Vec<sims_vm::Trace>,observer:Option<&mut dyn FnMut(&Session)->Result<()>>)->Result<original_session::Progress>{
 original_session::completed_replay::advance(session,trace,observer)
}
fn write_diagnostic_tick(
    session: &mut Session,
    store: &Store,
    output: &Path,
    checkpoint_dependents: bool,
    immutable: &Index,
) -> Result<()> {
    let tick_before = session.simulation.vm.tick;
    let rng_before = (
        session.simulation.vm.rng_state,
        session.simulation.vm.lot_rng_state,
        session.simulation.vm.crt_rng_state,
    );
    let program_before = session.program.fingerprint();
    let vm_before = postcard::to_allocvec(&session.simulation.vm)?;
    let mut trace = vec![];
    let mut cuts = vec![];
    let mut observed = std::collections::BTreeSet::new();
    let result = if checkpoint_dependents {
        diagnostic_step(
            session,
            &mut trace,
            Some(&mut |s| {
                let states = serde_json::to_value(&s.host.snapshot().dependent_jobs)?;
                let key = states.as_object().unwrap().values().next().map(|j| {
                    (
                        j["stage"].as_u64().unwrap(),
                        j["started"].as_bool().unwrap(),
                    )
                });
                if observed.insert(key) {
                    let saved = s.save_managed()?;
                    let reference = store.put_value(&saved)?;
                    cuts.push((reference, s.program.clone(), key));
                }
                Ok(())
            }),
        )
    } else {
        diagnostic_step(session, &mut trace, None)
    };
    fs::write(
        output.join("rust-trace.json"),
        serde_json::to_vec_pretty(&trace)?,
    )?;
    let outcome = match result {
        Ok(progress) => format!("{progress:?}"),
        Err(error) => format!("{error:#}"),
    };
    let vm_unchanged = postcard::to_allocvec(&session.simulation.vm)? == vm_before;
    let end = store.put_value(&session.save())?;
    let managed_end = store.put_value(&session.save_managed()?)?;
    fs::write(
        output.join("managed-endpoint.json"),
        serde_json::to_vec_pretty(
            &json!({"session":managed_end,"tick":session.simulation.vm.tick,"native_parity_ticks":0,
            "complete_native_Session_admitted":false}),
        )?,
    )?;
    if checkpoint_dependents {
        let mut receipts = vec![];
        for (reference, program, cut) in cuts {
            let saved: original_session::Saved = store.get_value(&reference)?;
            let mut resumed =
                Session::restore_completed_owned_with_program(immutable, saved, program)?;
            let mut suffix = vec![];
            let progress = diagnostic_step(&mut resumed, &mut suffix, None)?;
            let restored_end = store.put_value(&resumed.save())?;
            let restored_managed = store.put_value(&resumed.save_managed()?)?;
            ensure!(
                format!("{progress:?}") == "Tick" && restored_end == end,
                "dependent cold resume diverged at {cut:?}"
            );
            ensure!(
                restored_managed == managed_end,
                "dependent managed endpoint differs at {cut:?}"
            );
            ensure!(
                trace.ends_with(&suffix),
                "dependent resumed instruction order differs"
            );
            receipts.push(
                json!({"cut":cut,"checkpoint":reference,"endpoint":restored_end,
                "managed_endpoint":restored_managed,"trace_suffix_events":suffix.len()}),
            );
        }
        fs::write(
            output.join("dependent-cold-resume.json"),
            serde_json::to_vec_pretty(
                &json!({"cuts":receipts,"endpoint":end,"native_parity_ticks":0}),
            )?,
        )?;
    }

    fs::write(
        output.join("diagnostic-tick.json"),
        serde_json::to_vec_pretty(&json!({
            "diagnostic_only":true,"complete_native_Session_admitted":false,"native_parity_ticks":0,
            "tick_before":tick_before,"tick_after":session.simulation.vm.tick,"outcome":outcome,
            "rng_before":rng_before,"rng_after":(session.simulation.vm.rng_state,session.simulation.vm.lot_rng_state,session.simulation.vm.crt_rng_state),
            "vm_unchanged":vm_unchanged,"program_before":program_before,"program_after":session.program.fingerprint(),
            "interim_state":end,"reference_comparison_executed":false,
            "scope":"Unadmitted Rust-only diagnostic. Unknown frontend owners and original ordered successor remain unresolved. Interim state is not a qualified resumable reference."
        }))?,
    )?;
    Ok(())
}
fn cold_restore(
    root: &Path,
    config: &Value,
    immutable: &Index,
    checkpoint: &Path,
    output: &Path,
    diagnostic_tick: bool,
) -> Result<()> {
    let refs: Value = serde_json::from_slice(&fs::read(checkpoint.join("refs.json"))?)?;
    let saved: original_session::Saved = Store::open(checkpoint.join("store"))?
        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
    let owned = &saved
        .managed
        .as_ref()
        .context("completed owned content absent")?
        .owned;
    let (mut program, aliases, _, _) = retained_program(root, config, owned, immutable)?;
    if saved.headline_catalog {
        sims_content::vm::load_headline_catalog(&owned.mount(immutable)?, &mut program)?;
    }
    ensure!(
        saved.view.resource_aliases.as_ref() == Some(&aliases),
        "cold provider bindings differ"
    );
    let before = postcard::to_allocvec(&saved)?;
    let mut semantic_change = program.clone();
    let words = semantic_change
        .modules
        .values_mut()
        .flat_map(|m| m.object_definition_words.values_mut())
        .find(|v| !v.is_empty())
        .context("retained definitions absent")?;
    words[0] ^= 1;
    let semantic_error =
        Session::restore_completed_owned_with_program(immutable, saved.clone(), semantic_change)
            .err()
            .context("changed semantic Program unexpectedly admitted")?
            .to_string();
    ensure!(
        semantic_error == "room snapshot version/content mismatch",
        "changed Program did not reach fingerprint validation: {semantic_error}"
    );
    let mut wrong = saved.clone();
    wrong
        .view
        .resource_aliases
        .as_mut()
        .unwrap()
        .values_mut()
        .next()
        .context("provider alias absent")?
        .push_str(".missing");
    let alias_error =
        Session::restore_completed_owned_with_program(immutable, wrong, program.clone())
            .err()
            .context("changed provider alias unexpectedly admitted")?
            .to_string();
    let mut clock_controls = json!({});
    if saved
        .view
        .clock
        .as_ref()
        .is_some_and(|c| c.captured_code.is_some())
    {
        let mut wrong = saved.clone();
        wrong.view.clock.as_mut().unwrap().fields.poll_messages = 1;
        let error =
            Session::restore_completed_owned_with_program(immutable, wrong, program.clone())
                .err()
                .context("captured protected poll unexpectedly admitted")?
                .to_string();
        ensure!(
            error.contains("captured protected elapsed input-poll branch"),
            "protected poll failed for an unrelated reason: {error}"
        );
        clock_controls["protected_poll_rejected"] = json!(error);
        let mut wrong = saved.clone();
        let timing = wrong.view.clock.as_ref().unwrap().fields.timing;
        wrong
            .view
            .transport
            .as_mut()
            .context("captured transport missing")?
            .timing = Some(timing);
        let error =
            Session::restore_completed_owned_with_program(immutable, wrong, program.clone())
                .err()
                .context("duplicated elapsed counter owner unexpectedly admitted")?
                .to_string();
        ensure!(
            error.contains("elapsed counters have two owners"),
            "duplicated timer failed elsewhere: {error}"
        );
        clock_controls["duplicate_timing_owner_rejected"] = json!(error);
    }
    let mut session = Session::restore_completed_owned_with_program(immutable, saved, program)?;
    let runtime_contract = original_session::completed_epoch::session::evaluate(&session)?;
    ensure!(
        runtime_contract["difference_count"] == 0,
        "cold Session ownership contract differs"
    );
    let mut parent_controls = json!({});
    if session.host.presentation.application_parent.is_some() {
        let protected = postcard::to_allocvec(&session.save_managed()?)?;
        let error = session
            .step()
            .err()
            .context("direct tick bypassed the retained application")?
            .to_string();
        ensure!(
            error.contains("retained application parent owns"),
            "direct-tick parent guard failed elsewhere: {error}"
        );
        parent_controls["direct_tick_rejected"] = json!(error);
        let error = session
            .begin_clock_frame()
            .err()
            .context("fresh frame bypassed retained application")?
            .to_string();
        ensure!(
            error.contains("retained application caller must return"),
            "fresh-clock parent guard failed elsewhere: {error}"
        );
        parent_controls["fresh_clock_rejected"] = json!(error);
        ensure!(
            postcard::to_allocvec(&session.save_managed()?)? == protected,
            "rejected parent bypass changed state or RNG"
        );
        parent_controls["rejections_preserved_all_saved_state"] = json!(true);
    }
    let after = session.save_managed()?;
    ensure!(
        postcard::to_allocvec(&after)? == before,
        "cold complete runtime checkpoint differs"
    );
    fs::create_dir(output)?;
    fs::write(
        output.join("session-root-contract.json"),
        serde_json::to_vec_pretty(&runtime_contract)?,
    )?;
    let store = Store::create(output.join("store"))?;
    let blob = store.put_value(&after)?;
    fs::write(
        output.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"session":blob}))?,
    )?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":1,"complete_structural_Session_validated":true,
            "cold_runtime_restored":true,"checkpoint_identical":true,
            "program_independently_rebuilt_from_frozen_descriptors":true,
            "starting_save_files_opened":false,"executed_ticks":0,"native_parity_ticks":0,
            "complete_native_Session_admitted":false,
            "negative_controls":{"semantic_program_change":semantic_error,"changed_source_alias":alias_error,"elapsed":clock_controls,"application_parent":parent_controls}
        }))?,
    )?;
    if diagnostic_tick {
        write_diagnostic_tick(
            &mut session,
            &store,
            output,
            config["checkpoint_dependents"] == true,
            &immutable,
        )?;
    }
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 4
            || (args.len() == 5 && args[4] == "--diagnostic-tick")
            || (args.len() == 6
                && matches!(
                    args[4].as_str(),
                    "--restore-from" | "--diagnostic-tick-from"
                )),
        "usage: completed_session_join SOURCE_ROOT CONFIG IMMUTABLE_ROOT NEW_DIR [--restore-from|--diagnostic-tick-from CHECKPOINT_DIR]"
    );
    let root = Path::new(&args[0]);
    let config: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let mut immutable = Index::scan(Path::new(&args[2]))?;
    immutable.resources.retain(|r| {
        !r.locator
            .split('/')
            .any(|c| c.to_ascii_lowercase().starts_with("userdata"))
    });
    if args.len() == 6 {
        return cold_restore(
            root,
            &config,
            &immutable,
            Path::new(&args[5]),
            Path::new(&args[3]),
            args[4] == "--diagnostic-tick-from",
        );
    }
    let mut host: HostOwners = stored(root, &config, "host", "host")?;
    let house: House = stored(root, &config, "metadata", "house_metadata")?;
    let viewer: completed_viewer::Imported = stored(root, &config, "viewer", "viewer")?;
    let (program, aliases, unversioned_program, closure) =
        retained_program(root, &config, &host.owned, &immutable)?;
    ensure!(
        unversioned_program == host.simulation.program_hash,
        "retained host/program before version binding differ"
    );
    if config["simulation_owners"].is_string() {
        // Reuse the complete original clock/configuration owner validator,
        // including its exact hardware/boundary/source checks. Binding this
        // process input advances no animation, frame, instruction or RNG.
        let source = original_session::completed_simulation::import(
            &value(root, &config, "simulation_owners")?,
            &sims_vm::Snapshot::capture(&program, &host.simulation.state.vm),
        )?;
        host.simulation
            .state
            .native_character_settings
            .as_mut()
            .context("retained native character settings absent")?
            .terrain_scale = source.character.terrain_scale;
        if config["dependent_slot_input"].is_string() {
            let imported = original_session::completed_slot_anchors::import(
                &value(root, &config, "dependent_slot_input")?,
                &host.simulation.state.vm.world,
            )?;
            for (id, names) in imported.anchors {
                let slots = host
                    .simulation
                    .state
                    .vm
                    .world
                    .objects
                    .get_mut(&id)
                    .unwrap()
                    .native_slot_geometry
                    .as_mut()
                    .context("retained slot geometry absent")?;
                ensure!(
                    slots.len() == names.len(),
                    "slot anchor/geometry coverage differs"
                );
                for (slot, name) in slots.iter_mut().zip(names) {
                    slot.anchor = Some(name);
                }
            }
            host.assets
                .ordinary
                .bind_observed_process(source.fp_control, &imported.marker_values)?;
        }
        for resident in host.simulation.state.characters.values_mut() {
            resident
                .character
                .skin
                .animator
                .bind_fp_control(source.fp_control)
                .map_err(anyhow::Error::msg)?;
        }
    }
    if config["routing_input"].is_string() {
        let banks = original_session::completed_routing::import(
            &value(root, &config, "routing_input")?,
            &host.simulation.state.vm.world,
        )?;
        for (id, bank) in banks {
            host.simulation
                .state
                .vm
                .world
                .objects
                .get_mut(&id)
                .context("retained routing object lost")?
                .routing_slots = Some(bank);
        }
    }
    if config["person_labels_input"].is_string() {
        original_session::completed_labels::attach_before(
            &mut host.simulation.state,
            &value(root, &config, "person_labels_input")?,
            &value(root, &config, "reentry")?,
        )?;
    }
    let before_vm = postcard::to_allocvec(&host.simulation.state.vm)?;
    let index = host.owned.mount(&immutable)?;
    // Version receipts change the enclosing Program fingerprint, never VM state.
    host.simulation.program_hash = program.fingerprint();
    ensure!(
        postcard::to_allocvec(&host.simulation.state.vm)? == before_vm,
        "source binding changed VM state"
    );
    let lot = host
        .simulation
        .state
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("current live lot absent")?;
    let floors: Vec<_> = lot.floors.iter().map(|f| f.source.clone()).collect();
    let valuation = sims_content::house_valuation::load(
        &index,
        &house,
        lot.active_size
            .context("native active world extent absent")?,
        lot.size,
        &floors,
        &lot.pool,
    )?;
    let (presenter, notifications, camera) = viewer.runtime_owners(&closure)?;
    let tools = if config["tool_input"].is_string() {
        Some(original_session::completed_tools::import(
            &value(root, &config, "tool_input")?,
            &closure,
        )?)
    } else {
        None
    };
    let controls = if config["control_input"].is_string() {
        Some(original_session::completed_controls::import(&value(
            root,
            &config,
            "control_input",
        )?)?)
    } else {
        None
    };
    let (transport, clock) = if config["retain_elapsed"] == true {
        let (transport, clock) = original_session::completed_elapsed::resume(
            &value(root, &config, "control_input")?,
            &closure,
            host.simulation.state.vm.tick,
        )?;
        (Some(transport), Some(clock))
    } else {
        (controls.clone().map(|c| c.into_transport()), None)
    };
    let native_lot = host
        .simulation
        .state
        .native_lot
        .as_ref()
        .context("native lot owner absent")?;
    let retained_tool_inputs = if config["tool_threads_input"].is_string() {
        Some(original_session::completed_tools::import_retained_inputs(
            &value(root, &config, "tool_input")?,
            &closure,
            &value(root, &config, "tool_threads_input")?,
            &host.simulation.state.vm.world,
        )?)
    } else {
        None
    };
    let retained_tool_inputs = if config["tool_payload_input"].is_string() {
        Some(original_session::completed_tool_payloads::import(
            &value(root, &config, "tool_payload_input")?,
            &value(root, &config, "tool_input")?,
            retained_tool_inputs
                .as_ref()
                .context("tool bases require retained native input owners")?,
        )?)
    } else {
        retained_tool_inputs
    };
    let retained_tool_inputs = if config["editor_catalog_input"].is_string() {
        let texts: Vec<_> = immutable
            .resources
            .iter()
            .filter(|r| r.name.eq_ignore_ascii_case("UIText.iff"))
            .collect();
        ensure!(
            texts.len() == 1,
            "qualified stock UIText source absent/ambiguous"
        );
        Some(original_session::completed_stock::import(
            &value(root, &config, "tool_payload_input")?,
            &value(root, &config, "editor_catalog_input")?,
            &value(root, &config, "tool_input")?,
            retained_tool_inputs
                .as_ref()
                .context("stock settings require qualified tool bases")?,
            &texts[0].read()?,
        )?)
    } else {
        retained_tool_inputs
    };
    let application_parent = if config["application_input"].is_string() {
        Some(
            original_session::completed_application::import(&value(
                root,
                &config,
                "application_input",
            )?)?
            .runtime_parent(
                clock
                    .as_ref()
                    .context("application parent elapsed child absent")?,
                host.simulation.state.vm.tick,
            )?,
        )
    } else {
        None
    };
    let frontend = if config["frontend_input"].is_string() {
        Some(original_session::completed_frontend::import(
            &value(root, &config, "frontend_input")?,
            &value(root, &config, "tool_input")?,
            &value(root, &config, "application_input")?,
        )?)
    } else {
        None
    };
    let options = Options {
        initial_process_profile: None,
        application_context: None,
        scheduler: RegistryScheduler,
        native_load_completion: false,
        native_postload: false,
        routing: native_lot.routing_policy,
        crt_seed: host
            .simulation
            .state
            .vm
            .crt_rng_state
            .context("current CRT owner absent")?,
        defer_indicators: false,
    };
    // These None values are unpublished Rust adapter continuations or unbound
    // frontend scopes. They are not evidence that every native UI/event queue
    // was empty. This output remains unadmitted for gameplay parity.
    ensure!(
        viewer.object_focus == 0,
        "non-null native focus requires a qualified scheduler lifetime mapping"
    );
    let view = View {
        resource_aliases: Some(aliases.clone()),
        character_properties: Some(host.presentation.properties),
        character_visibility: Some(host.presentation.visibility),
        person_draw: None,
        neighborhood_input: None,
        valuation_catalog: Some(valuation),
        // Preserve the observed scalar independently of an as-yet unbound
        // CPState publication service. This does not run a publication.
        person_selection_override: controls.as_ref().map(|_| viewer.person_selection_override),
        publication: None,
        tool_selection: None,
        move_tool: None,
        move_button: None,
        purchase_cursor: None,
        current_tool: tools.as_ref().map(|t| t.current_tool.clone()),
        retained_tool_inputs,
        object_focus: None, // Observed native NULL, not an absent observation.
        object_budget: None,
        picker_target: tools.as_ref().and_then(|t| t.picker_target),
        picker_activation_word: tools.as_ref().map(|t| t.picker_activation_word),
        ground_input: None,
        object_input: None,
        live_menu: None,
        ui_portrait_tick: None,
        control_panel: None,
        interface_update: None,
        family_window: None,
        transport,
        clock,
        application_parent,
        ui_wait: None,
        camera_ui: frontend.as_ref().map(|f| f.camera),
        frontend_context: frontend.map(|f| f.context),
        camera_command: None,
        loading: None,
        postload: None,
        awaiting_house_open: false,
        selected: native_lot.selected_person,
        viewport: host.presentation.viewport,
        camera: Some(camera),
        visible: None,
        presenter: Some(presenter),
        notifications: Some(notifications),
        property_refresh: None,
    };
    let expected = postcard::to_allocvec(&host.simulation)?;
    let mut session = Session::from_completed_owned(completed_baseline::Input {
        immutable: Arc::new(immutable.clone()),
        mutable: host.owned,
        house,
        program,
        simulation: host.simulation,
        assets: host.assets,
        view,
        options,
    })?;
    ensure!(
        postcard::to_allocvec(&sims_sim::Snapshot::capture(
            &session.program,
            &session.simulation
        ))? == expected,
        "Session composition changed supplied simulation"
    );
    let saved = session.save_managed()?;
    let output = Path::new(&args[3]);
    fs::create_dir(output)?;
    let store = Store::create(output.join("store"))?;
    let blob = store.put_value(&saved)?;
    let cold: original_session::Saved = store.get_value(&blob)?;
    ensure!(
        postcard::to_allocvec(&saved)? == postcard::to_allocvec(&cold)?,
        "completed Session checkpoint codec changed state"
    );
    fs::write(
        output.join("refs.json"),
        serde_json::to_vec_pretty(&json!({"session":blob}))?,
    )?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&json!({
            "schema":1,"tick":88448,"complete_structural_Session_validated":true,
            "complete_native_Session_admitted":false,"native_parity_ticks":0,"executed_ticks":0,
            "managed_schema":saved.managed.as_ref().unwrap().schema,
            "provider_aliases":aliases,"unversioned_program":unversioned_program,
            "versioned_program":session.program.fingerprint(),"vm_state_unchanged":true,
            "simulation_unchanged_except_program_version_receipts":true,
            "native_person_label_cache_imported":config["person_labels_input"].is_string(),
            "checkpoint_codec_identical":true,"cold_runtime_restored":false,
        "observed_selection_override":viewer.person_selection_override,
        "known_unbound_scopes":["CPState/selection publication and native input/tool/window owners","audio/retirement presentation side effects","ordered native successor"],
            "starting_save_files_opened":false,
        }))?,
    )?;
    if args.len() == 5 {
        write_diagnostic_tick(
            &mut session,
            &store,
            output,
            config["checkpoint_dependents"] == true,
            &immutable,
        )?;
    }
    Ok(())
}
