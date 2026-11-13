//!437ce8's60-byte auxiliary list. Inactive entries are visited; active drawing
//!remains an explicit4380f3 child rather than an invented empty result.
use super::*;
const INPUT: &str = "87dfdd6a7cb1f933ddb3b7bbc12e95ac4519042a2a2d2995fe5836e4d9d180aa";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Owner {
    pointer: u32,
    begin: u32,
    end: u32,
    capacity: u32,
    raw: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Source {
    identity: String,
    owners: BTreeMap<i16, Owner>,
}
impl Source {
    pub fn import(encoded: &[u8], directory: &[u8]) -> Result<Self> {
        ensure!(
            sims_fixtures::digest(encoded) == INPUT
                && sims_fixtures::digest(directory) == DIRECTORY,
            "auxiliary B source differs"
        );
        let v: serde_json::Value = serde_json::from_slice(encoded)?;
        let d: serde_json::Value = serde_json::from_slice(directory)?;
        ensure!(
            v["tick"] == 89073
                && v["executes_target_code"] == false
                && v["directory_sha256"] == DIRECTORY,
            "auxiliary source epoch differs"
        );
        let memory = Memory::new(&v)?;
        let mut owners = BTreeMap::new();
        for entry in d["objects"]
            .as_array()
            .context("auxiliary object directory absent")?
        {
            let id = entry["id"].as_i64().context("auxiliary ID absent")? as i16;
            let pointer = entry["pointer"]
                .as_u64()
                .context("auxiliary pointer absent")? as u32;
            let obj = memory.read(pointer, 0x18c)?;
            ensure!(
                crate::original_session::completed_animations::bytes(
                    entry["raw"]
                        .as_str()
                        .context("auxiliary directory bytes absent")?
                )? == obj,
                "auxiliary B object disagrees with directory"
            );
            let begin = word(&obj, 0x148);
            let end = word(&obj, 0x14c);
            let capacity = word(&obj, 0x150);
            ensure!(
                begin <= end
                    && end <= capacity
                    && (end - begin) % 60 == 0
                    && end - begin <= 60 * 256,
                "auxiliary vector outside observed extent"
            );
            let raw = if begin == end {
                Vec::new()
            } else {
                memory.read(begin, (end - begin) as usize)?
            };
            ensure!(
                owners
                    .insert(
                        id,
                        Owner {
                            pointer,
                            begin,
                            end,
                            capacity,
                            raw
                        }
                    )
                    .is_none(),
                "duplicate auxiliary owner"
            );
        }
        let value = Self {
            identity: INPUT.into(),
            owners,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.identity == INPUT && self.owners.len() <= 1024,
            "auxiliary provenance/owner count differs"
        );
        for (&id, o) in &self.owners {
            ensure!(
                id > 0
                    && o.pointer != 0
                    && o.begin <= o.end
                    && o.end <= o.capacity
                    && (o.end - o.begin) % 60 == 0
                    && o.raw.len() == (o.end - o.begin) as usize
                    && o.raw.len() <= 60 * 256,
                "auxiliary owner storage differs"
            );
        }
        Ok(())
    }
    pub fn owner(&self, id: i16, pointer: u32) -> Result<&Owner> {
        let o = self.owners.get(&id).context("auxiliary owner absent")?;
        ensure!(o.pointer == pointer, "auxiliary owner identity changed");
        Ok(o)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Loop,
    Gate,
    Decision,
    Next,
    Incremented,
    Finished,
    Arguments,
    Call,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Scan {
    stage: Stage,
    pointer: u32,
}
impl Scan {
    pub fn new(o: &Owner) -> Self {
        Self {
            stage: Stage::Loop,
            pointer: o.begin,
        }
    }
    pub fn pc(&self) -> u32 {
        match self.stage {
            Stage::Loop => 0x437cee,
            Stage::Gate => 0x437cf9,
            Stage::Decision => 0x437cfd,
            Stage::Next => 0x437d26,
            Stage::Incremented => 0x437d29,
            Stage::Finished => 0x437d2b,
            Stage::Arguments => 0x437cff,
            Stage::Call => 0x437d07,
            Stage::Done => 0x437d3d,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    fn validate(&self, o: &Owner) -> Result<()> {
        ensure!(
            self.pointer >= o.begin && self.pointer <= o.end && (self.pointer - o.begin) % 60 == 0,
            "auxiliary scan escaped retained vector"
        );
        if !matches!(
            self.stage,
            Stage::Loop | Stage::Incremented | Stage::Finished | Stage::Done
        ) {
            ensure!(self.pointer < o.end, "auxiliary scan read past vector end");
        }
        Ok(())
    }
    pub fn advance(&mut self, o: &Owner) -> Result<bool> {
        self.validate(o)?;
        self.stage = match self.stage {
            Stage::Loop => {
                if self.pointer == o.end {
                    Stage::Finished
                } else {
                    Stage::Gate
                }
            }
            Stage::Gate => Stage::Decision,
            Stage::Decision => {
                if word(&o.raw, (self.pointer - o.begin) as usize + 0x10) == 0 {
                    Stage::Next
                } else {
                    Stage::Arguments
                }
            }
            Stage::Next => {
                self.pointer = self
                    .pointer
                    .checked_add(60)
                    .context("auxiliary iterator overflow")?;
                Stage::Incremented
            }
            Stage::Incremented => Stage::Loop,
            Stage::Finished => Stage::Done,
            Stage::Arguments => Stage::Call,
            Stage::Call | Stage::Done => return Ok(false),
        };
        self.validate(o)?;
        Ok(true)
    }
    pub fn compare(&self, q: &serde_json::Value, o: &Owner, id: i16) -> Result<()> {
        self.validate(o)?;
        ensure!(q["pc"] == self.pc(), "auxiliary scan PC differs");
        compare_owner(q, o, id)?;
        ensure!(
            q["registers"]["eax"] == self.pointer
                && q["auxiliary_vector"]["iterator"] == self.pointer,
            "auxiliary iterator differs"
        );
        Ok(())
    }
}
pub(super) fn compare_owner(q: &serde_json::Value, o: &Owner, id: i16) -> Result<()> {
    let v = &q["auxiliary_vector"];
    ensure!(
        v.is_object()
            && v["parent"] == o.pointer
            && v["id"] == id
            && v["begin"] == o.begin
            && v["end"] == o.end
            && v["capacity"] == o.capacity,
        "native auxiliary vector identity/extent differs"
    );
    let actual = crate::original_session::completed_animations::bytes(
        v["raw"].as_str().context("native auxiliary bytes absent")?,
    )?;
    ensure!(
        actual == o.raw,
        "live auxiliary records differ from retained inputs"
    );
    Ok(())
}
