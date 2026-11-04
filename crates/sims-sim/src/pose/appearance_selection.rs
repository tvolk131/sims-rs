//! Native 5874ca profile descriptors and represented 58a178 selection branches.
//! Unrepresented costume selectors remain explicit errors. Record replacement,
//! texture-cache notifications and material tint are ordered host services.
use super::appearance::{Record, Records};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Descriptor {
    pub name: String,
    pub texture: String,
}
/// Input is a decoded original single-byte string. Native temporary buffers
/// retain at most 255 characters, stop at NUL and do not trim whitespace.
pub fn descriptor(text: &str, secondary: bool) -> Descriptor {
    let text: String = text
        .split('\0')
        .next()
        .unwrap_or("")
        .chars()
        .take(255)
        .collect();
    let selected = if secondary {
        text.split_once(';').map_or("", |(_, b)| b)
    } else {
        text.split_once(';').map_or(text.as_str(), |(a, _)| a)
    };
    let (name, texture) = selected.split_once(',').unwrap_or((selected, ""));
    let texture = if let Some((key, value)) = texture.split_once('=') {
        format!("{key}={}", value.to_ascii_lowercase())
    } else {
        texture.to_ascii_lowercase()
    };
    Descriptor {
        name: name.to_ascii_lowercase(),
        texture,
    }
}
impl Descriptor {
    /// 58a12f compares at most the proposed string's length. A stored suffix
    /// does not itself require replacement. Empty proposed strings also match.
    pub fn replaces(&self, record: &Record) -> bool {
        record.binding.is_none()
            || !record.name.starts_with(&self.name)
            || !record.texture.starts_with(&self.texture)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Native +1b0/+1b4/+1b8/+1bc: outfit, alternate outfit, left/right hand.
    pub cached: [i32; 4],
    pub dirty: bool,
    pub special_dirty: bool,
    pub skin_dirty: bool,
    pub tint: [u8; 3],
}
impl State {
    pub fn constructed(tint: [u8; 3]) -> Self {
        Self {
            cached: [-1; 4],
            dirty: false,
            special_dirty: false,
            skin_dirty: false,
            tint,
        }
    }
    /// 5888d3 invalidates only the first two cached selectors after rebuilding.
    pub fn profile_reloaded(&mut self) {
        self.cached[0..2].fill(-1);
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Bind {
        record: usize,
        descriptor: Descriptor,
    },
    /// Native 4eb0cc for an absent head binding; 4eb0eb for replacement.
    TextureNotification {
        first: bool,
    },
    Tint([u8; 3]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Helper {
    Pajamas(i16),
    Career,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Direct,
    Notice,
    DecideBodies,
    Head,
    Body,
    Extra,
    Left,
    Right,
    Tint,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    /// Original one-based StringSet lookups, retained for native comparisons.
    pub lookups: Vec<usize>,
    /// A successful native helper invocation requires the process cache owner
    /// to survive saving even after its returned descriptor has been consumed.
    #[serde(default)]
    pub clothing_used: bool,
    chosen: [Descriptor; 5], // head, body, extra body, left hand, right hand
    replace: [bool; 3],
    stage: Stage,
    pending: Option<Request>,
    after: Stage,
}
impl Selection {
    /// Profile strings are immutable resource inputs. Only the represented
    /// profile-driven branches are accepted; no unsupported costume defaults.
    pub fn begin(
        state: &mut State,
        person: &mut BTreeMap<i16, i16>,
        profile: &[String],
        flags: u16,
        force: bool,
    ) -> Result<Self, String> {
        Self::begin_with_pajamas(state, person, profile, flags, force, |_| {
            Err("native appearance outfit 5 needs its pajama selector (45f034)".into())
        })
    }
    /// The fallback runs only after an empty entry 33 was observed. Its
    /// migration may publish a new entry 33; native 58a178 still consumes the
    /// returned legacy pajamas for this invocation. Caller owns profile writes.
    pub fn begin_with_pajamas(
        state: &mut State,
        person: &mut BTreeMap<i16, i16>,
        profile: &[String],
        flags: u16,
        force: bool,
        pajamas: impl FnOnce(i16) -> Result<Descriptor, String>,
    ) -> Result<Self, String> {
        let mut pajamas = Some(pajamas);
        Self::begin_with_helpers(
            state,
            person,
            profile,
            flags,
            force,
            |request| match request {
                Helper::Pajamas(age) => Ok([pajamas.take().unwrap()(age)?, Descriptor::default()]),
                Helper::Career => {
                    Err("native appearance outfit 3 needs its career/costume selector".into())
                }
            },
        )
    }
    pub fn begin_with_helpers(
        state: &mut State,
        person: &mut BTreeMap<i16, i16>,
        profile: &[String],
        flags: u16,
        force: bool,
        helpers: impl FnMut(Helper) -> Result<[Descriptor; 2], String>,
    ) -> Result<Self, String> {
        // An unsupported host boundary must not install a completed cache and
        // turn a retry into a silent no-op. Commit only a represented selection.
        let mut next_state = state.clone();
        let mut next_person = person.clone();
        let selection = Self::begin_inner(
            &mut next_state,
            &mut next_person,
            profile,
            flags,
            force,
            helpers,
        )?;
        *state = next_state;
        *person = next_person;
        Ok(selection)
    }
    fn begin_inner(
        state: &mut State,
        person: &mut BTreeMap<i16, i16>,
        profile: &[String],
        flags: u16,
        force: bool,
        mut helpers: impl FnMut(Helper) -> Result<[Descriptor; 2], String>,
    ) -> Result<Self, String> {
        let mut result = Self {
            lookups: vec![],
            clothing_used: false,
            chosen: Default::default(),
            replace: [false; 3],
            stage: Stage::Done,
            pending: None,
            after: Stage::Done,
        };
        let field = |n| {
            person
                .get(&n)
                .copied()
                .ok_or_else(|| format!("appearance person field {n} absent"))
        };
        let outfit = field(8)?;
        if flags & 8 != 0 {
            if outfit > 0 {
                state.dirty = true;
                state.special_dirty = true;
                state.skin_dirty = true;
                if state.cached[0] != i32::from(outfit) {
                    state.cached[0] = i32::from(outfit);
                    result.chosen[0] = result.read(profile, outfit as usize + 1)?;
                    result.stage = Stage::Direct;
                }
            }
            return Ok(result);
        }
        let species = field(65)?;
        let values = [
            if species & 6 != 0 { 0 } else { outfit },
            field(21)?,
            field(22)?,
            field(23)?,
        ];
        if species & 6 != 0 && outfit != 0 {
            person.insert(8, 0);
        }
        if !force && state.cached == values.map(i32::from) {
            return Ok(result);
        }
        // Reject unsupported branches before committing the selection cache.
        if (3..=60).contains(&values[0]) && !matches!(values[0], 3 | 4 | 5 | 17 | 18) {
            return Err(format!(
                "native appearance outfit {} needs its career/costume selector",
                values[0]
            ));
        }
        state.dirty = true;
        result.chosen[0] = result.read(profile, 3)?;
        state.cached[2] = i32::from(values[2]);
        state.cached[3] = i32::from(values[3]);
        // The original reads the right-hand descriptor before the left.
        for (slot, value, base, nohand) in
            [(4, values[3], 19, "nohandr"), (3, values[2], 18, "nohandl")]
        {
            result.chosen[slot] = if value == 3 {
                Descriptor {
                    name: nohand.into(),
                    texture: "nohand".into(),
                }
            } else {
                result.read(
                    profile,
                    base + match value {
                        1 => 2,
                        2 => 4,
                        4 => 8,
                        _ => 0,
                    },
                )?
            };
        }
        state.cached[0] = i32::from(values[0]);
        state.cached[1] = i32::from(values[1]);
        let body_entry = match values[0] {
            1 => 16,
            2 => {
                if result.raw(profile, 32)?.is_empty() {
                    17
                } else {
                    32
                }
            }
            3 => {
                let [body, extra] = helpers(Helper::Career)?;
                result.chosen[1] = body;
                result.chosen[2] = extra;
                // The career helper can return its retained cache, or reject
                // an absent career, without opening the clothing catalogs.
                // Its asset host records whether that process cache was used.
                0
            }
            4 => {
                if super::layer::atoi(result.raw(profile, 14)?) < 18 {
                    28
                } else if result.raw(profile, 31)?.is_empty() {
                    result.chosen[1] = result.read(profile, 28)?;
                    result.chosen[3] = result.read(profile, 29)?;
                    result.chosen[4] = result.read(profile, 30)?;
                    0
                } else {
                    31
                }
            }
            5 => {
                if result.raw(profile, 33)?.is_empty() {
                    let age = person
                        .get(&58)
                        .copied()
                        .ok_or("appearance person field 58 absent")?;
                    result.chosen[1] = helpers(Helper::Pajamas(age))?[0].clone();
                    result.clothing_used = true;
                    0
                } else {
                    33
                }
            }
            17 => 34,
            18 => 35,
            _ => 2,
        };
        if body_entry != 0 {
            result.chosen[1] = result.read(profile, body_entry)?;
        }
        if result.chosen[1].name.is_empty() && values[0] != 0 {
            result.chosen[1] = result.read(profile, 2)?;
        }
        result.stage = Stage::Notice;
        Ok(result)
    }
    fn raw<'a>(&mut self, profile: &'a [String], entry: usize) -> Result<&'a str, String> {
        self.lookups.push(entry);
        profile
            .get(entry.wrapping_sub(1))
            .map(String::as_str)
            .ok_or_else(|| format!("appearance profile entry {entry} absent"))
    }
    fn read(&mut self, profile: &[String], entry: usize) -> Result<Descriptor, String> {
        Ok(descriptor(self.raw(profile, entry)?, false))
    }
    pub fn is_complete(&self) -> bool {
        self.stage == Stage::Done && self.pending.is_none()
    }
    pub fn next(&mut self, state: &State, records: &Records) -> Option<Request> {
        if let Some(request) = &self.pending {
            return Some(request.clone());
        }
        let ids = [
            records.head,
            records.body[0],
            records.body[1],
            records.hands[0],
            records.hands[1],
        ];
        loop {
            let (request, after) = match self.stage {
                Stage::Direct => (
                    Request::Bind {
                        record: ids[1],
                        descriptor: self.chosen[0].clone(),
                    },
                    Stage::Done,
                ),
                Stage::Notice => {
                    let record = &records.entries[ids[0]];
                    self.replace[0] = self.chosen[0].replaces(record);
                    if !self.replace[0] {
                        self.stage = Stage::DecideBodies;
                        continue;
                    }
                    (
                        Request::TextureNotification {
                            first: record.binding.is_none(),
                        },
                        Stage::DecideBodies,
                    )
                }
                Stage::DecideBodies => {
                    self.replace[1] = self.chosen[1].replaces(&records.entries[ids[1]]);
                    self.replace[2] = if records.entries[ids[2]].binding.is_some() {
                        self.chosen[2].replaces(&records.entries[ids[2]])
                    } else {
                        !self.chosen[2].name.is_empty()
                    };
                    self.stage = Stage::Head;
                    continue;
                }
                Stage::Head | Stage::Body | Stage::Extra => {
                    let (i, after) = match self.stage {
                        Stage::Head => (0, Stage::Body),
                        Stage::Body => (1, Stage::Extra),
                        _ => (2, Stage::Left),
                    };
                    if !self.replace[i] {
                        self.stage = after;
                        continue;
                    }
                    (
                        Request::Bind {
                            record: ids[i],
                            descriptor: self.chosen[i].clone(),
                        },
                        after,
                    )
                }
                Stage::Left | Stage::Right => {
                    let (i, after) = if self.stage == Stage::Left {
                        (3, Stage::Right)
                    } else {
                        (4, Stage::Tint)
                    };
                    if self.chosen[3].name.is_empty()
                        || self.chosen[4].name.is_empty()
                        || !self.chosen[i].replaces(&records.entries[ids[i]])
                    {
                        self.stage = after;
                        continue;
                    }
                    (
                        Request::Bind {
                            record: ids[i],
                            descriptor: self.chosen[i].clone(),
                        },
                        after,
                    )
                }
                Stage::Tint => (Request::Tint(state.tint), Stage::Done),
                Stage::Done => return None,
            };
            self.pending = Some(request.clone());
            self.after = after;
            return Some(request);
        }
    }
    pub fn acknowledge(&mut self) -> Result<(), String> {
        if self.pending.take().is_none() {
            return Err("appearance selection has no pending service".into());
        }
        self.stage = self.after;
        Ok(())
    }
}
