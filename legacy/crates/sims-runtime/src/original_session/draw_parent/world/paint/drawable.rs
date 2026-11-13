//!4ab56d's eligibility, fine-coordinate rotation and shared lighting setup.
//!The virtual draw/hide calls remain explicit children. No drawing result,
//!appearance change or contained-object rectangle is synthesized here.
use super::*;
use sims_sim::lot_environment::lighting::point::{map_sample, Sample};
use sims_vm::scheduler::clock::draw::context::{Frame as Context, LightUpdate};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Cursor {
    object: sims_vm::ObjectId,
    pointer: u32,
    pc: u32,
    fine: Option<[i32; 2]>,
    tile: Option<[i8; 3]>,
    level: Option<i32>,
    eligible: Option<bool>,
    person: Option<bool>,
    light: Option<LightUpdate>,
}
impl Cursor {
    pub fn new(object: sims_vm::ObjectId, pointer: u32) -> Result<Self> {
        ensure!(object > 0 && pointer != 0, "drawable identity absent");
        Ok(Self { object, pointer, pc: 0x4ab56d, fine: None, tile: None,
            level: None, eligible: None, person: None, light: None })
    }
    pub fn pc(&self) -> u32 { self.pc }
    pub fn identity(&self) -> (sims_vm::ObjectId, u32) { (self.object, self.pointer) }
    pub fn pending(&self) -> Option<Request> {
        let function = match self.pc {
            0x4ab67a => if self.person? { 0x45e33e } else { 0x43776a },
            0x4ab712 => if self.person? { 0x45e414 } else { 0x437869 },
            _ => return None,
        };
        Some(Request::DrawableChild { object: self.object, pointer: self.pointer, function })
    }
    pub fn validate(&self, context: &Context) -> Result<()> {
        ensure!(self.object > 0 && self.pointer != 0, "drawable identity absent");
        ensure!(matches!(self.pc,0x4ab56d|0x4ab5bd|0x4ab5c0|0x4ab605|0x4ab60a|0x4ab62f|0x4ab633|0x4ab652|0x4ab657|0x4ab662|0x4ab667|0x4ab66e|0x4ab673|0x4ab67a|0x4ab712),"drawable cursor phase differs");
        if self.pc != 0x4ab56d { ensure!(self.fine.is_some()&&self.tile.is_some(),"drawable lost its position reads"); }
        if !matches!(self.pc,0x4ab56d|0x4ab5bd) { ensure!(self.level.is_some(),"drawable lost its view-level read"); }
        if !matches!(self.pc,0x4ab56d|0x4ab5bd|0x4ab5c0) { ensure!(self.eligible.is_some(),"drawable lost its interior test"); }
        if !matches!(self.pc,0x4ab56d|0x4ab5bd|0x4ab5c0|0x4ab605) { ensure!(self.person.is_some(),"drawable lost its RTTI result"); }
        ensure!(self.light.is_some()==matches!(self.pc,0x4ab667|0x4ab66e|0x4ab673|0x4ab67a),"drawable lost its light spill");
        if matches!(self.pc,0x4ab673|0x4ab67a) { ensure!(self.light.as_ref().is_some_and(LightUpdate::complete),"drawable returned from lighting before both stores"); }
        if let Some(light) = &self.light {
            light.validate_words(context.state().words()).map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
    pub fn advance(&mut self, sim: &sims_sim::Simulation,
        view: &composition::View, context: &mut Context) -> Result<bool> {
        self.validate(context)?;
        if self.pending().is_some() { return Ok(false); }
        let position = || sim.vm.world.positions.get(&self.object).context("drawable physical position absent");
        self.pc = match self.pc {
            0x4ab56d => {
                let p = position()?;
                self.fine = Some([p.x, p.y]);
                self.tile = Some([(p.y >> 4) as i8, (p.x >> 4) as i8, p.level as i8]);
                0x4ab5bd
            }
            0x4ab5bd => {
                self.level = Some(view.scene_level());
                0x4ab5c0
            }
            0x4ab5c0 => {
                let [x, y, z] = self.tile.context("drawable tile absent")?.map(i32::from);
                let size = sim.architecture.as_ref().and_then(|a|a.original.as_ref())
                    .context("drawable terrain absent")?.size as i32;
                self.eligible = Some(z <= self.level.context("drawable view level absent")?
                    && x >= 1 && x < size.wrapping_sub(1) && y >= 1 && y < size.wrapping_sub(1));
                0x4ab605
            }
            0x4ab605 => {
                self.person = Some(sim.vm.world.objects.get(&self.object)
                    .context("drawable object absent")?.person.is_some());
                0x4ab60a
            }
            0x4ab60a => {
                if self.person == Some(true) && self.eligible == Some(false) {
                    if i32::from(self.tile.unwrap()[2]) != self.level.unwrap().wrapping_add(1) {
                        self.pc = 0x4ab712;
                        return Ok(true);
                    }
                    self.eligible = Some(sim.vm.world.objects.get(&self.object)
                        .and_then(|o|o.person.as_ref()).and_then(|p|p.get(&40))
                        .copied().context("drawable person field40 absent")? != 0);
                }
                if self.person == Some(true) && self.eligible == Some(true) { 0x4ab652 }
                else { 0x4ab62f }
            }
            0x4ab62f => 0x4ab633,
            0x4ab633 => if self.eligible.context("drawable eligibility absent")? { 0x4ab652 } else { 0x4ab712 },
            0x4ab652 => {
                let [x, y] = self.fine.context("drawable fine position absent")?;
                self.fine = Some(match view.scene_rotation() {
                    0 => [x,y], 1 => [y,1023i32.wrapping_sub(x)],
                    2 => [1023i32.wrapping_sub(x),1023i32.wrapping_sub(y)],
                    3 => [1023i32.wrapping_sub(y),x],
                    _ => anyhow::bail!("drawable rotation differs"),
                });
                0x4ab657
            }
            0x4ab657 => 0x4ab662,
            0x4ab662 => {
                let level = i32::from(self.tile.context("drawable tile absent")?[2]);
                let lot = sim.architecture.as_ref().and_then(|a|a.original.as_ref())
                    .context("drawable lighting terrain absent")?;
                let floor = lot.floor(level).context("drawable lighting floor absent")?;
                let environment = &sim.room_environment.as_ref().context("drawable lighting environment absent")?.manager;
                let sample = map_sample(self.fine.context("drawable fine point absent")?, level,
                    view.scene_rotation(), lot.size, &floor.rooms.primary,
                    environment.light_bytes.get((level - 1) as usize).context("drawable room light bytes absent")?)
                    .map_err(anyhow::Error::msg)?;
                let Sample::Value(bits) = sample else { anyhow::bail!("drawable requires its split-room light child"); };
                self.light = Some(LightUpdate::new(bits));
                0x4ab667
            }
            0x4ab667 => 0x4ab66e,
            0x4ab66e => {
                let light = self.light.as_mut().context("drawable light spill absent")?;
                if !light.complete() { context.advance_light(light).map_err(anyhow::Error::msg)?; return Ok(true); }
                0x4ab673
            }
            0x4ab673 => 0x4ab67a,
            _ => anyhow::bail!("drawable requires its actual virtual child at {:x}",self.pc),
        };
        self.validate(context)?;
        Ok(true)
    }
    pub fn compare(&self, q: &serde_json::Value, context: &Context) -> Result<()> {
        self.validate(context)?;
        ensure!(q["pc"] == self.pc && q["tick"] == 89081, "drawable native phase differs");
        let e = &q["late_objects"];
        ensure!(e["drawable"]["pointer"] == self.pointer, "drawable pointer differs");
        let decode = |v:&serde_json::Value| crate::original_session::completed_animations::bytes(v.as_str().context("drawable bytes absent")?);
        let raw = decode(&e["drawable"]["raw"])?;
        if let Some(tile) = self.tile { ensure!(raw[0xfc..0xff] == tile.map(|v|v as u8), "drawable tile differs"); }
        let stack = decode(&e["stack"]["raw"])?;
        let base = e["stack"]["address"].as_u64().context("drawable stack absent")? as i64;
        let bp = q["registers"]["ebp"].as_u64().context("drawable frame absent")? as i64;
        if let Some(fine) = self.fine {
            let at = usize::try_from(bp - 0x20 - base).context("drawable locals outside captured stack")?;
            ensure!(stack.get(at..at+8) == Some(fine.into_iter().flat_map(i32::to_le_bytes).collect::<Vec<_>>().as_slice()), "drawable fine locals differ");
        }
        if self.pc == 0x4ab5c0 { ensure!(q["registers"]["eax"] == self.level.unwrap(), "drawable level getter differs"); }
        if self.pc == 0x4ab60a { ensure!(q["registers"]["eax"] == if self.person.unwrap(){self.pointer}else{0}, "drawable RTTI result differs"); }
        ensure!(decode(&q["context"]["raw"])? == context.state().words().iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>(), "drawable shared context differs");
        Ok(())
    }
}
