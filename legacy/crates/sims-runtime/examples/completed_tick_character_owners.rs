//! Native nested owner diagnostic. It does not certify complete Session parity.
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sims_fixtures::Store;
use sims_runtime::original_session::{
    Saved, completed_animations, completed_geometry, completed_skeletons,
};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_reload::Owner,
    appearance_selection::State,
    movement::path,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
#[path = "completed_tick_character_owners/meshes.rs"]
mod meshes;
#[path = "completed_tick_character_owners/profiles.rs"]
mod profiles;

fn raw(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("native bytes absent")?)
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("native owner is short")?
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
        ensure!(v.is_finite(), "nonfinite native owner");
    }
    Ok(out)
}
fn text(v: &Value) -> Result<String> {
    let b = raw(&v["used_raw"])?;
    ensure!(
        b.is_ascii() && !b.contains(&0),
        "unsupported native narrow name"
    );
    Ok(String::from_utf8(b)?)
}
fn vector(actor: &Value, ar: &[u8], o: usize) -> Result<Vec<u32>> {
    let v = &actor["vectors"][o.to_string()];
    let b = raw(&v["used_raw"])?;
    ensure!(
        v["begin"] == word(ar, o)?
            && v["end"] == word(ar, o + 4)?
            && v["capacity"] == word(ar, o + 8)?
            && word(ar, o + 4)?.checked_sub(word(ar, o)?) == Some(b.len() as u32)
            && b.len() % 4 == 0,
        "native appearance vector alias/extent differs"
    );
    b.chunks_exact(4).map(|r| word(r, 0)).collect()
}
fn binding(
    actor: &Value,
    ar: &[u8],
    id: i16,
    g: &completed_geometry::Character,
) -> Result<sims_content::saved_avatar::BindingState> {
    let generic = vector(actor, ar, 0xc0)?;
    let body = vector(actor, ar, 0x25c)?;
    let hands = vector(actor, ar, 0x268)?;
    let accessories = vector(actor, ar, 0x274)?;
    ensure!(
        generic.len() == 1 && body.len() == 2 && hands.len() == 2,
        "native record role profile differs"
    );
    let head = word(ar, 0x258)?;
    ensure!(
        actor["single_record"] == head,
        "native head role alias differs"
    );
    let mut pointers = vec![generic[0], head, hands[0], hands[1], body[0], body[1]];
    pointers.extend(accessories);
    ensure!(
        pointers.iter().copied().collect::<BTreeSet<_>>().len() == pointers.len(),
        "duplicate native record role"
    );
    let source = actor["records"]
        .as_array()
        .context("native records absent")?;
    ensure!(source.len() == pointers.len(), "unclaimed native record");
    let mut entries = vec![];
    let mut live = BTreeSet::new();
    for pointer in pointers.iter() {
        let r = source
            .iter()
            .find(|r| r["pointer"] == *pointer)
            .context("native role record absent")?;
        let b = raw(&r["raw"])?;
        ensure!(b.len() == 28, "native record size differs");
        for (o, k) in [(4, "name"), (16, "texture")] {
            let v = &r[k];
            for (j, k) in ["begin", "end", "capacity"].into_iter().enumerate() {
                ensure!(
                    v[k] == word(&b, o + 4 * j)?,
                    "native record string alias differs"
                );
            }
            ensure!(
                word(&b, o + 4)?.checked_sub(word(&b, o)?)
                    == Some(raw(&v["used_raw"])?.len() as u32),
                "native string length differs"
            );
        }
        let p = word(&b, 0)?;
        let bound = if p == 0 {
            None
        } else {
            let identity = *g
                .native_appearances
                .get(&p)
                .context("native record has no geometry owner")?;
            ensure!(
                live.insert(identity),
                "native appearance belongs to two records"
            );
            Some(Binding {
                id: identity,
                owner: id as u64,
            })
        };
        entries.push(Record {
            binding: bound,
            name: text(&r["name"])?,
            texture: text(&r["texture"])?,
        });
    }
    ensure!(
        live.len() == g.geometry.state.appearances.len() && word(ar, 0x250)? == 0,
        "native appearance/auxiliary owner differs"
    );
    let mut order: Vec<_> = g
        .geometry
        .appearances
        .iter()
        .map(|(&id, &n)| (n, id))
        .collect();
    order.sort_by_key(|x| x.0);
    let order: Vec<_> = order.into_iter().map(|x| x.1).collect();
    Ok(sims_content::saved_avatar::BindingState {
        records: Records {
            entries,
            generic: 0,
            head: 1,
            hands: [2, 3],
            body: [4, 5],
            accessories: (6..pointers.len()).collect(),
        },
        next_binding: order.iter().max().copied().unwrap_or(0) + 1,
        order,
        auxiliary: BTreeMap::new(),
    })
}
fn native_path(a: &Value, ar: &[u8], resident: &Value) -> Result<Option<path::Path>> {
    if word(ar, 0x184)? == 0 {
        ensure!(a["path"].is_null(), "unowned native path");
        return Ok(None);
    }
    let p = &a["path"];
    let b = raw(&p["raw"])?;
    let v = &p["points"];
    let used = raw(&v["used_raw"])?;
    ensure!(
        p["pointer"] == word(ar, 0x184)?
            && b.len() == 12
            && v["begin"] == word(&b, 0)?
            && v["end"] == word(&b, 4)?
            && v["capacity"] == word(&b, 8)?
            && word(&b, 4)?.checked_sub(word(&b, 0)?) == Some(used.len() as u32)
            && used.len() % 8 == 0,
        "native path ownership differs"
    );
    let t = raw(&resident["settings"][0x6d07a8u32.to_string()])?;
    Ok(Some(path::Path {
        points: used
            .chunks_exact(8)
            .map(|r| Ok([int(r, 0)?, int(r, 4)?]))
            .collect::<Result<_>>()?,
        settings: path::Settings {
            sample_step: floats::<1>(&t, 0)?[0],
            turn_threshold: floats::<1>(&t, 12)?[0],
            turn_step: floats::<1>(&t, 16)?[0],
            minimum_brake_time: int(&t, 44)?,
            endpoint_distance: floats(&t, 48)?,
        },
        index: int(ar, 0x180)?,
        progress: floats::<1>(ar, 0x1ec)?[0],
        deadline: int(ar, 0x1c0)?,
        phase_window: [floats::<1>(ar, 0x174)?[0], floats::<1>(ar, 0x170)?[0]],
        stop_distance: floats(ar, 0x178)?,
        start_distance: [floats::<1>(ar, 0x1f8)?[0], floats::<1>(ar, 0x204)?[0]],
        lookahead: [floats::<1>(ar, 0x200)?[0], floats::<1>(ar, 0x1fc)?[0]],
        interrupt: int(ar, 0x284)?,
        guard: int(ar, 0x28c)?,
        delta: floats(ar, 0x294)?,
        stop_mode: int(ar, 0x18c)?,
    }))
}
fn same<T: Serialize>(
    rows: &mut Vec<Value>,
    id: i16,
    field: &str,
    native: &T,
    rust: &T,
) -> Result<()> {
    let a = postcard::to_allocvec(native)?;
    let b = postcard::to_allocvec(rust)?;
    let mut row = json!({"object":id,"field":field,"matched":a==b,"native_sha256":sims_fixtures::digest(&a),"rust_sha256":sims_fixtures::digest(&b)});
    if a != b {
        row["native"] = serde_json::to_value(native)?;
        row["rust"] = serde_json::to_value(rust)?;
    }
    rows.push(row);
    Ok(())
}
fn observed(
    source: &Value,
    epoch: &str,
) -> Result<(
    BTreeMap<i16, completed_skeletons::Imported>,
    completed_geometry::Imported,
)> {
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
            .context("native epoch absent")?,
    )?;
    let parts = completed_skeletons::import(
        &serde_json::from_value::<Vec<completed_skeletons::Input>>(source["rows"].clone())?,
        imported
            .characters
            .into_iter()
            .map(|(id, c)| (id, c.animator))
            .collect(),
        epoch,
    )?;
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
    let geometry = completed_geometry::import_with_bindings(&source["geometry"], &bindings, epoch)?;
    Ok((parts, geometry))
}
fn evaluate(source: &Value, saved: &Saved) -> Result<Value> {
    ensure!(
        source["tick"] == saved.simulation.state.vm.tick
            && [88448, 88449].contains(&saved.simulation.state.vm.tick),
        "native nested-owner cut differs"
    );
    let (parts, geometry) = observed(source, "goth-native-nested-owner-observer")?;
    ensure!(
        parts.keys().eq(saved.assets.characters.keys())
            && parts.keys().eq(saved.simulation.state.characters.keys()),
        "nested native owner cohort differs"
    );
    let mut rows = vec![];
    for (&id, p) in &parts {
        let a = source["resident"]["actors"]
            .as_array()
            .context("native resident owners absent")?
            .iter()
            .find(|a| a["object"] == id)
            .context("native resident absent")?;
        let ar = raw(&a["animator_raw"])?;
        ensure!(
            ar.len() == 0x2a0
                && a["animator"] == p.native.animator
                && word(&ar, 8)? == p.native.model,
            "native nested owner alias differs"
        );
        let g = &geometry.characters[&id];
        let actual = &saved.assets.characters[&id];
        same(
            &mut rows,
            id,
            "complete_instance_geometry_vertices_and_appearance_map",
            &g.geometry,
            &actual.geometry,
        )?;
        same(
            &mut rows,
            id,
            "complete_record_roles_bindings_and_identity_order",
            &binding(a, &ar, id, g)?,
            &actual.binding,
        )?;
        let owner = Owner {
            selection: State {
                cached: [
                    int(&ar, 0x1b0)?,
                    int(&ar, 0x1b4)?,
                    int(&ar, 0x1b8)?,
                    int(&ar, 0x1bc)?,
                ],
                dirty: ar[0xd9] != 0,
                special_dirty: ar[0x109] != 0,
                skin_dirty: serde_json::to_value(&p.animator)?["dirty"]
                    .as_bool()
                    .context("native animator dirty owner absent")?,
                tint: ar[0xf4..0xf7].try_into()?,
            },
            model_present: word(&ar, 8)? != 0,
            attachment: None,
            attachment_metadata: word(&ar, 0x254)?,
        };
        same(
            &mut rows,
            id,
            "complete_appearance_owner",
            &owner,
            &actual.appearance_owner,
        )?;
        same(
            &mut rows,
            id,
            "retained_path_including_inactive_owned_vector",
            &native_path(a, &ar, &source["resident"])?,
            &saved.simulation.state.characters[&id]
                .controls
                .movement
                .path,
        )?;
        let resource = &a["needs_resource"];
        ensure!(
            resource["pointer"] == word(&ar, 0x24c)?,
            "native needs resource alias differs"
        );
        same(
            &mut rows,
            id,
            "model_needs_appearance",
            &text(&resource["name"])?,
            &actual.model.needs_appearance,
        )?;
    }
    let differences = rows.iter().filter(|r| r["matched"] == false).count();
    Ok(
        json!({"schema":1,"tick":saved.simulation.state.vm.tick,"comparisons":rows,"difference_count":differences,"comparison_count":rows.len(),"complete_native_Session_admitted":false,"native_parity_ticks":0,
        "scope":"Actual full per-person geometry/vertex and instance maps, appearance records/roles/order, selection owner, retained paths and needs resource names. Immutable skeleton and complete shared mesh aliases/lifetimes remain separate obligations."}),
    )
}
fn main() -> Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        a.len() == 5
            || ((a.len() == 8 || (a.len() == 13 && a[8] == "--ordinary-profiles"))
                && a[5] == "--mesh-owners"),
        "usage: completed_tick_character_owners STORE REFS FIELD NATIVE_PARTS_JSON NEW_DIR [--mesh-owners PROTOTYPES IMMUTABLE_ROOT [--ordinary-profiles PROFILES PROOF OBJECT_STORAGE PROVIDER_BINDINGS]]"
    );
    let refs: Value = serde_json::from_slice(&fs::read(&a[1])?)?;
    let saved: Saved =
        Store::open(&a[0])?.get_value(&serde_json::from_value(refs[&a[2]].clone())?)?;
    let source: Value = serde_json::from_slice(&fs::read(&a[3])?)?;
    let mut report = evaluate(&source, &saved)?;
    if a.len() >= 8 {
        let index = sims_content::Index::scan(Path::new(&a[7]))?;
        ensure!(
            !index.resources.iter().any(|r| r
                .locator
                .split('/')
                .any(|p| p.to_ascii_lowercase().starts_with("userdata"))),
            "mesh comparison needs only immutable assets"
        );
        let prototypes: Value = serde_json::from_slice(&fs::read(&a[6])?)?;
        let extra = meshes::evaluate(&source, &saved, &prototypes, &index)?;
        report["comparisons"].as_array_mut().unwrap().extend(extra);
        let rows = report["comparisons"].as_array().unwrap();
        report["comparison_count"] = rows.len().into();
        report["difference_count"] = report["comparisons"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["matched"] == false)
            .count()
            .into();
        report["mesh_owner_extension"] = true.into();
        report["scope"]="Actual person state, complete shared mesh values and all prototype aliases, ordinary full ROOT/model/geometry/record owners. Complete Session admission and source lifetime/caller obligations remain separate.".into();
        if a.len() == 13 {
            let raw = fs::read(&a[9])?;
            let proof: Value = serde_json::from_slice(&fs::read(&a[10])?)?;
            let objects: Value = serde_json::from_slice(&fs::read(&a[11])?)?;
            let bindings: Value = serde_json::from_slice(&fs::read(&a[12])?)?;
            let extra =
                profiles::evaluate(&source, &saved, &raw, &proof, &objects, &bindings, &index)?;
            report["comparisons"].as_array_mut().unwrap().extend(extra);
            report["comparison_count"] = report["comparisons"].as_array().unwrap().len().into();
            report["difference_count"] = report["comparisons"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["matched"] == false)
                .count()
                .into();
            report["ordinary_profile_extension"] = true.into();
            report["replacement_completed_calls_preserved_not_native_fields"] = true.into();
            report["native_device_material_trees_excluded_from_headless_equality"] = true.into();
            report["native_retirement_absence_requires_separate_proof"] = true.into();
        }
    }
    fs::create_dir(Path::new(&a[4]))?;
    fs::write(
        Path::new(&a[4]).join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "comparisons {}, differences {}",
        report["comparison_count"], report["difference_count"]
    );
    ensure!(
        report["difference_count"] == 0,
        "native nested character owners differ"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the private captured Goth native and retained managed sources"]
    fn actual_nested_owners_reject_vertex_record_path_selection_and_needs_changes() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(run)?
            .join()
            .map_err(|_| anyhow::anyhow!("nested character owner test worker panicked"))?
    }
    fn run() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cut = root.join("fixtures/private/goth-label-owners-20261029-first/session-first");
        let refs: Value = serde_json::from_slice(&fs::read(cut.join("refs.json"))?)?;
        let saved: Saved = Store::open(cut.join("store"))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        let source:Value=serde_json::from_slice(&fs::read(root.join("fixtures/private/goth-native-animation-rebase-20261028-first/native-before-complete-geometry-first.json"))?)?;
        ensure!(
            evaluate(&source, &saved)?["difference_count"] == 0,
            "actual nested baseline differs"
        );
        let vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
        for kind in 0..5 {
            let mut changed = saved.clone();
            match kind {
                0 => {
                    changed
                        .assets
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .geometry
                        .vertices
                        .values_mut()
                        .next()
                        .context("native vertex cache absent")?
                        .vertices[0]
                        .position[0] += 0.25
                }
                1 => {
                    changed
                        .assets
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .binding
                        .next_binding += 1
                }
                2 => {
                    changed
                        .simulation
                        .state
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .controls
                        .movement
                        .path
                        .as_mut()
                        .context("native inactive path owner absent")?
                        .index += 1
                }
                3 => {
                    changed
                        .assets
                        .characters
                        .get_mut(&16)
                        .unwrap()
                        .appearance_owner
                        .selection
                        .tint[0] ^= 1
                }
                4 => changed
                    .assets
                    .characters
                    .get_mut(&16)
                    .unwrap()
                    .model
                    .needs_appearance
                    .push('x'),
                _ => unreachable!(),
            }
            ensure!(
                evaluate(&source, &changed)?["difference_count"] == 1,
                "nested owner mutation was lost: {kind}"
            );
            ensure!(
                postcard::to_allocvec(&changed.simulation.state.vm)? == vm,
                "read-only comparator touched VM/RNG"
            );
        }
        Ok(())
    }
}
