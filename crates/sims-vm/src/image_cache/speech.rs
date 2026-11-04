//! 4e915d: retrieve the small/large person image used in a headline. Unlike
//! 4e92a8's dialog portrait, this selects slots 4/5 and never composes a frame.
//! Bitmap loading and mask-4 regeneration remain explicit host operations.
use super::{Event, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Refresh { index: u32, persist: bool },
    Load { index: u32, chunk: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// The host has applied regeneration's effects. Its Boolean is ignored.
    Refreshed,
    /// Borrowed loader output. The wrapper, rather than the loader's Boolean,
    /// determines success; this operation acquires a reference of its own.
    Image(Option<u32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Cached,
    Loaded,
    Regenerated,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speech {
    index: u32,
    zoom: i32,
    allow_regeneration: bool,
    output: Option<u32>,
    stage: Stage,
    pending: Option<Request>,
    returned: Option<bool>,
}

impl Speech {
    /// Transfers the caller's output wrapper. Invalid zoom/index and dirty
    /// cache entries with regeneration disabled leave that wrapper intact.
    pub fn new(index: u32, zoom: i32, output: Option<u32>, allow_regeneration: bool) -> Self {
        Self {
            index,
            zoom,
            allow_regeneration,
            output,
            stage: Stage::Start,
            pending: None,
            returned: None,
        }
    }

    fn selection(&self) -> Option<(usize, u16)> {
        match self.zoom {
            2 => Some((4, 2005)),
            3 => Some((5, 2006)),
            _ => None,
        }
    }

    pub fn output(&self) -> Option<u32> {
        self.output
    }

    pub fn returned(&self) -> Option<bool> {
        self.returned
    }

    pub fn pending(&self) -> Option<Request> {
        self.pending
    }

    pub fn validate(&self, cache: &State) -> Result<(), String> {
        cache.validate()?;
        let selected = self.selection();
        let valid = selected.is_some() && (self.index as usize) < cache.entries.len();
        if self.returned.is_some() != (self.stage == Stage::Complete)
            || self.stage == Stage::Complete && self.pending.is_some()
            || self.returned == Some(true) && (!valid || self.output.is_none())
            || !matches!(self.stage, Stage::Start | Stage::Complete) && !valid
            || self.stage == Stage::Regenerated && !self.allow_regeneration
            || self.pending.is_some_and(|request| match request {
                Request::Refresh { index, persist } => {
                    index != self.index
                        || !self.allow_regeneration
                        || self.stage
                            != if persist {
                                Stage::Regenerated
                            } else {
                                Stage::Cached
                            }
                }
                Request::Load { index, chunk } => {
                    index != self.index
                        || selected.map(|(_, c)| c) != Some(chunk)
                        || self.stage != Stage::Loaded
                        || self.output.is_some()
                }
            })
        {
            return Err("invalid speech image continuation".into());
        }
        // Cache validation does not account for this continuation's local owner.
        if let Some(image) = self.output {
            let refs = cache
                .refs
                .get(image.wrapping_sub(1) as usize)
                .ok_or("speech image absent")?;
            let owners = cache
                .entries
                .iter()
                .flat_map(|e| e.images)
                .filter(|&v| v == Some(image))
                .count();
            if *refs as usize <= owners {
                return Err("speech image ownership exceeds references".into());
            }
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
            let request = match self.stage {
                Stage::Start => {
                    let Some((slot, _)) = self
                        .selection()
                        .filter(|_| (self.index as usize) < cache.entries.len())
                    else {
                        self.complete(false);
                        continue;
                    };
                    let dirty = cache.entries[self.index as usize].dirty[slot];
                    if dirty && !self.allow_regeneration {
                        self.complete(false);
                        continue;
                    }
                    self.stage = Stage::Cached;
                    dirty.then_some(Request::Refresh {
                        index: self.index,
                        persist: false,
                    })
                }
                Stage::Cached => {
                    let (slot, chunk) = self.selection().unwrap();
                    let image = cache.entries[self.index as usize].images[slot];
                    cache.assign_image(&mut self.output, image, host)?;
                    if self.output.is_some() {
                        self.complete(true);
                        None
                    } else {
                        self.stage = Stage::Loaded;
                        Some(Request::Load {
                            index: self.index,
                            chunk,
                        })
                    }
                }
                Stage::Loaded => {
                    if self.output.is_some() {
                        let (slot, _) = self.selection().unwrap();
                        cache.store_image(self.index as usize, slot, self.output, host)?;
                        self.complete(true);
                        None
                    } else if self.allow_regeneration {
                        self.stage = Stage::Regenerated;
                        Some(Request::Refresh {
                            index: self.index,
                            persist: true,
                        })
                    } else {
                        self.complete(false);
                        None
                    }
                }
                Stage::Regenerated => {
                    let (slot, _) = self.selection().unwrap();
                    let image = cache.entries[self.index as usize].images[slot];
                    cache.assign_image(&mut self.output, image, host)?;
                    self.complete(self.output.is_some());
                    None
                }
                Stage::Complete => return Ok(None),
            };
            if request.is_some() {
                self.pending = request;
                return Ok(request);
            }
        }
    }

    fn complete(&mut self, returned: bool) {
        self.returned = Some(returned);
        self.stage = Stage::Complete;
    }

    pub fn respond(
        &mut self,
        cache: &mut State,
        response: Response,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<(), String> {
        self.validate(cache)?;
        match (self.pending, response) {
            (Some(Request::Refresh { .. }), Response::Refreshed) => {}
            (Some(Request::Load { .. }), Response::Image(image)) => {
                cache.assign_image(&mut self.output, image, host)?;
            }
            _ => return Err("speech image response does not match pending request".into()),
        }
        self.pending = None;
        Ok(())
    }

    /// Transfer the completed wrapper back without retaining/releasing it.
    pub fn finish(self) -> Result<(bool, Option<u32>), String> {
        self.returned
            .map(|r| (r, self.output))
            .ok_or("speech image retrieval has not completed".into())
    }
}
