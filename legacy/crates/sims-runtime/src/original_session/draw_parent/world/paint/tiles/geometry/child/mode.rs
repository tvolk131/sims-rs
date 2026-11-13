//! The ordinary 53dbf2 branch through 53dcc4. B supplies consumed model
//! inputs; the current Rust view and room owners supply the service values.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ZoomCall,
    ZoomReturned,
    LightCall,
    LightReturned,
    PointCall,
    PointReturned,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    point: [u8; 3],
    room_pointer: u32,
    stage: Stage,
    zoom: Option<u8>,
    pending_light: Option<u32>,
    adjusted: Option<[u8; 3]>,
}
impl State {
    pub fn import(m: &Memory, s: &serde_json::Value, raw: &[u8], view: u32) -> Result<Self> {
        let table = word(&m.read(view, 0x80)?, 0);
        let target = word(&m.read(table, 0x100)?, 0x14);
        ensure!(
            s["zoom_getter"] == target
                && target == 0x4aaf3cu32
                && sims_fixtures::digest(&m.read(target, 6)?) == "41d2c23fed43a1e28c11cda3efe2d5f2c1983f13669f2da6c9b83c8622baf8f3",
            "geometry zoom getter differs"
        );
        ensure!(
            s["zoom"] == word(&m.read(0x6f00d8, 4)?, 0),
            "geometry B zoom alias differs"
        );
        let offset = word(raw, 0x18)
            .checked_mul(4)
            .and_then(|v| v.checked_add(0x28))
            .context("geometry room index overflow")? as usize;
        ensure!(
            offset + 4 <= raw.len(),
            "geometry room pointer is outside recovered model input"
        );
        // The owning child verifies this model's descriptor aliases before
        // constructing its mode, including models beyond the first record.
        Ok(Self {
            point: raw[4..7].try_into()?,
            room_pointer: word(raw, offset),
            stage: Stage::Entry,
            zoom: None,
            pending_light: None,
            adjusted: None,
        })
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Entry
    }
    pub fn point_returned(&self) -> Option<[u8; 3]> {
        if self.stage == Stage::PointReturned {
            self.adjusted
        } else {
            None
        }
    }
    pub fn zoom(&self) -> Option<u8> {
        self.zoom
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.zoom.is_some() == !matches!(self.stage, Stage::Entry | Stage::ZoomCall),
            "geometry mode zoom lifetime differs"
        );
        ensure!(
            self.pending_light.is_some() == matches!(self.stage, Stage::LightReturned),
            "geometry mode pending light lifetime differs"
        );
        ensure!(
            self.adjusted.is_some() == matches!(self.stage, Stage::PointReturned),
            "geometry mode point lifetime differs"
        );
        Ok(())
    }
    pub fn advance(
        &mut self,
        sim: &sims_sim::Simulation,
        zoom: u8,
        branch: u8,
        orientation: u32,
        rooms: &BTreeMap<u16, u32>,
        model_light: &mut Option<u32>,
    ) -> Result<bool> {
        self.validate()?;
        ensure!(
            branch == 0,
            "geometry alternate drawing mode is not implemented"
        );
        match self.stage {
            Stage::Entry => self.stage = Stage::ZoomCall,
            Stage::ZoomCall => {
                self.zoom = Some(zoom);
                self.stage = Stage::ZoomReturned;
            }
            Stage::ZoomReturned => {
                self.stage = if self.room_pointer == 0 {
                    Stage::PointCall
                } else {
                    Stage::LightCall
                }
            }
            Stage::LightCall => {
                let id = rooms
                    .iter()
                    .find_map(|(id, p)| (*p == self.room_pointer).then_some(id))
                    .context("geometry model room identity is unobserved")?;
                self.pending_light = Some(
                    sim.room_environment
                        .as_ref()
                        .context("geometry current room owner absent")?
                        .manager
                        .rooms
                        .get(id)
                        .context("geometry current model room absent")?
                        .effective_light()
                        .to_bits(),
                );
                self.stage = Stage::LightReturned;
            }
            Stage::LightReturned => {
                *model_light = self.pending_light.take();
                self.stage = Stage::PointCall;
            }
            Stage::PointCall => {
                self.adjusted = Some(level_geometry::tile_point(self.point, orientation));
                self.stage = Stage::PointReturned;
            }
            Stage::PointReturned => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn at_cut(&self, pc: u32) -> bool {
        matches!(
            (pc, self.stage),
            (0x53dc67, Stage::ZoomCall)
                | (0x53dc6a, Stage::ZoomReturned)
                | (0x53dc78, Stage::LightCall)
                | (0x53dc7d, Stage::LightReturned)
                | (0x53dcbf, Stage::PointCall)
                | (0x53dcc4, Stage::PointReturned)
        )
    }
    pub fn compare(&self, q: &serde_json::Value) -> Result<()> {
        self.validate()?;
        if self.stage == Stage::Entry {
            return Ok(());
        }
        let pc = q["pc"].as_u64().context("geometry mode PC absent")? as u32;
        ensure!(self.at_cut(pc), "geometry mode cut differs");
        let p = &q["geometry_prefix"];
        let frame = decode(&p["frame"]["raw"])?;
        let raw = decode(&p["model_raw"])?;
        ensure!(
            raw[4..7] == self.point
                && word(&raw, word(&raw, 0x18) as usize * 4 + 0x28) == self.room_pointer,
            "geometry consumed point/room inputs differ"
        );
        let bp = p["bp"].as_u64().context("geometry mode BP absent")? as u32;
        match self.stage {
            Stage::ZoomCall => ensure!(
                q["registers"]["ecx"] == p["device"]
                    && q["registers"]["eax"] == p["device_table"]
                    && word(&decode(&p["device_vtable"])?, 0x14) == 0x4aaf3c,
                "geometry zoom call differs"
            ),
            Stage::ZoomReturned => ensure!(
                q["registers"]["eax"] == self.zoom.unwrap(),
                "geometry current zoom result differs"
            ),
            Stage::LightCall => ensure!(
                q["registers"]["ecx"] == self.room_pointer,
                "geometry model light receiver differs"
            ),
            Stage::PointCall | Stage::PointReturned => {
                let z = u32::from(self.zoom.unwrap());
                ensure!(
                    word(&frame, 0x178 - 0x58) == 8u32.wrapping_shl(z)
                        && word(&frame, 0x178 - 0xd4) == 4u32.wrapping_shl(z)
                        && frame[0x178 - 0x44..0x178 - 0x41] == self.point,
                    "geometry tile preparation differs"
                );
                let stack = decode(&p["stack"]["raw"])?;
                ensure!(
                    word(&stack, 0) == bp.wrapping_sub(0x1c)
                        && word(&stack, 4) == bp.wrapping_sub(0x44)
                        && word(&stack, 8) == word(&raw, 0x10),
                    "geometry point call arguments differ"
                );
                if let Some(point) = self.adjusted {
                    ensure!(
                        frame[0x178 - 0x1c..0x178 - 0x19] == point
                            && q["registers"]["eax"] == bp.wrapping_sub(0x1c),
                        "geometry point result differs"
                    );
                }
            }
            _ => (),
        }
        if !matches!(self.stage, Stage::ZoomCall | Stage::ZoomReturned) {
            ensure!(
                word(&frame, 0x178 - 0x24) == u32::from(self.zoom.unwrap()),
                "geometry stored zoom differs"
            );
        }
        Ok(())
    }
}
