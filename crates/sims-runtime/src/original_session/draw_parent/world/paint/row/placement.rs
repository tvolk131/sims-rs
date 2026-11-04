//! Frozen-B placement/depth inputs. Bounded samples are indexed spans, never
//! assertions about an allocation's complete extent. C8 expected bytes are not
//! used as replies to geometry or table services.
use super::*;
use sims_sim::{
    lot_presenter::room_masks::{Camera, View},
    pose::transform::Terrain,
};
use sims_vm::scheduler::clock::draw::context::placement::{
    self as native, Cache, Caches, PlacementUpdate,
};
const SOURCE: &str = "758be7c4f7c0d897f37aea4a29166ea26e7193a74b36a300dde68861ec0ec1b0";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    pub caches: Caches,
    clip: [i32; 4],
    camera: Camera,
    bounds: [i32; 4],
    extents: [i32; 3],
    heights: [i32; 3],
    level_depth: [i32; 3],
    limit: i32,
    rows: Vec<i32>,
    #[serde(default)]
    wider_rows: Option<String>,
    curve: Vec<i16>,
    pub ground: u8,
    pub active: i32,
}
impl Source {
    pub fn import(input: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(input) == SOURCE,
            "placement source profile differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "placement source epoch differs"
        );
        let m = Memory::new(&v)?;
        let view = 0x09e88960;
        let vr = m.read(view, 0x520)?;
        let depth = m.read(view + 0xb8, 0xb0)?;
        let project = m.read(0x6f00d0, 16)?;
        let zoom = word(&project, 8);
        ensure!((1..=3).contains(&zoom), "placement zoom differs");
        let lot = word(&m.read(0x6e552c, 4)?, 0);
        let lr = m.read(lot, 0x88)?;
        let table = word(&depth, (7 - zoom) as usize * 4);
        let curve = word(&depth, (3 - zoom) as usize * 4 + 0x48);
        let rows = m
            .read(table - 128, 512)?
            .chunks_exact(4)
            .map(|r| i32::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let curve = m
            .read(curve - 128, 1024)?
            .chunks_exact(2)
            .map(|r| i16::from_le_bytes(r.try_into().unwrap()))
            .collect();
        let cache = |p| -> Result<Cache> {
            let r = m.read(p, 8)?;
            Ok(Cache {
                flags: word(&r, 0),
                value: word(&r, 4),
            })
        };
        let rect = m.read(word(&vr, 0x68) + 0x68, 16)?;
        let s = Self {
            source: SOURCE.into(),
            caches: Caches {
                alpha: cache(0x703f28)?,
                beta: cache(0x703f38)?,
            },
            clip: std::array::from_fn(|i| word(&rect, i * 4) as i32),
            camera: Camera {
                origin: [word(&project, 0) as i32, word(&project, 4) as i32],
                zoom: zoom as u8,
                tile_width: word(&vr, 0x58) as i32,
                half_width: word(&vr, 0x5c) as i32,
                bottom: word(&vr, 0x60) as i32,
                floor_heights: (0..4).map(|i| word(&vr, 0x130 - 4 * i) as i32).collect(),
            },
            bounds: std::array::from_fn(|i| word(&lr, 0x70 + 4 * i) as i32),
            extents: std::array::from_fn(|i| word(&depth, 0x1c + 4 * i) as i32),
            heights: std::array::from_fn(|i| word(&depth, 0x6c + 4 * i) as i32),
            level_depth: std::array::from_fn(|i| word(&depth, 0x84 + 4 * i) as i32),
            limit: word(&depth, 0x28) as i32,
            rows,
            wider_rows: None,
            curve,
            ground: m.read(0x6c724a, 1)?[0],
            active: word(&lr, 0x10) as i32,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && match self.wider_rows.as_deref() {
                    None => self.rows.len() == 128,
                    Some("d4d6ae65fbabf1468a7e4495bd41274f7eb4f79180921e0e4d1e1e71d57487d8") =>
                        self.rows.len() == 1056,
                    Some(_) => false,
                }
                && self.curve.len() == 512
                && (1..=3).contains(&self.camera.zoom),
            "placement/depth input owner differs"
        );
        Ok(())
    }
    pub fn compare(&self, row: &serde_json::Value) -> Result<()> {
        self.validate()?;
        if let Some(value) = row.get("placement_cache") {
            let raw = crate::original_session::completed_animations::bytes(
                value.as_str().context("native placement cache absent")?,
            )?;
            let expected: Vec<u8> = [self.caches.alpha.flags, self.caches.alpha.value]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect();
            ensure!(raw == expected, "native shared projection cache differs");
        }
        Ok(())
    }
    /// Extend only the immutable B row lookup span; preserve mutable caches.
    /// This is an observed index range, not a table allocation-size claim.
    pub fn extend_rows(&mut self, input: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.wider_rows.is_none(), "depth rows already extended");
        let digest = sims_fixtures::digest(input);
        ensure!(
            digest == "d4d6ae65fbabf1468a7e4495bd41274f7eb4f79180921e0e4d1e1e71d57487d8",
            "wider depth source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(input)?;
        let m = Memory::new(&v)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "wider depth epoch differs"
        );
        let z = word(&m.read(0x6f00d0, 16)?, 8);
        ensure!(z == u32::from(self.camera.zoom), "wider depth zoom differs");
        let d = m.read(0x09e88960 + 0xb8, 0xb0)?;
        let pointer = word(&d, ((7 - z) * 4) as usize);
        let rows: Vec<i32> = m
            .read(pointer - 128, 1056 * 4)?
            .chunks_exact(4)
            .map(|v| i32::from_le_bytes(v.try_into().unwrap()))
            .collect();
        ensure!(
            rows[..128] == self.rows,
            "wider depth prefix disagrees with retained B"
        );
        self.rows = rows;
        self.wider_rows = Some(digest);
        self.validate()
    }
    pub fn tile_depth(
        &self,
        sim: &sims_sim::Simulation,
        point: [i8; 3],
        rotation: u8,
    ) -> Result<i32> {
        self.tile_depth_clamped(sim, point, rotation, 1)
    }
    fn tile_depth_clamped(
        &self,
        sim: &sims_sim::Simulation,
        point: [i8; 3],
        rotation: u8,
        clamp: u8,
    ) -> Result<i32> {
        self.validate()?;
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("ground depth terrain absent")?;
        sims_sim::lot_presenter::tile_depth::Inputs {
            size: lot.size,
            rotation,
            heights: &lot.heights,
            zoom: self.camera.zoom,
            row_start: -32,
            rows: &self.rows,
            curve_start: -64,
            curve: &self.curve,
        }
        .sample(point, clamp)
        .map_err(anyhow::Error::msg)
    }
    pub(super) fn ground_depth_unclamped(&self,sim:&sims_sim::Simulation,point:[i8;3],rotation:u8)->Result<i32>{self.tile_depth_clamped(sim,point,rotation,0)}
    pub fn sprite_depth(
        &self,
        sim: &sims_sim::Simulation,
        point: [i8; 3],
        rotation: u8,
        y: i32,
        height: usize,
    ) -> Result<(i32, Vec<i32>)> {
        let start = y
            .checked_sub(self.camera.origin[1])
            .context("sprite row origin overflow")?;
        let at = usize::try_from(start + 32).context("sprite row starts before retained span")?;
        let rows = self
            .rows
            .get(at..at + height)
            .context("sprite depth rows outside retained span")?
            .to_vec();
        let first = *rows.first().context("sprite depth row set empty")?;
        let layer =
            self.level_depth[(3 - self.camera.zoom) as usize].wrapping_mul(i32::from(point[2]) - 1);
        let base = self
            .tile_depth_clamped(sim, point, rotation, 0)?
            .wrapping_add(layer)
            .wrapping_add(65)
            .wrapping_sub(first);
        Ok((base, rows))
    }
    ///4b9991: same-zoom projection and terrain sample use distinct physical
    /// coordinates. The final depth continuation shares the retained caches.
    pub(super) fn project_contained(&self,sim:&sims_sim::Simulation,context:&Frame,rotation:u8)->Result<([i32;2],i32)>{
        self.validate()?;ensure!(rotation<=3,"contained projection rotation differs");
        let w=context.state().words();ensure!(w[8]==u32::from(self.camera.zoom),"contained projection zoom owner differs");
        let cv=|v:f64|->Result<i32>{ensure!(v.is_finite()&&v>=i64::MIN as f64&&v<9223372036854775808.0,"contained projection conversion unobserved");Ok(v.trunc()as i64 as i32)};
        let x=cv(f64::from(f32::from_bits(w[0])+0.5))?;let y=cv(f64::from(f32::from_bits(w[1])+0.5))?;
        let [rx,ry]=match rotation{1=>[1024i32.wrapping_sub(y),x],2=>[1024i32.wrapping_sub(x),1024i32.wrapping_sub(y)],3=>[y,1024i32.wrapping_sub(x)],_=>[x,y]};
        let lot=sim.architecture.as_ref().and_then(|a|a.original.as_ref()).context("contained projection lot absent")?;
        let floor=lot.floor(1).context("contained projection floor absent")?;let splits=std::collections::BTreeMap::new();
        let screen=View{size:lot.size,rotation:0,bounds:self.bounds,camera:&self.camera,heights:&lot.heights,walls:&floor.source.walls,splits:&splits}.fine_screen([ry,rx],w[3]as i32).map_err(anyhow::Error::msg)?;
        let height=Terrain{size:lot.size,corners:&lot.heights}.height_with_control([y,x],0x003f).map_err(anyhow::Error::msg)?.context("contained projection height outside map")?;
        let z=f32::from_bits(w[2]);ensure!(z.is_finite(),"contained projection height is nonfinite");
        let adjust=cv(f64::from((z-height as f32)*(1i32.wrapping_shl(w[8]+1)as f32)+0.5))?;
        Ok(([i32::from(screen[1]),i32::from(screen[0])],adjust))
    }
    pub fn advance(
        &mut self,
        update: &mut PlacementUpdate,
        simulation: &sims_sim::Simulation,
        context: &mut Frame,
    ) -> Result<bool> {
        self.validate()?;
        update
            .validate_words(context.state().words())
            .map_err(anyhow::Error::msg)?;
        if let Some(request) = update.pending() {
            let reply = match request {
                native::Request::Height { physical } => {
                    let lot = simulation
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("placement terrain absent")?;
                    native::Reply::Height(
                        (Terrain {
                            size: lot.size,
                            corners: &lot.heights,
                        }
                        .height_with_control(physical, 0x003f)
                        .map_err(anyhow::Error::msg)?
                        .context("placement terrain lookup outside map")?
                            as f32)
                            .to_bits(),
                    )
                }
                native::Request::Screen {
                    physical,
                    level,
                    height,
                } => {
                    let lot = simulation
                        .architecture
                        .as_ref()
                        .and_then(|a| a.original.as_ref())
                        .context("placement lot absent")?;
                    let floor = lot.floor(1).context("placement floor absent")?;
                    let splits = std::collections::BTreeMap::new();
                    native::Reply::Screen(
                        View {
                            size: lot.size,
                            rotation: 0,
                            bounds: self.bounds,
                            camera: &self.camera,
                            heights: &lot.heights,
                            walls: &floor.source.walls,
                            splits: &splits,
                        }
                        .fine_screen_with_height(physical, level, Some(height))
                        .map_err(anyhow::Error::msg)?,
                    )
                }
                native::Request::OffsetRect { rect, delta } => {
                    native::Reply::Rect(std::array::from_fn(|i| rect[i].wrapping_add(delta[i % 2])))
                }
            };
            update
                .acknowledge(request, reply)
                .map_err(anyhow::Error::msg)?;
            return Ok(true);
        }
        let d = native::Depth {
            origin_y: self.camera.origin[1],
            clip: self.clip,
            heights: self.heights,
            extents: self.extents,
            limit: self.limit,
            level_depth: self.level_depth,
            row_start: -32,
            rows: &self.rows,
            curve_start: -64,
            curve: &self.curve,
        };
        context
            .advance_placement(update, &mut self.caches, &d)
            .map_err(anyhow::Error::msg)
    }
}
