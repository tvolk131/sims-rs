//! Explicit completed-World join. This is a model assembly seam, not native
//! baseline admission: callers retain/authenticate byte evidence and unresolved
//! provider/host demands separately. No constructor, defaults, or VM work run.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use sims_vm::{Object, ObjectId, World};
use std::collections::{BTreeMap, BTreeSet};
pub mod program;
pub mod captured;
pub mod callbacks;
pub mod interactions;
pub mod threads;

/// Source-owned objects/person/registry and the separately supplied current
/// definition cache. None of these owners is reconstructed by the join.
pub struct Owners {
    pub objects: BTreeMap<ObjectId, Object>,
    pub scheduler: sims_vm::scheduler::storage::Storage,
    pub person: sims_vm::person::WorldState,
    pub body_profiles: sims_vm::body_profiles::State,
}

const FIELDS: [&str; 23] = [
    "autonomy_constants",
    "map_size",
    "registry_ui_disabled",
    "magic_history",
    "registry_order",
    "indicator_queues",
    "camera_rotation",
    "special_effects",
    "resource_storage",
    "changed_resources",
    "image_cache",
    "ui_owner",
    "dialogs",
    "fires",
    "positions",
    "room_outdoors",
    "room_half_tiles",
    "budget",
    "globals",
    "tree_advertisements",
    "neighborhood",
    "refresh",
    "actions",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fields {
    autonomy_constants: sims_vm::autonomy_constants::Owner,
    map_size: sims_vm::world_map::State,
    registry_ui_disabled: bool,
    magic_history: sims_vm::magic_history::Context,
    registry_order: sims_vm::presenter::RegistryOrder,
    indicator_queues: sims_vm::indicator_unregister::Queues,
    camera_rotation: u8,
    special_effects: sims_vm::effect::Context,
    resource_storage: sims_vm::resource_storage::State,
    changed_resources: BTreeSet<u32>,
    image_cache: sims_vm::image_cache::State,
    ui_owner: sims_vm::ui_owner::State,
    dialogs: sims_vm::dialog::State,
    fires: sims_vm::fires::State,
    positions: BTreeMap<ObjectId, sims_vm::autonomy::Placement>,
    room_outdoors: BTreeMap<i16, bool>,
    room_half_tiles: BTreeMap<i16, i32>,
    budget: sims_vm::budget::State,
    globals: BTreeMap<i16, i16>,
    tree_advertisements: Vec<[i16; 3]>,
    neighborhood: BTreeMap<i16, i16>,
    refresh: sims_vm::refresh::RefreshState,
    actions: sims_vm::action::WorldState,
}

pub fn join(fields: Value, owners: Owners) -> Result<World> {
    let map = fields
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("World system fields are not a map"))?;
    ensure!(
        map.len() == FIELDS.len() && FIELDS.iter().all(|key| map.contains_key(*key)),
        "completed World system fields missing/extra; no defaults permitted"
    );
    let f: Fields = serde_json::from_value(fields)?;
    ensure!(
        f.positions.keys().eq(owners.objects.keys())
            && f.refresh.draw.keys().eq(owners.objects.keys()),
        "completed World physical/draw coverage differs"
    );
    ensure!(
        f.room_outdoors.keys().eq(f.room_half_tiles.keys())
            && f.room_outdoors
                .keys()
                .map(|id| *id as u16)
                .collect::<BTreeSet<_>>()
                == f.refresh.rooms.keys().copied().collect(),
        "completed World room owner coverage differs"
    );
    // Exhaustive literal: adding a World field requires an explicit decision
    // here instead of silently inheriting World::default/serde migration state.
    let world = World {
        autonomy_constants: Some(f.autonomy_constants),
        map_size: Some(f.map_size),
        registry_ui_disabled: f.registry_ui_disabled,
        magic_history: Some(f.magic_history),
        scheduler_storage: Some(owners.scheduler),
        registry_order: Some(f.registry_order),
        indicator_queues: f.indicator_queues,
        camera_rotation: f.camera_rotation,
        special_effects: f.special_effects,
        resource_storage: f.resource_storage,
        changed_resources: f.changed_resources,
        image_cache: f.image_cache,
        body_profiles: owners.body_profiles,
        ui_owner: f.ui_owner,
        dialogs: f.dialogs,
        fires: f.fires,
        positions: f.positions,
        room_outdoors: f.room_outdoors,
        room_half_tiles: f.room_half_tiles,
        budget: Some(f.budget),
        objects: owners.objects,
        globals: f.globals,
        tree_advertisements: f.tree_advertisements,
        neighborhood: f.neighborhood,
        refresh: Some(f.refresh),
        actions: Some(f.actions),
        person: Some(owners.person),
    };
    world
        .scheduler_storage
        .as_ref()
        .unwrap()
        .require_complete_backing(&world)
        .map_err(anyhow::Error::msg)?;
    world
        .validate_registry_order()
        .map_err(anyhow::Error::msg)?;
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }
    fn fields() -> Value {
        let value: Value = serde_json::from_slice(
            &fs::read(root().join(
                "fixtures/private/goth-world-composition-20261028-first/systems-seventh.json",
            ))
            .unwrap(),
        )
        .unwrap();
        value["fields"].clone()
    }
    fn source() -> World {
        let path =
            root().join("fixtures/private/goth-world-composition-20261028-first/executed-seventh");
        let refs: Value =
            serde_json::from_slice(&fs::read(path.join("model-refs.json")).unwrap()).unwrap();
        let store = sims_fixtures::Store::open(path.join("model-store")).unwrap();
        let snapshot: sims_vm::Snapshot = store
            .get_value(&serde_json::from_value(refs["vm"].clone()).unwrap())
            .unwrap();
        snapshot.state.world
    }
    fn owners(mut world: World) -> Owners {
        Owners {
            objects: world.objects,
            scheduler: world.scheduler_storage.take().unwrap(),
            person: world.person.take().unwrap(),
            body_profiles: world.body_profiles,
        }
    }

    #[test]
    #[ignore = "requires private captured original World model"]
    fn actual_completed_world_join_preserves_every_owned_field() {
        let before = source();
        let joined = join(fields(), owners(before.clone())).unwrap();
        assert_eq!(joined, before);
        assert_eq!(joined.objects.len(), 344);
    }

    #[test]
    #[ignore = "requires private captured original World model"]
    fn actual_completed_world_rejects_missing_extra_and_incomplete_owners() {
        let original = fields();
        let world = source();
        for key in FIELDS {
            let mut changed = original.clone();
            changed.as_object_mut().unwrap().remove(key);
            assert!(
                join(changed, owners(world.clone())).is_err(),
                "missing {key} accepted"
            );
        }
        let mut changed = original.clone();
        changed["invented_field"] = serde_json::json!(0);
        assert!(join(changed, owners(world.clone())).is_err());
        let mut changed = original.clone();
        changed["positions"].as_object_mut().unwrap().remove("16");
        assert!(join(changed, owners(world.clone())).is_err());
        let mut changed = original;
        changed["room_outdoors"]
            .as_object_mut()
            .unwrap()
            .remove("0");
        assert!(join(changed, owners(world)).is_err());
    }
}
