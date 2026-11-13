//! Frozen Goth B application Draw owner. The body retains its FPS history and
//! suspends at real callbacks. It does not complete framework rendering.
use super::completed_lot::Memory;
use anyhow::{Result, ensure, Context as _};
use serde::{Deserialize, Serialize};
use sims_vm::scheduler::clock::{draw, fps::History, profiler::Buffer};
mod children;
mod action_view;
mod framework;
mod world;
mod windows;
pub use world::Request as WorldRequest;
pub use world::paint::Request as PaintRequest;
pub use world::paint::GeometryInput;
pub use windows::Inputs as WindowInputs;
pub use world::classification::{State as WorldClassification,Request as ClassificationRequest};
pub use world::scene::{State as WorldScene,Request as SceneRequest};
pub use world::composition::{View as WorldComposition, PointInputs as CursorInputs, Request as CompositionRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    fields: draw::Fields,
    history: History,
    frame: draw::Frame,
    #[serde(default)]
    children: Option<children::Owners>,
    #[serde(default)]
    actions: Option<action_view::Owner>,
    #[serde(default)]
    framework:Option<framework::Owner>,
    #[serde(default)]
    world:Option<world::Owner>,
}
fn word(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap())
}
pub fn import(source: &[u8]) -> Result<State> {
    ensure!(
        sims_fixtures::digest(source)
            == "891373e26b6aad4c4f17c98c3f4a1238d9864f90038b06a0bb9d3f73e1d07c65",
        "application Draw source differs"
    );
    let value: serde_json::Value = serde_json::from_slice(source)?;
    ensure!(
        value["schema"] == 1
            && value["tick"] == 89073
            && value["executes_target_code"] == false
            && value["native_parity_credit"] == 0,
        "application Draw source epoch differs"
    );
    let memory = Memory::new(&value)?;
    for (address, size, hash) in [
        (
            0x4d1b37,
            180,
            "54be876feb783c50dddf95317601856b628a343c9e87dcac6a2d4d73142d605e",
        ),
        (
            0x47306a,
            30,
            "8c5e0a95cfa3c3dc61f4104ca42d22d11f67e44c2e235f76cda4d3f3ea13b83b",
        ),
        (
            0x473103,
            95,
            "0802f88d309afe292a94b118eada9ab35ba9cb4d9fc8a67a8b7ff6ec56a44b27",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&memory.read(address, size)?) == hash,
            "application Draw body differs"
        );
    }
    let app = memory.read(0x0315a250, 0x11c0)?;
    let h = std::array::from_fn::<_, 4, _>(|i| word(&app, 0x1154 + i * 4));
    let history = History::from_captured_parts(
        (0..1000).map(|i| word(&app, 0x1b4 + i * 4)).collect(),
        h[0],
        h[1],
        h[2],
        h[3],
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        history.fields() == [1000, 3, 0, 0],
        "frozen application FPS cursor differs"
    );
    let surface_owner = word(&memory.read(0x6e48bc, 4)?, 0);
    let fields = draw::Fields {
        busy: memory.read(0x6f0f48, 1)?[0],
        view: match word(&app, 0x174) {
            0 => None,
            owner => Some(owner),
        },
        unload: match word(&app, 0x11ac) {
            0 => None,
            owner => Some(owner),
        },
        framework: word(&memory.read(0x6fe944, 4)?, 0),
        world: word(&memory.read(surface_owner, 8)?, 4),
    };
    ensure!(fields.busy == 0, "frozen application Draw already entered");
    let state = State {
        fields,
        history,
        frame: draw::Frame::at_entry(),
        children: None,
        actions: None,
        framework:None,
        world:None,
    };
    state.validate()?;
    Ok(state)
}
impl State {
    pub fn at_entry(&self) -> bool {
        self.frame.is_at_entry()
    }
    pub fn complete(&self) -> bool {
        self.frame.complete()
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    pub fn pending(&self) -> Option<draw::Request> {
        self.frame.pending()
    }
    pub fn validate(&self) -> Result<()> {
        self.frame.validate().map_err(anyhow::Error::msg)?;
        self.history.validate().map_err(anyhow::Error::msg)?;
        if let Some(children) = &self.children {
            children.validate(self.frame.past_view_begin())?;
        }
        if let Some(actions) = &self.actions {
            actions.validate()?;
            ensure!(!self.frame.past_application_view() || actions.complete(), "Draw bypassed retained action-window caller");
            ensure!(actions.at_entry() || self.frame.at_application_view() || (actions.complete() && self.frame.past_application_view()), "action-window caller escaped its Draw parent");
        }
        if let Some(framework)=&self.framework {
            framework.validate(self.actions.as_ref().ok_or_else(||anyhow::anyhow!("framework shared action owner absent"))?.state())?;
            ensure!(!self.frame.past_framework()||framework.complete(),"Draw bypassed retained framework caller");
            ensure!(framework.at_entry()||matches!(self.pending(),Some(draw::Request::Framework{..}))||framework.complete()&&self.frame.past_framework(),"framework caller escaped Draw");
        }
        if let Some(world)=&self.world {
            world.validate()?;
            ensure!(world.at_entry() || self.pending()==Some(draw::Request::World{owner:0x09e88960,argument:0}) || world.complete()&&self.frame.past_world(),"World prefix escaped its pending Draw callback");
            ensure!(!self.frame.past_world() || world.complete(),"Draw bypassed retained World callback");
        }
        ensure!(
            self.fields.view == Some(0x0ac04dd8)
                && self.fields.unload == Some(0x0a9ab808)
                && self.fields.framework == 0x6fedf8
                && self.fields.world == 0x09e88960,
            "source-bound Draw callback owners differ"
        );
        ensure!(
            self.fields.busy == u8::from(!self.at_entry() && !self.complete()),
            "application Draw guard disagrees with cursor"
        );
        Ok(())
    }
    pub fn bind_child_source(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(
            self.at_entry() && self.children.is_none(),
            "draw child binding already started"
        );
        self.children = Some(children::import(source)?);
        Ok(())
    }
    pub fn bind_action_source(&mut self, source: &[u8]) -> Result<()> {
        self.validate()?;
        ensure!(self.at_entry() && self.actions.is_none(), "action-window binding already started");
        self.actions=Some(action_view::import(source)?);self.validate()
    }
    pub fn action_view_pending(&self) -> Option<&draw::actions::Request> {
        self.actions.as_ref().and_then(action_view::Owner::pending)
    }
    pub fn bind_action_record_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"record resource binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action-window caller absent"))?.bind_records(source)
    }
    pub fn action_record_pending(&self)->Option<draw::actions::clear::Request> {
        self.actions.as_ref().and_then(action_view::Owner::record_pending)
    }
    pub fn bind_action_collection_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"collection source binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action-window caller absent"))?.bind_collection(source)
    }
    pub fn action_collection_pending(&self)->Option<&draw::actions::collect::Request> {
        self.actions.as_ref().and_then(action_view::Owner::collection_pending)
    }
    pub fn bind_action_record_build_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"record build source binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action-window caller absent"))?.bind_record_build(source)
    }
    pub fn bind_action_record_services(&mut self,code:&[u8],object:&[u8],pixels:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"record services binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action caller absent"))?.bind_services(code,object,pixels)
    }
    pub fn bind_action_image_publication(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"image publication binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action caller absent"))?.bind_publication(source)
    }
    pub fn bind_action_tail(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"action tail binding already executing");
        self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action caller absent"))?.bind_tail(source)
    }
    pub fn bind_framework_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.framework.is_none(),"framework binding already started");
        self.framework=Some(framework::import(source,self.actions.as_ref().ok_or_else(||anyhow::anyhow!("framework action owner absent"))?.state())?);self.validate()
    }
    pub fn framework_region_pending(&self)->Option<&draw::regions::Request>{self.framework.as_ref().and_then(framework::Owner::pending_region)}
    pub fn bind_world_source(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry()&&self.world.is_none(),"World source binding already started");
        self.world=Some(world::import(source)?);self.validate()
    }
    pub fn world_pending(&self)->Option<WorldRequest>{self.world.as_ref().and_then(world::Owner::pending)}
    pub fn world_context(&self)->Option<&draw::context::State>{self.world.as_ref().and_then(world::Owner::context)}
    pub fn bind_world_prefix(&mut self,source:&[u8])->Result<()> {
        self.validate()?;ensure!(self.at_entry(),"World prefix binding already executing");
        self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source owner absent"))?.bind_prefix(source)?;self.validate()
    }
    pub fn service_world_window(&mut self,tick:u64,inputs:&mut WindowInputs)->Result<bool> {
        self.validate_window_inputs(inputs)?;
        ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"World OS input does not own Draw");
        let Some(WorldRequest::ClientRectangle{window,..})=self.world_pending() else{return Ok(false)};
        let Some(reply)=inputs.peek(tick,window)? else{return Ok(false)};
        self.world.as_mut().unwrap().client_reply(reply)?;inputs.consume(reply)?;self.validate_window_inputs(inputs)?;Ok(true)
    }
    pub fn validate_window_inputs(&self,inputs:&WindowInputs)->Result<()> {
        self.validate()?;inputs.validate()?;
        let world=self.world.as_ref().ok_or_else(||anyhow::anyhow!("OS window input lost World caller"))?;
        ensure!(inputs.consumed()==usize::from(world.client_returned()),"OS reply cursor and World return disagree");Ok(())
    }
    pub fn advance_world_scan(&mut self,simulation:&sims_sim::Simulation)->Result<()> {
        self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"World redraw scan does not own Draw");
        self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source owner absent"))?.scan(simulation)?;self.validate()
    }
    /// Borrow the single retained view/mask owner rather than accepting Done.
    pub fn advance_world_composition(&mut self,view:&mut WorldComposition,simulation:&sims_sim::Simulation,controls:&sims_vm::interface::state::State)->Result<bool> {
        self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"World composition does not own Draw");
        let world=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source owner absent"))?;
        let advanced=world.compose(view,simulation,controls)?;
        self.validate()?;Ok(advanced)
    }
    pub(super) fn world_at_entry(&self)->bool{self.world.as_ref().is_some_and(world::Owner::at_entry)}
    pub fn advance_world_scene(&mut self,scene:&mut WorldScene,view:&mut WorldComposition,simulation:&mut sims_sim::Simulation)->Result<bool>{
        self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"scene does not own Draw");
        let result=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World caller absent"))?.scene(scene,view,simulation)?;self.validate()?;Ok(result)
    }
    pub fn advance_world_classification(&mut self,child:&mut WorldClassification,simulation:&mut sims_sim::Simulation,ordinary:&sims_content::graphic_loading::Saved,scene:&mut WorldScene,view:&WorldComposition)->Result<bool>{
        self.validate()?;
        ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"classification does not own Draw");
        let progress=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World caller absent"))?.classification(child,simulation,ordinary,scene,view)?;
        self.validate()?;Ok(progress)
    }


    pub fn bind_world_camera(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source absent"))?.bind_camera(source)?;self.validate()}
    pub fn advance_world_camera(&mut self,view:&mut WorldComposition)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"camera preparation does not own Draw");let progress=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World caller absent"))?.camera_prepare(view)?;self.validate()?;Ok(progress)}
    pub fn world_camera(&self)->Option<&world::camera_prepare::State>{self.world.as_ref().and_then(world::Owner::camera)}
    pub fn bind_world_people(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source absent"))?.bind_people(source)?;self.validate()}
    pub fn advance_world_people(&mut self,simulation:&mut sims_sim::Simulation,assets:&std::collections::BTreeMap<sims_vm::ObjectId,sims_content::character_loading::live::SavedAssets>,source:super::character_visibility::Source,classify:&mut WorldClassification)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"person visibility does not own Draw");let progress=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World caller absent"))?.people_visibility(simulation,assets,source,classify)?;self.validate()?;Ok(progress)}
    pub fn world_people(&self)->Option<&world::person_visibility::State>{self.world.as_ref().and_then(world::Owner::people)}

    pub fn bind_world_drawing(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().ok_or_else(||anyhow::anyhow!("World absent"))?.bind_drawing(source)?;self.validate()}
    pub fn world_drawing(&self)->Option<&world::drawing::State>{self.world.as_ref().and_then(world::Owner::drawing)}
    pub fn advance_world_drawing(&mut self,profiler:&mut Buffer)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"drawing setup does not own Draw");let progress=self.world.as_mut().ok_or_else(||anyhow::anyhow!("World absent"))?.advance_drawing(self.framework.as_mut().ok_or_else(||anyhow::anyhow!("shared framework absent"))?,profiler)?;self.validate()?;Ok(progress)}
    pub fn compare_world_drawing(&self,row:&serde_json::Value,scene:&WorldScene,profiler:&Buffer)->Result<()>{self.world.as_ref().ok_or_else(||anyhow::anyhow!("World absent"))?.compare_drawing(row,self.framework.as_ref().ok_or_else(||anyhow::anyhow!("shared framework absent"))?,scene,profiler)}
    pub fn world_redraw_match(&self)->Option<sims_vm::ObjectId>{self.world.as_ref().and_then(world::Owner::matched)}
    pub fn bind_world_paint(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_paint(source)?;self.validate()}
    pub fn world_paint(&self)->Option<&world::paint::State>{self.world.as_ref().and_then(world::Owner::paint)}
    pub fn bind_world_region_loop(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_region_loop(source)?;self.validate()}
    pub fn bind_world_tiles(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_tiles(source)?;self.validate()}
    pub fn bind_world_row_ordinary_objects(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_ordinary_objects(source)?;self.validate()}
    pub fn bind_world_row_ordinary_physical(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_ordinary_physical(source)?;self.validate()}
    pub fn bind_world_row_ordinary(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_ordinary(source)?;self.validate()}
    pub fn bind_world_row_prefix(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row(source)?;self.validate()}
    pub fn bind_world_object_shadow(&mut self,encoded:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_shadow(encoded)?;self.validate()}
    pub fn bind_world_object_raster(&mut self,encoded:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_object_raster(encoded)?;self.validate()}
    pub fn bind_world_object_raster_batch(&mut self,encoded:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_raster_batch(encoded)?;self.validate()}
    pub fn bind_world_object_raster_mirror(&mut self,encoded:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_raster_mirror(encoded)?;self.validate()}
    pub fn bind_world_object_auxiliary(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_auxiliary(encoded,directory)?;self.validate()}
    pub fn bind_world_object_slot_records(&mut self,encoded:&[u8],directory:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_slot_records(encoded,directory)?;self.validate()}
    pub fn bind_world_object_slots(&mut self,directory:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("world absent")?.bind_row_object_slots(directory)?;self.validate()}
    pub fn bind_world_object_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_object_catalog(encoded,directory,sim)?;self.validate()}
    pub fn bind_world_override_catalog(&mut self,encoded:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_override_catalog(encoded,directory,sim)?;self.validate()}
    pub fn bind_world_surface_end(&mut self,source:&[u8],native:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_surface_end(source,native)?;self.validate()}
    pub fn bind_world_post_window(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_post_window(source)?;self.validate()}
    pub fn bind_world_surface_copy(&mut self,source:&[u8],inputs:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_surface_copy(source,inputs)?;self.validate()}
    pub fn bind_world_surface_front(&mut self,source:&[u8],inputs:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_surface_front(source,inputs)?;self.validate()}
    pub fn bind_world_surface_post_end(&mut self,source:&[u8],inputs:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_surface_post_end(source,inputs)?;self.validate()}
    pub fn bind_world_model(&mut self,source:&[u8],inputs:&[u8],classify:&WorldClassification)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_model(source,inputs,classify)?;self.validate()}
    pub fn at_world_model_cut(&self,q:&serde_json::Value)->bool{self.world.as_ref().is_some_and(|w|w.at_model_cut(q))}
    pub fn at_world_drawable_cut(&self,q:&serde_json::Value)->bool{self.world.as_ref().is_some_and(|w|w.at_drawable_cut(q))}
    pub fn advance_world_drawable(&mut self,sim:&sims_sim::Simulation,view:&WorldComposition)->Result<bool>{
        self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"drawable does not own Draw");
        let progress=self.world.as_mut().context("World absent")?.advance_drawable(sim,view)?;self.validate()?;Ok(progress)
    }
    pub fn world_post_window_request(&self)->Option<PaintRequest>{self.world.as_ref()?.post_window_request()}
    pub fn world_drawing_excluded_person(&self)->Option<u32>{self.world.as_ref()?.drawing_excluded_person()}
    pub fn compare_world_post_window(&self,q:&serde_json::Value,profiler:&Buffer)->Result<()>{self.validate()?;self.world.as_ref().context("World absent")?.compare_post_window(q,profiler)}
    pub fn compare_world_surface_end(&self,row:&serde_json::Value)->Result<()>{self.validate()?;self.world.as_ref().context("World absent")?.compare_surface_end(row)}
    pub fn bind_world_row_visibility(&mut self,view:&WorldComposition)->Result<()>{self.validate()?;let value=view.object_visibility()?;self.world.as_mut().context("World absent")?.bind_row_visibility(value)?;self.validate()}
    pub fn bind_world_raster_family(&mut self,input:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_raster_family(input)?;self.validate()}
    /// Apply only the retained renderer's pending object-field store. It cannot
    /// advance the VM scheduler, consume RNG, or reconstruct a missing owner.
    pub fn advance_world_row_objects_mut(&mut self,sim:&mut sims_sim::Simulation,classify:&mut WorldClassification)->Result<bool>{
        self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"row does not own Draw");
        if let Some((id,old,value))=self.world_paint().and_then(|p|p.row_prefix()).and_then(|r|r.visibility_write()){
            let object=sim.vm.world.objects.get_mut(&id).context("visibility store object absent")?;
            ensure!(object.data.get(&34)==Some(&old),"visibility store source word changed");object.data.insert(34,value);
            self.world.as_mut().context("World absent")?.commit_row_visibility((id,old,value))?;self.validate()?;return Ok(true);
        }
        self.advance_world_row_objects(sim,classify)
    }
    pub fn bind_world_object_sprite_frames(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_object_sprite_frames(source)?;self.validate()}
    pub fn bind_world_object_sprites(&mut self,source:&[u8],directory:&[u8],sim:&sims_sim::Simulation)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_object_sprites(source,directory,sim)?;self.validate()}
    pub fn advance_world_row_objects(&mut self,sim:&sims_sim::Simulation,classify:&mut WorldClassification)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"row does not own Draw");let progress=self.world.as_mut().context("World absent")?.advance_row_objects(sim,Some(classify))?;self.validate()?;Ok(progress)}
    pub fn advance_world_row_prefix(&mut self,simulation:&sims_sim::Simulation)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"row does not own Draw");let progress=self.world.as_mut().context("World absent")?.advance_row(simulation)?;self.validate()?;Ok(progress)}
    /// Batch only the world-owned row microsteps. The borrowed world cannot
    /// modify the parent's other owners; their deep guards run before/after.
    /// World/row/cursor validation still runs on every microstep. Observations
    /// select a stopping point and never supply simulated values.
    pub fn advance_world_row_prefix_to(&mut self,simulation:&sims_sim::Simulation,observation:&serde_json::Value,limit:usize)->Result<usize>{
        self.validate()?;let mut count=0;
        for _ in 0..limit{
            ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"row does not own Draw");
            if self.world_paint().and_then(|p|p.row_prefix()).context("row absent")?.at_native_observation(observation)?{break;}
            if !self.world.as_mut().context("World absent")?.advance_row(simulation)?{break;}count+=1;
        }
        self.validate()?;Ok(count)
    }
    pub fn compare_world_row_prefix(&self,row:&serde_json::Value)->Result<()>{self.validate()?;self.world.as_ref().context("World absent")?.compare_row(row)}
    pub fn bind_world_row_raster(&mut self,source:&[u8],color:&[u8],depth:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_raster(source,color,depth)?;self.validate()}
    pub fn extend_world_row_depth(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.extend_row_depth(source)?;self.validate()}
    pub fn bind_world_row_floor_sprite(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_floor_sprite(source)?;self.validate()}
    pub fn bind_world_row_floor(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_floor(source)?;self.validate()}
    pub fn bind_world_row_split(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_split(source)?;self.validate()}
    pub fn bind_world_row_contents(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_contents(source)?;self.validate()}
    pub fn bind_world_row_terrain(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_terrain(source)?;self.validate()}
    pub fn bind_world_row_tail(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_tail(source)?;self.validate()}
    pub fn return_world_row(&mut self)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.return_row()?;self.validate()}
    pub fn bind_world_row_renderer(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_renderer(source)?;self.validate()}
    pub fn bind_world_row_placement(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_row_placement(source)?;self.validate()}
    pub fn compare_world_tiles(&self,row:&serde_json::Value)->Result<()>{self.world.as_ref().context("World absent")?.compare_tiles(row)}
    pub fn compare_world_shared_graphics(&self,row:&serde_json::Value)->Result<()>{self.validate()?;self.world.as_ref().context("World absent")?.compare_shared_graphics(row)}
    pub fn bind_world_level_geometry(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_level_geometry(source)?;self.validate()}
    pub fn bind_world_geometry_body(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_body(source)?;self.validate()}
    pub fn bind_world_geometry_child(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_child(source)?;self.validate()}
    pub fn bind_world_geometry_corner(&mut self,source:&[u8],setup:&[u8],scene:&WorldScene)->Result<()>{self.validate()?;scene.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_corner(source,setup,&scene.graph)?;self.validate()}
    pub fn bind_world_geometry_segments(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_segments(source,None)?;self.validate()}
    pub fn bind_world_geometry_segments_from_scene(&mut self,source:&[u8],scene:&WorldScene)->Result<()>{self.validate()?;scene.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_segments(source,Some(&scene.graph))?;self.validate()}
    pub fn bind_world_geometry_visible(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_visible(source)?;self.validate()}
    pub fn bind_world_geometry_materials(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_materials(source)?;self.validate()}
    pub fn bind_world_geometry_catalog(&mut self,meta:&[u8],raw:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_catalog(meta,raw)?;self.validate()}
    pub fn bind_world_geometry_diagonal_columns(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_diagonal_columns(source)?;self.validate()}
    pub fn bind_world_geometry_raster(&mut self,source:&[u8],material:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_raster(source,material)?;self.validate()}
    pub fn bind_world_geometry_side(&mut self,source:&[u8],material:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_side(source,material)?;self.validate()}
    pub fn bind_world_geometry_side_alternate(&mut self,input:&[u8],color:&[u8],mask:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_geometry_side_alt(input,color,mask)?;self.validate()}
    pub fn start_world_geometry_last_side_if_pending(&mut self)->Result<bool>{self.validate()?;let bound=self.world.as_mut().context("World absent")?.start_geometry_last_bound_side_if_pending()?;self.validate()?;Ok(bound)}
    pub fn start_world_geometry_bound_side(&mut self)->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.start_geometry_bound_side()?;self.validate()}
    pub fn advance_world_geometry_child(&mut self,sim:&sims_sim::Simulation)->Result<bool>{self.advance_world_geometry_child_controls(sim,None,None)}
    /// Borrow the live shared CPState at native wall mode/tool getters.
    pub fn advance_world_geometry_child_with_controls(&mut self,sim:&sims_sim::Simulation,controls:&sims_vm::interface::state::State)->Result<bool>{self.advance_world_geometry_child_controls(sim,Some(controls),None)}
    /// Borrow current wall endpoints as well as the live shared CPState.
    pub fn advance_world_geometry_child_with_scene(&mut self,sim:&sims_sim::Simulation,controls:&sims_vm::interface::state::State,scene:&WorldScene)->Result<bool>{scene.validate()?;self.advance_world_geometry_child_controls(sim,Some(controls),Some(&scene.graph))}
    fn advance_world_geometry_child_controls(&mut self,sim:&sims_sim::Simulation,controls:Option<&sims_vm::interface::state::State>,graph:Option<&sims_sim::lot_presenter::walls::construction::Graph>)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"geometry child does not own Draw");let p=self.world.as_mut().context("World absent")?.advance_geometry_child(sim,controls,graph)?;self.validate()?;Ok(p)}
    pub fn bind_world_background(&mut self,source:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_background(source)?;self.validate()}
    pub fn compare_world_background(&self,row:&serde_json::Value,scene:&WorldScene)->Result<()>{self.world.as_ref().context("World absent")?.compare_background(row,scene)}
    pub fn bind_world_surfaces(&mut self,source:&[u8],native:&[u8])->Result<()>{self.validate()?;self.world.as_mut().context("World absent")?.bind_surfaces(source,native)?;self.validate()}
    pub fn compare_world_surfaces(&self,row:&serde_json::Value)->Result<()>{self.world.as_ref().context("World absent")?.compare_surfaces(row)}
    pub fn advance_world_paint(&mut self,scene:&mut WorldScene,profiler:&mut Buffer)->Result<bool>{self.validate()?;ensure!(matches!(self.pending(),Some(draw::Request::World{..})),"paint does not own Draw");let progress=self.world.as_mut().context("World absent")?.advance_paint(scene,profiler)?;self.validate()?;Ok(progress)}
    pub fn compare_world_paint(&self,row:&serde_json::Value,scene:&WorldScene,profiler:&Buffer)->Result<()>{self.world.as_ref().context("World absent")?.compare_paint(row,scene,profiler)}
    pub fn acknowledge_world_timestamp(&mut self,time:u64,profiler:&mut Buffer)->Result<()> {
        self.validate()?;ensure!(self.pending()==Some(draw::Request::World{owner:0x09e88960,argument:0}),"World timestamp does not own Draw");
        self.world.as_mut().ok_or_else(||anyhow::anyhow!("World source owner absent"))?.timestamp(time,profiler)?;self.validate()
    }
    pub fn action_image_pending(&self)->Option<&draw::actions::image::Request>{self.actions.as_ref().and_then(action_view::Owner::image_pending)}
    pub fn advance_action_view_for_program(&mut self,controls:&sims_vm::interface::state::State,simulation:&mut sims_sim::Simulation,program:&sims_vm::Program)->Result<Option<draw::actions::Request>> {
        self.validate()?;ensure!(self.pending()==Some(draw::Request::ApplicationView),"record services do not own Draw");
        let result=self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action caller absent"))?.advance_for_program(controls,simulation,program)?;
        self.validate()?;Ok(result)
    }
    pub fn advance_action_view_for_simulation(&mut self,controls:&sims_vm::interface::state::State,simulation:&sims_sim::Simulation)->Result<Option<draw::actions::Request>> {
        self.validate()?;ensure!(self.pending()==Some(draw::Request::ApplicationView),"collection child does not own Draw");
        let result=self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action-window caller absent"))?.advance_for_simulation(controls,simulation)?;
        self.validate()?;Ok(result)
    }
    pub fn action_view_state(&self) -> Option<&draw::actions::State> {
        self.actions.as_ref().map(action_view::Owner::state)
    }
    pub fn advance_action_view(&mut self, controls:&sims_vm::interface::state::State) -> Result<Option<draw::actions::Request>> {
        self.validate()?;
        ensure!(self.pending()==Some(draw::Request::ApplicationView), "action-window child does not own Draw");
        let result=self.actions.as_mut().ok_or_else(||anyhow::anyhow!("source-bound action-window caller absent"))?.advance(controls)?;
        self.validate()?;Ok(result)
    }
    /// Execute only imported, qualified child bodies. Other Draw requests stay
    /// pending, and no enclosing render acknowledgement is manufactured here.
    pub fn service_child(&mut self, profiler: &mut Buffer) -> Result<bool> {
        self.validate()?;
        let Some(request) = self.pending() else {
            return Ok(false);
        };
        if request==draw::Request::ApplicationView && self.actions.as_ref().is_some_and(action_view::Owner::complete) {
            self.frame.acknowledge(request,draw::Reply::Done,&mut self.history,profiler).map_err(anyhow::Error::msg)?;
            self.validate()?;return Ok(true);
        }
        if matches!(request,draw::Request::Framework{..})&&self.framework.is_some(){
            let framework=self.framework.as_mut().unwrap();
            let progress=framework.advance(self.actions.as_ref().ok_or_else(||anyhow::anyhow!("framework shared action owner absent"))?.state())?;
            if framework.complete(){self.frame.acknowledge(request,draw::Reply::Done,&mut self.history,profiler).map_err(anyhow::Error::msg)?;}
            self.validate()?;return Ok(progress);
        }
        if matches!(request,draw::Request::World{..})&&self.world.is_some(){
            let world=self.world.as_mut().unwrap();let progress=world.advance()?;
            if world.complete(){self.frame.acknowledge(request,draw::Reply::Done,&mut self.history,profiler).map_err(anyhow::Error::msg)?;}
            self.validate()?;return Ok(progress);
        }
        let Some(children) = &mut self.children else {
            return Ok(false);
        };
        if !children.supports(request) {
            return Ok(false);
        }
        let mut frame = self.frame.clone();
        frame
            .acknowledge(request, draw::Reply::Done, &mut self.history, profiler)
            .map_err(anyhow::Error::msg)?;
        children.apply(request)?;
        self.frame = frame;
        self.validate()?;
        Ok(true)
    }
    pub fn view_frame_word(&self) -> Option<u32> {
        self.children
            .as_ref()
            .map(children::Owners::view_frame_word)
    }
    pub fn advance(&mut self) -> Result<Option<draw::Request>> {
        self.validate()?;
        self.frame
            .advance(&mut self.fields)
            .map_err(anyhow::Error::msg)
    }
    pub fn acknowledge(
        &mut self,
        request: draw::Request,
        reply: draw::Reply,
        profiler: &mut Buffer,
    ) -> Result<()> {
        self.validate()?;
        ensure!(!matches!(request,draw::Request::World{..})||self.world.is_none(),"source-bound World callback must execute retained children");
        ensure!(!matches!(request,draw::Request::Framework{..})||self.framework.is_none(),"source-bound framework caller must execute retained region services");
        ensure!(request!=draw::Request::ApplicationView || self.actions.is_none(), "source-bound action-window caller must complete its retained children");
        ensure!(
            !self
                .children
                .as_ref()
                .is_some_and(|children| children.supports(request)),
            "imported draw child must execute through its retained owner"
        );
        self.frame
            .acknowledge(request, reply, &mut self.history, profiler)
            .map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    #[test]
    #[ignore = "private source-bound draw services and original live headers"]
    fn observed_view_clear_idle_unload_and_noop_end_survive_cold_cuts() -> Result<()> {
        use std::io::Read;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read(root.join(
                "fixtures/private/goth-render-parent-20261031-first/render-source-second.json",
            ))?;
        let children = std::fs::read(root.join(
            "fixtures/private/goth-draw-services-20261031-first/services-source-first.json",
        ))?;
        let encoded = std::fs::read(root.join("fixtures/private/goth-draw-children-witness-20261031-second/draw-children-trace.jsonl.gz"))?;
        ensure!(
            sims_fixtures::digest(&encoded)
                == "2eae5ee86eeebfe349c8fdd8c6e0cd8d9035d5b488fc82747e6f0aa1dcaaa8ff",
            "original draw-child trace differs"
        );
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(encoded.as_slice()).read_to_end(&mut raw)?;
        let rows = String::from_utf8(raw)?
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let at = |pc: u64| {
            rows.iter()
                .find(|r| r["kind"] == "same_stop_outer_readback" && r["pc"] == pc)
                .context("original draw-child point absent")
        };
        let header = |pc: u64, field: &str| -> Result<Vec<u8>> {
            super::super::completed_animations::bytes(
                at(pc)?[field]["raw"]
                    .as_str()
                    .context("child header absent")?,
            )
        };
        let mut state = import(&source)?;
        state.bind_child_source(&children)?;
        assert_eq!(
            state.children.as_ref().unwrap().headers().0,
            header(0x4d4ffd, "view_child")?
        );
        assert_eq!(
            state.children.as_ref().unwrap().headers().1,
            header(0x4d1b93, "unload_child")?
        );
        let mut profiler = Buffer::from_captured(vec![], 32).map_err(anyhow::Error::msg)?;
        let mut served = Vec::new();
        let mut supplied = Vec::new();
        while let Some(request) = state.advance()? {
            let frozen = postcard::to_allocvec(&(&state, &profiler))?;
            (state, profiler) = postcard::from_bytes(&frozen)?;
            assert_eq!(state.advance()?, Some(request));
            assert_eq!(postcard::to_allocvec(&(&state, &profiler))?, frozen);
            if state.children.as_ref().unwrap().supports(request) {
                assert!(
                    state
                        .acknowledge(request, draw::Reply::Done, &mut profiler)
                        .is_err()
                );
                assert_eq!(postcard::to_allocvec(&(&state, &profiler))?, frozen);
                assert!(state.service_child(&mut profiler)?);
                served.push(request);
                assert_eq!(state.view_frame_word(), Some(0));
                assert_eq!(
                    state.children.as_ref().unwrap().headers().0,
                    header(0x4d1b8c, "view_child")?
                );
                assert_eq!(
                    state.children.as_ref().unwrap().headers().1,
                    header(0x4d1ba2, "unload_child")?
                );
            } else {
                assert!(!state.service_child(&mut profiler)?);
                let reply = match request {
                    draw::Request::Timestamp { .. } => draw::Reply::Timestamp(123),
                    draw::Request::FpsClock => draw::Reply::Clock(
                        at(0x473077)?["registers"]["eax"]
                            .as_u64()
                            .context("FPS sample absent")?
                            .try_into()?,
                    ),
                    // Explicit supplied callbacks test this owner's cursor;
                    // they do not claim execution of the missing renderers.
                    draw::Request::ApplicationView
                    | draw::Request::Framework { .. }
                    | draw::Request::World { .. } => draw::Reply::Done,
                    _ => anyhow::bail!("unexpected source-bound child"),
                };
                supplied.push(request);
                state.acknowledge(request, reply, &mut profiler)?;
            }
        }
        assert!(state.complete());
        assert_eq!(
            served,
            [
                draw::Request::ViewBegin { owner: 0x0ac04dd8 },
                draw::Request::Unload { owner: 0x0a9ab808 },
                draw::Request::ViewEnd { owner: 0x0ac04dd8 }
            ]
        );
        assert_eq!(supplied.len(), 6);
        let mut bad = serde_json::to_value(&state)?;
        bad["children"]["view"][4] = 1.into();
        assert!(serde_json::from_value::<State>(bad)?.validate().is_err());
        let mut bad = serde_json::to_value(&state)?;
        bad["children"]["view"][0x38] = 1.into();
        assert!(serde_json::from_value::<State>(bad)?.validate().is_err());
        Ok(())
    }
    #[test]
    #[ignore = "private frozen Draw owner and actual original C8 sample"]
    fn actual_sample_and_original_slots_survive_binary_suspension() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read(root.join(
                "fixtures/private/goth-render-parent-20261031-first/render-source-second.json",
            ))?;
        let proof_raw = std::fs::read(root.join(
            "fixtures/private/goth-render-parent-20261031-first/verified-render-first.json",
        ))?;
        ensure!(
            sims_fixtures::digest(&proof_raw)
                == "4d4987becfd4c2fee454bcedb592877328996a1236296ceac9729c1779350cb8",
            "actual Draw verification differs"
        );
        let proof: serde_json::Value = serde_json::from_slice(&proof_raw)?;
        let mut state = import(&source)?;
        assert_eq!(
            serde_json::json!(state.history.timestamps()),
            proof["fps_before"]["timestamps"]
        );
        let mut profiler = Buffer::from_captured(vec![], 32).map_err(anyhow::Error::msg)?;
        let marker = state.advance()?.context("Draw marker absent")?;
        assert_eq!(marker, draw::Request::Timestamp { name: 0x6c7c7c });
        // This test samples no original RDTSC; the marker value is explicitly
        // supplied. Only the actual FPS sample and all history slots are compared.
        state.acknowledge(marker, draw::Reply::Timestamp(123), &mut profiler)?;
        let request = state.advance()?.context("FPS clock absent")?;
        assert_eq!(request, draw::Request::FpsClock);
        let saved = postcard::to_allocvec(&(&state, &profiler))?;
        let (mut restored, mut restored_profiler): (State, Buffer) = postcard::from_bytes(&saved)?;
        assert_eq!(restored.advance()?, Some(request));
        assert_eq!(
            postcard::to_allocvec(&(&restored, &restored_profiler))?,
            saved
        );
        let reply = draw::Reply::Clock(
            proof["clock"]
                .as_u64()
                .context("FPS sample absent")?
                .try_into()?,
        );
        state.acknowledge(request, reply, &mut profiler)?;
        restored.acknowledge(request, reply, &mut restored_profiler)?;
        assert_eq!(state, restored);
        assert_eq!(profiler, restored_profiler);
        assert_eq!(
            serde_json::json!(state.history.timestamps()),
            proof["fps_after"]["timestamps"]
        );
        assert_eq!(
            serde_json::json!(state.history.fields()),
            proof["fps_after"]["fields"]
        );
        assert_eq!(
            state.advance()?,
            Some(draw::Request::ViewBegin { owner: 0x0ac04dd8 })
        );
        assert!(!state.complete());
        Ok(())
    }
}
