//! Qualified move-only handoff of retained skeleton parts into a Live/Instance.
//!
//! This bounded profile extends a pinned main-thread caller proof with the
//! actual synchronous skin/layer/property call path. It does NOT declare an
//! event bus, queue, future callback, native initialization history, model or
//! complete Character/Resident absent/qualified. Raw evidence stays caller-owned.
use super::completed_skeletons::{Anchor, Imported as Parts, NativeBinding};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_sim::{
    head_look::rig::{Anchor as RigAnchor, Kind, Rig},
    pose::{
        animator::Animator, live::Live, model::Bounds, sampling::BoneInput, skeleton::Instance,
    },
};
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};
pub mod captured;

const PARENT: &str = "bcbced9059a201f08ab04b68ea432d1adc2cbab3a0ef098646e8056f85944e50";
const OBSERVATION: &str = "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c";
const IMAGE: &str = "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f";
const CODE: [(&str, u32, usize, &str); 11] = [
    (
        "person",
        0x45cc8c,
        616,
        "a68dcdebef223b8c275b1ea48de77156345cd4916709a12dc8be81cb2654db2d",
    ),
    (
        "ordinary",
        0x58dc72,
        7728,
        "4fb4bc597355a68e2c40d9d3ed2c8155f9bb3cefda83c8c57aa45cb8dbb7e12c",
    ),
    (
        "skin",
        0x5992b3,
        152,
        "6d19c2802e3ecabfb53eca0249df3771ce5804db261f0d7c6f919b894b8421e4",
    ),
    (
        "layer",
        0x597c4c,
        2538,
        "7207d9a6f3ee9c34816253d5a73063819a0c603aadffa26d6c4c9db857b611ca",
    ),
    (
        "bridge",
        0x587c56,
        46,
        "7fa403c146591ee053a760c7faf44295a34501ee60d11a8da9cd9a62010bc174",
    ),
    (
        "handler",
        0x5912fd,
        1218,
        "82b324bb4fb6beb966fd3c0ddf5a6ecf143eb5dee7a9bba5d8223975d606af91",
    ),
    (
        "anchor",
        0x5a1631,
        940,
        "3406629e0ac5a6fec7af1bf139402f0726079fa977e393868426948f9c4ecfb2",
    ),
    (
        "record",
        0x5a19dd,
        63,
        "fe57dcf7be1881a615e4fb1a0bb1d9ddf8a6f938529df057bcbe4baae6cef7d7",
    ),
    (
        "lookup",
        0x59910a,
        30,
        "873fd133a47ddb6006be53832aa2fb46fabdd81247d56d959a8b3111d54cec31",
    ),
    (
        "head",
        0x590768,
        791,
        "037ab266849d2efbfe539b286eda2ff377c24329c05d0a422620dde2bb9f929b",
    ),
    (
        "restore",
        0x5904d2,
        662,
        "22d3d88fdf6666482dff018df46dd0fb0634e34bf5fcb32d3a199086e46ab8e3",
    ),
];
const IDENTITY: [u32; 16] = [
    0x3f800000, 0, 0, 0, 0, 0x3f800000, 0, 0, 0, 0, 0x3f800000, 0, 0, 0, 0, 0x3f800000,
];
const PALM: [u32; 16] = [
    0xbf800000, 0x362a2217, 0, 0, 0xb62a2217, 0xbf800000, 0, 0, 0, 0, 0x3f800000, 0, 0x3f000000, 0,
    0, 0x3f800000,
];
const TOP: [u32; 16] = [
    0x35aa2217, 0xbf800000, 0, 0, 0x3f800000, 0x35aa2217, 0, 0, 0, 0, 0x3f800000, 0, 0x3f4ccccd, 0,
    0, 0x3f800000,
];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    pub name: String,
    pub address: u32,
    pub bytes: usize,
    pub raw: String,
    pub sha256: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Callback {
    pub address: u32,
    pub raw: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Literal {
    pub name: String,
    pub address: u32,
    pub raw: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub object: ObjectId,
    pub person: u32,
    pub animator: u32,
    pub model: u32,
    pub skin: u32,
    pub animator_raw: String,
    pub model_raw: String,
    pub skin_raw: String,
    pub tick_slot_address: u32,
    pub tick_slot_raw: String,
    pub head_temp_flag: u8,
}
/// New same-cut source proof; this is NOT the old construction/menu absence
/// relabeled as a skin proof. All code bytes and inherited source pins are
/// mandatory, including the ordinary animator and actual property bridge.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub schema: u32,
    pub contract: String,
    pub tick: u64,
    pub observation_sha256: String,
    pub image_sha256: String,
    pub parent_closure: Value,
    pub parent_closure_sha256: String,
    pub code: Vec<Witness>,
    pub callback: Callback,
    pub rig_labels: Vec<Literal>,
    pub actors: Vec<Actor>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Retained {
    pub native: NativeBinding,
    pub anchors: BTreeMap<String, Option<Anchor>>,
    pub bone_bounds: Vec<Bounds>,
    pub bone_bounds_generations: Vec<u32>,
    pub bone_vertex_bounds: Vec<Bounds>,
    pub local_bounds: Bounds,
    pub callback_function: u32,
    pub callback_context: u32,
    pub inherited_component_demands: Vec<String>,
    pub parent_closure_sha256: String,
    pub observation_sha256: String,
    pub image_sha256: String,
    pub adapter_epoch: String,
}
/// Only skin/Instance and Rig are composed. This carries no Model, appearance,
/// Character, Resident, World, native continuation or tick-parity admission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qualified {
    pub instance: Instance,
    pub rig: Rig,
    pub retained: Retained,
}
fn field<'a>(v: &'a Value, k: &str) -> Result<&'a Value> {
    v.get(k)
        .with_context(|| format!("missing parent skin-closure field {k}"))
}
fn raw(s: &str) -> Result<Vec<u8>> {
    super::completed_animations::bytes(s)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn digest(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn parent(proof: &Input) -> Result<BTreeSet<ObjectId>> {
    ensure!(
        proof.schema == 1
            && proof.contract == "completed-native-skin-closure-v1"
            && proof.tick == 88448,
        "unsupported retained skin closure profile"
    );
    ensure!(
        proof.observation_sha256 == OBSERVATION
            && proof.image_sha256 == IMAGE
            && proof.parent_closure_sha256 == PARENT
            && digest(&serde_json::to_vec(&proof.parent_closure)?) == PARENT,
        "retained skin parent/source epoch binding differs"
    );
    let p = &proof.parent_closure;
    ensure!(
        field(p, "status")?.as_str() == Some("source_closure_observed")
            && field(p, "tick")?.as_u64() == Some(proof.tick)
            && field(p, "observation_sha256")?.as_str() == Some(OBSERVATION)
            && field(p, "image_sha256")?.as_str() == Some(IMAGE)
            && field(p, "executes_target_code")?.as_bool() == Some(false),
        "retained skin parent caller closure differs"
    );
    // The pinned parent includes the fixed source unwind, exact frozen stack,
    // excluded4CC message-poll branch and both344-object identity sets. It is
    // consumed as a known main-thread C-stack fact, NOT global event absence.
    let ids: Vec<ObjectId> =
        serde_json::from_value(field(p, "construction_absent_objects")?.clone())?;
    let consumer: Vec<ObjectId> =
        serde_json::from_value(field(p, "native_consumer_absent_objects")?.clone())?;
    let set: BTreeSet<_> = ids.iter().copied().collect();
    ensure!(
        ids.len() == 344 && set.len() == 344 && ids == consumer,
        "retained skin parent object coverage differs"
    );
    Ok(set)
}
fn witnesses(code: &[Witness]) -> Result<()> {
    ensure!(
        code.len() == CODE.len(),
        "retained skin code coverage differs"
    );
    let mut names = BTreeSet::new();
    for w in code {
        ensure!(
            names.insert(&w.name),
            "duplicate retained skin code witness"
        );
        let &(name, address, count, sha) = CODE
            .iter()
            .find(|(name, _, _, _)| *name == w.name)
            .context("unknown retained skin code witness")?;
        let b = raw(&w.raw)?;
        ensure!(
            w.name == name
                && w.address == address
                && w.bytes == count
                && b.len() == count
                && w.sha256 == sha
                && digest(&b) == sha,
            "retained synchronous skin code witness differs: {name}"
        );
    }
    Ok(())
}

/// Reuse the pinned completed main-thread caller fact for other ordinary
/// synchronous skin owners. This proves no active native C-stack consumer for
/// the returned objects; it does not assert global event/queue absence.
pub(super) fn synchronous_boundary(proof: &Input) -> Result<BTreeSet<ObjectId>> {
    let objects = parent(proof)?;
    witnesses(&proof.code)?;
    Ok(objects)
}
fn literals(rows: &[Literal]) -> Result<()> {
    let mut identity = IDENTITY
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect::<Vec<_>>();
    identity.extend(1u32.to_le_bytes());
    let expected: [(&str, u32, &[u8]); 6] = [
        ("Palm", 0x6d1e58, b"Palm\0"),
        ("Top", 0x6d1e54, b"Top\0"),
        ("HeadFake", 0x6d1a64, b"HeadFake\0"),
        ("Shoulder", 0x6d1e48, b"Shoulder\0"),
        ("PELVIS", 0x6c45ac, b"PELVIS\0"),
        ("origin_identity", 0x6fe720, &identity),
    ];
    ensure!(
        rows.len() == expected.len(),
        "retained rig literal coverage differs"
    );
    let mut names = BTreeSet::new();
    for r in rows {
        ensure!(names.insert(&r.name), "duplicate rig literal");
        let (_, address, bytes) = expected
            .iter()
            .find(|(n, _, _)| *n == r.name)
            .context("unknown rig literal")?;
        ensure!(
            r.address == *address && raw(&r.raw)? == *bytes,
            "native rig literal differs"
        );
    }
    Ok(())
}
fn anchor_kind(a: &Anchor) -> Result<Kind> {
    let b = a.local.values.map(f32::to_bits);
    if a.local.kind == 1 && b == IDENTITY {
        return Ok(Kind::Origin);
    }
    ensure!(a.local.kind == 0, "unsupported native rig matrix cache tag");
    if b == PALM {
        return Ok(Kind::Palm);
    }
    if b == TOP {
        return Ok(Kind::Top);
    }
    let mut forward = TOP;
    forward[14] = 100f32.to_bits();
    if b == forward {
        return Ok(Kind::HeadForward);
    }
    let mut shoulder = PALM;
    shoulder[12] = 0;
    shoulder[14] = 1f32.to_bits();
    if b == shoulder {
        return Ok(Kind::Shoulder);
    }
    bail!("native rig matrix does not match an exact source-supported recipe")
}
/// Decode the observed anchor/lookup recipe without constructing a Live skin
/// or claiming source callback/pass absence. Useful for endpoint comparison.
pub fn rig_from_parts(parts: &Parts) -> Result<Rig> {
    let names: [&str; 4] = ["head", "hand", "pelvis", "forward"];
    ensure!(
        parts.anchors.len() == 4 && names.iter().all(|n| parts.anchors.contains_key(*n)),
        "native rig anchor coverage differs"
    );
    let get = |name: &str| -> Result<Option<RigAnchor>> {
        parts.anchors[name]
            .as_ref()
            .map(|a| {
                ensure!(
                    a.bone < parts.bones.len() && a.native_pointer != 0,
                    "native rig bone owner absent"
                );
                Ok(RigAnchor {
                    bone: a.bone,
                    kind: anchor_kind(a)?,
                })
            })
            .transpose()
    };
    let turn = *parts
        .native
        .lookup
        .get(31)
        .context("native turn-bone lookup31 is absent")?;
    let turn = turn.context("native turn-bone lookup31 is null")?;
    ensure!(
        parts
            .native
            .bone_names
            .get(turn)
            .is_some_and(|n| n.as_slice() == b"HEAD"),
        "native lookup31 HEAD identity differs"
    );
    let r = Rig {
        head: get("head")?,
        hand: get("hand")?,
        pelvis: get("pelvis")?,
        forward: get("forward")?,
        turn_bone: Some(turn),
    };
    if let Some(a) = r.pelvis {
        ensure!(
            a.kind == Kind::Origin && parts.native.bone_names[a.bone] == b"PELVIS",
            "native pelvis selection differs"
        );
    }
    if let (Some(h), Some(f)) = (r.head, r.forward) {
        ensure!(
            h.bone == f.bone && matches!(f.kind, Kind::Origin | Kind::HeadForward),
            "native head/forward selection differs"
        );
    }
    Ok(r)
}
fn actor(parts: &Parts, a: &Actor) -> Result<()> {
    ensure!(
        a.person == parts.native.person
            && a.animator == parts.native.animator
            && a.model == parts.native.model
            && a.skin == parts.native.skin,
        "inherited native skin owner differs"
    );
    let ar = raw(&a.animator_raw)?;
    let mr = raw(&a.model_raw)?;
    let sr = raw(&a.skin_raw)?;
    let slot = raw(&a.tick_slot_raw)?;
    ensure!(
        ar.len() == 0x2a0 && mr.len() == 0xb0 && sr.len() == 0xd4 && slot.len() == 4,
        "native skin closure owner extent differs"
    );
    ensure!(
        word(&ar, 0) == 0x671ee4
            && word(&ar, 0x138) == a.person
            && word(&ar, 8) == a.model
            && word(&mr, 4) == a.skin
            && word(&sr, 0x9c) == a.model
            && word(&mr, 0x14) == a.animator,
        "native property bridge reverse owner differs"
    );
    ensure!(
        a.tick_slot_address == 0x671f34 && word(&slot, 0) == 0x58dc72,
        "native ordinary animator Tick slot differs"
    );
    ensure!(
        a.head_temp_flag == ar[0x150] && a.head_temp_flag == 0,
        "native temporary head rotation remains pending"
    );
    for (name, off) in [
        ("head", 0x144),
        ("hand", 0x140),
        ("pelvis", 0x148),
        ("forward", 0x14c),
    ] {
        ensure!(
            parts
                .anchors
                .get(name)
                .context("inherited native rig anchor absent")?
                .as_ref()
                .map_or(0, |a| a.native_pointer)
                == word(&ar, off),
            "inherited native anchor pointer differs"
        );
    }
    ensure!(
        parts.native.bone_pointers.get(parts.native.root) == Some(&word(&sr, 0x60))
            && parts.hierarchy.evaluated_generation == word(&sr, 0xa0)
            && parts.geometry.generation == word(&sr, 0xac)
            && parts.geometry.evaluated_generation == word(&sr, 0xa8)
            && parts.geometry.bounds_generation == word(&sr, 0xb0)
            && parts.geometry.dirty == (word(&sr, 0x98) != 0),
        "inherited native skeleton generation/identity differs"
    );
    let av = serde_json::to_value(&parts.animator)?;
    ensure!(
        !parts.animator.busy()
            && field(&av, "generation")?.as_u64() == Some(u64::from(word(&sr, 0xb4)))
            && field(&av, "pose_generation")?.as_u64() == Some(u64::from(word(&sr, 0xa4)))
            && field(&av, "last_clock")?.as_i64() == Some(i64::from(word(&sr, 0xb8) as i32))
            && field(&av, "dirty")?.as_bool() == Some(word(&sr, 0x94) != 0),
        "inherited native Animator storage differs"
    );
    ensure!(
        parts.rig.is_none(),
        "skeleton parts already carry an unrelated qualified Rig"
    );
    Ok(())
}
fn live(animator: Animator, bones: Vec<BoneInput>) -> Result<Live> {
    // Normal restore of EVERY explicit private/public field. This consumes the
    // already-imported owners; it does not use Live::new or omitted defaults.
    // handler:null is permitted ONLY after the separate source proof above.
    let value = json!({"animator":serde_json::to_value(animator)?,"bones":serde_json::to_value(bones)?,"handler":null});
    let expected = serde_json::to_vec(&value)?;
    let skin: Live = serde_json::from_value(value)?;
    ensure!(
        serde_json::to_vec(&serde_json::to_value(&skin)?)? == expected && !skin.busy(),
        "explicit retained Live restore lost state"
    );
    Ok(skin)
}

/// Consume only results of completed_skeletons::import and a separately checked
/// same-cut synchronous main-thread skin proof. No source event queue, native
/// property delivery history or future consumer is removed or declared absent.
/// All native matrices/bounds and source/adapter identities remain retained.
pub fn import(
    parts: BTreeMap<ObjectId, Parts>,
    proof: &Input,
    fresh_epoch: &str,
) -> Result<BTreeMap<ObjectId, Qualified>> {
    let inherited = parent(proof)?;
    import_fields(parts, proof, fresh_epoch, inherited)
}

fn import_fields(
    parts: BTreeMap<ObjectId, Parts>,
    proof: &Input,
    fresh_epoch: &str,
    inherited: BTreeSet<ObjectId>,
) -> Result<BTreeMap<ObjectId, Qualified>> {
    ensure!(
        !fresh_epoch.trim().is_empty()
            && fresh_epoch.len() <= 4096
            && !fresh_epoch.chars().any(char::is_control),
        "new skin adapter epoch absent/invalid"
    );
    witnesses(&proof.code)?;
    literals(&proof.rig_labels)?;
    let cb = raw(&proof.callback.raw)?;
    ensure!(
        proof.callback.address == 0x6f7690 && cb.len() == 8 && word(&cb, 0) == 0x587c56,
        "native property callback registration differs"
    );
    let context = word(&cb, 4); // Registration data, not an outstanding callback.
    ensure!(
        !parts.is_empty() && parts.len() == proof.actors.len() && parts.len() <= 256,
        "retained skin closure actor coverage differs"
    );
    let mut actors = BTreeMap::new();
    for a in &proof.actors {
        ensure!(
            a.object > 0 && inherited.contains(&a.object) && actors.insert(a.object, a).is_none(),
            "native skin closure identity coverage differs"
        );
    }
    let mut out = BTreeMap::new();
    for (id, parts) in parts {
        actor(
            &parts,
            actors
                .get(&id)
                .context("native skin closure actor absent")?,
        )?;
        let rig = rig_from_parts(&parts)?;
        let Parts {
            native,
            animator,
            bones,
            hierarchy,
            geometry,
            wiggle,
            flags,
            named_bone_flags,
            bone_bounds,
            bone_bounds_generations,
            bone_vertex_bounds,
            local_bounds,
            anchors,
            rig: _,
            demands,
        } = parts;
        let instance = Instance {
            skin: live(animator, bones)?,
            hierarchy,
            geometry,
            wiggle,
            flags,
            named_bone_flags,
        };
        out.insert(
            id,
            Qualified {
                instance,
                rig,
                retained: Retained {
                    native,
                    anchors,
                    bone_bounds,
                    bone_bounds_generations,
                    bone_vertex_bounds,
                    local_bounds,
                    callback_function: word(&cb, 0),
                    callback_context: context,
                    inherited_component_demands: demands,
                    parent_closure_sha256: proof.parent_closure_sha256.clone(),
                    observation_sha256: proof.observation_sha256.clone(),
                    image_sha256: proof.image_sha256.clone(),
                    adapter_epoch: fresh_epoch.to_string(),
                },
            },
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_sim::{
        head_look::rig::AbsoluteBone,
        pose::{
            LocalBone,
            model::Matrix,
            refresh::Hierarchy,
            skeleton::{Geometry, WiggleState},
        },
    };
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    fn put(b: &mut [u8], o: usize, n: u32) {
        b[o..o + 4].copy_from_slice(&n.to_le_bytes());
    }
    fn native_anchor(p: u32, bone: usize, values: [u32; 16], kind: u32) -> Anchor {
        Anchor {
            native_pointer: p,
            bone,
            local: Matrix {
                values: values.map(f32::from_bits),
                kind,
            },
        }
    }
    fn fixture() -> (Parts, Actor) {
        let person = 0x10000u32;
        let animator = 0x20000u32;
        let model = 0x30000u32;
        let skin = 0x40000u32;
        let mut forward = TOP;
        forward[14] = 100f32.to_bits();
        let anchors = BTreeMap::from([
            ("head".into(), Some(native_anchor(0x50000, 1, TOP, 0))),
            ("hand".into(), Some(native_anchor(0x50004, 1, PALM, 0))),
            (
                "pelvis".into(),
                Some(native_anchor(0x50008, 0, IDENTITY, 1)),
            ),
            (
                "forward".into(),
                Some(native_anchor(0x5000c, 1, forward, 0)),
            ),
        ]);
        let mut lookup = vec![None; 40];
        lookup[31] = Some(1);
        let parts = Parts {
            native: NativeBinding {
                person,
                animator,
                model,
                skin,
                bone_pointers: vec![0x60000, 0x70000],
                bone_names: vec![b"PELVIS".to_vec(), b"HEAD".to_vec()],
                parent_names: vec![b"NULL".to_vec(), b"PELVIS".to_vec()],
                lookup,
                root: 0,
            },
            animator: Animator::new(5, 7, 8, false, -1),
            bones: vec![
                BoneInput {
                    local: LocalBone {
                        translation: [1., 2., 3.],
                        rotation: [0., 0., 0., 1.]
                    },
                    priority: -10000,
                    blend_marker: 1
                };
                2
            ],
            hierarchy: Hierarchy {
                parents: vec![None, Some(0)],
                absolute: vec![
                    AbsoluteBone {
                        translation: [9., 8., 7.],
                        rotation: [0., 0., 0., 0.99]
                    };
                    2
                ],
                updates: vec![8; 2],
                evaluated_generation: 8,
            },
            geometry: Geometry {
                dirty: false,
                generation: 10,
                evaluated_generation: 11,
                bounds_generation: 12,
                bone_generations: vec![13; 2],
            },
            wiggle: vec![
                WiggleState {
                    gradient_scale: -5,
                    amplitude: 0.03,
                    records: [[-1, -2, -3, -4]; 4]
                };
                2
            ],
            flags: vec![[1, 0, 1]; 2],
            named_bone_flags: vec![true; 2],
            bone_bounds: vec![
                Bounds {
                    minimum: [-3.; 3],
                    maximum: [4.; 3]
                };
                2
            ],
            bone_bounds_generations: vec![14; 2],
            bone_vertex_bounds: vec![
                Bounds {
                    minimum: [-5.; 3],
                    maximum: [6.; 3]
                };
                2
            ],
            local_bounds: Bounds {
                minimum: [-7.; 3],
                maximum: [8.; 3],
            },
            anchors,
            rig: None,
            demands: vec!["prior v1 scope".into()],
        };
        let mut ar = vec![0u8; 0x2a0];
        put(&mut ar, 0, 0x671ee4);
        put(&mut ar, 8, model);
        put(&mut ar, 0x138, person);
        for (name, off) in [
            ("head", 0x144),
            ("hand", 0x140),
            ("pelvis", 0x148),
            ("forward", 0x14c),
        ] {
            put(
                &mut ar,
                off,
                parts.anchors[name].as_ref().unwrap().native_pointer,
            );
        }
        let mut mr = vec![0u8; 0xb0];
        put(&mut mr, 4, skin);
        put(&mut mr, 0x14, animator);
        let mut sr = vec![0u8; 0xd4];
        put(&mut sr, 0x60, 0x60000);
        put(&mut sr, 0x9c, model);
        for (off, n) in [
            (0xa0, 8),
            (0xa4, 8),
            (0xac, 10),
            (0xa8, 11),
            (0xb0, 12),
            (0xb4, 7),
            (0xb8, 5),
        ] {
            put(&mut sr, off, n);
        }
        (
            parts,
            Actor {
                object: 16,
                person,
                animator,
                model,
                skin,
                animator_raw: hex(&ar),
                model_raw: hex(&mr),
                skin_raw: hex(&sr),
                tick_slot_address: 0x671f34,
                tick_slot_raw: hex(&0x58dc72u32.to_le_bytes()),
                head_temp_flag: 0,
            },
        )
    }
    fn literal_fixture() -> Vec<Literal> {
        let mut identity = IDENTITY
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        identity.extend(1u32.to_le_bytes());
        [
            ("Palm", 0x6d1e58, b"Palm\0".to_vec()),
            ("Top", 0x6d1e54, b"Top\0".to_vec()),
            ("HeadFake", 0x6d1a64, b"HeadFake\0".to_vec()),
            ("Shoulder", 0x6d1e48, b"Shoulder\0".to_vec()),
            ("PELVIS", 0x6c45ac, b"PELVIS\0".to_vec()),
            ("origin_identity", 0x6fe720, identity),
        ]
        .into_iter()
        .map(|(name, address, b)| Literal {
            name: name.into(),
            address,
            raw: hex(&b),
        })
        .collect()
    }
    #[test]
    fn rig_uses_exact_native_recipes_and_lookup_not_matrix_kind_as_enum() {
        let (p, a) = fixture();
        actor(&p, &a).unwrap();
        let r = rig_from_parts(&p).unwrap();
        assert_eq!(
            r.head,
            Some(RigAnchor {
                bone: 1,
                kind: Kind::Top
            })
        );
        assert_eq!(r.hand.unwrap().kind, Kind::Palm);
        assert_eq!(r.pelvis.unwrap().kind, Kind::Origin);
        assert_eq!(r.forward.unwrap().kind, Kind::HeadForward);
        assert_eq!(r.turn_bone, Some(1));
        let mut unknown = p.anchors["head"].clone().unwrap();
        unknown.local.values[0] = f32::from_bits(unknown.local.values[0].to_bits() ^ 1);
        assert!(anchor_kind(&unknown).is_err());
        let mut origin = p.anchors["pelvis"].clone().unwrap();
        origin.local.values[4] = -0.0;
        assert!(anchor_kind(&origin).is_err());
        let mut wrong = p.clone();
        wrong.native.lookup[31] = Some(0);
        assert!(rig_from_parts(&wrong).is_err());
        let mut wrong = p;
        wrong
            .anchors
            .get_mut("head")
            .unwrap()
            .as_mut()
            .unwrap()
            .local
            .kind = 1;
        assert!(rig_from_parts(&wrong).is_err());
    }
    #[test]
    fn live_handoff_restores_all_explicit_fields_without_pose_or_clock_work() {
        let (p, _) = fixture();
        // Synchronous pass closure is NOT queue absence. Retain even an
        // authored nonempty numbered-event FIFO through the new adapter seam.
        let mut av = serde_json::to_value(p.animator).unwrap();
        av["numbered_events"] = json!([23, -99]);
        let animator: Animator = serde_json::from_value(av).unwrap();
        let before_animator = serde_json::to_vec(&animator).unwrap();
        let before_bones = p.bones.clone();
        let live = live(animator, p.bones).unwrap();
        assert!(!live.busy());
        assert_eq!(live.bones, before_bones);
        assert_eq!(serde_json::to_vec(&live.animator).unwrap(), before_animator);
        let v = serde_json::to_value(&live).unwrap();
        assert_eq!(
            v.as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
            vec!["animator", "bones", "handler"]
        );
        assert!(v.get("handler").unwrap().is_null());
        let saved = serde_json::to_vec(&live).unwrap();
        let restored: Live = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored, live);
    }
    #[test]
    fn reverse_bridge_slot_and_temporary_head_proof_are_not_optional() {
        let (p, a) = fixture();
        let mut bad = a.clone();
        let mut mr = raw(&bad.model_raw).unwrap();
        put(&mut mr, 0x14, 0x80000);
        bad.model_raw = hex(&mr);
        assert!(actor(&p, &bad).is_err());
        let mut bad = a.clone();
        let mut sr = raw(&bad.skin_raw).unwrap();
        put(&mut sr, 0x9c, 0x80000);
        bad.skin_raw = hex(&sr);
        assert!(actor(&p, &bad).is_err());
        let mut bad = a.clone();
        bad.tick_slot_raw = hex(&0x585a46u32.to_le_bytes());
        assert!(actor(&p, &bad).is_err());
        let mut bad = a.clone();
        bad.head_temp_flag = 1;
        let mut ar = raw(&bad.animator_raw).unwrap();
        ar[0x150] = 1;
        bad.animator_raw = hex(&ar);
        assert!(actor(&p, &bad).is_err());
        let mut bad = a.clone();
        bad.person += 4;
        assert!(actor(&p, &bad).is_err());
        let mut changed = p;
        changed.geometry.generation += 1;
        assert!(actor(&changed, &a).is_err());
    }
    #[test]
    fn source_literal_mapping_keeps_native_headfake_and_origin_identity() {
        let l = literal_fixture();
        literals(&l).unwrap();
        let mut bad = l.clone();
        bad[2].name = "HeadForward".into();
        assert!(literals(&bad).is_err());
        let mut bad = l.clone();
        bad[5].raw.replace_range(32..40, "00000080");
        assert!(literals(&bad).is_err());
        let mut bad = l;
        bad[0].address += 1;
        assert!(literals(&bad).is_err());
    }
    #[test]
    fn incomplete_source_code_is_not_a_skin_absence_attestation() {
        assert!(witnesses(&[]).is_err());
        let rows: Vec<_> = CODE
            .iter()
            .map(|&(name, address, bytes, sha)| Witness {
                name: name.into(),
                address,
                bytes,
                raw: hex(&vec![0; bytes]),
                sha256: sha.into(),
            })
            .collect();
        // Correct metadata and asserted hashes cannot replace actual code bytes.
        assert!(witnesses(&rows).is_err());
    }

    /// Private evidence is intentionally not a compile-time dependency or an
    /// invented portable fixture. Run this explicitly against the kept frozen
    /// component/proof; absent files fail instead of silently skipping.
    #[test]
    #[ignore = "requires the private same-cut Goth component and source proof"]
    fn actual_frozen_goth_skin_handoff_and_negative_controls() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let v:Value=serde_json::from_slice(&std::fs::read(root.join("fixtures/private/goth-completed-stack-comparison-20261027-first/skeleton-input.json")).unwrap()).unwrap();
        let proof: Input = serde_json::from_slice(
            &std::fs::read(root.join(
                "fixtures/private/goth-character-assembly-20261027-second/skin-proof-first.json",
            ))
            .unwrap(),
        )
        .unwrap();
        let a = &v["animation_input"];
        let assets: Vec<super::super::completed_animations::AssetInput> =
            serde_json::from_value(a["payload"]["assets"].clone()).unwrap();
        let characters: Vec<super::super::completed_animations::CharacterInput> =
            serde_json::from_value(a["characters"].clone()).unwrap();
        let anims =
            super::super::completed_animations::import(&assets, &characters, IMAGE).unwrap();
        let rows: Vec<super::super::completed_skeletons::Input> =
            serde_json::from_value(v["rows"].clone()).unwrap();
        let parts = super::super::completed_skeletons::import(
            &rows,
            anims
                .characters
                .into_iter()
                .map(|(id, c)| (id, c.animator))
                .collect(),
            IMAGE,
        )
        .unwrap();
        let output = import(parts.clone(), &proof, IMAGE).unwrap();
        assert_eq!(output.len(), 3);
        assert_eq!(
            output
                .values()
                .map(|c| c.instance.skin.bones.len())
                .sum::<usize>(),
            87
        );
        for (&id, q) in &output {
            assert_eq!(q.instance.skin.bones, parts[&id].bones);
            assert_eq!(q.instance.hierarchy, parts[&id].hierarchy);
            assert!(!q.instance.skin.busy());
            assert_eq!(q.rig.turn_bone, Some(16));
            assert_eq!(q.rig.head.unwrap().kind, Kind::Top);
            assert_eq!(q.rig.hand.unwrap().kind, Kind::Palm);
            assert_eq!(q.rig.pelvis.unwrap().kind, Kind::Origin);
            assert_eq!(q.rig.forward.unwrap().kind, Kind::HeadForward);
            assert_eq!(q.retained.callback_context, 0);
        }
        let encoded = serde_json::to_vec(&output).unwrap();
        assert_eq!(
            serde_json::from_slice::<BTreeMap<ObjectId, Qualified>>(&encoded).unwrap(),
            output
        );
        let mut bad = proof.clone();
        bad.parent_closure["scope"] = json!("skin absence asserted from empty fresh queues");
        assert!(import(parts.clone(), &bad, IMAGE).is_err());
        let mut bad = proof.clone();
        bad.code[3].raw.replace_range(0..2, "00");
        assert!(import(parts.clone(), &bad, IMAGE).is_err());
        let mut bad = proof.clone();
        bad.callback.raw = hex(&[0; 8]);
        assert!(import(parts.clone(), &bad, IMAGE).is_err());
        let mut bad = proof.clone();
        bad.actors[0].head_temp_flag = 1;
        assert!(import(parts.clone(), &bad, IMAGE).is_err());
        let mut bad = proof;
        bad.actors.pop();
        assert!(import(parts, &bad, IMAGE).is_err());
    }
}
