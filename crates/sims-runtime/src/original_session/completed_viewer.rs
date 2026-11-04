//! Actual completed native viewer/Commander/cache owners. No F6, camera
//! setters, graph construction, callbacks or simulation instructions execute.
use super::completed_lot::{self, Memory};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_sim::lot_presenter::{
    self, camera, hover, lifecycle, motion, open, scroll, visibility,
    walls::{self, construction, cutaway::Mask},
};
use sims_vm::commander::{
    Node, Registry,
    house::{Fields, Interface, Receiver},
};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Imported {
    pub person_selection_override: u32,
    pub object_focus: u32,
    pub view: open::State,
    pub camera: camera::State,
    pub registry: Registry<Receiver>,
    pub fields: Fields,
    pub inputs: lot_presenter::Inputs,
    pub source: Value,
}
impl Imported {
    /// Only the actually proven synchronous completed caller is accepted.
    /// Cursor/list output storage is a fresh Rust delivery epoch; it neither
    /// replays F6 nor asserts that original future/event queues were empty.
    pub fn runtime_owners(
        &self,
        closure: &Value,
    ) -> Result<(
        super::presenter::State,
        super::notifications::State,
        sims_sim::pose::portrait::view::Input,
    )> {
        ensure!(
            closure["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
                && closure["image_sha256"]
                    == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f",
            "viewer caller proof epoch differs"
        );
        ensure!(
            closure["source_frame_unwind"]["completed_tick_ebp"] == 8518784u32
                && closure["source_frame_unwind"]["returns"]
                    == json!(["46da75", "4d1ab8", "4f7563", "5d5e7c"]),
            "viewer completed caller proof differs"
        );
        let presenter = super::presenter::State::idle(
            self.view.clone(),
            self.inputs.clone(),
            None,
            self.camera.clone(),
        )?;
        let notifications = super::notifications::State::from_completed_registry(
            self.registry.clone(),
            self.fields.clone(),
        )?;
        Ok((presenter, notifications, self.portrait_input()?))
    }
    /// Derive the portrait camera from the actually observed viewer. This is
    /// separate from the viewer's render camera and creates no service call.
    pub fn portrait_input(&self) -> Result<sims_sim::pose::portrait::view::Input> {
        let [left, top, right, bottom] = self.camera.settings.viewport;
        ensure!(
            left >= 0 && top >= 0 && right > left && bottom > top,
            "retained portrait viewport unavailable"
        );
        Ok(sims_sim::pose::portrait::view::Input {
            viewport: [
                left as u32,
                top as u32,
                (right - left) as u32,
                (bottom - top) as u32,
            ],
            center: self.view.origin,
            zoom: self.view.zoom,
            rotation: self.view.display_rotation,
        })
    }
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("viewer raw bytes absent")?;
    ensure!(
        s.is_ascii() && s.len() % 2 == 0 && s.len() <= (2 << 20),
        "viewer raw extent invalid"
    );
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("viewer field outside owner")?
            .try_into()?,
    ))
}
fn number(v: &Value) -> Result<u32> {
    Ok(v.as_u64().context("viewer pointer absent")?.try_into()?)
}
fn raw(m: &Memory, p: u32, n: usize, v: &Value) -> Result<Vec<u8>> {
    let b = m.read(p, n)?;
    ensure!(b == bytes(v)?, "viewer source field/readset differs");
    Ok(b)
}
fn mask(b: &[u8]) -> Result<Mask> {
    ensure!(b.len() == 512, "viewer mask extent differs");
    Ok(Mask {
        first_row: 0,
        rows: b
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect(),
    })
}
fn ptrs(m: &Memory, v: &Value) -> Result<Vec<u32>> {
    let h = raw(m, number(&v["pointer"])?, 12, &v["raw"])?;
    let (a, b, c) = (word(&h, 0)?, word(&h, 4)?, word(&h, 8)?);
    ensure!(
        a <= b && b <= c && (b - a) % 4 == 0 && (c - a) % 4 == 0 && (c - a) / 4 <= 16384,
        "wall pointer vector invalid"
    );
    let data = m.read(a, (b - a) as usize)?;
    ensure!(
        data == bytes(&v["values_raw"])?,
        "wall pointer vector bytes differ"
    );
    let result: Vec<_> = data
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    ensure!(
        serde_json::to_value(&result)? == v["values"],
        "wall pointer vector values differ"
    );
    Ok(result)
}
pub(super) fn graph(m: &Memory, input: &Value, lot: &Value, world: &[u8]) -> Result<construction::Graph> {
    let manager = word(world, 0x54)?;
    ensure!(
        number(&input["wall_manager"])? == manager,
        "wall manager alias differs"
    );
    let mr = raw(m, manager, 24, &input["wall_manager_raw"])?;
    let floors = input["wall_graph"]
        .as_array()
        .context("native wall graph absent")?;
    ensure!(floors.len() == 2, "native wall floor coverage differs");
    let mut out = construction::Graph {
        rotation: word(world, 0x84)? as u8,
        geometry: construction::Geometry {
            positions: vec![],
            rooms: vec![],
        },
        state: walls::State {
            corners: vec![],
            edges: vec![],
            floors: std::array::from_fn(|_| walls::Floor {
                corners: vec![],
                edges: vec![],
            }),
        },
    };
    let room_ids: BTreeMap<_, _> = lot["rooms"]
        .as_array()
        .context("source rooms absent")?
        .iter()
        .map(|r| Ok((number(&r["pointer"])?, number(&r["id"])? as u16)))
        .collect::<Result<_>>()?;
    for (floor, source) in floors.iter().enumerate() {
        let level = floor + 1;
        ensure!(
            source["level"].as_u64() == Some(level as u64),
            "wall floor order differs"
        );
        ensure!(
            number(&source["corners"]["pointer"])? == word(&mr, 4 + 4 * level)?
                && number(&source["edges"]["pointer"])? == word(&mr, 12 + 4 * level)?,
            "wall floor vector alias differs"
        );
        let cs = ptrs(m, &source["corners"])?;
        let es = ptrs(m, &source["edges"])?;
        let cb = out.state.corners.len();
        let eb = out.state.edges.len();
        let ci: BTreeMap<_, _> = cs.iter().enumerate().map(|(i, &p)| (p, cb + i)).collect();
        let ei: BTreeMap<_, _> = es.iter().enumerate().map(|(i, &p)| (p, eb + i)).collect();
        ensure!(
            ci.len() == cs.len() && ei.len() == es.len(),
            "duplicate native wall identity"
        );
        let records = source["corner_records"]
            .as_array()
            .context("wall corner records absent")?;
        ensure!(records.len() == cs.len(), "wall corner coverage differs");
        for (&p, record) in cs.iter().zip(records) {
            ensure!(
                number(&record["pointer"])? == p,
                "wall corner order differs"
            );
            let b = raw(m, p, 80, &record["raw"])?;
            let mut edges = [None; 4];
            for (i, e) in edges.iter_mut().enumerate() {
                let q = word(&b, 0x10 + 4 * i)?;
                if q != 0 {
                    *e = Some(
                        *ei.get(&q)
                            .context("wall corner edge outside current floor")?,
                    );
                }
            }
            out.geometry
                .positions
                .push([b[0] as i8, b[1] as i8, b[2] as i8]);
            out.state.corners.push(walls::Corner {
                base: b[8],
                masks: b[9..13].try_into()?,
                edges,
                valid: b[0x24..0x26].try_into()?,
                caps: [b[0x28..0x3c].try_into()?, b[0x3c..0x50].try_into()?],
            });
        }
        let records = source["edge_records"]
            .as_array()
            .context("wall edge records absent")?;
        ensure!(records.len() == es.len(), "wall edge coverage differs");
        for (&p, record) in es.iter().zip(records) {
            ensure!(number(&record["pointer"])? == p, "wall edge order differs");
            let b = raw(m, p, 60, &record["raw"])?;
            let (a, z, c) = (word(&b, 0x30)?, word(&b, 0x34)?, word(&b, 0x38)?);
            ensure!(
                a < z && z <= c && (z - a) % 6 == 0 && (c - a) % 6 == 0 && (z - a) / 6 <= 128,
                "native wall segment vector invalid"
            );
            let data = raw(m, a, (z - a) as usize, &record["segments_raw"])?;
            let segments = data
                .chunks_exact(6)
                .map(|b| walls::Segment {
                    flags: b[0],
                    style: b[1],
                    patterns: [
                        u16::from_le_bytes([b[2], b[3]]),
                        u16::from_le_bytes([b[4], b[5]]),
                    ],
                })
                .collect();
            let ends = [
                *ci.get(&word(&b, 0x20)?).context("native wall end absent")?,
                *ci.get(&word(&b, 0x24)?).context("native wall end absent")?,
            ];
            let mut rooms = [None; 2];
            for (i, r) in rooms.iter_mut().enumerate() {
                let q = word(&b, 0x28 + 4 * i)?;
                if q != 0 {
                    *r = Some(*room_ids.get(&q).context("native wall room owner absent")?);
                }
            }
            ensure!(
                serde_json::to_value(rooms)? == record["room_ids"],
                "native wall room values differ"
            );
            out.geometry.rooms.push(rooms);
            out.state.edges.push(walls::Edge {
                kind: word(&b, 0x10)? as i32,
                ends,
                bits: [b[0x1c], b[0x1d]],
                segments,
            });
        }
        out.state.floors[floor] = walls::Floor {
            corners: (cb..out.state.corners.len()).collect(),
            edges: (eb..out.state.edges.len()).collect(),
        };
    }
    out.validate().map_err(anyhow::Error::msg)?;
    Ok(out)
}

pub fn import(input: &Value, lot: &Value) -> Result<Imported> {
    for v in [input, lot] {
        ensure!(
            v["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
                && v["provenance"]["observation_sha256"]
                    == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
            "viewer source epoch differs"
        );
    }
    ensure!(
        input["tick"] == 88448
            && input["native_parity_ticks"] == 0
            && input["executes_target_code"] == false,
        "viewer observation profile differs"
    );
    let m = Memory::new(input)?;
    let lm = Memory::new(lot)?;
    for (address, count, hash) in [
        (
            0x40760d,
            50,
            "f61d622967c087fccc7f692727cd606005498d09ac2c98fc196028e5d885680f",
        ),
        (
            0x407577,
            42,
            "7ae14bd7464b7a4b4a619ffc7159b0f76bb8be411fb48aa4cf7b83c97cee482d",
        ),
        (
            0x4f65bf,
            128,
            "348142e0bd9e26013b0fb0da997118b4baf11b3a5fb2703fd9e8b8f554a02018",
        ),
        (
            0x4a82db,
            128,
            "5633621b40750d279a6db343952b69b7ab50f4278bcf14c14691358b51e92221",
        ),
        (
            0x4a6b15,
            128,
            "1afc41b6b8c9bce37af938ebb2c1f1b315c4aa3bcefe1556362e4a579643e4ca",
        ),
        (
            0x49e869,
            64,
            "64cdfc92a2a349391b1070640d5145c57ea5db04db5c2377a971e3b1b44209be",
        ),
        (
            0x4b0615,
            128,
            "ec7c8f2151a8e717da1d7fd6101890e112eaf0e74076aea3b6f5a31d0ac9f4d8",
        ),
        (
            0x416920,
            16,
            "e53578e063ce3c9a05389e48dddadd88cdda666e7071e50a1798e6ca9adb4d8a",
        ),
        (
            0x479375,
            16,
            "379526d0f6a2f183a5d77ab4bf21fdace8e8de93d306726ac7c1e77357c83512",
        ),
        (
            0x5408a6,
            16,
            "164ce66eb6f8f80e60b7c33adf6293f76aa76b09f770e95114b3d96bc67a44d2",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(address, count)?) == hash,
            "viewer source code profile differs"
        );
    }
    let owner = word(&m.read(0x6e48bc, 4)?, 0)?;
    ensure!(
        number(&input["owner"])? == owner,
        "viewer process owner differs"
    );
    let owner_raw = raw(&m, owner, 0x60, &input["owner_raw"])?;
    let viewer = word(&owner_raw, 4)?;
    ensure!(
        number(&input["viewer"])? == viewer,
        "viewer primary alias differs"
    );
    let vr = raw(&m, viewer, 0x408, &input["viewer_raw"])?;
    let world = word(&m.read(0x6e552c, 4)?, 0)?;
    ensure!(
        world == number(&input["world"])? && world == number(&lot["world"])?,
        "viewer world alias differs"
    );
    let wr = raw(&m, world, 0x88, &input["world_raw"])?;
    ensure!(
        wr == lm.read(world, 0x88)?,
        "viewer/lot world epoch differs"
    );
    let app = word(&owner_raw, 0)?;
    ensure!(
        number(&input["application"])? == app,
        "viewer application alias differs"
    );
    let _ar = raw(&m, app, 0x80, &input["application_raw"])?;
    let device = word(&vr, 0x54)?;
    ensure!(
        number(&input["device"])? == device,
        "viewer device alias differs"
    );
    let dr = raw(&m, device, 0x130, &input["device_raw"])?;
    let scalar = |name: &str, p: u32, n: usize| -> Result<Vec<u8>> {
        raw(&m, p, n, &input["scalars"][name])
    };
    let origin = scalar("origin_zoom_rotation", 0x6f00d0, 16)?;
    let hist = word(&vr, 0x1fc)?;
    ensure!(
        number(&input["history"]["pointer"])? == hist,
        "cutaway history alias differs"
    );
    let h = raw(&m, hist, 20, &input["history"]["raw"])?;
    let (a, z, c) = (word(&h, 4)?, word(&h, 8)?, word(&h, 12)?);
    ensure!(
        a <= z && z <= c && (z - a) % 4 == 0 && (c - a) % 4 == 0 && (c - a) / 4 <= 4096,
        "cutaway history vector invalid"
    );
    let hb = raw(&m, a, (z - a) as usize, &input["history"]["rooms_raw"])?;
    let view = open::State {
        floor: lifecycle::State {
            level: word(&vr, 0x18)? as i32,
            all_walls: vr[0x48],
            history: hover::History {
                capacity: word(&h, 0x10)? as usize,
                rooms: hb
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect(),
            },
            visibility: visibility::State {
                enabled: vr[0x49],
                dirty: vr[0x4e],
                include_visible: vr[0x1f8],
                mask: mask(&vr[0x208..0x408])?,
            },
        },
        origin: [word(&origin, 0)? as i32, word(&origin, 4)? as i32],
        pending: [word(&vr, 0x170)? as i32, word(&vr, 0x174)? as i32],
        zoom: word(&origin, 8)?,
        display_rotation: word(&origin, 12)?,
        widths: std::array::from_fn(|i| word(&vr, 0x58 + 4 * i).unwrap() as i32),
    };
    view.floor.validate().map_err(anyhow::Error::msg)?;
    let mut extrema = BTreeMap::new();
    for e in input["extrema"]
        .as_array()
        .context("native extrema absent")?
    {
        let name = e["kind"].as_str().context("extrema kind absent")?;
        let offset = match name {
            "maximum" => 0x1ec,
            "minimum" => 0x1f0,
            _ => anyhow::bail!("unknown extrema kind"),
        };
        let p = word(&vr, offset)?;
        ensure!(
            p != 0 && number(&e["pointer"])? == p,
            "native extrema unavailable/aliased"
        );
        ensure!(
            extrema.insert(name, raw(&m, p, 128, &e["raw"])?).is_none(),
            "duplicate native extrema"
        );
    }
    let mut rooms = BTreeMap::new();
    for room in lot["rooms"].as_array().context("source room list absent")? {
        let p = number(&room["pointer"])?;
        let b = raw(&lm, p, 672, &room["raw"])?;
        let id = u16::from_le_bytes(b[..2].try_into()?);
        ensure!(
            number(&room["id"])? == id as u32,
            "camera room identity differs"
        );
        rooms.insert(
            id,
            visibility::Room {
                outdoors: b[0x72] != 0,
                mask: mask(&b[0x80..0x280])?,
            },
        );
    }
    let imported_lot = completed_lot::import(lot)?;
    let motion: Vec<_> = (0..13)
        .map(|i| word(&vr, 0x184 + 4 * i))
        .collect::<Result<_>>()?;
    let settings = camera::Settings {
        roof: Some(vr[0x51]),
        clock: word(&scalar("clock", 0x6e54e0, 4)?, 0)?,
        application_mode: word(&wr, 8)? as i32,
        viewport: std::array::from_fn(|i| word(&vr, 0x28 + 4 * i).unwrap() as i32),
        bounds: std::array::from_fn(|i| word(&wr, 0x70 + 4 * i).unwrap() as i32),
        world_rotation: word(&wr, 0x84)?,
        camera_mode: scalar("camera_mode", 0x6e48c0, 1)?[0],
        moving: vr[0x17a],
        projection_ready: word(&vr, 0x68)?,
        renderer_ready: word(&dr, 4)? != 0,
        graphics_enabled: word(&vr, 0x1c)? != 0,
        ui_present: word(&scalar("global_focus", 0x6f6bac, 4)?, 0)? != 0,
        skip_edges: scalar("skip_edges", 0x6f00e5, 1)?[0],
        depth_scales: std::array::from_fn(|i| word(&dr, (8 - i) * 4).unwrap()),
        floor_heights: (0..4)
            .map(|i| word(&vr, (0x4c - i) * 4).map(|n| n as i32))
            .collect::<Result<_>>()?,
        deferred: vr[0x178],
        locked: vr[0x179],
        attempts: word(&scalar("scroll_attempts", 0x6e54f8, 4)?, 0)?,
        bottom_margin: word(&vr, 0x1f4)? as i32,
        motion: motion::State {
            x: motion[..4].try_into()?,
            y: motion[4..8].try_into()?,
            active: motion[8],
            policy: motion[9],
            duration: motion[10] as i32,
            speed_bits: motion[11],
            rate_bits: motion[12],
        },
        duration_input: word(&scalar("duration_input", 0x6c7788, 4)?, 0)? as i32,
        application_value_bits: word(&scalar("application_value", 0x6fe888, 4)?, 0)?,
        saved_value_bits: word(&vr, 0x1b8)?,
    };
    let camera = camera::State {
        world_size: Some(word(&wr, 0x10)? as i32),
        settings,
        graph: graph(&m, input, lot, &wr)?,
        extrema: scroll::Extrema {
            maximum: extrema
                .remove("maximum")
                .context("maximum extrema missing")?,
            minimum: extrema
                .remove("minimum")
                .context("minimum extrema missing")?,
        },
        rooms,
        tiles: imported_lot
            .lot
            .floor(1)
            .context("camera presentation tiles absent")?
            .source
            .coverings
            .clone(),
    };
    camera.validate(&view).map_err(anyhow::Error::msg)?;
    let globals = raw(&m, 0x6de858, 8, &input["commander"]["globals_raw"])?;
    let mut nodes = BTreeMap::new();
    let mut order = vec![];
    let mut next = word(&globals, 0)?;
    let mut recipient = BTreeMap::new();
    for node in input["commander"]["nodes"]
        .as_array()
        .context("native recipients absent")?
    {
        let p = number(&node["pointer"])?;
        ensure!(p == next && p != 0, "Commander source order differs");
        let b = raw(&m, p, 16, &node["raw"])?;
        let vt = word(&b, 0)?;
        ensure!(
            number(&node["vtable"])? == vt,
            "Commander vtable alias differs"
        );
        let table = raw(&m, vt, 8, &node["vtable_raw"])?;
        let handler = word(&table, 4)?;
        let kind = match handler {
            0x41637f => Receiver::World,
            0x43c0e4 => Receiver::Module,
            0x44302e => Receiver::Objects,
            0x46d2c2 => Receiver::Globals,
            0x479375 => Receiver::Lot,
            0x4a82db => Receiver::Viewer,
            0x4c3ea5 => Receiver::Renderer,
            0x4f65bf => Receiver::Interface,
            _ => anyhow::bail!("unqualified native Commander handler {handler:x}"),
        };
        ensure!(
            serde_json::to_value(kind)? == node["receiver"] && number(&node["handler"])? == handler,
            "Commander receiver differs"
        );
        next = word(&b, 4)?;
        ensure!(
            nodes
                .insert(
                    p,
                    Node {
                        next: (next != 0).then_some(next),
                        auxiliary: word(&b, 8)?,
                        serial: word(&b, 12)?,
                        handler: kind
                    }
                )
                .is_none(),
            "duplicate Commander identity"
        );
        order.push(kind);
        recipient.insert(format!("{kind:?}"), p);
    }
    ensure!(
        next == 0 && nodes.len() == 8 && recipient.len() == 8,
        "native Commander coverage differs"
    );
    ensure!(
        recipient["Viewer"] == viewer + 8,
        "secondary viewer recipient alias differs"
    );
    let registry = Registry {
        head: (word(&globals, 0)? != 0).then_some(word(&globals, 0)?),
        generation: word(&globals, 4)?,
        nodes,
    };
    registry.validate().map_err(anyhow::Error::msg)?;
    let ui = recipient["Interface"];
    let renderer = recipient["Renderer"];
    let ur = m.read(ui, 0x100)?;
    let panel = word(&ur, 0x1c)?;
    let shared = word(&scalar("shared_root", 0x6fe944, 4)?, 0)?;
    let shared_present = if shared == 0 {
        false
    } else {
        let b = raw(&m, shared + 500, 4, &input["scalars"]["shared_present_raw"])?;
        word(&b, 0)? != 0
    };
    let fields = Fields {
        viewer_interface: word(&scalar("viewer_interface", 0x6e54fc, 4)?, 0)? != 0,
        include_visible: vr[0x1f8],
        renderer_dirty: m.read(renderer + 0x1e, 1)?[0],
        ui: Interface {
            mask_owner_present: word(&ur, 0x48)? != 0,
            panel_active: panel != 0 && word(&m.read(panel + 0x11ac, 4)?, 0)? != 0,
            shared_present,
            gated: ur[0x8c],
            creation_gated: ur[0x8d],
            dirty: ur[0x8f],
        },
    };
    let originals: BTreeMap<_, _> = lot["objects"]
        .as_array()
        .context("original presenter objects absent")?
        .iter()
        .map(|o| Ok((number(&o["object"])? as i16, number(&o["pointer"])?)))
        .collect::<Result<_>>()?;
    let graphic_ready: BTreeMap<_, _> = input["graphic_ready"]
        .as_array()
        .context("presenter objects absent")?
        .iter()
        .map(|o| {
            let id = number(&o["object"])? as i16;
            let pointer = number(&o["pointer"])?;
            ensure!(
                originals.get(&id) == Some(&pointer),
                "ordinary readiness registry binding differs"
            );
            let b = raw(&m, pointer + 0x180, 1, &o["raw"])?;
            ensure!(
                b == lm.read(pointer + 0x180, 1)?,
                "ordinary readiness epoch differs"
            );
            Ok((id, b[0]))
        })
        .collect::<Result<_>>()?;
    ensure!(
        graphic_ready.len() == 344 && graphic_ready.keys().eq(originals.keys()),
        "ordinary readiness coverage differs"
    );
    Ok(Imported {
        person_selection_override: word(&scalar("person_selection_override", 0x6e4f64, 4)?, 0)?,
        object_focus: word(&vr, 0x1bc)?,
        view,
        camera,
        registry,
        fields,
        inputs: lot_presenter::Inputs {
            force_dynamic: owner_raw[0x5c] != 0,
            graphic_ready,
            keep_dynamic: BTreeMap::new(),
        },
        source: json!({"schema":1,"tick":88448,"native_parity_ticks":0,"provenance":input["provenance"],"commander_order":order,"no_constructors_setters_or_callbacks_executed":true,"complete_native_Session_admitted":false}),
    })
}
