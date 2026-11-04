//! Execute a retained startup prefix; stop at resize_return or clear_return.
//! House is an unused argument at this boundary, not a qualified saved reader.
mod retained_support;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sims_content::{Index, catalog::Controller, house_startup::Incoming};
use sims_fixtures::{BlobRef, Pin, Store};
use sims_vm::{Program, Snapshot, Vm};
use std::{collections::BTreeSet, fs, path::PathBuf};

const ENTRY: &str = "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: u32,
    vm_input: PathBuf,
    program_pins: PathBuf,
    components_store: PathBuf,
    incoming: BlobRef,
    graphics: BlobRef,
    controller_plan: PathBuf,
    controller_plan_sha256: String,
    #[serde(default)]
    career_source_sha256: Option<String>,
    #[serde(default)]
    through_clear: bool,
    unused_house_root: PathBuf,
    unused_house: String,
}
#[derive(Deserialize)]
struct Planned {
    ordinal: usize,
    guid: u32,
    kind: u16,
    global_flag: u16,
    source: String,
    source_sha256: String,
    object: u16,
    label_bytes: Vec<u8>,
    normalized_words: Vec<u16>,
}
#[derive(Deserialize)]
struct Plan {
    schema: u32,
    entry: serde_json::Value,
    controllers: Vec<Planned>,
    expected_return: serde_json::Value,
    all_controller_backings_qualified_readonly: bool,
}

struct PrefixAssets<'a> {
    registry: sims_content::graphic_loading::Registry,
    prepared: bool,
    allow_teardown: bool,
    index: &'a Index,
    store: &'a Store,
    // This adapter-only pool is never consumed: the host requires the retained
    // branch before calling advance_retirement, which returns before pool use.
    unused_pool: Option<sims_content::character_geometry::MeshPool>,
    models: std::collections::BTreeMap<i16, u64>,
    notifications: Vec<serde_json::Value>,
    service_cuts: Vec<BlobRef>,
    service_cut_checks: Vec<serde_json::Value>,
}
impl PrefixAssets<'_> {
    fn checkpoint(&mut self, simulation: &sims_sim::Simulation, program: &Program) -> Result<()> {
        let snapshot = sims_sim::Snapshot::capture(program, simulation);
        let saved = self.registry.snapshot();
        let blob = self.store.put_value(&(
            snapshot.clone(),
            saved.clone(),
            self.models.clone(),
            self.notifications.clone(),
        ))?;
        self.service_cuts.push(blob.clone());
        let (cold, graphics, models, notifications): (
            sims_sim::Snapshot,
            sims_content::graphic_loading::Saved,
            std::collections::BTreeMap<i16, u64>,
            Vec<serde_json::Value>,
        ) = self.store.get_value(&blob)?;
        ensure!(
            cold == snapshot && models == self.models && notifications == self.notifications,
            "teardown research roundtrip differs"
        );
        let restored = match cold.restore(program) {
            Ok(restored) => {
                self.service_cut_checks
                    .push(serde_json::json!({"blob":blob,"resumable_simulation":true}));
                restored
            }
            Err(error) => {
                //445f84's outer loop retains its pointer until the destructor
                //returns. That loop is on the Rust caller stack, not in this
                //Snapshot. Preserve the research state, never claim a player
                //save or weaken the normal complete-Simulation validator.
                let target = simulation
                    .synchronous_removal
                    .as_ref()
                    .context("rejected checkpoint has no frozen removal owner")?
                    .target();
                let native = simulation
                    .native_lot
                    .as_ref()
                    .context("rejected checkpoint has no native lot")?;
                let absent: Vec<_> = [native.indicators.primary, native.indicators.secondary]
                    .into_iter()
                    .flatten()
                    .filter(|id| !simulation.vm.world.objects.contains_key(id))
                    .collect();
                ensure!(
                    error == "native runtime indicator is not registered"
                        && self.allow_teardown
                        && simulation.vm.world.registry_ui_disabled
                        && absent == [target]
                        && simulation
                            .deletion
                            .last()
                            .is_some_and(|d| d.target == target && d.storage_unlinked)
                        && simulation
                            .vm
                            .world
                            .scheduler_storage
                            .as_ref()
                            .is_some_and(|s| s.handle(target).is_none()),
                    "unexpected nonresumable research checkpoint: {error}"
                );
                self.service_cut_checks.push(
                    serde_json::json!({"blob":blob,"resumable_simulation":false,
                    "reason":error,"frozen_indicator":target,"full_research_roundtrip_equal":true,
                    "outer_clear_loop_not_serialized":true}),
                );
                snapshot.state.clone()
            }
        };
        let restored_graphics =
            sims_content::graphic_loading::Registry::restore(self.index, graphics)?;
        restored_graphics.validate_with_retired(&restored.vm, &restored.retired_graphics)?;
        ensure!(
            sims_sim::Snapshot::capture(program, &restored) == snapshot
                && restored_graphics.snapshot() == saved
                && models == self.models
                && notifications == self.notifications,
            "teardown service cold restore differs"
        );
        Ok(())
    }
}
impl sims_content::house_startup::AssetHost for PrefixAssets<'_> {
    fn prepare_incoming(&mut self, vm: &Vm, program: &Program) -> Result<BTreeSet<i16>> {
        ensure!(!self.prepared, "duplicate incoming preparation");
        self.registry.validate(vm)?;
        ensure!(
            self.registry.pending_objects().next().is_none()
                && !self.registry.updates_pending()
                && self.registry.retirements.is_empty()
                && self
                    .registry
                    .retained
                    .objects()
                    .all(|id| !self.registry.retained.started(id)),
            "incoming graphics already have a suspended service"
        );
        let ready = self.registry.presenter_readiness(vm, program)?;
        ensure!(
            ready == [(1, 1), (2, 1)].into_iter().collect(),
            "readiness differs"
        );
        self.prepared = true;
        Ok(ready.into_keys().collect())
    }
    fn service_teardown(
        &mut self,
        simulation: &mut sims_sim::Simulation,
        program: &Program,
    ) -> Result<()> {
        ensure!(
            self.allow_teardown,
            "resize-only host cannot service teardown"
        );
        self.checkpoint(simulation, program)?;
        self.registry
            .validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
        if let Some(id) = simulation
            .deletion
            .last()
            .map(|d| d.target)
            .filter(|id| simulation.retired_graphics.contains_key(id))
        {
            simulation
                .validate_graphic_retirement_owner(id)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                self.registry.retained.contains(id) && self.registry.graphics.is_empty(),
                "only source-bound retained graphics are supported"
            );
            let retired = &simulation.retired_graphics[&id];
            if !retired.asset_service_started {
                self.registry.begin_retirement(id, false)?;
                simulation
                    .begin_graphic_retirement(id)
                    .map_err(anyhow::Error::msg)?;
            } else {
                let models = &mut self.models;
                let notifications = &mut self.notifications;
                let done = self.registry.advance_retirement(
                    self.index,
                    id,
                    self.unused_pool
                        .as_mut()
                        .context("retained API adapter pool absent")?,
                    retired.tick as i32,
                    sims_sim::pose::animator::Settings {
                        fade_enabled: false,
                        fade_duration: 0,
                        sampling_config: 0,
                    },
                    false,
                    |request| match request {
                        sims_sim::pose::graphic_destruction::Request::DestroyModel(model) => {
                            ensure!(
                                models.remove(&id) == Some(model),
                                "model removal lost its owner"
                            );
                            notifications
                                .push(serde_json::json!({"object":id,"removed_model":model}));
                            Ok(())
                        }
                        other => bail!("unexpected retained presentation request: {other:?}"),
                    },
                )?;
                if done {
                    simulation
                        .complete_graphic_retirement(id)
                        .map_err(anyhow::Error::msg)?;
                }
            }
        } else if let Some((id, _)) = simulation.pending_removal_graphic() {
            sims_content::graphic_loading::retained::refresh::service(
                &mut self.registry,
                simulation,
                program,
                id,
            )?;
        } else {
            bail!("teardown has no owning pending graphic service");
        }
        ensure!(
            self.unused_pool.as_ref() == Some(&Default::default()),
            "retained branch consumed the adapter-only pool"
        );
        self.checkpoint(simulation, program)?;
        Ok(())
    }
    fn finish_teardown(&mut self, vm: &Vm) -> Result<()> {
        ensure!(
            self.allow_teardown && self.prepared,
            "resize-only host cannot finish teardown"
        );
        self.registry.validate(vm)?;
        ensure!(
            self.registry.owned_objects().next().is_none()
                && self.registry.pending_objects().next().is_none()
                && !self.registry.updates_pending()
                && self.registry.retirements.is_empty()
                && self.models.is_empty()
                && self.notifications.len() == 2,
            "clear finish retained pending asset/model effects"
        );
        Ok(())
    }
}

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: retained_resize_prefix REQUEST.json NEW_OUTPUT"
    );
    let request_raw = fs::read(&args[0])?;
    let request: Request = serde_json::from_slice(&request_raw)?;
    ensure!(request.schema == 1, "request schema");
    let output = PathBuf::from(&args[1]);
    fs::create_dir(&output).context("output must be new")?;
    let store = Store::create(output.join("store"))?;
    let vm_input_raw = fs::read(&request.vm_input)?;
    let vm_input: serde_json::Value = serde_json::from_slice(&vm_input_raw)?;
    ensure!(
        sha(&vm_input_raw) == "22ad3afe992c69c5ee2017664199817f7371d2af6e2fdb26f3f2db8a26c459ed",
        "foreign VM input"
    );
    let mut program =
        retained_support::load_program(request.program_pins.as_os_str(), &vm_input["program"])?;
    let pins: serde_json::Value = serde_json::from_slice(&fs::read(&request.program_pins)?)?;
    let index = Index::scan(std::path::Path::new(
        pins["assets_root"].as_str().context("asset root")?,
    ))?;
    let components = Store::open(&request.components_store)?;
    ensure!(
        request.incoming.sha256
            == "be2e217cf398e1b33417ff2455b82b2d4f51debfaac8dafedf4219c9b9d935e1"
            && request.graphics.sha256
                == "b3601cb4c1bea04a2a4ed7d876bb89e4dcb73908276b3d02c37ea4409a2ae677",
        "foreign Incoming/Registry components"
    );
    let incoming: Incoming = components.get_value(&request.incoming)?;
    ensure!(
        incoming.vm.tick == 88444 && incoming.vm.world.objects.len() == 2,
        "incoming cut differs"
    );
    let initial_snapshot = Snapshot::capture(&program, &incoming.vm);
    let initial_cold = initial_snapshot
        .clone()
        .restore(&program)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        Snapshot::capture(&program, &initial_cold) == initial_snapshot,
        "initial VM cold restore differs"
    );
    let saved_graphics = components.get_value(&request.graphics)?;
    let registry = sims_content::graphic_loading::Registry::restore(&index, saved_graphics)?;
    let mut models = std::collections::BTreeMap::new();
    for id in registry.retained.objects() {
        let input = registry
            .retained
            .owner(id)
            .context("retained owner absent")?
            .frame()
            .context("retained frame owner absent")?
            .input();
        ensure!(
            input.retained.global_focus.is_none(),
            "captured global base focus is not null"
        );
        models.insert(id, input.retained.model.id);
    }
    let mut host = PrefixAssets {
        registry,
        prepared: false,
        allow_teardown: request.through_clear,
        index: &index,
        store: &store,
        unused_pool: request.through_clear.then(Default::default),
        models,
        notifications: vec![],
        service_cuts: vec![],
        service_cut_checks: vec![],
    };
    host.registry.validate(&incoming.vm)?;
    let graphics_before = host.registry.snapshot();
    let packed = fs::read(&request.controller_plan)?;
    ensure!(
        sha(&packed) == request.controller_plan_sha256,
        "controller plan changed"
    );
    let plan: Plan = serde_json::from_slice(&packed)?;
    ensure!(
        plan.schema == 1
            && plan.entry["row"]["sha256"] == ENTRY
            && plan.controllers.len() == 141
            && plan.all_controller_backings_qualified_readonly,
        "controller plan cut/coverage differs"
    );
    ensure!(
        plan.controllers
            .windows(2)
            .all(|w| w[0].ordinal < w[1].ordinal),
        "controller order differs"
    );
    let mut guids = BTreeSet::new();
    let mut source_receipts = std::collections::BTreeMap::new();
    let mut catalog = vec![];
    for c in &plan.controllers {
        ensure!(
            guids.insert(c.guid)
                && c.normalized_words.len() == 108
                && matches!(c.kind, 7 | 12)
                && c.global_flag != 0
                && c.normalized_words[9] == c.kind
                && c.normalized_words[42] == c.global_flag
                && c.normalized_words[102] & 1 == 0,
            "invalid controller metadata"
        );
        ensure!(
            !c.source
                .to_ascii_lowercase()
                .starts_with("captured-start/userdata"),
            "unqualified writable controller"
        );
        let source = &index.resources[index.find(&c.source)?];
        let digest = sha(&source.read()?);
        ensure!(
            digest == c.source_sha256,
            "controller source changed: {}",
            c.source
        );
        source_receipts.insert(c.source.clone(), digest);
        catalog.push(Controller {
            guid: c.guid,
            label: sims_formats::strings::decode(&c.label_bytes, 1),
            definition: sims_content::saves::DefinitionSource {
                source: c.source.clone(),
                sha256: c.source_sha256.clone(),
                chunk: c.object,
                kind: c.kind,
                multipart: c.normalized_words[10] != 0,
            },
        });
    }
    sims_content::catalog::link_controllers(&index, &mut program, &catalog)?;
    if let Some(expected) = &request.career_source_sha256 {
        ensure!(
            expected == "7038417822b72f1190788b3070d7ec9ccabe7f4becab233b7d59e2003dff6a0f",
            "unrecognized immutable career source"
        );
        let careers = sims_content::careers::load_with_expansions(&index, 0xff)?;
        ensure!(&careers.content_hash == expected, "career source changed");
        program.careers = Some(careers);
    }
    for c in &plan.controllers {
        let module = program
            .modules
            .get(&c.source)
            .context("controller module absent")?;
        let words = module
            .object_definition_words
            .get(&c.object)
            .context("controller definition absent")?;
        ensure!(
            words.iter().map(|&w| w as u16).collect::<Vec<_>>() == c.normalized_words,
            "normalized controller words differ: {:08x}",
            c.guid
        );
    }
    let linked_snapshot = Snapshot::capture(&program, &incoming.vm);
    ensure!(
        Snapshot::capture(
            &program,
            &linked_snapshot
                .clone()
                .restore(&program)
                .map_err(anyhow::Error::msg)?
        ) == linked_snapshot,
        "linked Program changed entry VM"
    );
    // The cleared-Simulation seam needs no starting House file at all. Keep
    // old request locator fields only as inert historical provenance.
    let program_blob = store.put_value(&program)?;
    let mut checkpoints = vec![];
    let mut returned: Option<Snapshot> = None;
    let mut removals_at_return = vec![];
    let stop_boundary = if request.through_clear {
        "clear_return"
    } else {
        "resize_return"
    };
    let stop_error = format!("intentional research stop at {stop_boundary}");
    let result = incoming.prepare_cleared_with_assets(
        &program,
        &catalog,
        |name, vm, removals| {
            ensure!(
                matches!(
                    name,
                    "resize_entry" | "resize_return" | "clear_entry" | "clear_return"
                ),
                "unexpected boundary {name}"
            );
            let snapshot = Snapshot::capture(&program, vm);
            let blob = store.put_value(&(snapshot.clone(), removals.to_vec()))?;
            checkpoints.push(
                serde_json::json!({"boundary":name,"snapshot_and_removals":blob,
            "tick":vm.tick,"objects":vm.world.objects.len(),"pending_removals":removals.len()}),
            );
            if name == "resize_entry" {
                ensure!(snapshot == linked_snapshot, "preparation changed entry VM");
                Ok(())
            } else if name == stop_boundary {
                returned = Some(snapshot);
                removals_at_return = removals.to_vec();
                if request.through_clear {
                    Ok(())
                } else {
                    bail!(stop_error.clone())
                }
            } else {
                Ok(())
            }
        },
        &mut host,
    );
    let mut cleared_simulation = None;
    let error = match result {
        Err(e) => format!("{e:#}"),
        Ok(simulation) if request.through_clear => {
            let snapshot = sims_sim::Snapshot::capture(&program, &simulation);
            let blob = store.put_value(&snapshot)?;
            let cold: sims_sim::Snapshot = store.get_value(&blob)?;
            let restored = cold.restore(&program).map_err(anyhow::Error::msg)?;
            ensure!(
                restored == simulation,
                "cleared Simulation cold restore differs"
            );
            cleared_simulation = Some(blob);
            stop_error.clone()
        }
        Ok(_) => format!("UNEXPECTED successful continuation beyond {stop_boundary}"),
    };
    let mut report = serde_json::json!({"schema":1,"request_sha256":sha(&request_raw),
        "controller_plan_sha256":sha(&packed),"controller_sources":source_receipts,
        "linked_program":program_blob,"linked_modules":program.modules.len(),
        "controllers":catalog.len(),"entry_vm_unchanged_by_program_link":true,
        "career_profile":program.careers,"career_same_cut_memory_qualified":false,
        "unused_house_locator":{"root":request.unused_house_root,"file":request.unused_house},
        "starting_house_parsed":false,"house_fields_consumed_by_startup":false,
        "cleared_simulation":cleared_simulation,"cleared_simulation_cold_equal":cleared_simulation.is_some(),
        "reader_or_mesh_pool_constructed":request.through_clear,"reader_constructed":false,
        "adapter_only_unconsumed_mesh_pool":request.through_clear,"native_mesh_pool_claimed":false,
        "asset_host_prepared":host.prepared,"stop_boundary":stop_boundary,
        "asset_service_checkpoints":host.service_cuts,"model_notifications":host.notifications,
        "asset_service_checkpoint_checks":host.service_cut_checks,
        "remaining_model_ids":host.models,"graphics_after":store.put_value(&host.registry.snapshot())?,
        "checkpoints":checkpoints,"result":error,
        "resize_return_reached":checkpoints.iter().any(|v|v["boundary"] == "resize_return"),
        "clear_return_reached":checkpoints.iter().any(|v|v["boundary"] == "clear_return"),
        "full_startup_qualified":false,"original_gameplay_ticks_added":0});
    if let Some(snapshot) = returned {
        let checkpoint: BlobRef = serde_json::from_value(
            checkpoints.last().context("return checkpoint absent")?["snapshot_and_removals"]
                .clone(),
        )?;
        let (cold_snapshot, cold_removals): (Snapshot, Vec<sims_vm::removal::Pending>) =
            store.get_value(&checkpoint)?;
        ensure!(
            cold_snapshot == snapshot && cold_removals == removals_at_return,
            "serialized return VM/removal owners differ"
        );
        let vm = cold_snapshot
            .restore(&program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            Snapshot::capture(&program, &vm) == snapshot,
            "return VM cold restore differs"
        );
        let registry_order: Vec<_> = vm
            .world
            .person
            .as_ref()
            .context("registry absent")?
            .object_order
            .clone();
        let ids: Vec<_> = vm.world.objects.keys().copied().collect();
        let ordered_guids: Vec<_> = ids
            .iter()
            .map(|id| vm.world.objects[id].definition.as_ref().unwrap().guid)
            .collect();
        let expected = &plan.expected_return;
        let match_flags = serde_json::json!({
            "tick":vm.tick == expected["tick"].as_u64().unwrap(),
            "object_count":ids.len() == if request.through_clear {0} else {143},
            "capture_slot_ids":ids == if request.through_clear {vec![]} else {(1..=143).collect::<Vec<i16>>()},
            "native_registry_order":registry_order == if request.through_clear {vec![]} else {(1..=143).rev().collect::<Vec<i16>>()},
            "guid_order":serde_json::to_value(&ordered_guids)? == if request.through_clear {serde_json::json!([])} else {expected["ordered_guids"].clone()},
            "global_rng":serde_json::to_value(vm.rng_state)? == expected["global_rng"],
            "lot_rng":serde_json::to_value(vm.lot_rng_state)? == expected["lot_rng"],
            "crt_rng":serde_json::to_value(vm.crt_rng_state)? == expected["crt_rng"]});
        report["comparison"] = match_flags;
        report["actual_rng"] = serde_json::json!({"global":vm.rng_state,"lot":vm.lot_rng_state,"crt":vm.crt_rng_state});
        report["return_vm_cold_equal"] = true.into();
        report["pending_removals"] = serde_json::to_value(&removals_at_return)?;
        report["graphics_registry_unmodified"] =
            (host.registry.snapshot() == graphics_before).into();
    }
    store.pin(&Pin {
        schema: 1,
        id: "retained-resize-prefix".into(),
        reason:
            "Actual startup prefix only; deliberate stop before saved resource overlay; no strict gameplay parity"
                .into(),
        blobs: std::iter::once(program_blob)
            .chain(
                checkpoints.iter().filter_map(|v| {
                    serde_json::from_value(v["snapshot_and_removals"].clone()).ok()
                }),
            )
            .chain(host.service_cuts.iter().cloned())
            .chain(cleared_simulation)
            .chain(std::iter::once(serde_json::from_value(report["graphics_after"].clone())?))
            .collect(),
    })?;
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string(&report)?);
    ensure!(
        error == stop_error,
        "prefix failed before expected boundary: {error}"
    );
    ensure!(
        report["comparison"]
            .as_object()
            .context("return comparison absent")?
            .values()
            .all(|v| v.as_bool() == Some(true)),
        "initial native return comparison differs"
    );
    Ok(())
}
