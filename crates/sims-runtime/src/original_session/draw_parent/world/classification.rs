//! Retained4464db ->4aacb9. Shared VM algorithms borrow live state; captured
//! definitions and physical allocations are baseline inputs, never endpoints.
use super::*;
use anyhow::Context as _;
use sims_sim::lot_presenter::{Definitions, Inputs, View};
use sims_vm::scheduler::clock::draw::regions;
use sims_vm::{
    Definition, ObjectId, indicator_refresh,
    presenter::{Classification, Lists},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SourceDefinitions(BTreeMap<String, BTreeMap<u16, Vec<i16>>>);
impl Definitions for SourceDefinitions {
    fn words(&self, d: &Definition) -> std::result::Result<&[i16], String> {
        self.0
            .get(&d.source)
            .and_then(|m| m.get(&d.object))
            .map(Vec::as_slice)
            .ok_or_else(|| "classification definition storage is unobserved".into())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Vector {
    header: Vec<u8>,
    backing: Vec<u8>,
}
impl Vector {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.header.len() == 12,
            "classification vector header absent"
        );
        let [a, z, c] = [0, 4, 8].map(|i| word(&self.header, i));
        ensure!(
            a <= z
                && z <= c
                && (z - a) % 4 == 0
                && (c - a) % 4 == 0
                && self.backing.len() == (c - a) as usize
                && c - a <= 65536,
            "classification vector extent differs"
        );
        Ok(())
    }
    fn ids(&self, pointers: &BTreeMap<ObjectId, u32>) -> Result<Vec<ObjectId>> {
        self.validate()?;
        let reverse: BTreeMap<_, _> = pointers.iter().map(|(&k, &v)| (v, k)).collect();
        self.backing[..(word(&self.header, 4) - word(&self.header, 0)) as usize]
            .chunks_exact(4)
            .map(|b| {
                reverse
                    .get(&word(b, 0))
                    .copied()
                    .context("classification vector references unknown native object")
            })
            .collect()
    }
    fn publish(&mut self, ids: &[ObjectId], pointers: &BTreeMap<ObjectId, u32>) -> Result<()> {
        self.validate()?;
        ensure!(
            ids.len() * 4 <= self.backing.len(),
            "classification list requires native allocation child"
        );
        for (i, id) in ids.iter().enumerate() {
            self.backing[4 * i..4 * i + 4].copy_from_slice(
                &pointers
                    .get(id)
                    .context("classification pointer identity absent")?
                    .to_le_bytes(),
            );
        }
        let end = word(&self.header, 0) + ids.len() as u32 * 4;
        self.header[4..8].copy_from_slice(&end.to_le_bytes());
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Drain,
    Order,
    Classify,
    Region,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Indicator(indicator_refresh::Request),
    Invalidate(ObjectId),
    Region(regions::Request),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    view: u32,
    module: u32,
    definitions: SourceDefinitions,
    pointers: BTreeMap<ObjectId, u32>,
    rectangles: BTreeMap<ObjectId, [[i32; 4]; 2]>,
    force_dynamic: bool,
    #[serde(default)]
    rectangle_flags:Option<u32>,
    stage: Stage,
    drain: indicator_refresh::Drain,
    classify: Classification,
    pub lists: Lists,
    vectors: [Vector; 3],
    cache: Vector,
    region: Option<regions::Frame>,
    pub invalidations: Vec<ObjectId>,
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("classification bytes absent")?,
    )
}
impl State {
    pub fn import(encoded: &[u8], simulation: &sims_sim::Simulation) -> Result<Self> {
        let source = sims_fixtures::digest(encoded);
        ensure!(
            source == "c92c35dd68417da337a4ac34b042c6e5c77105f257d412e5ede8093a63c04d1e",
            "classification baseline source differs"
        );
        let value: serde_json::Value = serde_json::from_slice(encoded)?;
        let m = Memory::new(&value)?;
        let c = &value["classification"];
        let process = c["process"]
            .as_u64()
            .context("classification process absent")? as u32;
        ensure!(
            word(&m.read(0x6e48bc, 4)?, 0) == process
                && decode(&c["force_dynamic"])? == m.read(process + 0x5c, 1)?,
            "classification force-dynamic owner differs"
        );
        ensure!(
            value["tick"] == 89073
                && value["executes_target_code"] == false
                && value["native_parity_credit"] == 0,
            "classification source epoch differs"
        );
        let view = c["view"].as_u64().context("classification view absent")? as u32;
        let raw = m.read(view, 0x4c0)?;
        ensure!(
            raw == decode(&c["view_raw"])?,
            "classification view alias differs"
        );
        let module = c["module"]
            .as_u64()
            .context("classification module absent")? as u32;
        let mr = m.read(module, 0xd0)?;
        ensure!(
            mr == decode(&c["module_raw"])?,
            "classification registry alias differs"
        );
        let table = m.read(word(&raw, 0), 0x60)?;
        ensure!(
            word(&table, 0x1c) == 0x4aaf48,
            "classification invalidation virtual differs"
        );
        let mut pointers = BTreeMap::new();
        let mut rectangles = BTreeMap::new();
        for o in c["objects"]
            .as_array()
            .context("classification object identities absent")?
        {
            let id = o["id"]
                .as_i64()
                .context("classification object id absent")? as i16;
            let pointer = o["pointer"]
                .as_u64()
                .context("classification pointer absent")? as u32;
            let body = m.read(pointer, 0x18c)?;
            ensure!(
                body == decode(&o["raw"])?
                    && i16::from_le_bytes(body[0xf0..0xf2].try_into()?) == id,
                "classification object identity differs"
            );
            ensure!(
                pointers.insert(id, pointer).is_none(),
                "duplicate classification object identity"
            );
            rectangles.insert(
                id,
                std::array::from_fn(|n| {
                    std::array::from_fn(|i| word(&body, 0x160 + n * 16 + i * 4) as i32)
                }),
            );
        }
        let mut definitions = SourceDefinitions(BTreeMap::new());
        for d in value["kinds"]
            .as_array()
            .context("classification definitions absent")?
        {
            let id = d["object"]
                .as_i64()
                .context("classification definition object absent")? as i16;
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
                d["chunk"] == definition.object,
                "classification source/chunk alias differs"
            );
            let body = m.read(
                d["definition"]
                    .as_u64()
                    .context("classification native definition absent")? as u32,
                0xd8,
            )?;
            let words: Vec<_> = body
                .chunks_exact(2)
                .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
                .collect();
            if let Some(old) = definitions
                .0
                .entry(definition.source.clone())
                .or_default()
                .insert(definition.object, words.clone())
            {
                ensure!(old == words, "classification definition aliases disagree");
            }
        }
        let vector = |header: &[u8], v: &serde_json::Value| -> Result<Vector> {
            let v = Vector {
                header: header.to_vec(),
                backing: decode(&v["raw"])?,
            };
            v.validate()?;
            Ok(v)
        };
        let vectors = [
            vector(&raw[0x1c8..0x1d4], &c["vectors"][0])?,
            vector(&raw[0x1d4..0x1e0], &c["vectors"][1])?,
            vector(&raw[0x1e0..0x1ec], &c["vectors"][2])?,
        ];
        let lists = Lists {
            dynamic: vectors[0].ids(&pointers)?,
            ordinary: vectors[1].ids(&pointers)?,
            special: vectors[2].ids(&pointers)?,
        };
        let s = Self {
            source,
            view,
            module,
            definitions,
            pointers,
            rectangles,
            force_dynamic: decode(&value["viewer"]["owner_raw"])?[0x5c] != 0,
            rectangle_flags:Some(word(&m.read(0x6d61b4,4)?,0)),
            stage: Stage::Drain,
            drain: Default::default(),
            classify: Default::default(),
            lists,
            vectors,
            cache: vector(&mr[0x4c..0x58], &c["cache"])?,
            region: None,
            invalidations: vec![],
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == "c92c35dd68417da337a4ac34b042c6e5c77105f257d412e5ede8093a63c04d1e"
                && self.view == 0x09e88960
                && self.module == 0x09e267a0,
            "classification owner differs"
        );
        self.drain.validate().map_err(anyhow::Error::msg)?;
        self.classify.validate().map_err(anyhow::Error::msg)?;
        for v in &self.vectors {
            v.validate()?;
        }
        self.cache.validate()?;
        ensure!(
            self.region.is_some() == (self.stage == Stage::Region),
            "classification region child lifetime differs"
        );
        ensure!(
            self.stage != Stage::Complete || self.classify.complete(),
            "classification caller skipped child"
        );
        Ok(())
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::Drain && self.drain == indicator_refresh::Drain::default()
    }
    pub fn after_drain(&self) -> bool {
        self.stage != Stage::Drain
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    /// Join an earlier executed C8 owner to renderer checkpoints that retained
    /// only its baseline cache component. Never replace changed cache values,
    /// identities or an in-flight classification continuation.
    pub fn join_completed(&mut self,mut executed:Self)->Result<()>{
        self.validate()?;executed.validate()?;
        ensure!(self.at_entry()&&executed.complete(),"classification handoff must join baseline component to executed completion");
        ensure!(self.source==executed.source&&self.view==executed.view&&self.module==executed.module
            &&self.definitions==executed.definitions&&self.pointers==executed.pointers
            &&self.rectangles==executed.rectangles&&self.force_dynamic==executed.force_dynamic,
            "classification handoff changes current identities or rectangle caches");
        match(self.rectangle_flags,executed.rectangle_flags){
            (Some(a),Some(b))=>ensure!(a==b,"classification rectangle-cache policy differs"),
            // Older saved owners predate this field. Both owners bind the same
            // immutable B source containing the actual policy word.
            (Some(a),None)=>executed.rectangle_flags=Some(a),
            _=>{},
        }
        executed.validate()?;*self=executed;Ok(())
    }
    pub(super) fn draw_lists(&self)->Result<[super::paint::model::DrawList;2]>{
        self.validate()?;ensure!(self.complete(),"drawable lists requested before classification completed");
        let make=|index:usize,ids:&[ObjectId]|->Result<super::paint::model::DrawList>{
            let vector=&self.vectors[index];ensure!(vector.ids(&self.pointers)?==ids,"drawable vector/list alias differs");
            let entries=ids.iter().map(|id|Ok((*id,*self.pointers.get(id).context("drawable native identity absent")?))).collect::<Result<Vec<_>>>()?;
            Ok(super::paint::model::DrawList{header:std::array::from_fn(|i|word(&vector.header,i*4)),entries})
        };Ok([make(1,&self.lists.ordinary)?,make(2,&self.lists.special)?])
    }
    pub fn pending(&self) -> Option<Request> {
        match self.stage {
            Stage::Drain => self.drain.pending().map(Request::Indicator),
            Stage::Classify => self.classify.pending().map(Request::Invalidate),
            Stage::Region => self
                .region
                .as_ref()
                .and_then(|r| r.pending().cloned())
                .map(Request::Region),
            _ => None,
        }
    }
    fn sync_vectors(&mut self) -> Result<()> {
        for (v, ids) in self.vectors.iter_mut().zip([
            &self.lists.dynamic,
            &self.lists.ordinary,
            &self.lists.special,
        ]) {
            v.publish(ids, &self.pointers)?;
        }
        Ok(())
    }
    fn advance(
        &mut self,
        simulation: &mut sims_sim::Simulation,
        ordinary: &sims_content::graphic_loading::Saved,
        scene: &mut scene::State,
        view: &composition::View,
    ) -> Result<bool> {
        self.validate()?;
        scene.validate_view(view)?;
        match self.stage {
            Stage::Drain => {
                if self
                    .drain
                    .advance(&mut simulation.vm.world.indicator_queues)
                    .map_err(anyhow::Error::msg)?
                    .is_some()
                {
                    return Ok(false);
                }
                self.stage = Stage::Order;
            }
            Stage::Order => {
                let inputs = Inputs {
                    force_dynamic: self.force_dynamic,
                    graphic_ready: ordinary
                        .presenter_readiness_with(&simulation.vm, &self.definitions)?,
                    keep_dynamic: BTreeMap::new(),
                };
                let links = &simulation
                    .native_lot
                    .as_ref()
                    .context("classification physical links absent")?
                    .placement
                    .links;
                let world = &mut simulation.vm.world;
                let mut cache = world
                    .registry_order
                    .take()
                    .context("classification registry order absent")?;
                let result = (|| -> Result<()> {
                    cache
                        .get(
                            &View::with_definitions(world, &self.definitions, links, &inputs, &[])
                                .map_err(anyhow::Error::msg)?,
                        )
                        .map_err(anyhow::Error::msg)?;
                    Ok(())
                })();
                world.registry_order = Some(cache);
                result?;
                self.cache.publish(
                    &world.registry_order.as_ref().unwrap().objects,
                    &self.pointers,
                )?;
                self.stage = Stage::Classify;
            }
            Stage::Classify => {
                if let Some(id) = self.classify.pending() {
                    let [a, b] = self
                        .rectangles
                        .get(&id)
                        .copied()
                        .context("classification cached rectangle owner absent")?;
                    let rect = [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ];
                    if let Some(rect) = view.object_rectangle(rect, scene.viewport()) {
                        self.region = Some(regions::Frame::new(rect));
                        self.stage = Stage::Region;
                    } else {
                        simulation
                            .vm
                            .world
                            .refresh
                            .as_mut()
                            .context("classification draw owner absent")?
                            .invalidated
                            .insert(id);
                        self.invalidations.push(id);
                        self.classify.acknowledge(id).map_err(anyhow::Error::msg)?;
                    }
                } else {
                    let inputs = Inputs {
                        force_dynamic: self.force_dynamic,
                        graphic_ready: ordinary
                            .presenter_readiness_with(&simulation.vm, &self.definitions)?,
                        keep_dynamic: BTreeMap::new(),
                    };
                    let links = &simulation
                        .native_lot
                        .as_ref()
                        .context("classification physical links absent")?
                        .placement
                        .links;
                    let world = &mut simulation.vm.world;
                    let order = world
                        .registry_order
                        .as_ref()
                        .context("classification spatial order absent")?
                        .objects
                        .clone();
                    let mut v =
                        View::with_definitions(world, &self.definitions, links, &inputs, &order)
                            .map_err(anyhow::Error::msg)?;
                    if self
                        .classify
                        .advance(&mut v, &mut self.lists)
                        .map_err(anyhow::Error::msg)?
                        .is_none()
                    {
                        ensure!(
                            self.classify.complete(),
                            "classification returned before completion"
                        );
                        self.stage = Stage::Complete;
                    }
                    self.sync_vectors()?;
                }
            }
            Stage::Region => {
                if !scene.advance_region(self.region.as_mut().unwrap())? {
                    return Ok(false);
                }
                if self.region.as_ref().unwrap().complete() {
                    let id = self
                        .classify
                        .pending()
                        .context("classification region lost its invalidation")?;
                    simulation
                        .vm
                        .world
                        .refresh
                        .as_mut()
                        .unwrap()
                        .invalidated
                        .insert(id);
                    self.invalidations.push(id);
                    self.classify.acknowledge(id).map_err(anyhow::Error::msg)?;
                    self.region = None;
                    self.stage = Stage::Classify;
                }
            }
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn classification(
        &mut self,
        child: &mut State,
        simulation: &mut sims_sim::Simulation,
        ordinary: &sims_content::graphic_loading::Saved,
        scene: &mut scene::State,
        view: &composition::View,
    ) -> Result<bool> {
        ensure!(
            matches!(self.phase, Phase::Indicators | Phase::Classify) && self.owner == child.view,
            "classification does not own World caller"
        );
        let progress = child.advance(simulation, ordinary, scene, view)?;
        if child.after_drain() {
            self.phase = Phase::Classify;
        }
        if child.complete() {
            self.phase = Phase::Camera;
        }
        self.validate()?;
        Ok(progress)
    }
}

impl State {
    /// Expected native records are comparisons only; no live owner is replaced.
    pub fn compare_native(
        &self,
        encoded: &[u8],
        pc: u32,
        simulation: &sims_sim::Simulation,
        ordinary: &sims_content::graphic_loading::Saved,
        scene: &scene::State,
    ) -> Result<()> {
        use std::io::Read;
        ensure!(
            sims_fixtures::digest(encoded)
                == "f52fce1331b82220843c59f13bc06db6d2e8e5c870d93bbcbd34918576cbe627",
            "classification witness differs"
        );
        let mut text = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut text)?;
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let row = rows
            .iter()
            .find(|r| r["pc"] == pc && r["kind"] == "same_stop_outer_readback")
            .context("classification native stop absent")?;
        let c = &row["classification"];
        ensure!(
            c["view"] == self.view
                && c["module"] == self.module
                && row["tick"] == simulation.vm.tick,
            "classification native identity/epoch differs"
        );
        let mr = decode(&c["module_raw"])?;
        let vr = decode(&c["view_raw"])?;
        let reverse: BTreeMap<_, _> = self.pointers.iter().map(|(&id, &p)| (p, id)).collect();
        let queue = |index: usize, offset: usize| -> Result<Vec<Option<ObjectId>>> {
            let raw = decode(&c["queues"][index]["raw"])?;
            let len = word(&mr, offset + 4) - word(&mr, offset);
            raw.get(..len as usize)
                .context("classification queue backing short")?
                .chunks_exact(4)
                .map(|b| {
                    let p = word(b, 0);
                    if p == 0 {
                        Ok(None)
                    } else {
                        Ok(Some(
                            *reverse
                                .get(&p)
                                .context("classification queued identity absent")?,
                        ))
                    }
                })
                .collect()
        };
        let queues = &simulation.vm.world.indicator_queues;
        ensure!(
            queues.dirty == (mr[0xac] != 0)
                && queues.dates == queue(0, 0xb0)?
                && queues.stars == queue(1, 0xbc)?,
            "classification indicator queues differ at {pc:x}"
        );
        let native_cache = Vector {
            header: mr[0x4c..0x58].to_vec(),
            backing: decode(&c["cache"]["raw"])?,
        };
        ensure!(
            self.cache == native_cache,
            "classification native cache allocation differs at {pc:x}"
        );
        let cache = simulation
            .vm
            .world
            .registry_order
            .as_ref()
            .context("classification live spatial cache absent")?;
        ensure!(
            cache.dirty == (word(&mr, 0x44) != 0)
                && cache.rotation == Some(word(&mr, 0x48))
                && cache.objects == native_cache.ids(&self.pointers)?,
            "classification native spatial order differs at {pc:x}"
        );
        for (i, offset) in [0x1c8, 0x1d4, 0x1e0].into_iter().enumerate() {
            let expected = Vector {
                header: vr[offset..offset + 12].to_vec(),
                backing: decode(&c["vectors"][i]["raw"])?,
            };
            ensure!(
                self.vectors[i] == expected,
                "classification native list {i} allocation differs at {pc:x}"
            );
        }
        ensure!(
            scene.regions.header == vr[0x88..0xa0]
                && scene.regions.backing == decode(&c["regions_raw"])?,
            "classification shared region allocation differs at {pc:x}"
        );
        let ready = ordinary.presenter_readiness_with(&simulation.vm, &self.definitions)?;
        for o in c["objects"]
            .as_array()
            .context("classification native objects absent")?
        {
            let id = o["id"]
                .as_i64()
                .context("classification native object id absent")? as i16;
            let raw = decode(&o["raw"])?;
            let object = simulation
                .vm
                .world
                .objects
                .get(&id)
                .context("classification live object absent")?;
            ensure!(
                self.pointers[&id] == o["pointer"].as_u64().unwrap() as u32,
                "classification current object pointer differs"
            );
            let d = object
                .definition
                .as_ref()
                .context("classification live definition absent")?;
            let expected: Vec<_> = decode(&o["definition_raw"])?
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes(b.try_into().unwrap()))
                .collect();
            ensure!(
                self.definitions.words(d).map_err(anyhow::Error::msg)? == expected,
                "classification native definition differs for {id}"
            );
            let draw = &simulation
                .vm
                .world
                .refresh
                .as_ref()
                .context("classification live draw owner absent")?
                .draw[&id];
            ensure!(
                draw.level == word(&raw, 0x110) as i32
                    && draw.dynamic == (word(&raw, 0x15c) != 0)
                    && draw.expiry == word(&raw, 0x154) as i32
                    && draw.refreshed_at == word(&raw, 0x158),
                "classification native draw cache differs for {id} at {pc:x}: {draw:?}"
            );
            let bounds = std::array::from_fn(|n| {
                std::array::from_fn(|i| word(&raw, 0x160 + n * 16 + i * 4) as i32)
            });
            ensure!(
                self.rectangles[&id] == bounds,
                "classification cached rectangle input differs for {id}"
            );
            if object.person.is_none() {
                ensure!(
                    ready.get(&id) == Some(&raw[0x180]),
                    "classification live graphic readiness differs for {id}"
                );
            }
            if let Some(keep) = object.native_keep_dynamic {
                ensure!(
                    keep == raw[0x188],
                    "classification keep-dynamic input differs for {id}"
                );
            }
        }
        let fields = crate::original_session::completed_fields::compare(
            &serde_json::json!({"snapshot":row["simulation_snapshot"]}),
            simulation,
        )?;
        ensure!(
            fields["differences"] == 0,
            "classification native scalar/stack difference: {}",
            fields["first_differences"]
        );
        ensure!(
            simulation.vm.world.refresh.as_ref().unwrap().render_clock
                == word(&decode(&row["global_time"]["raw"])?, 0),
            "classification shared render clock differs"
        );
        if self.complete() {
            let expected: Vec<_> = rows
                .iter()
                .filter(|r| {
                    r["kind"] == "classification_callback"
                        && r["pc"] == 0x4a68dd
                        && r["classification_active"] == true
                })
                .map(|r| r["object"].as_i64().unwrap() as i16)
                .collect();
            ensure!(
                self.invalidations == expected,
                "classification native callback order differs"
            );
        }
        Ok(())
    }
}

impl State {
    /// 438f1b/4a68dd observes the old flag before434657 commits it.
    pub fn compare_invalidation(
        &self,
        encoded: &[u8],
        simulation: &sims_sim::Simulation,
    ) -> Result<()> {
        use std::io::Read;
        ensure!(
            sims_fixtures::digest(encoded)
                == "f52fce1331b82220843c59f13bc06db6d2e8e5c870d93bbcbd34918576cbe627",
            "classification callback witness differs"
        );
        let id = self
            .classify
            .pending()
            .context("classification invalidation not pending")?;
        let mut text = String::new();
        flate2::read::GzDecoder::new(encoded).read_to_string(&mut text)?;
        let rows: Vec<serde_json::Value> = text
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<_, _>>()?;
        let at = self.invalidations.len();
        let row = rows
            .iter()
            .filter(|r| {
                r["kind"] == "classification_callback"
                    && r["classification_active"] == true
                    && r["pc"] == 0x438f1b
            })
            .nth(at)
            .context("native object rectangle callback absent")?;
        ensure!(
            row["object"] == id,
            "native classification callback order differs"
        );
        let body = decode(&row["object_raw"])?;
        let draw = &simulation
            .vm
            .world
            .refresh
            .as_ref()
            .context("classification refresh absent")?
            .draw[&id];
        ensure!(
            draw.dynamic == (word(&body, 0x15c) != 0)
                && draw.expiry == word(&body, 0x154) as i32
                && draw.refreshed_at == word(&body, 0x158),
            "classification flag committed before its invalidation"
        );
        let stack = decode(&row["stack"])?;
        ensure!(
            word(&stack, 8) == 3,
            "native classification rectangle mask differs"
        );
        let [a, b] = self.rectangles[&id];
        let expected = [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ];
        let next = rows
            .iter()
            .filter(|r| {
                r["kind"] == "classification_callback"
                    && r["classification_active"] == true
                    && r["pc"] == 0x4a68dd
            })
            .nth(at)
            .context("native classification rectangle publication absent")?;
        let raw = decode(&next["rectangle_raw"])?;
        ensure!(
            expected == [0, 4, 8, 12].map(|o| word(&raw, o) as i32),
            "classification original cached rectangle differs"
        );
        Ok(())
    }
}

impl State {
    /// Inspect the existing owner; a renderer must not create a second cache.
    pub fn compare_draw_cache(&self,id:ObjectId,q:&serde_json::Value)->Result<()> {
        let cursor=&q["object_cursor"];
        let (pointer,bytes)=if cursor["id"]==id {(&cursor["pointer"],&cursor["raw"])}else{(&q["object_lookup"]["selected"],&q["object_lookup"]["object_raw"])};
        ensure!(*pointer==*self.pointers.get(&id).context("draw cache pointer absent")?,"draw cache identity differs");
        let raw=decode(bytes)?;
        let expected=std::array::from_fn(|n|std::array::from_fn(|i|word(&raw,0x160+16*n+4*i)as i32));
        ensure!(self.cached_rectangles(id)?==expected,"shared draw cache differs");
        Ok(())
    }
    pub(super) fn clear_cached_rectangle(&mut self,id:ObjectId,mask:u8)->Result<()> {
        if self.rectangle_flags.context("native rectangle-cache gate is unobserved")?!=0 {
            let bounds=self.rectangles.get_mut(&id).context("native rectangle-cache owner absent")?;
            for(i,b)in bounds.iter_mut().enumerate(){if mask&(1<<i)!=0{*b=[i32::MAX,i32::MAX,i32::MIN,i32::MIN];}}
        }
        Ok(())
    }
    pub(super) fn store_cached_rectangle(&mut self,id:ObjectId,mask:u8,rectangle:[i32;4])->Result<()>{
        if self.rectangle_flags.context("native rectangle-cache gate is unobserved")?!=0{
            let bounds=self.rectangles.get_mut(&id).context("native rectangle-cache owner absent")?;
            for(i,b)in bounds.iter_mut().enumerate(){if mask&(1<<i)!=0{*b=rectangle;if b[0]<b[2]{b[0]&=!3;b[2]=b[2].wrapping_add(3)&!3;}}}
        }Ok(())
    }
    pub(super) fn cached_rectangles(&self,id:ObjectId)->Result<[[i32;4];2]>{self.rectangles.get(&id).copied().context("native rectangle-cache owner absent")}
}
