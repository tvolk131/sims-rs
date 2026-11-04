//! Frozen-B renderer owners; expected C8 bytes are comparison-only.
use super::*;
mod raster;
mod terrain;
use sims_vm::scheduler::clock::draw::ground::visible::Frame as Visible;
use sims_vm::scheduler::clock::draw::ground::{self, Frame as Delegate};
const SOURCE: &str = "efb04abd85f0c784afb4e99fc413855fec9ea4fcf6d93ec72323faaa3d1c5439";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Secondary,
    AfterSecondary,
    Transitions,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    stage: Stage,
    wrapper: Vec<u8>,
    primary: Vec<u8>,
    secondary: Vec<u8>,
    cells: Vec<u32>,
    size: usize,
    rotation: u8,
    bounds: [i32; 4],
    flags: Option<Vec<u8>>,
    point: [i32; 2],
    zoom: u32,
    primary_enabled: bool,
    delegate: Option<Delegate>,
    original_alias: u32,
    alias_current: bool,
    #[serde(default)]
    prior_alias_current: bool,
    #[serde(default)]
    terrain: Option<terrain::Source>,
    #[serde(default)]
    visible: Option<Visible>,
    #[serde(default)]
    arguments: Option<([i32; 2], [u8; 4])>,
    #[serde(default)]
    rectangle: Option<sims_sim::lot_presenter::terrain_clip::Frame>,
    #[serde(default)]
    raster_data: Option<sims_sim::lot_presenter::terrain_raster::Data>,
    #[serde(default)]
    raster_frame: Option<sims_sim::lot_presenter::terrain_raster::Frame>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Secondary(ground::Request),
    Terrain(sims_vm::scheduler::clock::draw::ground::visible::Request),
    Primary,
    Transitions,
    RectangleRaster,
}
impl State {
    pub(super) fn raster_data(&self) -> Option<&sims_sim::lot_presenter::terrain_raster::Data> {
        self.raster_data.as_ref()
    }
    pub(super) fn raster_data_mut(
        &mut self,
    ) -> Option<&mut sims_sim::lot_presenter::terrain_raster::Data> {
        self.raster_data.as_mut()
    }
    pub fn import(input: &[u8], point: [i8; 3], zoom: u32) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "ground renderer source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "ground renderer epoch differs"
        );
        let m = Memory::new(&v)?;
        ensure!(
            zoom == word(&m.read(0x6f00d0, 16)?, 8),
            "ground requested/global zoom needs separately retained lookup spans"
        );
        let lot = word(&m.read(0x6e552c, 4)?, 0);
        let lr = m.read(lot, 0x88)?;
        let size = word(&lr, 0x10) as usize;
        let process = word(&m.read(0x6e48bc, 4)?, 0);
        let renderer = word(&m.read(process + 0x20, 4)?, 0);
        let wrapper = m.read(renderer, 0x24)?;
        let primary = m.read(word(&wrapper, 0x10), 0x20)?;
        let secondary = m.read(word(&wrapper, 0x18), 0x3d8)?;
        ensure!(
            word(&m.read(word(&primary, 0), 0x80)?, 0x14) == 0x4c3ba4
                && word(&m.read(word(&secondary, 0), 0x80)?, 0x14) == 0x49ce09,
            "ground delegate dispatch differs"
        );
        let row_pointers = m.read(word(&secondary, 8), size * 4)?;
        let mut cells = vec![];
        for p in row_pointers.chunks_exact(4) {
            cells.extend(
                m.read(word(p, 0), size * 4)?
                    .chunks_exact(4)
                    .map(|b| word(b, 0)),
            );
        }
        let flag_map = word(&m.read(0x6e5810, 4)?, 0);
        let flags = if flag_map == 0 {
            None
        } else {
            let h = m.read(flag_map, 0x18)?;
            let pointers = m.read(word(&h, 0xc), 256)?;
            let mut f = vec![];
            for p in pointers.chunks_exact(4) {
                f.extend(m.read(word(p, 0), 64)?);
            }
            Some(f)
        };
        let s = Self {
            source: SOURCE.into(),
            stage: Stage::Entry,
            wrapper,
            primary,
            secondary,
            cells,
            size,
            rotation: word(&lr, 0x84) as u8,
            bounds: std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
            flags,
            point: [i32::from(point[0]), i32::from(point[1])],
            zoom,
            primary_enabled: false,
            delegate: None,
            original_alias: word(&m.read(0x6edebc, 4)?, 0),
            alias_current: false,
            prior_alias_current: false,
            terrain: None,
            visible: None,
            arguments: None,
            rectangle: None,
            raster_data: None,
            raster_frame: None,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(d) = &self.raster_data {
            d.validate(&self.secondary).map_err(anyhow::Error::msg)?;
        }
        if let Some(f) = &self.raster_frame {
            f.validate(
                self.raster_data.as_ref().context("raster data absent")?,
                &self.secondary,
            )
            .map_err(anyhow::Error::msg)?;
        }
        if let Some(t) = &self.terrain {
            t.validate()?;
        }
        if let Some(r) = &self.rectangle {
            r.validate().map_err(anyhow::Error::msg)?;
            let (screen, corners) = self.arguments.context("rectangle arguments absent")?;
            ensure!(
                r.matches_arguments(
                    self.point.map(|v| v as i8),
                    screen,
                    corners,
                    self.zoom,
                    self.bounds
                ) && matches!(self.stage, Stage::Transitions | Stage::Complete),
                "rectangle child lifetime differs"
            );
        }
        if let Some(v) = &self.visible {
            v.validate(&self.secondary).map_err(anyhow::Error::msg)?;
        }
        ensure!(
            self.source == SOURCE
                && (1..=64).contains(&self.size)
                && self.cells.len() == self.size * self.size
                && self.wrapper.len() == 0x24
                && self.primary.len() == 0x20
                && self.secondary.len() == 0x3d8
                && (1..=3).contains(&self.zoom),
            "ground retained owner differs"
        );
        ensure!(
            self.flags.as_ref().is_none_or(|v| v.len() == 4096),
            "ground flags extent differs"
        );
        ensure!(
            self.delegate.is_some() == (self.stage != Stage::Entry),
            "ground child lifetime differs"
        );
        if let Some(d) = &self.delegate {
            d.validate(&self.secondary).map_err(anyhow::Error::msg)?;
            ensure!(
                self.alias_current == (self.prior_alias_current || d.alias_written()),
                "ground context reference lost its publication"
            );
        }
        Ok(())
    }
    pub fn restart(&mut self, point: [i8; 3], zoom: u32) -> Result<()> {
        self.validate()?;
        ensure!(
            self.complete(),
            "ground renderer still owns its preceding row"
        );
        self.prior_alias_current = self.alias_current;
        self.point = [i32::from(point[0]), i32::from(point[1])];
        self.zoom = zoom;
        self.primary_enabled = false;
        self.delegate = None;
        self.visible = None;
        self.arguments = None;
        self.rectangle = None;
        self.raster_frame = None;
        self.stage = Stage::Entry;
        self.validate()
    }
    pub fn bind_terrain(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.terrain.is_none(), "terrain inputs already bound");
        self.terrain = Some(terrain::Source::import(source)?);
        self.validate()
    }
    pub fn bind_raster(&mut self, source: &[u8], color: &[u8], depth: &[u8]) -> Result<()> {
        ensure!(self.raster_data.is_none(), "raster data already bound");
        self.raster_data = Some(raster::import(source, color, depth, &self.secondary)?);
        self.validate()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn supply_arguments(&mut self, screen: [i32; 2], corners: [u8; 4]) -> Result<()> {
        if let Some(old) = self.arguments {
            ensure!(
                old == (screen, corners),
                "retained rectangle arguments changed"
            );
        } else {
            self.arguments = Some((screen, corners));
        }
        self.validate()
    }
    pub fn pending(&self) -> Option<Request> {
        if self.raster_frame.is_some() {
            return None;
        }
        if let Some(v) = &self.visible {
            if let Some(r) = v.pending() {
                return Some(Request::Terrain(r));
            }
        }
        match self.stage {
            Stage::Secondary => self
                .delegate
                .as_ref()
                .and_then(|d| d.pending())
                .map(Request::Secondary),
            Stage::AfterSecondary if self.primary_enabled => Some(Request::Primary),
            Stage::Transitions if self.wrapper[0x1d] != 0 && self.interior() => {
                match &self.rectangle {
                    Some(r) if r.raster_pending() => Some(Request::RectangleRaster),
                    Some(_) => None,
                    None => Some(Request::Transitions),
                }
            }
            _ => None,
        }
    }
    fn interior(&self) -> bool {
        let [x, y] = self.point;
        x > self.bounds[0] && x < self.bounds[1] && y > self.bounds[2] && y < self.bounds[3]
    }
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        placement: &placement::Source,
        covering: u16,
        clip: [i16; 4],
        draw: bool,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => {
                self.primary_enabled =
                    word(&self.wrapper, 0x10) != 0 && self.wrapper[0x1c] != 0 && covering == 0;
                self.delegate = Some(Delegate::new(self.point, self.zoom));
                self.stage = Stage::Secondary;
            }
            Stage::Secondary => {
                let d = self.delegate.as_mut().unwrap();
                if d.complete() {
                    self.stage = Stage::AfterSecondary;
                } else if let Some(r) = d.pending() {
                    match r {
                        ground::Request::Common => {
                            let p = [self.point[0] as i8, self.point[1] as i8, 0];
                            let depth = placement.tile_depth(sim, p, self.rotation)?;
                            let x = p[0].wrapping_sub(self.bounds[0] as i8);
                            let y = p[1].wrapping_sub(self.bounds[2] as i8);
                            ensure!(
                                x >= 0
                                    && y >= 0
                                    && (x as usize) < self.size
                                    && (y as usize) < self.size,
                                "ground cell lookup outside retained grid"
                            );
                            d.common(self.cells[x as usize * self.size + y as usize], depth)
                                .map_err(anyhow::Error::msg)?;
                        }
                        ground::Request::Visible => {
                            let Some(source) = &self.terrain else {
                                return Ok(false);
                            };
                            let f = self.visible.get_or_insert_with(|| {
                                Visible::new(self.point, self.bounds, self.zoom)
                            });
                            if f.complete() {
                                d.complete_visible().map_err(anyhow::Error::msg)?;
                            } else if !source.advance(
                                f,
                                &mut self.secondary,
                                sim,
                                self.rotation,
                                self.size,
                                self.zoom,
                            )? {
                                return Ok(false);
                            }
                        }
                        ground::Request::Zoom { .. } => {
                            return Ok(false);
                        }
                    }
                } else {
                    let flags = if let Some(f) = &self.flags {
                        let [x, y] = self.point;
                        ensure!(
                            (0..64).contains(&x) && (0..64).contains(&y),
                            "ground flag query outside retained grid"
                        );
                        Some(f[x as usize * 64 + y as usize])
                    } else {
                        None
                    };
                    if !d
                        .advance(&mut self.secondary, flags, self.bounds)
                        .map_err(anyhow::Error::msg)?
                    {
                        return Ok(false);
                    }
                    self.alias_current = self.prior_alias_current || d.alias_written();
                }
            }
            Stage::AfterSecondary => {
                if self.primary_enabled {
                    return Ok(false);
                }
                self.stage = Stage::Transitions;
            }
            Stage::Transitions => {
                if self.wrapper[0x1d] != 0 && self.interior() {
                    let Some((screen, corners)) = self.arguments else {
                        return Ok(false);
                    };
                    let Some(terrain) = &self.terrain else {
                        return Ok(false);
                    };
                    let r = self.rectangle.get_or_insert_with(|| {
                        sims_sim::lot_presenter::terrain_clip::Frame::new(
                            self.point.map(|v| v as i8),
                            screen,
                            corners,
                            self.zoom,
                            self.bounds,
                        )
                    });
                    if r.raster_pending() {
                        let Some(data) = &mut self.raster_data else {
                            return Ok(false);
                        };
                        if self.raster_frame.is_none() {
                            if let Some(spec) = r
                                .raster_spec(
                                    terrain.particle_height(self.zoom)?,
                                    clip,
                                    self.wrapper[0x1f] != 0,
                                    draw,
                                )
                                .map_err(anyhow::Error::msg)?
                            {
                                self.raster_frame = Some(
                                    sims_sim::lot_presenter::terrain_raster::Frame::new(
                                        spec,
                                        &self.secondary,
                                    )
                                    .map_err(anyhow::Error::msg)?,
                                );
                            } else {
                                r.finish_raster().map_err(anyhow::Error::msg)?;
                            }
                            return Ok(true);
                        }
                        let frame = self.raster_frame.as_mut().unwrap();
                        if frame.complete() {
                            r.finish_raster().map_err(anyhow::Error::msg)?;
                            return Ok(true);
                        }
                        return frame
                            .advance(data, &mut self.secondary)
                            .map_err(anyhow::Error::msg);
                    }
                    if !r.complete() {
                        return r
                            .advance(
                                [self.secondary[0x80], self.secondary[0x82]],
                                terrain.particle_height(self.zoom)?,
                                clip,
                            )
                            .map_err(anyhow::Error::msg);
                    }
                }
                self.stage = Stage::Complete;
            }
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        if pc == 0x4c451c {
            return self.raster_frame.is_none()
                && self.rectangle.as_ref().is_some_and(|r| r.raster_pending());
        }
        if let Some(f) = &self.raster_frame {
            if f.at_native_cut(pc) {
                return true;
            }
            if matches!(pc, 0x4c454b | 0x4c45bd | 0x4b444f | 0x4c45c5) && f.at_native_cut(0x49dacd)
            {
                return true;
            }
            if pc == 0x4c45e3 && f.complete() {
                return true;
            }
        }
        if pc == 0x49d24c {
            return self.rectangle.as_ref().is_some_and(|r| r.at_native_cut(pc));
        }
        if self.stage == Stage::Secondary {
            if let Some(v) = &self.visible {
                if v.at_native_cut(pc) {
                    return true;
                }
            } else if pc == 0x49cf02
                && self
                    .delegate
                    .as_ref()
                    .is_some_and(|d| d.pending() == Some(ground::Request::Visible))
            {
                return true;
            }
        }

        match pc {
            0x4af199 | 0x4af1c0 | 0x4c42f0 | 0x4c4319 => self.stage == Stage::Entry,
            0x4c434d | 0x4c4363 => {
                self.stage == Stage::Secondary
                    && self
                        .delegate
                        .as_ref()
                        .is_some_and(|d| d.at_native_cut(0x49ce09))
            }
            0x4c4366 => self.stage == Stage::AfterSecondary,
            0x4c437e | 0x4c4388 => self.stage == Stage::Transitions,
            0x4c4622 | 0x4af1c5 => self.complete(),
            pc => {
                self.stage == Stage::Secondary
                    && self.delegate.as_ref().is_some_and(|d| d.at_native_cut(pc))
            }
        }
    }
    pub(super) fn compare_owned_buffers(&self, row: &serde_json::Value) -> Result<()> {
        self.validate()?;
        let buffers=row.get("buffers").or_else(||row["visible_geometry"].get("buffers")).or_else(||row["corner_geometry"].get("buffers"));
        if buffers.is_some() {
            ensure!(self.raster_data.is_some(),"observed raster planes require their retained Rust owner");
            ensure!(buffers.and_then(|v|v.as_array()).is_some_and(|v|v.len()==2),"observed shared plane count differs");
        }
        if let (Some(d), Some(arrays)) = (
            &self.raster_data,
            row.get("particle_arrays").and_then(|v| v.as_array()),
        ) {
            for (p, a) in d.particles.iter().zip(arrays) {
                ensure!(
                    a["begin"] == p.begin
                        && crate::original_session::completed_animations::bytes(
                            a["raw"].as_str().context("native particles absent")?
                        )? == p.records,
                    "native raster particles differ"
                );
            }
        }
        if let (Some(d), Some(arrays)) = (
            &self.raster_data,
            buffers.and_then(|v| v.as_array()),
        ) {
            for (data, a) in [&d.color, &d.depth].iter().zip(arrays) {
                ensure!(
                    a["sha256"] == sims_fixtures::digest(data),
                    "native raster plane differs"
                );
            }
        }
        Ok(())
    }
    pub fn compare(&self, row: &serde_json::Value) -> Result<()> {
        self.compare_owned_buffers(row)?;
        let pc = row["pc"].as_u64().context("raster observed PC absent")? as u32;
        if pc == 0x49d14b {
            let flags = self
                .visible
                .as_ref()
                .and_then(|f| f.wall_reply())
                .context("terrain wall reply absent")?;
            let raw = crate::original_session::completed_animations::bytes(
                row["inner_frame"]["raw"]
                    .as_str()
                    .context("native wall reply frame absent")?,
            )?;
            ensure!(
                word(&raw, 0) == u32::from(flags),
                "native wall segment reply differs"
            );
        }
        if pc == 0x467690 && !self.complete() {
            ensure!(
                self.matches_wall_call(row)?,
                "native wall query arguments differ"
            );
        }
        if matches!(
            pc,
            0x4c451c | 0x49d24c | 0x4c4622 | 0x4c454b | 0x4c45bd | 0x4b444f | 0x4c45c5 | 0x4c45e3
        ) && self.rectangle.is_some()
        {
            let raw = crate::original_session::completed_animations::bytes(
                row["inner_frame"]["raw"]
                    .as_str()
                    .context("native rectangle frame absent")?,
            )?;
            let expected = std::array::from_fn(|i| word(&raw, 0x14 + i * 4) as i32);
            let actual = if let Some(f) = &self.raster_frame {
                Some(f.spec.rectangle)
            } else {
                self.rectangle.as_ref().and_then(|r| r.rectangle())
            };
            ensure!(
                actual == Some(expected),
                "native rectangle differs at{pc:x}"
            );
        }
        if pc == 0x49dacd {
            let f = self.raster_frame.as_ref().context("raster frame absent")?;
            let raw = crate::original_session::completed_animations::bytes(
                row["stack"]["raw"].as_str().context("raster call absent")?,
            )?;
            ensure!(
                [word(&raw, 4) as i32, word(&raw, 8) as i32] == [f.spec.left, f.spec.bottom]
                    && word(&raw, 16) == f.spec.zoom,
                "raster arguments differ"
            );
        }
        if row["pc"] == 0x49d237u32 {
            if let Some(v) = &self.visible {
                ensure!(
                    v.result().map(u64::from) == row["registers"]["eax"].as_u64(),
                    "terrain preparation quotient differs"
                );
            }
        }
        if row["pc"] == 0x49cd43u32 {
            ensure!(
                self.matches_shade_call(row)?,
                "terrain shade call arguments differ"
            );
        }
        if let Some(owners) = row.get("ground_owners") {
            let owners = owners.as_array().context("ground observed owners absent")?;
            ensure!(owners.len() == 3, "ground observed owner count differs");
            for ((raw, p), o) in [
                (&self.wrapper, 0u32),
                (&self.primary, word(&self.wrapper, 0x10)),
                (&self.secondary, word(&self.wrapper, 0x18)),
            ]
            .into_iter()
            .zip(owners)
            {
                ensure!(
                    (p == 0 || o["pointer"] == p)
                        && crate::original_session::completed_animations::bytes(
                            o["raw"].as_str().context("ground owner bytes absent")?
                        )? == *raw,
                    "ground original owner differs at{:x}: {:?}",
                    row["pc"].as_u64().unwrap(),
                    crate::original_session::completed_animations::bytes(
                        o["raw"].as_str().unwrap()
                    )?
                    .iter()
                    .zip(raw)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(24)
                    .map(|(i, (a, b))| (i, *a, *b))
                    .collect::<Vec<_>>()
                );
            }
            let alias = word(
                &crate::original_session::completed_animations::bytes(
                    row["ground_alias"]
                        .as_str()
                        .context("ground alias absent")?,
                )?,
                0,
            );
            if self.alias_current {
                let expected = crate::original_session::completed_animations::bytes(
                    row["shared_context"]
                        .as_str()
                        .context("ground context absent")?,
                )?;
                ensure!(
                    row["reads"]
                        .as_array()
                        .context("ground observed reads absent")?
                        .iter()
                        .any(|r| r["address"] == alias
                            && r["bytes"] == 116
                            && r["raw"] == row["shared_context"]),
                    "ground current-context alias differs"
                );
                ensure!(
                    expected.len() == 116,
                    "ground current-context extent differs"
                );
            } else {
                ensure!(alias == self.original_alias, "ground prior alias differs");
            }
            ensure!(
                crate::original_session::completed_animations::bytes(
                    row["ground_map_global"]
                        .as_str()
                        .context("ground map owner absent")?
                )? == 0u32.to_le_bytes()
                    && self.flags.is_none(),
                "ground captured flag-map owner differs"
            );
        }
        Ok(())
    }
    pub fn matches_shade_call(&self, row: &serde_json::Value) -> Result<bool> {
        let Some(sims_vm::scheduler::clock::draw::ground::visible::Request::Shade { point }) =
            self.visible.as_ref().and_then(|v| v.pending())
        else {
            return Ok(false);
        };
        let raw = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("native shade stack absent")?,
        )?;
        ensure!(raw.len() >= 12, "native shade call stack short");
        Ok([word(&raw, 4) as i32, word(&raw, 8) as i32] == point
            && row["registers"]["ecx"] == word(&self.wrapper, 0x18))
    }
    pub fn matches_wall_call(&self, row: &serde_json::Value) -> Result<bool> {
        let Some(sims_vm::scheduler::clock::draw::ground::visible::Request::WallFlags { point }) =
            self.visible.as_ref().and_then(|v| v.pending())
        else {
            return Ok(false);
        };
        let raw = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("native wall query stack absent")?,
        )?;
        ensure!(raw.len() >= 11, "native wall query stack short");
        Ok(raw[8..11] == point.map(|v| v as u8))
    }
}
