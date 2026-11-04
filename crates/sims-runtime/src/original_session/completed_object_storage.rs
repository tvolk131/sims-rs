//! Constructorless projection of one captured epoch's object-owned storage.
//!
//! `native`, `legacy`, and `retained` remain the caller's unchanged source
//! sidecars. They also contain padding, allocation/physical sentinel metadata,
//! and owners (actions, bounds, etc.) outside `Object`; callers must keep them
//! with the completed-baseline evidence. This is not complete World admission.
//! In particular resident Program SLOT banks are not supplied by this seam.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sims_vm::{
    Definition, Object, ObjectId, career_appearance, headline, idle, motives, multitile::Part,
};
use std::collections::{BTreeMap, BTreeSet};
pub mod accessories;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub native: Value,
    pub legacy: Value,
    pub retained: Value,
    pub definition: Definition,
    pub multitile: Option<Part>,
}

fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key)
        .with_context(|| format!("missing captured field {key}"))
}
fn unsigned(v: &Value, max: u64) -> Result<u64> {
    let n = v.as_u64().context("captured unsigned integer required")?;
    ensure!(n <= max, "captured integer exceeds field width");
    Ok(n)
}
fn u32_(v: &Value) -> Result<u32> {
    Ok(unsigned(v, u32::MAX.into())? as u32)
}
fn id(v: &Value) -> Result<ObjectId> {
    let n = v.as_i64().context("captured signed identity required")?;
    Ok(i16::try_from(n).context("captured identity exceeds short")?)
}
fn array(v: &Value) -> Result<&[Value]> {
    v.as_array()
        .map(Vec::as_slice)
        .context("captured array required")
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("captured hex string required")?;
    ensure!(
        s.len() % 2 == 0 && s.len() <= 16 << 20,
        "invalid bounded captured hex"
    );
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| Ok(u8::from_str_radix(std::str::from_utf8(c)?, 16)?))
        .collect()
}
fn word(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn half(b: &[u8], off: usize) -> i16 {
    i16::from_le_bytes(b[off..off + 2].try_into().unwrap())
}
fn shorts(v: &Value, max: usize) -> Result<Vec<i16>> {
    let a = array(v)?;
    ensure!(a.len() <= max, "captured short array exceeds bound");
    a.iter()
        .map(|n| Ok(unsigned(n, u16::MAX.into())? as u16 as i16))
        .collect()
}
fn words<const N: usize>(v: &Value) -> Result<[u32; N]> {
    let a = array(v)?;
    ensure!(a.len() == N, "captured dword array has wrong length");
    let out: Vec<_> = a.iter().map(u32_).collect::<Result<_>>()?;
    Ok(out.try_into().unwrap())
}
fn vector(v: &Value, stride: usize, max: usize) -> Result<usize> {
    let begin = u32_(field(v, "begin")?)?;
    let end = u32_(field(v, "end")?)?;
    let cap = u32_(field(v, "capacity")?)?;
    ensure!(
        (begin != 0 || (end == 0 && cap == 0))
            && end >= begin
            && cap >= end
            && (end - begin) as usize % stride == 0
            && (cap - begin) as usize % stride == 0
            && (end - begin) as usize / stride <= max,
        "captured vector extent is invalid"
    );
    Ok((end - begin) as usize / stride)
}
fn raw_vector(v: &Value, raw: &[u8], off: usize) -> Result<()> {
    for (i, key) in ["begin", "end", "capacity"].into_iter().enumerate() {
        ensure!(
            u32_(field(v, key)?)? == word(raw, off + i * 4),
            "captured vector header differs from owner bytes"
        );
    }
    Ok(())
}

fn relationships(v: &Value, raw: &[u8]) -> Result<(sims_vm::relationship::Table, Vec<ObjectId>)> {
    let order: Vec<_> = array(field(v, "order")?)?
        .iter()
        .map(id)
        .collect::<Result<_>>()?;
    let rows = field(v, "rows")?
        .as_object()
        .context("captured relationship rows required")?;
    let storage = field(v, "storage")?;
    raw_vector(storage, raw, 0xe4)?;
    ensure!(
        vector(storage, 4, 32767)? == order.len() && rows.len() == order.len(),
        "relationship row count differs"
    );
    let records = array(field(storage, "records")?)?;
    ensure!(
        records.len() == order.len(),
        "relationship source-record coverage differs"
    );
    let mut out = BTreeMap::new();
    let mut pointers = BTreeSet::new();
    for (&key, record) in order.iter().zip(records) {
        let p = u32_(field(record, "record")?)?;
        ensure!(p != 0 && pointers.insert(p), "aliased relationship record");
        ensure!(
            u32_(field(record, "key")?)? as i32 == i32::from(key),
            "relationship key/order differs"
        );
        let values: Vec<i32> = serde_json::from_value(
            rows.get(&key.to_string())
                .context("relationship order has no row")?
                .clone(),
        )?;
        ensure!(
            vector(record, 4, 256)? == values.len() && out.insert(key, values).is_none(),
            "relationship row extent or uniqueness differs"
        );
    }
    Ok((out, order))
}

fn slots(v: &Value, raw: &[u8]) -> Result<Vec<Option<ObjectId>>> {
    raw_vector(v, raw, 0x130)?;
    let count = vector(v, 36, 257)?;
    ensure!(count > 0, "physical slot sentinel is absent");
    let b = bytes(field(v, "bytes")?)?;
    let ids: Vec<_> = array(field(v, "ids")?)?
        .iter()
        .map(id)
        .collect::<Result<_>>()?;
    ensure!(
        b.len() == count * 36 && ids.len() == count,
        "physical slot body extent differs"
    );
    for (i, &id) in ids.iter().enumerate() {
        ensure!(
            id == half(&b, i * 36 + 16) && id >= 0,
            "physical slot identity differs"
        );
    }
    // First physical record is the native sentinel/tile-chain link. Its ID may
    // be nonzero; it is NOT a logical containment slot or an extra occupant.
    Ok(ids
        .into_iter()
        .skip(1)
        .map(|n| (n != 0).then_some(n))
        .collect())
}

const HEADLINE_WITNESSES: [(&str, u32, usize, &str); 7] = [
    (
        "headline_ctor_unused_payload",
        0x46df5a,
        41,
        "c7b392b25fb038885419e7c35dc85eed2383c52dba36603198900781e38ac8f8",
    ),
    (
        "headline_primitive_activation",
        0x44b724,
        770,
        "016f6c45990fe8c39cf45ff5c4acbb9ea5891f3cdc44b6e696e86632e3e85c71",
    ),
    (
        "headline_sprite_payload",
        0x46dfb5,
        52,
        "50e06d1e26bd6eda5a16b541176f92b8510fb64375e9fa66af1971060e7444d2",
    ),
    (
        "headline_portrait_payload",
        0x46dfe9,
        27,
        "cf281015456392752dccea611647f8edb80c47504c915d24947f8f7c80d92d17",
    ),
    (
        "headline_duration_gate",
        0x46e004,
        62,
        "9a95a7326b6af360a0fbf72ac36b9f12ecb8096769b6143067146234f799db1f",
    ),
    (
        "headline_duration_set",
        0x46e042,
        14,
        "aebfd93a49863274a60704193e95f26c78645e770fc25423da890b4e74c7ecca",
    ),
    (
        "headline_duration_frames",
        0x46e050,
        24,
        "01b5dc6865257b1f0065dbbc39b1b3d20d7749b9ea4b8c52c5c98a9c800f3913",
    ),
];
fn headline_semantics(v: &Value) -> Result<()> {
    ensure!(
        field(v, "contract")?.as_str() == Some("sims191-dormant-headline-semantic-v1"),
        "headline semantic contract is unknown"
    );
    let code = array(field(v, "code")?)?;
    ensure!(
        code.len() == HEADLINE_WITNESSES.len(),
        "headline semantic witness coverage differs"
    );
    for (row, &(name, address, length, sha)) in code.iter().zip(&HEADLINE_WITNESSES) {
        let raw = bytes(field(row, "raw")?)?;
        ensure!(
            field(row, "name")?.as_str() == Some(name)
                && u32_(field(row, "address")?)? == address
                && unsigned(field(row, "bytes")?, u32::MAX.into())? as usize == length
                && raw.len() == length
                && field(row, "sha256")?.as_str() == Some(sha)
                && format!("{:x}", Sha256::digest(&raw)) == sha,
            "headline semantic code witness differs"
        );
    }
    Ok(())
}
fn headline(
    v: &Value,
    raw: &[u8],
    portrait: &Value,
    semantics: Option<&Value>,
) -> Result<Option<headline::State>> {
    raw_vector(v, raw, 0x148)?;
    let count = vector(v, 60, 4096)?;
    ensure!(u32_(field(v, "stride")?)? == 60, "headline stride differs");
    let b = bytes(field(v, "used_raw")?)?;
    ensure!(b.len() == count * 60, "headline body extent differs");
    ensure!(
        count <= 1,
        "multiple native headline slots need a separate retained owner"
    );
    if count == 0 {
        ensure!(portrait.is_null(), "portrait exists without headline slot");
        return Ok(None);
    }
    if word(&b, 0x10) == 0
        && word(&b, 0x14) == 0
        && word(&b, 0x18) == 0
        && word(&b, 0x20) == 0
        && word(&b, 0x34) == 0
        && b[0x38] == 0
    {
        headline_semantics(semantics.context("dormant headline semantic witnesses absent")?)?;
        ensure!(portrait.is_null(), "dormant headline has a portrait owner");
        // Source-qualified semantic normalization, NOT raw-record equality:
        // 46df5a leaves +1c/+24/+28/+2c/+30 uninitialized; 46e004 gates use on
        // duration != 0. Successful 44b724 activation overwrites that payload via
        // 46dfb5/46dfe9 and duration setters before a subsequent tick uses it.
        // Every native byte remains in the unchanged input sidecar. This inert
        // Rust payload does not assert those source allocation cells were zero.
        return Ok(Some(headline::State {
            duration: 0,
            sprite: 0,
            portrait: None,
            frames: 0,
            frame: 0,
            step: 0,
            ticks: 0,
            background: 0,
            overlay: 0,
            priority: 0,
            flag: false,
            last_update: None,
        }));
    }
    let native_sprite = word(&b, 0x14) as i32;
    let sprite = if native_sprite == -1 {
        u16::MAX
    } else {
        // The Rust65535 tag denotes a portrait; native65535 is not native-1.
        ensure!(
            native_sprite != i32::from(u16::MAX),
            "headline sprite collides with the portrait representation"
        );
        u16::try_from(native_sprite)
            .context("headline sprite cannot be represented without narrowing")?
    };
    let resource = word(&b, 0x18);
    let portrait = if native_sprite == -1 && resource != 0 {
        ensure!(
            u32_(field(portrait, "resource")?)? == resource
                && u32_(field(portrait, "definition")?)? != 0,
            "headline portrait resource owner differs"
        );
        Some(u32_(field(portrait, "guid")?)?)
    } else {
        ensure!(portrait.is_null(), "unexpected headline portrait identity");
        if native_sprite != -1 {
            ensure!(
                resource == 0,
                "sprite headline has unsupported retained resource pointer"
            );
        }
        None
    };
    // These native storage cells are dwords. Never silently truncate dormant
    // allocation contents into the current model's narrower resource IDs.
    let background = u16::try_from(word(&b, 0x2c))
        .context("headline background cannot be represented without narrowing")?;
    let overlay = u16::try_from(word(&b, 0x30))
        .context("headline overlay cannot be represented without narrowing")?;
    ensure!(b[0x38] <= 1, "noncanonical headline flag");
    Ok(Some(headline::State {
        duration: word(&b, 0x10) as i32,
        sprite,
        portrait,
        frames: word(&b, 0x1c) as i32,
        frame: word(&b, 0x20) as i32,
        step: word(&b, 0x24) as i32,
        ticks: word(&b, 0x28) as i32,
        background,
        overlay,
        priority: word(&b, 0x34) as i32,
        flag: b[0x38] != 0,
        last_update: None,
    }))
}

fn career(v: &Value, pointer: u32) -> Result<career_appearance::State> {
    let key = bytes(field(v, "career_key_raw")?)?;
    ensure!(key.len() == 4, "career key extent differs");
    let strings = field(v, "career_strings")?;
    let read = |name: &str, off: u32| -> Result<Vec<u8>> {
        let s = field(strings, name)?;
        let p = pointer.checked_add(off).context("career owner overflow")?;
        let raw = bytes(field(s, "raw")?)?;
        ensure!(
            raw.len() == 76 && u32_(field(s, "pointer")?)? == p,
            "career fixed-string owner differs"
        );
        let length = u16::from_le_bytes(raw[8..10].try_into().unwrap()) as usize;
        let capacity = u16::from_le_bytes(raw[10..12].try_into().unwrap());
        ensure!(
            word(&raw, 4)
                == p.checked_add(12)
                    .context("career inline pointer overflow")?
                && capacity == 64
                && length <= 63
                && raw[12 + length] == 0,
            "career fixed-string storage differs"
        );
        let value = bytes(field(s, "value_raw")?)?;
        ensure!(
            value == raw[12..12 + length] && !value.contains(&0),
            "career fixed-string value differs"
        );
        Ok(value)
    };
    let state = career_appearance::State {
        key: [half(&key, 0), half(&key, 2)],
        body: read("body", 0xb30)?,
        texture: read("texture", 0xb7c)?,
        extra: read("extra", 0xbc8)?,
    };
    state.validate().map_err(anyhow::Error::msg)?;
    Ok(state)
}

fn accessory_names(v: &Value, mapped: Option<&[accessories::Record]>) -> Result<Vec<String>> {
    ensure!(
        u32_(field(v, "animator_pointer")?)? != 0,
        "person accessory animator absent"
    );
    let vector_ = field(v, "accessory_vector")?;
    ensure!(
        u32_(field(vector_, "stride")?)? == 4,
        "accessory pointer stride differs"
    );
    let count = vector(vector_, 4, 1024)?;
    let raw = bytes(field(vector_, "used_raw")?)?;
    let records = array(field(v, "accessory_records")?)?;
    ensure!(
        raw.len() == count * 4 && records.len() == count,
        "accessory record coverage differs"
    );
    // A native bound skin/name/texture record is not merely a Program label.
    // The observed profile is empty; admit no nonempty record until its loaded
    // owner -> Program appearance identity is independently supplied/proved.
    if let Some(mapped) = mapped {
        ensure!(
            mapped.len() == records.len(),
            "retained accessory mapping extent differs"
        );
        for ((record, owner), p) in records.iter().zip(mapped).zip(raw.chunks_exact(4)) {
            ensure!(
                u32_(field(record, "pointer")?)? == owner.pointer
                    && word(p, 0) == owner.pointer
                    && bytes(field(record, "raw")?)? == owner.raw,
                "retained accessory record/mapping differs"
            );
        }
        return Ok(mapped.iter().map(|r| r.name.clone()).collect());
    }
    ensure!(
        count == 0,
        "nonempty native accessory binding requires a retained appearance mapping"
    );
    Ok(Vec::new())
}

fn one(row: &Input, mapped: Option<&[accessories::Record]>) -> Result<(ObjectId, u32, Object)> {
    let n = &row.native;
    let l = &row.legacy;
    let r = &row.retained;
    let object = id(field(n, "object")?)?;
    let pointer = u32_(field(n, "pointer")?)?;
    ensure!(
        object > 0
            && pointer != 0
            && id(field(l, "id")?)? == object
            && id(field(r, "id")?)? == object
            && u32_(field(l, "pointer")?)? == pointer
            && u32_(field(r, "pointer")?)? == pointer,
        "captured object identities differ"
    );
    let raw = bytes(field(n, "base_raw")?)?;
    ensure!(
        raw.len() == 0x188 && bytes(field(l, "base_storage")?)? == raw,
        "captured base storage differs"
    );
    ensure!(
        half(&raw, 0xf0) == object,
        "native registered identity differs"
    );
    let def = bytes(field(n, "definition_raw")?)?;
    let binding = field(r, "binding")?;
    ensure!(
        def.len() == 0xd8 && bytes(field(binding, "definition_storage")?)? == def,
        "captured definition bytes differ"
    );
    let short_def = bytes(field(l, "definition")?)?;
    ensure!(
        short_def.len() == 0xa6 && short_def == def[..0xa6],
        "legacy definition prefix differs"
    );
    ensure!(
        !row.definition.source.trim().is_empty()
            && row.definition.guid == word(&def, 0x1c)
            && row.definition.guid == u32_(field(binding, "guid")?)?
            && row.definition.object
                == unsigned(field(binding, "chunk_id")?, u16::MAX.into())? as u16
            && row.definition.object == (u32_(field(binding, "flags")?)? >> 16) as u16
            && row.definition.slots == half(&def, 40) as u16
            && u32_(field(binding, "definition")?)? == word(&raw, 0x118)
            && u32_(field(binding, "resource")?)? == word(&raw, 0x11c)
            && word(&raw, 0x118) != 0
            && word(&raw, 0x11c) != 0,
        "captured definition/resource identity differs"
    );
    let kind = half(&def, 18);
    let group = half(&def, 20) != 0;
    let person = match word(&raw, 0) {
        0x666d34 => {
            ensure!(
                matches!(kind, 4 | 7 | 11 | 12) && !group && row.multitile.is_none(),
                "ordinary object class/kind/group differs"
            );
            false
        }
        0x666c20 => {
            ensure!(
                kind == 4 && group && row.multitile.is_some(),
                "group class/kind/part differs"
            );
            false
        }
        0x667480 => {
            ensure!(
                kind == 8 && group && row.multitile.is_some(),
                "portal group class/kind/part differs"
            );
            false
        }
        0x667378 => {
            ensure!(
                kind == 2 && !group && row.multitile.is_none(),
                "person class/kind/group differs"
            );
            true
        }
        _ => bail!("unsupported retained object class"),
    };
    let data = shorts(field(l, "data")?, 72)?;
    let temps = shorts(field(l, "temps")?, 8)?;
    let attributes = shorts(field(l, "attributes")?, 256)?;
    ensure!(
        data.len() == 72
            && temps.len() == 8
            && data
                .iter()
                .enumerate()
                .all(|(i, v)| *v == half(&raw, 0x4a + i * 2))
            && temps
                .iter()
                .enumerate()
                .all(|(i, v)| *v == half(&raw, 0x3a + i * 2))
            && data[11] == object
            && word(&raw, 0x30) as usize == attributes.len()
            && (attributes.is_empty() || word(&raw, 0x2c) != 0),
        "captured object registers/attributes differ"
    );
    let dynamic_raw = bytes(field(n, "dynamic_sprite_flags_raw")?)?;
    let dynamic_count = half(&raw, 0x38);
    ensure!(
        dynamic_count >= 0
            && dynamic_count <= 31999
            && dynamic_raw.len() == dynamic_count as usize * 2
            && (dynamic_count == 0 || word(&raw, 0x34) != 0),
        "native dynamic sprite flag storage differs"
    );
    let dynamic_sprite_flags = dynamic_raw.chunks_exact(2).map(|c| half(c, 0)).collect();
    let native_tree_error = unsigned(field(r, "tree_error")?, u16::MAX.into())? as u16 as i16;
    let native_flags = u32_(field(r, "native_flags")?)?;
    let native_keep_dynamic = unsigned(field(r, "keep_dynamic")?, u8::MAX.into())? as u8;
    ensure!(
        native_tree_error == half(&raw, 0x28) && native_flags == word(&raw, 0x114),
        "retained scalar storage differs"
    );
    let (relationships, relationship_order) = relationships(field(l, "relationships")?, &raw)?;
    let logical_slots = slots(field(r, "physical_slots")?, &raw)?;
    let container = if data[14] > 0 {
        ensure!(data[26] > 0, "contained object has no physical parent");
        Some((data[26], (data[14] - 1) as u16))
    } else {
        None
    };
    let np = field(n, "person")?;
    let lp = l.get("person");
    let (person_data, motives, motive_update, motive_changes, career_appearance, accessories) =
        if person {
            ensure!(
                np.is_object() && lp.is_some_and(Value::is_object),
                "native person storage absent"
            );
            let lp = lp.unwrap();
            let pd = shorts(field(lp, "data")?, 256)?;
            ensure!(pd.len() == 256, "person data extent differs");
            let current = words::<16>(field(lp, "motives_bits")?)?;
            let previous = words::<16>(field(lp, "previous_motives_bits")?)?;
            ensure!(
                current
                    .iter()
                    .chain(&previous)
                    .all(|b| f32::from_bits(*b).is_finite()),
                "nonfinite captured motive storage"
            );
            let continuation = field(r, "motive_continuation")?;
            let changes = field(continuation, "changes")?;
            let entries = array(field(changes, "entries")?)?;
            ensure!(
                vector(changes, 12, 4096)? == entries.len(),
                "motive delta vector extent differs"
            );
            let motive_changes = entries
                .iter()
                .map(|e| {
                    let [index, delta, target] = words::<3>(e)?;
                    ensure!(
                        matches!(index, 5..=9 | 14..=15)
                            && f32::from_bits(delta).is_finite()
                            && f32::from_bits(target).is_finite(),
                        "unsupported captured motive delta"
                    );
                    Ok(motives::Change {
                        motive: index as u8,
                        delta,
                        target,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            (
                Some(
                    pd.into_iter()
                        .enumerate()
                        .map(|(i, v)| (i as i16, v))
                        .collect(),
                ),
                Some(current),
                Some(motives::UpdateState {
                    previous,
                    last_decay: u32_(field(continuation, "last_decay")?)? as i32,
                }),
                motive_changes,
                Some(career(np, pointer)?),
                accessory_names(np, mapped)?,
            )
        } else {
            ensure!(
                np.is_null()
                    && lp.is_none_or(Value::is_null)
                    && r.get("motive_continuation").is_none(),
                "person-only storage exists on ordinary object"
            );
            (None, None, None, Vec::new(), None, Vec::new())
        };
    // New unpublished adapter bookkeeping has no native prior-run claim. These
    // Nones are NOT native absence of routing banks, cursor state, or history.
    let object_state = Object {
        native_tree_error: Some(native_tree_error),
        native_flags: Some(native_flags),
        native_keep_dynamic: Some(native_keep_dynamic),
        native_motive_scores: None,
        native_slot_geometry: None,
        routing_slots: None,
        relationships,
        relationship_order: Some(relationship_order),
        accessories,
        multitile: row.multitile.clone(),
        cursor: None,
        last_update: None,
        headline: headline(
            field(n, "headline_vector")?,
            &raw,
            field(n, "headline_portrait")?,
            n.get("headline_semantics"),
        )?,
        definition: Some(row.definition.clone()),
        attributes,
        temps: temps.try_into().unwrap(),
        dynamic_sprite_flags,
        data: data
            .into_iter()
            .enumerate()
            .map(|(i, v)| (i as i16, v))
            .collect(),
        person: person_data,
        career_appearance,
        motives,
        motive_update,
        motive_changes,
        slots: logical_slots,
        container,
        idle: Some(idle::IdleState {
            countdown: u32_(field(l, "registry_word")?)? as u16 as i16,
            flags: (u32_(field(l, "registry_word")?)? >> 16) as u16,
            last_run: None,
        }),
    };
    Ok((object, pointer, object_state))
}

/// Import exact observed owner values without construction, Init, Reset,
/// registration, RNG, VM execution, slot repair, or source-side mutation.
/// `fresh_epoch` names the caller's NEW unpublished Rust bookkeeping epoch:
/// last_update, Idle.last_run and Headline.last_update begin unknown there.
/// Source owner sidecars and their provenance remain caller-owned; this return
/// value alone is not an executable/authenticated native World or baseline.
pub fn import(rows: &[Input], fresh_epoch: &str) -> Result<BTreeMap<ObjectId, Object>> {
    import_inner(rows, fresh_epoch, None)
}
/// Nonempty accessories require actual same-cut resident appearance owners.
/// Existing callers keep the strict empty-only profile above.
pub fn import_with_accessories(
    rows: &[Input],
    fresh_epoch: &str,
    mapped: &accessories::Imported,
) -> Result<BTreeMap<ObjectId, Object>> {
    let people = rows
        .iter()
        .filter(|r| r.native["person"].is_object())
        .map(|r| id(&r.native["object"]))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        mapped.records.keys().copied().collect::<BTreeSet<_>>() == people,
        "retained accessory actor mapping coverage differs"
    );
    import_inner(rows, fresh_epoch, Some(mapped))
}
fn import_inner(
    rows: &[Input],
    fresh_epoch: &str,
    mapped: Option<&accessories::Imported>,
) -> Result<BTreeMap<ObjectId, Object>> {
    ensure!(
        !fresh_epoch.trim().is_empty()
            && fresh_epoch.len() <= 4096
            && !fresh_epoch.chars().any(char::is_control),
        "fresh object-adapter epoch is absent/invalid"
    );
    ensure!(
        rows.len() <= 32767,
        "object storage registry exceeds identity bound"
    );
    let mut out = BTreeMap::new();
    let mut pointers = BTreeSet::new();
    for row in rows {
        let (id, pointer, object) = one(
            row,
            mapped
                .and_then(|m| m.records.get(&id(&row.native["object"]).ok()?))
                .map(Vec::as_slice),
        )
        .with_context(|| format!("captured object {:?}", row.native.get("object")))?;
        ensure!(
            pointers.insert(pointer) && out.insert(id, object).is_none(),
            "duplicate object identity/native pointer"
        );
    }
    for (&id, object) in &out {
        for (slot, occupant) in object.slots.iter().enumerate() {
            if let Some(occupant) = occupant {
                ensure!(
                    *occupant != id
                        && out.get(occupant).and_then(|o| o.container) == Some((id, slot as u16)),
                    "captured physical membership is not bidirectional"
                );
            }
        }
        if let Some((parent, slot)) = object.container {
            ensure!(
                parent != id
                    && out
                        .get(&parent)
                        .and_then(|o| o.slots.get(usize::from(slot)))
                        == Some(&Some(id)),
                "captured container lacks matching physical membership"
            );
        }
        let mut seen = BTreeSet::from([id]);
        let mut container = object.container;
        while let Some((parent, _)) = container {
            ensure!(seen.insert(parent), "captured containment cycle");
            container = out
                .get(&parent)
                .context("captured physical parent is not live")?
                .container;
        }
    }
    Ok(out)
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "completed_object_storage/private_tests.rs"]
#[rustfmt::skip]
mod tests;
