//! 4cc6a3 / 4cc3b9: object speech-image retrieval and load/generate/persist
//! ordering. The resource/image host supplies pixels and storage. No missing
//! drawing service is converted into a successful empty image.
use super::{Event, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graphic {
    /// Object graphic discriminator at +24. Only 1 enters 4cc3b9.
    pub kind: i32,
    /// Borrowed registered definition handle, not a live object ID.
    pub resource: Option<u32>,
    /// The two owned wrappers at +30/+34.
    pub images: [Option<u32>; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// 457588: raw definition words 10 and 4.
    pub group: i16,
    pub base_graphic: i16,
    /// Signed raw definition word 41 (+52), captured before the first load.
    pub icon_base: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Definition {
        resource: u32,
    },
    Load {
        resource: u32,
        chunk: i32,
    },
    Template {
        chunk: u16,
    },
    /// 4e5cff updates the supplied wrappers/pixels. Its Boolean is ignored.
    Generate {
        resource: u32,
        images: [u32; 2],
    },
    /// Lock64, encode with 4b7d93, then unlock; None skips persistence.
    Encode {
        image: u32,
    },
    /// Add BMP_ with label Speech and converted=1. Return code is ignored.
    Add {
        resource: u32,
        chunk: i32,
        buffer: u32,
    },
    Error {
        resource: u32,
    },
    Flush {
        resource: u32,
    },
    Release {
        resource: u32,
        buffer: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Definition(Definition),
    /// Borrowed host owner. 5a5752/5a578d clear their raw output and return
    /// whether it is nonnull; a missing image leaves the old cached slot alone.
    Image(Option<u32>),
    Buffer(Option<u32>),
    /// The generation host applies any wrapper changes before acknowledging.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Definition,
    LoadSmall,
    LoadLarge,
    Decide,
    TemplateSmall,
    TemplateLarge,
    Generate,
    Encode,
    Add,
    ErrorAfterAdd,
    Flush,
    ErrorAfterFlush,
    Release,
    ErrorAfterRelease,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loading {
    base: Option<i16>,
    stage: Stage,
    cursor: usize,
    buffer: Option<u32>,
    pending: Option<Request>,
    returned: Option<bool>,
}

impl Default for Loading {
    fn default() -> Self {
        Self {
            base: None,
            stage: Stage::Start,
            cursor: 0,
            buffer: None,
            pending: None,
            returned: None,
        }
    }
}

fn resource(graphic: &Graphic) -> Result<u32, String> {
    graphic
        .resource
        .filter(|&r| r != 0)
        .ok_or_else(|| "object speech resource absent".into())
}

fn validate_owners(cache: &State, graphic: &Graphic, output: Option<u32>) -> Result<(), String> {
    cache.validate()?;
    if graphic.resource == Some(0) {
        return Err("null object speech resource must be absent".into());
    }
    let mut counts = vec![0usize; cache.refs.len()];
    for image in cache
        .entries
        .iter()
        .flat_map(|e| e.images)
        .chain(graphic.images)
        .chain([output])
        .flatten()
    {
        *counts
            .get_mut(image.wrapping_sub(1) as usize)
            .ok_or("object speech image absent")? += 1;
    }
    if counts
        .iter()
        .zip(&cache.refs)
        .any(|(&owners, &refs)| owners > refs as usize)
    {
        return Err("object speech ownership exceeds references".into());
    }
    Ok(())
}

impl Loading {
    pub fn returned(&self) -> Option<bool> {
        self.returned
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }

    fn chunk(&self, cursor: usize) -> i32 {
        i32::from(self.base.unwrap()) + (cursor as i32 + 1) * 2000
    }

    pub fn validate(&self, cache: &State, graphic: &Graphic) -> Result<(), String> {
        validate_owners(cache, graphic, None)?;
        let valid_stage = match (self.stage, self.pending) {
            (_, None) => true,
            (Stage::Definition, Some(Request::Definition { resource })) => resource != 0,
            (Stage::LoadSmall, Some(Request::Load { resource, chunk })) => {
                resource != 0 && self.base.is_some() && chunk == self.chunk(0)
            }
            (Stage::LoadLarge, Some(Request::Load { resource, chunk })) => {
                resource != 0 && self.base.is_some() && chunk == self.chunk(1)
            }
            (Stage::TemplateSmall, Some(Request::Template { chunk: 9000 })) => true,
            (Stage::TemplateLarge, Some(Request::Template { chunk: 9001 })) => true,
            (Stage::Generate, Some(Request::Generate { resource, images })) => {
                resource != 0 && images.iter().all(|&x| x != 0)
            }
            (Stage::Encode, Some(Request::Encode { image })) => image != 0,
            (
                Stage::Add,
                Some(Request::Add {
                    resource,
                    chunk,
                    buffer,
                }),
            ) => {
                resource != 0
                    && self.base.is_some()
                    && self.cursor < 2
                    && chunk == self.chunk(self.cursor)
                    && self.buffer == Some(buffer)
            }
            (
                Stage::ErrorAfterAdd | Stage::ErrorAfterFlush | Stage::ErrorAfterRelease,
                Some(Request::Error { resource }),
            ) => resource != 0,
            (Stage::Flush, Some(Request::Flush { resource })) => resource != 0,
            (Stage::Release, Some(Request::Release { resource, buffer })) => {
                resource != 0 && self.buffer == Some(buffer)
            }
            _ => false,
        };
        let holds_buffer = matches!(
            self.stage,
            Stage::Add
                | Stage::ErrorAfterAdd
                | Stage::Flush
                | Stage::ErrorAfterFlush
                | Stage::Release
        );
        if !valid_stage
            || self.returned.is_some() != (self.stage == Stage::Complete)
            || self.stage == Stage::Complete && self.pending.is_some()
            || !matches!(
                self.stage,
                Stage::Start | Stage::Definition | Stage::Complete
            ) && self.base.is_none()
            || self.cursor > 2
            || self.buffer == Some(0)
            || holds_buffer != self.buffer.is_some()
        {
            return Err("invalid object speech loader continuation".into());
        }
        Ok(())
    }

    fn complete(&mut self, value: bool) {
        self.stage = Stage::Complete;
        self.returned = Some(value);
    }

    /// force is the current native global flag, read after both saved-image
    /// loads. Resource handles are reread at each native service call.
    pub fn advance(
        &mut self,
        cache: &mut State,
        graphic: &mut Graphic,
        force: bool,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<Option<Request>, String> {
        self.validate(cache, graphic)?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if graphic.kind != 1 || graphic.resource.is_none() {
                        self.complete(false);
                        continue;
                    }
                    self.stage = Stage::Definition;
                    Some(Request::Definition {
                        resource: resource(graphic)?,
                    })
                }
                Stage::Definition => return Err("object speech definition reply absent".into()),
                Stage::LoadSmall => Some(Request::Load {
                    resource: resource(graphic)?,
                    chunk: self.chunk(0),
                }),
                Stage::LoadLarge => Some(Request::Load {
                    resource: resource(graphic)?,
                    chunk: self.chunk(1),
                }),
                Stage::Decide => {
                    if !force && graphic.images.iter().all(Option::is_some) {
                        self.complete(true);
                        continue;
                    }
                    cache.assign_image(&mut graphic.images[0], None, host)?;
                    cache.assign_image(&mut graphic.images[1], None, host)?;
                    self.stage = Stage::TemplateSmall;
                    None
                }
                Stage::TemplateSmall => Some(Request::Template { chunk: 9000 }),
                Stage::TemplateLarge => Some(Request::Template { chunk: 9001 }),
                Stage::Generate => {
                    if let [Some(small), Some(large)] = graphic.images {
                        Some(Request::Generate {
                            resource: resource(graphic)?,
                            images: [small, large],
                        })
                    } else {
                        self.stage = Stage::Encode;
                        None
                    }
                }
                Stage::Encode => {
                    if self.cursor == 2 {
                        self.complete(graphic.images.iter().all(Option::is_some));
                        continue;
                    }
                    if let Some(image) = graphic.images[self.cursor] {
                        Some(Request::Encode { image })
                    } else {
                        self.cursor += 1;
                        None
                    }
                }
                Stage::Add => Some(Request::Add {
                    resource: resource(graphic)?,
                    chunk: self.chunk(self.cursor),
                    buffer: self.buffer.unwrap(),
                }),
                Stage::ErrorAfterAdd | Stage::ErrorAfterFlush | Stage::ErrorAfterRelease => {
                    Some(Request::Error {
                        resource: resource(graphic)?,
                    })
                }
                Stage::Flush => Some(Request::Flush {
                    resource: resource(graphic)?,
                }),
                Stage::Release => Some(Request::Release {
                    resource: resource(graphic)?,
                    buffer: self.buffer.unwrap(),
                }),
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
        graphic: &mut Graphic,
        response: Response,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<(), String> {
        self.validate(cache, graphic)?;
        match (self.pending, response) {
            (Some(Request::Definition { .. }), Response::Definition(info)) => {
                if info.group == 0 && info.base_graphic == 0 {
                    self.complete(false);
                } else {
                    self.base = Some(info.icon_base);
                    self.stage = Stage::LoadSmall;
                }
            }
            (Some(Request::Load { .. } | Request::Template { .. }), Response::Image(image)) => {
                let (slot, next) = match self.stage {
                    Stage::LoadSmall => (0, Stage::LoadLarge),
                    Stage::LoadLarge => (1, Stage::Decide),
                    Stage::TemplateSmall => (0, Stage::TemplateLarge),
                    Stage::TemplateLarge => (1, Stage::Generate),
                    _ => return Err("object speech image stage invalid".into()),
                };
                if image.is_some() {
                    cache.assign_image(&mut graphic.images[slot], image, host)?;
                }
                self.stage = next;
            }
            (Some(Request::Generate { .. }), Response::Done) => self.stage = Stage::Encode,
            (Some(Request::Encode { .. }), Response::Buffer(buffer)) => {
                if buffer == Some(0) {
                    return Err("null encoded speech buffer must be absent".into());
                }
                self.buffer = buffer;
                if buffer.is_some() {
                    self.stage = Stage::Add;
                } else {
                    self.cursor += 1;
                }
            }
            (Some(Request::Add { .. }), Response::Done) => self.stage = Stage::ErrorAfterAdd,
            (Some(Request::Error { .. }), Response::Done) => {
                self.stage = match self.stage {
                    Stage::ErrorAfterAdd => Stage::Flush,
                    Stage::ErrorAfterFlush => Stage::Release,
                    Stage::ErrorAfterRelease => {
                        self.cursor += 1;
                        Stage::Encode
                    }
                    _ => return Err("object speech error query stage invalid".into()),
                }
            }
            (Some(Request::Flush { .. }), Response::Done) => self.stage = Stage::ErrorAfterFlush,
            (Some(Request::Release { .. }), Response::Done) => {
                self.buffer = None;
                self.stage = Stage::ErrorAfterRelease;
            }
            _ => return Err("object speech response does not match pending request".into()),
        }
        self.pending = None;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retrieval {
    zoom: i32,
    output: Option<u32>,
    started: bool,
    loader: Option<Loading>,
    returned: Option<bool>,
}
impl Retrieval {
    pub fn returned(&self) -> Option<bool> {
        self.returned
    }
    pub fn pending(&self) -> Option<Request> {
        self.loader.as_ref().and_then(Loading::pending)
    }
    pub fn new(zoom: i32, output: Option<u32>) -> Self {
        Self {
            zoom,
            output,
            started: false,
            loader: None,
            returned: None,
        }
    }
    pub fn validate(&self, cache: &State, graphic: &Graphic) -> Result<(), String> {
        validate_owners(cache, graphic, self.output)?;
        if !self.started && self.returned.is_some()
            || self.loader.is_some()
                && (!self.started || self.returned.is_some() || !(2..=3).contains(&self.zoom))
            || self.returned == Some(true)
                && (!(2..=3).contains(&self.zoom) || self.output.is_none())
        {
            return Err("invalid object speech retrieval".into());
        }
        if let Some(loader) = &self.loader {
            loader.validate(cache, graphic)?;
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        cache: &mut State,
        graphic: &mut Graphic,
        force: bool,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<Option<Request>, String> {
        self.validate(cache, graphic)?;
        if self.returned.is_some() {
            return Ok(None);
        }
        if !self.started {
            self.started = true;
            if !(2..=3).contains(&self.zoom) {
                self.returned = Some(false);
                return Ok(None);
            }
            // Native tests SMALL even when the caller requests LARGE.
            if graphic.images[0].is_none() {
                self.loader = Some(Loading::default());
            }
        }
        if let Some(loader) = &mut self.loader {
            if let Some(request) = loader.advance(cache, graphic, force, host)? {
                return Ok(Some(request));
            }
            self.loader = None; // Original ignores the loader's Boolean.
        }
        cache.assign_image(
            &mut self.output,
            graphic.images[(self.zoom - 2) as usize],
            host,
        )?;
        self.returned = Some(self.output.is_some());
        Ok(None)
    }
    pub fn respond(
        &mut self,
        cache: &mut State,
        graphic: &mut Graphic,
        response: Response,
        host: &mut impl FnMut(Event, &State),
    ) -> Result<(), String> {
        self.validate(cache, graphic)?;
        self.loader
            .as_mut()
            .ok_or("object speech loader absent")?
            .respond(cache, graphic, response, host)
    }
    pub fn finish(self) -> Result<(bool, Option<u32>), String> {
        self.returned
            .map(|r| (r, self.output))
            .ok_or("object speech retrieval incomplete".into())
    }
}
