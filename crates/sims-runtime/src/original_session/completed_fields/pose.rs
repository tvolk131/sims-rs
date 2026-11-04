//! Actual completed pose, movement and CPU geometry fields, without a VM/source
//! reconstruction or asset constructor. Native mesh identities join shared pools.
use super::*;
use crate::original_session::{
    completed_animations as anim, completed_geometry as geo, completed_routes,
    completed_skeletons as skeleton,
};
use sims_content::{character_geometry::MeshPool, character_loading::live::SavedAssets};
use sims_fixtures::readset::Readset;

pub(super) fn differences(
    path: &str,
    a: &Value,
    b: &Value,
    checked: &mut usize,
    total: &mut usize,
    first: &mut Vec<Value>,
) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) if a.keys().eq(b.keys()) => {
            for (k, v) in a {
                differences(&format!("{path}/{k}"), v, &b[k], checked, total, first);
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}/{i}"), a, b, checked, total, first);
            }
        }
        _ => {
            *checked += 1;
            if a != b {
                *total += 1;
                if first.len() < 100 {
                    first.push(json!({"path":path,"native":a,"rust":b}));
                }
            }
        }
    }
}
fn geometry(
    value: &sims_content::character_geometry::CharacterGeometry,
    pointers: &[u32],
) -> Result<Value> {
    let mut v = serde_json::to_value(value)?;
    for binding in v["state"]["bindings"]
        .as_array_mut()
        .context("geometry bindings absent")?
    {
        let index: usize = binding["mesh"]
            .as_u64()
            .context("geometry mesh index absent")?
            .try_into()?;
        binding["mesh"] = json!(
            *pointers
                .get(index)
                .context("geometry mesh identity unobserved")?
        );
    }
    // Fresh appearance handles are adapter identities. Ordered appearance and
    // binding vectors remain intact; aliases become their actual vector ordinal.
    let mut appearances = serde_json::Map::new();
    for ordinal in value.appearances.values() {
        ensure!(
            appearances
                .insert(ordinal.to_string(), json!(ordinal))
                .is_none(),
            "geometry appearance aliases share an ordinal"
        );
    }
    v["appearances"] = Value::Object(appearances);
    Ok(v)
}

pub fn compare(
    state: &Value,
    memory: &Readset<'_>,
    simulation: &sims_sim::Simulation,
    assets: &BTreeMap<sims_vm::ObjectId, SavedAssets>,
    pool: &MeshPool,
    pointers: &[u32],
    epoch: &str,
) -> Result<Value> {
    ensure!(
        state["tick"] == simulation.vm.tick && state["readset"]["sha256"] == memory.source_sha256(),
        "pose comparison epoch differs"
    );
    let movement_code = sims_fixtures::digest(&memory.read(0x58ede3, 149)?);
    ensure!(
        movement_code == "f66858131d28bb13b8b8760d8f1108c370e779ec1ebea675c220f62e2929bfc5"
            && memory.read(0x668390, 4)? == 0.33333334_f32.to_le_bytes()
            && memory.read(0x666a54, 4)? == 16_f32.to_le_bytes(),
        "current native movement arithmetic source differs"
    );
    let source = &state["domains"]["animations"];
    skeleton::captured::reads(&source["payload"], memory)?;
    let animation = anim::import_with_resident_lists(
        &serde_json::from_value::<Vec<anim::AssetInput>>(source["payload"]["assets"].clone())?,
        &serde_json::from_value::<Vec<anim::CharacterInput>>(source["characters"].clone())?,
        epoch,
        &source["payload"]["resident_lists"],
        &state["characters"]["event_cursors"],
        memory,
    )?;
    let parts = skeleton::captured::import(
        state,
        animation
            .characters
            .into_iter()
            .map(|(id, c)| (id, c.animator))
            .collect(),
        memory,
        epoch,
    )?;
    skeleton::captured::reads(&state["domains"]["geometry"], memory)?;
    skeleton::captured::reads(&state["domains"]["residents"], memory)?;
    let bindings = parts
        .iter()
        .map(|(&id, p)| {
            (
                id,
                geo::Skin {
                    native: &p.native,
                    bone_vertex_bounds: &p.bone_vertex_bounds,
                },
            )
        })
        .collect();
    let native_geometry =
        geo::import_with_bindings(&state["domains"]["geometry"], &bindings, epoch)?;
    ensure!(
        parts.keys().eq(simulation.characters.keys()) && parts.keys().eq(assets.keys()),
        "pose owner domain differs"
    );
    ensure!(
        pointers.len() == pool.meshes.len()
            && pointers.iter().copied().collect::<BTreeSet<_>>().len() == pointers.len(),
        "runtime mesh identity domain differs"
    );
    let mut checked = 0;
    let mut total = 0;
    let mut first = Vec::new();
    let mut groups=Vec::new();
    let mut compare = |path: String, mut a: Value, mut b: Value| {
        animation::float_bits(&mut a);
        animation::float_bits(&mut b);
        let before=(checked,total);
        differences(&path, &a, &b, &mut checked, &mut total, &mut first);
        groups.push(json!({"path":path,"checks":checked-before.0,"differences":total-before.1}));
    };
    for (&id, p) in &parts {
        let r = &simulation.characters[&id];
        let a = &assets[&id];
        let g = &native_geometry.characters[&id];
        for (name, expected, actual) in [
            (
                "bones",
                serde_json::to_value(&p.bones)?,
                serde_json::to_value(&r.character.skin.bones)?,
            ),
            (
                "hierarchy",
                serde_json::to_value(&p.hierarchy)?,
                serde_json::to_value(&r.character.hierarchy)?,
            ),
            (
                "wiggle",
                serde_json::to_value(&p.wiggle)?,
                serde_json::to_value(&a.model.wiggle)?,
            ),
            ("bone_flags", json!(p.flags), json!(a.model.flags)),
            (
                "named_bone_flags",
                json!(p.named_bone_flags),
                json!(a.model.named_bone_flags),
            ),
            (
                "geometry_bone_generations",
                json!(p.geometry.bone_generations),
                json!(a.model.bone_generations),
            ),
            (
                "geometry_evaluated_generation",
                json!(p.geometry.evaluated_generation),
                json!(a.model.evaluated_generation),
            ),
            (
                "geometry_bounds_generation",
                json!(p.geometry.bounds_generation),
                json!(a.model.bounds_generation),
            ),
            (
                "geometry_dirty",
                json!(p.geometry.dirty),
                json!(a.model.dirty),
            ),
            (
                "bone_bounds",
                json!(p.bone_bounds),
                json!(a.model.bone_bounds),
            ),
            (
                "bone_bounds_generations",
                json!(p.bone_bounds_generations),
                json!(a.model.bone_bounds_generations),
            ),
            (
                "local_bounds",
                json!(g.local_bounds),
                json!(a.model.local_bounds),
            ),
            (
                "render_bounds",
                json!(g.render_bounds),
                json!(a.model.render_bounds),
            ),
            ("model", json!(g.model), json!(r.character.model)),
            (
                "published_matrix",
                json!(g.matrix.values),
                json!(r.character.matrix),
            ),
            (
                "geometry",
                geometry(&g.geometry, &native_geometry.native_meshes)?,
                geometry(&a.geometry, pointers)?,
            ),
        ] {
            compare(format!("/people/{id}/{name}"), expected, actual);
        }
        let actor = state["domains"]["residents"]["actors"]
            .as_array()
            .context("resident rows absent")?
            .iter()
            .find(|a| a["object"] == id)
            .context("resident row missing")?;
        let raw = geo::bytes(actor, "animator_raw")?;
        ensure!(
            raw.len() == 0x2a0
                && geo::word(actor, "person")? == p.native.person
                && geo::word(actor, "animator")? == p.native.animator
                && memory.read(p.native.animator, raw.len())? == raw,
            "pose movement actor alias differs"
        );
        let vector = |offset: usize| -> Result<[f32; 3]> {
            Ok([
                f32::from_bits(geo::u(&raw, offset)?),
                f32::from_bits(geo::u(&raw, offset + 4)?),
                f32::from_bits(geo::u(&raw, offset + 8)?),
            ])
        };
        let placement = sims_sim::pose::transform::Placement {
            base: vector(0x80)?,
            previous: vector(0x1c4)?,
            offset: vector(0x8c)?,
            dirty: raw[0xd9] != 0,
        };
        compare(
            format!("/people/{id}/placement_cache"),
            json!(placement),
            json!(r.character.placement),
        );
        let mut points = Vec::new();
        if !actor["path"].is_null() {
            let path = &actor["path"];
            let header = geo::bytes(path, "raw")?;
            ensure!(
                header.len() == 12 && memory.read(geo::word(path, "pointer")?, 12)? == header,
                "native movement path header differs"
            );
            let bytes = geo::bytes(&path["points"], "used_raw")?;
            ensure!(
                bytes.len() % 8 == 0
                    && geo::u(&header, 4)?.checked_sub(geo::u(&header, 0)?)
                        == Some(bytes.len() as u32)
                    && memory.read(geo::u(&header, 0)?, bytes.len())? == bytes,
                "native movement points differ"
            );
            for p in bytes.chunks_exact(8) {
                points.push([geo::u(p, 0)? as i32, geo::u(p, 4)? as i32]);
            }
        }
        let native = completed_routes::movement(memory, p.native.person, &raw, &points)?;
        compare(
            format!("/people/{id}/movement"),
            json!(native),
            json!(r.controls.movement),
        );
    }
    for (i, &pointer) in native_geometry.native_meshes.iter().enumerate() {
        let index = pointers
            .iter()
            .position(|&p| p == pointer)
            .context("native mesh absent from runtime pool")?;
        compare(
            format!("/meshes/{pointer:08x}"),
            json!(native_geometry.meshes.meshes[i]),
            json!(pool.meshes[index]),
        );
    }
    Ok(
        json!({"schema":1,"kind":"observed_pose_geometry_movement_diagnostic","native_parity_credit":0,
        "movement_code_sha256":movement_code,"checks":checked,"differences":total,"first_differences":first,"groups":groups,
        "normalization":"Only unpublished appearance and mesh indices. Native floats compare as f32 bits; ordered storage is retained.",
        "qualification":"Selected current pose/movement/geometry fields. Route continuation/callers, links, ordinary graphics and full Session remain separate."}),
    )
}
