//! Retain the actual room caches; never run Manager::new or a score rebuild.
use super::*;
use sims_sim::lot_environment::{
    Aggregate, Scores, Totals, Tuning, lighting,
    manager::{Manager, Room, Settings},
};
use sims_vm::World;

const CODE: &[(u32, usize, &str)] = &[
    (
        0x4665d1,
        699,
        "67633c22b0542da71cc2fe762e06625e185c1615e82013314923af15cd9286a2",
    ),
    (
        0x466c3c,
        720,
        "49773f11bfce0f141639a7f37a627ba3419672d51681d2e1f403b121ca93b91f",
    ),
    (
        0x4673d0,
        63,
        "e71837722e5b90c09e0f266f584b8690ea5041dd5f443cb7a9af10cad6fbbd8c",
    ),
    (
        0x46740f,
        39,
        "8e8c064119c34c1b0a563d1ea0b782b0213da81c775d943a028269f22fbbada0",
    ),
    (
        0x46772d,
        110,
        "482d6d3c160e747022fecbcfc64bd64f616f60168f983333c5e2095d01412533",
    ),
    (
        0x467980,
        500,
        "457826fc64ad6561aff5d0fa5d03cdd9ed2bf70de4992ef4d4351bffb8c26ecf",
    ),
    (
        0x468802,
        106,
        "548ddfeca6e6a146f1093ab21155bca1114c4fb4b417d88dde99086d4dedac41",
    ),
    (
        0x4688bf,
        53,
        "1ee92133354f45edc3e8d978b81e3f87053a63987b69143c0c86cf710f9568a9",
    ),
    (
        0x478909,
        49,
        "d877d18699ac5b6a43e8da841f7864f435fc7a13fa00820b14b2ded44676817d",
    ),
    (
        0x4b4e8b,
        428,
        "12bc73720bcdca46d968e3ea885dc29de689aa54d6103ae213c8e977974635f8",
    ),
];
const TUNING: &[(&str, u32)] = &[
    ("object_light", 0x6c4768),
    ("area_clip", 0x6e5424),
    ("area_zero", 0x6e5420),
    ("area_slope", 0x6e541c),
    ("light_zero", 0x6e5418),
    ("light_slope", 0x6e5414),
    ("impact_area_min", 0x6e5410),
    ("impact_area_max", 0x6e540c),
    ("impact_multiplier", 0x6e5408),
    ("outdoor_impact", 0x6e5404),
    ("outdoor_good", 0x6e5400),
    ("outdoor_divisor", 0x6e53fc),
    ("daylight_bonus", 0x6e53f8),
    ("dawn_dusk_bonus", 0x6e53f4),
    ("nighttime_bonus", 0x6e53f0),
    ("floor_zero", 0x6e53e8),
    ("floor_slope", 0x6e53ec),
    ("wall_zero", 0x6e53e0),
    ("wall_slope", 0x6e53e4),
];
const TOTALS: &[(&str, usize)] = &[
    ("half_tiles", 0x38),
    ("covered_half_tiles", 0x3c),
    ("objects", 0x40),
    ("window_light", 0x44),
    ("artificial_light", 0x48),
    ("object_field39", 0x4c),
    ("impact", 0x50),
    ("good", 0x54),
    ("category4", 0x58),
    ("category5_to7", 0x5c),
    ("walls", 0x60),
    ("covered_walls", 0x64),
    ("lights", 0x280),
    ("people", 0x284),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub manager: Manager,
    pub source: Value,
    /// This replacement-only diagnostic has no native persistent float owner.
    /// manager::update overwrites it before reading; occupancy also writes it.
    pub publication_diagnostic_initialized_from_current_effective_light: bool,
    pub complete_native_lot_admitted: bool,
}
fn vector(m: &Memory, v: &Value, data: &[u8], stride: usize) -> Result<()> {
    let p = pointer(&v["address"])?;
    let raw = bytes(&v["raw"])?;
    ensure!(
        raw.len() == 12 && m.read(p, 12)? == raw,
        "room vector header differs"
    );
    let (b, e, c) = (word(&raw, 0)?, word(&raw, 4)?, word(&raw, 8)?);
    ensure!(
        (b >= 0x10000 || b == 0 && e == 0 && c == 0)
            && b <= e
            && e <= c
            && (e - b) as usize == data.len()
            && data.len() % stride == 0
            && (c - b) as usize % stride == 0
            && v["begin"] == b
            && v["end"] == e
            && v["capacity"] == c,
        "room vector extent differs"
    );
    ensure!(
        data.is_empty() || m.read(b, data.len())? == data,
        "room vector payload differs"
    );
    Ok(())
}
pub fn import(geometry: &super::Imported, input: &Value, world: &World) -> Result<Imported> {
    ensure!(
        input["schema"] == 1
            && input["provenance"]["image_sha256"] == geometry.source["provenance"]["image_sha256"]
            && input["provenance"]["observation_sha256"]
                == geometry.source["provenance"]["observation_sha256"],
        "room source epoch differs"
    );
    let m = Memory::new(input)?;
    for &(p, n, hash) in CODE {
        ensure!(
            sims_fixtures::digest(&m.read(p, n)?) == hash,
            "native room layout code differs"
        );
    }
    let owner = pointer(&input["owner"])?;
    let owner_raw = bytes(&input["owner_raw"])?;
    let application = pointer(&input["application"])?;
    let app = bytes(&input["application_raw"])?;
    let manager = pointer(&input["manager"])?;
    let raw = bytes(&input["manager_raw"])?;
    let native_world = pointer(&input["world"])?;
    let world_raw = bytes(&input["world_raw"])?;
    ensure!(
        word(&m.read(0x6e48bc, 4)?, 0)? == owner
            && owner_raw.len() == 0x28
            && m.read(owner, 0x28)? == owner_raw
            && word(&owner_raw, 0)? == application
            && app.len() == 0x28
            && m.read(application, 0x28)? == app
            && word(&app, 0x18)? == native_world
            && word(&app, 0x24)? == manager
            && raw.len() == 0x80
            && m.read(manager, 0x80)? == raw
            && input["manager_raw"] == geometry.source["manager_raw"]
            && input["world_raw"] == geometry.source["world_raw"]
            && m.read(native_world, 0x88)? == world_raw,
        "native room owner aliases differ"
    );
    ensure!(
        input["tuning_ready"] == 1 && m.read(0x6e542c, 1)? == [1] && raw[0x38] == 1,
        "room cached tuning/models uninitialized"
    );
    let mut tuning = serde_json::Map::new();
    for &(name, p) in TUNING {
        let bits = word(&m.read(p, 4)?, 0)?;
        let value = f32::from_bits(bits);
        ensure!(
            input["tuning"][name] == bits && value.is_finite(),
            "cached native room tuning differs"
        );
        tuning.insert(name.into(), json!(value));
    }
    let tuning: Tuning = serde_json::from_value(Value::Object(tuning))?;
    let mut settings = vec![];
    for (name, p) in [
        ("daylight_multiplier", 0x6f015c),
        ("light_range", 0x6f016c),
        ("rooms_enabled", 0x6e5428),
        ("ambient", 0x6c741c),
    ] {
        let bits = word(&m.read(p, 4)?, 0)?;
        ensure!(input["settings"][name] == bits, "room setting differs");
        settings.push(bits);
    }
    ensure!(
        settings[2] == manager
            && f32::from_bits(settings[0]).is_finite()
            && f32::from_bits(settings[1]).is_finite(),
        "room settings/manager differ"
    );
    let pool = bytes(&input["pool_raw"])?;
    vector(&m, &input["pool"], &pool, 4)?;
    ensure!(
        pool.len() == 24 && input["pool"]["address"] == manager + 0x28,
        "native light model pool differs"
    );
    let pool: Vec<_> = pool
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    ensure!(
        pool.iter()
            .filter(|p| **p != 0)
            .collect::<BTreeSet<_>>()
            .len()
            == pool.iter().filter(|p| **p != 0).count(),
        "aliased native light pool slots"
    );
    let settings = Settings {
        daylight_multiplier: f32::from_bits(settings[0]),
        light_range: f32::from_bits(settings[1]),
        rooms_enabled: settings[2] != 0,
        models: std::array::from_fn(|i| if pool[i] == 0 { None } else { Some(i as u8) }),
    };
    let head = pointer(&input["room_head"])?;
    let head_raw = bytes(&input["room_head_raw"])?;
    ensure!(
        word(&raw, 4)? == head && head_raw.len() == 24 && m.read(head, 24)? == head_raw,
        "room directory sentinel differs"
    );
    let mut nodes = BTreeMap::new();
    let mut identities = BTreeMap::new();
    for n in input["directory"]
        .as_array()
        .context("room directory absent")?
    {
        let p = pointer(&n["pointer"])?;
        let b = bytes(&n["raw"])?;
        ensure!(
            b.len() == 24
                && m.read(p, 24)? == b
                && n["id"] == i16::from_le_bytes(b[0x10..0x12].try_into()?)
                && n["room"] == word(&b, 0x14)?,
            "native room directory node differs"
        );
        ensure!(
            nodes.insert(p, [word(&b, 8)?, word(&b, 12)?]).is_none()
                && identities
                    .insert(n["id"].as_i64().unwrap() as u16, word(&b, 0x14)?)
                    .is_none(),
            "duplicate native room directory identity"
        );
    }
    let mut pending = vec![word(&head_raw, 4)?];
    let mut seen = BTreeSet::new();
    while let Some(p) = pending.pop() {
        if p == 0 || p == head {
            continue;
        }
        ensure!(seen.insert(p), "room directory cycle");
        pending.extend(nodes.get(&p).context("room directory child omitted")?);
    }
    ensure!(
        nodes.len() == word(&raw, 8)? as usize && nodes.len() == seen.len(),
        "room directory coverage differs"
    );
    let mut rooms = BTreeMap::new();
    for row in input["rooms"].as_array().context("current rooms absent")? {
        let id: u16 = row["id"].as_u64().context("room id absent")?.try_into()?;
        let p = pointer(&row["pointer"])?;
        let b = bytes(&row["raw"])?;
        let source = geometry.source["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .context("geometry room absent")?;
        ensure!(
            identities.get(&id) == Some(&p)
                && source["raw"] == row["raw"]
                && b.len() == 0x2a0
                && m.read(p, 0x2a0)? == b
                && word(&b, 0x78)? == manager
                && word(&b, 0x34)? <= 1
                && b[0x71] == 1
                && b[0x72..0x76].iter().all(|v| *v <= 1),
            "current native room owner differs"
        );
        let totals: Totals = serde_json::from_value(Value::Object(
            TOTALS
                .iter()
                .map(|&(n, o)| Ok((n.into(), json!(word(&b, o)? as i32))))
                .collect::<Result<_>>()?,
        ))?;
        let model_raw = bytes(&row["models_raw"])?;
        let position_raw = bytes(&row["positions_raw"])?;
        vector(&m, &row["models"], &model_raw, 4)?;
        vector(&m, &row["positions"], &position_raw, 12)?;
        ensure!(
            model_raw.len() <= 24
                && model_raw.len() / 4 == position_raw.len() / 12
                && row["models"]["address"] == p + 0x10
                && row["positions"]["address"] == p + 0x1c,
            "room light model ownership differs"
        );
        for (i, model) in model_raw.chunks_exact(4).enumerate() {
            ensure!(
                u32::from_le_bytes(model.try_into()?) == pool[i],
                "room light pool alias differs"
            );
        }
        let mut room = Room {
            active: word(&b, 0x34)? != 0,
            outdoors: b[0x72] != 0,
            occupied: b[0x73] != 0,
            category8: b[0x74] != 0,
            pool: b[0x75] != 0,
            tiles: bytes(&source["tiles_raw"])?
                .chunks_exact(3)
                .map(|p| [p[0] as usize, p[1] as usize, p[2] as usize])
                .collect(),
            totals,
            scores: Scores {
                light: word(&b, 0x6c)?,
                environment: word(&b, 0x68)?,
                published_light: word(&b, 0x6c)?,
            },
            models: settings.models[..model_raw.len() / 4].to_vec(),
            positions: position_raw
                .chunks_exact(12)
                .map(|b| {
                    std::array::from_fn(|i| {
                        u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
                    })
                })
                .collect(),
            route_rectangles: bytes(&source["rectangles_raw"])?
                .chunks_exact(20)
                .map(|b| {
                    sims_sim::regions::Region(std::array::from_fn(|i| {
                        i32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap())
                    }))
                })
                .collect(),
        };
        room.scores.published_light = room.effective_light().to_bits();
        let refresh = world
            .refresh
            .as_ref()
            .and_then(|r| r.rooms.get(&id))
            .context("World room refresh absent")?;
        ensure!(
            refresh.dirty == (b[0x70] != 0)
                && refresh.category == ((id >> 10) & 3) as u8
                && world.room_outdoors.get(&(id as i16)) == Some(&room.outdoors)
                && world.room_half_tiles.get(&(id as i16)) == Some(&room.totals.half_tiles),
            "native room/World caches differ for {id}: dirty actual {:?} native {}; outdoors actual {:?} native {}; half_tiles actual {:?} native {}", refresh.dirty,b[0x70]!=0,world.room_outdoors.get(&(id as i16)),room.outdoors,world.room_half_tiles.get(&(id as i16)),room.totals.half_tiles
        );
        ensure!(rooms.insert(id, room).is_none(), "duplicate current room");
    }
    ensure!(
        rooms.keys().eq(identities.keys()),
        "native room coverage differs"
    );
    let mut light_bytes = vec![];
    let mut light_maps = vec![];
    ensure!(
        input["light_bytes"]
            .as_array()
            .is_some_and(|a| a.len() == 2)
            && input["light_maps"].as_array().is_some_and(|a| a.len() == 2),
        "native light floors absent"
    );
    for level in 0..2 {
        let a = &input["light_bytes"][level];
        let p = pointer(&a["pointer"])?;
        let header = bytes(&a["raw"])?;
        let columns = bytes(&a["columns_raw"])?;
        let data = bytes(&a["data_raw"])?;
        ensure!(
            word(&raw, 0x44 + level * 4)? == p
                && header.len() == 32
                && m.read(p, 32)? == header
                && word(&header, 4)? == 64
                && word(&header, 8)? == 64
                && word(&header, 28)? == 1
                && columns.len() == 256
                && m.read(word(&header, 12)?, 256)? == columns
                && data.len() == 4096,
            "native room light byte array differs"
        );
        for (i, c) in columns.chunks_exact(4).enumerate() {
            ensure!(
                m.read(u32::from_le_bytes(c.try_into()?), 64)? == data[i * 64..i * 64 + 64],
                "native light byte column differs"
            );
        }
        light_bytes.push(data);
        let a = &input["light_maps"][level];
        let data = bytes(&a["data_raw"])?;
        vector(&m, &a["vector"], &data, 4)?;
        ensure!(
            a["pointer"] == word(&world_raw, 0x68 + level * 4)?
                && a["vector"]["address"] == a["pointer"]
                && data.len() == 16384,
            "native local light map differs"
        );
        light_maps.push(lighting::Map {
            cells: data
                .chunks_exact(4)
                .map(|b| std::array::from_fn(|i| b[i] as i8))
                .collect(),
        });
    }
    let aggregate = Aggregate {
        indoor_area: i32::from(
            *world
                .globals
                .get(&27)
                .context("native published indoor area absent")?,
        ),
        outdoor_sum: word(&raw, 0x40)?,
        outdoor_environment: word(&raw, 0x3c)?,
        dirty_floors: word(&raw, 0x34)?,
    };
    ensure!(
        aggregate.dirty_floors
            == world
                .refresh
                .as_ref()
                .context("native refresh absent")?
                .dirty_categories,
        "room dirty-floor alias differs"
    );
    Ok(Imported {
        manager: Manager {
            size: geometry.lot.size,
            tuning,
            settings,
            rooms,
            aggregate,
            light_bytes,
            light_maps,
            ambient: word(&m.read(0x6c741c, 4)?, 0)?,
        },
        source: input.clone(),
        publication_diagnostic_initialized_from_current_effective_light: true,
        complete_native_lot_admitted: false,
    })
}
