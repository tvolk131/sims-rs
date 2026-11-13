//! Join current native owners into the real Simulation checkpoint type.
//! This is a fresh, unpublished Rust service epoch at the proved native main
//! stack boundary, not a reconstruction of historical Rust requests or logs.
//! Asset Host/Session and gameplay parity remain separate admission gates.
use super::Imported;
use crate::original_session::{completed_lot, completed_skin};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::{Body, Point, Room, Simulation};
use sims_vm::{ObjectId, Program, Snapshot};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Composed {
    pub snapshot: sims_sim::Snapshot,
    pub source: Value,
    pub complete_native_simulation_admitted: bool,
}

pub struct Input<'a> {
    pub vm: Snapshot,
    pub program: &'a Program,
    pub owners: &'a Imported,
    pub lot: &'a completed_lot::Imported,
    pub environment: &'a Value,
    /// The original component input binds each Resident to its physical and
    /// animator addresses. A typed Resident alone does not establish aliases.
    pub resident_source: &'a Value,
    pub boundary: &'a completed_skin::Input,
}

/// Extend a qualified partial Program with the actually resident clips. Only
/// the checkpoint's Program binding metadata changes; Vm bytes do not. These
/// semantic transports do not claim an original animation file/cache identity.
pub fn with_current_clips(
    mut program: Program,
    mut vm: Snapshot,
    source: &Value,
) -> Result<(Program, Snapshot, Value)> {
    vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    let previous = vm.program_hash.clone();
    let state = serde_json::to_vec(&vm.state)?;
    let animation = &source["skin_input"]["skeleton_input"]["animation_input"];
    let observed = crate::conformance::completed_animations::import_parts(animation)?;
    let mut names = vec![];
    for (name, asset) in observed.assets {
        if let Some(existing) = program.animations.get(&name) {
            ensure!(
                serde_json::to_vec(existing)? == serde_json::to_vec(&asset)?,
                "current animation conflicts with Program: {name}"
            );
        } else {
            program.animations.insert(name.clone(), asset);
        }
        names.push(name);
    }
    vm.program_hash = program.fingerprint();
    let restored = vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    ensure!(
        serde_json::to_vec(&restored)? == state,
        "clip join changed VM/RNG"
    );
    let receipt = json!({"previous_program":previous,"current_program":vm.program_hash,
        "observed_clips":names,"vm_state_unchanged":true,"original_file_identity_claimed":false,
        "provenance":animation["provenance"]});
    Ok((program, vm, receipt))
}

pub fn import(input: Input<'_>) -> Result<Composed> {
    let Input {
        vm,
        program,
        owners,
        lot,
        environment,
        resident_source,
        boundary,
    } = input;
    // This also checks every original thread's reentry and the score cache:
    // there is no accepted Rust request, registry visit or RNG scope to erase.
    let household = owners.household(&vm, boundary)?;
    let ids = completed_skin::synchronous_boundary(boundary)?;
    let residents = crate::conformance::completed_residents::import_parts(resident_source)?;
    import_fields(vm, program, owners, lot, environment, resident_source, boundary,
        residents, ids, household, None)
}

pub struct CapturedInput<'a, 'm> {
    pub vm: Snapshot,
    pub program: &'a Program,
    pub owners: &'a Imported,
    pub lot: &'a completed_lot::Imported,
    pub environment: &'a Value,
    pub resident_source: &'a Value,
    pub boundary: &'a completed_skin::Input,
    pub residents: super::super::completed_residents::Imported,
    pub state: &'a Value,
    pub memory: &'a sims_fixtures::readset::Readset<'m>,
    pub routes: &'a BTreeMap<ObjectId, super::super::completed_routes::Person>,
    pub pending: &'a BTreeMap<ObjectId, super::super::completed_route_controls::Pending>,
    pub route_source: &'a Value,
    pub epoch: &'a str,
}

pub fn import_captured(input: CapturedInput<'_, '_>) -> Result<Composed> {
    let CapturedInput {vm, program, owners, lot, environment, resident_source,
        boundary, residents, state, memory, routes, pending, route_source, epoch}=input;
    let household=owners.household_captured(&vm,memory,epoch)?;
    let ids=completed_skin::captured::boundary(boundary,state,memory)?;
    import_fields(vm,program,owners,lot,environment,resident_source,boundary,
        residents,ids,household,Some(Routes{state,memory,routes,pending,source:route_source,epoch}))
}
struct Routes<'a,'m> {
    state:&'a Value,
    memory:&'a sims_fixtures::readset::Readset<'m>,
    routes:&'a BTreeMap<ObjectId,super::super::completed_routes::Person>,
    pending:&'a BTreeMap<ObjectId,super::super::completed_route_controls::Pending>,
    source:&'a Value,
    epoch:&'a str,
}

/// Current clips already qualified by the resident backing/list/cursor importer.
/// Only Program binding metadata changes; all VM/RNG state remains identical.
pub fn with_captured_clips(
    mut program: Program,
    mut vm: Snapshot,
    assets: BTreeMap<String, sims_vm::program::AnimationAsset>,
) -> Result<(Program, Snapshot)> {
    vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    let state=serde_json::to_vec(&vm.state)?;
    for (name,asset) in assets {
        if let Some(old)=program.animations.get(&name) {
            ensure!(serde_json::to_vec(old)?==serde_json::to_vec(&asset)?,"current clip conflicts with Program: {name}");
        } else { program.animations.insert(name,asset); }
    }
    vm.program_hash=program.fingerprint();
    let restored=vm.clone().restore(&program).map_err(anyhow::Error::msg)?;
    ensure!(serde_json::to_vec(&restored)?==state,"current clip join changed VM/RNG");
    Ok((program,vm))
}

#[allow(clippy::too_many_arguments)]
fn import_fields(
    mut vm:Snapshot,program:&Program,owners:&Imported,lot:&completed_lot::Imported,
    environment:&Value,resident_source:&Value,boundary:&completed_skin::Input,
    residents:super::super::completed_residents::Imported,ids:BTreeSet<ObjectId>,
    household:super::HouseholdJoin,routes:Option<Routes<'_,'_>>,
) -> Result<Composed> {
    let before = serde_json::to_vec(&vm)?;
    let previous_slots: BTreeMap<_, _> = vm
        .state
        .world
        .objects
        .iter()
        .map(|(&id, o)| (id, o.native_slot_geometry.clone()))
        .collect();
    let physical = completed_lot::physical::import(lot, &vm.state.world)?;
    let environment = completed_lot::environment::import(lot, environment, &vm.state.world)?;
    let architecture = completed_lot::architecture(lot, program, &vm.state.world)?;
    let native_people: BTreeSet<_> = vm
        .state
        .world
        .objects
        .iter()
        .filter_map(|(&id, object)| object.person.as_ref().map(|_| id))
        .collect();
    ensure!(
        residents.residents.keys().copied().collect::<BTreeSet<_>>() == native_people,
        "completed Simulation resident/World person coverage differs"
    );
    let source_people = resident_source["resident_input"]["actors"]
        .as_array()
        .context("completed Simulation resident source absent")?;
    ensure!(
        source_people.len() == native_people.len(),
        "resident source coverage differs"
    );
    let physical_rows = lot.source["objects"]
        .as_array()
        .context("physical rows absent")?;
    let mut paths = BTreeMap::new();
    let mut offsets = BTreeMap::new();
    let mut ordinary = BTreeSet::new();
    for row in physical_rows {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("physical id absent")?
            .try_into()?;
        let raw = super::super::completed_geometry::bytes(row, "raw")?;
        let animator = u32::from_le_bytes(raw[0x120..0x124].try_into()?);
        let slot_bytes = super::super::completed_geometry::bytes(row, "slots_raw")?;
        let geometry: Vec<_> = slot_bytes
            .chunks_exact(36)
            .skip(1)
            .map(|b| sims_vm::slot_geometry::Slot {
                anchor: None,
                offset: std::array::from_fn(|i| {
                    u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
                }),
                height: i32::from_le_bytes(b[20..24].try_into().unwrap()),
            })
            .collect();
        let object = vm
            .state
            .world
            .objects
            .get_mut(&id)
            .context("slot owner absent")?;
        ensure!(
            geometry.len() == object.slots.len(),
            "current slot geometry count differs"
        );
        for slot in &geometry {
            slot.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            object
                .native_slot_geometry
                .as_ref()
                .is_none_or(|old| *old == geometry),
            "contradictory retained slot geometry"
        );
        object.native_slot_geometry = Some(geometry);
        if native_people.contains(&id) {
            let a = source_people
                .iter()
                .find(|a| a["object"] == id)
                .context("resident physical alias absent")?;
            let parent = boundary
                .actors
                .iter()
                .find(|p| p.object == id)
                .context("resident completed binding absent")?;
            // Person animators are person+adc. Base object+120 is the
            // separate ordinary graphic owner and is null on these people.
            ensure!(
                a["person"] == row["pointer"]
                    && a["person_raw"] == row["raw"]
                    && a["person"] == parent.person
                    && a["animator"] == parent.animator
                    && a["animator_raw"] == parent.animator_raw,
                "resident physical/animator epoch differs"
            );
            let pointer = a["person"]
                .as_u64()
                .context("resident person pointer absent")?;
            // Animator +184 points at the person's retained +ae0 vector,
            // not a new path assembled from destination or current position.
            ensure!(
                a["path"]["pointer"] == pointer + 0xae0,
                "native person/animator retained path alias differs"
            );
            let resident = &residents.residents[&id];
            let path = resident
                .controls
                .movement
                .path
                .as_ref()
                .context("observed native retained path owner absent")?;
            paths.insert(id, path.points.clone());
            // Current compensation mirror, not an assertion that a Snap has
            // or has not happened. The authoritative animator retains it too.
            offsets.insert(id, resident.character.placement.offset.map(f32::to_bits));
        } else if animator != 0 {
            ordinary.insert(id);
        }
    }
    let registry = super::super::completed_geometry::bytes(&lot.source, "registry_raw")?;
    ensure!(
        registry[0x58..0x5c] == registry[0x5c..0x60],
        "completed native removal queue is not empty"
    );
    let mut bodies = BTreeMap::new();
    let mut unplaced = BTreeSet::new();
    for (&id, object) in &vm.state.world.objects {
        ensure!(ids.contains(&id), "object outside completed boundary proof");
        let p = vm
            .state
            .world
            .positions
            .get(&id)
            .context("retained position absent")?;
        if p.x < 0 || p.y < 0 {
            unplaced.insert(id);
            continue;
        }
        let direction = *object.data.get(&1).context("retained direction absent")?;
        bodies.insert(
            id,
            Body {
                position: Point::new(p.y, p.x),
                level: p.level,
                facing: 1 << (direction as u16 & 7),
                radius: 0,
                elevation: 0,
            },
        );
    }
    // All native physical collision/extents/elevation remain in the lot,
    // bounds, slots and Residents. These are the native mode's projection
    // caches, not guessed footprints or replacement obstacle geometry.
    let room = Room {
        width: architecture.width,
        height: architecture.height,
        obstacles: vec![],
    };
    let mut stripped = vm.clone();
    for (&id, object) in &mut stripped.state.world.objects {
        object.native_slot_geometry = previous_slots[&id].clone();
    }
    ensure!(
        serde_json::to_vec(&stripped)? == before,
        "slot owner join changed existing VM/RNG"
    );
    let before = serde_json::to_vec(&vm)?;
    let native = sims_sim::lot_runtime::State {
        placement: physical.placement,
        membership: physical.membership,
        indicators: physical.indicators,
        diagnostics: owners.diagnostics,
        route_paths: paths,
        snap_offsets: offsets,
        selected_person: physical.selected_person,
        // These are new Rust service calls/logs. Original suspended VM frames
        // remain NativeRegistryReentry, and no native callback is synthesized.
        observers: vec![],
        route: None,
        shoo: None,
        movement: None,
        completion: None,
        snap: None,
        find_location: None,
        removal: None,
        room_effects: vec![],
        routing_policy: sims_sim::lot_route::path::Policy::Strict,
    };
    let mut state = Simulation {
        native_scheduler: Some(
            sims_sim::scheduler::State::from_completed_cursor(owners.scheduler_cursor)
                .map_err(anyhow::Error::msg)?,
        ),
        fixture_driver: None,
        visit: None,
        visited: BTreeSet::new(),
        immediate: None,
        cancellation: None,
        reach_callback: None,
        ui_wait_callback: None,
        backgrounds: BTreeMap::new(),
        characters: residents.residents,
        retired_characters: BTreeMap::new(),
        retired_graphics: BTreeMap::new(),
        retained_ordinary_graphics: ordinary.clone(),
        native_character_settings: Some(owners.character),
        room_environment: Some(sims_sim::lot_environment::live::Live {
            manager: environment.manager,
            room_effects_consumed: 0,
            clock_events_consumed: 0,
        }),
        architecture: Some(architecture),
        native_lot: Some(native),
        person_factories: BTreeMap::new(),
        portrait_person: None,
        synchronous_removal: None,
        ground_placement: None,
        controller_reconciliation: None,
        object_pickup: None,
        object_placement: None,
        cursor_factory: None,
        cursor_rotation: None,
        cursor_release: None,
        cursor_cleanup: None,
        native_selection: None,
        // Configuration of the replacement dispatcher's original application
        // semantics; this is not represented as a native saved boolean.
        native_selection_broadcasts: true,
        native_indicator_pools: physical.pools,
        native_creations: BTreeMap::new(),
        native_transfers: BTreeMap::new(),
        native_reaches: BTreeMap::new(),
        native_burn: None,
        removals: vec![],
        deletion: vec![],
        carry: BTreeMap::new(),
        unplaced,
        turns: BTreeMap::new(),
        household: Some(household.state),
        vm: vm.state,
        room,
        bodies,
        routes: BTreeMap::new(),
        route_feedback: BTreeMap::new(),
        route_control: BTreeMap::new(),
        portal_cache: BTreeMap::new(),
        shoo_creation: None,
        reservations: BTreeMap::new(),
        pending_actions: BTreeMap::new(),
        animations: BTreeMap::new(),
        interactions: BTreeMap::new(),
    };
    let mut retained_execution=None;
    if let Some(r)=routes {
        let execution=super::super::completed_route_execution::import(
            super::super::completed_route_execution::Input {
                state:r.state,source:r.source,people:r.routes,pending:r.pending,
                memory:r.memory,vm:&state.vm,bodies:&state.bodies,epoch:r.epoch,
            })?;
        state.route_control=execution.controls;
        state.portal_cache=execution.caches;
        for (actor,walk)in execution.walks {
            state.characters.get_mut(&actor).context("retained walking Character absent")?.native_walk=Some(walk);
        }
        retained_execution=Some(execution.source);
    }
    let snapshot = sims_sim::Snapshot::capture(program, &state);
    let restored = snapshot
        .clone()
        .restore(program)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        restored == state,
        "completed Simulation validator altered retained state"
    );
    ensure!(
        serde_json::to_vec(&sims_vm::Snapshot::capture(program, &restored.vm))? == before,
        "completed Simulation cold validation altered VM/RNG"
    );
    Ok(Composed {
        snapshot,
        complete_native_simulation_admitted: false,
        source: json!({
            "schema":1,"tick":state.vm.tick,"native_parity_ticks":0,
            "delivery_epoch":household.delivery_epoch,
            "retained_route_execution":retained_execution,
            "native_subscriber_or_request_history_claimed":false,
            "person_route_vectors":"observed person+ae0, aliased by animator+184",
            "snap_compensation":"current retained animator+8c, not historical Snap log",
        "contained_slot_geometry":"observed object+130 records: f32 bits+0, height+14; no SLOT reload",
        "existing_vm_state_unchanged_except_newly_observed_slot_owners":true,
            "ordinary_graphics":ordinary,"native_removal_vector_observed_empty":true,
            "projection_caches":"body XY/facing from physical state; collision/slots/terrain stay native",
            "replacement_configuration":{"routing_policy":"Strict","application_selection_broadcasts":true},
            "fresh_rust_domains":["scheduler pass/call/request continuations","service construction/removal/edit/selection wrappers",
                "delivery logs and consumption cursors","authored fixture routing/interaction/animation/carry maps"],
            "remaining":["complete shared asset Host and source file epochs","Session admission",
                "original next-tick state, RNG, ordered instructions and yields"]
        }),
    })
}
