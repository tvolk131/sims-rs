//! Research-only publication of an explicitly supplied ordinary VM epoch.
//! This validates supplied state, not native provenance or a complete Incoming.
mod retained_support;
use anyhow::{Context, Result, ensure};
use retained_support::load_program;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sims_vm::{
    Program, Snapshot, Vm, World,
    native_suspension::Evidence,
    retained_epoch::{FreshUnpublishedRequestEpoch, OrdinaryThreadInput, VmProjection},
    retained_frames::{ContextBinding, RetainedFrame, RetainedStack, ThreadControls},
    scheduler::storage::Storage,
};
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameInput {
    object: i16,
    raw_arena: String,
    frames: Vec<RetainedFrame>,
    bindings: Vec<ContextBinding>,
    last_result: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    program: serde_json::Value,
    world: World,
    projection: VmProjection,
    frames: Vec<FrameInput>,
    words: Vec<u32>,
    handles: Vec<Option<u32>>,
    last_handle: u32,
    evidence: Evidence,
    epoch: FreshUnpublishedRequestEpoch,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IncomingInput {
    schema: u32,
    source_row_sha256: String,
    vm_input_sha256: String,
    lot_store: PathBuf,
    lot: sims_fixtures::BlobRef,
    indicators: sims_vm::load_indicators::Indicators,
    pools: sims_vm::indicator_pools::Pools,
    room_settings: sims_sim::lot_environment::manager::Settings,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphicInput {
    object: i16,
    native_actor: u64,
    ready: u8,
    input: sims_content::graphic_retirement::observed::frame::Input,
    environment: sims_content::graphic_retirement::observed::frame::Environment,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphicsInput {
    schema: u32,
    source_row_sha256: String,
    vm_input_sha256: String,
    source_cases_sha256: String,
    frames: Vec<GraphicInput>,
    evidence: serde_json::Value,
    components: Option<Vec<ComponentOutput>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentOutput {
    object: i16,
    only_visible: bool,
    expected_input: sims_content::graphic_retirement::observed::frame::Input,
    expected_draw: sims_vm::refresh::DrawState,
}
fn exercise_components(
    store: &sims_fixtures::Store,
    program: &Program,
    vm: &Vm,
    frames: Vec<(
        sims_content::graphic_retirement::observed::frame::Binding,
        sims_content::graphic_retirement::observed::frame::Input,
        sims_content::graphic_retirement::observed::frame::Environment,
    )>,
    outputs: Vec<ComponentOutput>,
) -> Result<serde_json::Value> {
    use sims_content::graphic_retirement::observed::frame;
    ensure!(
        outputs.len() == frames.len(),
        "component golden coverage differs"
    );
    let mut expected = Snapshot::capture(program, vm);
    let mut branch = expected
        .clone()
        .restore(program)
        .map_err(anyhow::Error::msg)?;
    let mut cuts = Vec::new();
    let mut seen = BTreeSet::new();
    let mut report = Vec::new();
    for (binding, input, environment) in frames {
        let id = binding.object;
        let golden = outputs
            .iter()
            .find(|o| o.object == id)
            .context("component native output absent")?;
        ensure!(seen.insert(id), "duplicate component object");
        let mut owner = frame::Owner::new(input, binding)?;
        owner.validate_vm(&branch)?;
        owner.begin_refresh(golden.only_visible)?;
        let mut polls = 0;
        let mut pending_invalidations = 0;
        while !owner.complete() {
            ensure!(
                polls < 64,
                "component Refresh exceeded bounded service count"
            );
            let request = owner.advance(&environment)?;
            polls += 1;
            // This is an independent experiment fork. The live registry above
            // remains the unexecuted incoming snapshot, not a second mutable
            // owner sharing this branch's allocations.
            let before = Snapshot::capture(program, &branch);
            let checkpoint = store.put_value(&(before.clone(), owner.clone()))?;
            let (cold_vm, cold_owner): (Snapshot, frame::Owner) = store.get_value(&checkpoint)?;
            let cold_vm = cold_vm.restore(program).map_err(anyhow::Error::msg)?;
            cold_owner.validate_vm(&cold_vm)?;
            ensure!(
                Snapshot::capture(program, &cold_vm) == before && cold_owner == owner,
                "component service cold continuation differs"
            );
            branch = cold_vm;
            owner = cold_owner;
            cuts.push(checkpoint);
            if request == Some(sims_sim::pose::graphic_frame::Request::Invalidate) {
                ensure!(
                    owner.pending() == request.as_ref(),
                    "invalidation owner differs"
                );
                pending_invalidations += 1;
                // Execute the actual VM primitive. Never acknowledge this
                // external service using an authored Done reply.
                owner.perform_invalidation(&mut branch)?;
            }
        }
        ensure!(
            owner.input() == &golden.expected_input,
            "object {id} original-code Refresh output differs"
        );
        expected
            .state
            .world
            .refresh
            .as_mut()
            .context("expected draw owner absent")?
            .draw
            .insert(id, golden.expected_draw.clone());
        ensure!(
            Snapshot::capture(program, &branch) == expected,
            "object {id} Refresh changed fields outside native draw output"
        );
        ensure!(
            pending_invalidations == 1 && owner.pending().is_none(),
            "component invalidation/return coverage differs"
        );
        report.push(serde_json::json!({"object": id, "polls": polls,
            "pending_invalidation_cold_cuts": pending_invalidations,
            "native_frame_and_draw_equal": true, "other_vm_state_and_rng_unchanged": true}));
    }
    store.pin(&sims_fixtures::Pin {
        schema: 1,
        id: "retained-refresh-component-cuts".into(),
        reason:
            "Forked component experiment with per-service cold resumes; not startup/gameplay ticks"
                .into(),
        blobs: cuts.clone(),
    })?;
    Ok(
        serde_json::json!({"objects": report, "cold_service_cuts": cuts.len(),
        "full_vm_equal_to_native_draw_delta": true, "startup_executed": false,
        "original_gameplay_ticks_added": 0}),
    )
}
fn bytes(hex: &str) -> Result<Vec<u8>> {
    ensure!(hex.len() % 2 == 0, "odd arena hex extent");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|b| Ok(u8::from_str_radix(std::str::from_utf8(b)?, 16)?))
        .collect()
}
fn exact_keys(value: &serde_json::Value, required: &[&str]) -> Result<()> {
    let object = value.as_object().context("explicit object required")?;
    let actual: BTreeSet<_> = object.keys().map(String::as_str).collect();
    let expected: BTreeSet<_> = required.iter().copied().collect();
    ensure!(
        actual == expected,
        "projection field presence differs: {actual:?}"
    );
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        (3..=5).contains(&args.len()),
        "usage: retained_vm_projection INPUT.json PINNED_PROGRAM_LOADER.json NEW_OUTPUT_DIRECTORY [INCOMING_COMPONENTS.json [GRAPHICS_COMPONENTS.json]]"
    );
    let raw = fs::read(&args[0])?;
    let document: serde_json::Value = serde_json::from_slice(&raw)?;
    // General historical snapshot serde intentionally tolerates missing fields.
    // This supplied-state ingress must not silently use those restore defaults.
    exact_keys(
        &document["world"],
        &[
            "autonomy_constants",
            "map_size",
            "registry_ui_disabled",
            "magic_history",
            "scheduler_storage",
            "registry_order",
            "indicator_queues",
            "camera_rotation",
            "special_effects",
            "resource_storage",
            "changed_resources",
            "image_cache",
            "body_profiles",
            "ui_owner",
            "dialogs",
            "fires",
            "positions",
            "room_outdoors",
            "room_half_tiles",
            "budget",
            "objects",
            "globals",
            "tree_advertisements",
            "neighborhood",
            "refresh",
            "actions",
            "person",
        ],
    )?;
    for object in document["world"]["objects"]
        .as_object()
        .context("objects map required")?
        .values()
    {
        exact_keys(
            object,
            &[
                "native_tree_error",
                "native_flags",
                "native_keep_dynamic",
                "routing_slots",
                "relationships",
                "relationship_order",
                "accessories",
                "multitile",
                "cursor",
                "last_update",
                "headline",
                "definition",
                "attributes",
                "temps",
                "dynamic_sprite_flags",
                "data",
                "person",
                "career_appearance",
                "motives",
                "motive_update",
                "motive_changes",
                "slots",
                "container",
                "idle",
            ],
        )?;
    }
    let input: Input = serde_json::from_slice(&raw)?;
    ensure!(input.schema == 1, "unsupported input schema");
    let program = load_program(&args[1], &input.program)?;
    let mut world = input.world;
    ensure!(
        world.scheduler_storage.is_none(),
        "scheduler owner must be assembled from sole raw backing"
    );
    world.scheduler_storage = Some(
        Storage::from_retained(&world, input.handles, input.words, input.last_handle)
            .map_err(anyhow::Error::msg)?,
    );
    let before = world.clone();
    let mut entries = Vec::new();
    for item in input.frames {
        let stack = RetainedStack::from_bytes(
            &program,
            &bytes(&item.raw_arena)?,
            item.frames,
            item.bindings,
        )
        .map_err(anyhow::Error::msg)?;
        let suspension = stack
            .native_suspension(&program, input.evidence.clone())
            .map_err(anyhow::Error::msg)?;
        entries.push(OrdinaryThreadInput {
            object: item.object,
            stack,
            controls: ThreadControls {
                service: None,
                dispatch: None,
                autonomy: None,
                find_best: vec![],
                completed_actions: vec![],
                last_result: item.last_result,
                callback: None,
                callback_result: None,
                check: false,
                interrupted: false,
                status: sims_vm::Status::NativeSuspended(suspension),
            },
        });
    }
    let vm =
        Vm::from_retained_ordinary_epoch(&program, world, input.projection, entries, input.epoch)
            .map_err(anyhow::Error::msg)?;
    ensure!(vm.world == before, "import changed supplied World");
    let snapshot = Snapshot::capture(&program, &vm);
    let output = PathBuf::from(&args[2]);
    fs::create_dir(&output)?;
    // General snapshots skip historical optional fields: use named maps, not
    // a positional struct encoding that shifts fields when one is omitted.
    let store = sims_fixtures::Store::create(output.join("store"))?;
    let blob = store.put_value(&snapshot)?;
    let cold: Snapshot = store.get_value(&blob)?;
    let restored = cold.restore(&program).map_err(anyhow::Error::msg)?;
    ensure!(
        Snapshot::capture(&program, &restored) == snapshot,
        "cold VM state differs"
    );
    store.pin(&sims_fixtures::Pin {
        schema: 1,
        id: "retained-ordinary-vm".into(),
        reason: "Typed ordinary VM epoch only; wider original caller/Incoming still unqualified"
            .into(),
        blobs: vec![blob.clone()],
    })?;
    let graphics_report = if let Some(path) = args.get(4) {
        let graphic_raw = fs::read(path)?;
        let supplied: GraphicsInput = serde_json::from_slice(&graphic_raw)?;
        ensure!(
            (1..=2).contains(&supplied.schema)
                && supplied.source_row_sha256
                    == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a"
                && supplied.source_cases_sha256
                    == "41661036fb7e6a287a6fc9654ff59c99d088d57615c929c09a449b173302f36e"
                && supplied.vm_input_sha256 == format!("{:x}", Sha256::digest(&raw))
                && supplied.evidence["row"]["sha256"] == supplied.source_row_sha256
                && supplied.evidence["elf_sha256"]
                    == "280e32b85aac6e4e4e9933e7fc5b6a12d58fd2918a7420b16e95bbd91aea3e65",
            "graphic components belong to another VM/cut"
        );
        ensure!(
            (supplied.schema == 1 && supplied.components.is_none())
                || (supplied.schema == 2 && supplied.components.is_some()),
            "component schema/output presence differs"
        );
        ensure!(
            supplied.frames.len() == vm.world.objects.len(),
            "requires complete captured ordinary graphic coverage"
        );
        let mut graphics = sims_content::graphic_loading::Registry::default();
        let mut readies = std::collections::BTreeMap::new();
        let mut component_frames = Vec::new();
        for entry in supplied.frames {
            let object = vm
                .world
                .objects
                .get(&entry.object)
                .context("graphic object absent")?;
            let definition = object
                .definition
                .clone()
                .context("graphic definition absent")?;
            let lifetime = vm
                .world
                .scheduler_storage
                .as_ref()
                .context("graphic registry absent")?
                .handle(entry.object)
                .context("graphic canonical lifetime absent")?;
            let position = vm
                .world
                .positions
                .get(&entry.object)
                .context("graphic position absent")?;
            ensure!(
                entry.environment.tick == vm.tick as i32
                    && entry.environment.camera_rotation == vm.world.camera_rotation
                    && entry.environment.physical.position == [position.x, position.y]
                    && entry.environment.physical.level == position.level
                    && object.data.get(&1) == Some(&entry.environment.physical.facing),
                "graphic environment differs from retained VM"
            );
            ensure!(
                readies.insert(entry.object, entry.ready).is_none(),
                "duplicate graphic object"
            );
            let binding = sims_content::graphic_retirement::observed::frame::Binding {
                object: entry.object,
                definition,
                registry_lifetime: lifetime,
                native_actor: entry.native_actor,
            };
            component_frames.push((
                binding.clone(),
                entry.input.clone(),
                entry.environment.clone(),
            ));
            graphics.install_retained_frame(
                binding,
                entry.ready,
                entry.input,
                sims_content::graphic_loading::retained::RefreshContext {
                    floor_vector_bits: entry.environment.floor_vector_bits,
                    fp_control: entry.environment.fp_control,
                    origin_pose_bits: entry.environment.origin_pose_bits,
                },
            )?;
        }
        graphics.validate(&vm)?;
        ensure!(
            graphics.presenter_readiness(&vm, &program)? == readies,
            "captured graphic readiness differs"
        );
        let saved = graphics.snapshot();
        let graphic_blob = store.put_value(&saved)?;
        let cold: sims_content::graphic_loading::Saved = store.get_value(&graphic_blob)?;
        let pins: serde_json::Value = serde_json::from_slice(&fs::read(&args[1])?)?;
        let index = sims_content::Index::scan(std::path::Path::new(
            pins["assets_root"]
                .as_str()
                .context("graphic immutable asset root absent")?,
        ))?;
        let restored = sims_content::graphic_loading::Registry::restore(&index, cold)?;
        restored.validate(&vm)?;
        ensure!(
            restored.snapshot() == saved
                && restored.presenter_readiness(&vm, &program)? == readies
                && Snapshot::capture(&program, &vm) == snapshot,
            "cold graphics or retained VM differ"
        );
        store.pin(&sims_fixtures::Pin {
            schema: 1, id: "retained-ordinary-graphics".into(),
            reason: "Source-bound ordinary Animator registry only; no Reader/mesh-pool/startup execution".into(),
            blobs: vec![graphic_blob.clone()],
        })?;
        let component_report = supplied
            .components
            .map(|outputs| exercise_components(&store, &program, &vm, component_frames, outputs))
            .transpose()?;
        ensure!(
            Snapshot::capture(&program, &vm) == snapshot,
            "component fork changed incoming VM"
        );
        Some(serde_json::json!({"snapshot": graphic_blob,
            "input_sha256": format!("{:x}", Sha256::digest(&graphic_raw)),
            "source_cases_sha256": supplied.source_cases_sha256,
            "ready": readies, "cold_equal": true, "vm_unchanged": true,
            "forked_refresh_components": component_report,
            "reader_or_mesh_pool_constructed": false, "startup_executed": false}))
    } else {
        None
    };
    let incoming_report = if let Some(path) = args.get(3) {
        let component_raw = fs::read(path)?;
        let component_document: serde_json::Value = serde_json::from_slice(&component_raw)?;
        exact_keys(&component_document["indicators"], &["primary", "secondary"])?;
        let components: IncomingInput = serde_json::from_slice(&component_raw)?;
        ensure!(
            components.schema == 1,
            "unsupported incoming component schema"
        );
        ensure!(
            components.source_row_sha256
                == "e5002772820e16fc1ee15628ab08c598676d61b31a1d8e0df80a05d8938dde7a"
                && components.vm_input_sha256 == format!("{:x}", Sha256::digest(&raw)),
            "incoming components belong to a different supplied VM/cut"
        );
        ensure!(
            components.lot.sha256
                == "885591d9ad92ceba2e4009444f8019eb4cfa2d950a9f869fd64566bcd72494c7"
                && components.lot.bytes == 495036,
            "unsupported captured lot component"
        );
        let lot_store = sims_fixtures::Store::open(&components.lot_store)?;
        let lot: sims_sim::lot_geometry::Lot = lot_store.get_value(&components.lot)?;
        lot.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            vm.world
                .map_size
                .context("VM active extent absent")?
                .active_size
                == i32::try_from(lot.active_extent())?,
            "incoming VM/lot active extents differ"
        );
        let metrics: std::collections::BTreeMap<_, _> = lot
            .floors
            .iter()
            .flat_map(|f| f.metrics.iter().map(|(&id, m)| (id, m)))
            .collect();
        ensure!(
            metrics.len() == vm.world.room_outdoors.len()
                && metrics.iter().all(|(id, m)| vm.world.room_outdoors.get(id)
                    == Some(&m.outdoors)
                    && vm.world.room_half_tiles.get(id) == Some(&m.half_tiles)),
            "incoming VM/lot room context differs"
        );
        ensure!(
            vm.world.objects.values().all(|o| o.person.is_none())
                && vm.world.positions.values().all(|p| p.x < 0 && p.y < 0),
            "unsupported incoming ordinary/off-lot profile"
        );
        for id in [
            components.indicators.primary,
            components.indicators.secondary,
        ]
        .into_iter()
        .chain(components.pools.active.iter().copied())
        .chain(components.pools.spare.iter().copied())
        .flatten()
        {
            ensure!(
                vm.world.objects.contains_key(&id),
                "incoming indicator has no object"
            );
        }
        let incoming = sims_content::house_startup::Incoming {
            vm,
            lot,
            indicators: components.indicators,
            pools: components.pools,
            room_settings: components.room_settings,
        };
        let incoming_blob = store.put_value(&incoming)?;
        let cold: sims_content::house_startup::Incoming = store.get_value(&incoming_blob)?;
        // Plain serde is not VM validation. Reuse the existing Snapshot restore
        // checks, then compare the remaining explicitly retained DTO owners.
        let cold_vm = Snapshot::capture(&program, &cold.vm)
            .restore(&program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            Snapshot::capture(&program, &cold_vm) == snapshot
                && cold.lot == incoming.lot
                && cold.indicators == incoming.indicators
                && cold.pools == incoming.pools
                && cold.room_settings == incoming.room_settings,
            "cold incoming components differ"
        );
        cold.lot.validate().map_err(anyhow::Error::msg)?;
        store.pin(&sims_fixtures::Pin {
            schema: 1,
            id: "retained-incoming-dto".into(),
            reason:
                "Typed incoming DTO only; graphics host and startup execution are not qualified"
                    .into(),
            blobs: vec![incoming_blob.clone()],
        })?;
        Some(
            serde_json::json!({"components_sha256": format!("{:x}", Sha256::digest(&component_raw)),
            "snapshot": incoming_blob, "cold_equal": true,
            "graphics_asset_host_constructed": false, "startup_executed": false}),
        )
    } else {
        None
    };
    store.verify_all()?;
    let report = serde_json::json!({
        "schema": 1, "tick": snapshot.state.tick, "objects": before.objects.len(),
        "threads": snapshot.state.threads.len(), "world_unchanged": true, "cold_equal": true,
        "snapshot": blob, "full_original_process_or_incoming": false,
        "typed_incoming_dto": incoming_report,
        "retained_graphics_registry": graphics_report,
        "original_parity_ticks_added": 0,
    });
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
