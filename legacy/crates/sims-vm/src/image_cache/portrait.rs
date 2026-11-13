//! 4e92a8: person-portrait retrieval with explicit host service boundaries.
//! Regeneration, saved BMP decoding and composition belong to the asset host.
//! The caller retains this continuation (including its image owners) when a
//! service suspends. No missing service is replaced with an empty result.
use super::{Event, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Refresh { index: u32, persist: bool },
    Load { index: u32, chunk: u16 },
    Compose { faces: u32, background: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// The host has applied regeneration's cache effects. The native caller
    /// ignores that routine's Boolean return, including partial failures.
    Refreshed,
    /// Borrowed host image; the continuation acquires its own reference. The
    /// native caller tests the resulting wrapper, not the loader's Boolean.
    Image(Option<u32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Cached,
    Loaded,
    Composed,
    Regenerated,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Portrait {
    index: u32,
    /// Borrowed global owner. Its lifetime must cover this synchronous native
    /// operation; the host must preserve that owner across suspension.
    background: Option<u32>,
    output: Option<u32>,
    loaded: Option<u32>,
    stage: Stage,
    pending: Option<Request>,
    returned: Option<bool>,
}

impl Portrait {
    /// Transfer ownership of the caller's existing output wrapper. Invalid
    /// indexes return false without replacing that wrapper, as in the native.
    pub fn new(index: u32, output: Option<u32>, background: Option<u32>) -> Self {
        Self {
            index,
            background,
            output,
            loaded: None,
            stage: Stage::Start,
            pending: None,
            returned: None,
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

    pub fn background(&self) -> Option<u32> {
        self.background
    }

    pub fn validate(&self, cache: &State) -> Result<(), String> {
        cache.validate()?;
        if self.returned.is_some() != (self.stage == Stage::Complete)
            || self.stage == Stage::Complete
                && (self.pending.is_some()
                    || self.returned
                        != Some(
                            (self.index as usize) < cache.entries.len() && self.output.is_some(),
                        ))
            || self.stage == Stage::Composed && self.loaded.is_none()
            || matches!(self.stage, Stage::Loaded | Stage::Regenerated) && self.output.is_some()
            || (self.stage != Stage::Start
                && self.stage != Stage::Complete
                && self.index as usize >= cache.entries.len())
            || self.loaded.is_some() && !matches!(self.stage, Stage::Loaded | Stage::Composed)
            || self.pending.is_some_and(|request| match request {
                Request::Refresh { index, persist } => {
                    index != self.index
                        || self.stage
                            != if persist {
                                Stage::Regenerated
                            } else {
                                Stage::Cached
                            }
                }
                Request::Load { index, chunk } => {
                    index != self.index
                        || chunk != 2002
                        || self.stage != Stage::Loaded
                        || self.loaded.is_some()
                }
                Request::Compose { faces, background } => {
                    self.stage != Stage::Composed
                        || self.loaded != Some(faces)
                        || self.background != Some(background)
                        || self.output.is_some()
                }
            })
        {
            return Err("invalid portrait continuation".into());
        }
        // Include the two local wrappers as well as the borrowed global owner.
        // Cache validation alone cannot detect lost ownership of these images.
        let mut counts = vec![0u32; cache.refs.len()];
        for image in cache
            .entries
            .iter()
            .flat_map(|e| e.images)
            .chain([self.output, self.loaded, self.background])
            .flatten()
        {
            let count = counts
                .get_mut(image.wrapping_sub(1) as usize)
                .ok_or("portrait image absent")?;
            *count += 1;
        }
        if counts
            .iter()
            .zip(&cache.refs)
            .any(|(owners, refs)| owners > refs)
        {
            return Err("portrait ownership exceeds references".into());
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
                    if self.index as usize >= cache.entries.len() {
                        self.returned = Some(false);
                        self.stage = Stage::Complete;
                        continue;
                    }
                    self.stage = Stage::Cached;
                    cache.entries[self.index as usize].dirty[1].then_some(Request::Refresh {
                        index: self.index,
                        persist: false,
                    })
                }
                Stage::Cached => {
                    let image = cache.entries[self.index as usize].images[1];
                    cache.assign_image(&mut self.output, image, host)?;
                    if self.output.is_some() {
                        self.returned = Some(true);
                        self.stage = Stage::Complete;
                        None
                    } else {
                        self.stage = Stage::Loaded;
                        Some(Request::Load {
                            index: self.index,
                            chunk: 2002,
                        })
                    }
                }
                Stage::Loaded => {
                    if let Some(faces) = self.loaded {
                        cache.store_image(self.index as usize, 0, Some(faces), host)?;
                        self.stage = Stage::Composed;
                        self.background
                            .map(|background| Request::Compose { faces, background })
                    } else {
                        self.stage = Stage::Regenerated;
                        Some(Request::Refresh {
                            index: self.index,
                            persist: true,
                        })
                    }
                }
                Stage::Composed => {
                    if self.background.is_some() {
                        cache.store_image(self.index as usize, 1, self.output, host)?;
                    }
                    cache.assign_image(&mut self.loaded, None, host)?;
                    self.returned = Some(self.output.is_some());
                    self.stage = Stage::Complete;
                    None
                }
                Stage::Regenerated => {
                    let image = cache.entries[self.index as usize].images[1];
                    cache.assign_image(&mut self.output, image, host)?;
                    cache.assign_image(&mut self.loaded, None, host)?;
                    self.returned = Some(self.output.is_some());
                    self.stage = Stage::Complete;
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
                cache.assign_image(&mut self.loaded, image, host)?;
            }
            (Some(Request::Compose { .. }), Response::Image(image)) => {
                cache.assign_image(&mut self.output, image, host)?;
            }
            _ => return Err("portrait response does not match pending request".into()),
        }
        self.pending = None;
        Ok(())
    }

    /// Transfer the completed output wrapper back to the caller. No reference
    /// is added or removed; incomplete requests cannot lose their local owners.
    pub fn finish(self) -> Result<(bool, Option<u32>), String> {
        self.returned
            .map(|returned| (returned, self.output))
            .ok_or("portrait request has not completed".into())
    }
}
