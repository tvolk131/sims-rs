//! Borrowed current owners at the actual 88448 or 88449 native cut.
//! No VM scheduling, construction, resource loading or state mutation occurs.
use super::{completed_animations, completed_lot::Memory, completed_simulation};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::{ObjectId, World, body_profiles, motives::score_owner::Owner};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub offset: [u32; 3],
    pub height: i32,
}
pub struct Imported {
    pub scores: BTreeMap<ObjectId, Owner>,
    pub geometry: BTreeMap<ObjectId, Vec<Geometry>>,
    pub head_names: BTreeMap<ObjectId, Vec<u8>>,
    pub body_profiles: body_profiles::State,
    pub opaque_slot_words: usize,
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    completed_animations::bytes(v.as_str().context("supplemental bytes absent")?)
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("supplemental number absent")?
        .try_into()?)
}
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn cstring(m: &Memory, p: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for i in 0..=4096u32 {
        let b = m.read(p.checked_add(i).context("string pointer overflow")?, 1)?[0];
        if b == 0 {
            return Ok(out);
        }
        out.push(b);
    }
    anyhow::bail!("supplemental string extent exceeds bound")
}
fn profiles(m: &Memory, input: &Value) -> Result<body_profiles::State> {
    // This exact captured profile has no resident STR# publication overlays.
    // Its three lazy bank records must remain null, not be loaded from assets.
    ensure!(
        input["banks"]
            .as_array()
            .context("body banks absent")?
            .is_empty(),
        "resident body bank needs its own publication owner"
    );
    let mut definitions = serde_json::Map::new();
    let mut cache_keys = BTreeSet::new();
    for row in input["retained"]
        .as_array()
        .context("body cache inventory absent")?
    {
        let resource = number(&row["resource"])?;
        let definition = number(&row["definition"])?;
        let cache = number(&row["cache"])?;
        let r = m.read(resource, 0xb8)?;
        let d = m.read(definition, 0xd8)?;
        ensure!(
            word(&r, 0x60) == definition
                && word(&r, 0x88) == cache
                && i16::from_le_bytes(d[0x12..0x14].try_into()?) == 2,
            "BodyStrings concrete cache alias/class differs"
        );
        if cache == 0 {
            continue;
        }
        let h = m.read(cache, 0x6c)?;
        let provider = word(&h, 4);
        let source = format!("native-provider:{provider:08x}");
        let chunk = word(&r, 0xb4) >> 16;
        let bank = u16::from_le_bytes(h[0x54..0x56].try_into()?);
        ensure!(
            row["provider"] == provider
                && row["source"] == source
                && row["chunk"] == chunk
                && row["bank"] == bank
                && h[0x5c..0x5e] == [0, 0]
                && d[0x26..0x28] == bank.to_le_bytes(),
            "BodyStrings bank/provider differs"
        );
        let (a, b, c) = (word(&h, 0x60), word(&h, 0x64), word(&h, 0x68));
        ensure!(
            a >= 0x10000
                && a <= b
                && b <= c
                && (b - a) % 4 == 0
                && (c - a) % 4 == 0
                && (b - a) / 4 >= 35
                && (c - a) / 4 <= 4096,
            "BodyStrings current vector extent differs"
        );
        let pointers = m.read(a, (b - a) as usize)?;
        let mut values = Vec::new();
        for p in pointers.chunks_exact(4) {
            let raw = cstring(m, word(p, 0))?;
            ensure!(raw.is_ascii(), "captured BodyStrings encoding differs");
            values.push(String::from_utf8(raw)?);
        }
        ensure!(
            serde_json::to_value(&values)? == row["values"]
                && cache_keys.insert((source.clone(), bank)),
            "BodyStrings decoded/cache coverage differs"
        );
        let entry = json!({"bank":bank,"values":values});
        let map = definitions.entry(source).or_insert_with(|| json!({}));
        ensure!(
            map.get(chunk.to_string()).is_none(),
            "duplicate body definition"
        );
        map[chunk.to_string()] = entry;
    }
    ensure!(
        Value::Object(definitions.clone()) == input["definitions"],
        "BodyStrings declared definitions differ from native owned values"
    );
    let mut lazy = BTreeSet::new();
    for row in input["demands"]
        .as_array()
        .context("body lazy bank inventory absent")?
    {
        let record = &row["record"];
        let raw = bytes(&record["raw"])?;
        let p = number(&record["pointer"])?;
        ensure!(
            m.read(p, raw.len())? == raw
                && raw.len() >= 20
                && row["status"] == "observed_null_lazy_owner_not_reconstructed"
                && record["handle"] == 0,
            "BodyStrings missing lazy-owner proof"
        );
        // The resource-map payload starts at the intentionally unaligned +7;
        // its four shorts precede the resident handle at native node+0f.
        ensure!(word(&raw, 15) == 0, "BodyStrings lazy bank became resident");
        let source = row["source"]
            .as_str()
            .context("lazy provider absent")?
            .to_owned();
        let bank: u16 = number(&row["bank"])?.try_into()?;
        ensure!(
            u16::from_le_bytes(raw[4..6].try_into()?) == bank && raw[6] == 0,
            "lazy BodyStrings record key differs"
        );
        ensure!(
            lazy.insert((source, bank)),
            "duplicate lazy BodyStrings bank"
        );
    }
    ensure!(lazy == cache_keys, "BodyStrings lazy-bank coverage differs");
    Ok(serde_json::from_value(
        json!({"definitions":definitions,"banks":{}}),
    )?)
}
pub fn import(v: &Value, world: &World) -> Result<Imported> {
    ensure!(
        v["schema"] == 1
            && matches!(v["tick"].as_u64(), Some(88448 | 88449))
            && v["executes_target_code"] == false
            && v["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && v["provenance"]["replay_report_sha256"]
                == "d241a655ea5bde58100927aaea95994311097e93847544c675c5ecb92e34471e",
        "supplemental native cut/epoch differs"
    );
    let m = Memory::with_union_limit(v, 4 << 20)?;
    for (a, n, h) in [
        (
            0x45d145,
            32,
            "8a84f630424348fdd0fd0314d08b3d74345306549820ef7304b9e6fdbaadf210",
        ),
        (
            0x42138d,
            169,
            "9eb479943de660173e2032c587c1bc3fad9cb09ddcca9e1a1a36367beb7cadc8",
        ),
        (
            0x42144e,
            83,
            "4a709f28d9dc90199f71149aec1e32ce7384dd2908fe4c0e38590601c08e7aff",
        ),
        (
            0x4214a1,
            226,
            "afc008b0a1ec71714dd2ffc0df3838b680a25a28dcbe30ac7648dec028e19239",
        ),
        (
            0x45de01,
            11,
            "70cb6efcd9a5a469a2bf9717c024ab495b5d15f83a3418b760762ccf5da9bbaf",
        ),
        (
            0x45e125,
            25,
            "ddcbbe804cdf46c6c76b4317d5f12063d600cce62cc4ec96fbb0808fe50769ff",
        ),
        (
            0x46de93,
            40,
            "05d6dfba4a1613d3e1d2b1e1b5bc39fab26ec1cbe524094e6e9db821816c504f",
        ),
        (
            0x46debb,
            61,
            "689dd96363f17cbbf471431ad2dd368fc2c1a4ab60426364eceec4f0a0ac4b42",
        ),
        (
            0x45962b,
            180,
            "60d31e4e59de95df1b14370a2c017cc0674db7c0eda63db73c96b339ef859933",
        ),
        (
            0x58f8ff,
            403,
            "74c9d0c95ab4ab040d630b91030f41fdd9e71ee6244d44d392ab8415cfbcea52",
        ),
        (
            0x457f60,
            428,
            "d541a7a6d851c8be36edd9c16570abaef6a3a9d5c037718f099df4a6d98ce050",
        ),
        (
            0x587b4b,
            114,
            "fae2568acb1e20437a1b6f0f3d20854c23163d42000f2ed0a060c92d80143a16",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == h,
            "supplemental source instructions differ"
        );
    }
    let rows = v["objects"]
        .as_array()
        .context("supplemental object cohort absent")?;
    ensure!(
        rows.len() == world.objects.len(),
        "supplemental cohort extent differs"
    );
    let mut pointers = BTreeMap::new();
    let mut geometry = BTreeMap::new();
    let mut head_names = BTreeMap::new();
    let mut opaque_slot_words = 0;
    for row in rows {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("supplemental id absent")?
            .try_into()?;
        let o = world
            .objects
            .get(&id)
            .context("supplemental object absent")?;
        let p = number(&row["pointer"])?;
        let raw = bytes(&row["raw"])?;
        ensure!(
            raw.len() == 0x188
                && m.read(p, raw.len())? == raw
                && i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id
                && pointers.insert(id, p).is_none()
                && (word(&raw, 0) == 0x667378) == o.person.is_some(),
            "supplemental object alias differs"
        );
        let slots = bytes(&row["slots_raw"])?;
        let (a, b, c) = (word(&raw, 0x130), word(&raw, 0x134), word(&raw, 0x138));
        ensure!(
            a >= 0x10000
                && a <= b
                && b <= c
                && b - a == slots.len() as u32
                && slots.len() >= 36
                && slots.len() % 36 == 0
                && (c - a) % 36 == 0
                && m.read(a, slots.len())? == slots
                && slots.len() / 36 == o.slots.len() + 1,
            "supplemental slot ownership/extent differs"
        );
        let mut values = Vec::new();
        for (i, r) in slots.chunks_exact(36).skip(1).enumerate() {
            let occupant = i16::from_le_bytes(r[16..18].try_into()?);
            ensure!(
                occupant >= 0 && o.slots[i] == (occupant != 0).then_some(occupant),
                "slot occupant differs"
            );
            let named = o.person.is_some() && i == 1;
            ensure!(
                word(r, 28) == if named { 2 } else { 0 },
                "unsupported current slot flags"
            );
            if named {
                let name = cstring(&m, word(r, 32))?;
                ensure!(
                    row["head_name"] == "HEAD" && name == b"HEAD",
                    "current HEAD alias differs"
                );
                head_names.insert(id, name);
            } else {
                opaque_slot_words += 1;
            }
            values.push(Geometry {
                offset: std::array::from_fn(|j| word(r, j * 4)),
                height: word(r, 20) as i32,
            });
        }
        geometry.insert(id, values);
    }
    ensure!(
        pointers.keys().eq(world.objects.keys()),
        "supplemental cohort membership differs"
    );
    let mut scores = BTreeMap::new();
    for row in v["people"]
        .as_array()
        .context("supplemental scores absent")?
    {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("score id absent")?
            .try_into()?;
        let p = number(&row["pointer"])?;
        let root = number(&row["root"])?;
        let raw = bytes(&row["raw"])?;
        let age = m.read(p + 0x600, 2)?;
        ensure!(
            pointers.get(&id) == Some(&p)
                && m.read(p + 0xaec, 4)? == bytes(&row["binding_raw"])?
                && word(&m.read(p + 0xaec, 4)?, 0) == root
                && raw.len() == 0xc4
                && m.read(root, raw.len())? == raw
                && word(&raw, 0xbc) == p
                && word(&raw, 0xc0) == p + 0x78c
                && row["data"]["pointer"] == word(&raw, 0)
                && row["data"]["count"] == word(&raw, 4)
                && age == bytes(&row["age_raw"])?
                && m.read(p + 0xf0, 2)? == id.to_le_bytes()
                && world.objects[&id].person.as_ref().and_then(|p| p.get(&58))
                    == Some(&i16::from_le_bytes(age.as_slice().try_into()?)),
            "supplemental personal score binding differs"
        );
        let owner = Owner::capture(&completed_simulation::curves(&m, &row["data"])?)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            scores.insert(id, owner).is_none(),
            "duplicate personal score owner"
        );
    }
    ensure!(
        scores.keys().eq(head_names.keys()),
        "supplemental person coverage differs"
    );
    Ok(Imported {
        scores,
        geometry,
        head_names,
        body_profiles: profiles(&m, &v["body_profiles"])?,
        opaque_slot_words,
    })
}
