//! Retained floor preparation and sprite caller. Diagonal wall paths stay pending.
use super::*;
use std::collections::BTreeMap;
mod diagonal;
mod gradient;
mod sprite;
const SOURCE: &str = "d06ddae3eaf2d3b9b6c5f6300f89b2b577255b16fe625d162fed42f364e223e5";
const SHADOW: &str = "ae484ce169c5bd510a1e5eae4e06a630d2c82d81186498112d951a6c0a138f24";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Shadow {
    source: String,
    radii: [i32; 4],
    enabled: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    scene: u32,
    catalog: u32,
    catalog_raw: Vec<u8>,
    cells: [Vec<u8>; 2],
    entries: BTreeMap<u32, ([u32; 3], u32)>,
    origin_offset: i32,
    /// Not consumed by earlier floor proofs. Bind its actual B owner before
    /// a descending half uses it, preserving historical snapshot bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    height_offset: Option<i16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sprite: Option<sprite::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sprite2: Option<sprite::Source>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    residents: BTreeMap<u32, sprite::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shadow: Option<Shadow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gradient: Option<gradient::Source>,
}
impl Source {
    pub(super) fn native_scene(&self)->u32{self.scene}
    pub(super) fn geometry_descriptor(&self,point:[i8;3],rotation:u8)->Result<[u8;4]>{self.validate()?;let level=usize::try_from(i32::from(point[2])-1).context("geometry descriptor floor invalid")?;sims_sim::lot_presenter::floor_prep::descriptor(self.cells.get(level).context("geometry descriptor floor absent")?,point,rotation).map_err(anyhow::Error::msg)}
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "floor source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "floor source epoch differs"
        );
        let m = Memory::new(&v)?;
        let scene = word(&m.read(0x6e5528, 4)?, 0);
        let sr = m.read(scene, 0x90)?;
        let cells = (0..2)
            .map(|i| {
                let h = word(&sr, 0x68 + 4 * i);
                m.read(word(&m.read(h, 4)?, 0), 64 * 64 * 4)
            })
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .unwrap();
        let catalog = word(&m.read(0x6efcb4, 4)?, 0);
        let raw = m.read(catalog, 0x40)?;
        let head = word(&raw, 0);
        let mut pending = vec![word(&m.read(head, 24)?, 4)];
        let mut seen = std::collections::BTreeSet::new();
        let mut entries = BTreeMap::new();
        let vector = m.read(
            word(&raw, 0x24),
            (word(&raw, 0x28) - word(&raw, 0x24)) as usize,
        )?;
        while let Some(p) = pending.pop() {
            if p == 0 || p == head {
                continue;
            }
            ensure!(
                seen.insert(p) && seen.len() <= 4096,
                "floor catalog tree differs"
            );
            let n = m.read(p, 24)?;
            let body = m.read(word(&n, 20), 20)?;
            let index = word(&body, 0) as usize;
            ensure!(
                index * 8 + 8 <= vector.len(),
                "floor palette index outside vector"
            );
            ensure!(
                entries
                    .insert(
                        word(&n, 16),
                        (
                            [word(&body, 4), word(&body, 8), word(&body, 12)],
                            word(&vector, index * 8 + 4)
                        )
                    )
                    .is_none(),
                "duplicate floor key"
            );
            pending.extend([word(&n, 8), word(&n, 12)]);
        }
        let s = Self {
            source: SOURCE.into(),
            scene,
            catalog,
            catalog_raw: raw,
            cells,
            entries,
            origin_offset: word(&m.read(0x09e88960, 0x80)?, 0x5c) as i32,
            height_offset: None,
            sprite: None,
            sprite2: None,
            residents: BTreeMap::new(),
            shadow: None,
            gradient: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(g) = &self.gradient {
            g.validate()?;
        }
        if let Some(s) = &self.sprite {
            s.validate()?;
        }
        if let Some(s) = &self.sprite2 {
            s.validate()?;
        }
        for (&id, s) in &self.residents {
            ensure!(id == s.image, "resident floor key differs");
            s.validate()?;
        }
        if let Some(s) = &self.shadow {
            ensure!(
                s.source == SHADOW && s.radii[1..].iter().all(|&r| (0..=127).contains(&r)),
                "floor shadow owner differs"
            );
        }
        ensure!(
            self.source == SOURCE
                && self.scene != 0
                && self.catalog != 0
                && self.catalog_raw.len() == 0x40
                && self.cells.iter().all(|v| v.len() == 16384)
                && self.entries.len() == 237,
            "floor input owner differs"
        );
        Ok(())
    }
    pub fn bind_sprite(&mut self, input: &[u8]) -> Result<()> {
        if sims_fixtures::digest(input) == SOURCE {
            ensure!(
                self.height_offset.is_none(),
                "floor view height already bound"
            );
            let v: serde_json::Value = serde_json::from_slice(input)?;
            let m = Memory::new(&v)?;
            let view = m.read(0x09e88960, 0x80)?;
            ensure!(
                word(&m.read(0x6e5528, 4)?, 0) == self.scene
                    && word(&m.read(0x6efcb4, 4)?, 0) == self.catalog
                    && word(&view, 0x5c) as i32 == self.origin_offset,
                "floor view height owner differs"
            );
            self.height_offset = Some(i16::from_le_bytes(view[0x64..0x66].try_into()?));
            return self.validate();
        }
        if sims_fixtures::digest(input) == gradient::SOURCE {
            ensure!(
                self.gradient.is_none(),
                "gradient floor source already bound"
            );
            self.gradient = Some(gradient::Source::import(input)?);
            return self.validate();
        }
        if sims_fixtures::digest(input) == SHADOW {
            ensure!(self.shadow.is_none(), "floor shadow source already bound");
            let v: serde_json::Value = serde_json::from_slice(input)?;
            ensure!(
                v["tick"] == 89073 && v["executes_target_code"] == false,
                "floor shadow epoch differs"
            );
            let m = Memory::new(&v)?;
            let owner = m.read(0x6f01a0, 0x70)?;
            let mut radii = [0; 4];
            for zoom in 1..=3 {
                let h = m.read(word(&owner, 0x50 + zoom * 4), 16)?;
                let bytes = word(&h, 4) as i32;
                let pitch = word(&h, 8) as i32;
                ensure!(bytes > 0 && pitch > 0, "floor shadow dimensions differ");
                radii[zoom] = (bytes / pitch - 1) / 2;
            }
            self.shadow = Some(Shadow {
                source: SHADOW.into(),
                radii,
                enabled: m.read(0x6f0180, 12)?[10] != 0,
            });
            return self.validate();
        }
        if sims_fixtures::digest(input)
            == "dacd8b0d097cc774835e28ef5335baca39241c5fd3c0ed6052ba3a86d221aedb"
        {
            ensure!(
                self.residents.is_empty(),
                "resident floor catalog already bound"
            );
            let imported = sprite::Source::import_catalog(input)?;
            for s in imported {
                ensure!(
                    self.residents.insert(s.image, s).is_none(),
                    "duplicate resident floor image"
                );
            }
            return self.validate();
        }
        let imported = sprite::Source::import(input)?;
        let slot = match imported.image {
            27 => &mut self.sprite,
            2 => &mut self.sprite2,
            _ => anyhow::bail!("floor sprite image unavailable"),
        };
        ensure!(slot.is_none(), "floor sprite source already bound");
        *slot = Some(imported);
        self.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Selection {
    Missing,
    Found { image: u32, palette: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    point: [i8; 3],
    screen: [i32; 2],
    corners: [u8; 4],
    rotation: u8,
    zoom: u32,
    covering: u16,
    walls: u8,
    stage: u8,
    descriptor: Option<[u8; 4]>,
    light: Option<u32>,
    selection: Option<Selection>,
    sprite: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draw: Option<sprite::Cursor>,
    /// Only the consumed first word of467690's expanded record is retained.
    /// Its other40 bytes remain unobserved until a diagonal branch consumes them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wall_reply: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    diagonal: Option<diagonal::Cursor>,
}
impl Cursor {
    pub fn new(
        point: [i8; 3],
        screen: [i32; 2],
        corners: [u8; 4],
        rotation: u8,
        zoom: u32,
        covering: u16,
        walls: u8,
    ) -> Self {
        Self {
            point,
            screen,
            corners,
            rotation,
            zoom,
            covering,
            walls,
            stage: 0,
            descriptor: None,
            light: None,
            selection: None,
            sprite: None,
            draw: None,
            wall_reply: None,
            diagonal: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(d) = &self.diagonal {
            d.validate()?;
            ensure!(self.stage == 13, "diagonal lost its floor caller");
        }
        if let Some(d) = &self.draw {
            ensure!(self.stage == 7, "sprite lost floor caller");
            d.validate()?;
        }
        ensure!(
            self.rotation <= 3
                && (1..=3).contains(&self.zoom)
                && (1..=2).contains(&self.point[2])
                && self.point[..2].iter().all(|v| (0..64).contains(v))
                && self.stage <= 13,
            "floor cursor differs"
        );
        ensure!(
            self.descriptor.is_some() == (self.stage >= 2)
                && self.light.is_some() == (self.stage >= 3)
                && self.selection.is_some() == matches!(self.stage, 6 | 7 | 9)
                && self.sprite.is_some() == (self.stage == 7),
            "floor suspended owners differ"
        );
        ensure!(
            !(10..=13).contains(&self.stage) || self.wall_reply.is_some(),
            "floor wall reply absent"
        );
        Ok(())
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if self.draw.as_ref().is_some_and(|d| d.at_native_cut(pc)) { return true; }
        if self.diagonal.as_ref().is_some_and(|d| d.at_native_cut(pc)) {
            return true;
        }
        if matches!(
            pc,
            0x4c811f
                | 0x4c8e0d
                | 0x4c8f52
                | 0x4c8152
                | 0x49f42d
                | 0x49f628
                | 0x49f949
                | 0x4b954a
                | 0x4c82c9
                | 0x4b0000
        ) {
            if let Some(d) = &self.draw {
                return d.at_native_cut(pc);
            }
        }
        match pc {
            0x4af453 => self.stage == 0,
            0x4aed23 => self.stage == 1,
            0x4af479 => self.stage == 2,
            0x4af485 => self.stage == 3,
            0x467690 => self.stage == 8,
            0x4af4da => self.stage == 10,
            0x4af4e7 => self.stage == 11,
            0x4afae4 => self.stage == 12,
            0x4aff8d => self.stage == 4,
            0x4c6383 => self.stage == 5,
            0x4affb1 => self.stage == 6,
            0x4c811f => self.stage == 7,
            _ => false,
        }
    }
    pub fn draw_bound(&self, source: &Source) -> bool {
        self.stage == 7 && self.sprite_source(source).is_some()
    }
    fn sprite_source<'a>(&self, source: &'a Source) -> Option<&'a sprite::Source> {
        if let Some(Selection::Found { image, .. }) = self.selection {
            if let Some(s) = source.residents.get(&image) {
                return Some(s);
            }
        }
        match self.selection {
            Some(Selection::Found { image: 27, .. }) => source.sprite.as_ref(),
            Some(Selection::Found { image: 2, .. }) => source.sprite2.as_ref(),
            _ => None,
        }
    }
    pub fn draw_complete(&self) -> bool {
        self.draw.as_ref().is_some_and(|d| d.complete())
            || self.diagonal.as_ref().is_some_and(|d| d.complete())
    }
    pub fn diagonal_pending(&self) -> bool {
        self.stage == 13
    }
    pub fn advance_diagonal(
        &mut self,
        source: &Source,
        rooms: &super::split::Source,
        placement: &placement::Source,
        sim: &sims_sim::Simulation,
        data: &mut sims_sim::lot_presenter::terrain_raster::Data,
        context: &mut sims_vm::scheduler::clock::draw::context::Frame,
        clip: [i16; 4],
    ) -> Result<bool> {
        if self.diagonal.is_none() {
            self.diagonal = Some(diagonal::Cursor::new(self, sim, rooms, source, clip)?);
            return Ok(true);
        }
        let mut d = self.diagonal.take().unwrap();
        let result = d.advance(self, source, placement, sim, data, context);
        self.diagonal = Some(d);
        result
    }
    pub fn advance_draw(
        &mut self,
        source: &Source,
        placement: &placement::Source,
        sim: &sims_sim::Simulation,
        data: &mut sims_sim::lot_presenter::terrain_raster::Data,
        context: &[u32; 29],
        clip: [i16; 4],
    ) -> Result<bool> {
        let s = self.sprite_source(source).context("sprite source absent")?;
        let Some(Selection::Found { image, palette }) = &self.selection else {
            anyhow::bail!("sprite selection absent")
        };
        ensure!(
            *image == s.image && *palette == s.palette_owner,
            "sprite branch needs different original resources/planes"
        );
        let descriptor = self
            .descriptor
            .context("floor lighting descriptor absent")?;
        let d = self.draw.get_or_insert_with(|| {
            sprite::Cursor::new(self.point, self.sprite.unwrap(), self.rotation)
        });
        if d.dispatch_pending() {
            if let Some(g) = &source.gradient {
                let shadow = source.shadow.as_ref().context("floor shadow radius owner absent")?;
                if let Some(progress) = d.advance_lighting(s, g, descriptor,
                    shadow.radii[self.zoom as usize], self.zoom as u8,
                    self.light.context("floor brightness absent")?, data)? {
                    return Ok(progress);
                }
            }
        }
        if descriptor != [127; 4] && d.dispatch_pending() {
            let shadow = source
                .shadow
                .as_ref()
                .context("floor shadow radius owner absent")?;
            let count = if shadow.enabled {
                sims_sim::lot_presenter::floor_light::classify(
                    descriptor.map(|v| v as i8),
                    shadow.radii[self.zoom as usize],
                    [0; 4],
                )
                .0
            } else {
                0
            };
            ensure!(
                count == 0 || self.light == Some(1f32.to_bits()),
                "sprite gradient lighting remains pending"
            );
        }
        d.advance(s, placement, sim, data, context, clip)
    }
    pub fn advance(
        &mut self,
        source: &Source,
        context: &[u32; 29],
        sim: &sims_sim::Simulation,
    ) -> Result<bool> {
        source.validate()?;
        self.validate()?;
        match self.stage {
            0 => self.stage = 1,
            1 => {
                self.descriptor = Some(
                    sims_sim::lot_presenter::floor_prep::descriptor(
                        &source.cells[self.point[2] as usize - 1],
                        self.point,
                        self.rotation,
                    )
                    .map_err(anyhow::Error::msg)?,
                );
                self.stage = 2;
            }
            2 => {
                self.light = Some(sims_sim::lot_presenter::floor_prep::brightness(context));
                self.stage = 3;
            }
            3 => self.stage = if self.walls == 0 { 4 } else { 8 },
            4 => self.stage = 5,
            5 => {
                self.selection = Some(
                    match sims_sim::lot_presenter::floor_prep::lookup(
                        &source.entries,
                        u32::from(self.covering),
                        self.zoom,
                    )
                    .map_err(anyhow::Error::msg)?
                    {
                        Some((image, palette)) => Selection::Found { image, palette },
                        None => Selection::Missing,
                    },
                );
                self.stage = 6;
            }
            6 => {
                if matches!(self.selection, Some(Selection::Found { .. })) {
                    self.sprite = Some([
                        self.screen[0].wrapping_sub(source.origin_offset),
                        self.screen[1]
                            .wrapping_sub(i32::from(self.corners[1]).wrapping_shl(self.zoom + 1)),
                    ]);
                    self.stage = 7;
                } else {
                    self.stage = 9;
                }
            }
            8 => {
                let lot = sim
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("floor wall live lot absent")?;
                let floor = lot
                    .floor(i32::from(self.point[2]))
                    .context("floor wall live level absent")?;
                self.wall_reply = Some(
                    sims_sim::lot_presenter::terrain_walls::segments(
                        lot.size,
                        &floor.source.walls,
                        self.point,
                        self.rotation,
                    )
                    .map_err(anyhow::Error::msg)?,
                );
                self.stage = 10;
            }
            10 => self.stage = 11,
            11 => {
                self.stage = if self.wall_reply.unwrap() & 16 != 0 {
                    13
                } else {
                    12
                }
            }
            12 => {
                self.stage = if self.wall_reply.unwrap() & 32 != 0 {
                    13
                } else {
                    4
                }
            }
            _ => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn matches_wall_call(&self, row: &serde_json::Value) -> Result<bool> {
        let stack = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("floor wall stack absent")?,
        )?;
        Ok(self.stage == 8
            && stack.len() >= 11
            && word(&stack, 0) == 0x4af4da
            && stack[8..11] == self.point.map(|v| v as u8))
    }
    pub fn compare(
        &self,
        source: &Source,
        row: &serde_json::Value,
        context: &[u32; 29],
        data: &sims_sim::lot_presenter::terrain_raster::Data,
    ) -> Result<()> {
        self.validate()?;
        source.validate()?;
        let pc = row["pc"].as_u64().context("floor observed PC absent")? as u32;
        if let Some(d) = &self.diagonal {
            if d.at_native_cut(pc) { return d.compare(self, source, row, data); }
        }
        if row["gradient"].is_object() {
            return self.draw.as_ref().context("gradient draw absent")?.compare_lighting(
                source.gradient.as_ref().context("gradient inputs absent")?,
                self.sprite_source(source).context("gradient sprite absent")?, row, data);
        }
        if matches!(
            pc,
            0x4c8e0d
                | 0x4c8f52
                | 0x4c8152
                | 0x49f42d
                | 0x49f628
                | 0x49f949
                | 0x4b954a
                | 0x4c82c9
                | 0x4b0000
        ) {
            return self.draw.as_ref().context("sprite cursor absent")?.compare(
                self.sprite_source(source).context("sprite inputs absent")?,
                row,
                data,
            );
        }
        ensure!(self.at_native_cut(pc), "floor observed cursor differs");
        let raw = crate::original_session::completed_animations::bytes(
            row["floor_frame"]["raw"]
                .as_str()
                .context("floor native frame absent")?,
        )?;
        ensure!(
            raw[0x214..0x217] == self.point.map(|v| v as u8),
            "floor native point differs"
        );
        if self.stage >= 2 {
            ensure!(
                raw[0x1e0..0x1e4] == self.descriptor.unwrap(),
                "floor native descriptor differs"
            );
        }
        if (4..=8).contains(&self.stage) || (10..=13).contains(&self.stage) {
            ensure!(
                word(&raw, 0x1c8) == self.light.unwrap(),
                "floor native brightness differs"
            );
        }
        let stack = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("floor native stack absent")?,
        )?;
        if pc == 0x467690 {
            let bp = row["floor_frame"]["bp"]
                .as_u64()
                .context("floor wall caller frame absent")? as u32;
            ensure!(
                self.matches_wall_call(row)?
                    && word(&stack, 4) == bp - 0x210
                    && row["registers"]["ecx"] == source.scene,
                "floor wall query arguments differ"
            );
        }
        if pc == 0x4af4da {
            let result = &row["floor_wall_result"];
            let r = crate::original_session::completed_animations::bytes(
                result["raw"].as_str().context("floor wall result absent")?,
            )?;
            let bp = row["floor_frame"]["bp"].as_u64().unwrap() as u32;
            ensure!(
                result["pointer"] == bp - 0x210
                    && word(&r, 0) == u32::from(self.wall_reply.unwrap()),
                "floor wall returned segment word differs"
            );
        }
        if matches!(pc, 0x4af4e7 | 0x4afae4) {
            let r = crate::original_session::completed_animations::bytes(
                row["floor_wall_copy"]
                    .as_str()
                    .context("floor wall copy absent")?,
            )?;
            ensure!(
                word(&r, 0) == u32::from(self.wall_reply.unwrap()),
                "floor wall copied segment word differs"
            );
        }
        if pc == 0x4aed23 {
            ensure!(
                stack[8..11] == self.point.map(|v| v as u8)
                    && row["registers"]["ecx"] == source.scene,
                "floor descriptor call differs"
            );
        }
        if pc == 0x4c6383 {
            ensure!(
                word(&stack, 4) == u32::from(self.covering)
                    && word(&stack, 8) == self.zoom
                    && row["registers"]["ecx"] == source.catalog,
                "floor lookup call differs"
            );
            let b = crate::original_session::completed_animations::bytes(
                row["floor_catalog"]["raw"]
                    .as_str()
                    .context("native floor catalog absent")?,
            )?;
            ensure!(b == source.catalog_raw, "floor catalog changed since B");
        }
        if pc == 0x4affb1 {
            match self.selection.as_ref().unwrap() {
                Selection::Found { image, palette } => ensure!(
                    row["registers"]["eax"].as_u64().unwrap() & 255 == 1
                        && row["floor_image_result"]["image"] == *image
                        && row["floor_image_result"]["palette"] == *palette,
                    "floor resident selection differs"
                ),
                Selection::Missing => ensure!(
                    row["registers"]["eax"].as_u64().unwrap() & 255 == 0,
                    "floor missing lookup differs"
                ),
            }
        }
        if pc == 0x4c811f {
            let Selection::Found { image, palette } = self.selection.as_ref().unwrap() else {
                unreachable!()
            };
            let bp = row["floor_frame"]["bp"].as_u64().unwrap() as u32;
            let sp = self.sprite.unwrap();
            ensure!(
                word(&stack, 4) == bp - 0x184
                    && word(&stack, 8) == bp - 0x10
                    && word(&stack, 12) == *image
                    && word(&stack, 16) == 0
                    && [word(&stack, 20) as i32, word(&stack, 24) as i32] == sp
                    && word(&stack, 28) == 0
                    && word(&stack, 36) == bp - 0x44
                    && (40..=52).step_by(4).all(|i| word(&stack, i) == 0)
                    && row["registers"]["ecx"] == *palette,
                "floor sprite dispatch differs"
            );
            ensure!(
                self.light == Some(sims_sim::lot_presenter::floor_prep::brightness(context)),
                "floor brightness caller changed"
            );
        }
        Ok(())
    }
}
