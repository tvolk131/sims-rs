//! 4e9652: temporary portrait person through the ordinary engine factory.
//! The host must perform real construction, typed lookup, location search,
//! virtual movement and removal. Missing services never become fake successes.
use super::State;
use serde::{Deserialize, Serialize};

/// Defined fields of 4347d1. Unanchored searches have no initialized level;
/// they obtain their origin from the native camera service, not a chosen tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Search {
    pub anchored: bool,
    pub position: [i32; 2],
    pub level: Option<i32>,
    pub direction: i32,
    pub same_room: bool,
    pub empty: bool,
    pub inside: bool,
    pub random: bool,
}
impl Default for Search {
    fn default() -> Self {
        Self {
            anchored: false,
            position: [0; 2],
            level: None,
            direction: -1,
            same_room: true,
            empty: true,
            inside: false,
            random: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Resource {
        guid: u32,
    },
    Create {
        resource: u32,
    },
    /// 40743b lookup followed by native person RTTI, not resource kind alone.
    Person {
        id: i16,
    },
    Find {
        person: u32,
        descriptor: Search,
        output: [i32; 2],
    },
    Move {
        person: u32,
        position: [i32; 2],
        level: i32,
        flags: [u32; 2],
    },
    Remove {
        created: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Resource(Option<u32>),
    /// Full native factory return; lookup uses only its signed low word, but
    /// failed-placement removal receives the original return value.
    Created(u32),
    Person(Option<u32>),
    Found {
        found: bool,
        position: [i32; 2],
        anchored_level: Option<i32>,
    },
    /// Native move/removal return values are ignored by this caller.
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Resource,
    Create,
    Person,
    Find,
    Move,
    Remove,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Temporary {
    index: u32,
    guid: u32,
    resource: Option<u32>,
    created: u32,
    person: Option<u32>,
    position: [i32; 2],
    level: i32,
    stage: Stage,
    pending: Option<Request>,
}
impl Temporary {
    pub fn new(index: u32) -> Self {
        Self {
            index,
            guid: 0,
            resource: None,
            created: 0,
            person: None,
            position: [0; 2],
            level: 1,
            stage: Stage::Start,
            pending: None,
        }
    }
    pub fn returned(&self) -> Option<Option<u32>> {
        (self.stage == Stage::Complete).then_some(self.person)
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    /// Next native service, including before it has been polled by the host.
    pub fn request(&self) -> Result<Option<Request>, String> {
        let person = || self.person.ok_or("temporary portrait person absent");
        Ok(match self.stage {
            Stage::Start | Stage::Complete => None,
            Stage::Resource => Some(Request::Resource { guid: self.guid }),
            Stage::Create => Some(Request::Create {
                resource: self.resource.ok_or("temporary portrait resource absent")?,
            }),
            Stage::Person => Some(Request::Person {
                id: self.created as i16,
            }),
            Stage::Find => Some(Request::Find {
                person: person()?,
                descriptor: Search::default(),
                output: [0; 2],
            }),
            Stage::Move => Some(Request::Move {
                person: person()?,
                position: self.position,
                level: self.level,
                flags: [0; 2],
            }),
            Stage::Remove => Some(Request::Remove {
                created: self.created,
            }),
        })
    }
    pub fn created(&self) -> u32 {
        self.created
    }
    pub fn guid(&self) -> u32 {
        self.guid
    }
    pub fn validate(&self, cache: &State) -> Result<(), String> {
        cache.validate()?;
        if self.resource == Some(0)
            || self.person == Some(0)
            || self.stage != Stage::Start
                && self.guid != cache.entries.get(self.index as usize).map_or(0, |e| e.guid)
            || !matches!(self.stage, Stage::Start | Stage::Complete) && self.guid == 0
            || matches!(
                self.stage,
                Stage::Create | Stage::Person | Stage::Find | Stage::Move | Stage::Remove
            ) && self.resource.is_none()
            || matches!(
                self.stage,
                Stage::Person | Stage::Find | Stage::Move | Stage::Remove
            ) && self.created as i16 == 0
            || matches!(self.stage, Stage::Find | Stage::Move | Stage::Remove)
                && self.person.is_none()
            || self.stage == Stage::Complete && self.pending.is_some()
            || self.pending.is_some() && self.pending != self.request()?
        {
            return Err("invalid temporary portrait-person continuation".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, cache: &State) -> Result<Option<Request>, String> {
        self.validate(cache)?;
        if self.stage == Stage::Start {
            self.guid = cache.entries.get(self.index as usize).map_or(0, |e| e.guid);
            self.stage = if self.guid == 0 {
                Stage::Complete
            } else {
                Stage::Resource
            };
        }
        self.pending = self.request()?;
        Ok(self.pending)
    }
    pub fn respond(&mut self, cache: &State, response: Response) -> Result<(), String> {
        self.validate(cache)?;
        match (self.pending, response) {
            (Some(Request::Resource { .. }), Response::Resource(resource)) => {
                if resource == Some(0) {
                    return Err("null resource handle must be absent".into());
                }
                self.resource = resource;
                self.stage = if resource.is_some() {
                    Stage::Create
                } else {
                    Stage::Complete
                };
            }
            (Some(Request::Create { .. }), Response::Created(created)) => {
                self.created = created;
                self.stage = if created as i16 == 0 {
                    Stage::Complete
                } else {
                    Stage::Person
                };
            }
            (Some(Request::Person { .. }), Response::Person(person)) => {
                if person == Some(0) {
                    return Err("null person handle must be absent".into());
                }
                self.person = person;
                // The native does not call removal when lookup/RTTI fails.
                self.stage = if person.is_some() {
                    Stage::Find
                } else {
                    Stage::Complete
                };
            }
            (
                Some(Request::Find { .. }),
                Response::Found {
                    found,
                    position,
                    anchored_level,
                },
            ) => {
                self.position = position;
                self.level = anchored_level.unwrap_or(1);
                self.stage = if found { Stage::Move } else { Stage::Remove };
            }
            (Some(Request::Move { .. }), Response::Done) => self.stage = Stage::Complete,
            (Some(Request::Remove { .. }), Response::Done) => {
                self.person = None;
                self.stage = Stage::Complete;
            }
            _ => return Err("temporary portrait response does not match pending request".into()),
        }
        self.pending = None;
        Ok(())
    }
}
