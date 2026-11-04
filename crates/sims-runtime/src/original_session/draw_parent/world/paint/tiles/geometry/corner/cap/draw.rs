//! Visible53c890..53ca46 caller. B supplies identities; current Scene supplies
//! segment style and room assignments. No C8 call or return supplies a reply.
use super::*;
use crate::original_session::draw_parent::world::paint::row::{
    GeometryMaterial, GeometryRasterJob, GeometryRasterParams,
};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Room {
    id: u16,
    pointer: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Edge {
    pointer: u32,
    kind: u32,
    room: Option<Room>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Input {
    style: u8,
    edges: [Option<Edge>; 4],
}
impl Input {
    pub(super) fn import(
        memory: &Memory,
        source: &serde_json::Value,
        graph: &Graph,
        id: usize,
        style: u8,
    ) -> Result<Self> {
        let mut edges = std::array::from_fn(|_| None);
        for (slot, edge_id) in graph.state.corners[id].edges.iter().enumerate() {
            let Some(edge_id) = edge_id else { continue };
            let edge = &graph.state.edges[*edge_id];
            let n = graph.state.floors[0]
                .edges
                .iter()
                .position(|v| v == edge_id)
                .context("cap neighbor B identity absent")?;
            let entry = &source["first_vector"]["models"][n];
            let pointer = entry["pointer"]
                .as_u64()
                .context("cap neighbor pointer absent")? as u32;
            let raw = memory.read(pointer, 0x100)?;
            ensure!(
                decode(&entry["raw"])? == raw && word(&raw, 0x10) == edge.kind as u32,
                "cap neighbor B orientation differs"
            );
            let selected = word(&raw, 0x18) as usize;
            ensure!(selected < 2, "cap neighbor B side unsupported");
            let room = graph.geometry.rooms[*edge_id][selected]
                .map(|room_id| -> Result<Room> {
                    let v = source["setup"]["room_directory"]
                        .as_array()
                        .context("cap room directory absent")?
                        .iter()
                        .find(|r| r["id"] == room_id)
                        .context("cap current room native identity absent")?;
                    Ok(Room {
                        id: room_id,
                        pointer: v["room"].as_u64().context("cap room pointer absent")? as u32,
                    })
                })
                .transpose()?;
            edges[slot] = Some(Edge {
                pointer,
                kind: edge.kind as u32,
                room,
            });
        }
        Ok(Self { style, edges })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Material,
    MaterialReturned,
    Rotate,
    Rotated,
    Center,
    Centered,
    Getter,
    GetterReturned,
    Place,
    Placing,
    Placed,
    Neighbor,
    NeighborReturned,
    Light,
    LightReturned,
    LightStore,
    Lighting,
    LightStored,
    PostLight,
    PointCopy,
    PointCopied,
    Descriptor,
    DescriptorReturned,
    Palette,
    PaletteReturned,
    Call,
    Raster,
    Returned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct State {
    input: Input,
    stage: Stage,
    model: u32,
    view: u32,
    point: [u8; 3],
    rotation: u8,
    zoom: u8,
    height: u8,
    level: u8,
    mask: u32,
    side: u32,
    pattern: u16,
    rectangle: [i32; 4],
    clip: [i32; 4],
    direction: usize,
    material: Option<GeometryMaterial>,
    coverage: Option<u32>,
    index: Option<u32>,
    rotated: Option<[u8; 3]>,
    physical: Option<[i32; 2]>,
    placement: Option<PlacementUpdate>,
    cursor: usize,
    orientation: Option<u32>,
    room: Option<Room>,
    selected_edge: Option<u32>,
    light: Option<u32>,
    light_update: Option<LightUpdate>,
    descriptor: Option<[u8; 4]>,
    raster: Option<GeometryRasterJob>,
}
impl State {
    pub(super) fn new(
        input: Input,
        parent: &super::Input,
        point: [u8; 3],
        rotation: u8,
        zoom: u8,
        height: u8,
        side: u32,
        rectangle: [i32; 4],
        clip: [i32; 4],
        view: u32,
        level: u8,
    ) -> Result<Self> {
        let direction = word(&parent.descriptor, 8) as usize;
        ensure!(
            input
                .edges
                .get(direction)
                .and_then(|v| v.as_ref())
                .is_some(),
            "cap primary room edge absent"
        );
        Ok(Self {
            input,
            stage: Stage::Material,
            model: parent.model,
            view,
            point,
            rotation,
            zoom,
            height,
            level,
            mask: word(&parent.descriptor, 12),
            side,
            pattern: parent.pattern,
            rectangle,
            clip,
            direction,
            material: None,
            coverage: None,
            index: None,
            rotated: None,
            physical: None,
            placement: None,
            cursor: 0,
            orientation: None,
            room: None,
            selected_edge: None,
            light: None,
            light_update: None,
            descriptor: None,
            raster: None,
        })
    }
    pub(super) fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.rotation < 4
                && (1..=3).contains(&self.zoom)
                && self.direction < 4
                && self.cursor <= 4,
            "visible cap retained caller invalid"
        );
        let n = self.stage as u8;
        for (present, from, name) in [
            (self.material.is_some(), Stage::MaterialReturned, "material"),
            (self.coverage.is_some(), Stage::Rotate, "coverage"),
            (self.index.is_some(), Stage::Rotate, "frame"),
            (self.rotated.is_some(), Stage::Rotated, "rotation"),
            (self.physical.is_some(), Stage::Centered, "center"),
            (self.placement.is_some(), Stage::Place, "placement"),
            (self.light.is_some(), Stage::LightReturned, "light"),
            (self.light_update.is_some(), Stage::Lighting, "light update"),
            (
                self.descriptor.is_some(),
                Stage::DescriptorReturned,
                "descriptor",
            ),
            (self.raster.is_some(), Stage::Raster, "raster"),
        ] {
            ensure!(
                present == (n >= from as u8),
                "visible cap {name} lifetime differs"
            );
        }
        if let Some(p) = &self.light_update {
            p.validate().map_err(anyhow::Error::msg)?;
        }
        if let Some(r) = &self.raster {
            r.validate()?;
        }
        Ok(())
    }
    fn select_room(&mut self) {
        self.room = self.input.edges[self.direction]
            .as_ref()
            .unwrap()
            .room
            .clone();
        self.cursor = 0;
        while self.cursor < 4 && self.input.edges[self.cursor].is_none() {
            self.cursor += 1;
        }
        self.stage = if self.cursor < 4 {
            Stage::Neighbor
        } else {
            Stage::Light
        };
    }
    fn params(&self) -> Result<GeometryRasterParams> {
        let m = self
            .material
            .as_ref()
            .context("cap material result absent")?;
        Ok(GeometryRasterParams {
            coverage_cache: m.default_palette,
            coverage_override: 0,
            coverage_image: self.coverage.context("cap coverage key absent")?,
            wallpaper_cache: m.palette,
            wallpaper_image: m.images[usize::from(self.zoom) - 1],
            index: self.index.context("cap frame index absent")?,
            point: [self.rectangle[0], self.rectangle[1]],
            offset: if self.mask == 2 {
                8i32.wrapping_shl(u32::from(self.zoom))
            } else {
                0
            },
            mask: self.mask,
            clip: self.clip,
            descriptor: self.descriptor.context("cap floor descriptor absent")?,
        })
    }
    pub(super) fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        context: &mut ContextFrame,
        row: &mut Row,
    ) -> Result<bool> {
        self.validate()?;
        if matches!(self.stage, Stage::Placing | Stage::Placed) {
            self.placement
                .as_ref()
                .context("cap placement continuation absent")?
                .validate_words(context.state().words())
                .map_err(anyhow::Error::msg)?;
        }
        match self.stage {
            Stage::Material => {
                self.material = Some(
                    row.lookup_geometry_material(u32::from(self.pattern))?
                        .context("cap material catalog is unobserved")?,
                );
                self.stage = Stage::MaterialReturned
            }
            Stage::MaterialReturned => {
                let style = if self.input.style == 255 { 255 } else { 1 };
                self.coverage = Some(
                    ((u32::from(self.zoom) << 8) + style)
                        .wrapping_mul(2)
                        .wrapping_sub(512)
                        .wrapping_add(u32::from(self.side != 0)),
                );
                self.index = Some(match self.mask {
                    1 => 0,
                    2 => 1,
                    16 => 2,
                    32 => 3,
                    _ => anyhow::bail!("cap unknown C4 orientation"),
                });
                self.stage = Stage::Rotate;
            }
            Stage::Rotate => {
                let [x, y, z] = self.point;
                self.rotated = Some(match 4u8.wrapping_sub(self.rotation) & 3 {
                    1 => [63u8.wrapping_sub(y), x, z],
                    2 => [63u8.wrapping_sub(x), 63u8.wrapping_sub(y), z],
                    3 => [y, 63u8.wrapping_sub(x), z],
                    _ => [x, y, z],
                });
                self.stage = Stage::Rotated
            }
            Stage::Rotated => self.stage = Stage::Center,
            Stage::Center => {
                let [x, y, _] = self.rotated.unwrap().map(|v| v as i8);
                self.physical = Some([(i32::from(y) << 4) | 8, (i32::from(x) << 4) | 8]);
                self.stage = Stage::Centered
            }
            Stage::Centered => self.stage = Stage::Getter,
            Stage::Getter => self.stage = Stage::GetterReturned,
            Stage::GetterReturned => {
                self.placement = Some(PlacementUpdate::new(
                    self.physical.unwrap(),
                    i32::from(self.level as i8),
                    i32::from(self.height),
                    self.rotation,
                ));
                self.stage = Stage::Place
            }
            Stage::Place => self.stage = Stage::Placing,
            Stage::Placing => {
                let p = self.placement.as_mut().unwrap();
                let progress = row.advance_geometry_placement(p, sim, context)?;
                ensure!(progress || p.complete(), "cap placement stalled");
                if p.complete() {
                    self.stage = Stage::Placed
                }
            }
            Stage::Placed => self.select_room(),
            Stage::Neighbor => {
                self.orientation = Some(level_geometry::orientation_mask(
                    self.input.edges[self.cursor]
                        .as_ref()
                        .context("cap neighbor disappeared")?
                        .kind,
                ));
                self.stage = Stage::NeighborReturned
            }
            Stage::NeighborReturned => {
                if self.orientation == Some(self.mask) {
                    let e = self.input.edges[self.cursor].as_ref().unwrap();
                    self.room = e.room.clone();
                    self.selected_edge = Some(e.pointer);
                    self.stage = Stage::Light
                } else {
                    self.cursor += 1;
                    while self.cursor < 4 && self.input.edges[self.cursor].is_none() {
                        self.cursor += 1;
                    }
                    self.stage = if self.cursor < 4 {
                        Stage::Neighbor
                    } else {
                        Stage::Light
                    };
                }
            }
            Stage::Light => {
                let r = self
                    .room
                    .as_ref()
                    .context("cap default light reset is pending")?;
                self.light = Some(
                    sim.room_environment
                        .as_ref()
                        .context("cap room environment absent")?
                        .manager
                        .rooms
                        .get(&r.id)
                        .context("cap current room absent")?
                        .effective_light()
                        .to_bits(),
                );
                self.stage = Stage::LightReturned
            }
            Stage::LightReturned => self.stage = Stage::LightStore,
            Stage::LightStore => {
                self.light_update = Some(LightUpdate::new(self.light.unwrap()));
                self.stage = Stage::Lighting
            }
            Stage::Lighting => {
                let u = self.light_update.as_mut().unwrap();
                context.advance_light(u).map_err(anyhow::Error::msg)?;
                if u.complete() {
                    self.stage = Stage::LightStored
                }
            }
            Stage::LightStored => self.stage = Stage::PostLight,
            Stage::PostLight => self.stage = Stage::PointCopy,
            Stage::PointCopy => self.stage = Stage::PointCopied,
            Stage::PointCopied => self.stage = Stage::Descriptor,
            Stage::Descriptor => {
                self.descriptor =
                    Some(row.geometry_descriptor(self.point.map(|v| v as i8), self.rotation)?);
                self.stage = Stage::DescriptorReturned
            }
            Stage::DescriptorReturned => self.stage = Stage::Palette,
            Stage::Palette => self.stage = Stage::PaletteReturned,
            Stage::PaletteReturned => self.stage = Stage::Call,
            Stage::Call => {
                self.raster = Some(GeometryRasterJob::new(self.params()?)?);
                self.stage = Stage::Raster
            }
            Stage::Raster => {
                let r = self.raster.as_mut().unwrap();
                let progress = row.advance_geometry_raster(r, context)?;
                ensure!(progress || r.complete(), "cap raster stalled");
                if r.complete() {
                    self.stage = Stage::Returned
                }
            }
            Stage::Returned => self.stage = Stage::Complete,
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub(super) fn at_cut(&self, pc: u32) -> bool {
        matches!(
            (self.stage, pc),
            (Stage::Material, 0x53c890)
                | (Stage::MaterialReturned, 0x53c895)
                | (Stage::Rotate, 0x53c951)
                | (Stage::Rotated, 0x53c956)
                | (Stage::Center, 0x53c96a)
                | (Stage::Centered, 0x53c96f)
                | (Stage::Getter, 0x53c973)
                | (Stage::GetterReturned, 0x53c978)
                | (Stage::Place, 0x53c97a)
                | (Stage::Placed, 0x53c97f)
                | (Stage::Neighbor, 0x53c9a0)
                | (Stage::NeighborReturned, 0x53c9a5)
                | (Stage::Light, 0x53c9d3)
                | (Stage::LightReturned, 0x53c9d8)
                | (Stage::LightStore, 0x53c9df)
                | (Stage::LightStored, 0x53c9e4)
                | (Stage::PostLight, 0x53c9ee)
                | (Stage::PointCopy, 0x53c9fb)
                | (Stage::PointCopied, 0x53ca00)
                | (Stage::Descriptor, 0x53ca0a)
                | (Stage::DescriptorReturned, 0x53ca0f)
                | (Stage::Palette, 0x53ca3a)
                | (Stage::PaletteReturned, 0x53ca3f)
                | (Stage::Call, 0x53ca41)
                | (Stage::Returned, 0x53ca46)
        ) || self.stage == Stage::Raster
            && pc == 0x4c79ca
            && self.raster.as_ref().is_some_and(|r| r.at_cut(pc))
    }
    pub(super) fn compare(&self, q: &serde_json::Value, row: Option<&Row>) -> Result<()> {
        self.validate()?;
        let pc = q["pc"].as_u64().context("cap draw PC absent")? as u32;
        ensure!(
            self.at_cut(pc) || self.complete() && matches!(pc, 0x53ca46 | 0x53ca56),
            "cap draw stage differs"
        );
        let bp = q["corner_prefix"]["bp"]
            .as_u64()
            .context("cap draw frame absent")? as u32;
        let f = decode(&q["corner_prefix"]["frame"]["raw"])?;
        let s = decode(&q["corner_prefix"]["stack"]["raw"])?;
        let n = self.stage as u8;
        let context = q["context"]["pointer"]
            .as_u64()
            .context("cap context pointer absent")? as u32;
        let local = |off: usize| word(&f, 0xa0 - off);
        ensure!(
            q["corner_model"]["pointer"] == self.model,
            "visible cap model identity differs"
        );
        if let Some(m) = &self.material {
            ensure!(
                local(0x50) == m.palette && local(0x38) == m.images[usize::from(self.zoom) - 1],
                "cap material outputs differ"
            );
        }
        if n >= Stage::Rotate as u8 {
            ensure!(
                local(0x28) == self.coverage.unwrap()
                    && local(0x14) == self.index.unwrap() + 1
                    && f[0xa0 - 0x30..0xa0 - 0x2d] == self.rotated.unwrap_or(self.point),
                "cap coverage/frame/point differs"
            );
        }
        if n >= Stage::Centered as u8 {
            ensure!(
                local(0x40) as i32 == self.physical.unwrap()[0],
                "cap physical x differs"
            );
            if n < Stage::Neighbor as u8 {
                ensure!(
                    local(0x3c) as i32 == self.physical.unwrap()[1],
                    "cap physical y differs"
                );
            }
        }
        if n >= Stage::Neighbor as u8 {
            ensure!(
                local(0x2c) == self.cursor as u32,
                "cap neighbor cursor differs"
            );
        }
        if n >= Stage::Neighbor as u8 && n <= Stage::PostLight as u8 {
            ensure!(
                local(0x3c) == self.room.as_ref().map_or(0, |r| r.pointer),
                "cap current room alias differs"
            );
        }
        if let Some(d) = self.descriptor {
            ensure!(
                f[0xa0 - 0x3c..0xa0 - 0x38] == d,
                "cap descriptor output differs"
            );
        }
        if let Some(e) = self.selected_edge {
            if n >= Stage::Light as u8 {
                ensure!(
                    q["registers"]["edi"] == e,
                    "cap selected edge register differs"
                );
            }
        }
        let args = |expected: &[u32]| -> bool {
            (0..expected.len()).all(|i| word(&s, 4 * i) == expected[i])
        };
        match self.stage {
            Stage::Material => ensure!(
                q["registers"]["ecx"]
                    == row
                        .context("cap material comparison owner absent")?
                        .lookup_geometry_material(u32::from(self.pattern))?
                        .context("cap material comparison source absent")?
                        .pool
                    && args(&[
                        u32::from(self.pattern),
                        u32::from(self.zoom),
                        bp - 0x50,
                        bp - 0x38
                    ]),
                "cap material call differs"
            ),
            Stage::MaterialReturned => ensure!(
                q["registers"]["eax"].as_u64().unwrap() as u8 != 0,
                "cap material lookup bool differs"
            ),
            Stage::Rotate => ensure!(
                q["registers"]["ecx"] == bp - 0x30
                    && args(&[u32::from(4u8.wrapping_sub(self.rotation) & 3)]),
                "cap rotation ABI differs"
            ),
            Stage::Center => ensure!(
                q["registers"]["ecx"] == bp - 0x30
                    && args(&[
                        bp - 0x40,
                        self.level as i8 as i32 as u32,
                        context,
                        u32::from(self.height)
                    ]),
                "cap center ABI differs"
            ),
            Stage::Centered => ensure!(
                q["registers"]["eax"] == bp - 0x40,
                "cap center result differs"
            ),
            Stage::Getter | Stage::Place => ensure!(
                q["registers"]["ecx"] == context && args(&[bp - 0x40]),
                "cap placement ABI differs"
            ),
            Stage::GetterReturned => ensure!(
                q["registers"]["eax"] == context,
                "cap placement getter differs"
            ),
            Stage::Neighbor => ensure!(
                args(&[self.input.edges[self.cursor].as_ref().unwrap().kind]),
                "cap neighbor orientation argument differs"
            ),
            Stage::NeighborReturned => ensure!(
                q["registers"]["eax"] == self.orientation.unwrap(),
                "cap orientation result differs"
            ),
            Stage::Light => ensure!(
                q["registers"]["ecx"] == self.room.as_ref().unwrap().pointer,
                "cap light receiver differs"
            ),
            Stage::LightStore => ensure!(
                q["registers"]["ecx"] == context && args(&[self.light.unwrap()]),
                "cap light publication ABI differs"
            ),
            Stage::PointCopy => ensure!(
                args(&[bp - 0x18]),
                "cap descriptor point copy argument differs"
            ),
            Stage::PointCopied => {
                ensure!(s[..3] == self.point, "cap descriptor point copy differs")
            }
            Stage::Descriptor => ensure!(
                q["registers"]["ecx"]
                    == row
                        .context("cap scene comparison owner absent")?
                        .geometry_scene()?
                    && args(&[bp - 0x3c])
                    && s[4..7] == self.point,
                "cap descriptor query argument differs"
            ),
            Stage::DescriptorReturned => ensure!(
                q["registers"]["eax"] == bp - 0x3c,
                "cap descriptor result pointer differs"
            ),
            Stage::PaletteReturned => ensure!(
                q["registers"]["eax"] == self.material.as_ref().unwrap().default_palette,
                "cap palette getter result differs"
            ),
            Stage::Palette | Stage::Call => {
                let p = self.params()?;
                ensure!(
                    args(&[
                        bp - 0x78,
                        p.coverage_override,
                        p.coverage_image,
                        p.wallpaper_cache,
                        p.wallpaper_image,
                        p.index,
                        p.point[0] as u32,
                        p.point[1] as u32,
                        0,
                        context,
                        p.offset as u32,
                        p.mask,
                        bp - 0x3c,
                        0
                    ]),
                    "cap C4 caller stack differs"
                );
                if self.stage == Stage::Call {
                    ensure!(
                        q["registers"]["ecx"] == p.coverage_cache,
                        "cap C4 receiver differs"
                    );
                }
            }
            Stage::Raster => {
                let p = self.params()?;
                ensure!(
                    q["registers"]["ecx"] == p.coverage_cache
                        && args(&[
                            0x53ca46,
                            bp - 0x78,
                            p.coverage_override,
                            p.coverage_image,
                            p.wallpaper_cache,
                            p.wallpaper_image,
                            p.index,
                            p.point[0] as u32,
                            p.point[1] as u32,
                            0,
                            context,
                            p.offset as u32,
                            p.mask,
                            bp - 0x3c,
                            0
                        ]),
                    "cap C4 entry ABI differs"
                );
            }
            Stage::Returned | Stage::Complete => {
                let r = self.raster.as_ref().unwrap().rectangle()?;
                ensure!(
                    (0..4).all(|i| word(&f, 0xa0 - 0x78 + i * 4) as i32 == r[i]),
                    "cap C4 output rectangle differs"
                );
            }
            _ => (),
        }
        Ok(())
    }
}
