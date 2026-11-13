//! Retained bone/hierarchy owners from one stopped native epoch.
//!
//! This intentionally stops before Live/Instance/Character construction: the
//! existing Check/menu caller closure does not prove that a native property or
//! skin pass has no retained completion. No handler/pass absence is fabricated.
//! The supplied Animator moves into this owner; no clip is begun, no pose is
//! recomputed, and no skeleton prototype, cache stamp or RNG history is made.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_sim::{
    head_look::rig::{AbsoluteBone, Rig},
    pose::{
        LocalBone,
        animator::Animator,
        model::{Bounds, Matrix},
        refresh::Hierarchy,
        sampling::BoneInput,
        skeleton::{Geometry, WiggleState},
    },
};
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};
pub mod captured;

/// Exact rows from skeletons.characters/skins/bones/anchors and the matching
/// characters.characters row. Complete raw sidecars remain caller-owned and
/// unchanged, including unconsumed fields and external appearance references.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub character: Value,
    pub animator: Value,
    pub skin: Value,
    pub bones: Vec<Value>,
    pub anchors: Vec<Value>,
    pub code: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeBinding {
    pub person: u32,
    pub animator: u32,
    pub model: u32,
    pub skin: u32,
    pub bone_pointers: Vec<u32>,
    pub bone_names: Vec<Vec<u8>>,
    pub parent_names: Vec<Vec<u8>>,
    /// Preserve the source's allocated name-lookup order and observed nulls.
    pub lookup: Vec<Option<usize>>,
    pub root: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Anchor {
    pub native_pointer: u32,
    pub bone: usize,
    pub local: Matrix,
}

/// Typed parts of a retained skeleton; NOT a Live/Instance/Character/Resident.
/// Even a completed registry boundary cannot by itself fill private pass or
/// handler fields with None. Model/drawable/appearance owners remain separate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub native: NativeBinding,
    pub animator: Animator,
    pub bones: Vec<BoneInput>,
    pub hierarchy: Hierarchy,
    pub geometry: Geometry,
    pub wiggle: Vec<WiggleState>,
    pub flags: Vec<[u32; 3]>,
    pub named_bone_flags: Vec<bool>,
    pub bone_bounds: Vec<Bounds>,
    pub bone_bounds_generations: Vec<u32>,
    pub bone_vertex_bounds: Vec<Bounds>,
    pub local_bounds: Bounds,
    pub anchors: BTreeMap<String, Option<Anchor>>,
    pub rig: Option<Rig>,
    pub demands: Vec<String>,
}
const CODE: [(&str, u32, usize, &str); 8] = [
    (
        "bone_pose_and_links",
        0x598967,
        103,
        "2407335bf32b4082cb8b35798b3fd25159807d30bb3e09e0652f2abee8f01af0",
    ),
    (
        "bone_flags_and_wiggle",
        0x5989ce,
        98,
        "97c5e6bb588a794329cc8aa42242882777d2b04f8e5e300d6d1b1b299aa7870a",
    ),
    (
        "skin_bones_and_lookup",
        0x599030,
        23,
        "175b54cd0cadd072e3f6e5fc47cfa78cf8e82225cd6f3e248eb86cb84c615e0c",
    ),
    (
        "bone_skin_backlink",
        0x599074,
        22,
        "64db8468a5476c3d03f39b75895dac1a4781675c2e8917fe98aa59d3534dc27a",
    ),
    (
        "bone_topology",
        0x5990b6,
        35,
        "bba4ba26881ff308f6051adeab0ed2fb775a40471a159f9d862afae9a30ebeae",
    ),
    (
        "hierarchy_generations",
        0x5995ed,
        71,
        "7838caa172ec4a24132d1dfbb161cbd23fad3aa6f8cbdb540c6428f3f9b93dd4",
    ),
    (
        "rig_anchor_bindings",
        0x592543,
        165,
        "09d8e8efea32609ad0c2b0c066d0bf83a89f1195b3a6682a9eda9590290779e8",
    ),
    (
        "anchor_model_skin",
        0x5a1641,
        13,
        "599054b765c3f9485049cdeae7b0ac4aa9791b8393680c298dcffd8f12dd9770",
    ),
];
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k)
        .with_context(|| format!("missing retained skeleton field {k}"))
}
fn u32_(v: &Value) -> Result<u32> {
    Ok(u32::try_from(
        v.as_u64()
            .context("retained skeleton unsigned word required")?,
    )?)
}
fn array(v: &Value) -> Result<&[Value]> {
    v.as_array()
        .map(Vec::as_slice)
        .context("retained skeleton array required")
}
fn raw(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("retained skeleton hex required")?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn floats<const N: usize>(b: &[u8], o: usize) -> Result<[f32; N]> {
    let v = std::array::from_fn(|i| f32::from_bits(word(b, o + 4 * i)));
    ensure!(
        v.iter().all(|n| n.is_finite()),
        "nonfinite retained skeleton field"
    );
    Ok(v)
}
fn bits<const N: usize>(v: &Value) -> Result<[u32; N]> {
    let a = array(v)?;
    ensure!(a.len() == N, "retained skeleton dword extent differs");
    let a: Vec<_> = a.iter().map(u32_).collect::<Result<_>>()?;
    Ok(a.try_into().unwrap())
}
fn compare_bits(v: &Value, b: &[u8], off: usize, n: usize) -> Result<()> {
    let a = array(v)?;
    ensure!(a.len() == n, "retained bone decoded extent differs");
    for (i, v) in a.iter().enumerate() {
        ensure!(
            u32_(v)? == word(b, off + 4 * i),
            "retained bone decoded storage differs"
        );
    }
    Ok(())
}
fn compare_word(v: &Value, key: &str, b: &[u8], off: usize) -> Result<()> {
    ensure!(
        u32_(field(v, key)?)? == word(b, off),
        "retained scalar {key} differs from owner bytes"
    );
    Ok(())
}
fn vector(v: &Value, b: &[u8], off: usize, stride: usize, max: usize) -> Result<Vec<u8>> {
    let begin = word(b, off);
    let end = word(b, off + 4);
    let cap = word(b, off + 8);
    for (i, k) in ["begin", "end", "capacity"].into_iter().enumerate() {
        ensure!(
            u32_(field(v, k)?)? == word(b, off + 4 * i),
            "retained vector header differs"
        );
    }
    ensure!(
        u32_(field(v, "stride")?)? as usize == stride
            && (begin != 0 || end == 0 && cap == 0)
            && end >= begin
            && cap >= end
            && (end - begin) as usize % stride == 0
            && (cap - begin) as usize % stride == 0
            && (cap - begin) as usize / stride <= max,
        "invalid retained skeleton vector"
    );
    let data = raw(field(v, "used_raw")?)?;
    ensure!(
        data.len() == (end - begin) as usize,
        "retained vector bytes differ"
    );
    Ok(data)
}
fn code(v: &[Value]) -> Result<()> {
    ensure!(
        v.len() == CODE.len(),
        "retained skeleton code coverage differs"
    );
    for (v, &(name, address, len, sha)) in v.iter().zip(&CODE) {
        ensure!(
            field(v, "label")?.as_str() == Some(name)
                && u32_(field(v, "address")?)? == address
                && u32_(field(v, "bytes")?)? as usize == len
                && field(v, "sha256")?.as_str() == Some(sha),
            "retained skeleton source witness differs"
        );
    }
    Ok(())
}
fn bounds(b: &[u8], o: usize) -> Result<Bounds> {
    let v = floats::<6>(b, o)?;
    Ok(Bounds {
        minimum: v[..3].try_into().unwrap(),
        maximum: v[3..].try_into().unwrap(),
    })
}

fn one(row: &Input, animator: Animator) -> Result<(ObjectId, Imported)> {
    code(&row.code)?;
    let c = &row.character;
    let a = &row.animator;
    let s = &row.skin;
    let object = i16::try_from(u32_(field(c, "object")?)?)?;
    ensure!(
        object > 0 && u32_(field(a, "object")?)? == object as u32,
        "retained skeleton object differs"
    );
    let person = u32_(field(c, "person")?)?;
    let ap = u32_(field(c, "animator")?)?;
    let model = u32_(field(c, "model")?)?;
    let skin = u32_(field(c, "skin")?)?;
    ensure!(
        person != 0 && ap != 0 && model != 0 && skin != 0,
        "retained skeleton live owner absent"
    );
    for (key, p) in [
        ("person", person),
        ("animator", ap),
        ("model", model),
        ("skin", skin),
    ] {
        ensure!(
            u32_(field(a, key)?)? == p,
            "retained character ownership differs"
        );
    }
    ensure!(
        u32_(field(s, "pointer")?)? == skin,
        "retained skin identity differs"
    );
    let ar = raw(field(a, "header_raw")?)?;
    let mr = raw(field(a, "model_header_raw")?)?;
    let sr = raw(field(s, "raw")?)?;
    ensure!(
        ar.len() == 0x2a0
            && mr.len() == 0xb0
            && sr.len() == 0xd4
            && word(&ar, 0) == 0x671ee4
            && word(&ar, 0x138) == person
            && word(&ar, 8) == model
            && word(&mr, 4) == skin
            && raw(field(a, "skin_header_raw")?)? == sr[..0xbc],
        "retained skin/animator raw ownership differs"
    );
    let bone_raw = vector(field(s, "bone_vector")?, &sr, 0x64, 4, 512)?;
    let lookup_raw = vector(field(s, "lookup_vector")?, &sr, 0x70, 4, 4096)?;
    let pointers: Vec<_> = bone_raw.chunks_exact(4).map(|b| word(b, 0)).collect();
    let lookup: Vec<_> = lookup_raw.chunks_exact(4).map(|b| word(b, 0)).collect();
    let decoded: Vec<u32> = array(field(s, "ordered_bones")?)?
        .iter()
        .map(u32_)
        .collect::<Result<_>>()?;
    ensure!(
        pointers == decoded
            && pointers.len() == row.bones.len()
            && !pointers.is_empty()
            && pointers.len() <= 256
            && !pointers.contains(&0),
        "retained ordered bone coverage differs"
    );
    ensure!(
        lookup
            == array(field(s, "lookup")?)?
                .iter()
                .map(u32_)
                .collect::<Result<Vec<_>>>()?,
        "retained bone lookup order differs"
    );
    let index: BTreeMap<_, _> = pointers
        .iter()
        .copied()
        .enumerate()
        .map(|(i, p)| (p, i))
        .collect();
    ensure!(index.len() == pointers.len(), "aliased retained bone owner");
    let ident = |p: u32| -> Result<Option<usize>> {
        if p == 0 {
            Ok(None)
        } else {
            Ok(Some(
                *index.get(&p).context("foreign retained bone identity")?,
            ))
        }
    };
    let root = word(&sr, 0x60);
    ensure!(u32_(field(s, "root")?)? == root, "retained root differs");
    let root = ident(root)?.context("retained skeleton root absent")?;
    let lookup = lookup.into_iter().map(ident).collect::<Result<Vec<_>>>()?;
    let mut by_pointer = BTreeMap::new();
    for b in &row.bones {
        ensure!(
            by_pointer.insert(u32_(field(b, "pointer")?)?, b).is_none(),
            "duplicate bone source row"
        );
    }
    let mut locals = Vec::new();
    let mut parents = Vec::new();
    let mut next = Vec::new();
    let mut child = Vec::new();
    let mut absolute = Vec::new();
    let mut updates = Vec::new();
    let mut wiggle = Vec::new();
    let mut flags = Vec::new();
    let mut named = Vec::new();
    let mut names = Vec::new();
    let mut parent_names = Vec::new();
    let mut bone_bounds = Vec::new();
    let mut bone_bounds_generations = Vec::new();
    let mut vertex_bounds = Vec::new();
    let mut bone_generations = Vec::new();
    let mut unique_names = BTreeSet::new();
    for &p in &pointers {
        let b = *by_pointer
            .get(&p)
            .context("ordered retained bone has no source row")?;
        let r = raw(field(b, "raw")?)?;
        ensure!(
            r.len() == 0x16c && word(&r, 0x1c) == skin && u32_(field(b, "skin")?)? == skin,
            "retained bone/skin backlink differs"
        );
        let name = vector(field(b, "name")?, &r, 0, 1, 4096)?;
        let parent_name = vector(field(b, "parent_name")?, &r, 0x10, 1, 4096)?;
        ensure!(
            !name.is_empty()
                && !name.contains(&0)
                && !parent_name.contains(&0)
                && unique_names.insert(name.clone()),
            "invalid or duplicate retained bone name"
        );
        names.push(name);
        parent_names.push(parent_name);
        for (key, off) in [
            ("next_sibling", 0x20),
            ("parent", 0x24),
            ("first_child", 0x28),
            ("priority_bits", 0x68),
            ("wiggle_amplitude_bits", 0x7c),
            ("attachment_matrix_flags", 0xc0),
            ("wiggle_scale_bits", 0xe8),
            ("update_bits", 0x130),
            ("bounds_generation_bits", 0x134),
            ("geometry_stamp_bits", 0x150),
        ] {
            compare_word(b, key, &r, off)?;
        }
        for (key, off, n) in [
            ("local_pose_bits", 0x30, 7),
            ("absolute_pose_bits", 0x4c, 7),
            ("flags_bits", 0x6c, 3),
            ("attachment_matrix_bits", 0x80, 16),
            ("rotation_matrix_bits", 0xc4, 9),
            ("bounds_bits", 0x138, 6),
            ("vertex_bounds_bits", 0x154, 6),
        ] {
            compare_bits(field(b, key)?, &r, off, n)?;
        }
        parents.push(ident(word(&r, 0x24))?);
        next.push(ident(word(&r, 0x20))?);
        child.push(ident(word(&r, 0x28))?);
        let local = floats::<7>(&r, 0x30)?;
        let abs = floats::<7>(&r, 0x4c)?;
        let flag = std::array::from_fn(|i| word(&r, 0x6c + 4 * i));
        locals.push(BoneInput {
            local: LocalBone {
                translation: local[..3].try_into().unwrap(),
                rotation: local[3..].try_into().unwrap(),
            },
            priority: word(&r, 0x68) as i32,
            blend_marker: flag[2],
        });
        absolute.push(AbsoluteBone {
            translation: abs[..3].try_into().unwrap(),
            rotation: abs[3..].try_into().unwrap(),
        });
        updates.push(word(&r, 0x130));
        flags.push(flag);
        ensure!(
            r[0x12c] <= 1 && u32_(field(b, "named_byte")?)? == u32::from(r[0x12c]),
            "retained named classification differs"
        );
        named.push(r[0x12c] != 0);
        let wr = array(field(b, "wiggle_records_bits")?)?;
        ensure!(wr.len() == 4, "retained noise record count differs");
        let mut records = [[0; 4]; 4];
        for (i, row) in wr.iter().enumerate() {
            let a = bits::<4>(row)?;
            for (j, &v) in a.iter().enumerate() {
                ensure!(
                    v == word(&r, 0xec + i * 16 + j * 4),
                    "retained wiggle bytes differ"
                );
                records[i][j] = v as i32;
            }
        }
        let amplitude = f32::from_bits(word(&r, 0x7c));
        ensure!(amplitude.is_finite(), "nonfinite retained wiggle amplitude");
        wiggle.push(WiggleState {
            gradient_scale: word(&r, 0xe8) as i32,
            amplitude,
            records,
        });
        bone_bounds.push(bounds(&r, 0x138)?);
        bone_bounds_generations.push(word(&r, 0x134));
        vertex_bounds.push(bounds(&r, 0x154)?);
        bone_generations.push(word(&r, 0x150));
    }
    ensure!(
        parents[root].is_none()
            && next[root].is_none()
            && parents.iter().filter(|p| p.is_none()).count() == 1,
        "retained skeleton root topology differs"
    );
    let mut reached = BTreeSet::from([root]);
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        let mut c = child[parent];
        let mut siblings = BTreeSet::new();
        while let Some(i) = c {
            ensure!(
                siblings.insert(i) && reached.insert(i) && parents[i] == Some(parent),
                "cyclic or inconsistent retained bone topology"
            );
            queue.push(i);
            c = next[i];
        }
    }
    ensure!(
        reached.len() == pointers.len(),
        "retained topology does not cover bone vector"
    );
    for (i, p) in parents.iter().enumerate() {
        if let Some(p) = p {
            ensure!(
                *p < i && parent_names[i] == names[*p],
                "retained source parent order/name differs"
            );
        }
    }
    for (key, off) in [
        ("hierarchy_dirty_bits", 0x94),
        ("geometry_dirty_bits", 0x98),
        ("evaluated_generation_bits", 0xa0),
        ("pose_generation_bits", 0xa4),
        ("bounds_generation_bits", 0xa8),
        ("vertex_generation_bits", 0xac),
        ("reserved_generation_bits", 0xb0),
        ("generation_bits", 0xb4),
        ("last_clock_bits", 0xb8),
    ] {
        compare_word(s, key, &sr, off)?;
    }
    compare_bits(field(s, "pose_matrix_bits")?, &sr, 0x1c, 16)?;
    compare_word(s, "pose_matrix_flags", &sr, 0x5c)?;
    compare_bits(field(s, "bounds_bits")?, &sr, 0xbc, 6)?;
    ensure!(
        word(&sr, 0x94) <= 1 && word(&sr, 0x98) <= 1 && !animator.busy(),
        "retained skin pass/dirty state unsupported"
    );
    let av = serde_json::to_value(&animator)?;
    for (key, off) in [("generation", 0xb4), ("pose_generation", 0xa4)] {
        ensure!(
            u32_(field(&av, key)?)? == word(&sr, off),
            "imported Animator generation differs from skin"
        );
    }
    ensure!(
        field(&av, "dirty")?.as_bool() == Some(word(&sr, 0x94) != 0)
            && field(&av, "last_clock")?.as_i64() == Some(i64::from(word(&sr, 0xb8) as i32)),
        "imported Animator clock/dirty differs from skin"
    );
    let mut anchor_rows = BTreeMap::new();
    for a in &row.anchors {
        ensure!(
            anchor_rows.insert(u32_(field(a, "pointer")?)?, a).is_none(),
            "duplicate retained anchor row"
        );
    }
    let ca = field(c, "anchors")?;
    let mut anchors = BTreeMap::new();
    let mut used_anchors = BTreeSet::new();
    for (name, off) in [
        ("head", 0x144),
        ("hand", 0x140),
        ("pelvis", 0x148),
        ("forward", 0x14c),
    ] {
        let p = u32_(field(ca, name)?)?;
        ensure!(
            p == word(&ar, off),
            "retained anchor/animator pointer differs"
        );
        let anchor = if p == 0 {
            None
        } else {
            used_anchors.insert(p);
            let a = anchor_rows
                .get(&p)
                .context("retained anchor owner absent")?;
            let b = raw(field(a, "raw")?)?;
            ensure!(
                b.len() == 72 && word(&b, 0) == u32_(field(a, "bone")?)?,
                "retained anchor bone differs"
            );
            compare_bits(field(a, "local_matrix_bits")?, &b, 4, 16)?;
            compare_word(a, "flags_bits", &b, 68)?;
            Some(Anchor {
                native_pointer: p,
                bone: ident(word(&b, 0))?.context("anchor has null bone")?,
                local: Matrix {
                    values: floats::<16>(&b, 4)?,
                    kind: word(&b, 68),
                },
            })
        };
        anchors.insert(name.to_string(), anchor);
    }
    ensure!(
        used_anchors.len() == anchor_rows.len(),
        "unreferenced retained anchor source owner"
    );
    Ok((object,Imported{
        native:NativeBinding{person,animator:ap,model,skin,bone_pointers:pointers,bone_names:names,parent_names,lookup,root},
        animator,bones:locals,hierarchy:Hierarchy{parents,absolute,updates,evaluated_generation:word(&sr,0xa0)},
        // Geometry's names differ from the raw observer labels: +ac is the
        // current attachment-bounds generation; +a8 is its evaluated stamp;
        // +b0 is the vertex/bounds stamp compared against Animator's +b4.
        geometry:Geometry{dirty:word(&sr,0x98)!=0,generation:word(&sr,0xac),evaluated_generation:word(&sr,0xa8),bounds_generation:word(&sr,0xb0),bone_generations},
        wiggle,flags,named_bone_flags:named,bone_bounds,bone_bounds_generations,bone_vertex_bounds:vertex_bounds,
        local_bounds:bounds(&sr,0xbc)?,anchors,rig:None,
        demands:vec!["Live/Instance requires separately source-qualified native skin/property-pass ownership; no handlerNone supplied".into(),
            "Rig Kind/turn lookup requires exact same-cut anchor-selection/lookup source qualification; retained matrices are not guessed Kind values".into()],
    }))
}

/// Move already-imported animator ownership into useful retained skeleton parts.
/// The explicit unpublished epoch identifies replacement bookkeeping, but does
/// NOT establish source skin/property-pass absence or historical initialization.
/// Raw input sidecars stay unchanged with the caller. No model/geometry/asset
/// loader, native constructor, pose evaluation or random draw executes here.
pub fn import(
    rows: &[Input],
    mut animators: BTreeMap<ObjectId, Animator>,
    fresh_epoch: &str,
) -> Result<BTreeMap<ObjectId, Imported>> {
    ensure!(
        !fresh_epoch.trim().is_empty()
            && fresh_epoch.len() <= 4096
            && !fresh_epoch.chars().any(char::is_control),
        "new skeleton adapter epoch absent/invalid"
    );
    ensure!(
        rows.len() <= 256 && rows.len() == animators.len(),
        "retained skeleton/Animator coverage differs"
    );
    let mut out = BTreeMap::new();
    let mut animator_owners = BTreeSet::new();
    let mut model_owners = BTreeSet::new();
    let mut skin_owners = BTreeSet::new();
    let mut people = BTreeSet::new();
    let mut bones = BTreeSet::new();
    for row in rows {
        let id = i16::try_from(u32_(field(&row.character, "object")?)?)?;
        let animator = animators
            .remove(&id)
            .context("retained skeleton has no imported Animator")?;
        let (id, owner) =
            one(row, animator).with_context(|| format!("retained skeleton object {id}"))?;
        ensure!(
            animator_owners.insert(owner.native.animator)
                && model_owners.insert(owner.native.model)
                && skin_owners.insert(owner.native.skin)
                && people.insert(owner.native.person),
            "aliased retained skeleton character"
        );
        for &p in &owner.native.bone_pointers {
            ensure!(
                bones.insert(p),
                "bone aliases distinct retained character owners"
            );
        }
        ensure!(
            out.insert(id, owner).is_none(),
            "duplicate retained skeleton object"
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn hex(b: &[u8]) -> String {
        b.iter().map(|b| format!("{b:02x}")).collect()
    }
    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn floats_(b: &mut [u8], o: usize, v: &[f32]) {
        for (i, v) in v.iter().enumerate() {
            put(b, o + 4 * i, v.to_bits());
        }
    }
    fn text(b: &mut [u8], o: usize, p: u32, s: &[u8]) -> Value {
        put(b, o, p);
        put(b, o + 4, p + s.len() as u32);
        put(b, o + 8, p + s.len() as u32);
        json!({"begin":p,"end":p+s.len()as u32,"capacity":p+s.len()as u32,"stride":1,"used_raw":hex(s)})
    }
    fn metadata(v: &mut Value, b: &[u8]) {
        v["raw"] = json!(hex(b));
        for (k, o) in [
            ("next_sibling", 0x20),
            ("parent", 0x24),
            ("first_child", 0x28),
            ("priority_bits", 0x68),
            ("wiggle_amplitude_bits", 0x7c),
            ("attachment_matrix_flags", 0xc0),
            ("wiggle_scale_bits", 0xe8),
            ("update_bits", 0x130),
            ("bounds_generation_bits", 0x134),
            ("geometry_stamp_bits", 0x150),
        ] {
            v[k] = json!(word(b, o));
        }
        for (k, o, n) in [
            ("local_pose_bits", 0x30, 7),
            ("absolute_pose_bits", 0x4c, 7),
            ("flags_bits", 0x6c, 3),
            ("attachment_matrix_bits", 0x80, 16),
            ("rotation_matrix_bits", 0xc4, 9),
            ("bounds_bits", 0x138, 6),
            ("vertex_bounds_bits", 0x154, 6),
        ] {
            v[k] = json!((0..n).map(|i| word(b, o + 4 * i)).collect::<Vec<_>>());
        }
        v["wiggle_records_bits"] = json!(
            (0..4)
                .map(|i| (0..4)
                    .map(|j| word(b, 0xec + 16 * i + 4 * j))
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        v["named_byte"] = json!(b[0x12c]);
    }
    fn bone(
        p: u32,
        skin: u32,
        name: &[u8],
        parent_name: &[u8],
        parent: u32,
        first_child: u32,
    ) -> Value {
        let mut b = vec![0u8; 0x16c];
        let name = text(&mut b, 0, p + 0x200, name);
        let parent_name = text(&mut b, 0x10, p + 0x300, parent_name);
        put(&mut b, 0x1c, skin);
        put(&mut b, 0x24, parent);
        put(&mut b, 0x28, first_child);
        floats_(&mut b, 0x30, &[0.5, 1., 2., 0., 0., 0., 1.]);
        // These are retained caches, deliberately different from recomputation.
        floats_(&mut b, 0x4c, &[9., 8., 7., 0., 0., 0., 0.99]);
        put(&mut b, 0x68, (-10000i32) as u32);
        put(&mut b, 0x6c, 1);
        put(&mut b, 0x70, 1);
        put(&mut b, 0x74, 1);
        floats_(&mut b, 0x7c, &[0.03]);
        put(&mut b, 0xe8, (-5i32) as u32);
        for i in 0..16 {
            put(&mut b, 0xec + i * 4, (i as i32 - 8) as u32);
        }
        b[0x12c] = 1;
        put(&mut b, 0x130, 45);
        put(&mut b, 0x134, 17);
        put(&mut b, 0x150, 19);
        floats_(&mut b, 0x138, &[-3., -2., -1., 4., 5., 6.]);
        floats_(&mut b, 0x154, &[-9., -8., -7., 10., 11., 12.]);
        let mut v = json!({"pointer":p,"skin":skin,"name":name,"parent_name":parent_name});
        metadata(&mut v, &b);
        v
    }
    fn fixture() -> (Input, BTreeMap<ObjectId, Animator>) {
        let person = 0x10000u32;
        let animator = 0x20000u32;
        let model = 0x30000u32;
        let skin = 0x40000u32;
        let root = 0x60000u32;
        let leaf = 0x70000u32;
        let anchor = 0x80000u32;
        let mut sr = vec![0u8; 0xd4];
        put(&mut sr, 0x60, root);
        let bp: [u32; 2] = [root, leaf];
        let lp: [u32; 3] = [root, 0, leaf];
        let mut vector_ = |off, begin, pointers: &[u32]| -> Value {
            let end = begin + pointers.len() as u32 * 4;
            put(&mut sr, off, begin);
            put(&mut sr, off + 4, end);
            put(&mut sr, off + 8, end);
            json!({"begin":begin,"end":end,"capacity":end,"stride":4,"used_raw":hex(&pointers.iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>())})
        };
        let bv = vector_(0x64, 0x51000, &bp);
        let lv = vector_(0x70, 0x52000, &lp);
        for (o, v) in [
            (0x94, 1),
            (0x98, 1),
            (0xa0, 45),
            (0xa4, 45),
            (0xa8, 23),
            (0xac, 24),
            (0xb0, 25),
            (0xb4, 26),
            (0xb8, 27),
        ] {
            put(&mut sr, o, v);
        }
        floats_(&mut sr, 0xbc, &[-5., -4., -3., 6., 7., 8.]);
        let mut s = json!({"pointer":skin,"root":root,"bone_vector":bv,"lookup_vector":lv,"ordered_bones":bp,"lookup":lp});
        for (k, o) in [
            ("hierarchy_dirty_bits", 0x94),
            ("geometry_dirty_bits", 0x98),
            ("evaluated_generation_bits", 0xa0),
            ("pose_generation_bits", 0xa4),
            ("bounds_generation_bits", 0xa8),
            ("vertex_generation_bits", 0xac),
            ("reserved_generation_bits", 0xb0),
            ("generation_bits", 0xb4),
            ("last_clock_bits", 0xb8),
            ("pose_matrix_flags", 0x5c),
        ] {
            s[k] = json!(word(&sr, o));
        }
        s["pose_matrix_bits"] = json!((0..16).map(|i| word(&sr, 0x1c + i * 4)).collect::<Vec<_>>());
        s["bounds_bits"] = json!((0..6).map(|i| word(&sr, 0xbc + i * 4)).collect::<Vec<_>>());
        s["raw"] = json!(hex(&sr));
        let mut ar = vec![0u8; 0x2a0];
        put(&mut ar, 0, 0x671ee4);
        put(&mut ar, 8, model);
        put(&mut ar, 0x138, person);
        put(&mut ar, 0x144, anchor);
        let mut mr = vec![0u8; 0xb0];
        put(&mut mr, 4, skin);
        let mut abr = vec![0u8; 72];
        put(&mut abr, 0, leaf);
        put(&mut abr, 68, 1);
        for i in 0..4 {
            put(&mut abr, 4 + i * 20, 1f32.to_bits());
        }
        let a = json!({"pointer":anchor,"raw":hex(&abr),"bone":leaf,"local_matrix_bits":(0..16).map(|i|word(&abr,4+i*4)).collect::<Vec<_>>(),"flags_bits":1});
        let c = json!({"object":1,"person":person,"animator":animator,"model":model,"skin":skin,"anchors":{"head":anchor,"hand":0,"pelvis":0,"forward":0}});
        let arow = json!({"object":1,"person":person,"animator":animator,"model":model,"skin":skin,"header_raw":hex(&ar),"model_header_raw":hex(&mr),"skin_header_raw":hex(&sr[..0xbc])});
        (Input{character:c,animator:arow,skin:s,bones:vec![bone(root,skin,b"ROOT",b"NULL",0,leaf),bone(leaf,skin,b"HEAD",b"ROOT",root,0)],anchors:vec![a],
            code:CODE.iter().map(|&(label,address,bytes,sha256)|json!({"label":label,"address":address,"bytes":bytes,"sha256":sha256})).collect()},
            BTreeMap::from([(1,Animator::new(27,26,45,true,-1))]))
    }
    fn one_(row: &Input, a: BTreeMap<ObjectId, Animator>) -> Result<Imported> {
        Ok(import(
            std::slice::from_ref(row),
            a,
            "new-unpublished-skeleton-epoch",
        )?
        .remove(&1)
        .unwrap())
    }
    fn change(row: &mut Input, index: usize, off: usize, v: u32) {
        let mut b = raw(&row.bones[index]["raw"]).unwrap();
        put(&mut b, off, v);
        metadata(&mut row.bones[index], &b);
    }

    #[test]
    fn preserves_native_pose_stamps_noise_and_aliases_without_recomputation() {
        let (row, a) = fixture();
        let before = (
            row.character.clone(),
            row.animator.clone(),
            row.skin.clone(),
            row.bones.clone(),
            row.anchors.clone(),
        );
        let animator_before = serde_json::to_value(&a[&1]).unwrap();
        let out = one_(&row, a).unwrap();
        assert_eq!(out.bones.len(), 2);
        assert_eq!(out.bones[0].priority, -10000);
        assert_eq!(out.bones[1].blend_marker, 1);
        assert_eq!(out.bones[0].local.translation, [0.5, 1., 2.]);
        assert_eq!(out.hierarchy.absolute[0].translation, [9., 8., 7.]);
        assert_eq!(
            out.hierarchy.absolute[0].rotation[3].to_bits(),
            0.99f32.to_bits()
        );
        assert_eq!(out.hierarchy.parents, vec![None, Some(0)]);
        assert_eq!(out.hierarchy.updates, vec![45, 45]);
        assert_eq!(out.hierarchy.evaluated_generation, 45);
        assert_eq!(
            (
                out.geometry.generation,
                out.geometry.evaluated_generation,
                out.geometry.bounds_generation
            ),
            (24, 23, 25)
        );
        assert_eq!(out.geometry.bone_generations, vec![19, 19]);
        assert_eq!(out.bone_bounds_generations, vec![17, 17]);
        assert_eq!(out.wiggle[0].gradient_scale, -5);
        assert_eq!(out.wiggle[0].records[0], [-8, -7, -6, -5]);
        assert_eq!(out.flags, vec![[1, 1, 1]; 2]);
        assert_eq!(out.native.lookup, vec![Some(0), None, Some(1)]);
        assert_eq!(out.anchors["head"].as_ref().unwrap().bone, 1);
        assert_eq!(out.anchors["head"].as_ref().unwrap().local.kind, 1);
        assert!(out.rig.is_none());
        assert!(out.demands.iter().any(|d| d.contains("handlerNone")));
        assert_eq!(
            serde_json::to_value(&out.animator).unwrap(),
            animator_before
        );
        assert_eq!(
            before,
            (
                row.character.clone(),
                row.animator.clone(),
                row.skin.clone(),
                row.bones.clone(),
                row.anchors.clone()
            )
        );
        let cold = serde_json::to_vec(&out).unwrap();
        assert_eq!(serde_json::from_slice::<Imported>(&cold).unwrap(), out);
    }
    #[test]
    fn rejects_foreign_missing_and_duplicate_bone_owners_without_panics() {
        let (r, a) = fixture();
        let mut bad = r.clone();
        bad.bones[1]["pointer"] = json!(0x90000);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        bad.bones.push(bad.bones[0].clone());
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        change(&mut bad, 1, 0x24, 0x90000);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        change(&mut bad, 1, 0x1c, 0x80000);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r;
        bad.bones[1]["parent"] = json!(0);
        assert!(one_(&bad, a).is_err());
    }
    #[test]
    fn rejects_cycles_wrong_parent_order_and_disconnected_nodes() {
        let (r, a) = fixture();
        let mut bad = r.clone();
        change(&mut bad, 1, 0x28, 0x60000);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        change(&mut bad, 0, 0x28, 0);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r;
        change(&mut bad, 0, 0x20, 0x70000);
        assert!(one_(&bad, a).is_err());
    }
    #[test]
    fn rejects_mutated_pose_decoding_nonfinite_values_and_source_code() {
        let (r, a) = fixture();
        let mut bad = r.clone();
        bad.bones[0]["local_pose_bits"][0] = json!(0);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        change(&mut bad, 0, 0x30, f32::NAN.to_bits());
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        bad.code[0]["sha256"] = json!("0".repeat(64));
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r;
        bad.code[1]["address"] = json!(0x5989cf);
        assert!(one_(&bad, a).is_err());
    }
    #[test]
    fn checks_animator_generation_anchor_and_exact_owner_coverage() {
        let (r, a) = fixture();
        let wrong = BTreeMap::from([(1, Animator::new(27, 27, 45, true, -1))]);
        assert!(one_(&r, wrong).is_err());
        let mut bad = r.clone();
        bad.anchors[0]["bone"] = json!(0x60000);
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        bad.anchors.clear();
        assert!(one_(&bad, a.clone()).is_err());
        let mut bad = r.clone();
        bad.anchors.push(bad.anchors[0].clone());
        assert!(one_(&bad, a.clone()).is_err());
        assert!(import(std::slice::from_ref(&r), BTreeMap::new(), "e").is_err());
        assert!(import(&[r], a, "").is_err());
    }
}
