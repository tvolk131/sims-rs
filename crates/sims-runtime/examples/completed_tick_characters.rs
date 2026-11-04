//! Bit-preserving character component diagnostic, not full Session admission.
use anyhow::{Context, Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved, completed_animations, completed_geometry, completed_skeletons, completed_skin,
};
use std::{collections::BTreeMap, fs, path::Path};
#[path = "completed_tick_characters/ordinary.rs"]
mod ordinary;

fn same<T: Serialize>(field: &str, native: &T, rust: &T, rows: &mut Vec<Value>) -> Result<()> {
    let native_bytes = postcard::to_allocvec(native)?;
    let rust_bytes = postcard::to_allocvec(rust)?;
    if native_bytes == rust_bytes {
        rows.push(json!({"field":field,"native_sha256":sims_fixtures::digest(&native_bytes),"rust_sha256":sims_fixtures::digest(&rust_bytes),"matched":true}));
    } else {
        fn visit(a: &Value, b: &Value, path: String, out: &mut Vec<Value>) {
            if a == b || out.len() >= 20 {
                return;
            }
            match (a, b) {
                (Value::Object(a), Value::Object(b)) => {
                    let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
                    for k in keys {
                        visit(
                            a.get(k).unwrap_or(&Value::Null),
                            b.get(k).unwrap_or(&Value::Null),
                            format!("{path}/{k}"),
                            out,
                        )
                    }
                }
                (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
                    for (i, (a, b)) in a.iter().zip(b).enumerate() {
                        visit(a, b, format!("{path}/{i}"), out)
                    }
                }
                _ => out.push(json!({"path":path,"native":a,"rust":b})),
            }
        }
        let mut first = vec![];
        visit(
            &serde_json::to_value(native)?,
            &serde_json::to_value(rust)?,
            String::new(),
            &mut first,
        );
        rows.push(json!({"field":field,"native_sha256":sims_fixtures::digest(&native_bytes),"rust_sha256":sims_fixtures::digest(&rust_bytes),"first_differences":first,"first_byte_offset":native_bytes.iter().zip(&rust_bytes).position(|(a,b)|a!=b)}));
    }
    Ok(())
}
fn projected<T: Serialize + DeserializeOwned>(
    field: &str,
    source: Value,
    rust: &T,
    rows: &mut Vec<Value>,
) -> Result<()> {
    let expected: T = serde_json::from_value(source.clone())?;
    ensure!(
        serde_json::to_value(&expected)? == source,
        "native projection defaulted or discarded fields: {field}"
    );
    same(field, &expected, rust, rows)
}
fn raw(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("native scalar bytes absent")?)
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("short native scalar owner")?
            .try_into()?,
    ))
}
fn int(b: &[u8], o: usize) -> Result<i32> {
    Ok(word(b, o)? as i32)
}
fn floats<const N: usize>(b: &[u8], o: usize) -> Result<[f32; N]> {
    let mut out = [0.; N];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f32::from_bits(word(b, o + 4 * i)?);
        ensure!(v.is_finite(), "nonfinite native scalar");
    }
    Ok(out)
}
fn observed_mesh_caches(source: &Value) -> Result<Vec<sims_sim::pose::mesh::MeshBounds>> {
    let mut seen = BTreeMap::new();
    let mut caches = vec![];
    for g in [&source["geometry"], &source["ordinary"]["geometry"]] {
        ensure!(
            g.is_object(),
            "complete shared mesh diagnostic requires people and ordinary geometry owners"
        );
        for m in g["meshes"].as_array().context("native meshes absent")? {
            let pointer = m["pointer"]
                .as_u64()
                .context("native mesh identity absent")?;
            let mr = raw(&m["raw"])?;
            let vector = |name: &str, offset: usize, stride: usize| -> Result<Vec<u8>> {
                let v = &m[name];
                let bytes = raw(&v["used_raw"])?;
                ensure!(
                    v["begin"] == word(&mr, offset)?
                        && v["end"] == word(&mr, offset + 4)?
                        && v["capacity"] == word(&mr, offset + 8)?
                        && word(&mr, offset + 4)?.checked_sub(word(&mr, offset)?)
                            == Some(bytes.len() as u32)
                        && bytes.len() % stride == 0,
                    "native mesh vector owner differs"
                );
                Ok(bytes)
            };
            let br = vector("bindings", 0x40, 24)?;
            let lb = vector("local_bounds", 0x70, 24)?;
            let bounds = |b: &[u8], o| -> Result<sims_sim::pose::model::Bounds> {
                Ok(sims_sim::pose::model::Bounds {
                    minimum: floats(b, o)?,
                    maximum: floats(b, o + 12)?,
                })
            };
            let cache = sims_sim::pose::mesh::MeshBounds {
                local: lb
                    .chunks_exact(24)
                    .map(|b| bounds(b, 0))
                    .collect::<Result<_>>()?,
                stamps: br
                    .chunks_exact(24)
                    .map(|b| word(b, 20))
                    .collect::<Result<_>>()?,
                bounds: bounds(&mr, 0x94)?,
            };
            ensure!(
                cache.local.len() == cache.stamps.len(),
                "native mesh local/cache lengths differ"
            );
            if let Some(&old) = seen.get(&pointer) {
                ensure!(caches[old] == cache, "aliased native mesh caches disagree");
            } else {
                seen.insert(pointer, caches.len());
                caches.push(cache);
            }
        }
    }
    Ok(caches)
}
fn resident_scalars(
    source: &Value,
    id: sims_vm::ObjectId,
    native: &completed_skeletons::Imported,
    rust: &sims_sim::character::Resident,
    model: &sims_content::character_model::State,
    tick: u64,
    differences: &mut Vec<Value>,
) -> Result<()> {
    let source = &source["resident"];
    if !source.is_object() {
        return Ok(());
    }
    let a = source["actors"]
        .as_array()
        .context("resident actors absent")?
        .iter()
        .find(|a| a["object"] == id)
        .context("native scalar actor absent")?;
    let ar = raw(&a["animator_raw"])?;
    ensure!(
        ar.len() == 0x2a0
            && word(&ar, 8)? == native.native.model
            && word(&ar, 0x138)? == native.native.person,
        "native resident aliases differ"
    );
    projected(
        &format!("{id}/head"),
        json!({"last_clock":word(&ar,0x164)?,"basis_heading":floats::<1>(&ar,0x210)?[0],
        "target_heading":floats::<1>(&ar,0x214)?[0],"turn":floats::<1>(&ar,0x218)?[0],"target_point":floats::<3>(&ar,0x21c)?}),
        &rust.character.head,
        differences,
    )?;
    projected(
        &format!("{id}/placement"),
        json!({"base":floats::<3>(&ar,0x80)?,"previous":floats::<3>(&ar,0x1c4)?,
        "offset":floats::<3>(&ar,0x8c)?,"dirty":ar[0xd9]!=0}),
        &rust.character.placement,
        differences,
    )?;
    ensure!(
        word(&ar, 0x250)? == 0,
        "native censor binding owner requires projection"
    );
    projected(
        &format!("{id}/censor"),
        json!({"binding":null,"mask":int(&ar,0x254)?}),
        &rust.character.censor,
        differences,
    )?;
    same(
        &format!("{id}/clock"),
        &int(&ar, 0xb4)?,
        &rust.character.clock,
        differences,
    )?;
    same(
        &format!("{id}/visible"),
        &(ar[0x109] != 0),
        &rust.character.visible,
        differences,
    )?;
    same(
        &format!("{id}/profile_dirty"),
        &(ar[0xd8] != 0),
        &rust.profile_dirty,
        differences,
    )?;
    ensure!(
        sims_sim::pose::ordinary::clock((tick - 1) as i32, ar[0xa8]) == int(&ar, 0xb4)?,
        "native completed clock cut differs"
    );
    same(
        &format!("{id}/evaluated_tick"),
        &(tick - 1),
        &rust.evaluated_tick,
        differences,
    )?;
    projected(
        &format!("{id}/context"),
        a["property_context"].clone(),
        &rust.context,
        differences,
    )?;
    projected(
        &format!("{id}/ordinary"),
        json!({"clock_mode":ar[0xa8],"base_posture":int(&ar,0x198)?,"pickup":int(&ar,0x1a4)?,
        "carry":{"mode":int(&ar,0x1a8)?,"selection":int(&ar,0x1ac)?},"mode":int(&ar,0x190)?,
        "rate":floats::<1>(&ar,0x228)?[0],"target_rate":floats::<1>(&ar,0x22c)?[0]}),
        &rust.controls.ordinary,
        differences,
    )?;
    projected(
        &format!("{id}/movement_state"),
        json!({"status":int(&ar,0x188)?,"flags":[int(&ar,0x19c)?,int(&ar,0x290)?],
        "start":floats::<3>(&ar,0x1d0)?,"end":floats::<3>(&ar,0x1dc)?,"turn_delta":floats::<1>(&ar,0x1f0)?[0],
        "nominal_turn":floats::<1>(&ar,0x1f4)?[0],"reason":int(&ar,0x288)?,"apply_rate":ar[0x29c]}),
        &rust.controls.movement.state,
        differences,
    )?;
    same(
        &format!("{id}/movement_scalar"),
        &floats::<1>(&ar, 0x7c)?[0],
        &rust.controls.movement.scalar_7c,
        differences,
    )?;
    let mut anchors = [None; 2];
    for (j, o) in [0x168, 0x16c].into_iter().enumerate() {
        let p = word(&ar, o)?;
        if p != 0 {
            anchors[j] = Some(
                native
                    .native
                    .bone_pointers
                    .iter()
                    .position(|&b| b == p)
                    .context("movement anchor missing")?,
            );
        }
    }
    same(
        &format!("{id}/movement_anchors"),
        &anchors,
        &rust.controls.movement_anchors,
        differences,
    )?;
    same(
        &format!("{id}/engaged"),
        &int(&ar, 0xa0)?,
        &rust.controls.engaged,
        differences,
    )?;
    same(
        &format!("{id}/animation_events"),
        &int(&ar, 0xa4)?,
        &rust.controls.animation_events,
        differences,
    )?;
    same(
        &format!("{id}/previous_mode"),
        &int(&ar, 0x194)?,
        &model.previous_mode,
        differences,
    )?;
    same(
        &format!("{id}/phase_window"),
        &[floats::<1>(&ar, 0x174)?[0], floats::<1>(&ar, 0x170)?[0]],
        &model.phase_window,
        differences,
    )?;
    same(
        &format!("{id}/stop_distance"),
        &floats::<2>(&ar, 0x178)?,
        &model.stop_distance,
        differences,
    )?;
    same(
        &format!("{id}/clip_distances"),
        &floats::<6>(&ar, 0x1f8)?,
        &model.clip_distances,
        differences,
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 5,
        "usage: completed_tick_characters STORE REFS_JSON REF_FIELD NATIVE_PARTS_JSON NEW_DIR"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let saved: Saved =
        Store::open(&args[0])?.get_value(&serde_json::from_value(refs[&args[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&args[3])?)?;
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick,
        "character source tick differs"
    );
    let animation = &source["animation"];
    let imported = completed_animations::import(
        &serde_json::from_value::<Vec<completed_animations::AssetInput>>(
            animation["payload"]["assets"].clone(),
        )?,
        &serde_json::from_value::<Vec<completed_animations::CharacterInput>>(
            animation["characters"].clone(),
        )?,
        source["epoch_sha256"]
            .as_str()
            .context("character source epoch absent")?,
    )?;
    let animators: BTreeMap<_, _> = imported
        .characters
        .into_iter()
        .map(|(id, c)| (id, c.animator))
        .collect();
    let parts = completed_skeletons::import(
        &serde_json::from_value::<Vec<completed_skeletons::Input>>(source["rows"].clone())?,
        animators,
        "goth-completed88448-character-component-diagnostic",
    )?;
    ensure!(
        parts.keys().eq(saved.simulation.state.characters.keys()),
        "character inventory differs"
    );
    let geometries = if source["geometry"].is_object() {
        let bindings = parts
            .iter()
            .map(|(&id, p)| {
                (
                    id,
                    completed_geometry::Skin {
                        native: &p.native,
                        bone_vertex_bounds: &p.bone_vertex_bounds,
                    },
                )
            })
            .collect();
        Some(completed_geometry::import_with_bindings(
            &source["geometry"],
            &bindings,
            "goth-observed-endpoint-geometry",
        )?)
    } else {
        None
    };
    let mut differences = vec![];
    if let Some(geometry) = &geometries {
        if source["ordinary"].is_object() {
            same(
                "shared_mesh_caches",
                &observed_mesh_caches(&source)?,
                &saved.assets.mesh_pool.meshes,
                &mut differences,
            )?;
        } else {
            same(
                "shared_mesh_caches",
                &geometry.meshes.meshes,
                &saved.assets.mesh_pool.meshes,
                &mut differences,
            )?;
        }
    }
    for (id, mut native) in parts {
        if let Some(control) = source["fp_control"].as_u64() {
            native
                .animator
                .bind_fp_control(control.try_into()?)
                .map_err(anyhow::Error::msg)?;
        }
        let rust = &saved.simulation.state.characters[&id].character;
        same(
            &format!("{id}/animator"),
            &native.animator,
            &rust.skin.animator,
            &mut differences,
        )?;
        if let Some(geometry) = &geometries {
            let g = &geometry.characters[&id];
            let assets = &saved.assets.characters[&id];
            let model = &assets.model;
            same(
                &format!("{id}/appearance_geometry"),
                &g.geometry.state,
                &assets.geometry.state,
                &mut differences,
            )?;
            resident_scalars(
                &source,
                id,
                &native,
                &saved.simulation.state.characters[&id],
                model,
                saved.simulation.state.vm.tick,
                &mut differences,
            )?;
            let observed = sims_sim::pose::skeleton::Geometry {
                dirty: model.dirty,
                generation: saved.simulation.state.characters[&id]
                    .controls
                    .geometry_generation,
                evaluated_generation: model.evaluated_generation,
                bounds_generation: model.bounds_generation,
                bone_generations: model.bone_generations.clone(),
            };
            same(
                &format!("{id}/geometry_stamps"),
                &native.geometry,
                &observed,
                &mut differences,
            )?;
            same(
                &format!("{id}/wiggle"),
                &native.wiggle,
                &model.wiggle,
                &mut differences,
            )?;
            same(
                &format!("{id}/bone_flags"),
                &native.flags,
                &model.flags,
                &mut differences,
            )?;
            same(
                &format!("{id}/named_bones"),
                &native.named_bone_flags,
                &model.named_bone_flags,
                &mut differences,
            )?;
            same(
                &format!("{id}/bone_bounds"),
                &native.bone_bounds,
                &model.bone_bounds,
                &mut differences,
            )?;
            same(
                &format!("{id}/bone_bounds_stamps"),
                &native.bone_bounds_generations,
                &model.bone_bounds_generations,
                &mut differences,
            )?;
            same(
                &format!("{id}/bone_vertex_bounds"),
                &native.bone_vertex_bounds,
                &assets.geometry.state.bone_vertex_bounds,
                &mut differences,
            )?;
            same(
                &format!("{id}/local_bounds"),
                &native.local_bounds,
                &model.local_bounds,
                &mut differences,
            )?;
            same(
                &format!("{id}/render_bounds"),
                &g.render_bounds,
                &model.render_bounds,
                &mut differences,
            )?;
            same(
                &format!("{id}/model"),
                &g.model,
                &rust.model,
                &mut differences,
            )?;
            same(
                &format!("{id}/matrix"),
                &g.matrix.values,
                &rust.matrix,
                &mut differences,
            )?;
            same(
                &format!("{id}/rig"),
                &completed_skin::rig_from_parts(&native)?,
                &rust.rig,
                &mut differences,
            )?;
        }
        same(
            &format!("{id}/bones"),
            &native.bones,
            &rust.skin.bones,
            &mut differences,
        )?;
        same(
            &format!("{id}/hierarchy"),
            &native.hierarchy,
            &rust.hierarchy,
            &mut differences,
        )?;
    }
    ordinary::compare(&source, &saved, &mut differences)?;
    let output = Path::new(&args[4]);
    fs::create_dir(output)?;
    let comparisons = differences;
    let differences: Vec<_> = comparisons
        .iter()
        .filter(|v| v["matched"] != true)
        .cloned()
        .collect();
    let report = json!({"comparison_count":comparisons.len(),"comparisons":comparisons,"scope":if geometries.is_some(){"retained Animator/bones/hierarchy, Resident motion/head/controls, rig recipe, complete skeleton geometry/cache stamps, ordinary dependent graphics and model/placement/bounds; frontend and other simulation owners are separate"}else{"complete retained Animator, sampled bone and hierarchy component bytes; pointers and model/rig/frontend owners are outside this diagnostic"}, "difference_count":differences.len(),"differences":differences,"native_parity_ticks":0});
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    ensure!(
        report["difference_count"] == 0,
        "retained character components differ"
    );
    Ok(())
}
