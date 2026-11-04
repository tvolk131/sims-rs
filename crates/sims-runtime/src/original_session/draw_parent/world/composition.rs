//! Retained 4ab992 view owner, borrowed VM selection/terrain and explicit OS
//! point replies. Active cursor-tool callbacks remain unsupported services.
use super::*;
mod points;
use anyhow::Context as _;
pub use points::Inputs as PointInputs;
use sims_sim::lot_presenter::{room_masks, visibility, walls::cutaway::Mask};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    GetCursorPos,
    ScreenToClient { window: u32, point: [i32; 2] },
    Project,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Ready,
    Cursor,
    Client,
    Project,
    Finish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    source: String,
    owner: u32,
    size: i32,
    bounds: [i32; 4],
    rotation: u8,
    level: i32,
    history: Vec<u16>,
    rooms: BTreeMap<u16, visibility::Room>,
    camera: room_masks::Camera,
    window: u32,
    dimensions: [i32; 2],
    debug_hidden: u8,
    tool_active: u8,
    pub visibility: visibility::State,
    stage: Stage,
    pending: Option<visibility::Pending>,
    point: Option<[i32; 2]>,
    cursor_calls: usize,
    pub projected: Option<[i32; 2]>,
    pub cursor_end: Option<[i8; 3]>,
}
fn mask(raw: &[u8]) -> Mask {
    Mask {
        first_row: 0,
        rows: raw
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect(),
    }
}
impl View {
    pub(in crate::original_session::draw_parent) fn object_visibility(&self)->Result<visibility::State>{self.validate()?;ensure!(self.stage==Stage::Complete,"object drawing precedes composition completion");Ok(self.visibility.clone())}
    pub(super) fn owner(&self)->u32 {self.owner}
    pub(super) fn validate_frame_camera(&self,origin:[i32;2],zoom:u32,level:i32)->Result<()>{self.validate()?;ensure!(self.camera.origin==origin&&u32::from(self.camera.zoom)==zoom&&self.level==level,"camera preparation lost shared view coordinates");Ok(())}
    pub(super) fn publish_frame_origin(&mut self,origin:[i32;2]){self.camera.origin=origin;}

    pub(super) fn object_rectangle(&self,mut rect:[i32;4],viewport:[i32;4])->Option<[i32;4]>{
        if rect[0]>=rect[2]||rect[1]>=rect[3]{return None;}
        for i in 0..4{rect[i]=rect[i].wrapping_add(self.camera.origin[i%2]);}
        let rect=room_masks::clip_rectangle(rect,viewport);
        (rect[0]<rect[2]&&rect[1]<rect[3]).then_some(rect)
    }
    pub fn import(source: &[u8]) -> Result<Self> {
        let digest = sims_fixtures::digest(source);
        ensure!(
            digest == "2cd3676d8b1fc6a082427ee5f1dace0fe27c6c851b6af5efcf0e80472912173b",
            "composition source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(source)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["native_parity_credit"] == 0,
            "composition source epoch differs"
        );
        let m = Memory::new(&v)?;
        let process = word(&m.read(0x6e48bc, 4)?, 0);
        let pr = m.read(process, 0x60)?;
        let owner = word(&pr, 4);
        ensure!(owner == 0x09e88960, "composition view alias differs");
        let vr = m.read(owner, 0x4c0)?;
        let table = m.read(word(&vr, 0), 0x60)?;
        ensure!(
            word(&table, 4) == 0x4a97c3
                && word(&table, 0x14) == 0x4aaf3c
                && word(&table, 0x38) == 0x4a6e0d,
            "composition projection getters differ"
        );
        let app = m.read(word(&pr, 0), 0x80)?;
        let lot = word(&app, 0x18);
        ensure!(
            word(&m.read(0x6e552c, 4)?, 0) == lot && word(&m.read(0x6e5528, 4)?, 0) == lot,
            "composition lot aliases differ"
        );
        let lr = m.read(lot, 0x88)?;
        let manager = word(&app, 0x24);
        ensure!(
            manager == word(&m.read(0x6e5428, 4)?, 0),
            "composition room directory alias differs"
        );
        let mut rooms = BTreeMap::new();
        for room in v["rooms"]
            .as_array()
            .context("composition room directory absent")?
        {
            let p = u32::try_from(room["pointer"].as_u64().context("room pointer absent")?)?;
            let body = m.read(p, 0x2a0)?;
            let id = u16::from_le_bytes(body[..2].try_into()?);
            ensure!(room["id"] == id, "composition room identity differs");
            ensure!(
                rooms
                    .insert(
                        id,
                        visibility::Room {
                            outdoors: body[0x72] != 0,
                            mask: mask(&body[0x80..0x280])
                        }
                    )
                    .is_none(),
                "duplicate composition room"
            );
        }
        let hr = m.read(word(&vr, 0x1fc), 20)?;
        let (a, z, c) = (word(&hr, 4), word(&hr, 8), word(&hr, 12));
        ensure!(
            a <= z && z <= c && (z - a) % 4 == 0 && (c - a) % 4 == 0 && c - a <= 16384,
            "composition history extent differs"
        );
        let history = m
            .read(a, (z - a) as usize)?
            .chunks_exact(4)
            .map(|b| word(b, 0) as u16)
            .collect();
        let ui = m.read(word(&m.read(0x6f2a58, 4)?, 0), 0xc0)?;
        let debug = m.read(word(&ui, 0x84), 0x178)?;
        let child = word(&debug, 0x174);
        ensure!(
            child != 0,
            "composition needs retained debug-view construction"
        );
        let cr = m.read(child, 0x100)?;
        let ct = m.read(word(&cr, 0), 0x100)?;
        ensure!(
            sims_fixtures::digest(&m.read(word(&ct, 0x54), 7)?) == "555ec9d4ac5626fac7385093e799055f2fa16dc8edbca263a9b6d93bbbdd7885",
            "debug-view predicate differs"
        );
        ensure!(
            word(&m.read(0x6f6bac, 4)?, 0) == 0,
            "composition focus override needs retained lookup"
        );
        let interface = word(&m.read(0x6e54fc, 4)?, 0);
        ensure!(interface != 0, "composition interface profile differs");
        let ir = m.read(interface, 0x100)?;
        let it = m.read(word(&ir, 0), 0x48)?;
        ensure!(
            word(&it, 0x40) == 0x492c14 && sims_fixtures::digest(&m.read(word(&it, 0x44), 3)?) == "e598d0c3ba86d917b177d7adde0556aa99bc355543c57aee0a3e50b684dd7e99",
            "cursor interface handlers differ"
        );
        ensure!(
            m.read(0x6e4878, 6)? == [255, 255, 0, 1, 1, 0],
            "cursor clipping steps differ"
        );
        let drawing = m.read(word(&pr, 0x2c), 0x520)?;
        let coords = m.read(0x6f00d0, 24)?;
        let result = Self {
            source: digest,
            owner,
            size: word(&lr, 0x10) as i32,
            bounds: std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
            rotation: word(&lr, 0x84) as u8,
            level: word(&vr, 0x18) as i32,
            history,
            rooms,
            camera: room_masks::Camera {
                origin: [word(&coords, 0) as i32, word(&coords, 4) as i32],
                zoom: word(&coords, 8) as u8,
                tile_width: word(&vr, 0x58) as i32,
                half_width: word(&vr, 0x5c) as i32,
                bottom: word(&vr, 0x60) as i32,
                floor_heights: (0..4).map(|i| word(&vr, 0x130 - 4 * i) as i32).collect(),
            },
            window: word(&drawing, 0x128),
            dimensions: [word(&drawing, 0x48) as i32, word(&drawing, 0x4c) as i32],
            debug_hidden: cr[0x93],
            tool_active: ir[0x28],
            visibility: visibility::State {
                enabled: vr[0x49],
                dirty: vr[0x4e],
                include_visible: vr[0x1f8],
                mask: mask(&vr[0x208..0x408]),
            },
            stage: Stage::Ready,
            pending: None,
            point: None,
            cursor_calls: 0,
            projected: None,
            cursor_end: None,
        };
        result.validate()?;
        Ok(result)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == "2cd3676d8b1fc6a082427ee5f1dace0fe27c6c851b6af5efcf0e80472912173b"
                && self.owner == 0x09e88960
                && self.size == 52
                && self.rotation < 4,
            "composition owner differs"
        );
        ensure!(
            self.visibility.mask.first_row == 0 && self.visibility.mask.rows.len() == 64,
            "composition mask extent differs"
        );
        ensure!(
            self.pending.is_some() == !matches!(self.stage, Stage::Ready | Stage::Complete),
            "composition stack lifetime differs"
        );
        let expected_calls = match self.stage {
            Stage::Ready | Stage::Cursor => 0,
            Stage::Client => 1,
            Stage::Project => 2,
            Stage::Finish | Stage::Complete => {
                if self.point.is_some() {
                    2
                } else {
                    0
                }
            }
        };
        ensure!(
            self.cursor_calls == expected_calls,
            "composition cursor reply lifetime differs"
        );
        if let Some(p) = &self.pending {
            p.validate().map_err(anyhow::Error::msg)?;
        }
        ensure!(
            !matches!(self.stage, Stage::Client | Stage::Project) || self.point.is_some(),
            "composition cursor storage absent"
        );
        Ok(())
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Cursor => Some(Request::GetCursorPos),
            Stage::Client => self.point.map(|point| Request::ScreenToClient {
                window: self.window,
                point,
            }),
            Stage::Project => Some(Request::Project),
            _ => None,
        }
    }
    pub fn cursor_calls(&self) -> usize {
        self.cursor_calls
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub(super) fn advance(
        &mut self,
        simulation: &sims_sim::Simulation,
        controls: &sims_vm::interface::state::State,
        include_visible: u8,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Ready => {
                ensure!(
                    self.tool_active == 0,
                    "active cursor tool requires its retained mask continuation"
                );
                let size = simulation
                    .vm
                    .world
                    .map_size
                    .as_ref()
                    .context("composition live map size absent")?
                    .active_size;
                ensure!(
                    size == self.size,
                    "composition view needs resize publication"
                );
                let selected = if controls.mode == 2 {
                    simulation
                        .native_selected_person()
                        .map_err(anyhow::Error::msg)?
                        .map(|id| -> Result<_> {
                            let p = simulation
                                .vm
                                .world
                                .positions
                                .get(&id)
                                .context("composition selected position absent")?;
                            let room = *simulation
                                .native_lot
                                .as_ref()
                                .context("composition physical owner absent")?
                                .membership
                                .0
                                .get(&id)
                                .context("composition cached room absent")?
                                as u16;
                            Ok(visibility::Selected {
                                room,
                                level: p.level,
                                tile: [(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8],
                            })
                        })
                        .transpose()?
                } else {
                    None
                };
                let pending = self
                    .visibility
                    .begin(visibility::Inputs {
                        size,
                        rotation: self.rotation,
                        bounds: self.bounds,
                        visible_level: self.level,
                        mode: controls.mode,
                        include_visible,
                        visible_rooms: &self.history,
                        rooms: &self.rooms,
                        selected: selected.as_ref(),
                    })
                    .map_err(anyhow::Error::msg)?;
                self.pending = pending;
                self.stage = if self.pending.is_none() {
                    Stage::Complete
                } else if self.debug_hidden != 0 {
                    Stage::Finish
                } else {
                    Stage::Cursor
                };
            }
            Stage::Project => {
                let p = self.point.unwrap();
                if p[0] < 0 || p[1] < 0 || p[0] >= self.dimensions[0] || p[1] >= self.dimensions[1]
                {
                    self.stage = Stage::Finish;
                } else {
                    let lot = simulation
                        .architecture
                        .as_ref()
                        .context("composition architecture absent")?
                        .original
                        .as_ref()
                        .context("composition terrain absent")?;
                    let floor = lot
                        .floor(self.level)
                        .context("composition terrain floor absent")?;
                    let splits = room_masks::SplitRooms::new();
                    let projection = room_masks::View {
                        size: lot.size,
                        rotation: self.rotation,
                        bounds: self.bounds,
                        camera: &self.camera,
                        heights: &lot.heights,
                        walls: &floor.source.walls,
                        splits: &splits,
                    };
                    // The cursor interface's slot44 is the retained no-op. Native
                    // screen shorts are vertical-first, after the zoom offset.
                    let screen = [
                        p[1].wrapping_add(
                            8i32.wrapping_shl(u32::from(self.camera.zoom))
                                .wrapping_mul(4),
                        ) as i16,
                        p[0] as i16,
                    ];
                    let fine = projection
                        .fine_point(screen, self.level)
                        .map_err(anyhow::Error::msg)?;
                    let tile = [(fine[1] >> 4) as i8, (fine[0] >> 4) as i8, self.level as i8];
                    let outdoors = self
                        .history
                        .iter()
                        .any(|id| self.rooms.get(id).is_some_and(|r| r.outdoors));
                    let end = self
                        .pending
                        .as_ref()
                        .unwrap()
                        .cursor(&mut self.visibility, tile, outdoors)
                        .map_err(anyhow::Error::msg)?;
                    self.projected = Some(fine);
                    self.cursor_end = Some(end);
                    self.stage = Stage::Finish;
                }
            }
            Stage::Finish => {
                self.pending
                    .take()
                    .unwrap()
                    .finish(&mut self.visibility)
                    .map_err(anyhow::Error::msg)?;
                self.stage = Stage::Complete;
            }
            _ => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn reply_point(&mut self, request: Request, result: u32, point: [i32; 2]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.pending() == Some(request),
            "cursor reply does not own composition"
        );
        ensure!(
            result != 0,
            "failed cursor output needs its retained prior scratch words"
        );
        ensure!(
            request != Request::Project,
            "projection is not an OS point reply"
        );
        self.point = Some(point);
        self.cursor_calls += 1;
        self.stage = match request {
            Request::GetCursorPos => Stage::Client,
            Request::ScreenToClient { .. } => Stage::Project,
            Request::Project => anyhow::bail!("projection is not an OS point reply"),
        };
        self.validate()
    }
}

impl Owner {
    pub(in crate::original_session::draw_parent) fn compose(
        &mut self,
        view: &mut View,
        simulation: &sims_sim::Simulation,
        controls: &sims_vm::interface::state::State,
    ) -> Result<bool> {
        let Some(super::Request::Compose { include_visible }) = self.pending() else {
            anyhow::bail!("World composition is not pending")
        };
        ensure!(
            self.owner == view.owner,
            "World composition owner alias differs"
        );
        let advanced = view.advance(simulation, controls, include_visible)?;
        if view.complete() {
            self.finish_composition()?;
        }
        Ok(advanced)
    }
    pub(in crate::original_session::draw_parent) fn finish_composition(&mut self) -> Result<()> {
        ensure!(
            matches!(self.pending(), Some(super::Request::Compose { .. })),
            "World composition lifetime differs"
        );
        self.phase = Phase::Prepare;
        self.validate()
    }
}
impl View {
    /// Verification only: never publish an expected observation into the owner.
    pub fn compare_native(
        &self,
        encoded: &[u8],
        pc: u32,
        simulation: &sims_sim::Simulation,
        controls: &sims_vm::interface::state::State,
    ) -> Result<()> {
        use std::io::Read;
        ensure!(
            sims_fixtures::digest(encoded)
                == "ff32d8e212835b5a894656d7e68d18bd6bb0f68ef1125a268c8281046288d049",
            "composition witness differs"
        );
        let mut raw = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut raw)?;
        let rows = raw
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<serde_json::Value>, _>>()?;
        let row = rows
            .iter()
            .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == pc)
            .context("composition native cut absent")?;
        let bytes = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("composition bytes absent")?,
            )
        };
        if let Some(c) = row.get("composition_inputs") {
            let b = bytes(&c["raw"])?;
            ensure!(
                c["view"] == self.owner && row["tick"] == simulation.vm.tick,
                "composition native identity differs"
            );
            ensure!(
                self.visibility
                    == visibility::State {
                        enabled: b[0x49],
                        dirty: b[0x4e],
                        include_visible: b[0x1f8],
                        mask: mask(&b[0x208..0x408])
                    },
                "composition native mask/flags differ at {pc:x}"
            );
            ensure!(
                word(&bytes(&c["mode_raw"])?, 8) as i32 == controls.mode
                    && self.debug_hidden == bytes(&c["debug_child_raw"])?[0x93]
                    && self.tool_active == bytes(&c["interface_body"])?[0x28],
                "composition native mode/view/tool differs"
            );
            ensure!(
                word(&bytes(&c["interface_body"])?, 0) == 0x667f1c
                    && c["step_vectors"] == "ffff00010100",
                "composition interface/steps differ"
            );
            let lr = bytes(&c["lot_raw"])?;
            ensure!(
                word(&lr, 0x10) as i32 == self.size
                    && word(&lr, 0x84) as u8 == self.rotation
                    && self.bounds == std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
                "composition native bounds differ"
            );
            ensure!(
                self.level == word(&b, 0x18) as i32
                    && self.camera.tile_width == word(&b, 0x58) as i32
                    && self.camera.half_width == word(&b, 0x5c) as i32
                    && self.camera.bottom == word(&b, 0x60) as i32
                    && self.camera.floor_heights
                        == (0..4)
                            .map(|i| word(&b, 0x130 - 4 * i) as i32)
                            .collect::<Vec<_>>(),
                "composition native camera differs"
            );
            let h = bytes(&c["history_used"])?;
            ensure!(
                self.history
                    == h.chunks_exact(4)
                        .map(|b| word(b, 0) as u16)
                        .collect::<Vec<_>>(),
                "composition native history differs"
            );
            let rooms = c["rooms"].as_array().context("native rooms absent")?;
            ensure!(
                rooms.len() == self.rooms.len(),
                "composition native room count differs"
            );
            for r in rooms {
                let raw = bytes(&r["raw"])?;
                let id = u16::from_le_bytes(raw[..2].try_into()?);
                ensure!(
                    self.rooms.get(&id)
                        == Some(&visibility::Room {
                            outdoors: raw[0x72] != 0,
                            mask: mask(&raw[0x80..0x280])
                        }),
                    "composition native room mask differs"
                );
            }
            let people = c["people"].as_array().context("native people absent")?;
            let mut ids = vec![];
            for p in people {
                let b = bytes(&p["raw"])?;
                let id = i16::from_le_bytes(b[..2].try_into()?);
                ids.push(id);
                ensure!(
                    simulation.vm.world.objects[&id].native_flags == Some(word(&b, 36)),
                    "composition person flags differ"
                );
                let pos = &simulation.vm.world.positions[&id];
                ensure!(
                    [(pos.y >> 4) as i8, (pos.x >> 4) as i8, pos.level as i8]
                        == [b[12] as i8, b[13] as i8, b[14] as i8],
                    "composition person position differs"
                );
                ensure!(
                    simulation
                        .native_lot
                        .as_ref()
                        .context("live lot absent")?
                        .membership
                        .0
                        .get(&id)
                        == Some(&i16::from_le_bytes(bytes(&p["room"])?[..2].try_into()?)),
                    "composition cached person room differs"
                );
            }
            ensure!(
                simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .context("live person registry absent")?
                    .person_order
                    == ids,
                "composition person order differs"
            );
        }
        if let Some(c) = row.get("cursor_geometry") {
            let raw = bytes(&c["raw"])?;
            if pc == 0x4abd68 {
                ensure!(
                    self.projected == Some([word(&raw, 8) as i32, word(&raw, 12) as i32]),
                    "composition native projected point differs"
                );
            }
            if pc == 0x4abe87 {
                ensure!(
                    self.cursor_end == Some([raw[0x3c] as i8, raw[0x3d] as i8, raw[0x3e] as i8]),
                    "composition native cursor end differs"
                );
            }
        }
        Ok(())
    }
}

impl View {
    pub(super) fn scene_level(&self)->i32{self.level}
    pub(super) fn scene_rotation(&self)->u8{self.rotation}
    pub(super) fn scene_rectangle(&self,simulation:&sims_sim::Simulation,tile:[i8;3],viewport:[i32;4])->Result<Option<[i32;4]>> {
        let lot=simulation.architecture.as_ref().and_then(|a|a.original.as_ref()).context("scene projection terrain absent")?;
        let floor=lot.floor(self.level).context("scene projection floor absent")?;let splits=room_masks::SplitRooms::new();
        room_masks::View{size:lot.size,rotation:self.rotation,bounds:self.bounds,camera:&self.camera,heights:&lot.heights,walls:&floor.source.walls,splits:&splits}.invalidation_rectangle(tile,self.level,viewport).map_err(anyhow::Error::msg)
    }
}
