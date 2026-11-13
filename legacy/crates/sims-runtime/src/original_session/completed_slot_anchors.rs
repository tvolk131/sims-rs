//! Observed physical-slot anchor names and ordinary marker scalars.
//! This supplements an already imported completed World; no native code runs.
use super::{completed_animations::bytes, completed_lot::Memory};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_vm::{ObjectId, World, slot_geometry::Anchor};
use std::collections::BTreeMap;

pub struct Imported {
    pub anchors: BTreeMap<ObjectId, Vec<Anchor>>,
    pub marker_values: BTreeMap<ObjectId, i32>,
}

/// Only the initialized physical HEAD record is admitted here. Other slot
/// words can contain untouched heap bytes and must remain unobserved names.
pub fn import_captured_heads(
    state: &Value,
    memory: &sims_fixtures::readset::Readset<'_>,
    world: &World,
) -> Result<BTreeMap<ObjectId, Anchor>> {
    let input = &state["domains"]["supplemental"];
    ensure!(
        input["schema"] == 1
            && input["tick"] == state["tick"]
            && input["executes_target_code"] == false,
        "current dependent slot epoch differs"
    );
    super::completed_skeletons::captured::reads(input, memory)?;
    ensure!(
        sims_fixtures::digest(&memory.read(0x58f8ff, 0x193)?)
            == "74c9d0c95ab4ab040d630b91030f41fdd9e71ee6244d44d392ab8415cfbcea52",
        "current dependent slot consumer differs"
    );
    let mut result = BTreeMap::new();
    for row in input["objects"]
        .as_array()
        .context("current slot owners absent")?
    {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("current slot id absent")?
            .try_into()?;
        let object = world
            .objects
            .get(&id)
            .context("current slot object absent")?;
        if object.person.is_none() {
            continue;
        }
        let raw = hex(&row["raw"])?;
        let body = hex(&row["slots_raw"])?;
        ensure!(
            raw.len() == 0x188
                && memory.read(pointer(&row["pointer"])?, raw.len())? == raw
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id
                && body.len() == 36 * (object.slots.len() + 1)
                && body.len() >= 108
                && word(&raw, 0x134)?.checked_sub(word(&raw, 0x130)?) == Some(body.len() as u32)
                && memory.read(word(&raw, 0x130)?, body.len())? == body,
            "current person physical/logical slot alias differs"
        );
        for (i, record) in body.chunks_exact(36).skip(1).enumerate() {
            let child = i16::from_le_bytes(record[16..18].try_into()?);
            ensure!(
                child >= 0 && object.slots[i] == (child != 0).then_some(child),
                "current slot occupant differs"
            );
        }
        let name = row["head_name"]
            .as_str()
            .context("initialized HEAD name absent")?;
        ensure!(
            name == "HEAD" && memory.read(word(&body, 72 + 32)?, 5)? == b"HEAD\0",
            "initialized HEAD record/name alias differs"
        );
        ensure!(
            result
                .insert(id, Anchor::Named(name.as_bytes().to_vec()))
                .is_none(),
            "duplicate current HEAD owner"
        );
    }
    ensure!(
        result.keys().copied().eq(world
            .objects
            .iter()
            .filter_map(|(&id, object)| object.person.is_some().then_some(id))),
        "current HEAD owner coverage differs"
    );
    Ok(result)
}
fn word(b: &[u8], i: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(i..i + 4).context("short slot owner")?.try_into()?,
    ))
}
fn hex(v: &Value) -> Result<Vec<u8>> {
    bytes(v.as_str().context("slot owner bytes absent")?)
}
fn pointer(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("slot pointer absent")?.try_into()?)
}
pub fn import(input: &Value, world: &World) -> Result<Imported> {
    ensure!(
        input["schema"] == 1
            && input["tick"] == 88448
            && input["executes_target_code"] == false
            && input["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f",
        "slot owner source profile differs"
    );
    let m = Memory::new(input)?;
    ensure!(
        sims_fixtures::digest(&m.read(0x58f8ff, 0x193)?)
            == "74c9d0c95ab4ab040d630b91030f41fdd9e71ee6244d44d392ab8415cfbcea52",
        "dependent graphic source code differs"
    );
    let mut anchors = BTreeMap::new();
    for row in input["objects"].as_array().context("slot objects absent")? {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("slot id absent")?
            .try_into()?;
        let o = world.objects.get(&id).context("slot object absent")?;
        ensure!(o.person.is_some(), "slot profile requires a person");
        let raw = hex(&row["raw"])?;
        let p = pointer(&row["pointer"])?;
        ensure!(
            raw.len() == 0x188
                && m.read(p, raw.len())? == raw
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id,
            "slot object alias differs"
        );
        let body = hex(&row["slots_raw"])?;
        ensure!(
            word(&raw, 0x134)?.checked_sub(word(&raw, 0x130)?) == Some(body.len() as u32)
                && m.read(word(&raw, 0x130)?, body.len())? == body
                && body.len() % 36 == 0
                && body.len() / 36 == o.slots.len() + 1,
            "physical/logical slot extent differs"
        );
        let names = row["names"].as_array().context("slot names absent")?;
        ensure!(names.len() == o.slots.len(), "slot anchor coverage differs");
        let mut values = vec![];
        for (i, record) in body.chunks_exact(36).skip(1).enumerate() {
            let child = i16::from_le_bytes(record[16..18].try_into()?);
            ensure!(
                child >= 0 && o.slots[i] == (child != 0).then_some(child),
                "slot occupant differs"
            );
            let name = word(record, 32)?;
            let value = if name == 0 {
                ensure!(names[i].is_null(), "null slot anchor has supplied text");
                Anchor::Absent
            } else {
                let b = hex(&names[i])?;
                ensure!(
                    b.len() < 256 && !b.contains(&0),
                    "invalid captured slot name"
                );
                let mut terminated = b.clone();
                terminated.push(0);
                ensure!(
                    m.read(name, terminated.len())? == terminated,
                    "slot name pointer/content differs"
                );
                Anchor::Named(b)
            };
            values.push(value);
        }
        ensure!(anchors.insert(id, values).is_none(), "duplicate slot owner");
    }
    ensure!(
        anchors.keys().copied().eq(world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_some())
            .map(|(&id, _)| id)),
        "person slot owner inventory differs"
    );
    let mut marker_values = BTreeMap::new();
    for row in input["ordinary"]
        .as_array()
        .context("ordinary scalar owners absent")?
    {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("ordinary id absent")?
            .try_into()?;
        ensure!(
            world.objects.get(&id).is_some_and(|o| o.person.is_none()),
            "ordinary actor differs"
        );
        let p = pointer(&row["pointer"])?;
        let raw = hex(&row["raw"])?;
        let a = pointer(&row["animator"])?;
        let ar = hex(&row["animator_raw"])?;
        ensure!(
            raw.len() == 0x188
                && ar.len() == 0x110
                && m.read(p, raw.len())? == raw
                && m.read(a, ar.len())? == ar
                && word(&raw, 0x120)? == a
                && word(&ar, 4)? == p
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id,
            "ordinary scalar aliases differ"
        );
        ensure!(
            marker_values.insert(id, word(&ar, 0x100)? as i32).is_none(),
            "duplicate ordinary scalar owner"
        );
    }
    Ok(Imported {
        anchors,
        marker_values,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private original completed physical-slot/marker owners"]
    fn actual_slot_names_require_matching_source_words_and_preserve_vm() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(32 << 20)
            .spawn(|| -> Result<()> {
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                let p = root.join("fixtures/private/goth-dependent-graphic-20261029-first");
                let refs: Value =
                    serde_json::from_slice(&std::fs::read(p.join("session-second/refs.json"))?)?;
                let saved: super::super::Saved =
                    sims_fixtures::Store::open(p.join("session-second/store"))?
                        .get_value(&serde_json::from_value(refs["session"].clone())?)?;
                let input: Value =
                    serde_json::from_slice(&std::fs::read(p.join("slot-owners-first.json"))?)?;
                let before = postcard::to_allocvec(&saved.simulation.state.vm)?;
                let imported = import(&input, &saved.simulation.state.vm.world)?;
                ensure!(
                    imported.anchors[&21][1] == Anchor::Named(b"HEAD".to_vec())
                        && imported.marker_values[&383] == 83,
                    "actual slot/marker owners lost"
                );
                for change in 0..5 {
                    let mut bad = input.clone();
                    match change {
                        0 => bad["objects"][2]["names"][1] = serde_json::json!("524f4f54"),
                        1 => bad["objects"][2]["names"][1] = Value::Null,
                        2 => {
                            bad["objects"].as_array_mut().unwrap().pop();
                        }
                        3 => bad["tick"] = 88449.into(),
                        _ => bad["ordinary"][0]["object"] = 384.into(),
                    }
                    ensure!(
                        import(&bad, &saved.simulation.state.vm.world).is_err(),
                        "slot alias mutation {change} admitted"
                    );
                }
                ensure!(
                    postcard::to_allocvec(&saved.simulation.state.vm)? == before,
                    "slot import ran/mutated VM/RNG"
                );
                Ok(())
            })?
            .join()
            .expect("slot owner test panicked")
    }
}
