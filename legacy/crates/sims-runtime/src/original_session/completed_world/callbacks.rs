//! Bind supplemental physical callback bytes and fresh retained lifetimes.
//! The caller authenticates the physical archive/artifact. This checks source
//! identity and sparse overlaps, not construction absence or Session admission.
use crate::original_session::{completed_animations, completed_invocations};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_vm::{
    ObjectId, Program, World,
    retained_frames::{RetainedFrame, RetainedStack},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Main {
    pub init: u16,
    pub main: u16,
    pub owner: String,
    pub definition: u16,
}
pub struct Imported {
    pub program: Program,
    pub cached: BTreeMap<ObjectId, Main>,
}
fn n(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("native callback word absent")?
        .try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("native callback bytes absent")?)
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    v[key]
        .as_array()
        .map(Vec::as_slice)
        .context("native callback rows absent")
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
pub(super) fn overlap(memory: &Readset<'_>, address: u32, bytes: &[u8]) -> Result<()> {
    let end = u64::from(address) + bytes.len() as u64;
    ensure!(end <= 1 << 32, "native callback address overflow");
    for r in memory.coverage() {
        let a = u64::from(address).max(u64::from(r.address));
        let b = end.min(u64::from(r.address) + r.bytes.len() as u64);
        if a < b {
            ensure!(
                bytes[(a - u64::from(address)) as usize..(b - u64::from(address)) as usize]
                    == r.bytes
                        [(a - u64::from(r.address)) as usize..(b - u64::from(r.address)) as usize],
                "supplemental native callback/sparse overlap differs"
            );
        }
    }
    Ok(())
}
pub fn import(
    state: &Value,
    source: &Value,
    memory: &Readset<'_>,
    invocations: &completed_invocations::Imported,
    program: Program,
) -> Result<Imported> {
    ensure!(
        source["schema"] == 1
            && source["tick"] == state["tick"]
            && source["source_readset_sha256"] == memory.source_sha256()
            && invocations.readset_sha256 == memory.source_sha256()
            && source["physical"]["compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"]
            && source["physical"]["decoded_sha256"]
                == state["physical_baseline"]["archive"]["decoded_sha256"],
        "native callback physical/source epoch differs"
    );
    let mut tables = BTreeMap::new();
    ensure!(
        rows(source, "tables")?.len() <= 32767,
        "native callback table count exceeds bound"
    );
    for table in rows(source, "tables")? {
        let pointer = n(&table["pointer"])?;
        let h = bytes(&table["header_raw"])?;
        ensure!(h.len() == 12, "native callback header extent differs");
        let (a, b, c) = (word(&h, 0), word(&h, 4), word(&h, 8));
        ensure!(
            a >= 0x10000
                && a <= b
                && b <= c
                && (b - a) % 4 == 0
                && (c - a) % 4 == 0
                && (8..=128000).contains(&(b - a))
                && c - a <= 128000
                && table["begin"] == a
                && table["end"] == b
                && table["capacity"] == c,
            "native callback vector extent differs"
        );
        let used = bytes(&table["used_raw"])?;
        let spare = bytes(&table["spare_raw"])?;
        ensure!(
            used.len() == (b - a) as usize && spare.len() == (c - b) as usize,
            "native callback used/spare extent differs"
        );
        let mut allocated = used.clone();
        allocated.extend(spare);
        ensure!(
            table["allocated_sha256"] == sims_fixtures::digest(&allocated),
            "native callback allocation hash differs"
        );
        overlap(memory, pointer, &h)?;
        overlap(memory, a, &allocated)?;
        let functions: Vec<_> = used
            .chunks_exact(4)
            .map(|r| sims_formats::execution::ObjectFunction {
                action: u16::from_le_bytes([r[0], r[1]]),
                condition: u16::from_le_bytes([r[2], r[3]]),
            })
            .collect();
        ensure!(
            serde_json::to_value(&functions)? == table["records"],
            "native callback decoded record differs"
        );
        ensure!(
            tables.insert(pointer, functions).is_none(),
            "duplicate native callback table"
        );
    }
    let mut program = program;
    let mut cached = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in rows(source, "objects")? {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("native callback object absent")?
            .try_into()?;
        let p = n(&row["pointer"])?;
        let resource = n(&row["resource"])?;
        let table = n(&row["table"])?;
        let object = invocations
            .objects
            .get(&id)
            .context("native callback object not in current stack inventory")?;
        let raw = memory.read(p, 0x188)?;
        ensure!(
            i16::from_le_bytes(raw[0xf0..0xf2].try_into().unwrap()) == id
                && word(&raw, 0x11c) == resource
                && word(&memory.read(resource + 0x90, 4)?, 0) == table,
            "native callback object/resource/table binding differs"
        );
        let functions = tables
            .get(&table)
            .context("native callback table missing")?;
        seen.insert(table);
        let root = object
            .stack
            .as_ref()
            .and_then(|s| s.frames().first())
            .context("native callback Main root absent")?;
        ensure!(
            functions[0].action == u16::try_from(n(&row["init"])?)?
                && functions[1].action == u16::try_from(n(&row["main"])?)?
                && functions[1].action == root.location.bhav
                && root.callee == id,
            "native callback cache/root Main differs"
        );
        let d = &object.binding.definition;
        if let Some(old) = program
            .modules
            .get_mut(&d.source)
            .context("native callback module absent")?
            .object_functions
            .insert(d.object, functions.clone())
        {
            ensure!(
                old == *functions,
                "shared native definition callback cache differs"
            );
        }
        ensure!(
            cached
                .insert(
                    id,
                    Main {
                        init: functions[0].action,
                        main: functions[1].action,
                        owner: d.source.clone(),
                        definition: d.object
                    }
                )
                .is_none(),
            "duplicate native cached lifecycle owner"
        );
    }
    ensure!(
        cached.keys().eq(invocations.objects.keys()) && seen.len() == tables.len(),
        "native callback owner/table coverage differs"
    );
    Ok(Imported { program, cached })
}

/// The supplied registry already owns fresh unpublished replacement handles.
/// These are not native generations; no raw header or hot frame is rewritten.
pub fn bind_lifetimes(
    invocations: &completed_invocations::Imported,
    program: &Program,
    world: &World,
) -> Result<BTreeMap<ObjectId, RetainedStack>> {
    let registry = world
        .scheduler_storage
        .as_ref()
        .context("retained lifetime registry absent")?;
    registry
        .require_complete_backing(world)
        .map_err(anyhow::Error::msg)?;
    let mut stacks = BTreeMap::new();
    for (&id, object) in &invocations.objects {
        let Some(stack) = &object.stack else {
            continue;
        };
        let mut ids = BTreeSet::new();
        let mut frames = Vec::new();
        for frame in stack.frames() {
            ids.insert(frame.callee);
            frames.push(RetainedFrame {
                context_object: frame.callee,
                frame: frame.clone(),
            });
        }
        let mut bindings = Vec::new();
        for id in ids {
            let mut binding = invocations
                .objects
                .get(&id)
                .context("retained lifetime context absent")?
                .binding
                .clone();
            ensure!(
                world.objects.get(&id).and_then(|o| o.definition.as_ref())
                    == Some(&binding.definition),
                "retained lifetime context/World definition differs"
            );
            binding.registry_lifetime = Some(
                registry
                    .handle(id)
                    .context("retained lifetime handle absent")?,
            );
            bindings.push(binding);
        }
        let retained = RetainedStack::from_bytes(program, &object.raw_arena, frames, bindings)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            retained.frames() == stack.frames() && retained.raw_bytes() == object.raw_arena,
            "fresh lifetime binding changed captured stack bytes/state"
        );
        stacks.insert(id, retained);
    }
    Ok(stacks)
}
