//! Frozen-B split-directory input, retaining the native first/second pair.
//! Current room light/occupancy comes from the computed simulation, not B.
use super::*;
use sims_sim::lot_environment::lighting::point::{split_sample, RoomLight};

const SOURCE: &str = "25ad8dfefae45713646363a2feb14c77394b8e126ca8f964083e622397ef51fd";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pair {
    point: [i8; 3],
    rooms: [i32; 2],
    directions: [i32; 2],
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    source: String,
    pairs: Vec<Pair>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub point: [i8; 3],
    pub flags: u32,
    pub ids: [u16; 2],
    pub directions: [i32; 2],
    pub rooms: [Option<RoomLight>; 2],
}
impl Source {
    fn wall_flags(
        &self,
        sim: &sims_sim::Simulation,
        physical: [i32; 2],
        rotation: u8,
    ) -> Result<u32> {
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("split lot absent")?;
        let floor = lot.floor(1).context("split floor absent")?;
        let point = [(physical[1] >> 4) as i8, (physical[0] >> 4) as i8, 1];
        Ok(u32::from(
            sims_sim::lot_presenter::terrain_walls::segments(
                lot.size,
                &floor.source.walls,
                point,
                rotation,
            )
            .map_err(anyhow::Error::msg)?,
        ))
    }
    pub fn import(bytes: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(bytes) == SOURCE,
            "split input differs"
        );
        let v: serde_json::Value = serde_json::from_slice(bytes)?;
        ensure!(
            v["tick"] == 89073 && v["executes_target_code"] == false,
            "split directory epoch differs"
        );
        let m = Memory::new(&v)?;
        let manager = word(&m.read(0x6e5428, 4)?, 0);
        let raw = m.read(manager, 0x80)?;
        let head = word(&raw, 0x10);
        let mut pending = vec![word(&m.read(head, 36)?, 4)];
        let mut seen = std::collections::BTreeSet::new();
        let mut pairs = Vec::new();
        while let Some(node) = pending.pop() {
            if node == 0 || node == head {
                continue;
            }
            ensure!(
                seen.insert(node) && seen.len() < 1024,
                "split directory cycle/extent differs"
            );
            let n = m.read(node, 36)?;
            pending.extend([word(&n, 8), word(&n, 12)]);
            pairs.push(Pair {
                point: [n[16] as i8, n[17] as i8, n[18] as i8],
                rooms: [word(&n, 20) as i32, word(&n, 28) as i32],
                directions: [word(&n, 24) as i32, word(&n, 32) as i32],
            });
        }
        ensure!(
            pairs.len() == word(&raw, 0x14) as usize,
            "split directory count differs"
        );
        let s = Self {
            source: SOURCE.into(),
            pairs,
        };
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source == SOURCE && self.pairs.len() == 7,
            "split source owner differs"
        );
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.pairs {
            ensure!(
                p.point[..2].iter().all(|v| (0..64).contains(v))
                    && (1..=2).contains(&p.point[2])
                    && p.directions.iter().all(|v| (0..=4).contains(v))
                    && seen.insert(p.point),
                "split pair differs"
            );
        }
        Ok(())
    }
    pub fn inputs(
        &self,
        sim: &sims_sim::Simulation,
        physical: [i32; 2],
        rotation: u8,
    ) -> Result<Inputs> {
        self.validate()?;
        let lot = sim
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .context("split lot absent")?;
        let floor = lot.floor(1).context("split floor absent")?;
        let point = [(physical[1] >> 4) as i8, (physical[0] >> 4) as i8, 1];
        let rotated = sims_sim::lot_presenter::terrain_shade::inverse(point, rotation)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            rotated[..2]
                .iter()
                .all(|&v| v >= 0 && (v as usize) < lot.size),
            "split pair lookup outside retained lot"
        );
        let (ids, mut directions) = if let Some(p) = self.pairs.iter().find(|p| p.point == rotated)
        {
            (p.rooms.map(|r| r as u16), p.directions)
        } else {
            //4671f8's actual not-found branch asks466fcc for the primary ID.
            let id =
                floor.rooms.primary[rotated[0] as usize * lot.size + rotated[1] as usize] as u16;
            ([id; 2], [0; 2])
        };
        let mut ids = ids;
        if ids[1] >= 0xfffb {
            ids[1] = ids[0];
            directions = [0; 2];
        }
        let manager = &sim
            .room_environment
            .as_ref()
            .context("split room manager absent")?
            .manager;
        let rooms = ids.map(|id| {
            manager.rooms.get(&id).map(|r| RoomLight {
                light: r.scores.light,
                pool: r.pool,
                occupied: r.occupied,
                lights: r.totals.lights,
                people: r.totals.people,
            })
        });
        let flags = sims_sim::lot_presenter::terrain_walls::segments(
            lot.size,
            &floor.source.walls,
            point,
            rotation,
        )
        .map_err(anyhow::Error::msg)?;
        Ok(Inputs {
            point: rotated,
            flags: u32::from(flags),
            ids,
            directions,
            rooms,
        })
    }
}

/// Original lookup phases: wall return precedes the room pair and float return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    stage: u8,
    flags: Option<u32>,
    inputs: Option<Inputs>,
    result: Option<u32>,
}
impl Cursor {
    pub fn new() -> Self {
        Self {
            stage: 0,
            flags: None,
            inputs: None,
            result: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.stage <= 7
                && self.flags.is_some() == (self.stage >= 3)
                && self.inputs.is_some() == (self.stage >= 5)
                && self.result.is_some() == (self.stage >= 6),
            "split continuation owner differs"
        );
        Ok(())
    }
    pub fn complete(&self) -> bool {
        self.stage == 7
    }
    pub fn result(&self) -> Option<u32> {
        self.result
    }
    pub fn advance(
        &mut self,
        source: &Source,
        sim: &sims_sim::Simulation,
        physical: [i32; 2],
        rotation: u8,
    ) -> Result<()> {
        self.validate()?;
        match self.stage {
            0 | 1 | 3 => (),
            2 => self.flags = Some(source.wall_flags(sim, physical, rotation)?),
            4 => {
                let inputs = source.inputs(sim, physical, rotation)?;
                ensure!(
                    Some(inputs.flags) == self.flags,
                    "split wall changed during its child"
                );
                self.inputs = Some(inputs);
            }
            5 => {
                let i = self.inputs.as_ref().unwrap();
                self.result = Some(
                    split_sample(physical, rotation, i.flags, i.directions, i.rooms)
                        .map_err(anyhow::Error::msg)?
                        .light,
                );
            }
            6 => (),
            _ => anyhow::bail!("completed split lookup advanced"),
        }
        self.stage += 1;
        self.validate()
    }
    pub fn at_native_cut(&self, pc: u32) -> bool {
        match pc {
            0x4674e4 => self.stage == 0,
            0x467515 => self.stage == 1,
            0x467690 => self.stage == 2,
            0x46754e => self.stage == 3,
            0x4671f8 => self.stage == 4,
            0x467574 => self.stage == 5,
            0x46767f => self.stage == 6,
            _ => false,
        }
    }
    pub fn matches_wall_call(&self, row: &serde_json::Value) -> Result<bool> {
        let raw = crate::original_session::completed_animations::bytes(
            row["stack"]["raw"]
                .as_str()
                .context("split wall stack absent")?,
        )?;
        Ok(self.stage == 2 && word(&raw, 0) == 0x46754e)
    }
    pub fn compare(&self, row: &serde_json::Value) -> Result<()> {
        self.validate()?;
        let pc = row["pc"].as_u64().context("split PC absent")? as u32;
        ensure!(self.at_native_cut(pc), "split native phase differs");
        let extra = &row["split_light"];
        let bytes = |v: &serde_json::Value| {
            crate::original_session::completed_animations::bytes(
                v.as_str().context("split observation bytes absent")?,
            )
        };
        if pc == 0x4674e4 {
            ensure!(
                bytes(&extra["primary_room"]["raw"])? == (-5i16).to_le_bytes(),
                "split primary room differs"
            );
        }
        if pc == 0x467690 {
            ensure!(self.matches_wall_call(row)?, "split wall caller differs");
        }
        if pc == 0x46754e {
            ensure!(
                word(&bytes(&extra["expanded_wall"]["raw"])?, 0) == self.flags.unwrap(),
                "split expanded wall flags differ"
            );
        }
        if pc == 0x467574 {
            let i = self.inputs.as_ref().unwrap();
            ensure!(
                extra["outputs"][2] == i.directions[0] && extra["outputs"][3] == i.directions[1],
                "split native pair directions differ"
            );
            for n in 0..2 {
                let native = &extra["rooms"][n];
                if let Some(r) = i.rooms[n] {
                    let raw = bytes(&native["raw"])?;
                    ensure!(
                        u16::from_le_bytes(raw[..2].try_into()?) == i.ids[n]
                            && word(&raw, 0x6c) == r.light
                            && (raw[0x73] != 0) == r.occupied
                            && (raw[0x75] != 0) == r.pool
                            && word(&raw, 0x280) as i32 == r.lights
                            && word(&raw, 0x284) as i32 == r.people,
                        "split current room fields differ for side{n}"
                    );
                } else {
                    ensure!(native["pointer"] == 0, "split null room differs");
                }
            }
        }
        Ok(())
    }
}
