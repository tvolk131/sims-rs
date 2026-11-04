//! Current completed Simulation inputs, not a Simulation constructor.
//! Retains per-person score owners separately instead of substituting global
//! resource curves. No scheduler, publication, initialization or RNG runs.
use super::completed_lot::Memory;
pub mod compose;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sims_vm::{
    ObjectId, Snapshot,
    motives::{Curve, Rates},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Imported {
    pub clock: sims_sim::clock::LotClock,
    pub character: sims_sim::character_update::Settings,
    pub diagnostics: sims_sim::lot_acceptance::person::Diagnostics,
    pub statistics: sims_sim::house_statistics::State,
    pub valuation: sims_sim::house_valuation::Retained,
    pub scheduler_cursor: i32,
    pub fp_control: u16,
    pub rates: [Rates; 3],
    pub adult_mood: Vec<Curve>,
    pub child_mood: Vec<Curve>,
    pub person_scores: BTreeMap<ObjectId, Vec<Curve>>,
    pub source: Value,
    pub complete_native_simulation_admitted: bool,
}
impl Imported {
    /// Add only the separately observed per-person semantic owner. Existing
    /// script arrays, frames, queues, yields, clocks and random streams stay put.
    pub fn with_score_owners(&self, snapshot: Snapshot) -> Result<Snapshot> {
        let checked = import(&self.source, &snapshot)?;
        ensure!(
            &checked == self,
            "Simulation owner envelope differs from source"
        );
        self.attach_score_owners(snapshot)
    }
    pub fn with_captured_score_owners(
        &self,
        snapshot: Snapshot,
        memory: &sims_fixtures::readset::Readset<'_>,
    ) -> Result<Snapshot> {
        ensure!(
            &import_captured(&self.source, &snapshot.state, memory)? == self,
            "current Simulation score owner envelope differs"
        );
        self.attach_score_owners(snapshot)
    }
    fn attach_score_owners(&self, mut snapshot: Snapshot) -> Result<Snapshot> {
        for (&id, curves) in &self.person_scores {
            let owner = sims_vm::motives::score_owner::Owner::capture(curves)
                .map_err(anyhow::Error::msg)?;
            let object = snapshot
                .state
                .world
                .objects
                .get_mut(&id)
                .context("score actor absent")?;
            ensure!(
                object
                    .native_motive_scores
                    .as_ref()
                    .is_none_or(|old| *old == owner),
                "native personal score cache contradicts existing owner"
            );
            object.native_motive_scores = Some(owner);
        }
        Ok(snapshot)
    }
    /// A new replacement wrapper at the proved completed C-stack boundary.
    /// Native decay history and native subscribers remain separately owned.
    pub fn household(
        &self,
        snapshot: &Snapshot,
        proof: &super::completed_skin::Input,
    ) -> Result<HouseholdJoin> {
        let objects = super::completed_skin::synchronous_boundary(proof)?;
        ensure!(
            objects == snapshot.state.world.objects.keys().copied().collect()
                && proof.tick == snapshot.state.tick
                && snapshot.state.scheduled_object_visit.is_none()
                && snapshot.state.lot_rng_scope.is_none()
                && snapshot.state.player_menu.is_none()
                && snapshot
                    .state
                    .threads
                    .values()
                    .all(|t| matches!(t.status, sims_vm::Status::NativeRegistryReentry(_))),
            "household wrapper requires the actual completed source boundary and no accepted Rust work"
        );
        let checked = import(&self.source, snapshot)?;
        ensure!(&checked == self, "household inputs differ from source");
        self.household_fields(snapshot, "goth88448-household-fresh-unpublished-v1")
    }
    /// Current completed caller; pending VM route/animation owners are retained,
    /// while the replacement person-update delivery ledger starts unpublished.
    pub fn household_captured(
        &self,
        snapshot: &Snapshot,
        memory: &sims_fixtures::readset::Readset<'_>,
        epoch: &str,
    ) -> Result<HouseholdJoin> {
        ensure!(
            !epoch.is_empty()
                && epoch.len() <= 4096
                && snapshot.state.scheduled_object_visit.is_none()
                && snapshot.state.lot_rng_scope.is_none()
                && snapshot.state.player_menu.is_none(),
            "current household completed adapter boundary differs"
        );
        ensure!(
            &import_captured(&self.source, &snapshot.state, memory)? == self,
            "current household inputs differ from source"
        );
        self.household_fields(snapshot, epoch)
    }
    fn household_fields(&self, snapshot: &Snapshot, epoch: &str) -> Result<HouseholdJoin> {
        let mut profiles: BTreeMap<bool, Vec<Curve>> = BTreeMap::new();
        let mut people = BTreeMap::new();
        for (&id, curves) in &self.person_scores {
            let object = &snapshot.state.world.objects[&id];
            let owner = object
                .native_motive_scores
                .as_ref()
                .context("household personal score owner not attached")?;
            ensure!(
                *owner
                    == sims_vm::motives::score_owner::Owner::capture(curves)
                        .map_err(anyhow::Error::msg)?,
                "household score owner differs"
            );
            let age = *object
                .person
                .as_ref()
                .and_then(|p| p.get(&58))
                .context("household native age absent")?;
            let adult = age == 0 || age > 17;
            if let Some(previous) = profiles.get(&adult) {
                ensure!(
                    sims_vm::motives::score_owner::Owner::capture(previous)
                        .map_err(anyhow::Error::msg)?
                        == *owner,
                    "observed per-person score tables cannot also fill this household's compatibility profile"
                );
            } else {
                profiles.insert(adult, curves.clone());
            }
            people.insert(
                id,
                sims_sim::household::PersonUpdate {
                    started: None,
                    finished: None,
                },
            );
        }
        ensure!(
            profiles.len() == 2,
            "this Goth wrapper requires observed adult and child compatibility slots"
        );
        let tuning = sims_vm::motives::Tuning {
            update_precision: sims_vm::motives::Precision::Single,
            rates: self.rates.clone(),
            adult_mood: self.adult_mood.clone(),
            child_mood: self.child_mood.clone(),
            adult_scores: profiles.remove(&true).unwrap(),
            child_scores: profiles.remove(&false).unwrap(),
        };
        Ok(HouseholdJoin {
            state: sims_sim::household::Household {
                valuation: Some(self.valuation.clone()),
                house_statistics: Some(self.statistics.clone()),
                clock: self.clock.clone(),
                tuning,
                people,
                clock_events: vec![],
            },
            delivery_epoch: epoch.into(),
            native_subscriber_history_claimed: false,
            score_compatibility_slots_observed_from_current_actors: true,
            complete_native_simulation_admitted: false,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HouseholdJoin {
    pub state: sims_sim::household::Household,
    pub delivery_epoch: String,
    pub native_subscriber_history_claimed: bool,
    pub score_compatibility_slots_observed_from_current_actors: bool,
    pub complete_native_simulation_admitted: bool,
}
const CODE: &[(u32, usize, &str)] = &[
    (
        0x46d345,
        480,
        "86b48a6aa3fdc9eb2f0306405e64f0ddf2bcd129b537159bb76b1aa082671502",
    ),
    (
        0x45cc8c,
        616,
        "a68dcdebef223b8c275b1ea48de77156345cd4916709a12dc8be81cb2654db2d",
    ),
    (
        0x4337f6,
        937,
        "c801b61c37a9c9c14f651d2cdb3885f99710085faa553a4da7e0d5a0b33ccba7",
    ),
    (
        0x58dc72,
        7728,
        "4fb4bc597355a68e2c40d9d3ed2c8155f9bb3cefda83c8c57aa45cb8dbb7e12c",
    ),
    (
        0x588fa0,
        111,
        "d229bdc4c3f9579a613dabd9ddecceb8838bb7aa1f961459f7a93b32cfb98557",
    ),
    (
        0x592132,
        882,
        "8e4f45486fbf8754467943ea39988a68b73266266b0b16dfd43e81af268ad6e4",
    ),
    (
        0x4aaf3c,
        6,
        "41d2c23fed43a1e28c11cda3efe2d5f2c1983f13669f2da6c9b83c8622baf8f3",
    ),
    (
        0x4aaf42,
        6,
        "d704a42c87acd4d7563c0bad91000eee9c08eff7402c4bcfd3c170906cc982ef",
    ),
    (
        0x4a6e11,
        7,
        "b26137ce17a8386215bb1fde2b53f227d965b04ec16598700dc9cd28de01928f",
    ),
    (
        0x420c36,
        806,
        "1cae8b1c3f304b6ab1f1d1b910ffc3084bef09f25b33c3c36e531d00c6ae9132",
    ),
    (
        0x45d145,
        32,
        "8a84f630424348fdd0fd0314d08b3d74345306549820ef7304b9e6fdbaadf210",
    ),
    (
        0x42138d,
        169,
        "9eb479943de660173e2032c587c1bc3fad9cb09ddcca9e1a1a36367beb7cadc8",
    ),
    (
        0x45962b,
        701,
        "3adef58d7d8186469eca4a44cd2741bd4b259b336d2b6ebee8fde186d821d118",
    ),
    (
        0x46137b,
        127,
        "7514809a8d015188298374a32d60e9c56ab5a20004e67db16fa6ae28771c3e2c",
    ),
    (
        0x46131d,
        94,
        "d9a6d2e49bcc53c1b8c257c0ddf10c57d52db1e7aac4b20671462469964d612f",
    ),
    (
        0x4613fa,
        41,
        "f72d47a0564fb752bfa7bb519cdd4efb9a4640af8ea2ca101f413f82e9e9045d",
    ),
    (
        0x416085,
        216,
        "00e20e14c072a09b4d32ca545b3d0ffcea52be76a5bee7b88ffb104eabd39e33",
    ),
    (
        0x4165e3,
        16,
        "4f3fa3fccb5654041f639de96292ccb95a71f389b74b0b2ce46b355e39b0b698",
    ),
    (
        0x42144e,
        83,
        "4a709f28d9dc90199f71149aec1e32ce7384dd2908fe4c0e38590601c08e7aff",
    ),
    (
        0x4214a1,
        226,
        "afc008b0a1ec71714dd2ffc0df3838b680a25a28dcbe30ac7648dec028e19239",
    ),
    (
        0x45de01,
        11,
        "70cb6efcd9a5a469a2bf9717c024ab495b5d15f83a3418b760762ccf5da9bbaf",
    ),
    (
        0x45e125,
        25,
        "ddcbbe804cdf46c6c76b4317d5f12063d600cce62cc4ec96fbb0808fe50769ff",
    ),
];
const MOVEMENT: &[(&str, u32)] = &[
    ("short_distance", 0x6d07b0),
    ("turn_rate", 0x6d07bc),
    ("small_turn", 0x6d07c4),
    ("medium_turn", 0x6d07c8),
    ("large_turn", 0x6d07cc),
    ("minimum_weight", 0x6d07e0),
    ("snap_distance", 0x6d07e4),
    ("base", 0x6d079c),
    ("positive", 0x6d07a0),
    ("negative", 0x6d07a4),
    ("change", 0x6d07e8),
];
const RATES: &[(&str, u32)] = &[
    ("bladder_asleep", 0x6e4948),
    ("bladder_awake", 0x6e4954),
    ("hygiene_asleep", 0x6e4960),
    ("hygiene_awake", 0x6e496c),
    ("fun_asleep", 0x6e4978),
    ("fun_awake", 0x6e4984),
    ("social_multiplier", 0x6e4990),
    ("social_base", 0x6e499c),
    ("hunger_ratio", 0x6e49a8),
    ("comfort_slope", 0x6e49b4),
    ("comfort_base", 0x6e49c0),
    ("hunger_bladder", 0x6e49cc),
    ("energy_drift", 0x6e49d8),
    ("sleep_hour", 0x6e49e4),
    ("wake_hour", 0x6e49f0),
    ("energy_asleep", 0x6e49fc),
    ("energy_awake", 0x6e4a08),
];
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("simulation bytes absent")?)
}
fn p(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("simulation pointer absent")?
        .try_into()?)
}
fn word(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .context("short simulation word")?
            .try_into()?,
    ))
}
fn half(b: &[u8], o: usize) -> Result<i16> {
    Ok(i16::from_le_bytes(
        b.get(o..o + 2)
            .context("short simulation half")?
            .try_into()?,
    ))
}
fn f(bits: u32) -> Result<f32> {
    let v = f32::from_bits(bits);
    ensure!(v.is_finite(), "nonfinite retained simulation configuration");
    Ok(v)
}
pub(super) fn curves(m: &Memory, data: &Value) -> Result<Vec<Curve>> {
    let count = data["count"].as_u64().context("curve count absent")? as usize;
    let address = p(&data["pointer"])?;
    let raw = bytes(&data["raw"])?;
    let rows = data["curves"].as_array().context("curves absent")?;
    ensure!(
        (1..=16).contains(&count)
            && rows.len() == count
            && raw.len() == count * 20
            && m.read(address, raw.len())? == raw,
        "curve directory differs"
    );
    let mut result = vec![];
    for (i, row) in rows.iter().enumerate() {
        let b = bytes(&row["raw"])?;
        ensure!(
            b.len() == 20
                && row["pointer"] == address + i as u32 * 20
                && b == raw[i * 20..i * 20 + 20],
            "curve descriptor alias differs"
        );
        let (points, slopes, n, capacity, index) = (
            word(&b, 0)?,
            word(&b, 4)?,
            word(&b, 8)? as usize,
            word(&b, 12)? as usize,
            word(&b, 16)? as usize,
        );
        let point_raw = bytes(&row["points_raw"])?;
        let slope_raw = bytes(&row["slopes_raw"])?;
        ensure!(
            n <= capacity
                && capacity > 0
                && capacity <= 256
                && index < 16
                && point_raw.len() == capacity * 8
                && slope_raw.len() == (capacity - 1) * 4
                && m.read(points, point_raw.len())? == point_raw
                && (slope_raw.is_empty() || m.read(slopes, slope_raw.len())? == slope_raw),
            "native curve buffers differ"
        );
        let points = point_raw[..n * 8]
            .chunks_exact(8)
            .map(|b| Ok([f(word(b, 0)?)?, f(word(b, 4)?)?]))
            .collect::<Result<Vec<_>>>()?;
        let curve = Curve {
            motive: index,
            points,
        };
        curve.validate().map_err(anyhow::Error::msg)?;
        for (j, w) in curve.points.windows(2).enumerate() {
            let inverse = (1.0 / (f64::from(w[1][0]) - f64::from(w[0][0]))) as f32;
            ensure!(
                word(&slope_raw, j * 4)? == inverse.to_bits(),
                "native retained inverse slope cannot be represented by current Curve"
            );
        }
        result.push(curve);
    }
    Ok(result)
}

pub fn import(input: &Value, snapshot: &Snapshot) -> Result<Imported> {
    import_vm(input, &snapshot.state)
}

/// The same source validation against borrowed VM state, without making a
/// throwaway Snapshot or inventing a Program binding for diagnostics.
pub fn import_vm(input: &Value, vm: &sims_vm::Vm) -> Result<Imported> {
    ensure!(
        input["schema"] == 1
            && input["tick"] == 88448
            && vm.tick == 88448
            && input["provenance"]["image_sha256"]
                == "877982344b1c8b95eec2858bd40f11551b8f2b2805dd1ce7fc6eaac2c983109f"
            && input["provenance"]["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c",
        "simulation source epoch differs"
    );
    import_fields(input, vm)
}
pub fn import_captured(
    input: &Value,
    vm: &sims_vm::Vm,
    memory: &sims_fixtures::readset::Readset<'_>,
) -> Result<Imported> {
    ensure!(
        input["schema"] == 1 && input["tick"] == vm.tick,
        "current Simulation owner epoch differs"
    );
    for r in input["reads"]
        .as_array()
        .context("current Simulation owner reads absent")?
    {
        let raw = bytes(&r["raw"])?;
        ensure!(
            raw.len() <= 1 << 20 && memory.read(p(&r["address"])?, raw.len())? == raw,
            "current Simulation owner/readset differs"
        );
    }
    import_fields(input, vm)
}
fn import_fields(input: &Value, vm: &sims_vm::Vm) -> Result<Imported> {
    let m = Memory::new(input)?;
    for &(a, n, hash) in CODE {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == hash,
            "simulation native layout code differs"
        );
    }
    let owner = p(&input["owner"])?;
    let owner_raw = bytes(&input["owner_raw"])?;
    let app = p(&input["application"])?;
    let app_raw = bytes(&input["application_raw"])?;
    let simulator = p(&input["simulator"])?;
    let sim = bytes(&input["simulator_raw"])?;
    let viewer = p(&input["viewer"])?;
    let vr = bytes(&input["viewer_raw"])?;
    ensure!(
        word(&m.read(0x6e48bc, 4)?, 0)? == owner
            && owner_raw.len() == 0x60
            && m.read(owner, owner_raw.len())? == owner_raw
            && word(&owner_raw, 0)? == app
            && word(&owner_raw, 4)? == viewer
            && word(&owner_raw, 0x1c)? == simulator
            && app_raw.len() == 0x80
            && m.read(app, app_raw.len())? == app_raw
            && sim.len() == 0x1b0
            && m.read(simulator, sim.len())? == sim
            && vr.len() == 0x180
            && m.read(viewer, vr.len())? == vr,
        "simulation owner aliases differ"
    );
    let world = &vm.world;
    for i in 0..64 {
        ensure!(
            world.globals.get(&(i as i16)) == Some(&half(&sim, 0x10 + 2 * i)?),
            "simulation clock/World globals differ"
        );
    }
    ensure!(
        word(&sim, 0x98)? as u64 == vm.tick && Some(word(&sim, 0xac)?) == vm.lot_rng_state,
        "simulation clock/VM RNG differs"
    );
    let hardware = input["hardware_registers"]
        .as_str()
        .context("native hardware evidence absent")?;
    ensure!(
        input["fp_control"] == 0x003f
            && hardware.matches("FCW=003f").count() == 1
            && hardware.contains("EIP=0046d51d")
            && hardware.contains(&format!("EBX={simulator:08x}")),
        "simulation hardware control/boundary differs"
    );
    let clock = sims_sim::clock::LotClock {
        hour: half(&sim, 0x10)?,
        day: half(&sim, 0x12)?,
        period: half(&sim, 0x18)?,
        minute: half(&sim, 0x1a)?,
        second: half(&sim, 0x1c)?,
        month: half(&sim, 0x1e)?,
        year: half(&sim, 0x20)?,
        elapsed_days: half(&sim, 0x48)?,
        month_days: half(&sim, 0x4a)?,
        ticks: word(&sim, 0x98)?,
        statistics: std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                i32::from_le_bytes(
                    sim[0xe4 + i * 32 + j * 4..0xe8 + i * 32 + j * 4]
                        .try_into()
                        .unwrap(),
                )
            })
        }),
    };
    clock.validate().map_err(anyhow::Error::msg)?;
    let mut motion = serde_json::Map::new();
    for &(name, a) in MOVEMENT {
        let bits = word(&m.read(a, 4)?, 0)?;
        ensure!(
            input["movement"][name] == bits,
            "native movement setting differs"
        );
        motion.insert(name.into(), json!(f(bits)?));
    }
    let rate = json!({"base":motion.remove("base").unwrap(),"positive":motion.remove("positive").unwrap(),
        "negative":motion.remove("negative").unwrap(),"change":motion.remove("change").unwrap()});
    motion.insert("rate".into(), rate);
    let movement = serde_json::from_value(Value::Object(motion))?;
    let vtable = p(&input["viewer_vtable"])?;
    let vt = bytes(&input["viewer_vtable_raw"])?;
    ensure!(
        word(&vr, 0)? == vtable
            && vt.len() == 0x28
            && m.read(vtable, vt.len())? == vt
            && [word(&vt, 0x14)?, word(&vt, 0x18)?, word(&vt, 0x24)?]
                == [0x4aaf3c, 0x4aaf42, 0x4a6e11],
        "native viewer getter bindings differ"
    );
    let zoom = word(&m.read(0x6f00d8, 4)?, 0)?;
    let rotation = word(&m.read(0x6f00dc, 4)?, 0)?;
    ensure!(
        zoom <= 2
            && rotation <= 3
            && input["zoom"] == zoom
            && input["rotation"] == rotation
            && world.camera_rotation as u32 == rotation,
        "native simulation view configuration differs"
    );
    let floor = bytes(&input["floor_vector_raw"])?;
    ensure!(
        floor.len() == 12 && m.read(viewer + 0x170 - 12 * zoom, 12)? == floor,
        "native floor vector differs"
    );
    let root_motion = m.read(0x6f6d70, 1)?[0];
    ensure!(
        input["root_motion"] == root_motion,
        "native root motion control differs"
    );
    let person = sims_sim::person_update::Context {
        only_visible: m.read(0x6e48c0, 1)?[0] == 0,
        display_animation_names: owner_raw[0x5a] != 0,
        debug_recording: m.read(0x6e5488, 1)?[0] != 0,
    };
    ensure!(
        serde_json::to_value(person)? == input["person_context"],
        "native person context differs"
    );
    let terrain_scale = if input["terrain_scale_raw"].is_null() {
        None // older component evidence did not observe this process cache
    } else {
        let raw = bytes(&input["terrain_scale_raw"])?;
        ensure!(
            raw.len() == 8 && m.read(0x703f28, 8)? == raw && word(&raw, 0)? & 1 != 0,
            "retained terrain scale is absent or not initialized"
        );
        ensure!(
            sims_fixtures::digest(&m.read(0x5868fa, 57)?)
                == "f545b0ccf6fd1464f8dcad54297c84604025daf1b783fc7164a49e07c508d48b",
            "retained terrain scale-cache code differs"
        );
        let value = f(word(&raw, 4)?)?;
        ensure!(value > 0., "invalid retained terrain scale");
        Some(value)
    };
    let character = sims_sim::character_update::Settings {
        terrain_scale,
        movement,
        root_motion,
        person,
        floor_vector: [
            f(word(&floor, 0)?)?,
            f(word(&floor, 4)?)?,
            f(word(&floor, 8)?)?,
        ],
    };
    let diagnostic = bytes(&input["diagnostics_raw"])?;
    ensure!(
        diagnostic.len() == 8
            && m.read(0x6e4acc, 8)? == diagnostic
            && word(&diagnostic, 4)? == 0
            && input["blocker"].is_null(),
        "native diagnostic blocker needs retained identity"
    );
    let diagnostics = sims_sim::lot_acceptance::person::Diagnostics {
        error: word(&diagnostic, 0)? as i32,
        blocker: None,
    };
    let statistics = sims_sim::house_statistics::State {
        pairs: std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                u32::from_le_bytes(
                    app_raw[0x38 + i * 8 + j * 4..0x3c + i * 8 + j * 4]
                        .try_into()
                        .unwrap(),
                )
            })
        }),
    };
    let valuation = sims_sim::house_valuation::Retained {
        objects_value: word(&sim, 0xa0)? as i32,
        architecture_value: word(&sim, 0xa4)? as i32,
    };
    let mut rates = vec![];
    for i in 0..3 {
        let mut values = serde_json::Map::new();
        for &(name, a) in RATES {
            let raw = m.read(a, 12)?;
            ensure!(
                bytes(&input["rates"][name])? == raw,
                "native motive rate arrays differ"
            );
            let bits = word(&raw, i * 4)?;
            values.insert(
                name.into(),
                if name.ends_with("_hour") {
                    json!(bits as i32)
                } else {
                    json!(f(bits)?)
                },
            );
        }
        rates.push(serde_json::from_value(Value::Object(values))?);
    }
    ensure!(
        input["mood_loaded"] == 1 && m.read(0x6e5290, 1)? == [1],
        "native mood cache not initialized"
    );
    let mood = |name: &str, a: u32| -> Result<Vec<Curve>> {
        let v = &input["moods"][name];
        let raw = bytes(&v["holder_raw"])?;
        ensure!(
            v["holder"] == a
                && raw.len() == 8
                && m.read(a, 8)? == raw
                && v["data"]["pointer"] == word(&raw, 0)?
                && v["data"]["count"] == word(&raw, 4)?,
            "native mood holder differs"
        );
        curves(&m, &v["data"])
    };
    let adult_mood = mood("adult_mood", 0x6e51f8)?;
    let child_mood = mood("child_mood", 0x6e5160)?;
    let mut person_scores = BTreeMap::new();
    for row in input["people"]
        .as_array()
        .context("native person score owners absent")?
    {
        let id: ObjectId = row["object"]
            .as_i64()
            .context("native score actor absent")?
            .try_into()?;
        let a = p(&row["pointer"])?;
        let root = p(&row["root"])?;
        let binding = bytes(&row["binding_raw"])?;
        let raw = bytes(&row["raw"])?;
        let age = bytes(&row["age_raw"])?;
        let identity = bytes(&row["identity_raw"])?;
        ensure!(
            binding.len() == 4
                && m.read(a + 0xaec, 4)? == binding
                && word(&binding, 0)? == root
                && raw.len() == 0xc4
                && m.read(root, raw.len())? == raw
                && word(&raw, 0xbc)? == a
                && word(&raw, 0xc0)? == a + 0x78c
                && identity.len() == 2
                && half(&identity, 0)? == id
                && m.read(a + 0xf0, 2)? == identity
                && row["data"]["pointer"] == word(&raw, 0)?
                && row["data"]["count"] == word(&raw, 4)?
                && age.len() == 2
                && m.read(a + 0x600, 2)? == age
                && world
                    .objects
                    .get(&id)
                    .and_then(|o| o.person.as_ref())
                    .and_then(|p| p.get(&58))
                    == Some(&half(&age, 0)?),
            "native personal score binding differs"
        );
        ensure!(
            person_scores
                .insert(id, curves(&m, &row["data"])?)
                .is_none(),
            "duplicate native personal score owner"
        );
    }
    ensure!(
        person_scores.keys().copied().collect::<BTreeSet<_>>()
            == world
                .objects
                .iter()
                .filter(|(_, o)| o.person.is_some())
                .map(|(&id, _)| id)
                .collect(),
        "native person score coverage differs"
    );
    let scheduler_cursor = word(&sim, 0x1a8)? as i32;
    ensure!(
        scheduler_cursor == 0,
        "non-completed native scheduler needs its pass owner"
    );
    Ok(Imported {
        clock,
        character,
        diagnostics,
        statistics,
        valuation,
        scheduler_cursor,
        fp_control: 0x003f,
        rates: rates
            .try_into()
            .map_err(|_| anyhow::anyhow!("native rate profile count differs"))?,
        adult_mood,
        child_mood,
        person_scores,
        source: input.clone(),
        complete_native_simulation_admitted: false,
    })
}
