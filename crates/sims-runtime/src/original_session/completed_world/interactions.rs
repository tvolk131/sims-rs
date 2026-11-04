//! Existing native interaction caches, not a TTAB reload or fabricated menu.
//! Physical authenticity belongs to the externally pinned source artifact.
use super::callbacks::overlap;
use crate::original_session::{completed_animations, completed_invocations};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_fixtures::readset::Readset;
use sims_vm::{
    ObjectId, Program,
    program::{AutonomyAdvertisement, MenuEntry},
};
use std::collections::{BTreeMap, BTreeSet};
fn n(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("native interaction word absent")?
        .try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("native interaction bytes absent")?)
}
fn rows<'a>(v: &'a Value, k: &str) -> Result<&'a [Value]> {
    v[k].as_array()
        .map(Vec::as_slice)
        .context("native interaction rows absent")
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn short(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn view(e: &MenuEntry) -> Value {
    json!({"id":e.id,"flags":e.flags,"label":e.label,"action":e.action,"check":e.check,
    "autonomy":{"threshold":e.autonomy.threshold,"attenuation_code":e.autonomy.attenuation_code,
    "attenuation_bits":e.autonomy.attenuation.to_bits(),"joining_index":e.autonomy.joining_index,"motives":e.autonomy.motives,"flags2":e.autonomy.flags2}})
}
pub fn import(
    state: &Value,
    source: &Value,
    memory: &Readset<'_>,
    invocations: &completed_invocations::Imported,
    mut program: Program,
) -> Result<Program> {
    ensure!(
        source["schema"] == 1
            && source["tick"] == state["tick"]
            && source["source_readset_sha256"] == memory.source_sha256()
            && invocations.readset_sha256 == memory.source_sha256()
            && source["physical_compressed_sha256"]
                == state["physical_baseline"]["archive"]["compressed_sha256"],
        "native interaction source epoch differs"
    );
    let mut observed = Vec::new();
    for r in rows(source, "reads")? {
        let p = n(&r["address"])?;
        let raw = bytes(&r["raw"])?;
        ensure!(
            raw.len() <= 1 << 20
                && r["bytes"] == raw.len()
                && r["sha256"] == sims_fixtures::digest(&raw),
            "native interaction source read differs"
        );
        overlap(memory, p, &raw)?;
        observed.push((p, raw));
    }
    let read = |p: u32, n: usize| -> Result<Vec<u8>> {
        if n == 0 {
            return Ok(Vec::new());
        }
        let r = observed
            .iter()
            .find(|(a, b)| {
                u64::from(*a) <= u64::from(p)
                    && u64::from(p) + n as u64 <= u64::from(*a) + b.len() as u64
            })
            .context("native interaction physical bytes unobserved")?;
        Ok(r.1[(p - r.0) as usize..(p - r.0) as usize + n].to_vec())
    };
    let mut tables = BTreeMap::new();
    ensure!(
        rows(source, "tables")?.len() <= 32767,
        "native interaction table count exceeds bound"
    );
    for t in rows(source, "tables")? {
        let p = n(&t["pointer"])?;
        let h = read(p, 0x1c)?;
        ensure!(
            h == bytes(&t["raw"])? && word(&h, 4) <= 4096 && word(&h, 8) <= 16,
            "native interaction table header differs"
        );
        let body = read(word(&h, 0x10), word(&h, 4) as usize * 4)?;
        ensure!(
            body == bytes(&t["pointer_raw"])? && rows(t, "entries")?.len() * 4 == body.len(),
            "native interaction entry coverage differs"
        );
        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        for (ptr, r) in body.chunks_exact(4).zip(rows(t, "entries")?) {
            let ep = word(ptr, 0);
            let raw = read(ep, 0x30)?;
            ensure!(
                r["pointer"] == ep
                    && raw == bytes(&r["raw"])?
                    && seen.insert(ep)
                    && word(&raw, 4) <= 16,
                "native interaction entry binding differs"
            );
            let ad = read(word(&raw, 8), word(&raw, 4) as usize * 6)?;
            ensure!(
                ad == bytes(&r["advertisements_raw"])?,
                "native interaction advertisement bytes differ"
            );
            let (a, b, c) = (word(&raw, 0x24), word(&raw, 0x28), word(&raw, 0x2c));
            ensure!(
                a <= b && b <= c && c - a <= 65536,
                "native interaction label extent differs"
            );
            let label = read(a, (b - a) as usize)?;
            ensure!(
                label == bytes(&r["label_raw"])? && !label.contains(&0),
                "native interaction label bytes differ"
            );
            let e = MenuEntry {
                id: word(&raw, 0x18),
                flags: word(&raw, 0x14),
                label: sims_formats::strings::decode(&label, 1),
                action: short(&raw, 2),
                check: short(&raw, 0),
                autonomy: AutonomyAdvertisement {
                    threshold: word(&raw, 0x1c),
                    attenuation_code: word(&raw, 0xc),
                    attenuation: f32::from_bits(word(&raw, 0x10)),
                    joining_index: word(&raw, 0x20) as i32,
                    motives: ad
                        .chunks_exact(6)
                        .map(|b| sims_vm::motives::Advertisement {
                            personality: short(b, 0) as i16,
                            minimum: short(b, 2) as i16,
                            delta: short(b, 4) as i16,
                        })
                        .collect(),
                    flags2: 0,
                },
            };
            ensure!(
                view(&e) == r["entry"],
                "native interaction raw/decoded fields differ"
            );
            entries.push(e);
        }
        ensure!(
            tables.insert(p, entries).is_none(),
            "duplicate native interaction table"
        );
    }
    let mut objects = BTreeSet::new();
    let mut used = BTreeSet::new();
    for r in rows(source, "objects")? {
        let id: ObjectId = r["object"]
            .as_i64()
            .context("native interaction object absent")?
            .try_into()?;
        ensure!(objects.insert(id), "duplicate native interaction object");
        let object = invocations
            .objects
            .get(&id)
            .context("native interaction object outside current inventory")?;
        let p = n(&r["pointer"])?;
        let resource = n(&r["resource"])?;
        let t = n(&r["table"])?;
        let raw = memory.read(p, 0x188)?;
        ensure!(
            short(&raw, 0xf0) as i16 == id
                && word(&raw, 0x11c) == resource
                && word(&memory.read(resource + 0x64, 4)?, 0) == t,
            "native interaction resource/cache alias differs"
        );
        if t == 0 {
            continue;
        } // Unloaded cache is not an empty native table.
        used.insert(t);
        let entries = tables
            .get(&t)
            .context("native cached interaction table missing")?;
        let def = &object.binding.definition;
        if let Some(old) = program
            .modules
            .get_mut(&def.source)
            .context("native interaction module absent")?
            .object_interactions
            .insert(def.object, entries.clone())
        {
            ensure!(
                old.iter().map(view).eq(entries.iter().map(view)),
                "shared native definition has conflicting interaction caches"
            );
        }
    }
    ensure!(
        objects.iter().eq(invocations.objects.keys()) && used.len() == tables.len(),
        "native interaction object/table coverage differs"
    );
    Ok(program)
}
