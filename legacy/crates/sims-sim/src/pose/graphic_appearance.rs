//! Base Animator appearance reload, 585555 (ordinary 3D objects/indicators).
//! The profile uses the same 5874ca descriptor decoder as person appearances.
//! Record replacement and material tint are ordered services, not assumed
//! successful just because an appearance name was resolved.
use super::appearance_selection::Descriptor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// +9c, compared with the sign-extended owner object-data word 68 (+d2).
    pub cached: i32,
    pub dirty: bool,
    /// +f8, +fc and +f7. Unrecognized names retain these previous values.
    pub marker: bool,
    pub kind: u32,
    pub ordinary_material: bool,
    pub tint: [u8; 3],
}
impl State {
    /// Defined by 58464f, before owner/model initialization.
    pub fn constructed() -> Self {
        Self {
            cached: -1,
            dirty: true,
            marker: false,
            kind: 0,
            ordinary_material: true,
            tint: [0; 3],
        }
    }
}

/// Stable identities returned by the live model getter; not memory addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Model {
    pub model: u64,
    pub skin: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Resource +0c, one-based string index 2, primary descriptor.
    Descriptor,
    /// 583559 on the current first generic record. Its success flag is unused.
    Bind {
        skin: u64,
        descriptor: Descriptor,
    },
    Tint {
        model: u64,
        tint: [u8; 3],
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Descriptor(Descriptor),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Descriptor,
    Bind,
    Classify,
    Tint,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reload {
    force: bool,
    stage: Stage,
    descriptor: Option<Descriptor>,
    pending: Option<Request>,
}
impl Reload {
    pub fn new(force: bool) -> Self {
        Self {
            force,
            stage: Stage::Start,
            descriptor: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.stage, Stage::Bind | Stage::Classify | Stage::Tint)
            && self.descriptor.is_none()
            || self
                .pending
                .as_ref()
                .is_some_and(|request| match (self.stage, request) {
                    (Stage::Descriptor, Request::Descriptor) => false,
                    (Stage::Bind, Request::Bind { skin, descriptor }) => {
                        *skin == 0 || Some(descriptor) != self.descriptor.as_ref()
                    }
                    (Stage::Tint, Request::Tint { model, .. }) => *model == 0,
                    _ => true,
                })
        {
            return Err("invalid base Animator appearance continuation".into());
        }
        Ok(())
    }
    /// Re-supply the live owner's word and model getter on each call. Neither
    /// is consumed while a service is pending; callbacks may replace the model.
    pub fn advance(
        &mut self,
        state: &mut State,
        index: i16,
        model: Option<Model>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if !self.force && state.cached == i32::from(index) {
                        self.stage = Stage::Done;
                    } else {
                        state.dirty = true;
                        state.cached = i32::from(index);
                        self.stage = Stage::Descriptor;
                    }
                    continue;
                }
                Stage::Descriptor => Request::Descriptor,
                Stage::Bind => {
                    let Some(model) = model else {
                        self.stage = Stage::Done;
                        continue;
                    };
                    if model.model == 0 || model.skin == 0 {
                        return Err("invalid base Animator model identity".into());
                    }
                    Request::Bind {
                        skin: model.skin,
                        descriptor: self.descriptor.clone().unwrap(),
                    }
                }
                Stage::Classify => {
                    let kind = match self.descriptor.as_ref().unwrap().name.as_str() {
                        "head-arrow" => Some(1),
                        "head-arrow-date" => Some(2),
                        "head-star" => Some(3),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        state.marker = true;
                        state.kind = kind;
                        state.ordinary_material = false;
                    }
                    self.stage = Stage::Tint;
                    continue;
                }
                Stage::Tint => {
                    let model = model.ok_or("base Animator model disappeared before tint")?;
                    if model.model == 0 {
                        return Err("invalid base Animator model identity".into());
                    }
                    Request::Tint {
                        model: model.model,
                        tint: state.tint,
                    }
                }
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending.clone());
        }
    }
    pub fn acknowledge(&mut self, request: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale base Animator appearance acknowledgment".into());
        }
        match (request, reply) {
            (Request::Descriptor, Reply::Descriptor(descriptor)) => {
                self.descriptor = Some(descriptor);
                self.stage = Stage::Bind;
            }
            (Request::Bind { .. }, Reply::Done) => self.stage = Stage::Classify,
            (Request::Tint { .. }, Reply::Done) => self.stage = Stage::Done,
            _ => return Err("mismatched base Animator appearance reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
