//! Join the same-cut captured World owners, including active action records.
//! This does not admit a complete native Session or execute its scheduler.
use super::{Owners, join, program};
use crate::original_session::{
    completed_actions, completed_groups, completed_invocations, completed_object_storage,
    completed_person_world,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_vm::{ObjectId, Program, World, scheduler::storage::Storage};
use std::collections::BTreeMap;

pub struct Imported {
    pub world: World,
    pub program: Program,
    pub relationship_orders: BTreeMap<i16, Vec<i16>>,
    pub external_families: Value,
    pub delivery: completed_person_world::Delivery,
    pub readset_sha256: String,
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    v[key]
        .as_array()
        .map(Vec::as_slice)
        .with_context(|| format!("captured World {key} rows absent"))
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("captured World word absent")?
        .try_into()?)
}
fn id(v: &Value) -> Result<ObjectId> {
    Ok(v.as_i64().context("captured World ID absent")?.try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::super::completed_animations::bytes(v.as_str().context("captured World bytes absent")?)
}
fn keyed<'a>(v: &'a Value, key: &str) -> Result<BTreeMap<ObjectId, &'a Value>> {
    let mut map = BTreeMap::new();
    for r in v.as_array().context("captured World inventory absent")? {
        ensure!(
            map.insert(id(&r[key])?, r).is_none(),
            "duplicate captured World owner"
        );
    }
    Ok(map)
}
fn vector(memory: &Readset<'_>, v: &Value) -> Result<Vec<u32>> {
    let p = number(&v["header"]["address"])?;
    let header = memory.read(p, 12)?;
    ensure!(
        header == bytes(&v["header"]["raw"])?,
        "captured registry header differs"
    );
    let word = |i| u32::from_le_bytes(header[i..i + 4].try_into().unwrap());
    let (a, b, c) = (word(0), word(4), word(8));
    ensure!(
        a <= b
            && b <= c
            && (a != 0 || c == 0)
            && (c - a) % 4 == 0
            && (b - a) % 4 == 0
            && c - a <= 1 << 20
            && v["begin"] == a
            && v["end"] == b
            && v["capacity"] == c,
        "captured registry vector extent differs"
    );
    let backing = memory.read(a, (c - a) as usize)?;
    ensure!(
        backing == bytes(&v["allocated"]["raw"])?,
        "captured registry backing differs"
    );
    Ok(backing[..(b - a) as usize]
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect())
}

pub fn import(
    state: &Value,
    invocations: &completed_invocations::Imported,
    actions: &completed_actions::Imported,
    person: completed_person_world::Imported,
    memory: &Readset<'_>,
    epoch: &str,
) -> Result<Imported> {
    ensure!(
        state["readset"]["sha256"] == memory.source_sha256()
            && invocations.readset_sha256 == memory.source_sha256(),
        "captured World source epoch differs"
    );
    let d = &state["domains"];
    ensure!(
        state["tick"] == state["snapshot"]["tick"]
            && d["tick"] == state["tick"]
            && d["status"] == "captured",
        "captured World completed boundary differs"
    );
    let legacy = keyed(&state["snapshot"]["objects"], "id")?;
    let retained = keyed(&state["snapshot"]["retained_owners"]["objects"], "id")?;
    let native = keyed(&d["objects"]["objects"], "object")?;
    let pointers: BTreeMap<_, _> = legacy
        .iter()
        .map(|(&id, r)| Ok((id, number(&r["pointer"])?)))
        .collect::<Result<_>>()?;
    ensure!(
        legacy.keys().eq(native.keys())
            && legacy.keys().eq(retained.keys())
            && legacy.keys().eq(invocations.objects.keys()),
        "captured World inventories differ"
    );
    let contexts: BTreeMap<_, _> = rows(&d["world"], "frame_private_contexts")?
        .iter()
        .map(|r| Ok((number(&r["resource"])?, r)))
        .collect::<Result<_>>()?;
    let mut group_inputs = Vec::new();
    for g in rows(&d["world"], "group_fields")? {
        let ident = id(&g["object"])?;
        let l = legacy
            .get(&ident)
            .context("captured group object missing")?;
        let r = retained[&ident];
        let p = pointers[&ident];
        let raw = bytes(&g["raw"])?;
        ensure!(
            g["native_pointer"] == p
                && memory.read(
                    p.checked_add(0x18c).context("group address overflow")?,
                    raw.len()
                )? == raw,
            "captured group storage differs"
        );
        let context = contexts
            .get(&number(&r["binding"]["resource"])?)
            .context("captured group resource context absent")?;
        group_inputs.push(completed_groups::Input {
            object: ident,
            native_pointer: p,
            vtable: number(&g["vtable"])?,
            private_provider: number(&context["private_file"])?,
            definition_raw: r["binding"]["definition_storage"]
                .as_str()
                .context("group definition absent")?
                .into(),
            base_raw: l["base_storage"]
                .as_str()
                .context("group base absent")?
                .into(),
            part_raw: g["raw"].as_str().context("group part absent")?.into(),
            part: serde_json::from_value(g["part"].clone())?,
        });
    }
    let parts = completed_groups::import(&group_inputs, &pointers)?;
    let mut object_inputs = Vec::new();
    for (&ident, l) in &legacy {
        let p = pointers[&ident];
        let base = memory.read(p, 0x188)?;
        ensure!(
            base == bytes(&l["base_storage"])?
                && base == bytes(&native[&ident]["base_raw"])?
                && retained[&ident]["pointer"] == p
                && native[&ident]["pointer"] == p,
            "captured object storage/readset differs"
        );
        object_inputs.push(completed_object_storage::Input {
            native: native[&ident].clone(),
            legacy: (*l).clone(),
            retained: retained[&ident].clone(),
            definition: invocations.objects[&ident].binding.definition.clone(),
            multitile: parts.get(&ident).cloned(),
        });
    }
    let accessories = completed_object_storage::accessories::import(state, memory)?;
    let objects =
        completed_object_storage::import_with_accessories(&object_inputs, epoch, &accessories)
            .context("moving World object storage")?;
    let ptrs = vector(memory, &state["threads"]["registry"]["pointer_vector"])?;
    let words = vector(memory, &state["threads"]["registry"]["word_vector"])?;
    let by_pointer: BTreeMap<_, _> = pointers.iter().map(|(&id, &p)| (p, id)).collect();
    ensure!(
        by_pointer.len() == pointers.len(),
        "aliased captured World object pointer"
    );
    let handles = ptrs
        .iter()
        .enumerate()
        .map(|(i, &p)| {
            if p == 0 {
                Ok(None)
            } else {
                let id = *by_pointer
                    .get(&p)
                    .context("registry pointer has no object")?;
                ensure!(i + 1 == id as usize, "registry slot/object ID differs");
                Ok(Some(id as u32))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let last = handles
        .iter()
        .flatten()
        .copied()
        .max()
        .context("captured World registry empty")?;
    // This carrier is only borrowed by Storage's object/Idle validator. Its
    // unrelated defaults never escape into the actual World joined below.
    let carrier = World {
        objects,
        ..World::default()
    };
    let scheduler =
        Storage::from_retained(&carrier, handles, words, last).map_err(anyhow::Error::msg)?;
    let (mut program, profiles) =
        program::join(invocations.program.clone(), &d["systems"]["body_profiles"])
            .context("moving World body profile Program owners")?;
    for (name, receipt) in accessories.sources {
        ensure!(
            program.accessories.insert(name, receipt).is_none(),
            "captured accessory source already assigned"
        );
    }
    let mut fields = d["systems"]["fields"].clone();
    let mut action_state: sims_vm::action::WorldState =
        serde_json::from_value(fields["actions"].clone())?;
    ensure!(
        action_state.actors.keys().eq(actions.actors.keys()),
        "captured action actor coverage differs"
    );
    // Native cached labels and use records have their own raw owners; the
    // older capture's structured action view deliberately omitted labels.
    action_state.actors = actions.actors.clone();
    fields["actions"] = serde_json::to_value(action_state)?;
    let world = join(
        fields,
        Owners {
            objects: carrier.objects,
            scheduler,
            person: person.state,
            body_profiles: profiles,
        },
    )?;
    for (&id, &p) in &pointers {
        let raw = memory.read(p + 0xf4, 0x20)?;
        let word = |i| i32::from_le_bytes(raw[i..i + 4].try_into().unwrap());
        let position = world
            .positions
            .get(&id)
            .context("moving World position absent")?;
        ensure!(
            position.x == word(0) && position.y == word(4) && position.level == word(0x1c),
            "moving World placement differs from native object"
        );
        ensure!(
            world
                .person
                .as_ref()
                .unwrap()
                .autonomy
                .as_ref()
                .context("moving World autonomy owner absent")?
                .placements
                .get(&id)
                == Some(position),
            "moving World/person placement owners differ"
        );
    }
    Ok(Imported {
        world,
        program,
        relationship_orders: person.relationship_orders,
        external_families: person.external_families,
        delivery: person.delivery,
        readset_sha256: memory.source_sha256().into(),
    })
}
