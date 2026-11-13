//! Fixed World inventory. Equality obligations are not converted to passes by
//! a present Option or a matching field name; final admission requires each
//! separately qualified original projection, including nested object fields.
use super::{Result, Saved, Value, fact, json, obligation};
pub fn evaluate(saved: &Saved) -> Result<Value> {
    let sims_vm::World {
        autonomy_constants,
        map_size,
        registry_ui_disabled,
        magic_history,
        scheduler_storage,
        registry_order,
        indicator_queues,
        camera_rotation,
        special_effects,
        resource_storage,
        changed_resources,
        image_cache,
        body_profiles,
        ui_owner,
        dialogs,
        fires,
        positions,
        room_outdoors,
        room_half_tiles,
        budget,
        objects,
        globals,
        tree_advertisements,
        neighborhood,
        refresh,
        actions,
        person,
    } = &saved.simulation.state.vm.world;
    let mut rows = vec![];
    macro_rules! required { ($($field:ident => $source:literal),+ $(,)?)=>{$(
        fact(stringify!($field),"required_native_owner",$field.is_some(),&mut rows);
        rows.last_mut().unwrap()["comparison"]=$source.into();
    )+}; }
    required!(autonomy_constants=>"core/systems/autonomy_constants",map_size=>"core/systems/map_size",
        magic_history=>"core/systems/magic_history",scheduler_storage=>"physical/full_registry_and_links",
        registry_order=>"core/systems/registry_order",budget=>"core/systems/budget",
        refresh=>"core/systems/refresh",actions=>"core/systems/actions",person=>"core/person_world");
    // Borrowed fixed inventory rather than serde's compatibility omissions.
    let _ = (
        registry_ui_disabled,
        indicator_queues,
        camera_rotation,
        special_effects,
        resource_storage,
        changed_resources,
        image_cache,
        body_profiles,
        ui_owner,
        dialogs,
        fires,
        positions,
        room_outdoors,
        room_half_tiles,
        objects,
        globals,
        tree_advertisements,
        neighborhood,
    );
    for (field, source) in [
        ("registry_ui_disabled", "core/systems/registry_ui_disabled"),
        ("indicator_queues", "core/systems/indicator_queues"),
        ("camera_rotation", "core/systems/camera_rotation"),
        ("special_effects", "core/systems/special_effects"),
        ("resource_storage", "core/systems/resource_storage"),
        ("changed_resources", "core/systems/changed_resources"),
        ("image_cache", "core/systems/image_cache"),
        ("body_profiles", "supplemental/native_body_profile_owner"),
        ("ui_owner", "core/systems/ui_owner"),
        ("dialogs", "core/systems/dialogs"),
        ("fires", "core/systems/fires"),
        ("positions", "core/systems/positions+physical"),
        ("room_outdoors", "core/systems/room_outdoors"),
        ("room_half_tiles", "core/systems/room_half_tiles"),
        (
            "objects",
            "core+object_storage+characters+physical+supplemental/full_objects",
        ),
        ("globals", "core/systems/globals"),
        ("tree_advertisements", "core/systems/tree_advertisements"),
        ("neighborhood", "core/systems/neighborhood"),
    ] {
        obligation(field, source, &mut rows);
    }
    anyhow::ensure!(rows.len() == 27, "World field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All27 direct World fields. Required owners may not default; fixed native equality and nested-object obligations remain external and mandatory."}),
    )
}
