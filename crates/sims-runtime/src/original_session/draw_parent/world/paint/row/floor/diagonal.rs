//!4af4e7/4afae4 diagonal halves: ordered covering, per-room descriptor/light,
//! temporary vertical/horizontal clips, resident draws and caller restoration.
use super::*;
use sims_sim::lot_presenter::{floor_prep, terrain_raster::Data};
use sims_vm::scheduler::clock::draw::context::{Frame, LightUpdate};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    stage: u8,
    #[serde(default = "ascending", skip_serializing_if = "is_ascending")]
    orientation: u8,
    coverings: [u16; 2],
    half: usize,
    rooms: super::super::split::Inputs,
    descriptor: [u8; 4],
    selected: Option<Selection>,
    origin: [i32; 2],
    original_clip: [i16; 4],
    clip: [i16; 4],
    original_light: u32,
    light: Option<LightUpdate>,
    draw: Option<sprite::Cursor>,
}
fn ascending() -> u8 {
    32
}
fn is_ascending(v: &u8) -> bool {
    *v == 32
}
impl Cursor {
    pub fn new(
        parent: &super::Cursor,
        sim: &sims_sim::Simulation,
        rooms: &super::super::split::Source,
        source: &Source,
        clip: [i16; 4],
    ) -> Result<Self> {
        ensure!(
            parent.rotation == 0 && matches!(parent.wall_reply, Some(16 | 32)),
            "different diagonal floor orientation remains pending"
        );
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("diagonal lot absent")?;
        let floor = lot
            .floor(i32::from(parent.point[2]))
            .context("diagonal level absent")?;
        let p = sims_sim::lot_presenter::terrain_shade::inverse(parent.point, parent.rotation)
            .map_err(anyhow::Error::msg)?;
        let wall = sims_sim::lot_walls::Grid::new(lot.size, &floor.source.walls)
            .map_err(anyhow::Error::msg)?
            .expanded(p[0] as usize, p[1] as usize)
            .map_err(anyhow::Error::msg)?;
        let plan = floor_prep::diagonal_coverings(
            wall.segments,
            wall.styles[0].context("diagonal style owner absent")?,
            wall.patterns[0],
            parent.covering,
        )
        .context("diagonal requires ordinary covering branch")?;
        ensure!(
            plan.orientation == 32 || source.height_offset.is_some(),
            "descending diagonal view height is unobserved"
        );
        let physical = [
            (i32::from(parent.point[1]) << 4) | 8,
            (i32::from(parent.point[0]) << 4) | 8,
        ];
        let rooms = rooms.inputs(sim, physical, parent.rotation)?;
        ensure!(
            if plan.orientation == 16 {
                matches!(rooms.directions[0], 0 | 2 | 4)
            } else {
                matches!(rooms.directions[0], 0 | 1 | 3)
            },
            "diagonal room direction differs"
        );
        let s = Self {
            stage: 0,
            orientation: plan.orientation,
            coverings: [plan.first, plan.second],
            half: 0,
            rooms,
            descriptor: parent.descriptor.context("diagonal descriptor absent")?,
            selected: None,
            origin: [
                parent.screen[0].wrapping_sub(source.origin_offset),
                parent.screen[1]
                    .wrapping_sub(i32::from(parent.corners[1]).wrapping_shl(parent.zoom + 1)),
            ],
            original_clip: clip,
            clip,
            original_light: parent.light.context("diagonal caller light absent")?,
            light: None,
            draw: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.stage <= 9 && self.half < 2 && matches!(self.orientation, 16 | 32),
            "diagonal floor cursor differs"
        );
        if let Some(d) = &self.draw {
            d.validate()?;
            ensure!(self.stage == 5, "diagonal draw lost caller");
        }
        if let Some(l) = &self.light {
            l.validate().map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == 9
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if self.stage == 5 {
            return self.draw.as_ref().is_some_and(|d| d.at_native_cut(pc));
        }
        (pc == 0x4671f8 && self.stage == 0)
            || (pc == 0x4c6383 && self.stage == 3)
            || (pc == 0x4c811f && self.stage == 4)
    }
    fn room_index(&self) -> usize {
        let a = self.rooms.directions[0];
        if self.orientation == 16 {
            return usize::from(a != if self.half == 0 { 2 } else { 4 });
        }
        if self.half == 0 {
            usize::from(a != 1)
        } else {
            usize::from(a != 3)
        }
    }
    fn prepare(&mut self, parent: &super::Cursor, sim: &sims_sim::Simulation) -> Result<()> {
        self.descriptor = parent.descriptor.unwrap();
        let filter_index = if self.orientation == 16 {
            usize::from(!matches!(
                (self.half, self.rooms.directions[0]),
                (0, 0 | 2) | (1, 0 | 4)
            ))
        } else if self.half == 0 {
            usize::from(!matches!(self.rooms.directions[0], 0 | 1))
        } else {
            usize::from(!matches!(self.rooms.directions[0], 0 | 3))
        };
        if self.rooms.ids[0] != self.rooms.ids[1] && self.rooms.rooms[filter_index].is_some() {
            let lot = sim
                .architecture
                .as_ref()
                .and_then(|a| a.original.as_ref())
                .context("diagonal descriptor lot absent")?;
            let floor = lot
                .floor(i32::from(parent.point[2]))
                .context("diagonal descriptor floor absent")?;
            for (n, neighbor) in floor_prep::neighbors(parent.descriptor.unwrap(), parent.point)
                .into_iter()
                .enumerate()
            {
                let p = sims_sim::lot_presenter::terrain_shade::inverse(neighbor, parent.rotation)
                    .map_err(anyhow::Error::msg)?;
                ensure!(
                    p[..2].iter().all(|&v| v >= 0 && (v as usize) < lot.size),
                    "diagonal neighbor room outside retained map"
                );
                let id = floor.rooms.primary[p[0] as usize * lot.size + p[1] as usize] as u16;
                if id != self.rooms.ids[filter_index] {
                    self.descriptor = floor_prep::remove(self.descriptor, n as i32);
                }
            }
        }
        // Only a split primary map entry invokes the per-room light getter.
        // Ordinary entries retain the already established caller context.
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("diagonal light lot absent")?;
        let floor = lot
            .floor(i32::from(parent.point[2]))
            .context("diagonal light floor absent")?;
        let p = sims_sim::lot_presenter::terrain_shade::inverse(parent.point, parent.rotation)
            .map_err(anyhow::Error::msg)?;
        if floor.rooms.primary[p[0] as usize * lot.size + p[1] as usize] != -5 {
            self.light = None;
            return Ok(());
        }
        let light = self.rooms.rooms[self.room_index()]
            .map(|r| {
                let v = f32::from_bits(r.light);
                if r.pool || (r.occupied && r.lights == 0 && r.people > 0 && !(v >= 0.8)) {
                    0.8f32.to_bits()
                } else {
                    r.light
                }
            })
            .unwrap_or(1f32.to_bits());
        self.light = Some(LightUpdate::new(light));
        Ok(())
    }
    pub fn advance(
        &mut self,
        parent: &super::Cursor,
        source: &Source,
        placement: &placement::Source,
        sim: &sims_sim::Simulation,
        data: &mut Data,
        context: &mut Frame,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            0 => self.stage = 1,
            1 => {
                if self.coverings[self.half] == 0 {
                    self.stage = 6;
                } else {
                    self.prepare(parent, sim)?;
                    self.stage = 2;
                }
            }
            2 => {
                if let Some(light) = self.light.as_mut() {
                    context.advance_light(light).map_err(anyhow::Error::msg)?;
                }
                if self.light.as_ref().is_none_or(|v| v.complete()) {
                    self.clip = floor_prep::diagonal_clip(
                        self.orientation,
                        self.half,
                        self.original_clip,
                        parent.screen,
                        parent.corners[1],
                        parent.zoom,
                        source.height_offset,
                    )
                    .map_err(anyhow::Error::msg)?;
                    self.stage = 3;
                }
            }
            3 => {
                self.selected = Some(
                    match floor_prep::lookup(
                        &source.entries,
                        u32::from(self.coverings[self.half]),
                        parent.zoom,
                    )
                    .map_err(anyhow::Error::msg)?
                    {
                        Some((image, palette)) => Selection::Found { image, palette },
                        None => Selection::Missing,
                    },
                );
                self.stage = 4;
            }
            4 => {
                if matches!(self.selected, Some(Selection::Found { .. })) {
                    self.draw = Some(sprite::Cursor::new(
                        parent.point,
                        self.origin,
                        parent.rotation,
                    ));
                    self.stage = 5;
                } else {
                    self.stage = 6;
                }
            }
            5 => {
                let Some(Selection::Found { image, palette }) = self.selected else {
                    anyhow::bail!("diagonal selected resource absent")
                };
                let sprite = source
                    .residents
                    .get(&image)
                    .context("diagonal resident sprite absent")?;
                ensure!(
                    sprite.palette_owner == palette,
                    "diagonal palette owner differs"
                );
                let d = self.draw.as_mut().unwrap();
                if d.complete() {
                    self.draw = None;
                    self.selected = None;
                    self.clip = self.original_clip;
                    self.stage = 6;
                } else {
                    if d.dispatch_pending() {
                        if let Some(g) = &source.gradient {
                            let shadow = source.shadow.as_ref().context("diagonal shadow owner absent")?;
                            if let Some(progress) = d.advance_lighting(sprite, g, self.descriptor,
                                shadow.radii[parent.zoom as usize], parent.zoom as u8,
                                floor_prep::brightness(context.state().words()), data)? {
                                return Ok(progress);
                            }
                        }
                    }
                    if d.dispatch_pending() && self.descriptor != [127; 4] {
                        let shadow = source
                            .shadow
                            .as_ref()
                            .context("diagonal shadow owner absent")?;
                        let count = if shadow.enabled {
                            sims_sim::lot_presenter::floor_light::classify(
                                self.descriptor.map(|v| v as i8),
                                shadow.radii[parent.zoom as usize],
                                [0; 4],
                            )
                            .0
                        } else {
                            0
                        };
                        ensure!(
                            count == 0
                                || floor_prep::brightness(context.state().words())
                                    == 1f32.to_bits(),
                            "diagonal sprite gradient remains pending"
                        );
                    }
                    d.advance(
                        sprite,
                        placement,
                        sim,
                        data,
                        context.state().words(),
                        self.clip,
                    )?;
                }
            }
            6 => {
                if self.half == 0 {
                    self.half = 1;
                    self.light = None;
                    self.stage = 1;
                } else {
                    self.light = Some(LightUpdate::new(self.original_light));
                    self.stage = 7;
                }
            }
            7 => {
                context
                    .advance_light(self.light.as_mut().unwrap())
                    .map_err(anyhow::Error::msg)?;
                if self.light.as_ref().unwrap().complete() {
                    self.stage = 8;
                }
            }
            8 => self.stage = 9,
            _ => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn compare(
        &self,
        parent: &super::Cursor,
        source: &Source,
        row: &serde_json::Value,
        data: &Data,
    ) -> Result<()> {
        self.validate()?;
        let pc = row["pc"].as_u64().context("diagonal PC absent")? as u32;
        let bytes = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("diagonal observation bytes absent")?,
            )
        };
        if self.stage == 5 {
            let Some(Selection::Found { image, .. }) = self.selected else {
                unreachable!()
            };
            if row["gradient"].is_object() {
                return self.draw.as_ref().unwrap().compare_lighting(
                    source.gradient.as_ref().context("diagonal gradient source absent")?,
                    source.residents.get(&image).context("diagonal gradient sprite absent")?,row,data);
            }
            return self.draw.as_ref().unwrap().compare(
                source.residents.get(&image).unwrap(),
                row,
                data,
            );
        }
        let stack = bytes(&row["stack"]["raw"])?;
        if pc == 0x4671f8 {
            ensure!(
                word(&stack, 0)
                    == if self.orientation == 16 {
                        0x4af564
                    } else {
                        0x4afb66
                    },
                "diagonal room-pair caller differs"
            );
        }
        if pc == 0x4c6383 {
            ensure!(
                word(&stack, 4) == u32::from(self.coverings[self.half])
                    && word(&stack, 8) == parent.zoom
                    && row["registers"]["ecx"] == source.catalog,
                "diagonal catalog call differs"
            );
        }
        if pc == 0x4c811f {
            let Some(Selection::Found { image, palette }) = self.selected else {
                anyhow::bail!("diagonal sprite not selected")
            };
            let raw = bytes(&row["floor_frame"]["raw"])?;
            let bp = row["floor_frame"]["bp"].as_u64().unwrap() as u32;
            let (out, desc, return_pc) = match (self.orientation, self.half) {
                (16, 0) => (0x1d4, 0x40, 0x4af78a),
                (16, 1) => (0x1c4, 0x30, 0x4af9a6),
                (32, 0) => (0x174, 0x34, 0x4afd7e),
                (32, 1) => (0x1e4, 0x38, 0x4aff77),
                _ => unreachable!(),
            };
            ensure!(
                word(&stack, 0) == return_pc
                    && word(&stack, 4) == bp - out
                    && word(&stack, 8) == bp - 0x10
                    && word(&stack, 12) == image
                    && word(&stack, 16) == 0
                    && [word(&stack, 20) as i32, word(&stack, 24) as i32] == self.origin
                    && word(&stack, 36) == bp - desc
                    && raw[0x224 - desc as usize..0x228 - desc as usize] == self.descriptor
                    && source
                        .residents
                        .get(&image)
                        .is_some_and(|s| s.palette_owner == palette),
                "diagonal sprite caller differs"
            );
            ensure!(
                bytes(&row["clip"])?
                    == self
                        .clip
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                "diagonal half clip differs"
            );
        }
        Ok(())
    }
}
