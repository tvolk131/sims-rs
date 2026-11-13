//! Source-bound ordinary ROOT graphs, including the live selection indicator.
//! Geometry and the older empty-appearance frame path remain distinct owners.
//! This is not a complete process asset host or executable Session admission.
use super::{
    completed_geometry::{self, Skin, bytes, check_code, field, rows, u, word},
    completed_skeletons::NativeBinding,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_content::{
    Index, graphic_loading,
    graphic_retirement::observed::frame,
    saved_avatar::{BindingState, BoundAvatar},
};
use sims_sim::pose::{
    appearance::{Binding, Record, Records},
    model::Bounds,
};
use sims_vm::{Definition, ObjectId};
use std::collections::{BTreeMap, BTreeSet};

pub mod live;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub geometry: completed_geometry::Imported,
    pub frame_registry: graphic_loading::Saved,
    pub registry_lifetimes: BTreeMap<ObjectId, u32>,
    pub bindings: BTreeMap<ObjectId, BindingState>,
    /// Complete original ordinary graph, including unconsumed cache words.
    pub source: Value,
    pub immutable_sources: BTreeMap<String, String>,
    pub unadmitted_live_graphics: Vec<ObjectId>,
    pub adapter_epoch: String,
}

struct Memory(BTreeMap<u32, u8>);
impl Memory {
    fn from(input: &Value) -> Result<Self> {
        let mut m = Self(BTreeMap::new());
        for rs in [
            rows(input, "reads")?,
            rows(field(input, "geometry")?, "reads")?,
        ] {
            for r in rs {
                let raw = bytes(r, "raw")?;
                let p = word(r, "address")?;
                ensure!(
                    raw.len() == word(r, "bytes")? as usize
                        && r["sha256"] == sims_fixtures::digest(&raw)
                        && raw.len() <= 1 << 20,
                    "ordinary read receipt differs"
                );
                for (j, b) in raw.into_iter().enumerate() {
                    let a = p
                        .checked_add(j.try_into()?)
                        .context("ordinary read overflow")?;
                    ensure!(
                        m.0.insert(a, b).is_none_or(|old| old == b),
                        "ordinary same-cut read receipts conflict"
                    );
                }
                ensure!(m.0.len() <= 1 << 20, "ordinary read union exceeds profile");
            }
        }
        Ok(m)
    }
    fn read(&self, p: u32, n: usize) -> Result<Vec<u8>> {
        ensure!(n <= 1 << 20, "ordinary field extent exceeds profile");
        (0..n)
            .map(|j| {
                self.0
                    .get(
                        &p.checked_add(j.try_into()?)
                            .context("ordinary field overflow")?,
                    )
                    .copied()
                    .context("ordinary field outside read receipts")
            })
            .collect()
    }
}
fn at<'a>(input: &'a Value, path: &str) -> Result<&'a Value> {
    let mut v = if path.starts_with("fields.") {
        input
    } else {
        field(input, "retained")?
    };
    for part in path.split('.') {
        if let Some((key, rest)) = part.split_once('[') {
            v = field(v, key)?;
            let key = rest
                .strip_suffix(']')
                .context("invalid ordinary mapped subscript")?;
            v = if let Some(a) = v.as_array() {
                a.get(key.parse::<usize>()?)
                    .context("ordinary mapped index absent")?
            } else {
                field(v, key)?
            };
        } else {
            v = field(v, part)?;
        }
    }
    Ok(v)
}
fn bits(v: &Value, width: usize) -> Result<Vec<u8>> {
    let n = if v.is_null() {
        0
    } else if let Some(n) = v.as_u64() {
        u32::try_from(n)?
    } else {
        i32::try_from(v.as_i64().context("ordinary mapped integer absent")?)? as u32
    };
    match width {
        1 => Ok(vec![u8::try_from(n)?]),
        4 => Ok(n.to_le_bytes().to_vec()),
        _ => anyhow::bail!("ordinary mapped scalar width differs"),
    }
}
fn mapped(input: &Value, m: &Memory, mapping: &[Value]) -> Result<()> {
    let mut paths = BTreeSet::new();
    for f in mapping {
        let path = f["path"].as_str().context("ordinary mapping path absent")?;
        ensure!(paths.insert(path), "duplicate ordinary mapped field");
        let raw = m.read(word(f, "address")?, word(f, "bytes")? as usize)?;
        if let Some(base) = path.strip_suffix(".items") {
            let values = at(input, base)?
                .as_array()
                .context("ordinary mapped items absent")?;
            let mut expected = vec![];
            for v in values {
                expected.extend(bits(v.get("id").unwrap_or(v), 4)?);
            }
            ensure!(raw == expected, "ordinary mapped pointer items differ");
            continue;
        }
        if let Some(base) = path.strip_suffix(".header") {
            ensure!(raw.len() == 12, "ordinary mapped vector header differs");
            let (b, e, c) = (u(&raw, 0)?, u(&raw, 4)?, u(&raw, 8)?);
            ensure!(
                b <= e && e <= c && c - b <= 1 << 20,
                "ordinary mapped vector extent differs"
            );
            let value = at(input, base)?;
            if value.is_object() && value.get("bytes").is_some() {
                ensure!(
                    value["capacity"] == c - b
                        && value["allocation"] == if c > b { Value::from(b) } else { Value::Null },
                    "ordinary text storage differs"
                );
                let used = value["bytes"]
                    .as_array()
                    .context("ordinary mapped text absent")?;
                ensure!(
                    used.len() == (e - b) as usize,
                    "ordinary text length differs"
                );
                if c > e {
                    ensure!(m.read(e, 1)? == [0], "ordinary text terminator differs");
                }
            } else {
                let values = value.as_array().context("ordinary mapped vector absent")?;
                ensure!(
                    e - b == 4 * values.len() as u32,
                    "ordinary pointer vector length differs"
                );
                let mut expected = vec![];
                for v in values {
                    expected.extend(bits(v.get("id").unwrap_or(v), 4)?);
                }
                ensure!(
                    m.read(b, (e - b) as usize)? == expected,
                    "ordinary pointer vector order differs"
                );
                let storage_path = match base {
                    "notification.children" => "notification.storage".to_owned(),
                    _ => format!("{base}_storage"),
                };
                let storage = at(input, &storage_path)?;
                ensure!(
                    storage["capacity_bytes"] == c - b
                        && storage["allocation"]
                            == if c > b { Value::from(b) } else { Value::Null },
                    "ordinary vector storage differs"
                );
            }
            continue;
        }
        let v = at(input, path)?;
        let expected = if let Some(a) = v.as_array() {
            let width = if a.is_empty() { 1 } else { raw.len() / a.len() };
            let mut all = vec![];
            for v in a {
                all.extend(bits(v, width)?);
            }
            all
        } else {
            bits(v, raw.len())?
        };
        ensure!(raw == expected, "ordinary mapped field differs: {path}");
    }
    Ok(())
}
fn text(bytes: &[u8]) -> Result<String> {
    ensure!(
        bytes.is_ascii() && !bytes.contains(&0),
        "ordinary narrow name unsupported"
    );
    Ok(String::from_utf8(bytes.to_vec())?)
}
fn bounds(bits: [u32; 6]) -> Result<Bounds> {
    let f = bits.map(f32::from_bits);
    ensure!(f.iter().all(|v| v.is_finite()), "ordinary bounds nonfinite");
    Ok(Bounds {
        minimum: f[..3].try_into()?,
        maximum: f[3..].try_into()?,
    })
}

fn raw_aliases(value: &Value, m: &Memory) -> Result<()> {
    match value {
        Value::Array(a) => {
            for v in a {
                raw_aliases(v, m)?
            }
        }
        Value::Object(o) => {
            for (raw, pointer) in [
                ("raw", "pointer"),
                ("model_raw", "model"),
                ("skin_raw", "skin"),
                ("render_bounds_raw", "render_bounds"),
                ("resource_raw", "resource"),
                ("descriptor_raw", "descriptor"),
                ("vertices_raw", "vertices_pointer"),
                ("used_raw", "begin"),
                ("allocated_raw", "begin"),
            ] {
                if let (Some(raw), Some(p)) = (o.get(raw), o.get(pointer)) {
                    let raw = super::completed_animations::bytes(
                        raw.as_str().context("ordinary raw alias absent")?,
                    )?;
                    let p =
                        u32::try_from(p.as_u64().context("ordinary raw alias pointer absent")?)?;
                    ensure!(m.read(p, raw.len())? == raw, "ordinary raw alias differs");
                }
            }
            for v in o.values() {
                raw_aliases(v, m)?
            }
        }
        _ => {}
    }
    Ok(())
}

/// A fresh unpublished Rust identity domain is explicit. Read receipts and
/// source code close the graph; no graphics update, notification or RNG runs.
/// A nonempty appearance graph is retained and decoded, never fed to the old
/// empty-owner destructor/frame adapter by clearing the offending vectors.
pub fn import(
    input: &Value,
    definitions: &BTreeMap<ObjectId, Definition>,
    lifetimes: &BTreeMap<ObjectId, u32>,
    index: &Index,
    fresh_epoch: &str,
) -> Result<Imported> {
    ensure!(
        input["schema"] == 1
            && input["tick"] == 88448
            && input["executes_target_code"] == false
            && !fresh_epoch.is_empty(),
        "ordinary capture profile/epoch differs"
    );
    import_fields(
        input,
        definitions,
        lifetimes,
        index,
        fresh_epoch,
        word(input, "fp_control")?,
    )
}

/// Same-cut ordinary owners. The caller admits the frozen observation; every
/// original read receipt must also belong to that observation's sparse readset.
pub fn import_captured(
    input: &Value,
    definitions: &BTreeMap<ObjectId, Definition>,
    lifetimes: &BTreeMap<ObjectId, u32>,
    index: &Index,
    fresh_epoch: &str,
    state: &Value,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(
        input["schema"] == 1
            && input["tick"] == state["tick"]
            && input["executes_target_code"] == false
            && !fresh_epoch.is_empty(),
        "current ordinary source epoch differs"
    );
    super::completed_skeletons::captured::reads(input, memory)?;
    super::completed_skeletons::captured::reads(&input["geometry"], memory)?;
    let controls: Vec<_> = state["identity"]["hardware_registers"]
        .as_str()
        .context("current ordinary CPU state absent")?
        .split_whitespace()
        .filter_map(|s| s.strip_prefix("FCW="))
        .collect();
    ensure!(
        controls.len() == 1,
        "current ordinary x87 control is missing or ambiguous"
    );
    let fp_control = u32::from_str_radix(controls[0], 16)?;
    import_fields(
        input,
        definitions,
        lifetimes,
        index,
        fresh_epoch,
        fp_control,
    )
}

fn import_fields(
    input: &Value,
    definitions: &BTreeMap<ObjectId, Definition>,
    lifetimes: &BTreeMap<ObjectId, u32>,
    index: &Index,
    fresh_epoch: &str,
    fp_control: u32,
) -> Result<Imported> {
    check_code(input, CODE)?;
    ensure!(
        lifetimes.keys().eq(definitions.keys())
            && lifetimes.values().all(|v| *v > 0)
            && lifetimes.values().copied().collect::<BTreeSet<_>>().len() == lifetimes.len(),
        "ordinary adapter lifetime domain differs"
    );
    let m = Memory::from(input)?;
    raw_aliases(field(input, "geometry")?, &m)?;
    let view = field(input, "view")?;
    let app = word(view, "app")?;
    let view_pointer = word(view, "view")?;
    let zoom = word(view, "zoom")?;
    ensure!(
        zoom <= 2
            && word(view, "rotation")? <= 3
            && fp_control == 0x003f
            && u(&m.read(0x6e48bc, 4)?, 0)? == app
            && u(&m.read(app + 4, 4)?, 0)? == view_pointer
            && u(&m.read(0x6f00d8, 4)?, 0)? == zoom
            && u(&m.read(0x6f00dc, 4)?, 0)? == word(view, "rotation")?,
        "ordinary source view/x87 profile differs"
    );
    for (key, p, count) in [
        ("floor_vector_bits", view_pointer + 0x170 - zoom * 12, 3),
        ("origin_pose_bits", 0x6f7620, 3),
    ] {
        let values = view[key]
            .as_array()
            .context("ordinary source context absent")?;
        let mut expected = vec![];
        for v in values.iter().take(count) {
            expected.extend(bits(v, 4)?);
        }
        ensure!(
            expected.len() == count * 4 && m.read(p, count * 4)? == expected,
            "ordinary source context bits differ"
        );
    }
    let rotation = view["origin_pose_bits"]
        .as_array()
        .context("ordinary source origin absent")?;
    ensure!(rotation.len() == 7, "ordinary source origin extent differs");
    let mut expected = vec![];
    for v in &rotation[3..] {
        expected.extend(bits(v, 4)?);
    }
    ensure!(
        m.read(0x6f7610, 16)? == expected,
        "ordinary source origin rotation differs"
    );
    let mut natives = BTreeMap::new();
    let mut vertex_bounds = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    let mut inventory = BTreeSet::new();
    let mut registry = graphic_loading::Registry::default();
    let mut unadmitted = vec![];
    for row in rows(input, "observed_null_objects")? {
        let id = ObjectId::try_from(word(row, "object")?)?;
        ensure!(
            id > 0 && inventory.insert(id),
            "ordinary inventory identity differs"
        );
        ensure!(
            m.read(word(row, "native_actor")? + 0x120, 4)? == [0; 4],
            "ordinary observed-null owner differs"
        );
    }
    for row in rows(input, "actors")? {
        let id = ObjectId::try_from(word(row, "object")?)?;
        ensure!(
            id > 0 && inventory.insert(id),
            "ordinary inventory identity differs"
        );
        let definition = definitions.get(&id).context("ordinary definition absent")?;
        let raw_input = field(row, "input")?;
        let graph: frame::Input = serde_json::from_value(raw_input.clone())?;
        ensure!(
            serde_json::to_value(&graph)? == *raw_input,
            "ordinary typed projection lost fields"
        );
        mapped(raw_input, &m, rows(row, "field_mapping")?)?;
        let r = &graph.retained;
        let actor = u32::try_from(r.actor)?;
        let ar = bytes(row, "animator_raw")?;
        let obj = bytes(row, "object_raw")?;
        let resource = bytes(row, "resource_raw")?;
        let def = bytes(row, "definition_raw")?;
        ensure!(
            ar.len() == 0x138
                && obj.len() == 0x188
                && resource.len() == 0xb8
                && def.len() == 0xd8
                && m.read(actor, obj.len())? == obj
                && m.read(u(&obj, 0x11c)?, resource.len())? == resource
                && m.read(u(&obj, 0x118)?, def.len())? == def
                && m.read(r.animator.try_into()?, ar.len())? == ar
                && actor == word(row, "native_actor")?
                && u(&obj, 0x120)? as u64 == r.animator
                && u(&ar, 4)? == actor
                && u(&ar, 0)? == 0x671d6c
                && row["ready"] == obj[0x180]
                && u(&resource, 0x60)? == u(&obj, 0x118)?
                && definition.guid == u(&def, 0x1c)?
                && definition.object == (u(&resource, 0xb4)? >> 16) as u16
                && definition.source == format!("native-provider:{:08x}", u(&resource, 0)?),
            "ordinary actor/resource/definition aliases differ"
        );
        ensure!(
            r.layers == [None; 3]
                && r.skin.layers.is_empty()
                && r.skin.bones.len() == 1
                && graph.fields.bones.len() == 1
                && r.auxiliary.is_empty()
                && r.generic.len() == 1
                && r.skin.root == r.skin.bones[0].id
                && r.skin.bones[0].parent.is_none()
                && r.skin.bones[0].next_sibling.is_none()
                && graph.fields.bones[&r.skin.root].first_child.is_none()
                && r.skin.bones[0].name.bytes == b"ROOT",
            "ordinary built-in ROOT profile differs"
        );
        for (&p, allocation) in &r.allocations {
            ensure!(allocation.live, "ordinary allocation marked retired");
            m.read(p.try_into()?, allocation.bytes as usize)?;
        }
        let b = &r.skin.bones[0];
        ensure!(
            m.read(b.id.try_into()?, 0x16c)? == bytes(field(row, "bones_raw")?, &b.id.to_string())?,
            "ordinary retained bone raw alias differs"
        );
        let lookup = r
            .skin
            .lookup
            .iter()
            .map(|p| match p {
                None => Ok(None),
                Some(p) => {
                    ensure!(*p == b.id, "ordinary bone lookup alias differs");
                    Ok(Some(0))
                }
            })
            .collect::<Result<_>>()?;
        natives.insert(
            id,
            NativeBinding {
                person: actor,
                animator: r.animator.try_into()?,
                model: r.model.id.try_into()?,
                skin: r.skin.id.try_into()?,
                bone_pointers: vec![b.id.try_into()?],
                bone_names: vec![b.name.bytes.clone()],
                parent_names: vec![b.parent_name.bytes.clone()],
                lookup,
                root: 0,
            },
        );
        vertex_bounds.insert(
            id,
            vec![bounds(graph.fields.bones[&b.id].vertex_bounds_bits)?],
        );
        if r.skin.appearances.is_empty() && r.notification.children.is_empty() {
            registry.install_retained_frame(
                frame::Binding {
                    object: id,
                    definition: definition.clone(),
                    registry_lifetime: lifetimes[&id],
                    native_actor: r.actor,
                },
                obj[0x180],
                graph.clone(),
                sims_content::graphic_loading::retained::RefreshContext {
                    floor_vector_bits: serde_json::from_value(
                        input["view"]["floor_vector_bits"].clone(),
                    )?,
                    fp_control: u16::try_from(fp_control)?,
                    origin_pose_bits: serde_json::from_value(
                        input["view"]["origin_pose_bits"].clone(),
                    )?,
                },
            )?;
        } else {
            unadmitted.push(id);
        }
        inputs.insert(id, graph);
    }
    ensure!(
        inventory.iter().copied().collect::<BTreeSet<_>>() == definitions.keys().copied().collect(),
        "ordinary observed-null/positive inventory coverage differs"
    );
    let skins = natives
        .iter()
        .map(|(&id, native)| {
            (
                id,
                Skin {
                    native,
                    bone_vertex_bounds: &vertex_bounds[&id],
                },
            )
        })
        .collect();
    let geometry =
        completed_geometry::import_with_bindings(field(input, "geometry")?, &skins, fresh_epoch)?;
    let mut bindings = BTreeMap::new();
    let mut immutable_sources = BTreeMap::new();
    for (&id, graph) in &inputs {
        let g = &geometry.characters[&id];
        let r = &graph.retained;
        let record = &r.generic[0];
        let binding = record
            .binding
            .map(|p| {
                g.native_appearances
                    .get(&(p as u32))
                    .copied()
                    .map(|id_| Binding {
                        id: id_,
                        owner: id as u64,
                    })
                    .context("ordinary record appearance absent")
            })
            .transpose()?;
        ensure!(
            r.skin.appearances.len() == usize::from(binding.is_some()),
            "ordinary unowned appearance"
        );
        let raw_bindings = rows(field(input, "geometry")?, "bindings")?;
        let children = g
            .native_bindings
            .iter()
            .map(|&p| {
                let binding = raw_bindings
                    .iter()
                    .find(|b| word(b, "pointer").ok() == Some(p))
                    .context("ordinary notification drawable binding absent")?;
                Ok(u64::from(word(binding, "drawable")?))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            children == r.notification.children,
            "ordinary notification child/drawable order differs"
        );
        let order = binding.iter().map(|b| b.id).collect::<Vec<_>>();
        let state = BindingState {
            records: Records {
                entries: vec![Record {
                    binding,
                    name: text(&record.name.bytes)?,
                    texture: text(&record.texture.bytes)?,
                }],
                generic: 0,
                head: 0,
                hands: [0; 2],
                body: [0; 2],
                accessories: vec![],
            },
            next_binding: order.iter().max().copied().unwrap_or(0) + 1,
            order,
            auxiliary: BTreeMap::new(),
        };
        let restored = BoundAvatar::restore(index, "root", id as u64, &state)?;
        ensure!(
            restored.snapshot() == state,
            "ordinary immutable restore changed bindings"
        );
        for (&identity, &appearance) in &g.geometry.appearances {
            let indices = &g.geometry.state.appearances[appearance];
            let range = restored
                .part_ranges
                .get(&identity)
                .context("ordinary immutable part range absent")?;
            ensure!(
                range.len() == indices.len(),
                "ordinary immutable mesh count differs"
            );
            for (part, &binding_index) in restored.avatar.parts[range.clone()].iter().zip(indices) {
                let mesh = sims_formats::avatar::read_bmf(
                    &geometry.mesh_bmf[g.geometry.state.bindings[binding_index].mesh],
                )?;
                ensure!(
                    super::completed_appearances::mesh_signature(&part.mesh)
                        == super::completed_appearances::mesh_signature(&mesh),
                    "ordinary immutable/native mesh fields differ"
                );
            }
        }
        for (locator, hash) in restored.avatar.sources {
            ensure!(
                immutable_sources
                    .insert(locator, hash.clone())
                    .is_none_or(|old| old == hash),
                "ordinary immutable source changed"
            );
        }
        bindings.insert(id, state);
    }
    Ok(Imported {
        geometry,
        frame_registry: registry.snapshot(),
        registry_lifetimes: lifetimes.clone(),
        bindings,
        source: input.clone(),
        immutable_sources,
        unadmitted_live_graphics: unadmitted,
        adapter_epoch: fresh_epoch.into(),
    })
}

const CODE: &[(&str, u32, usize, &str)] = &[
    (
        "FUN_004376f6",
        0x4376f6,
        109,
        "56353835aa4bf75ca0d9ea11dedead2bb33ae86c093b18e450a80b8a4993d382",
    ),
    (
        "FUN_0058464f",
        0x58464f,
        509,
        "a517a1f3a9bdac37c49e1671fc9ea0f6b2c00bdd7746e586ac7822dadae03c79",
    ),
    (
        "FUN_00585483",
        0x585483,
        127,
        "e56e2de2b14bcf1a3d69609bfd93d6e94355d5672c635ed51b01064d7a507256",
    ),
    (
        "FUN_00585555",
        0x585555,
        658,
        "b3470fce5eabe2fa6cb110a3beedb5ea5694b48b61b685eadf8455d803301c22",
    ),
    (
        "FUN_0058598c",
        0x58598c,
        186,
        "82fa9b6ff102acc7f825a2615b54c147d83c22afe1cb56e4af0fb836c47d28ee",
    ),
    (
        "FUN_00585a46",
        0x585a46,
        421,
        "2683ead8953f6cac063e58832c94b3cd0600c576be56355c6b0586f4ba6835f4",
    ),
    (
        "FUN_005867d3",
        0x5867d3,
        1392,
        "191bfd887d875b9426b5db31df90b707ad239d9bdf046fac84961354745cce13",
    ),
    (
        "FUN_00585515",
        0x585515,
        55,
        "05c52ee4978944fa779e61c5f93b06d14afd110a4784f4e8bae3806f671f2c00",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }
    fn actual() -> Result<(
        Value,
        BTreeMap<ObjectId, Definition>,
        BTreeMap<ObjectId, u32>,
        Index,
    )> {
        let root = root();
        let input = serde_json::from_slice(&std::fs::read(
            root.join("fixtures/private/goth-ordinary-host-20261028-second/ordinary-sixth.json"),
        )?)?;
        let reentry: Value = serde_json::from_slice(&std::fs::read(root.join(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
        ))?)?;
        let definitions: BTreeMap<ObjectId, Definition> = rows(&reentry, "objects")?
            .iter()
            .map(|r| {
                Ok((
                    ObjectId::try_from(word(field(r, "legacy")?, "id")?)?,
                    serde_json::from_value(field(r, "definition")?.clone())?,
                ))
            })
            .collect::<Result<_>>()?;
        let mut index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
        index
            .resources
            .retain(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"));
        let lifetimes = definitions
            .keys()
            .map(|&id| {
                Ok((
                    id,
                    u32::try_from(
                        reentry["registry"]["handles"][id as usize - 1]
                            .as_u64()
                            .context("canonical lifetime absent")?,
                    )?,
                ))
            })
            .collect::<Result<_>>()?;
        Ok((input, definitions, lifetimes, index))
    }

    #[test]
    #[ignore = "private captured original Goth ROOT/arrow evidence and immutable installation"]
    fn actual_graphs_preserve_arrow_and_cold_empty_frame_owner() -> Result<()> {
        let (input, definitions, lifetimes, index) = actual()?;
        let imported = import(
            &input,
            &definitions,
            &lifetimes,
            &index,
            "new-unpublished-goth-ordinary",
        )?;
        ensure!(
            imported.geometry.characters.len() == 2
                && imported.geometry.mesh_bmf.len() == 1
                && imported.unadmitted_live_graphics == [383]
                && imported
                    .frame_registry
                    .retained
                    .objects()
                    .collect::<Vec<_>>()
                    == [384],
            "actual ordinary coverage differs"
        );
        let path = root().join("fixtures/private/goth-ordinary-host-20261028-second/cold-store");
        let store = if path.exists() {
            sims_fixtures::Store::open(&path)?
        } else {
            sims_fixtures::Store::create(&path)?
        };
        let encoded = store.put_value(&imported)?;
        let cold: Imported = store.get_value(&encoded)?;
        ensure!(cold == imported, "ordinary cold component differs");
        let registry = graphic_loading::Registry::restore(&index, cold.frame_registry.clone())?;
        ensure!(
            registry.snapshot() == imported.frame_registry,
            "ordinary retained frame restore changed state"
        );
        for (&id, binding) in &cold.bindings {
            ensure!(
                BoundAvatar::restore(&index, "root", id as u64, binding)?.snapshot() == *binding,
                "ordinary cold immutable restore differs"
            );
        }
        let mut bad = input.clone();
        bad["actors"][0]["input"]["retained"]["skin"]["appearances"] = serde_json::json!([]);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "cleared appearance accepted"
        );
        let mut bad = input.clone();
        bad["actors"][0]["input"]["retained"]["notification"]["children"] = serde_json::json!([]);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "cleared notification child accepted"
        );
        let mut bad = input.clone();
        bad["actors"][1]["input"]["fields"]["now"] = serde_json::json!(88448 * 50);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "rewound/refreshed frame clock accepted"
        );
        let mut bad = input.clone();
        bad["observed_null_objects"].as_array_mut().unwrap().pop();
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "missing null owner accepted"
        );
        let mut bad = input.clone();
        bad["view"]["floor_vector_bits"][0] = serde_json::json!(0);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "changed view context accepted"
        );
        let mut bad = input.clone();
        let raw = bad["geometry"]["drawables"][0]["vertices_raw"]
            .as_str()
            .unwrap();
        let raw = format!(
            "{}{}",
            if &raw[..2] == "00" { "01" } else { "00" },
            &raw[2..]
        );
        bad["geometry"]["drawables"][0]["vertices_raw"] = Value::String(raw);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "changed published vertex accepted"
        );
        let mut bad = input.clone();
        bad["actors"][0]["input"]["retained"]["generic"][0]["texture"]["bytes"][0] =
            serde_json::json!(0);
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "changed texture record accepted"
        );
        let mut bad = input.clone();
        bad["reads"][0]["sha256"] = serde_json::json!("0".repeat(64));
        ensure!(
            import(&bad, &definitions, &lifetimes, &index, "negative").is_err(),
            "bad read receipt accepted"
        );
        Ok(())
    }
}
