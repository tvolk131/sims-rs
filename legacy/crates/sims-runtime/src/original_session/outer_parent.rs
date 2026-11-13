//! Retained5d5e7c caller and shared process owners for fresh Goth B.
//! Rendering remains an explicit child. Completing this outer frame never
//! silently begins a new application frame or consumes an external clock.
use super::{completed_lot::Memory, completed_profiler};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::scheduler::clock::{application, outer, profiler::tail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Profile {
    Goth89073,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Application,
    Returning,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Callback {
    receiver: u32,
    priority: i32,
    returns_al: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    profile: Profile,
    phase: Phase,
    pub profiler: tail::State,
    pub application_time: application::Fields,
    counters: outer::Counters,
    registry: Vec<Callback>,
    frame: Option<outer::Frame>,
    #[serde(default)]
    window: Option<super::window_parent::State>,
    #[serde(default)]
    draw: Option<super::draw_parent::State>,
    #[serde(default)]
    window_inputs:Option<super::draw_parent::WindowInputs>,
    #[serde(default)]
    world_composition:Option<super::draw_parent::WorldComposition>,
    #[serde(default)]
    cursor_inputs:Option<super::draw_parent::CursorInputs>,
    #[serde(default)]
    world_scene:Option<super::draw_parent::WorldScene>,
    #[serde(default)]
    world_classification:Option<super::draw_parent::WorldClassification>,
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// Both supplements join the same frozen original B. No callback entry,
/// application entry, global-time update or constructor is replayed here.
pub fn import(process_source: &[u8], source: &[u8]) -> Result<State> {
    ensure!(
        sims_fixtures::digest(source)
            == "b9dc7312c18b832f3521cb7e232e2c458a7afa2ff0a6317907c4bdfe6999692b",
        "enclosing source differs"
    );
    let v: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        v["tick"] == 89073 && v["executes_target_code"] == false && v["native_parity_credit"] == 0,
        "enclosing source epoch differs"
    );
    let memory = Memory::new(&v)?;
    let process = completed_profiler::import_process(process_source)?;
    let header = memory.read(0x6fe8b8, 128)?;
    ensure!(
        [word(&header, 88), word(&header, 92)] == process.outer_counters,
        "enclosing counter supplement differs"
    );
    let time = memory.read(0x6e54e0, 24)?;
    ensure!(
        std::array::from_fn::<_, 6, _>(|i| word(&time, i * 4)) == process.application_time.words(),
        "enclosing time supplement differs"
    );
    let tree = memory.read(0x030f5d44, 16)?;
    let mut cursor = word(&tree, 4);
    let count = word(&tree, 8);
    ensure!(count == 4, "periodic registry count differs");
    let end = word(&memory.read(cursor, 24)?, 8);
    let mut callbacks = Vec::new();
    for _ in 0..count {
        ensure!(cursor != end, "periodic traversal ended early");
        let node = memory.read(cursor, 24)?;
        let parent = word(&node, 4);
        if node[0] == 0 && word(&memory.read(parent, 24)?, 4) == cursor {
            cursor = word(&node, 12);
        } else if word(&node, 8) != 0 {
            cursor = word(&node, 8);
            for _ in 0..8 {
                let next = word(&memory.read(cursor, 24)?, 12);
                if next == 0 {
                    break;
                }
                cursor = next;
            }
            ensure!(
                word(&memory.read(cursor, 24)?, 12) == 0,
                "periodic descent exceeds bound"
            );
        } else {
            let mut child = cursor;
            cursor = parent;
            for _ in 0..8 {
                let p = memory.read(cursor, 24)?;
                if child != word(&p, 8) {
                    break;
                }
                child = cursor;
                cursor = word(&p, 4);
            }
            ensure!(
                child != word(&memory.read(cursor, 24)?, 8),
                "periodic ascent exceeds bound"
            );
        }
        let node = memory.read(cursor, 24)?;
        let receiver = word(&node, 20);
        let table = word(&memory.read(receiver, 32)?, 0);
        let target = word(&memory.read(table + 12, 4)?, 0);
        let code = memory.read(target, 5)?;
        let returns_al = match code.as_slice() {
            [0x32, 0xc0, 0xc2, 4, 0] => 0,
            [0xb0, 1, 0xc2, 4, 0] => 1,
            _ => anyhow::bail!("periodic callback needs a qualified implementation"),
        };
        callbacks.push(Callback {
            receiver,
            priority: word(&node, 16) as i32,
            returns_al,
        });
    }
    ensure!(cursor == end, "periodic traversal did not consume registry");
    let state = State {
        profile: Profile::Goth89073,
        phase: Phase::Application,
        profiler: process.profiler,
        application_time: process.application_time,
        counters: outer::Counters::from_words(process.outer_counters),
        registry: callbacks,
        frame: None,
        window: None,
        draw: None,
        window_inputs:None,world_composition:None,cursor_inputs:None,world_scene:None,world_classification:None,
    };
    state.validate()?;
    Ok(state)
}
impl State {
    pub fn counters(&self) -> [u32; 2] {
        self.counters.words()
    }
    pub fn pending(&self) -> Option<outer::Request> {
        self.frame.as_ref().and_then(outer::Frame::pending)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(child)=&self.world_classification {
            child.validate()?;
            ensure!(self.world_scene.is_some()&&self.world_composition.is_some(),"classification lost its shared scene/view");
            if self.phase==Phase::Application||!self.framework_ready_for_draw(){ensure!(child.at_entry(),"classification ran before framework preparation");}
            if self.draw_world_pending()==Some(super::draw_parent::WorldRequest::PrepareCamera){ensure!(child.complete(),"World skipped its classification child");}
        }
        if let Some(scene)=&self.world_scene {
            scene.validate()?;
            scene.validate_view(self.world_composition.as_ref().context("scene lost its shared view owner")?)?;
            if self.phase==Phase::Application||!self.framework_ready_for_draw() {
                ensure!(scene.at_entry(),"scene ran before framework preparation");
            }
            if self.draw_world_pending()==Some(super::draw_parent::WorldRequest::RefreshIndicators) {
                ensure!(scene.complete(),"World skipped its retained scene child");
            }
        }
        ensure!(self.world_composition.is_some()==self.cursor_inputs.is_some(),"composition lost its cursor input owner");
        if let (Some(view),Some(inputs))=(&self.world_composition,&self.cursor_inputs) {
            view.validate()?;inputs.validate()?;ensure!(self.draw.is_some()&&view.cursor_calls()==inputs.consumed(),"composition cursor/input progress differs");
        }
        if let Some(inputs)=&self.window_inputs {self.draw.as_ref().context("OS window input owner lost Draw")?.validate_window_inputs(inputs)?;}
        if let Some(draw) = &self.draw {
            draw.validate()?;
            if self.phase == Phase::Application || !self.framework_ready_for_draw() {
                ensure!(
                    draw.at_entry(),
                    "application Draw ran before framework preparation"
                );
            }
            if self.phase == Phase::Complete || self.pending() == Some(outer::Request::Profiler) {
                ensure!(draw.complete(), "enclosing return skipped application Draw");
            }
        }
        if let Some(window) = &self.window {
            window.validate()?;
            if self.phase == Phase::Application
                || self.frame.as_ref().is_some_and(|f| {
                    f.at_entry() || matches!(f.pending(), Some(outer::Request::Periodic { .. }))
                })
            {
                ensure!(
                    window.at_entry(),
                    "framework dispatch ran before enclosing render call"
                );
            }
            if self.phase == Phase::Complete || self.pending() == Some(outer::Request::Profiler) {
                ensure!(
                    window.complete(),
                    "enclosing return skipped framework dispatch"
                );
            }
        }
        self.profiler.validate().map_err(anyhow::Error::msg)?;
        let expected = [
            (0x30f61dc, 1_100_000, 0),
            (0x30f5ee0, 1_000_000, 1),
            (0x30f5d54, 0, 1),
            (0x30f5458, -2_000_000, 0),
        ];
        ensure!(
            self.registry.len() == expected.len()
                && self.registry.iter().zip(expected).all(
                    |(a, (receiver, priority, returns_al))| (a.receiver, a.priority, a.returns_al)
                        == (receiver, priority, returns_al)
                ),
            "enclosing periodic source profile differs"
        );
        match self.phase {
            Phase::Application => ensure!(
                self.frame.is_none() && self.counters.words() == [9516, 2379],
                "enclosing application suspension differs"
            ),
            Phase::Returning | Phase::Complete => {
                let frame = self
                    .frame
                    .as_ref()
                    .context("enclosing return cursor absent")?;
                frame.validate().map_err(anyhow::Error::msg)?;
                ensure!(
                    frame.complete() == (self.phase == Phase::Complete),
                    "enclosing completion differs"
                );
                let counts = if frame.at_entry() {
                    [9516, 2379]
                } else {
                    [9517, 2380]
                };
                ensure!(
                    self.counters.words() == counts,
                    "enclosing source counters differ"
                );
            }
        }
        Ok(())
    }
    pub fn bind_window_source(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Application && self.window.is_none(),
            "framework window binding already started"
        );
        self.window = Some(super::window_parent::import(source)?);
        Ok(())
    }
    pub fn bind_draw_source(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Application && self.draw.is_none(),
            "application Draw binding already started"
        );
        self.draw = Some(super::draw_parent::import(source)?);
        Ok(())
    }
    pub fn bind_draw_child_source(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Application,
            "draw child binding already executing"
        );
        self.draw
            .as_mut()
            .context("source-bound application Draw absent")?
            .bind_child_source(source)
    }
    pub fn advance_draw(&mut self) -> Result<Option<sims_vm::scheduler::clock::draw::Request>> {
        self.validate()?;
        ensure!(
            self.pending() == Some(outer::Request::Render) && self.framework_ready_for_draw(),
            "application Draw does not own framework control"
        );
        let draw = self
            .draw
            .as_mut()
            .context("source-bound application Draw absent")?;
        for _ in 0..4 {
            let request = draw.advance()?;
            if request.is_none() || !draw.service_child(&mut self.profiler.events)? {
                return Ok(request);
            }
        }
        anyhow::bail!("draw service child work bound exceeded")
    }
    pub fn bind_draw_action_source(&mut self, source:&[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.phase==Phase::Application,"action-window binding already executing");
        self.draw.as_mut().context("source-bound application Draw absent")?.bind_action_source(source)
    }
    pub fn advance_draw_action_view(&mut self, controls:&sims_vm::interface::state::State) -> Result<Option<sims_vm::scheduler::clock::draw::actions::Request>> {
        self.validate()?;
        ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"action-window child does not own framework control");
        self.draw.as_mut().context("source-bound application Draw absent")?.advance_action_view(controls)
    }
    pub fn bind_draw_action_record_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"record resource binding already executing");
        self.draw.as_mut().context("source-bound application Draw absent")?.bind_action_record_source(source)
    }
    pub fn bind_draw_action_collection_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"collection binding already executing");
        self.draw.as_mut().context("source-bound application Draw absent")?.bind_action_collection_source(source)
    }
    pub fn bind_draw_action_record_build_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"record build source binding already executing");
        self.draw.as_mut().context("source-bound application Draw absent")?.bind_action_record_build_source(source)
    }
    pub fn bind_draw_action_record_services(&mut self,code:&[u8],object:&[u8],pixels:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"record services binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_action_record_services(code,object,pixels)
    }
    pub fn bind_draw_action_image_publication(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"image publication binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_action_image_publication(source)
    }
    pub fn bind_draw_action_tail(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"action tail binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_action_tail(source)
    }
    pub fn bind_draw_framework_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"framework source binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_framework_source(source)
    }
    pub fn bind_draw_world_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"World source binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_world_source(source)
    }
    pub fn bind_draw_world_prefix(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"World prefix binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_world_prefix(source)
    }
    pub fn bind_draw_window_inputs(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application&&self.window_inputs.is_none()&&self.draw.is_some(),"OS window inputs already executing/unbound");
        let inputs=super::draw_parent::WindowInputs::import(source)?;
        self.draw.as_ref().unwrap().validate_window_inputs(&inputs)?;
        self.window_inputs=Some(inputs);self.validate()
    }
    pub fn service_draw_world_window(&mut self,tick:u64)->Result<bool> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"World OS input does not own framework control");
        let inputs=self.window_inputs.as_mut().context("original OS window inputs absent")?;
        let result=self.draw.as_mut().context("source-bound Draw absent")?.service_world_window(tick,inputs)?;self.validate()?;Ok(result)
    }
    pub fn advance_draw_world_scan(&mut self,simulation:&sims_sim::Simulation)->Result<()> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"World scene does not own framework control");
        self.draw.as_mut().context("source-bound Draw absent")?.advance_world_scan(simulation)?;self.validate()
    }
    pub fn draw_world_pending(&self)->Option<super::draw_parent::WorldRequest>{self.draw.as_ref().and_then(super::draw_parent::State::world_pending)}
    pub fn acknowledge_draw_world_timestamp(&mut self,time:u64)->Result<()> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"World timestamp does not own framework control");
        self.draw.as_mut().context("source-bound Draw absent")?.acknowledge_world_timestamp(time,&mut self.profiler.events)?;self.validate()
    }
    pub fn bind_draw_world_composition(&mut self,source:&[u8],cursor:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application&&self.world_composition.is_none(),"composition source binding already executing");
        let view=super::draw_parent::WorldComposition::import(source)?;let inputs=super::draw_parent::CursorInputs::import(cursor)?;
        ensure!(self.draw.as_ref().context("source-bound Draw absent")?.world_at_entry(),"composition source needs its unstarted World caller");
        self.world_composition=Some(view);self.cursor_inputs=Some(inputs);self.validate()
    }
    pub fn service_draw_world_composition(&mut self,simulation:&sims_sim::Simulation,controls:&sims_vm::interface::state::State)->Result<bool> {
        self.validate()?;
        ensure!(matches!(self.draw.as_ref().and_then(super::draw_parent::State::world_pending),Some(super::draw_parent::WorldRequest::Compose{..})),"composition caller is not pending");
        let view=self.world_composition.as_mut().context("retained composition view absent")?;let inputs=self.cursor_inputs.as_mut().context("original cursor inputs absent")?;
        let result=if matches!(view.pending(),Some(super::draw_parent::CompositionRequest::GetCursorPos|super::draw_parent::CompositionRequest::ScreenToClient{..})) {inputs.service(view,simulation.vm.tick)?}
            else {self.draw.as_mut().context("source-bound Draw absent")?.advance_world_composition(view,simulation,controls)?};
        self.validate()?;Ok(result)
    }
    /// Bind the same frozen B wall/region owner that the World caller borrows.
    /// This never installs a reference endpoint or refreshes the simulation.
    pub fn bind_draw_world_scene(&mut self,source:&[u8],simulation:&sims_sim::Simulation)->Result<()> {
        self.validate()?;
        ensure!(self.phase==Phase::Application&&self.world_scene.is_none(),"scene source binding already executing");
        ensure!(self.draw.as_ref().context("source-bound Draw absent")?.world_at_entry(),"scene source needs its unstarted World caller");
        let scene=super::draw_parent::WorldScene::import(source,simulation)?;
        scene.validate_view(self.world_composition.as_ref().context("scene needs the retained composition view")?)?;
        self.world_scene=Some(scene);self.validate()
    }
    pub fn service_draw_world_scene(&mut self,simulation:&mut sims_sim::Simulation)->Result<bool> {
        self.validate()?;
        ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"scene does not own framework control");
        let scene=self.world_scene.as_mut().context("retained World scene absent")?;
        let view=self.world_composition.as_mut().context("retained composition view absent")?;
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_scene(scene,view,simulation)?;
        self.validate()?;Ok(progress)
    }
    pub fn bind_draw_world_camera(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"camera binding already executing");self.draw.as_mut().context("source-bound Draw absent")?.bind_world_camera(source)?;self.validate()
    }
    pub fn service_draw_world_camera(&mut self)->Result<bool> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"camera preparation does not own framework control");
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_camera(self.world_composition.as_mut().context("shared view absent")?)?;self.validate()?;Ok(progress)
    }
    pub fn bind_draw_world_drawing(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"drawing binding already executing");self.draw.as_mut().context("source-bound Draw absent")?.bind_world_drawing(source)?;self.validate()
    }
    pub fn service_draw_world_drawing(&mut self)->Result<bool>{
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"drawing setup does not own framework control");
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_drawing(&mut self.profiler.events)?;self.validate()?;Ok(progress)
    }
    pub fn bind_draw_world_paint(&mut self,source:&[u8])->Result<()>{
        self.validate()?;ensure!(self.phase==Phase::Application,"paint binding already executing");
        self.draw.as_mut().context("source-bound Draw absent")?.bind_world_paint(source)?;self.validate()
    }
    pub fn service_draw_world_paint(&mut self)->Result<bool>{
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"paint does not own framework control");
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_paint(self.world_scene.as_mut().context("retained World scene absent")?,&mut self.profiler.events)?;self.validate()?;Ok(progress)
    }
    pub fn bind_draw_world_surfaces(&mut self,source:&[u8],native:&[u8])->Result<()>{
        self.validate()?;ensure!(self.phase==Phase::Application,"surface binding already executing");self.draw.as_mut().context("source-bound Draw absent")?.bind_world_surfaces(source,native)?;self.validate()
    }
    pub fn bind_draw_world_people(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.phase==Phase::Application,"person visibility binding already executing");self.draw.as_mut().context("source-bound Draw absent")?.bind_world_people(source)?;self.validate()
    }
    pub fn service_draw_world_people(&mut self,simulation:&mut sims_sim::Simulation,assets:&std::collections::BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>,source:super::character_visibility::Source)->Result<bool>{
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"person visibility does not own framework control");
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_people(simulation,assets,source,self.world_classification.as_mut().context("shared classification absent")?)?;self.validate()?;Ok(progress)
    }
    pub fn bind_draw_world_classification(&mut self,source:&[u8],simulation:&sims_sim::Simulation)->Result<()> {
        self.validate()?;
        ensure!(self.phase==Phase::Application&&self.world_classification.is_none()&&self.world_scene.is_some(),"classification binding already executing/unbound");
        ensure!(self.draw.as_ref().context("source-bound Draw absent")?.world_at_entry(),"classification binding needs its unstarted World caller");
        let child=super::draw_parent::WorldClassification::import(source,simulation)?;
        self.world_classification=Some(child);self.validate()
    }
    pub fn service_draw_world_classification(&mut self,simulation:&mut sims_sim::Simulation,ordinary:&sims_content::graphic_loading::Saved)->Result<bool> {
        self.validate()?;
        ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"classification does not own framework control");
        let child=self.world_classification.as_mut().context("retained World classification absent")?;
        let scene=self.world_scene.as_mut().context("retained World scene absent")?;
        let view=self.world_composition.as_ref().context("retained World composition absent")?;
        let progress=self.draw.as_mut().context("source-bound Draw absent")?.advance_world_classification(child,simulation,ordinary,scene,view)?;
        self.validate()?;Ok(progress)
    }
    pub fn draw_action_image_pending(&self)->Option<&sims_vm::scheduler::clock::draw::actions::image::Request>{self.draw.as_ref().and_then(super::draw_parent::State::action_image_pending)}
    pub fn advance_draw_action_view_for_program(&mut self,controls:&sims_vm::interface::state::State,simulation:&mut sims_sim::Simulation,program:&sims_vm::Program)->Result<Option<sims_vm::scheduler::clock::draw::actions::Request>> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render) && self.framework_ready_for_draw(),"record services do not own framework control");
        self.draw.as_mut().context("source-bound Draw absent")?.advance_action_view_for_program(controls,simulation,program)
    }
    pub fn advance_draw_action_view_for_simulation(&mut self,controls:&sims_vm::interface::state::State,simulation:&sims_sim::Simulation)->Result<Option<sims_vm::scheduler::clock::draw::actions::Request>> {
        self.validate()?;ensure!(self.pending()==Some(outer::Request::Render)&&self.framework_ready_for_draw(),"collection child does not own framework control");
        self.draw.as_mut().context("source-bound application Draw absent")?.advance_action_view_for_simulation(controls,simulation)
    }
    pub fn draw_action_collection_pending(&self)->Option<&sims_vm::scheduler::clock::draw::actions::collect::Request> {
        self.draw.as_ref().and_then(super::draw_parent::State::action_collection_pending)
    }
    pub fn draw_action_record_pending(&self)->Option<sims_vm::scheduler::clock::draw::actions::clear::Request> {
        self.draw.as_ref().and_then(super::draw_parent::State::action_record_pending)
    }
    pub fn acknowledge_draw(
        &mut self,
        request: sims_vm::scheduler::clock::draw::Request,
        reply: sims_vm::scheduler::clock::draw::Reply,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.pending() == Some(outer::Request::Render) && self.framework_ready_for_draw(),
            "application Draw does not own framework reply"
        );
        self.draw
            .as_mut()
            .context("source-bound application Draw absent")?
            .acknowledge(request, reply, &mut self.profiler.events)
    }
    pub fn advance_window(&mut self) -> Result<Option<super::window_parent::Request>> {
        self.validate()?;
        ensure!(
            self.pending() == Some(outer::Request::Render),
            "render child does not own framework dispatch"
        );
        self.window
            .as_mut()
            .context("source-bound framework window absent")?
            .advance()
    }
    pub fn acknowledge_window(
        &mut self,
        request: super::window_parent::Request,
        reply: super::window_parent::Reply,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.pending() == Some(outer::Request::Render),
            "render child does not own framework reply"
        );
        self.window
            .as_mut()
            .context("source-bound framework window absent")?
            .acknowledge(request, reply)
    }
    /// Timer/message preparation does not complete the native draw child.
    pub fn framework_ready_for_draw(&self) -> bool {
        self.window
            .as_ref()
            .is_some_and(super::window_parent::State::complete)
    }
    /// Borrow the same global event vector while the application child returns.
    pub fn complete_application_profiler(
        &mut self,
        session: &mut super::Session,
        timestamp: u64,
    ) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Application,
            "application child has already returned"
        );
        session.complete_application_profiler(&mut self.profiler.events, timestamp)
    }
    pub fn begin_return(&mut self, session: &super::Session) -> Result<()> {
        self.validate()?;
        ensure!(
            self.phase == Phase::Application,
            "enclosing return already began"
        );
        let clock = session
            .host
            .presentation
            .clock
            .as_ref()
            .context("enclosing elapsed caller absent")?;
        clock.validate(session)?;
        let parent = session
            .host
            .presentation
            .application_parent
            .as_ref()
            .context("enclosing application caller absent")?;
        parent.validate(Some(clock), session.simulation.vm.tick)?;
        ensure!(
            parent.ready_for_enclosing_return()
                && !clock.pending()
                && clock.now == self.application_time.now,
            "application still owns enclosing control"
        );
        self.start_return();
        Ok(())
    }
    fn start_return(&mut self) {
        self.phase = Phase::Returning;
        self.frame = Some(outer::Frame::at_application_return());
    }
    pub fn advance_return(&mut self) -> Result<Option<outer::Request>> {
        self.validate()?;
        ensure!(
            self.phase != Phase::Application,
            "application child still owns enclosing control"
        );
        for _ in 0..3 {
            let frame = self.frame.as_mut().unwrap();
            let request = frame
                .advance(&mut self.counters)
                .map_err(anyhow::Error::msg)?;
            if let Some(request @ outer::Request::Periodic { .. }) = request {
                // The source-bound roster above contains only the two exact
                // RET4 stubs;603021 ignores their AL results. No VM/clock/RNG
                // or registry mutation occurs in those four callback bodies.
                frame.acknowledge(request).map_err(anyhow::Error::msg)?;
            } else {
                if request.is_none() {
                    self.phase = Phase::Complete;
                }
                self.validate()?;
                return Ok(request);
            }
        }
        anyhow::bail!("enclosing return exceeded work bound")
    }
    pub fn complete_profiler_tail(&mut self, timing: Option<[u64; 3]>, control: u16) -> Result<()> {
        self.validate()?;
        ensure!(
            self.pending() == Some(outer::Request::Profiler),
            "profiler is not the pending enclosing child"
        );
        ensure!(
            self.framework_ready_for_draw(),
            "framework dispatch is not complete"
        );
        self.profiler
            .finish_frame(timing, control)
            .map_err(anyhow::Error::msg)?;
        self.frame
            .as_mut()
            .unwrap()
            .acknowledge(outer::Request::Profiler)
            .map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    #[ignore = "private source-bound original enclosing registry"]
    fn source_roster_and_suspended_render_restore() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let process = std::fs::read(
            root.join("fixtures/private/goth-outer-frame-20261031-first/outer-source-third.json"),
        )?;
        let source = std::fs::read(root.join(
            "fixtures/private/goth-enclosing-frame-20261031-first/enclosing-source-first.json",
        ))?;
        let mut state = import(&process, &source)?;
        let absent = postcard::to_allocvec(&state)?;
        let restored_absent: State = postcard::from_bytes(&absent)?;
        assert_eq!(restored_absent, state);
        let window_source = std::fs::read(root.join(
            "fixtures/private/goth-enclosing-frame-20261031-first/window-source-second.json",
        ))?;
        state.bind_window_source(&window_source)?;
        let draw_source =
            std::fs::read(root.join(
                "fixtures/private/goth-render-parent-20261031-first/render-source-second.json",
            ))?;
        state.bind_draw_source(&draw_source)?;
        let os_input=std::fs::read(root.join("fixtures/private/goth-world-api-witness-20261031-first/world-api-trace.jsonl.gz"))?;
        let before_inputs=state.clone();assert!(state.bind_draw_window_inputs(&os_input).is_err());assert_eq!(state,before_inputs);
        let mut bound=state.clone();
        bound.bind_draw_world_source(&std::fs::read(root.join("fixtures/private/goth-world-draw-20261031-first/world-source-second.json"))?)?;
        bound.bind_draw_world_prefix(&std::fs::read(root.join("fixtures/private/goth-world-prefix-20261031-first/prefix-source-first.json"))?)?;
        bound.bind_draw_window_inputs(&os_input)?;
        bound.bind_draw_world_composition(&std::fs::read(root.join("fixtures/private/goth-world-composition-20261031-first/composition-source-fourth.json"))?,&std::fs::read(root.join("fixtures/private/goth-world-composition-witness-20261031-second/world-composition-trace.jsonl.gz"))?)?;
        let report=std::fs::read(root.join("fixtures/private/goth-world-scene-20261031-first/corrected-prefix-first.json"))?;
        ensure!(sims_fixtures::digest(&report)=="c0258d64c53fefe82d163a1d76ac12187f63a0dad41fba788b4533c3e64061d6","corrected C8 prefix changed");
        let report:serde_json::Value=serde_json::from_slice(&report)?;
        let store=sims_fixtures::Store::open(root.join("fixtures/private/goth-world-scene-20261031-first/corrected-prefix-first.store"))?;
        let retained:super::super::completed_assets::checkpoints::Retained=store.get_value(&serde_json::from_value(report["boundaries"][7]["state"].clone())?)?;
        let mut simulation=retained.simulation.state;
        bound.bind_draw_world_scene(&std::fs::read(root.join("fixtures/private/goth-world-scene-20261031-first/scene-source-second.json"))?,&simulation)?;
        bound.bind_draw_world_classification(&std::fs::read(root.join("fixtures/private/goth-world-classification-20261031-first/classification-source-third.json"))?,&simulation)?;
        bound.bind_draw_world_camera(&std::fs::read(root.join("fixtures/private/goth-world-classification-20261031-first/classification-source-third.json"))?)?;
        bound.bind_draw_world_people(&std::fs::read(root.join("fixtures/private/goth-world-person-visibility-20261031-first/person-source-first.json"))?)?;
        bound.bind_draw_world_drawing(&std::fs::read(root.join("fixtures/private/goth-world-drawing-20261031-first/drawing-source-second.json"))?)?;
        bound.bind_draw_world_paint(&std::fs::read(root.join("fixtures/private/goth-world-paint-20261031-first/paint-source-first.json"))?)?;
        bound.bind_draw_world_surfaces(&std::fs::read(root.join("fixtures/private/goth-world-surfaces-20261031-first/surface-source-first.json"))?,&std::fs::read(root.join("fixtures/private/goth-world-surface-witness-20261031-first/world-classification-trace.jsonl.gz"))?)?;
        let cold:State=postcard::from_bytes(&postcard::to_allocvec(&bound)?)?;cold.validate()?;assert_eq!(cold,bound);
        let before_bound=bound.clone();let before_vm=serde_json::to_vec(&simulation)?;
        assert!(bound.service_draw_world_scene(&mut simulation).is_err());
        assert!(bound.service_draw_world_camera().is_err());
        assert!(bound.service_draw_world_drawing().is_err());
        assert!(bound.service_draw_world_paint().is_err());
        assert!(bound.service_draw_world_people(&mut simulation,&retained.characters,retained.presentation.visibility).is_err());
        assert!(bound.service_draw_world_classification(&mut simulation,&retained.ordinary).is_err());
        assert_eq!(bound,before_bound);assert_eq!(serde_json::to_vec(&simulation)?,before_vm);
        let child_source = std::fs::read(root.join(
            "fixtures/private/goth-draw-services-20261031-first/services-source-first.json",
        ))?;
        state.bind_draw_child_source(&child_source)?;
        let action_source=std::fs::read(root.join("fixtures/private/goth-action-refresh-20261031-first/action-source-first.json"))?;
        state.bind_draw_action_source(&action_source)?;
        let before = postcard::to_allocvec(&state)?;
        assert!(state.advance_draw().is_err());
        assert_eq!(postcard::to_allocvec(&state)?, before);
        assert!(state.advance_window().is_err());
        assert_eq!(postcard::to_allocvec(&state)?, before);
        assert!(state.advance_return().is_err());
        assert_eq!(postcard::to_allocvec(&state)?, before);
        // Cursor test starts at the qualified return PC, without claiming any
        // Session or lot execution. The public handoff additionally checks it.
        state.start_return();
        assert_eq!(state.advance_return()?, Some(outer::Request::Render));
        assert_eq!(state.counters(), [9517, 2380]);
        let saved = postcard::to_allocvec(&state)?;
        let mut restored: State = postcard::from_bytes(&saved)?;
        assert_eq!(restored.advance_return()?, Some(outer::Request::Render));
        assert_eq!(postcard::to_allocvec(&restored)?, saved);
        assert!(
            restored
                .complete_profiler_tail(Some([0; 3]), 0x003f)
                .is_err()
        );
        assert_eq!(postcard::to_allocvec(&restored)?, saved);
        assert_eq!(state.application_time, restored.application_time);
        assert_eq!(state.profiler, restored.profiler);
        let native: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sims-vm/tests/fixtures/native-framework-window.json"
        )))?;
        let mut calls = Vec::new();
        while let Some(request) = restored.advance_window()? {
            let frozen = postcard::to_allocvec(&restored)?;
            restored = postcard::from_bytes(&frozen)?;
            assert_eq!(restored.advance_window()?, Some(request));
            assert_eq!(postcard::to_allocvec(&restored)?, frozen);
            use super::super::window_parent::{Reply as A, Request as W};
            use sims_vm::ui_effect::widgets::timers::dispatch::{Reply as R, Request as T};
            let reply = match request {
                W::Timer(T::PointerPosition) => {
                    calls.push("point");
                    A::Timer(R::Point(serde_json::from_value(native["point"].clone())?))
                }
                W::Timer(T::PublishPointer {
                    message,
                    argument,
                    packed,
                }) => {
                    calls.push("post");
                    assert_eq!(message, 13);
                    assert_eq!(native["posted"]["argument"], argument);
                    assert_eq!(native["posted"]["packed"], packed);
                    A::Timer(R::Done)
                }
                W::Timer(T::Clock) => {
                    calls.push("clock");
                    A::Timer(R::Clock(native["clock"].as_u64().unwrap().try_into()?))
                }
                _ => anyhow::bail!("captured framework requested an unexpected child"),
            };
            restored.acknowledge_window(request, reply)?;
        }
        assert_eq!(calls, ["point", "post", "clock"]);
        assert!(restored.framework_ready_for_draw());
        assert_eq!(restored.pending(), Some(outer::Request::Render));
        assert_eq!(restored.counters(), [9517, 2380]);
        assert_eq!(restored.application_time, state.application_time);
        assert_eq!(restored.profiler, state.profiler);
        let window = restored.window.as_ref().unwrap();
        let (timers, flags) = window.timers();
        assert_eq!(serde_json::to_value(timers)?, native["after"]["registry"]);
        assert_eq!(serde_json::to_value(flags)?, native["after"]["flags"]);
        assert_eq!(
            serde_json::to_value(window.messages())?,
            native["messages_after"]
        );
        assert!(
            restored
                .complete_profiler_tail(Some([0; 3]), 0x003f)
                .is_err()
        );
        let mut premature = restored.clone();
        premature.phase = Phase::Application;
        premature.frame = None;
        premature.counters = outer::Counters::from_words([9516, 2379]);
        assert!(premature.validate().is_err());
        use sims_vm::scheduler::clock::draw;
        let marker = restored.advance_draw()?.context("Draw marker absent")?;
        assert_eq!(marker, draw::Request::Timestamp { name: 0x6c7c7c });
        let saved = postcard::to_allocvec(&restored)?;
        restored = postcard::from_bytes(&saved)?;
        assert_eq!(restored.advance_draw()?, Some(marker));
        assert_eq!(postcard::to_allocvec(&restored)?, saved);
        let count = restored.profiler.events.records().len();
        restored.acknowledge_draw(marker, draw::Reply::Timestamp(123))?;
        assert_eq!(restored.profiler.events.records().len(), count + 1);
        assert_eq!(
            restored.profiler.events.records().last().unwrap().timestamp,
            123
        );
        let request = restored.advance_draw()?.context("Draw FPS clock absent")?;
        assert_eq!(request, draw::Request::FpsClock);
        let saved = postcard::to_allocvec(&restored)?;
        restored = postcard::from_bytes(&saved)?;
        assert_eq!(restored.advance_draw()?, Some(request));
        restored.acknowledge_draw(request, draw::Reply::Clock(758330))?;
        assert_eq!(
            restored.draw.as_ref().unwrap().history().fields(),
            [1000, 4, 0, 0]
        );
        assert_eq!(
            restored.advance_draw()?,
            Some(draw::Request::ApplicationView)
        );
        assert_eq!(restored.draw.as_ref().unwrap().view_frame_word(), Some(0));
        // This owner-coupling test supplies Live mode. The inner source-bound
        // Draw test separately imports and validates the actual captured CPState.
        let mut controls=sims_vm::interface::state::State::constructed(0);controls.mode=2;
        assert_eq!(restored.advance_draw_action_view(&controls)?,Some(draw::actions::Request::ClearRecords));
        let saved=postcard::to_allocvec(&restored)?;
        restored=postcard::from_bytes(&saved)?;
        assert_eq!(restored.advance_draw_action_view(&controls)?,Some(draw::actions::Request::ClearRecords));
        assert_eq!(postcard::to_allocvec(&restored)?,saved);
        assert!(restored.acknowledge_draw(draw::Request::ApplicationView,draw::Reply::Done).is_err());
        assert_eq!(restored.application_time, state.application_time);
        assert_eq!(restored.pending(), Some(outer::Request::Render));
        assert!(
            restored
                .complete_profiler_tail(Some([0; 3]), 0x003f)
                .is_err()
        );
        let mut invalid: serde_json::Value = serde_json::to_value(restored)?;
        invalid["registry"][0]["returns_al"] = 1.into();
        assert!(
            serde_json::from_value::<State>(invalid)?
                .validate()
                .is_err()
        );
        let mut changed: serde_json::Value = serde_json::from_slice(&source)?;
        changed["tick"] = 89074.into();
        assert!(import(&process, &serde_json::to_vec(&changed)?).is_err());
        Ok(())
    }
}
