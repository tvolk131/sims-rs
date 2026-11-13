//! Exhaustive Simulation field classification for the completed wrapper epoch.
use super::{Context, Result, Saved, Value, fact, json, obligation, same};
use sims_sim::{Body, Point, Room, Simulation};
use std::collections::{BTreeMap, BTreeSet};

pub fn evaluate(saved: &Saved) -> Result<Value> {
    let Simulation {
        native_scheduler,
        fixture_driver,
        visit,
        visited,
        immediate,
        cancellation,
        reach_callback,
        ui_wait_callback,
        backgrounds,
        characters,
        retired_characters,
        retired_graphics,
        retained_ordinary_graphics,
        native_character_settings,
        room_environment,
        architecture,
        native_lot,
        person_factories,
        portrait_person,
        synchronous_removal,
        ground_placement,
        controller_reconciliation,
        object_pickup,
        object_placement,
        cursor_factory,
        cursor_rotation,
        cursor_release,
        cursor_cleanup,
        native_selection,
        native_selection_broadcasts,
        native_indicator_pools,
        native_creations,
        native_transfers,
        native_reaches,
        native_burn,
        removals,
        deletion,
        carry,
        unplaced,
        turns,
        household,
        vm,
        room,
        bodies,
        routes,
        route_feedback,
        route_control,
        portal_cache,
        shoo_creation,
        reservations,
        pending_actions,
        animations,
        interactions,
    } = &saved.simulation.state;
    let mut rows = vec![];
    macro_rules! absent { ($($f:ident),+ $(,)?)=>{$(
        fact(stringify!($f),"unpublished_replacement_call",$f.is_none(),&mut rows);
    )+}; }
    macro_rules! empty { ($($f:ident),+ $(,)?)=>{$(
        fact(stringify!($f),"unpublished_replacement_collection",$f.is_empty(),&mut rows);
    )+}; }
    absent!(
        fixture_driver,
        visit,
        immediate,
        cancellation,
        reach_callback,
        ui_wait_callback,
        portrait_person,
        synchronous_removal,
        ground_placement,
        controller_reconciliation,
        object_pickup,
        object_placement,
        cursor_factory,
        cursor_rotation,
        cursor_release,
        cursor_cleanup,
        native_selection,
        native_burn,
        shoo_creation
    );
    empty!(
        visited,
        backgrounds,
        person_factories,
        native_creations,
        native_transfers,
        native_reaches,
        carry,
        turns,
        routes,
        route_feedback,
        route_control,
        portal_cache,
        reservations,
        pending_actions,
        animations,
        interactions
    );
    // A complete native frame still lives in the VM. The absent fields above
    // are new Rust callers/fixture caches, not reconstructed native requests.
    for (name, present, comparison) in [
        (
            "native_character_settings",
            native_character_settings.is_some(),
            "simulation_owners/native_character_settings_all_fields",
        ),
        (
            "room_environment",
            room_environment.is_some(),
            "environment/complete_manager",
        ),
        (
            "architecture",
            architecture.is_some(),
            "physical+environment/native_lot_architecture",
        ),
        (
            "native_lot",
            native_lot.is_some(),
            "physical+routing+simulation_owners",
        ),
        (
            "household",
            household.is_some(),
            "simulation_owners+core/person_history",
        ),
    ] {
        fact(name, "required_native_owner", present, &mut rows);
        // Keep the external equality obligation attached to this same row.
        rows.last_mut().unwrap()["comparison"] = comparison.into();
    }
    let scheduler = native_scheduler
        .as_ref()
        .context("completed native scheduler absent")?;
    let idle = sims_sim::scheduler::State::from_completed_cursor(scheduler.cursor)
        .map_err(anyhow::Error::msg)?;
    same(
        "native_scheduler/new_completed_pass",
        &Some(idle),
        native_scheduler,
        &mut rows,
    )?;
    rows.last_mut().unwrap()["native_cursor_comparison_required"] = true.into();
    fact(
        "native_selection_broadcasts",
        "replacement_application_semantics",
        *native_selection_broadcasts,
        &mut rows,
    );
    obligation(
        "characters",
        "characters/complete_source_Residents",
        &mut rows,
    );
    let _ = (characters, native_indicator_pools);
    obligation(
        "native_indicator_pools",
        "physical/actual_active_spare_pools",
        &mut rows,
    );
    obligation(
        "vm",
        "all_VM_fields_and_native_objects_require_full_VM_contract",
        &mut rows,
    );
    // These are real removal domains, not merely adapter calls. Their empty
    // shape cannot establish native absence; a separate original removal and
    // lifetime proof remains mandatory even when this shape check succeeds.
    for (name, empty) in [
        ("retired_characters", retired_characters.is_empty()),
        ("retired_graphics", retired_graphics.is_empty()),
        ("removals", removals.is_empty()),
        ("deletion", deletion.is_empty()),
    ] {
        fact(name, "native_absence_obligation", empty, &mut rows);
        rows.last_mut().unwrap()["native_absence_required_separately"] = true.into();
    }
    let actual_graphics: BTreeSet<_> = saved.assets.ordinary.graphics.keys().copied().collect();
    same(
        "retained_ordinary_graphics/asset_registry_alias",
        &actual_graphics,
        retained_ordinary_graphics,
        &mut rows,
    )?;
    // Authored routing caches are redundant projections; current native
    // footprint/terrain/routing owners are required independently above.
    let mut expected_bodies = BTreeMap::new();
    let mut expected_unplaced = BTreeSet::new();
    for (&id, object) in &vm.world.objects {
        let p = vm
            .world
            .positions
            .get(&id)
            .context("body projection position absent")?;
        if p.x < 0 || p.y < 0 {
            expected_unplaced.insert(id);
            continue;
        }
        let direction = *object
            .data
            .get(&1)
            .context("body projection direction absent")?;
        expected_bodies.insert(
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
    same(
        "bodies/native_position_projection",
        &expected_bodies,
        bodies,
        &mut rows,
    )?;
    same(
        "unplaced/native_position_projection",
        &expected_unplaced,
        unplaced,
        &mut rows,
    )?;
    let a = architecture
        .as_ref()
        .context("native architecture absent")?;
    let expected_room = Room {
        width: a.width,
        height: a.height,
        obstacles: vec![],
    };
    same(
        "room/authored_cache_projection",
        &expected_room,
        room,
        &mut rows,
    )?;
    anyhow::ensure!(
        rows.len() == 53,
        "Simulation field coverage inventory changed"
    );
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All53 direct Simulation fields classified. Fresh wrapper/cache checks do not establish native absence; explicit native obligations, VM/nested owner coverage and side effects remain mandatory."}),
    )
}
