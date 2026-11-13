//! Compare retained full lot inputs with the Rust geometry calculation.
//! No loading, VM scheduling, callbacks, or native state publication executes.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub mod environment;
pub mod physical;
#[cfg(test)]
mod tests;
use serde_json::{Value, json};
use sims_formats::lot::WallTile;
use sims_sim::{
    lot_geometry::{FloorSource, Lot},
    lot_rooms::{Partition, Split},
};
use std::collections::BTreeMap;
pub(super) struct Memory(BTreeMap<u32, u8>);
impl Memory {
    pub(super) fn new(input: &Value) -> Result<Self> {
        Self::with_union_limit(input, 1 << 20)
    }
    /// Some catalog readsets retain complete resident STR# payloads. Their
    /// explicit larger diagnostic budget does not relax existing callers.
    pub(super) fn with_union_limit(input: &Value, limit: usize) -> Result<Self> {
        ensure!(
            (1..=4 << 20).contains(&limit),
            "readset union budget exceeds bound"
        );
        let mut m = Self(BTreeMap::new());
        for r in input["reads"].as_array().context("lot readset absent")? {
            let raw = bytes(&r["raw"])?;
            let p = u32::try_from(r["address"].as_u64().context("lot address absent")?)?;
            ensure!(
                r["bytes"] == raw.len()
                    && r["sha256"] == sims_fixtures::digest(&raw)
                    && raw.len() <= 1 << 20,
                "lot read receipt hash/extent differs"
            );
            for (i, b) in raw.into_iter().enumerate() {
                let address = p.checked_add(i.try_into()?).context("lot read overflow")?;
                ensure!(
                    m.0.insert(address, b).is_none_or(|old| old == b),
                    "conflicting same-cut lot bytes"
                );
            }
            ensure!(m.0.len() <= limit, "lot readset union exceeds bound");
        }
        Ok(m)
    }
    pub(super) fn read(&self, p: u32, n: usize) -> Result<Vec<u8>> {
        ensure!(n <= 1 << 20, "lot range exceeds bound");
        (0..n)
            .map(|i| {
                self.0
                    .get(
                        &p.checked_add(i.try_into()?)
                            .context("lot address overflow")?,
                    )
                    .copied()
                    .context("lot field outside source readset")
            })
            .collect()
    }
}
fn word(raw: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        raw.get(offset..offset + 4)
            .context("short native lot storage")?
            .try_into()?,
    ))
}
fn pointer(value: &Value) -> Result<u32> {
    Ok(value.as_u64().context("lot pointer absent")?.try_into()?)
}
const CODE: &[(u32, usize, &str)] = &[
    (
        0x46711d,
        219,
        "0217c40efebe07e4ada0d4705dd962e1140fd35f1af6912b438997e5e55fa01b",
    ),
    (
        0x4672e0,
        114,
        "72f62fa9f374808c37855808955c081290f907445d0985aeb6ae409b894c5e82",
    ),
    (
        0x467352,
        126,
        "4296e688a23ac75acd2b047fecd7cc6f9b724f2ccb6a1d9799a1ddb600dad9e6",
    ),
    (
        0x4681e0,
        35,
        "d6c5b5f9652f967fc6bf7fda7690c2b80327499cf730d6fd549477027ff9036b",
    ),
    (
        0x466ff6,
        158,
        "cb8c2648afe73ca4f22564777afc50a2314a6cbcbd77ffcb7f0fc3f5ff0017b8",
    ),
];
fn validate_source(input: &Value) -> Result<()> {
    ensure!(
        input["schema"] == 1
            && input["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && input["provenance"]["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "lot source epoch differs"
    );
    validate_layout(input)
}
fn validate_layout(input: &Value) -> Result<()> {
    let m = Memory::new(input)?;
    let code = input["code"].as_array().context("lot code absent")?;
    ensure!(code.len() == CODE.len(), "lot code coverage differs");
    let mut seen = std::collections::BTreeSet::new();
    for c in code {
        let p = pointer(&c["address"])?;
        let (_, n, hash) = CODE
            .iter()
            .find(|(a, _, _)| *a == p)
            .context("unknown lot code method")?;
        let raw = bytes(&c["raw"])?;
        ensure!(
            seen.insert(p)
                && raw.len() == *n
                && c["bytes"] == *n
                && c["sha256"] == *hash
                && sims_fixtures::digest(&raw) == *hash
                && m.read(p, *n)? == raw,
            "lot code witness differs"
        );
    }
    let world = pointer(&input["world"])?;
    let raw = bytes(&input["world_raw"])?;
    ensure!(
        raw.len() == 0x88 && m.read(world, raw.len())? == raw && word(&raw, 0x10)? == 52,
        "lot world header differs"
    );
    let owner = word(&m.read(0x6e48bc, 4)?, 0)?;
    let owner_raw = m.read(owner, 0x28)?;
    let app = m.read(word(&owner_raw, 0)?, 0x28)?;
    ensure!(
        word(&app, 0x18)? == world
            && word(&app, 0x24)? == pointer(&input["manager"])?
            && m.read(pointer(&input["manager"])?, 0x80)? == bytes(&input["manager_raw"])?,
        "lot application/manager aliases differ"
    );
    let names = [
        ("heights", 0x14, 4, 12),
        ("pool", 0x60, 1, 16),
        ("water", 0x64, 1, 16),
        ("coverings1", 0x24, 2, 12),
        ("coverings2", 0x28, 2, 12),
        ("primary1", 0x2c, 2, 12),
        ("primary2", 0x30, 2, 12),
        ("flags1", 0x34, 1, 12),
        ("flags2", 0x38, 1, 12),
        ("heads1", 0x3c, 2, 12),
        ("heads2", 0x40, 2, 12),
        ("walls1", 0x44, 14, 12),
        ("walls2", 0x48, 14, 12),
    ];
    ensure!(
        input["arrays"]
            .as_object()
            .context("lot arrays absent")?
            .len()
            == names.len(),
        "lot array coverage differs"
    );
    for (name, offset, stride, directory_offset) in names {
        let a = &input["arrays"][name];
        let p = word(&raw, offset)?;
        let header = bytes(&a["raw"])?;
        ensure!(
            a["pointer"] == p
                && header.len() == 32
                && m.read(p, 32)? == header
                && word(&header, directory_offset - 8)? == 64
                && word(&header, directory_offset - 4)? == 64,
            "lot array header differs: {name}"
        );
        ensure!(
            directory_offset != 12 || word(&header, 28)? == stride as u32,
            "lot element stride differs"
        );
        let directory = word(&header, directory_offset)?;
        let columns = bytes(&a["columns_raw"])?;
        ensure!(
            a["directory"] == directory
                && columns.len() == 256
                && m.read(directory, 256)? == columns,
            "lot directory differs"
        );
        let data = array(input, name, stride)?;
        let pointers: Vec<u32> = columns
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(
            serde_json::to_value(&pointers)? == a["columns"],
            "lot column pointers differ"
        );
        for (i, p) in pointers.into_iter().enumerate() {
            ensure!(
                m.read(p, 64 * stride)? == data[i * 64 * stride..(i + 1) * 64 * stride],
                "lot column data differs: {name}/{i}"
            );
        }
    }
    let manager = bytes(&input["manager_raw"])?;
    let rooms = input["rooms"].as_array().context("rooms absent")?;
    ensure!(
        word(&manager, 8)? as usize == rooms.len(),
        "native room count differs"
    );
    let mut ids = std::collections::BTreeSet::new();
    for room in rooms {
        let raw = bytes(&room["raw"])?;
        let p = pointer(&room["pointer"])?;
        ensure!(
            raw.len() == 0x2a0 && m.read(p, raw.len())? == raw,
            "native room bytes differ"
        );
        let id = i16::from_le_bytes(raw[0..2].try_into()?);
        ensure!(
            room["id"] == id && ids.insert(id),
            "native room identity differs"
        );
        for (name, offset, stride) in [("tiles", 4, 3), ("rectangles", 0x28, 20)] {
            let (begin, end, capacity) = (
                word(&raw, offset)?,
                word(&raw, offset + 4)?,
                word(&raw, offset + 8)?,
            );
            let v = &room[name];
            let data = bytes(&room[format!("{name}_raw")])?;
            ensure!(
                begin <= end
                    && end <= capacity
                    && end - begin == data.len() as u32
                    && (end - begin) % stride == 0
                    && v["begin"] == begin
                    && v["end"] == end
                    && v["capacity"] == capacity
                    && bytes(&v["raw"])? == raw[offset..offset + 12],
                "native room vector differs"
            );
            ensure!(
                data.is_empty() || m.read(begin, data.len())? == data,
                "native room vector data differs"
            );
        }
    }
    let head = word(&manager, 0x10)?;
    let header = bytes(&input["half_header_raw"])?;
    ensure!(
        input["half_head"] == head && header.len() == 36 && m.read(head, 36)? == header,
        "native half-map sentinel differs"
    );
    let nodes = input["half_nodes"]
        .as_array()
        .context("half nodes absent")?;
    ensure!(
        nodes.len() == word(&manager, 0x14)? as usize,
        "native half-map count differs"
    );
    let mut tree = BTreeMap::new();
    for node in nodes {
        let raw = bytes(&node["raw"])?;
        let p = pointer(&node["pointer"])?;
        ensure!(
            raw.len() == 36
                && m.read(p, 36)? == raw
                && node["position"] == json!(&raw[0x10..0x13])
                && node["rooms"]
                    == json!([
                        i16::from_le_bytes(raw[0x14..0x16].try_into()?),
                        i16::from_le_bytes(raw[0x1c..0x1e].try_into()?)
                    ])
                && node["kinds"] == json!([word(&raw, 0x18)?, word(&raw, 0x20)?]),
            "native half-map node differs"
        );
        ensure!(
            tree.insert(p, [word(&raw, 8)?, word(&raw, 12)?]).is_none(),
            "duplicate native half-map node"
        );
    }
    let mut pending = vec![word(&header, 4)?];
    let mut seen = BTreeSet::new();
    while let Some(p) = pending.pop() {
        if p == 0 || p == head {
            continue;
        }
        ensure!(seen.insert(p), "native half-map cycle");
        pending.extend(tree.get(&p).context("native half-map child omitted")?);
    }
    ensure!(
        seen.len() == tree.len(),
        "disconnected native half-map node"
    );
    Ok(())
}
fn bytes(value: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(value.as_str().context("bytes absent")?)
}
fn array(input: &Value, name: &str, stride: usize) -> Result<Vec<u8>> {
    let a = &input["arrays"][name];
    let raw = bytes(&a["data_raw"])?;
    ensure!(
        a["width"] == 64
            && a["height"] == 64
            && a["stride"] == stride
            && raw.len() == 4096 * stride,
        "native array extent differs: {name}"
    );
    Ok(raw)
}
fn shorts(raw: &[u8]) -> Vec<i16> {
    raw.chunks_exact(2)
        .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn first_diff<T: PartialEq + serde::Serialize>(a: &[T], b: &[T]) -> Value {
    let first = a.iter().zip(b).position(|(a, b)| a != b);
    json!({"left_len":a.len(),"right_len":b.len(),"count":a.iter().zip(b).filter(|(a,b)|a!=b).count(),
        "first":first.map(|i|json!({"index":i,"tile":[i/64,i%64],"rust":a[i],"native":b[i]}))})
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub lot: Lot,
    pub source: Value,
    pub comparison: Value,
    pub complete_native_lot_admitted: bool,
}

/// The cached native OBJf tables imported by completed_reentry cover all live
/// definitions. Require that positive coverage before the derived footprint
/// adapter can interpret a missing portal action as false.
pub fn architecture(
    imported: &Imported,
    program: &sims_vm::Program,
    world: &sims_vm::World,
) -> Result<sims_sim::architecture::Architecture> {
    for module in program.modules.values() {
        for definition in module.object_definition_words.keys() {
            ensure!(
                module
                    .object_functions
                    .get(definition)
                    .is_some_and(|f| f.len() > 15),
                "architecture includes a definition without a native callback directory"
            );
        }
    }
    for object in world.objects.values() {
        let definition = object
            .definition
            .as_ref()
            .context("architecture live definition absent")?;
        let module = program
            .modules
            .get(&definition.source)
            .context("architecture live module absent")?;
        ensure!(
            module
                .object_definition_words
                .contains_key(&definition.object),
            "architecture OBJD absent"
        );
        ensure!(
            module
                .object_functions
                .get(&definition.object)
                .is_some_and(|f| f.len() > 15),
            "architecture native callback directory incomplete"
        );
    }
    let architecture =
        sims_sim::architecture::Architecture::from_original(imported.lot.clone(), program)
            .map_err(anyhow::Error::msg)?;
    ensure!(
        architecture.outdoors == world.room_outdoors
            && world.objects.values().all(|o| o
                .definition
                .as_ref()
                .is_some_and(|d| architecture.footprints.contains_key(&d.guid))),
        "architecture/World coverage differs"
    );
    Ok(architecture)
}

/// A pure Rust geometry calculation is checked against every retained native
/// output before it can become the imported cache. No native constructor,
/// callback, clock, VM scheduler or RNG executes.
pub fn import(input: &Value) -> Result<Imported> {
    validate_source(input)?;
    import_fields(input)
}

/// Same native layout/geometry validation against a supplied current readset.
/// The legacy proof wrapper above retains its exact historical source pins.
pub fn import_captured(
    input: &Value,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(input["schema"] == 1, "current lot schema differs");
    for r in input["reads"]
        .as_array()
        .context("current lot reads absent")?
    {
        let raw = bytes(&r["raw"])?;
        ensure!(
            raw.len() <= 1 << 20 && memory.read(pointer(&r["address"])?, raw.len())? == raw,
            "current lot source/readset differs"
        );
    }
    validate_layout(input)?;
    import_fields(input)
}
fn import_fields(input: &Value) -> Result<Imported> {
    ensure!(
        input["active_size"] == 52 && input["executes_target_code"] == false,
        "native lot source profile differs"
    );
    for receipt in input["reads"].as_array().context("reads absent")? {
        let raw = bytes(&receipt["raw"])?;
        ensure!(
            receipt["bytes"] == raw.len() && receipt["sha256"] == sims_fixtures::digest(&raw),
            "native receipt differs"
        );
    }
    let heights = array(&input, "heights", 4)?
        .chunks_exact(4)
        .map(|b| b.try_into().unwrap())
        .collect();
    let pool = array(&input, "pool", 1)?;
    let water = array(&input, "water", 1)?;
    let mut sources = vec![];
    let mut native = vec![];
    for level in 1..=2 {
        let walls = array(&input, &format!("walls{level}"), 14)?
            .chunks_exact(14)
            .map(|b| WallTile {
                segments: b[0],
                flags: b[1],
                styles: std::array::from_fn(|i| {
                    u16::from_le_bytes(b[2 + i * 2..4 + i * 2].try_into().unwrap())
                }),
                patterns: std::array::from_fn(|i| {
                    u16::from_le_bytes(b[6 + i * 2..8 + i * 2].try_into().unwrap())
                }),
            })
            .collect();
        sources.push(FloorSource {
            coverings: shorts(&array(&input, &format!("coverings{level}"), 2)?)
                .into_iter()
                .map(|v| v as u16)
                .collect(),
            walls,
            object_heads: shorts(&array(&input, &format!("heads{level}"), 2)?)
                .into_iter()
                .map(|v| v as u16)
                .collect(),
            flags: array(&input, &format!("flags{level}"), 1)?,
        });
        let mut room_tiles = BTreeMap::new();
        let mut metrics = BTreeMap::new();
        for r in input["rooms"].as_array().context("rooms absent")? {
            let id = r["id"].as_i64().context("room ID absent")? as i16;
            if ((id as u16 >> 10) & 3) + 1 != level {
                continue;
            }
            let raw = bytes(&r["raw"])?;
            ensure!(
                raw[0x71] == 1 && raw[0x72] <= 1,
                "native outdoors cache unavailable"
            );
            metrics.insert(id,json!({"outdoors":raw[0x72]!=0,"half_tiles":i32::from_le_bytes(raw[0x38..0x3c].try_into().unwrap())}));
            let tiles = bytes(&r["tiles_raw"])?;
            room_tiles.insert(
                id,
                tiles
                    .chunks_exact(3)
                    .map(|p| [p[0] as usize, p[1] as usize, p[2] as usize])
                    .collect(),
            );
        }
        let mut halves = vec![];
        for h in input["half_nodes"]
            .as_array()
            .context("half nodes absent")?
        {
            let pos: Vec<usize> = serde_json::from_value(h["position"].clone())?;
            if pos[2] != level as usize {
                continue;
            }
            let kinds: Vec<u8> = serde_json::from_value(h["kinds"].clone())?;
            let rooms: Vec<i16> = serde_json::from_value(h["rooms"].clone())?;
            halves.push(Split {
                tile: [pos[0], pos[1]],
                rooms: kinds.into_iter().zip(rooms).collect(),
            });
        }
        halves.sort_by_key(|h| h.tile);
        native.push((
            Partition {
                primary: shorts(&array(&input, &format!("primary{level}"), 2)?),
                halves,
                room_tiles,
            },
            metrics,
        ));
    }
    let lot = Lot::rebuild_in_active(64, 52, heights, pool, water, sources)
        .map_err(anyhow::Error::msg)?;
    let mut comparisons = vec![];
    for (i, f) in lot.floors.iter().enumerate() {
        let (n, metrics) = &native[i];
        let flags = array(&input, &format!("flags{}", i + 1), 1)?;
        let rust_tiles: Value = serde_json::to_value(&f.rooms.room_tiles)?;
        let native_tiles: Value = serde_json::to_value(&n.room_tiles)?;
        comparisons.push(json!({"level":i+1,"primary":first_diff(&f.rooms.primary,&n.primary),"halves_equal":f.rooms.halves==n.halves,
            "flags":first_diff(&f.flags.flags,&flags),"room_tiles_equal":rust_tiles==native_tiles,
            "rust_rooms":f.rooms.room_tiles.iter().map(|(id,v)|(*id,v.len())).collect::<Vec<_>>(),
            "native_rooms":n.room_tiles.iter().map(|(id,v)|(*id,v.len())).collect::<Vec<_>>(),
            "rust_metrics":f.metrics,"native_metrics":metrics,"metrics_equal":serde_json::to_value(&f.metrics)?==serde_json::to_value(metrics)?}));
    }
    let world = bytes(&input["world_raw"])?;
    let report = json!({"schema":1,"active":52,"backing":64,"comparisons":comparisons,
        "upper_summaries":{"native":[world[0x80],world[0x81]],"rust":[lot.floors[1].flags.upper_contents,lot.floors[1].flags.upper_support]},
        "native_parity_ticks":0,"complete_native_lot_admitted":false});
    for c in report["comparisons"]
        .as_array()
        .context("geometry comparisons absent")?
    {
        ensure!(
            c["primary"]["count"] == 0
                && c["primary"]["left_len"] == c["primary"]["right_len"]
                && c["flags"]["count"] == 0
                && c["halves_equal"] == true
                && c["room_tiles_equal"] == true
                && c["metrics_equal"] == true,
            "current native geometry differs from Rust: {}",
            serde_json::to_string(c)?
        );
    }
    ensure!(
        world[0x80] <= 1
            && world[0x81] <= 1
            && lot.floors[1].flags.upper_contents == (world[0x80] != 0)
            && lot.floors[1].flags.upper_support == (world[0x81] != 0),
        "native upper-floor summaries differ"
    );
    lot.validate().map_err(anyhow::Error::msg)?;
    Ok(Imported {
        lot,
        source: input.clone(),
        comparison: report,
        complete_native_lot_admitted: false,
    })
}
