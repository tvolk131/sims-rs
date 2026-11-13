//! Import one frozen epoch's resident animation component, without starting a
//! layer, advancing time, or synthesizing a full Character/Resident/Session.
//! Resident BCF is an explicit semantic transport, not an original file/cache
//! identity. Fresh LayerIds belong to an unpublished replacement adapter.
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_formats::animation::{Samples, read_bcf_native};
use sims_sim::pose::animator::{Animator, Slot};
use sims_vm::{ObjectId, program::AnimationAsset};
use std::collections::{BTreeMap, BTreeSet};
mod resident_lists;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetInput {
    pub pointer: u32,
    pub raw: String,
    pub name_raw: String,
    pub skill_raw: String,
    pub motions: Vec<MotionInput>,
    pub motion_vector_raw: String,
    pub resident_bcf: String,
    pub translation_samples_raw: String,
    pub rotation_samples_raw: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MotionInput {
    pub pointer: u32,
    pub raw: String,
    pub bone_raw: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerInput {
    pub pointer: u32,
    pub raw: String,
    pub bindings_raw: String,
    pub asset: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterInput {
    pub object: ObjectId,
    pub person_pointer: u32,
    pub animator_pointer: u32,
    pub skin_pointer: u32,
    pub animator: Value,
    pub animator_raw: String,
    pub skin_raw: String,
    pub bone_pointers: Vec<u32>,
    pub bone_lookup: Vec<u32>,
    pub numbered_events_raw: String,
    pub native_layers: Vec<LayerInput>,
    pub clock: i32,
    pub engaged: i32,
    pub animation_events: i32,
}
pub struct Character {
    pub animator: Animator,
    pub clock: i32,
    pub engaged: i32,
    pub animation_events: i32,
}
pub struct Imported {
    pub assets: BTreeMap<String, AnimationAsset>,
    pub characters: BTreeMap<ObjectId, Character>,
}

/// Qualify a correction of the two resident sample buffers, without changing
/// any live animator, immutable clip descriptor, bytecode or source epoch.
/// This repairs an importer projection; it does not migrate simulation state.
pub fn validate_sample_replacement(
    original: &Value,
    corrected: &Value,
    proof: &Value,
) -> Result<()> {
    for key in [
        "schema",
        "contract",
        "tick",
        "native_parity_ticks",
        "characters",
        "unqualified",
        "adapter_policy",
        "provenance",
    ] {
        ensure!(
            original.get(key).is_some() && original[key] == corrected[key],
            "sample correction changed non-sample input {key}"
        );
    }
    ensure!(
        original["payload"]["code"] == corrected["payload"]["code"],
        "sample correction changed original layout profile"
    );
    let witnesses = proof["code"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("sample pointer witnesses absent"))?;
    ensure!(
        witnesses.len() == 2,
        "sample pointer witness coverage differs"
    );
    for (w, (address, count, expected)) in witnesses.iter().zip([
        (
            0x59823e_u32,
            27,
            "28307598b81b8fa7edd2fee1af97cda4bdbdfa2c9a2338bfc389b494faf7ef27",
        ),
        (
            0x59834f,
            29,
            "ec309a52d5232ba329b1f090e47412f0484dd7326be7401bbc913cef2320bfc6",
        ),
    ]) {
        let raw = bytes(
            w["raw"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("sample witness bytes absent"))?,
        )?;
        ensure!(
            w["address"] == address
                && w["bytes"] == count
                && raw.len() == count
                && w["sha256"] == expected
                && format!("{:x}", Sha256::digest(&raw)) == expected,
            "native sample pointer instructions differ"
        );
    }
    let old = original["payload"]["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("original clip inventory absent"))?;
    let new = corrected["payload"]["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("corrected clip inventory absent"))?;
    let reads = corrected["payload"]["reads"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("sample source readset absent"))?;
    ensure!(
        !old.is_empty() && old.len() == new.len() && old.len() <= 256,
        "sample correction clip coverage differs"
    );
    let receipt = |address: u32, raw: &[u8], label: &str| -> Result<()> {
        let hash = format!("{:x}", Sha256::digest(raw));
        ensure!(
            reads.iter().any(|r| r["address"] == address
                && r["bytes"] == raw.len()
                && r["sha256"] == hash
                && r["label"] == label),
            "correct sample source read receipt absent at {address:08x}"
        );
        Ok(())
    };
    for (before, after) in old.iter().zip(new) {
        let mut a = before.clone();
        let mut b = after.clone();
        for field in ["translation_samples_raw", "rotation_samples_raw"] {
            ensure!(
                a.as_object_mut().and_then(|v| v.remove(field)).is_some()
                    && b.as_object_mut().and_then(|v| v.remove(field)).is_some(),
                "sample bank missing"
            );
        }
        ensure!(
            a == b,
            "sample correction changed a clip descriptor/motion/owner"
        );
        let header = bytes(
            after["raw"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("clip header absent"))?,
        )?;
        ensure!(header.len() == 0x74, "sample header extent differs");
        let address = u32::try_from(
            after["pointer"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("clip owner absent"))?,
        )?;
        receipt(address, &header, "animation.header")?;
        for (field, pointer, count, stride) in [
            ("translation_samples_raw", 0x60, 0x5c, 12),
            ("rotation_samples_raw", 0x68, 0x64, 16),
        ] {
            let raw = bytes(
                after[field]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("sample bank bytes absent"))?,
            )?;
            ensure!(
                word(&header, count) <= 65536
                    && raw.len() == word(&header, count) as usize * stride,
                "sample bank count differs"
            );
            if !raw.is_empty() {
                receipt(word(&header, pointer), &raw, "animation.samples")?;
            }
        }
    }
    Ok(())
}
pub fn bytes(s: &str) -> Result<Vec<u8>> {
    ensure!(
        s.len() % 2 == 0 && s.len() <= 16 << 20,
        "invalid bounded hex"
    );
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| Ok(u8::from_str_radix(std::str::from_utf8(b)?, 16)?))
        .collect()
}
fn word(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn float(b: &[u8], off: usize) -> f32 {
    f32::from_bits(word(b, off))
}

#[cfg(test)]
#[test]
fn captured_single_precision_decimal_survives_json_value_validation() {
    // Actual original 88449 phase, promoted exactly from native f32. The
    // default fast f64 parser rounded this one ulp upward and falsely rejected
    // an unchanged Animator. Keep the exact-value/unknown-field guard intact.
    let value: Value = serde_json::from_str("0.10005002468824387").unwrap();
    let phase = f32::from_bits(0x3dcc_e707);
    assert_eq!(value.as_f64().unwrap().to_bits(), (phase as f64).to_bits());
    assert_eq!(serde_json::to_value(phase).unwrap(), value);
}
fn bool_(b: &[u8], off: usize) -> Result<bool> {
    let v = word(b, off);
    ensure!(v <= 1, "noncanonical native boolean");
    Ok(v != 0)
}
fn words(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|c| word(c, 0)).collect()
}
fn samples<const N: usize>(s: &str, count: u32) -> Result<Vec<[f32; N]>> {
    ensure!(count <= 65536, "sample bound exceeded");
    let raw = bytes(s)?;
    ensure!(
        raw.len() == count as usize * N * 4,
        "resident sample extent differs"
    );
    raw.chunks_exact(N * 4)
        .map(|r| {
            let v = std::array::from_fn(|i| float(r, i * 4));
            ensure!(v.iter().all(|x| x.is_finite()), "nonfinite resident sample");
            Ok(v)
        })
        .collect()
}

/// All inputs must belong to the supplied immutable epoch. This validates only
/// the resident animation component; callers still owe whole-baseline admission.
pub fn import(
    assets: &[AssetInput],
    characters: &[CharacterInput],
    epoch: &str,
) -> Result<Imported> {
    import_inner(assets, characters, epoch, None)
}

/// Separate retained-list profile. Validate captured list headers, strings,
/// vectors and live cursors against observed native memory before importing.
/// This does not authenticate the capture or admit a whole Session/replay.
/// The original property-free `import` contract remains unchanged.
pub fn import_with_resident_lists(
    assets: &[AssetInput],
    characters: &[CharacterInput],
    epoch: &str,
    lists: &Value,
    cursors: &Value,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    let mut retained = resident_lists::Owners::read(lists, cursors, memory)?;
    resident_lists::backing(assets, characters, memory)?;
    let imported = import_inner(assets, characters, epoch, Some(&mut retained))?;
    retained.finish()?;
    Ok(imported)
}

fn import_inner(
    assets: &[AssetInput],
    characters: &[CharacterInput],
    epoch: &str,
    mut retained: Option<&mut resident_lists::Owners>,
) -> Result<Imported> {
    ensure!(
        epoch.len() == 64 && epoch.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid epoch identity"
    );
    ensure!(
        assets.len() <= 256 && characters.len() <= 256,
        "animation owner bound exceeded"
    );
    let mut result = Imported {
        assets: BTreeMap::new(),
        characters: BTreeMap::new(),
    };
    let mut owners = BTreeMap::new();
    for a in assets {
        let raw = bytes(&a.raw)?;
        let bcf = bytes(&a.resident_bcf)?;
        ensure!(
            raw.len() == 0x74 && a.pointer != 0 && !owners.contains_key(&a.pointer),
            "invalid/duplicate resident animation owner"
        );
        let mut clips = read_bcf_native(&bcf)?;
        ensure!(clips.len() == 1, "resident transport needs one clip");
        let clip = clips.remove(0);
        // Narrow ASCII is the qualified profile here. Other encodings demand
        // their own explicit native-name mapping, never lossy substitution.
        let name = bytes(&a.name_raw)?;
        let skill = bytes(&a.skill_raw)?;
        ensure!(
            name.is_ascii()
                && skill.is_ascii()
                && clip.name.as_bytes() == name
                && clip.skill.as_bytes() == skill,
            "resident clip identity differs"
        );
        ensure!(
            clip.duration_ms.to_bits() == word(&raw, 0x2c)
                && clip.distance.to_bits() == word(&raw, 0x30)
                && clip.moving == bool_(&raw, 0x34)?,
            "resident clip metadata differs"
        );
        ensure!(
            clip.translation_count == word(&raw, 0x5c) && clip.rotation_count == word(&raw, 0x64),
            "resident sample counts differ"
        );
        let vector = bytes(&a.motion_vector_raw)?;
        ensure!(
            a.motions.len() == clip.motions.len()
                && vector.len() == a.motions.len() * 4
                && word(&raw, 0x24).checked_sub(word(&raw, 0x20)) == Some(vector.len() as u32),
            "resident motion vector differs"
        );
        let mut motion_ids = BTreeSet::new();
        for (i, (m, c)) in a.motions.iter().zip(&clip.motions).enumerate() {
            let mr = bytes(&m.raw)?;
            ensure!(
                mr.len() == 0x34
                    && m.pointer != 0
                    && motion_ids.insert(m.pointer)
                    && word(&vector, i * 4) == m.pointer
                    && word(&mr, 0x10) == a.pointer,
                "resident motion ownership differs"
            );
            let bone = bytes(&m.bone_raw)?;
            ensure!(
                bone.is_ascii()
                    && c.bone.as_bytes() == bone
                    && c.frames == word(&mr, 0x14)
                    && c.duration_ms.to_bits() == word(&mr, 0x18),
                "resident motion metadata differs"
            );
            for (channel, flag, index) in [(c.translation, 0x1c, 0x24), (c.rotation, 0x20, 0x28)] {
                ensure!(
                    channel
                        == if bool_(&mr, flag)? {
                            Some(word(&mr, index))
                        } else {
                            None
                        },
                    "resident channel differs"
                );
            }
            if let Some(lists) = retained.as_deref_mut() {
                lists.motion(&mr, c)?;
            } else {
                ensure!(word(&mr, 0x2c) == 0
                    && word(&mr, 0x30) == 0
                    && c.properties.is_empty()
                    && c.event_lists.is_empty(),
                    "resident properties/events need another profile");
            }
        }
        let sample = Samples {
            translations: samples(&a.translation_samples_raw, clip.translation_count)?,
            rotations: samples(&a.rotation_samples_raw, clip.rotation_count)?,
        };
        let mut sample_bytes = bytes(&a.translation_samples_raw)?;
        sample_bytes.extend(bytes(&a.rotation_samples_raw)?);
        let name = clip.name.clone();
        owners.insert(a.pointer, (name.clone(), a));
        let source = format!("native-resident:{epoch}/{:08x}", a.pointer);
        ensure!(
            !result.assets.contains_key(&name),
            "distinct native assets alias one clip name"
        );
        result.assets.insert(
            name,
            AnimationAsset {
                source: format!("{source}/semantic.bcf"),
                content_hash: format!("{:x}", Sha256::digest(&bcf)),
                samples_source: format!("{source}/decoded-samples"),
                samples_hash: format!("{:x}", Sha256::digest(sample_bytes)),
                clip,
                samples: sample,
            },
        );
    }
    let mut animator_owners = BTreeSet::new();
    let mut skin_owners = BTreeSet::new();
    for c in characters {
        ensure!(
            c.object > 0
                && c.person_pointer != 0
                && c.animator_pointer != 0
                && c.skin_pointer != 0
                && animator_owners.insert(c.animator_pointer)
                && skin_owners.insert(c.skin_pointer)
                && !result.characters.contains_key(&c.object),
            "invalid/aliased character owner"
        );
        let ar = bytes(&c.animator_raw)?;
        let sr = bytes(&c.skin_raw)?;
        ensure!(
            ar.len() == 0x2a0
                && sr.len() == 0xd4
                && word(&ar, 0) == 0x671ee4
                && word(&ar, 0x138) == c.person_pointer,
            "native animator identity differs"
        );
        let mut fields = c.animator.clone();
        // Arithmetic control is a process owner, not part of the native skin
        // header. Keep it explicitly unbound until the separate hardware
        // observation is validated by the complete importer.
        ensure!(
            fields.get("fp_control").is_none_or(Value::is_null),
            "native skin input cannot supply process floating-point control"
        );
        fields["fp_control"] = Value::Null;
        ensure!(
            fields
                .get("pending_vertex_updates")
                .is_none_or(|v| v.as_object().is_some_and(|v| v.is_empty())),
            "native component cannot invent pending vertex-cache delivery"
        );
        fields["pending_vertex_updates"] = json!({});
        let animator: Animator = serde_json::from_value(fields.clone())?;
        ensure!(
            serde_json::to_value(&animator)? == fields && !animator.busy(),
            "animator fields lost or Rust update unexpectedly in flight"
        );
        let bones: BTreeMap<_, _> = c
            .bone_pointers
            .iter()
            .copied()
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect();
        ensure!(
            !bones.contains_key(&0) && bones.len() == c.bone_pointers.len() && bones.len() <= 1024,
            "invalid native bone identity map"
        );
        let bone = |p: u32| -> Result<Option<usize>> {
            if p == 0 {
                Ok(None)
            } else {
                Ok(Some(*bones.get(&p).ok_or_else(|| {
                    anyhow::anyhow!("foreign native anchor bone")
                })?))
            }
        };
        ensure!(
            c.native_layers.len() == animator.layers().len() && c.native_layers.len() <= 256,
            "native layer coverage differs"
        );
        let ids: BTreeMap<_, _> = c
            .native_layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.pointer, (i + 1) as u64))
            .collect();
        ensure!(
            ids.len() == c.native_layers.len() && !ids.contains_key(&0),
            "aliased/null native layers"
        );
        let id = |p: u32| -> Result<Value> {
            if p == 0 {
                Ok(Value::Null)
            } else {
                Ok(json!(*ids.get(&p).ok_or_else(|| anyhow::anyhow!(
                    "foreign native layer slot"
                ))?))
            }
        };
        for (i, (n, o)) in c.native_layers.iter().zip(animator.layers()).enumerate() {
            let lr = bytes(&n.raw)?;
            let bindings = bytes(&n.bindings_raw)?;
            ensure!(
                lr.len() == 0x80
                    && word(&lr, 0) == c.skin_pointer
                    && word(&lr, 4) == n.asset
                    && o.id.0 == (i + 1) as u64,
                "native layer ownership differs"
            );
            let (name, a) = owners
                .get(&n.asset)
                .ok_or_else(|| anyhow::anyhow!("native layer asset absent"))?;
            let l = &o.layer;
            let asset = &result.assets[name];
            ensure!(
                &l.name == name
                    && l.original_duration.to_bits() == asset.clip.duration_ms.to_bits()
                    && l.frames == word(&lr, 0x38),
                "native layer/asset metadata differs"
            );
            l.clock.validate().map_err(anyhow::Error::msg)?;
            let k = &l.clock;
            for (v, off) in [
                (k.current, 0x18),
                (k.previous, 0x1c),
                (k.event_time, 0x30),
                (k.previous_event_time, 0x34),
                (k.mode, 0x40),
                (k.fade_start, 0x48),
                (k.fade_duration, 0x4c),
            ] {
                ensure!(v as u32 == word(&lr, off), "native clock integer differs");
            }
            for (v, off) in [
                (k.duration, 0x20),
                (k.speed, 0x24),
                (k.phase, 0x28),
                (k.weight, 0x2c),
                (k.fade_from, 0x50),
                (k.fade_to, 0x54),
            ] {
                ensure!(
                    v.to_bits() == word(&lr, off),
                    "native clock float bits differ"
                );
            }
            ensure!(
                k.ended == bool_(&lr, 0x3c)? && k.fading == bool_(&lr, 0x44)?,
                "native clock flags differ"
            );
            for (v, off) in [
                (l.priority as u32, 0x14),
                (l.interruptible as u32, 0x68),
                (l.translation_policy as u32, 0x78),
                (l.rotation_policy as u32, 0x7c),
                (l.sampling_config, 0x5c),
                (l.frame_skip, 0x60),
                (o.anchor_state, 0x70),
            ] {
                ensure!(v == word(&lr, off), "native layer scalar differs");
            }
            ensure!(
                l.priority_override == bool_(&lr, 0x58)?
                    && l.remove_when_faded == bool_(&lr, 0x64)?
                    && o.anchor_bone == bone(word(&lr, 0x6c))?,
                "native layer flags/anchor differ"
            );
            ensure!(
                bindings.len() == l.bindings.len() * 12
                    && word(&lr, 0xc).checked_sub(word(&lr, 8)) == Some(bindings.len() as u32),
                "native binding extent differs"
            );
            for (i, b) in l.bindings.iter().enumerate() {
                let bp = word(&bindings, i * 12);
                let mp = word(&bindings, i * 12 + 4);
                ensure!(
                    bone(bp)? == Some(b.bone)
                        && b.motion < a.motions.len()
                        && mp == a.motions[b.motion].pointer,
                    "native binding identity differs"
                );
                let mr = bytes(&a.motions[b.motion].raw)?;
                let pointer = word(&bindings, i * 12 + 8);
                if let Some(lists) = retained.as_deref_mut() {
                    lists.cursor(pointer, word(&mr, 0x30), b.event_cursor)?;
                } else {
                    ensure!(pointer == 0 && b.event_cursor.is_none(), "native binding cursor needs another profile");
                }
                let bone_id = word(&mr, 0xc) as usize;
                ensure!(
                    c.bone_lookup.get(bone_id) == Some(&bp),
                    "native motion/bone lookup differs"
                );
            }
        }
        let value = &c.animator;
        for (slot, off) in [
            (Slot::Foreground, 0xc),
            (Slot::Carry, 0x13c),
            (Slot::Base, 0x14),
        ] {
            ensure!(
                json!(animator.slot(slot).map(|v| v.0)) == id(word(&ar, off))?,
                "native slot owner differs"
            );
        }
        ensure!(
            value["blend"] == id(word(&ar, 0x10))?
                && value["next_id"] == json!(ids.len() + 1)
                && value["pending_bone_updates"] == json!({})
                && value["pass"].is_null(),
            "fresh adapter ownership policy differs"
        );
        for (name, raw, off) in [("generation", &sr, 0xb4), ("pose_generation", &sr, 0xa4)] {
            ensure!(
                value[name] == json!(word(raw, off)),
                "native animator generation differs"
            );
        }
        ensure!(
            value["dirty"] == json!(bool_(&sr, 0x94)?)
                && value["last_clock"] == json!(word(&sr, 0xb8) as i32)
                && value["selection_cache"] == json!(word(&ar, 0x98) as i32)
                && value["promote_next_foreground"] == json!(bool_(&ar, 0x1a0)?),
            "native animator controls differ"
        );
        ensure!(
            value["anchor_valid"] == json!(bool_(&ar, 0x24)?)
                && value["anchor_bone"] == json!(bone(word(&ar, 0x28))?),
            "native animator anchor differs"
        );
        for (i, off) in [0x18, 0x1c, 0x20].into_iter().enumerate() {
            ensure!(
                (value["anchor_point"][i]
                    .as_f64()
                    .ok_or_else(|| anyhow::anyhow!("anchor point absent"))?
                    as f32)
                    .to_bits()
                    == word(&ar, off),
                "native anchor bits differ"
            );
        }
        let events = bytes(&c.numbered_events_raw)?;
        ensure!(
            events.len() % 4 == 0
                && word(&ar, 0x30).checked_sub(word(&ar, 0x2c)) == Some(events.len() as u32)
                && value["numbered_events"]
                    == json!(
                        words(&events)
                            .into_iter()
                            .map(|v| v as i32)
                            .collect::<Vec<_>>()
                    ),
            "native event FIFO differs"
        );
        ensure!(
            c.clock as u32 == word(&ar, 0xb4)
                && c.engaged as u32 == word(&ar, 0xa0)
                && c.animation_events as u32 == word(&ar, 0xa4),
            "native animation control words differ"
        );
        result.characters.insert(
            c.object,
            Character {
                animator,
                clock: c.clock,
                engaged: c.engaged,
                animation_events: c.animation_events,
            },
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private frozen original sample-bank correction"]
    fn actual_sample_replacement_requires_unchanged_owners_and_real_buffer_receipts() -> Result<()>
    {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |name: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(root.join(name))?)?)
        };
        let original =
            read("fixtures/private/goth-resource-closure-20261028-first/appearances-input.json")?;
        let old = &original["prototypes_input"]["resident_input"]["skin_input"]["skeleton_input"]["animation_input"];
        let correct = read(
            "fixtures/private/goth-original-replay-20261028-first/corrected-animations-third.json",
        )?;
        let proof = read(
            "fixtures/private/goth-original-replay-20261028-first/sample-buffer-offset-proof.json",
        )?;
        validate_sample_replacement(old, &correct, &proof)?;
        for kind in 0..5 {
            let mut changed = correct.clone();
            let mut p = proof.clone();
            match kind {
                0 => changed["characters"][0]["clock"] = serde_json::json!(0),
                1 => changed["provenance"]["image_sha256"] = serde_json::json!("0".repeat(64)),
                2 => changed["payload"]["reads"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|r| r["label"] != "animation.samples"),
                3 => {
                    changed["payload"]["assets"][0]["motions"][0]["bone_raw"] =
                        serde_json::json!("00")
                }
                _ => p["code"][0]["raw"] = serde_json::json!("00"),
            }
            assert!(
                validate_sample_replacement(old, &changed, &p).is_err(),
                "mutation {kind} admitted"
            );
        }
        Ok(())
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|v| format!("{v:02x}")).collect()
    }
    fn put(b: &mut [u8], o: usize, v: u32) {
        b[o..o + 4].copy_from_slice(&v.to_le_bytes())
    }
    fn fixture() -> AssetInput {
        let mut raw = vec![0; 0x74];
        let mut motion = vec![0; 0x34];
        for (o, v) in [
            (0x20, 0x5000),
            (0x24, 0x5004),
            (0x2c, 100f32.to_bits()),
            (0x5c, 1),
            (0x64, 1),
        ] {
            put(&mut raw, o, v)
        }
        for (o, v) in [
            (0x10, 0x1000),
            (0x14, 1),
            (0x18, 100f32.to_bits()),
            (0x1c, 1),
            (0x20, 1),
        ] {
            put(&mut motion, o, v)
        }
        let mut bcf = vec![];
        for v in [0u32, 0, 1] {
            bcf.extend(v.to_le_bytes())
        }
        for name in [b"clip".as_slice(), b"skill".as_slice()] {
            bcf.push(name.len() as u8);
            bcf.extend(name)
        }
        bcf.extend(&raw[0x2c..0x38]);
        for v in [1u32, 1, 1] {
            bcf.extend(v.to_le_bytes())
        }
        bcf.push(4);
        bcf.extend(b"ROOT");
        bcf.extend(&motion[0x14..0x34]);
        let floats = |v: &[f32]| {
            hex(&v
                .iter()
                .flat_map(|f| f.to_bits().to_le_bytes())
                .collect::<Vec<_>>())
        };
        AssetInput {
            pointer: 0x1000,
            raw: hex(&raw),
            name_raw: hex(b"clip"),
            skill_raw: hex(b"skill"),
            motions: vec![MotionInput {
                pointer: 0x2000,
                raw: hex(&motion),
                bone_raw: hex(b"ROOT"),
            }],
            motion_vector_raw: hex(&0x2000u32.to_le_bytes()),
            resident_bcf: hex(&bcf),
            translation_samples_raw: floats(&[-0., 2., 3.]),
            rotation_samples_raw: floats(&[0., 0., 0., 1.]),
        }
    }
    #[test]
    fn retained_payload_uses_real_native_parser_and_preserves_sample_bits() {
        let a = fixture();
        let before = a.resident_bcf.clone();
        let v = import(std::slice::from_ref(&a), &[], &"ab".repeat(32)).unwrap();
        let clip = &v.assets["clip"];
        assert_eq!(clip.samples.translations[0][0].to_bits(), (-0f32).to_bits());
        assert_eq!(clip.clip.motions[0].rotation, Some(0));
        assert_eq!(clip.clip.duration_ms.to_bits(), 100f32.to_bits());
        assert!(clip.source.starts_with("native-resident:"));
        assert_eq!(a.resident_bcf, before);
        assert!(v.characters.is_empty());
    }
    fn character() -> CharacterInput {
        let mut ar = vec![0; 0x2a0];
        let mut sr = vec![0; 0xd4];
        let mut lr = vec![0; 0x80];
        for (o, v) in [
            (0, 0x671ee4),
            (0xc, 0x7000),
            (0x138, 0x3000),
            (0xb4, 12345),
            (0xa0, 1),
            (0x98, u32::MAX),
        ] {
            put(&mut ar, o, v)
        }
        for (o, v) in [(0xb4, 7), (0xa4, 19), (0xb8, 12345)] {
            put(&mut sr, o, v)
        }
        for (o, v) in [
            (0, 0x4000),
            (4, 0x1000),
            (8, 0x8000),
            (0xc, 0x800c),
            (0x18, 12345),
            (0x1c, 12295),
            (0x20, 100f32.to_bits()),
            (0x24, (-1f32).to_bits()),
            (0x28, 0.25f32.to_bits()),
            (0x2c, 0.5f32.to_bits()),
            (0x30, 25),
            (0x34, 75),
            (0x38, 1),
            (0x40, 2),
        ] {
            put(&mut lr, o, v)
        }
        let mut animator = serde_json::to_value(Animator::new(12345, 7, 19, false, -1)).unwrap();
        animator["layers"] = json!([{"id":1,"anchor_bone":null,"anchor_state":0,"layer":{
            "name":"clip","original_duration":100.,"frames":1,"priority":0,"priority_override":false,"sampling_config":0,"frame_skip":0,"remove_when_faded":false,
            "interruptible":0,"translation_policy":0,"rotation_policy":0,"bindings":[{"bone":0,"motion":0,"event_cursor":null}],
            "clock":{"current":12345,"previous":12295,"duration":100.,"speed":-1.,"phase":0.25,"weight":0.5,"event_time":25,"previous_event_time":75,"ended":false,
                "mode":2,"fading":false,"fade_start":0,"fade_duration":0,"fade_from":0.,"fade_to":0.}}}]);
        animator["slots"] = json!([1, null, null]);
        animator["next_id"] = json!(2);
        CharacterInput {
            object: 7,
            person_pointer: 0x3000,
            animator_pointer: 0x6000,
            skin_pointer: 0x4000,
            animator,
            animator_raw: hex(&ar),
            skin_raw: hex(&sr),
            bone_pointers: vec![0x9000],
            bone_lookup: vec![0x9000],
            numbered_events_raw: String::new(),
            native_layers: vec![LayerInput {
                pointer: 0x7000,
                raw: hex(&lr),
                bindings_raw: hex(&[0x9000u32, 0x2000, 0]
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>()),
                asset: 0x1000,
            }],
            clock: 12345,
            engaged: 1,
            animation_events: 0,
        }
    }
    #[test]
    fn live_clock_phase_reverse_and_event_position_survive_import_without_restart() {
        let c = character();
        let before = c.animator.clone();
        let v = import(&[fixture()], std::slice::from_ref(&c), &"ab".repeat(32)).unwrap();
        let a = &v.characters[&7].animator;
        let k = &a.layers()[0].layer.clock;
        assert_eq!(
            (k.current, k.previous, k.event_time, k.previous_event_time),
            (12345, 12295, 25, 75)
        );
        assert_eq!(
            (k.phase.to_bits(), k.speed.to_bits()),
            (0.25f32.to_bits(), (-1f32).to_bits())
        );
        assert_eq!(serde_json::to_value(a).unwrap(), before);
        assert_eq!(c.animator, before);
        let saved = postcard::to_allocvec(a).unwrap();
        let restored: Animator = postcard::from_bytes(&saved).unwrap();
        assert_eq!(&restored, a);
    }
    #[test]
    fn conflicting_clock_slot_binding_and_adapter_pass_reject() {
        for change in 0..5 {
            let mut c = character();
            match change {
                0 => c.animator["layers"][0]["layer"]["clock"]["phase"] = json!(0.5),
                1 => c.animator["slots"] = json!([null, null, 1]),
                2 => {
                    c.native_layers[0].bindings_raw = hex(&[0x9000u32, 0xdead, 0]
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>())
                }
                3 => c.animator["pending_bone_updates"] = json!({"0":19}),
                _ => c.bone_lookup[0] = 0xdead,
            }
            assert!(
                import(&[fixture()], &[c], &"ab".repeat(32)).is_err(),
                "mutation {change}"
            );
        }
    }
    #[test]
    fn missing_or_conflicting_native_owners_channels_and_samples_are_not_defaulted() {
        for change in 0..8 {
            let mut a = fixture();
            match change {
                0 => a.translation_samples_raw.clear(),
                1 => a.rotation_samples_raw.replace_range(0..8, "0000807f"),
                2 => a.motion_vector_raw = hex(&0x3000u32.to_le_bytes()),
                3 => {
                    let mut b = bytes(&a.motions[0].raw).unwrap();
                    put(&mut b, 0x10, 0x9999);
                    a.motions[0].raw = hex(&b)
                }
                4 => {
                    let mut b = bytes(&a.motions[0].raw).unwrap();
                    put(&mut b, 0x30, 0x9999);
                    a.motions[0].raw = hex(&b)
                }
                5 => {
                    let mut b = bytes(&a.raw).unwrap();
                    put(&mut b, 0x5c, 2);
                    a.raw = hex(&b)
                }
                6 => a.name_raw = hex(b"other"),
                _ => a.raw = "00".into(),
            }
            assert!(
                import(&[a], &[], &"ab".repeat(32)).is_err(),
                "mutation {change}"
            );
        }
    }
}
