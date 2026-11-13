//! Source-bound record/selection state and immutable avatar reconstruction.
//! This is not the complete process asset-host/provider/job Snapshot.
use super::{
    completed_geometry::{bytes, check_code, field, i, rows, u, word},
    completed_prototypes,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_content::{
    Index,
    character_loading::live::SavedAssets,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    appearance_reload::Owner,
    appearance_selection::State,
};
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub characters: completed_prototypes::Imported,
    /// These are validation/source-bound views; live Session ownership is not
    /// admitted until the remaining process asset-host owners are supplied.
    pub saved: BTreeMap<ObjectId, SavedAssets>,
    pub native_records: BTreeMap<ObjectId, Vec<u32>>,
    pub immutable_sources: BTreeMap<String, String>,
}
fn text(v: &Value) -> Result<String> {
    let b = bytes(v, "used_raw")?;
    ensure!(
        b.is_ascii() && !b.contains(&0),
        "unsupported appearance narrow text"
    );
    Ok(String::from_utf8(b)?)
}
fn native_vector(actor: &Value, ar: &[u8], offset: usize) -> Result<Vec<u32>> {
    let v = field(field(actor, "vectors")?, &offset.to_string())?;
    for (j, k) in ["begin", "end", "capacity"].into_iter().enumerate() {
        ensure!(
            word(v, k)? == u(ar, offset + 4 * j)?,
            "appearance role vector alias differs"
        );
    }
    let raw = bytes(v, "used_raw")?;
    ensure!(
        u(ar, offset + 4)?.checked_sub(u(ar, offset)?) == Some(raw.len() as u32)
            && raw.len() % 4 == 0,
        "appearance role vector extent differs"
    );
    raw.chunks_exact(4).map(|r| u(r, 0)).collect()
}
pub(super) fn mesh_signature(mesh: &sims_formats::avatar::Mesh) -> Value {
    json!({"name":mesh.name,"texture":mesh.texture,"bones":mesh.bones,"bindings":mesh.bindings,"triangles":mesh.triangles,"vertices":mesh.vertices.iter().map(|v|(v.position.map(f32::to_bits),v.normal.map(f32::to_bits),v.uv.map(f32::to_bits),v.bone)).collect::<Vec<_>>(),"blends":mesh.blends.iter().map(|v|(v.vertex,v.weight.to_bits(),v.position.map(f32::to_bits),v.normal.map(f32::to_bits),v.bone)).collect::<Vec<_>>()})
}
pub fn import(
    raw_input: &Value,
    source_code: &Value,
    characters: completed_prototypes::Imported,
    index: &Index,
) -> Result<Imported> {
    ensure!(
        source_code["schema"] == 1
            && source_code["tick"] == 88448
            && source_code["executes_target_code"] == false,
        "appearance code cut differs"
    );
    import_fields(raw_input, source_code, characters, index)
}

/// Current native record/selection owners, backed by the same sparse epoch and
/// independently read immutable assets. The historical entry keeps its tick pin.
pub fn import_captured(
    raw_input: &Value,
    source_code: &Value,
    characters: completed_prototypes::Imported,
    index: &Index,
    tick: u64,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(
        source_code["schema"] == 1
            && source_code["tick"] == tick
            && source_code["contract"] == "completed-native-appearance-code-v2"
            && source_code["source_readset_sha256"] == memory.source_sha256()
            && source_code["executes_target_code"] == false,
        "current appearance source cut differs"
    );
    ensure!(
        !characters.characters.retained.is_empty(),
        "current appearance source actors absent"
    );
    for r in characters.characters.retained.values() {
        ensure!(
            source_code["source_observation_sha256"] == r.observation_sha256
                && source_code["physical_compressed_sha256"] == r.image_sha256,
            "current appearance code crosses native epochs"
        );
    }
    super::completed_skeletons::captured::reads(raw_input, memory)?;
    for r in rows(source_code, "reads")? {
        let b = bytes(r, "raw")?;
        ensure!(
            r["bytes"] == b.len() && r["sha256"] == sims_fixtures::digest(&b),
            "current appearance code receipt differs"
        );
        super::completed_skin::captured::overlap(memory, word(r, "address")?, &b)?;
    }
    for a in rows(raw_input, "actors")? {
        let b = bytes(a, "animator_raw")?;
        ensure!(
            memory.read(word(a, "animator")?, b.len())? == b,
            "current appearance animator differs"
        );
    }
    import_fields(raw_input, source_code, characters, index)
}
fn import_fields(
    raw_input: &Value,
    source_code: &Value,
    characters: completed_prototypes::Imported,
    index: &Index,
) -> Result<Imported> {
    check_code(source_code, CODE)?;
    let mut saved = BTreeMap::new();
    let mut native_records = BTreeMap::new();
    let mut immutable_sources = BTreeMap::new();
    let actors = rows(raw_input, "actors")?;
    ensure!(
        actors.len() == characters.characters.residents.len(),
        "appearance actor coverage differs"
    );
    for (&id, resident) in &characters.characters.residents {
        let actor = actors
            .iter()
            .find(|a| a["object"] == id)
            .context("appearance actor absent")?;
        let ar = bytes(actor, "animator_raw")?;
        let native = &characters.characters.retained[&id].native;
        ensure!(
            ar.len() == 0x2a0
                && word(actor, "animator")? == native.animator
                && u(&ar, 8)? == native.model,
            "appearance resident epoch differs"
        );
        let generic = native_vector(actor, &ar, 0xc0)?;
        let body = native_vector(actor, &ar, 0x25c)?;
        let hands = native_vector(actor, &ar, 0x268)?;
        let accessories = native_vector(actor, &ar, 0x274)?;
        ensure!(
            generic.len() == 1 && body.len() == 2 && hands.len() == 2,
            "unsupported appearance role cardinality"
        );
        let head = word(actor, "single_record")?;
        ensure!(
            head == u(&ar, 0x258)?,
            "appearance head record alias differs"
        );
        let mut pointers = vec![generic[0], head, hands[0], hands[1], body[0], body[1]];
        pointers.extend(&accessories);
        ensure!(
            pointers.iter().copied().collect::<BTreeSet<_>>().len() == pointers.len(),
            "appearance record role alias needs separate ownership"
        );
        let source_records = rows(actor, "records")?;
        ensure!(
            source_records.len() == pointers.len(),
            "unclaimed appearance records"
        );
        let geometry = &characters.characters.geometry.characters[&id];
        let mut entries = vec![];
        let mut live_ids = BTreeSet::new();
        for &pointer in &pointers {
            let record = source_records
                .iter()
                .find(|r| word(r, "pointer").ok() == Some(pointer))
                .context("appearance record absent")?;
            let raw = bytes(record, "raw")?;
            ensure!(raw.len() == 28, "appearance record size differs");
            for (offset, key) in [(4, "name"), (16, "texture")] {
                let v = field(record, key)?;
                for (j, k) in ["begin", "end", "capacity"].into_iter().enumerate() {
                    ensure!(
                        word(v, k)? == u(&raw, offset + 4 * j)?,
                        "appearance record string alias differs"
                    );
                }
                ensure!(
                    word(v, "end")?.checked_sub(word(v, "begin")?)
                        == Some(bytes(v, "used_raw")?.len() as u32),
                    "appearance record text extent differs"
                );
            }
            let instance = u(&raw, 0)?;
            let binding = if instance == 0 {
                None
            } else {
                let identity = *geometry
                    .native_appearances
                    .get(&instance)
                    .context("appearance record instance absent from geometry")?;
                ensure!(
                    live_ids.insert(identity),
                    "appearance instance claimed by two records"
                );
                Some(Binding {
                    id: identity,
                    owner: id as u64,
                })
            };
            entries.push(Record {
                binding,
                name: text(field(record, "name")?)?,
                texture: text(field(record, "texture")?)?,
            });
        }
        ensure!(
            live_ids.len() == geometry.geometry.state.appearances.len(),
            "geometry appearance has no retained record owner"
        );
        let order: Vec<_> = {
            let mut v: Vec<_> = geometry
                .geometry
                .appearances
                .iter()
                .map(|(&id, &n)| (n, id))
                .collect();
            v.sort_by_key(|x| x.0);
            v.into_iter().map(|x| x.1).collect()
        };
        let binding = BindingState {
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
        };
        ensure!(
            u(&ar, 0x250)? == 0,
            "appearance auxiliary owner needs separate import"
        );
        let owner = Owner {
            selection: State {
                cached: [
                    i(&ar, 0x1b0)?,
                    i(&ar, 0x1b4)?,
                    i(&ar, 0x1b8)?,
                    i(&ar, 0x1bc)?,
                ],
                dirty: ar[0xd9] != 0,
                special_dirty: ar[0x109] != 0,
                skin_dirty: serde_json::to_value(&resident.character.skin.animator)?["dirty"]
                    .as_bool()
                    .context("retained animator dirty flag absent")?,
                tint: ar[0xf4..0xf7].try_into()?,
            },
            model_present: u(&ar, 8)? != 0,
            attachment: None,
            attachment_metadata: u(&ar, 0x254)?,
        };
        let source = resident
            .source
            .as_ref()
            .context("appearance immutable skeleton source absent")?;
        let restored = BoundAvatar::restore(index, &source.name, id as u64, &binding)?;
        ensure!(
            restored.avatar.skeleton == *source && restored.snapshot() == binding,
            "appearance immutable reconstruction changed source/records"
        );
        for &identity in &binding.order {
            let appearance = *geometry
                .geometry
                .appearances
                .get(&identity)
                .context("appearance geometry identity absent")?;
            let indices = &geometry.geometry.state.appearances[appearance];
            let range = restored
                .part_ranges
                .get(&identity)
                .context("appearance part range absent")?;
            ensure!(
                range.len() == indices.len(),
                "native/restored appearance mesh count differs"
            );
            for (part, &index) in restored.avatar.parts[range.clone()].iter().zip(indices) {
                let native_mesh = geometry.geometry.state.bindings[index].mesh;
                let mesh = sims_formats::avatar::read_bmf(
                    &characters.characters.geometry.mesh_bmf[native_mesh],
                )?;
                ensure!(
                    mesh_signature(&part.mesh) == mesh_signature(&mesh),
                    "native/restored mesh bytes differ for {}",
                    part.mesh.name
                );
            }
        }
        for (locator, hash) in restored.avatar.sources {
            if let Some(old) = immutable_sources.insert(locator, hash.clone()) {
                ensure!(old == hash, "appearance immutable source changed");
            }
        }
        saved.insert(
            id,
            SavedAssets {
                skeleton: source.name.clone(),
                model: characters.characters.models[&id].clone(),
                geometry: geometry.geometry.clone(),
                binding,
                appearance_owner: owner,
            },
        );
        native_records.insert(id, pointers);
    }
    Ok(Imported {
        characters,
        saved,
        native_records,
        immutable_sources,
    })
}
const CODE: &[(&str, u32, usize, &str)] = &[
    (
        "FUN_0058a178",
        0x58a178,
        13859,
        "416c73fded50f3eadbcd9bdd5a0a535017cd1620a5fb07a8dcee70ff84a72cf5",
    ),
    (
        "FUN_005888d3",
        0x5888d3,
        1086,
        "a6271565d36d3ec04e84fc8cac8abf4b623c2440cdf68299bdb68eb3be597422",
    ),
    (
        "FUN_00583559",
        0x583559,
        112,
        "2b06a02b50378675c35c0071171117f86fbc4b56aa4d05994a4d7d75a61e838f",
    ),
    (
        "FUN_005835c9",
        0x5835c9,
        36,
        "cabeaab0eba299459bb392d2eeba2b3d7163594aa84b179a877dcc19c3a8c23d",
    ),
    (
        "FUN_00583541",
        0x583541,
        24,
        "de4815ad00113d777554ff00c606429472fdd12a6005169a7098fc9e7eb6ecb8",
    ),
];
