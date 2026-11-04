//! 4e8c5d: all seven portrait outputs, with explicit rendering/resource/person
//! services. This retains native callback and wrapper ownership order without
//! claiming to implement the pixel generators or temporary-person construction.
use super::{Event, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Resource {
        guid: u32,
    },
    Find {
        index: u32,
    },
    /// Resource virtual +20: writable capability (ObjResFile 4067b3),
    /// distinct from the +28 valid/open query (4067e4).
    Eligible {
        resource: u32,
    },
    Create {
        index: u32,
    },
    Clone {
        image: u32,
    },
    Render {
        slot: usize,
        person: u32,
        input: Option<u32>,
    },
    Key {
        image: u32,
        color: u32,
    },
    Persist {
        index: u32,
        chunk: u16,
        image: Option<u32>,
    },
    Compose {
        faces: Option<u32>,
        background: u32,
    },
    Remove {
        person: u32,
    },
    Flush {
        resource: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Resource(Option<u32>),
    Person(Option<u32>),
    Eligible(bool),
    /// Image is borrowed from a host owner. Render success is independent of
    /// whether the wrapper is empty. Composition's Boolean is ignored.
    Image {
        image: Option<u32>,
        success: bool,
    },
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Resource,
    Find,
    Eligible,
    Create,
    Select,
    Clone,
    Render,
    Rendered,
    Key,
    Store,
    Stored,
    Persisted,
    Compose,
    Composed,
    Cleanup,
    Remove,
    Flush,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Regeneration {
    index: u32,
    mask: u32,
    persist: i32,
    background: Option<u32>,
    thumbnail: Option<u32>,
    color: u32,
    resource: Option<u32>,
    person: Option<u32>,
    remove: bool,
    cursor: usize,
    local: [Option<u32>; 2],
    success: bool,
    rendered: bool,
    stage: Stage,
    pending: Option<Request>,
}

const SLOTS: [usize; 6] = [0, 2, 3, 4, 5, 6];
const MASKS: [u32; 6] = [1, 8, 2, 4, 4, 16];

impl Regeneration {
    pub fn new(
        index: u32,
        mask: u32,
        persist: i32,
        background: Option<u32>,
        thumbnail: Option<u32>,
        color: u32,
    ) -> Self {
        Self {
            index,
            mask,
            persist,
            background,
            thumbnail,
            color,
            resource: None,
            person: None,
            remove: false,
            cursor: 0,
            local: [None; 2],
            success: true,
            rendered: false,
            stage: Stage::Resource,
            pending: None,
        }
    }
    pub fn returned(&self) -> Option<bool> {
        (self.stage == Stage::Complete).then_some(self.success)
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    /// Immutable arguments retained by the synchronous native invocation.
    pub fn binding(&self) -> (u32, u32, i32, Option<u32>, Option<u32>, u32) {
        (
            self.index,
            self.mask,
            self.persist,
            self.background,
            self.thumbnail,
            self.color,
        )
    }
    fn slot(&self) -> Result<usize, String> {
        SLOTS
            .get(self.cursor)
            .copied()
            .ok_or_else(|| "portrait output cursor absent".into())
    }
    fn local_index(&self) -> Result<usize, String> {
        Ok(usize::from(matches!(self.slot()?, 2 | 4)))
    }
    fn stop(&mut self) {
        self.success = false;
        self.stage = Stage::Complete;
    }
    fn request(&self, cache: &State) -> Result<Option<Request>, String> {
        let resource = || self.resource.ok_or("portrait resource absent");
        let person = || self.person.ok_or("portrait person absent");
        Ok(match self.stage {
            Stage::Resource => cache
                .entries
                .get(self.index as usize)
                .filter(|e| e.resource.is_none())
                .map(|e| Request::Resource { guid: e.guid }),
            Stage::Find => Some(Request::Find { index: self.index }),
            Stage::Eligible => Some(Request::Eligible {
                resource: resource()?,
            }),
            Stage::Create => Some(Request::Create { index: self.index }),
            Stage::Clone => self.thumbnail.map(|image| Request::Clone { image }),
            Stage::Render => Some(Request::Render {
                slot: self.slot()?,
                person: person()?,
                input: self.local[self.local_index()?],
            }),
            Stage::Key => Some(Request::Key {
                image: self.local[self.local_index()?]
                    .ok_or("successful speech portrait has no image")?,
                color: self.color,
            }),
            Stage::Stored if self.persist == 1 => Some(Request::Persist {
                index: self.index,
                chunk: 2002 + self.slot()? as u16 - u16::from(self.slot()? > 0),
                image: self.local[self.local_index()?],
            }),
            Stage::Compose => self.background.map(|background| Request::Compose {
                faces: self.local[0],
                background,
            }),
            Stage::Remove if self.remove => Some(Request::Remove { person: person()? }),
            Stage::Flush if self.persist == 1 => Some(Request::Flush {
                resource: resource()?,
            }),
            _ => None,
        })
    }
    pub fn validate(&self, cache: &State) -> Result<(), String> {
        cache.validate()?;
        let generating = matches!(
            self.stage,
            Stage::Clone
                | Stage::Render
                | Stage::Rendered
                | Stage::Key
                | Stage::Store
                | Stage::Stored
                | Stage::Persisted
                | Stage::Compose
                | Stage::Composed
                | Stage::Cleanup
        );
        if self.cursor > SLOTS.len()
            || self.resource == Some(0)
            || self.person == Some(0)
            || self.stage != Stage::Resource
                && self.stage != Stage::Complete
                && (self.index as usize >= cache.entries.len() || self.resource.is_none())
            || self.stage == Stage::Complete && (self.pending.is_some() || self.local != [None; 2])
            || self.resource.is_some()
                && cache
                    .entries
                    .get(self.index as usize)
                    .is_none_or(|entry| entry.resource != self.resource)
            || generating
                && (self.person.is_none()
                    || self.cursor >= SLOTS.len()
                    || self.mask & MASKS[self.cursor] == 0)
            || matches!(self.stage, Stage::Remove | Stage::Flush)
                && (self.person.is_none() || self.cursor != SLOTS.len() || self.local != [None; 2])
            || self.stage == Stage::Clone && self.slot()? != 3
            || self.stage == Stage::Key && !matches!(self.slot()?, 4 | 5)
            || matches!(self.stage, Stage::Compose | Stage::Composed)
                && (self.slot()? != 0 || self.background.is_none())
            || matches!(
                self.stage,
                Stage::Resource | Stage::Find | Stage::Eligible | Stage::Create
            ) && (self.cursor != 0 || self.local != [None; 2])
            || self.pending.is_some() && self.request(cache)? != self.pending
        {
            return Err("invalid portrait regeneration continuation".into());
        }
        let mut counts = vec![0u32; cache.refs.len()];
        for image in cache
            .entries
            .iter()
            .flat_map(|e| e.images)
            .chain(self.local)
            .chain([self.background, self.thumbnail])
            .flatten()
        {
            *counts
                .get_mut(image.wrapping_sub(1) as usize)
                .ok_or("regeneration image absent")? += 1;
        }
        if counts
            .iter()
            .zip(&cache.refs)
            .any(|(owners, refs)| owners > refs)
        {
            return Err("portrait regeneration ownership exceeds references".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        cache: &mut State,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<Option<Request>, String> {
        self.validate(cache)?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            if let Some(request) = self.request(cache)? {
                self.pending = Some(request);
                return Ok(self.pending);
            }
            match self.stage {
                Stage::Resource => {
                    if let Some(entry) = cache.entries.get(self.index as usize) {
                        self.resource = entry.resource;
                        self.stage = Stage::Find;
                    } else {
                        self.stop();
                    }
                }
                Stage::Select => {
                    if self.person.is_none() {
                        self.stop();
                        continue;
                    }
                    while self.cursor < SLOTS.len() && self.mask & MASKS[self.cursor] == 0 {
                        self.cursor += 1;
                    }
                    self.stage = if self.cursor == SLOTS.len() {
                        Stage::Remove
                    } else if self.slot()? == 3 {
                        Stage::Clone
                    } else {
                        Stage::Render
                    };
                }
                Stage::Clone => self.stage = Stage::Cleanup, // no thumbnail template
                Stage::Rendered => {
                    self.stage = if !self.rendered {
                        // Thumbnail failures uniquely do not clear overall success.
                        if self.slot()? != 3 {
                            self.success = false;
                        }
                        Stage::Cleanup
                    } else if matches!(self.slot()?, 4 | 5) {
                        Stage::Key
                    } else {
                        Stage::Store
                    };
                }
                Stage::Store => {
                    cache.store_image(
                        self.index as usize,
                        self.slot()?,
                        self.local[self.local_index()?],
                        host,
                    )?;
                    self.stage = Stage::Stored;
                }
                Stage::Stored => self.stage = Stage::Persisted,
                Stage::Persisted => {
                    self.stage = if self.slot()? == 0 && self.background.is_some() {
                        Stage::Compose
                    } else {
                        Stage::Cleanup
                    }
                }
                Stage::Composed => {
                    cache.store_image(self.index as usize, 1, self.local[1], host)?;
                    self.stage = Stage::Cleanup;
                }
                Stage::Cleanup => {
                    // Faces/composite: composite then faces. Speech: keep the
                    // medium local alive until large completes, then large/medium.
                    match self.slot()? {
                        0 => {
                            cache.assign_image(&mut self.local[1], None, host)?;
                            cache.assign_image(&mut self.local[0], None, host)?;
                        }
                        2 => cache.assign_image(&mut self.local[1], None, host)?,
                        4 => {}
                        5 => {
                            cache.assign_image(&mut self.local[0], None, host)?;
                            cache.assign_image(&mut self.local[1], None, host)?;
                        }
                        _ => cache.assign_image(&mut self.local[0], None, host)?,
                    }
                    self.cursor += 1;
                    self.stage = Stage::Select;
                }
                Stage::Remove => self.stage = Stage::Flush,
                Stage::Flush => self.stage = Stage::Complete,
                Stage::Complete => return Ok(None),
                _ => return Err("portrait regeneration service absent".into()),
            }
        }
    }
    pub fn respond(
        &mut self,
        cache: &mut State,
        response: Response,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<(), String> {
        self.validate(cache)?;
        match (self.pending, response) {
            (Some(Request::Resource { .. }), Response::Resource(resource)) => {
                if resource == Some(0) {
                    return Err("null resource handle must be absent".into());
                }
                cache.entries[self.index as usize].resource = resource;
                self.resource = resource;
                if resource.is_some() {
                    self.stage = Stage::Find;
                } else {
                    self.stop();
                }
            }
            (Some(Request::Find { .. }), Response::Person(person)) => {
                if person == Some(0) {
                    return Err("null person handle must be absent".into());
                }
                self.person = person;
                self.remove = person.is_none() || self.mask == 16;
                self.stage = if self.remove {
                    Stage::Eligible
                } else {
                    Stage::Select
                };
            }
            (Some(Request::Eligible { .. }), Response::Eligible(eligible)) => {
                // If ineligible, the native keeps its previous live person and
                // still removes it for exact mask 0x10. Do not reinterpret this.
                self.stage = if eligible {
                    Stage::Create
                } else {
                    Stage::Select
                };
            }
            (Some(Request::Create { .. }), Response::Person(person)) => {
                if person == Some(0) {
                    return Err("null person handle must be absent".into());
                }
                self.person = person;
                self.stage = Stage::Select;
            }
            (Some(Request::Clone { .. }), Response::Image { image, .. }) => {
                cache.assign_image(&mut self.local[0], image, host)?;
                self.stage = if image.is_some() {
                    Stage::Render
                } else {
                    Stage::Cleanup
                };
            }
            (Some(Request::Render { .. }), Response::Image { image, success }) => {
                let local = self.local_index()?;
                cache.assign_image(&mut self.local[local], image, host)?;
                self.rendered = success;
                self.stage = Stage::Rendered;
            }
            (Some(Request::Compose { .. }), Response::Image { image, .. }) => {
                cache.assign_image(&mut self.local[1], image, host)?;
                self.stage = Stage::Composed;
            }
            (Some(Request::Key { .. }), Response::Done) => self.stage = Stage::Store,
            (Some(Request::Persist { .. }), Response::Done) => self.stage = Stage::Persisted,
            (Some(Request::Remove { .. }), Response::Done) => self.stage = Stage::Flush,
            (Some(Request::Flush { .. }), Response::Done) => self.stage = Stage::Complete,
            _ => {
                return Err("portrait regeneration response does not match pending request".into());
            }
        }
        self.pending = None;
        Ok(())
    }
}
