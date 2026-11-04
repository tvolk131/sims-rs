//! Shared current asset owners. Native pointer aliases select pool identity;
//! matching mesh contents never merge distinct native owners. No pose, bounds,
//! allocation callback, script instruction or RNG update runs during this join.
use super::{completed_appearances, completed_ordinary, completed_simulation};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_geometry::{CharacterGeometry, MeshPool},
    character_loading::live::SavedAssets,
    graphic_loading,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_vm::{ObjectId, Program};
use std::collections::BTreeMap;
pub mod checkpoints;
pub(super) mod clothing;
pub(super) mod global;

/// Complete the Program's Global view from actual already-resident BHAV bytes.
/// The native resource owner is validated in full; no by-ID lookup, seek,
/// handle allocation or lazy load is performed. Unloaded records stay absent.
pub fn import_resident_global_routines(
    v: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<usize> {
    global::routines(v, index, locator, program)
}

pub fn import_available_global_constants(
    v: &Value,
    tuning: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<Value> {
    global::constants(v, tuning, index, locator, program)
}

pub fn import_available_default_score_strings(
    v: &Value,
    banks: &Value,
    index: &Index,
    locator: &str,
    program: &mut Program,
) -> Result<Value> {
    global::scores(v, banks, index, locator, program)
}
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Joined {
    pub simulation: sims_sim::Snapshot,
    pub characters: BTreeMap<ObjectId, SavedAssets>,
    pub ordinary: graphic_loading::Saved,
    pub mesh_pool: MeshPool,
    pub skeletons: sims_content::skeleton_resources::Cache,
    pub initializer_settings: sims_sim::pose::animator::Settings,
    pub clothing: sims_content::clothing_catalog::Cache,
    pub autonomy_resources: sims_content::autonomy_resources::Provider,
    pub floors: BTreeMap<u16, sims_content::character_loading::floors::Binding>,
    pub source: Value,
    pub complete_native_host_admitted: bool,
}

fn exact<T: serde::de::DeserializeOwned + Serialize>(v: Value) -> Result<T> {
    let typed: T = serde_json::from_value(v.clone())?;
    ensure!(
        serde_json::to_value(&typed)? == v,
        "shared asset field defaulted/discarded"
    );
    Ok(typed)
}

fn pool(sources: &[(&MeshPool, &[u32])]) -> Result<(Value, Vec<Vec<usize>>, Vec<u32>)> {
    let mut meshes: Vec<Value> = vec![];
    let mut keys = BTreeMap::<String, usize>::new();
    let mut owners = BTreeMap::new();
    let mut pointers = vec![];
    let mut maps = vec![];
    for &(pool, native) in sources {
        let p = serde_json::to_value(pool)?;
        let source = p["meshes"]
            .as_array()
            .context("retained mesh pool absent")?;
        ensure!(
            source.len() == native.len(),
            "native mesh identity/cache coverage differs"
        );
        let mut mapping = vec![];
        for (&pointer, mesh) in native.iter().zip(source) {
            ensure!(pointer != 0, "null retained mesh identity");
            let index = if let Some(&i) = owners.get(&pointer) {
                ensure!(
                    serde_json::to_vec(&meshes[i])? == serde_json::to_vec(mesh)?,
                    "conflicting same-cut shared mesh cache"
                );
                i
            } else {
                let i = meshes.len();
                owners.insert(pointer, i);
                pointers.push(pointer);
                meshes.push(mesh.clone());
                i
            };
            mapping.push(index);
        }
        for (name, index) in p["prototypes"]
            .as_object()
            .context("retained mesh aliases absent")?
        {
            let old = usize::try_from(index.as_u64().context("mesh alias index absent")?)?;
            let global = *mapping.get(old).context("mesh alias outside native pool")?;
            ensure!(
                keys.insert(name.clone(), global)
                    .is_none_or(|old| old == global),
                "shared prototype alias collision"
            );
        }
        maps.push(mapping);
    }
    Ok((json!({"meshes":meshes,"prototypes":keys}), maps, pointers))
}

fn alias(
    index: &Index,
    skeleton: &str,
    id: ObjectId,
    binding: &BindingState,
    geometry: &CharacterGeometry,
    pool: &mut Value,
) -> Result<usize> {
    let (avatar, keys) = BoundAvatar::restore_with_mesh_keys(index, skeleton, id as u64, binding)?;
    ensure!(
        keys.len() == geometry.appearances.len(),
        "shared appearance/key coverage differs"
    );
    let mut count = 0;
    for (&identity, keys) in &keys {
        let ordinal = *geometry
            .appearances
            .get(&identity)
            .context("shared mesh appearance absent")?;
        let bindings = geometry
            .state
            .appearances
            .get(ordinal)
            .context("shared appearance bindings absent")?;
        let range = avatar
            .part_ranges
            .get(&identity)
            .context("shared immutable parts absent")?;
        ensure!(
            bindings.len() == keys.len() && range.len() == keys.len(),
            "shared mesh/part ordinal mismatch"
        );
        for ((&b, key), part) in bindings
            .iter()
            .zip(keys)
            .zip(&avatar.avatar.parts[range.clone()])
        {
            let global = geometry
                .state
                .bindings
                .get(b)
                .context("shared geometry binding absent")?
                .mesh;
            let local = sims_sim::pose::mesh::LocalBounds::from_mesh(&part.mesh)
                .map_err(anyhow::Error::msg)?;
            ensure!(
                serde_json::to_value(&local.local)? == pool["meshes"][global]["local"],
                "retained mesh local bounds differ from its immutable prototype"
            );
            let table = pool["prototypes"]
                .as_object_mut()
                .context("shared prototype table absent")?;
            ensure!(
                table
                    .insert(key.clone(), json!(global))
                    .is_none_or(|old| old == global),
                "distinct native mesh owners alias one live prototype key"
            );
            count += 1;
        }
    }
    Ok(count)
}

/// Join only source-qualified retained geometry. Native pointer identity owns
/// deduplication; this does not manufacture any other process host owner.
pub(super) fn join_geometry(
    assets: &BTreeMap<ObjectId, SavedAssets>,
    geometry: &super::completed_residents::Geometry,
    ordinary: &completed_ordinary::live::ImportedLive,
    index: &Index,
) -> Result<(
    graphic_loading::Saved,
    MeshPool,
    Vec<u32>,
    Vec<usize>,
    usize,
)> {
    let (mut shared, maps, native_meshes) = pool(&[
        (&geometry.meshes, &geometry.native_meshes),
        (
            &ordinary.ordinary.geometry.meshes,
            &ordinary.ordinary.geometry.native_meshes,
        ),
    ])?;
    ensure!(
        maps[0].iter().copied().eq(0..maps[0].len()),
        "first retained pool changes resident mesh IDs"
    );
    let mut generic = serde_json::to_value(&ordinary.registry)?;
    ensure!(
        generic["jobs"].as_object().is_some_and(|v| v.is_empty()),
        "cannot remap active ordinary construction"
    );
    ensure!(
        ordinary.registry.retirements.is_empty() && ordinary.registry.retained.is_empty(),
        "cannot remap pending ordinary retirement/retained frame owners"
    );
    let mut aliases = 0;
    for (&id, a) in assets {
        aliases += alias(index, &a.skeleton, id, &a.binding, &a.geometry, &mut shared)?;
    }
    for (id, g) in generic["graphics"]
        .as_object_mut()
        .context("ordinary current graphics absent")?
    {
        let id: ObjectId = id.parse()?;
        let geometry = g["geometry"]["state"]["bindings"]
            .as_array_mut()
            .context("ordinary mesh bindings absent")?;
        for binding in geometry {
            let old = usize::try_from(
                binding["mesh"]
                    .as_u64()
                    .context("ordinary mesh index absent")?,
            )?;
            binding["mesh"] = json!(
                *maps[1]
                    .get(old)
                    .context("ordinary mesh outside source pool")?
            );
        }
        let geometry: CharacterGeometry = exact(g["geometry"].clone())?;
        let binding: BindingState = exact(g["binding"].clone())?;
        aliases += alias(
            index,
            g["skeleton"]
                .as_str()
                .context("ordinary source skeleton absent")?,
            id,
            &binding,
            &geometry,
            &mut shared,
        )?;
    }
    let ordinary: graphic_loading::Saved = exact(generic)?;
    let restored = graphic_loading::Registry::restore(index, ordinary.clone())?;
    ensure!(
        restored.snapshot() == ordinary,
        "shared ordinary cold restore altered owners"
    );
    let mesh_pool: MeshPool = exact(shared)?;
    Ok((ordinary, mesh_pool, native_meshes, maps[1].clone(), aliases))
}

pub struct Input<'a> {
    pub appearances: &'a Value,
    pub ordinary: &'a completed_ordinary::live::ImportedLive,
    pub simulation: &'a completed_simulation::compose::Composed,
    pub program: &'a Program,
    pub index: &'a Index,
    pub clothing: &'a Value,
    pub global_provider: &'a Value,
    pub global_locator: &'a str,
}
pub fn import(input: Input<'_>) -> Result<Joined> {
    let Input {
        appearances: v,
        ordinary,
        simulation,
        program,
        index,
        clothing: clothing_source,
        global_provider,
        global_locator,
    } = input;
    let clothing = clothing::import(clothing_source)?;
    let autonomy_resources = global::import(global_provider, index, global_locator)?;
    ensure!(
        v["schema"] == 1
            && v["tick"] == 88448
            && v["contract"] == "completed-native-appearance-bindings-component-v1"
            && v["native_parity_ticks"] == 0
            && v["provenance"] == v["prototypes_input"]["provenance"],
        "shared asset appearance profile differs"
    );
    for k in ["image_sha256", "observation_sha256"] {
        ensure!(
            v["provenance"][k] == ordinary.ordinary.source["provenance"][k],
            "shared asset source epoch differs"
        );
    }
    let prototypes =
        crate::conformance::completed_prototypes::import_parts(&v["prototypes_input"])?;
    let assets = completed_appearances::import(
        &v["prototypes_input"]["resident_input"]["resident_input"],
        &v["appearance_code"],
        prototypes,
        index,
    )?;
    let current = &assets.characters.characters;
    let mut snapshot = simulation.snapshot.clone();
    let lot = snapshot
        .state
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("retained floor coverings absent")?;
    let floors = sims_content::character_loading::floors::from_builtin_coverings(
        index,
        lot.floors
            .iter()
            .flat_map(|f| f.source.coverings.iter().copied()),
    )?;
    sims_content::character_loading::floors::restore(index, &floors)?;
    ensure!(
        snapshot.state.vm.tick == 88448
            && snapshot
                .state
                .characters
                .keys()
                .eq(current.residents.keys()),
        "shared asset/current Simulation coverage differs"
    );
    let vm_before = serde_json::to_vec(&snapshot.state.vm)?;
    for (&id, qualified) in &current.residents {
        let old = &snapshot.state.characters[&id];
        let mut linked = old.clone();
        ensure!(
            old.source
                .as_ref()
                .is_none_or(|source| Some(source) == qualified.source.as_ref()),
            "contradictory immutable skeleton owner"
        );
        linked.source = qualified.source.clone();
        ensure!(
            &linked == qualified,
            "source-bound Resident differs from current Simulation"
        );
        snapshot.state.characters.insert(id, linked);
    }
    ensure!(
        snapshot
            .state
            .retained_ordinary_graphics
            .iter()
            .eq(ordinary.registry.graphics.keys()),
        "shared ordinary/current Simulation coverage differs"
    );
    let (ordinary, mesh_pool, native_meshes, ordinary_remap, aliases) =
        join_geometry(&assets.saved, &current.geometry, ordinary, index)?;
    let settings = current
        .residents
        .values()
        .next()
        .context("initializer settings absent")?
        .controls
        .settings;
    ensure!(
        current
            .residents
            .values()
            .all(|r| r.controls.settings == settings),
        "initializer settings disagree"
    );
    let restored = snapshot
        .clone()
        .restore(program)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        restored == snapshot.state && serde_json::to_vec(&restored.vm)? == vm_before,
        "asset join altered VM/RNG or existing current state"
    );
    Ok(Joined {
        simulation: snapshot,
        characters: assets.saved,
        ordinary,
        mesh_pool,
        skeletons: assets.characters.skeletons,
        initializer_settings: settings,
        clothing,
        autonomy_resources,
        floors,
        complete_native_host_admitted: false,
        source: json!({"schema":1,"tick":88448,"native_parity_ticks":0,
            "native_meshes":native_meshes,"ordinary_mesh_remap":ordinary_remap,"live_prototype_aliases":aliases,
            "provenance":v["provenance"],"immutable_sources":assets.immutable_sources,
            "source_skeletons_linked_without_changing_current_residents":true,
            "vm_rng_unchanged":true,"mesh_bounds_generations_unchanged":true,
            "clothing":{"bodies":2186,"textures":4431,"current_cache_not_reenumerated":true},
            "Global":{"resident_allocations":47,"current_stream_cursor":16238,"loads_or_lookups_executed":0,
                "diagnostic_and_detached_buffer_domain":"new unpublished Rust service epoch"},
            "remaining":["neighborhood source binding",
                "portable notification presenter epoch","complete Host/Session and source-file epoch admission"]}),
    })
}
