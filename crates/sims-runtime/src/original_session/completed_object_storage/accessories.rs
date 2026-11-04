//! Same-cut loaded accessory identity. Receipts identify observed resident
//! memory, not a guessed installed asset file/version. Complete asset-host and
//! immutable-file qualification remain separate from this World component.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use sims_fixtures::readset::Readset;
use sims_vm::ObjectId;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Record {
    pub pointer: u32,
    pub name: String,
    pub raw: Vec<u8>,
}
pub struct Imported {
    pub(super) records: BTreeMap<ObjectId, Vec<Record>>,
    pub readset_sha256: String,
    /// Native resident-source receipts, explicitly not immutable file hashes.
    pub sources: BTreeMap<String, BTreeMap<String, String>>,
}
fn n(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("accessory native word absent")?
        .try_into()?)
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::bytes(v)
}
fn word(b: &[u8], o: usize) -> u32 {
    super::word(b, o)
}
fn rows<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    super::array(super::field(v, key)?)
}
fn vector(
    memory: &Readset<'_>,
    raw: &[u8],
    offset: usize,
    v: &Value,
    stride: usize,
    max: usize,
) -> Result<Vec<u8>> {
    ensure!(raw.len() >= offset + 12, "accessory vector header absent");
    let (a, b, c) = (
        word(raw, offset),
        word(raw, offset + 4),
        word(raw, offset + 8),
    );
    ensure!(
        a <= b
            && b <= c
            && (a != 0 || c == 0)
            && (b - a) as usize % stride == 0
            && (c - a) as usize % stride == 0
            && (c - a) as usize / stride <= max
            && v["begin"] == a
            && v["end"] == b
            && v["capacity"] == c
            && v["stride"] == stride,
        "accessory vector owner/extent differs"
    );
    let body = memory.read(a, (b - a) as usize)?;
    ensure!(
        body == bytes(&v["used_raw"])?,
        "accessory vector/readset differs"
    );
    Ok(body)
}
pub fn import(state: &Value, memory: &Readset<'_>) -> Result<Imported> {
    ensure!(
        state["readset"]["sha256"] == memory.source_sha256(),
        "accessory source epoch differs"
    );
    let geometry = &state["domains"]["geometry"];
    let mut records = BTreeMap::new();
    let mut sources = BTreeMap::new();
    let mut owners = BTreeSet::new();
    for actor in rows(&state["domains"]["objects"], "objects")? {
        let p = &actor["person"];
        if p.is_null() {
            continue;
        }
        let id: ObjectId = actor["object"]
            .as_i64()
            .context("accessory actor absent")?
            .try_into()?;
        let pointer = n(&actor["pointer"])?;
        let animator = n(&p["animator_pointer"])?;
        ensure!(
            memory.read(
                pointer
                    .checked_add(0xadc)
                    .context("accessory person overflow")?,
                4
            )? == animator.to_le_bytes(),
            "accessory animator link differs"
        );
        let ar = memory.read(animator, 0x2a0)?;
        ensure!(
            word(&ar, 0) == 0x671ee4 && word(&ar, 0x138) == pointer,
            "accessory animator/person owner differs"
        );
        let body = vector(memory, &ar, 0x274, &p["accessory_vector"], 4, 256)?;
        let observed = rows(p, "accessory_records")?;
        ensure!(
            body.len() == 4 * observed.len(),
            "accessory record coverage differs"
        );
        let character = rows(geometry, "characters")?
            .iter()
            .find(|r| r["object"] == id)
            .context("accessory character geometry absent")?;
        ensure!(
            character["model"] == word(&ar, 8),
            "accessory character model differs"
        );
        let model = memory.read(word(&ar, 8), 0xb0)?;
        let skin = word(&model, 4);
        let sh = memory.read(skin, 0xd4)?;
        ensure!(
            model == bytes(&character["model_raw"])?
                && sh == bytes(&character["skin_raw"])?
                && character["skin"] == skin
                && word(&model, 0x14) == animator
                && word(&sh, 0x9c) == word(&ar, 8),
            "accessory live model/skin aliases differ"
        );
        let live = vector(memory, &sh, 0x88, &character["appearances"], 4, 1024)?;
        let mut retained = Vec::new();
        let mut names = BTreeSet::new();
        for (b, r) in body.chunks_exact(4).zip(observed) {
            let rp = word(b, 0);
            ensure!(
                rp == n(&r["pointer"])? && owners.insert(rp),
                "accessory record pointer alias differs"
            );
            let raw = memory.read(rp, 28)?;
            ensure!(raw == bytes(&r["raw"])?, "accessory record/readset differs");
            let name = vector(memory, &raw, 4, &r["name"], 1, 4096)?;
            let texture = vector(memory, &raw, 16, &r["texture"], 1, 4096)?;
            ensure!(
                !name.is_empty()
                    && name.is_ascii()
                    && !name.contains(&0)
                    && texture.is_ascii()
                    && !texture.contains(&0),
                "unsupported accessory narrow name/texture"
            );
            let name = String::from_utf8(name)?;
            ensure!(
                name == name.to_ascii_lowercase() && names.insert(name.clone()),
                "accessory name normalization/duplicates need another profile"
            );
            let appearance = word(&raw, 0);
            ensure!(
                appearance != 0
                    && r["bound_owner"] == appearance
                    && live.chunks_exact(4).any(|b| word(b, 0) == appearance),
                "accessory has no loaded appearance owner"
            );
            let a = rows(geometry, "appearances")?
                .iter()
                .find(|a| a["pointer"] == appearance)
                .context("accessory resident appearance absent")?;
            let header = memory.read(appearance, 28)?;
            let resource = n(&a["resource"])?;
            ensure!(
                header == bytes(&a["raw"])?
                    && character["skin"] == word(&header, 0)
                    && word(&header, 4) == resource
                    && a["skin"] == word(&header, 0),
                "accessory resident model/skin/resource alias differs"
            );
            let bindings = vector(memory, &header, 8, &a["bindings"], 12, 1024)?;
            let rr = memory.read(resource, 40)?;
            ensure!(
                rr == bytes(&a["resource_raw"])?,
                "accessory resident resource/readset differs"
            );
            let (begin, end, capacity) = (word(&rr, 4), word(&rr, 8), word(&rr, 12));
            ensure!(
                begin <= end && end <= capacity && capacity - begin <= 4096,
                "accessory loaded resource name extent differs"
            );
            // The record's observed name is the VM's removal/lookup identity.
            // The resource filename payload was not observed here and is not
            // claimed equal, filled from disk, or needed for that identity.
            let mut witness = header;
            witness.extend(rr);
            witness.extend(bindings);
            let receipt = BTreeMap::from([(
                format!(
                    "native-resident-readset:{}#appearance:{appearance:08x}",
                    memory.source_sha256()
                ),
                sims_fixtures::digest(&witness),
            )]);
            if let Some(old) = sources.insert(name.clone(), receipt.clone()) {
                ensure!(
                    old == receipt,
                    "accessory label has conflicting resident source owners"
                );
            }
            retained.push(Record {
                pointer: rp,
                name,
                raw,
            });
        }
        ensure!(
            records.insert(id, retained).is_none(),
            "duplicate accessory actor"
        );
    }
    Ok(Imported {
        records,
        sources,
        readset_sha256: memory.source_sha256().into(),
    })
}
