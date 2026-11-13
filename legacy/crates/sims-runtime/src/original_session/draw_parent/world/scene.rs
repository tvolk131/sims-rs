//! 4a8895: shared room rebuild, wall history, cutaway and dirty regions.
//! Immutable OBJD kind words are source inputs, not an empty substitute Program.
use super::*;
use anyhow::Context as _;
use sims_sim::lot_presenter::walls::{self, construction::Graph, cutaway};
use sims_vm::scheduler::clock::draw::regions;
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Ready,
    Rooms,
    History,
    Apply,
    Invalidate,
    Region,
    Clear,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Rooms,
    Compose,
    RebuildWalls,
    WallMode,
    Regions(regions::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    view: u32,
    pub graph: Graph,
    history: cutaway::History,
    pub regions: regions::List,
    kinds: BTreeMap<String, BTreeMap<u16, i16>>,
    stage: Stage,
    dirty: [u32; 4],
    compose: u8,
    all_walls: u8,
    override_walls: u8,
    viewport: [i32; 4],
    region_enabled: u32,
    cursor: Option<cutaway::Cursor>,
    region: Option<regions::Frame>,
    pub invalidations: Vec<[i8; 3]>,
}
fn segments(raw: &[u8]) -> Vec<walls::Segment> {
    raw.chunks_exact(6)
        .map(|b| walls::Segment {
            flags: b[0],
            style: b[1],
            patterns: [
                u16::from_le_bytes(b[2..4].try_into().unwrap()),
                u16::from_le_bytes(b[4..6].try_into().unwrap()),
            ],
        })
        .collect()
}
impl State {
    pub fn import(encoded: &[u8], simulation: &sims_sim::Simulation) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(
            source == "b331ab0d8ddf88c909eae825c058292cfcf3da488c134430efc7c9bedbd204bb",
            "scene source differs"
        );
        let s: serde_json::Value = serde_json::from_slice(encoded)?;
        let m = Memory::new(&s)?;
        let v = &s["viewer"];
        let vmemory = Memory::new(v)?;
        let raw = vmemory.read(
            v["world"].as_u64().context("scene lot absent")? as u32,
            0x88,
        )?;
        let graph = crate::original_session::completed_viewer::graph(
            &vmemory,
            v,
            &serde_json::json!({"rooms":s["rooms"]}),
            &raw,
        )?;
        let view = v["viewer"].as_u64().context("scene view absent")? as u32;
        let vr = m.read(view, 0x4c0)?;
        let manager = m.read(word(&raw, 0x54), 0x80)?;
        let begin = word(&vr, 0x88);
        let capacity = word(&vr, 0x90);
        let regions = regions::List {
            header: vr[0x88..0xa0].to_vec(),
            backing: m.read(begin, capacity.wrapping_sub(begin) as usize)?,
        };
        regions.validate().map_err(anyhow::Error::msg)?;
        let mut kinds: BTreeMap<String, BTreeMap<u16, i16>> = BTreeMap::new();
        for row in s["kinds"]
            .as_array()
            .context("scene kind definitions absent")?
        {
            let id = row["object"]
                .as_i64()
                .context("scene object identity absent")? as i16;
            let Some(definition) = simulation
                .vm
                .world
                .objects
                .get(&id)
                .and_then(|o| o.definition.as_ref())
            else {
                continue;
            };
            ensure!(
                row["chunk"] == definition.object,
                "scene definition identity differs"
            );
            let bytes = m.read(
                row["definition"]
                    .as_u64()
                    .context("native definition absent")? as u32,
                0xd8,
            )?;
            let kind = i16::from_le_bytes(bytes[18..20].try_into()?);
            ensure!(row["kind"] == kind, "scene native kind differs");
            ensure!(
                kinds
                    .entry(definition.source.clone())
                    .or_default()
                    .insert(definition.object, kind)
                    .is_none_or(|v| v == kind),
                "scene kind aliases disagree"
            );
        }
        let history = cutaway::History {
            segments: s["previous_segments"]
                .as_array()
                .context("scene previous vectors absent")?
                .iter()
                .map(|r| -> Result<_> {
                    Ok(segments(
                        &crate::original_session::completed_animations::bytes(
                            r["previous_raw"]
                                .as_str()
                                .context("scene previous bytes absent")?,
                        )?,
                    ))
                })
                .collect::<Result<_>>()?,
        };
        let state = Self {
            source,
            view,
            graph,
            history,
            regions,
            kinds,
            stage: Stage::Ready,
            dirty: std::array::from_fn(|i| word(&manager, 0x38 + 4 * i)),
            compose: vr[0x200],
            all_walls: vr[0x48],
            override_walls: vr[0x51],
            viewport: std::array::from_fn(|i| word(&vr, 0x28 + 4 * i) as i32),
            region_enabled: word(&m.read(0x6d61b0, 4)?, 0),
            cursor: None,
            region: None,
            invalidations: vec![],
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == "b331ab0d8ddf88c909eae825c058292cfcf3da488c134430efc7c9bedbd204bb"
                && self.view == 0x09e88960,
            "scene owner differs"
        );
        self.graph.validate().map_err(anyhow::Error::msg)?;
        self.history
            .validate(&self.graph)
            .map_err(anyhow::Error::msg)?;
        self.regions.validate().map_err(anyhow::Error::msg)?;
        if let Some(c) = &self.cursor {
            c.validate(&self.graph, &self.history)
                .map_err(anyhow::Error::msg)?;
        }
        ensure!(
            self.region.is_some() == (self.stage == Stage::Region),
            "scene region child lifetime differs"
        );
        Ok(())
    }
    pub fn validate_view(&self, view:&composition::View)->Result<()> {
        self.validate()?;view.validate()?;
        ensure!(self.view==view.owner(),"scene and composition view identities differ");
        Ok(())
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Rooms => Some(Request::Rooms),
            Stage::History if self.compose != 0 => Some(Request::Compose),
            Stage::History if self.dirty != [0; 4] => Some(Request::RebuildWalls),
            Stage::Apply if self.all_walls != 0 || self.override_walls != 0 => {
                Some(Request::WallMode)
            }
            Stage::Region => self
                .region
                .as_ref()
                .and_then(|f| f.pending().cloned())
                .map(Request::Regions),
            _ => None,
        }
    }
    pub fn after_rooms(&self) -> bool {
        self.stage == Stage::History
    }
    pub fn at_entry(&self)->bool {self.stage==Stage::Ready}
    pub fn after_history(&self) -> bool {
        self.stage == Stage::Apply
    }
    pub fn before_clear(&self) -> bool {
        self.stage == Stage::Clear
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub(super) fn advance(
        &mut self,
        view: &mut composition::View,
        simulation: &mut sims_sim::Simulation,
    ) -> Result<bool> {
        self.validate_view(view)?;
        match self.stage {
            Stage::Ready => self.stage = Stage::Rooms,
            Stage::Rooms => {
                let lot = simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("scene architecture absent")?;
                let native = simulation
                    .native_lot
                    .as_ref()
                    .context("scene physical owner absent")?;
                let period = simulation
                    .household
                    .as_ref()
                    .context("scene household clock absent")?
                    .clock
                    .period;
                simulation
                    .room_environment
                    .as_mut()
                    .context("scene room manager absent")?
                    .manager
                    .update_with(
                        &mut simulation.vm,
                        lot,
                        &native.placement.links,
                        period,
                        |world, id| {
                            sims_sim::lot_environment::objects::Input::read_with(world, id, |d| {
                                self.kinds
                                    .get(&d.source)
                                    .and_then(|m| m.get(&d.object))
                                    .copied()
                                    .ok_or_else(|| "scene native OBJD kind is unobserved".into())
                            })
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                self.stage = Stage::History;
            }
            Stage::History => {
                if self.compose != 0 || self.dirty != [0; 4] {
                    return Ok(false);
                }
                self.history =
                    cutaway::History::capture(&self.graph).map_err(anyhow::Error::msg)?;
                self.stage = Stage::Apply;
            }
            Stage::Apply => {
                if view.visibility.dirty == 0 {
                    self.stage = Stage::Clear;
                } else {
                    if self.all_walls != 0
                        || self.override_walls != 0
                        || view.scene_level() != 1
                        || view.visibility.enabled == 0
                    {
                        return Ok(false);
                    }
                    self.cursor = Some(
                        cutaway::Cursor::begin(
                            &mut self.graph,
                            &self.history,
                            &view.visibility.mask,
                            1,
                        )
                        .map_err(anyhow::Error::msg)?,
                    );
                    self.stage = Stage::Invalidate;
                }
            }
            Stage::Invalidate => {
                let cursor = self.cursor.as_mut().context("scene wall cursor absent")?;
                if let Some(tile) = cursor
                    .advance(&self.graph, &self.history)
                    .map_err(anyhow::Error::msg)?
                {
                    self.invalidations.push(tile);
                    if let Some(rect) = view.scene_rectangle(simulation, tile, self.viewport)? {
                        self.region = Some(regions::Frame::new(rect));
                        self.stage = Stage::Region;
                    } else {
                        cursor
                            .acknowledge(&self.graph, &self.history, tile)
                            .map_err(anyhow::Error::msg)?;
                    }
                } else {
                    ensure!(cursor.complete(), "scene cutaway returned early");
                    self.stage = Stage::Clear;
                }
            }
            Stage::Region => {
                let frame = self.region.as_mut().unwrap();
                if !advance_region_list(&mut self.regions,self.region_enabled,frame)? {return Ok(false);}
                if frame.complete() {
                    let tile = *self
                        .invalidations
                        .last()
                        .context("scene region tile absent")?;
                    self.cursor
                        .as_mut()
                        .unwrap()
                        .acknowledge(&self.graph, &self.history, tile)
                        .map_err(anyhow::Error::msg)?;
                    self.region = None;
                    self.stage = Stage::Invalidate;
                }
            }
            Stage::Clear => {
                view.visibility.dirty = 0;
                self.stage = Stage::Complete;
            }
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn scene(
        &mut self,
        scene: &mut State,
        view: &mut composition::View,
        simulation: &mut sims_sim::Simulation,
    ) -> Result<bool> {
        ensure!(
            self.phase == Phase::Prepare && self.owner == scene.view,
            "scene does not own World caller"
        );
        let progress = scene.advance(view, simulation)?;
        if scene.complete() {
            self.phase = Phase::Indicators;
        }
        self.validate()?;
        Ok(progress)
    }
}

impl State {
    /// Read-only reference comparison. Expected rows never replace live owners.
    pub fn compare_native(
        &self,
        encoded: &[u8],
        pc: u32,
        simulation: &sims_sim::Simulation,
        view: &composition::View,
    ) -> Result<()> {
        use std::io::Read;
        let digest = sims_fixtures::digest(encoded);
        ensure!(
            digest == "355ba674b7db9bac180f641b9a21549733d64e3699f564054de56530caba1bf8",
            "scene witness differs"
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
            .context("native scene stop absent")?;
        let scene = &row["scene"];
        let m = Memory::new(row)?;
        let decode = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("scene raw bytes absent")?,
            )
        };
        let vr = decode(&scene["raw"])?;
        ensure!(
            scene["view"] == self.view && row["tick"] == simulation.vm.tick,
            "scene identity/epoch differs"
        );
        ensure!(
            vr[0x4e] == view.visibility.dirty && vr[0x200] == self.compose,
            "scene dirty flag publication differs"
        );
        let native_mask: Vec<u64> = vr[0x208..0x408]
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        ensure!(
            view.visibility.mask.rows == native_mask,
            "scene mask changed"
        );
        ensure!(
            self.regions.header == vr[0x88..0xa0]
                && self.regions.backing == decode(&scene["regions_raw"])?,
            "scene dirty region allocation differs"
        );
        let mut input = scene.clone();
        input["world_raw"] = scene["lot_raw"].clone();
        input["wall_manager_raw"] = serde_json::json!(
            decode(&scene["wall_manager_raw"])?[..24]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let source = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../fixtures/private/goth-world-scene-20261031-first/scene-source-second.json",
        ))?;
        ensure!(
            sims_fixtures::digest(&source) == self.source,
            "scene reference source differs"
        );
        let source: serde_json::Value = serde_json::from_slice(&source)?;
        let room_ids: BTreeMap<u32, u16> = source["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["pointer"].as_u64().unwrap() as u32,
                    r["id"].as_u64().unwrap() as u16,
                )
            })
            .collect();
        let mut previous = vec![];
        for floor in input["wall_graph"]
            .as_array_mut()
            .context("scene native graph absent")?
        {
            for edge in floor["edge_records"].as_array_mut().unwrap() {
                let raw = decode(&edge["raw"])?;
                previous.push(segments(&decode(&edge["previous_raw"])?));
                edge["raw"] = serde_json::json!(
                    raw[..60]
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                );
                edge["room_ids"] = serde_json::to_value(
                    [0x28, 0x2c].map(|o| room_ids.get(&word(&raw, o)).copied()),
                )?;
            }
        }
        let expected = crate::original_session::completed_viewer::graph(
            &m,
            &input,
            &serde_json::json!({"rooms":source["rooms"]}),
            &decode(&scene["lot_raw"])?,
        )?;
        ensure!(
            self.graph == expected,
            "scene native wall graph differs at {pc:x}"
        );
        ensure!(
            self.history.segments == previous,
            "scene native wall history differs"
        );
        if let Some(snapshot) = row.get("simulation_snapshot") {
            let fields = crate::original_session::completed_fields::compare(
                &serde_json::json!({"snapshot":snapshot}),
                simulation,
            )?;
            ensure!(
                fields["differences"] == 0,
                "scene native scalar/stack difference: {}",
                fields["first_differences"]
            );
            for object in snapshot["objects"].as_array().unwrap() {
                let id = object["id"].as_i64().unwrap() as i16;
                let d = simulation.vm.world.objects[&id]
                    .definition
                    .as_ref()
                    .context("scene live definition absent")?;
                let raw = decode(&object["definition"])?;
                ensure!(
                    self.kinds.get(&d.source).and_then(|m| m.get(&d.object))
                        == Some(&i16::from_le_bytes(raw[18..20].try_into()?)),
                    "scene kind source differs for {id}"
                );
            }
        }
        if let Some(env) = row.get("room_environment") {
            let mut env = env.clone();
            let provenance = serde_json::json!({"image_sha256":"a4d9f7134f52a186685961800608af72c23036ba62637eb9a99d75d4c5b1b321","observation_sha256":digest});
            env["provenance"] = provenance.clone();
            let geometry = crate::original_session::completed_lot::Imported {
                lot: simulation
                    .architecture
                    .as_ref()
                    .and_then(|a| a.original.as_ref())
                    .context("scene reference geometry absent")?
                    .clone(),
                source: serde_json::json!({"provenance":provenance,"rooms":env["rooms"],"manager_raw":env["manager_raw"],"world_raw":env["world_raw"]}),
                comparison: serde_json::Value::Null,
                complete_native_lot_admitted: false,
            };
            let expected = crate::original_session::completed_lot::environment::import(
                &geometry,
                &env,
                &simulation.vm.world,
            )?;
            let actual = &simulation
                .room_environment
                .as_ref()
                .context("scene live environment absent")?
                .manager;
            // Native manager+34 belongs to World.refresh. The Aggregate value
            // also retains the last arithmetic result, which can precede later
            // invalidations. Compare the native cell through its live owner.
            let mut actual_value=serde_json::to_value(actual)?;
            actual_value["aggregate"]["dirty_floors"]=serde_json::to_value(simulation.vm.world.refresh.as_ref().context("scene refresh owner absent")?.dirty_categories)?;
            ensure!(
                actual_value == serde_json::to_value(&expected.manager)?,
                "scene native environment differs at {pc:x}: {}",
                format!(
                    "expected aggregate {:?}, actual {:?}; room4 expected {:?}, actual {:?}",
                    expected.manager.aggregate,
                    actual.aggregate,
                    expected.manager.rooms.get(&4),
                    actual.rooms.get(&4)
                )
            );
        }
        Ok(())
    }
}

fn advance_region_list(list:&mut regions::List,enabled:u32,frame:&mut regions::Frame)->Result<bool>{
    if let Some(request)=frame.pending().cloned(){
        match request {
            regions::Request::Append{rectangle}=>{
                if !list.append(rectangle).map_err(anyhow::Error::msg)?{return Ok(false);}
                frame.acknowledge(list,&request).map_err(anyhow::Error::msg)?;
            }
            regions::Request::Coalesce=>return Ok(false),
        }
    }else{frame.advance(list,enabled).map_err(anyhow::Error::msg)?;}
    Ok(true)
}
impl State {
    pub(super) fn viewport(&self)->[i32;4]{self.viewport}
    pub(super) fn advance_region(&mut self,frame:&mut regions::Frame)->Result<bool>{advance_region_list(&mut self.regions,self.region_enabled,frame)}
}
