//!4a8bad..4a8bfd. Walk the live person registry and retain the virtual-call
//! boundaries.590b8b publishes visibility before the caller tests its result.
use super::*;
use crate::original_session::character_visibility;
use anyhow::Context as _;
use sims_content::character_loading::live::SavedAssets;
use sims_sim::Simulation;
use sims_vm::ObjectId;
use std::collections::BTreeMap;

const SOURCE: &str = "426f975568ffe7f083ac262e027722902c841de67b6548858f415e393b7b7891";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Next,
    Model,
    Visibility,
    Returned,
    Clear,
    Increment,
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Binding {
    person: u32,
    animator: u32,
    model: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    source: String,
    view: u32,
    module: u32,
    device: u32,
    roster_header: Vec<u8>,
    bindings: BTreeMap<ObjectId, Binding>,
    index: usize,
    current: Option<ObjectId>,
    stage: Stage,
    pub results: Vec<(ObjectId, bool)>,
    pub rectangle_clears: Vec<ObjectId>,
}
impl State {
    pub fn import(encoded: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == SOURCE,
            "person visibility source differs"
        );
        let value: serde_json::Value = serde_json::from_slice(encoded)?;
        ensure!(
            value["tick"] == 89073 && value["executes_target_code"] == false,
            "person visibility source epoch differs"
        );
        let m = Memory::new(&value)?;
        let v = &value["person_visibility"];
        for (address, len, sha) in [
            (
                0x4594de,
                7,
                "63ef710ba3442ce8977b2dec96c4da852387e4661c79cb5bd07cface72ceac67",
            ),
            (
                0x590b8b,
                0x78,
                "687e879cc21c67c56c8ffb94c834f47ccc5d7977b800fefda413367f2993cae1",
            ),
            (
                0x438fe7,
                0x54,
                "de38803df22eb8d1b74400d3c0cfc70f001a767569f861d62b4efa279c51f3ac",
            ),
        ] {
            ensure!(
                sims_fixtures::digest(&m.read(address, len)?) == sha,
                "person visibility native consumer differs"
            );
        }

        let pointer = |v: &serde_json::Value| -> Result<u32> {
            v.as_u64()
                .context("person visibility pointer absent")?
                .try_into()
                .map_err(Into::into)
        };
        let module = pointer(&v["module"])?;
        let mr = m.read(module, 0xd0)?;
        ensure!(
            mr == decode(&v["module_raw"])?,
            "person visibility registry alias differs"
        );
        let mut bindings = BTreeMap::new();
        for p in v["people"]
            .as_array()
            .context("person visibility bindings absent")?
        {
            let id: ObjectId = p["object"]
                .as_i64()
                .context("person identity absent")?
                .try_into()?;
            let person = pointer(&p["pointer"])?;
            let animator = pointer(&p["animator"])?;
            let raw = m.read(animator, 0x260)?;
            let model = word(&raw, 8);
            ensure!(
                m.read(person + 0xadc, 4)? == animator.to_le_bytes()
                    && raw == decode(&p["animator_raw"])?
                    && pointer(&p["geometry"])? == model
                    && word(&m.read(word(&raw, 0), 0xb0)?, 0x6c) == 0x590b8b,
                "person Animator/model alias differs"
            );
            ensure!(
                bindings
                    .insert(
                        id,
                        Binding {
                            person,
                            animator,
                            model
                        }
                    )
                    .is_none(),
                "duplicate person model binding"
            );
        }
        let roster = decode(&v["roster_raw"])?;
        ensure!(
            m.read(word(&mr, 0x84), roster.len())? == roster
                && word(&mr, 0x88) - word(&mr, 0x84) == roster.len() as u32,
            "person roster allocation differs"
        );
        let state = Self {
            source: SOURCE.into(),
            view: pointer(&v["view"])?,
            module,
            device: word(&m.read(pointer(&v["view"])?, 0x58)?, 0x54),
            roster_header: mr[0x84..0x90].to_vec(),
            bindings,
            index: 0,
            current: None,
            stage: Stage::Entry,
            results: vec![],
            rectangle_clears: vec![],
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE
                && self.view == 0x09e88960
                && self.module == 0x09e267a0
                && self.device == 0x09e88e88
                && self.roster_header.len() == 12,
            "person visibility owner differs"
        );
        ensure!(
            self.current.is_some()
                == matches!(
                    self.stage,
                    Stage::Model
                        | Stage::Visibility
                        | Stage::Returned
                        | Stage::Clear
                        | Stage::Increment
                ),
            "person visibility iterator lifetime differs"
        );
        ensure!(
            self.current
                .is_none_or(|id| self.bindings.contains_key(&id)),
            "person visibility model owner absent"
        );
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4a8bad => self.stage == Stage::Entry,
            0x4a8bd5 => self.stage == Stage::Model,
            0x4a8bdb | 0x4a8be3 => self.stage == Stage::Visibility,
            0x4a8be6 => self.stage == Stage::Returned,
            0x4a8bef => self.stage == Stage::Clear,
            0x4a8bf4 => self.stage == Stage::Increment,
            0x4a8bfd => self.stage == Stage::Complete,
            _ => false,
        }
    }

    fn advance(
        &mut self,
        simulation: &mut Simulation,
        assets: &BTreeMap<ObjectId, SavedAssets>,
        visibility: character_visibility::Source,
        viewport: [i32; 4],
        classify: &mut classification::State,
    ) -> Result<bool> {
        self.validate()?;
        match self.stage {
            Stage::Entry => self.stage = Stage::Next,
            Stage::Next => {
                // Native compares against the registry's current end on every
                // iteration. A baseline roster is never the execution schedule.
                if let Some(&id) = simulation
                    .vm
                    .world
                    .person
                    .as_ref()
                    .context("live person registry absent")?
                    .person_order
                    .get(self.index)
                {
                    ensure!(
                        self.bindings.contains_key(&id),
                        "person getter needs its original native binding"
                    );
                    self.current = Some(id);
                    self.stage = Stage::Model;
                } else {
                    self.stage = Stage::Complete;
                }
            }
            Stage::Model => {
                let id = self.current.unwrap();
                ensure!(
                    simulation
                        .vm
                        .world
                        .objects
                        .get(&id)
                        .is_some_and(|o| o.person.is_some())
                        && simulation.characters.contains_key(&id),
                    "person getter lost its live resident"
                );
                if self.bindings[&id].model != 0 {
                    ensure!(
                        assets.contains_key(&id),
                        "person getter lost its actual retained model"
                    );
                }
                self.stage = Stage::Visibility;
            }
            Stage::Visibility => {
                let id = self.current.unwrap();
                let model = if self.bindings[&id].model == 0 {
                    None
                } else {
                    Some(&assets.get(&id).context("person model absent")?.model)
                };
                let resident = simulation
                    .characters
                    .get_mut(&id)
                    .context("person visibility resident absent")?;
                let value = visibility.visible_person(resident, model, viewport)?;
                resident.character.visible = value;
                self.results.push((id, value));
                self.stage = Stage::Returned;
            }
            Stage::Returned => {
                self.stage = if self
                    .results
                    .last()
                    .context("person visibility reply absent")?
                    .1
                {
                    Stage::Increment
                } else {
                    Stage::Clear
                }
            }
            Stage::Clear => {
                let id = self.current.unwrap();
                classify.clear_cached_rectangle(id, 2)?;
                self.rectangle_clears.push(id);
                self.stage = Stage::Increment;
            }
            Stage::Increment => {
                self.index += 1;
                self.current = None;
                self.stage = Stage::Next;
            }
            Stage::Complete => return Ok(false),
        }
        self.validate()?;
        Ok(true)
    }
    pub fn compare_row(
        &self,
        row: &serde_json::Value,
        simulation: &Simulation,
        assets: &BTreeMap<ObjectId, SavedAssets>,
        source: character_visibility::Source,
        viewport: [i32; 4],
        classify: &classification::State,
    ) -> Result<()> {
        self.validate()?;
        let pc = row["pc"]
            .as_u64()
            .context("person visibility stop absent")? as u32;
        ensure!(
            row["kind"] == "world_person_visibility"
                && row["tick"] == simulation.vm.tick
                && row["view"] == self.view
                && row["module"] == self.module,
            "person visibility native cut differs"
        );
        let mr = decode(&row["module_raw"])?;
        ensure!(
            mr[0x84..0x90] == self.roster_header,
            "person visibility registry storage changed"
        );
        let native_people = row["people"]
            .as_array()
            .context("native person roster absent")?;
        let order: Vec<_> = native_people
            .iter()
            .map(|p| {
                p["pointer"]
                    .as_u64()
                    .context("native person pointer absent")
                    .map(|x| x as u32)
            })
            .collect::<Result<_>>()?;
        let live: Vec<_> = simulation
            .vm
            .world
            .person
            .as_ref()
            .context("live person registry absent")?
            .person_order
            .iter()
            .map(|id| {
                self.bindings
                    .get(id)
                    .map(|b| b.person)
                    .context("live person binding absent")
            })
            .collect::<Result<_>>()?;
        ensure!(order == live, "live person roster differs from original");
        let view = decode(&row["raw"])?;
        ensure!(
            (0..4).all(|i| word(&view, 0x28 + 4 * i) as i32 == viewport[i]),
            "person visibility viewport differs"
        );
        for p in native_people {
            let pointer = p["pointer"].as_u64().unwrap() as u32;
            let (&id, binding) = self
                .bindings
                .iter()
                .find(|(_, b)| b.person == pointer)
                .context("native person alias absent")?;
            let raw = decode(&p["raw"])?;
            ensure!(
                word(&decode(&p["vtable_raw"])?, 0x8c) == 0x4594de,
                "person model getter virtual differs"
            );

            ensure!(
                i16::from_le_bytes(raw[0xf0..0xf2].try_into()?) == id,
                "native person identity differs"
            );
            let expected: [[i32; 4]; 2] = std::array::from_fn(|n| {
                std::array::from_fn(|i| word(&raw, 0x160 + 16 * n + 4 * i) as i32)
            });
            ensure!(
                classify.cached_rectangles(id)? == expected,
                "person cached rectangles differ for{id}"
            );
            ensure!(
                word(&decode(&p["animator_link"])?, 0) == binding.animator,
                "native person Animator link differs"
            );
        }
        match pc {
            0x4a8bad => ensure!(
                self.stage == Stage::Entry,
                "person loop did not start at entry"
            ),
            0x4a8bfd => ensure!(self.complete(), "person loop did not finish"),
            _ => {
                let id = self.current.context("person callback lost iterator")?;
                let b = self.bindings[&id];
                ensure!(row["person"] == b.person, "person callback order differs");
                let stage = match pc {
                    0x4a8bd5 => Stage::Model,
                    0x4a8bdb | 0x4a8be3 => Stage::Visibility,
                    0x4a8be6 => Stage::Returned,
                    0x4a8bef => Stage::Clear,
                    0x4a8bf4 => Stage::Increment,
                    _ => anyhow::bail!("unexpected person loop stop"),
                };
                ensure!(
                    self.stage == stage,
                    "person callback stage differs at{pc:x}"
                );
                if matches!(pc, 0x4a8bdb | 0x4a8be3 | 0x4a8be6) {
                    let m = &row["model"];
                    ensure!(
                        m["pointer"] == b.animator && m["geometry_pointer"] == b.model,
                        "native person model identity differs"
                    );
                    let raw = decode(&m["raw"])?;
                    let r = &simulation.characters[&id].character;
                    let floats = |raw: &[u8], offset: usize, values: &[f32]| {
                        (0..values.len()).all(|i| word(raw, offset + 4 * i) == values[i].to_bits())
                    };
                    ensure!(
                        (raw[0xd9] != 0) == r.placement.dirty,
                        "person placement dirty differs for{id}"
                    );
                    ensure!(
                        floats(&raw, 0x80, &r.placement.base)
                            && floats(&raw, 0x8c, &r.placement.offset)
                            && floats(&raw, 0x38, &r.matrix)
                            && word(&raw, 0x78) == 0,
                        "person placement/matrix differs for{id}"
                    );
                    ensure!(
                        (raw[0x109] != 0) == r.visible,
                        "person visibility publication differs for{id}"
                    );
                    let g = decode(&m["geometry_raw"])?;
                    ensure!(
                        word(&raw, 8) == b.model
                            && word(&decode(&m["vtable_raw"])?, 0x6c) == 0x590b8b
                            && m["skin_pointer"] == word(&g, 4)
                            && word(&g, 0x14) == b.animator
                            && row["device"]["pointer"] == self.device,
                        "person projection input owners differ"
                    );

                    let skin = decode(&m["skin_raw"])?;
                    let model = &assets
                        .get(&id)
                        .context("retained projection model absent")?
                        .model;
                    ensure!(
                        floats(&g, 0x68, &r.model.scale.values)
                            && word(&g, 0xa8) == r.model.scale.kind,
                        "person model scale differs for{id}"
                    );
                    ensure!(
                        floats(&skin, 0xbc, &model.local_bounds.minimum)
                            && floats(&skin, 0xc8, &model.local_bounds.maximum),
                        "person local bounds differ for{id}"
                    );
                    let device = decode(&row["device"]["raw"])?;
                    ensure!(
                        floats(&device, 0xec, &source.camera.values)
                            && word(&device, 0x12c) == source.camera.kind,
                        "person projection camera differs"
                    );
                    if pc == 0x4a8be6 {
                        ensure!(
                            row["result_al"].as_u64().map(|x| x != 0) == Some(r.visible),
                            "native person visibility result differs"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}
fn decode(v: &serde_json::Value) -> Result<Vec<u8>> {
    crate::original_session::completed_animations::bytes(
        v.as_str().context("person visibility raw bytes absent")?,
    )
}
impl Owner {
    pub(in crate::original_session::draw_parent) fn bind_people(
        &mut self,
        source: &[u8],
    ) -> Result<()> {
        ensure!(
            self.at_entry() && self.people.is_none(),
            "person visibility already bound/executing"
        );
        let s = State::import(source)?;
        ensure!(s.view == self.owner, "person visibility receiver differs");
        self.people = Some(s);
        self.validate()
    }
    pub(in crate::original_session::draw_parent) fn people_visibility(
        &mut self,
        simulation: &mut Simulation,
        assets: &BTreeMap<ObjectId, SavedAssets>,
        source: character_visibility::Source,
        classify: &mut classification::State,
    ) -> Result<bool> {
        ensure!(
            self.phase == Phase::PeopleVisibility,
            "person visibility does not own World"
        );
        let viewport = self
            .camera
            .as_ref()
            .context("person visibility camera absent")?
            .viewport();
        let people = self
            .people
            .as_mut()
            .context("person visibility source absent")?;
        let progress = people.advance(simulation, assets, source, viewport, classify)?;
        if people.complete() {
            self.phase = Phase::Drawing;
        }
        self.validate()?;
        Ok(progress)
    }
    pub(in crate::original_session::draw_parent) fn people(&self) -> Option<&State> {
        self.people.as_ref()
    }
}
