//! Original Options5179e0/517a4a/517a6f. Child window/tab/type and
//! People/auxiliary work has explicit caller-owned returns, never fake success.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    BaseShow(u32),
    SelectTab { window: u32, value: i32 },
    TutorialReceiver(Option<u32>),
    TutorialEnabled { receiver: u32, value: u8 },
    FlashPermission,
    Flash { window: u32, value: u32 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Done,
    Receiver(Option<u32>),
    Number(u32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub selected_tab: i32,
    pub tutorial_source: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Base,
    Tab,
    Cast,
    Tutorial,
    Permission,
    Flash,
    Returned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Show {
    window: u32,
    stage: Stage,
    pending: Option<Request>,
    #[serde(default)]
    value: Option<u32>,
}
impl Show {
    pub fn new(window: u32) -> Result<Self, String> {
        if window == 0 {
            return Err("Options window absent".into());
        }
        Ok(Self {
            window,
            stage: Stage::Base,
            pending: None,
            value: None,
        })
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    pub fn validate(&self) -> Result<(), String> {
        let pending_valid = match self.pending {
            None => true,
            Some(Request::BaseShow(id)) => self.stage == Stage::Tab && id == self.window,
            Some(Request::SelectTab { window, value: 0 }) => {
                self.stage == Stage::Cast && window == self.window
            }
            Some(Request::TutorialReceiver(source)) => {
                self.stage == Stage::Tutorial && source != Some(0)
            }
            Some(Request::TutorialEnabled { receiver, value }) => {
                self.stage == Stage::Permission && receiver != 0 && value <= 1
            }
            Some(Request::FlashPermission) => self.stage == Stage::Flash,
            Some(Request::Flash { window, .. }) => {
                self.stage == Stage::Returned && window == self.window
            }
            _ => false,
        };
        if self.window == 0 || !pending_valid || (self.complete() && self.pending.is_some()) {
            return Err("invalid Options Show continuation".into());
        }
        let owns_getter =
            matches!(self.stage, Stage::Tutorial | Stage::Flash) && self.pending.is_none();
        if self.value.is_some() != owns_getter {
            return Err("Options Show lost its getter return".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Options Show return differs".into());
        }
        let value = match (request, reply) {
            (Request::TutorialReceiver(_), Reply::Receiver(owner)) if owner != Some(0) => {
                Some(owner.unwrap_or(0))
            }
            (Request::FlashPermission, Reply::Number(value)) => Some(value),
            (
                Request::BaseShow(_)
                | Request::SelectTab { .. }
                | Request::TutorialEnabled { .. }
                | Request::Flash { .. },
                Reply::Done,
            ) => None,
            _ => return Err("Options Show reply kind differs".into()),
        };
        self.value = value;
        self.pending = None;
        Ok(())
    }
    /// Globals are read after the preceding child returns, as at517a23.
    pub fn advance(&mut self, fields: &Fields, global26: i16) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        let request = loop {
            match self.stage {
                Stage::Base => {
                    self.stage = Stage::Tab;
                    break Request::BaseShow(self.window);
                }
                Stage::Tab => {
                    self.stage = Stage::Cast;
                    if fields.selected_tab != 0 {
                        break Request::SelectTab {
                            window: self.window,
                            value: 0,
                        };
                    }
                }
                Stage::Cast => {
                    if fields.tutorial_source == Some(0) {
                        return Err("null Options tutorial identity".into());
                    }
                    self.stage = Stage::Tutorial;
                    break Request::TutorialReceiver(fields.tutorial_source);
                }
                Stage::Tutorial => {
                    let receiver = self.value.take().ok_or("Options tutorial return absent")?;
                    self.stage = Stage::Permission;
                    if receiver != 0 {
                        break Request::TutorialEnabled {
                            receiver,
                            value: u8::from(global26 == 0),
                        };
                    }
                }
                Stage::Permission => {
                    self.stage = Stage::Flash;
                    break Request::FlashPermission;
                }
                Stage::Flash => {
                    let value = self
                        .value
                        .take()
                        .ok_or("Options permission return absent")?;
                    self.stage = Stage::Returned;
                    break Request::Flash {
                        window: self.window,
                        value,
                    };
                }
                Stage::Returned => {
                    self.stage = Stage::Complete;
                    return Ok(None);
                }
                Stage::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        self.validate()?;
        Ok(self.pending)
    }
}

///4e629d's semantically consumed AL; native EAX also carries pointer upper
///bytes. Callers forwarding that full register can retain it through Show.
pub fn flash_permission(lot_present: bool, global26: i16, preference: u8) -> u8 {
    if lot_present && global26 != 0 {
        0
    } else {
        preference
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TutorialControl {
    pub child: Option<u32>,
    pub enabled: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnableTutorial {
    value: u8,
    started: bool,
    pending: Option<u32>,
    complete: bool,
}
impl EnableTutorial {
    pub fn new(value: u8) -> Self {
        Self {
            value,
            started: false,
            pending: None,
            complete: false,
        }
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.pending == Some(0)
            || (self.complete && (!self.started || self.pending.is_some()))
            || (self.pending.is_some() && !self.started)
        {
            return Err("invalid tutorial enable continuation".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, field: &mut TutorialControl) -> Result<Option<u32>, String> {
        self.validate()?;
        if self.pending.is_some() || self.complete {
            return Ok(self.pending);
        }
        if !self.started {
            if field.child == Some(0) {
                return Err("tutorial control child absent".into());
            }
            self.started = true;
            if let Some(child) = field.child {
                if field.enabled != self.value {
                    field.enabled = self.value;
                    self.pending = Some(child);
                    return Ok(self.pending);
                }
            }
        }
        self.complete = true;
        Ok(None)
    }
    pub fn acknowledge(&mut self, child: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(child) {
            return Err("tutorial invalidation return differs".into());
        }
        self.pending = None;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashControl {
    pub active: u32,
    pub interval: u32,
    pub phase: u8,
}
impl FlashControl {
    fn assign(&mut self, interval: u32, active: u32) {
        self.active = if interval == 0 {
            0
        } else {
            self.interval = interval;
            active
        };
        self.phase = 0;
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashFields {
    ///Offsets f8(6),12c(3),144(4),168(4). Null entries are skipped.
    pub groups: [Vec<Option<u32>>; 4],
    pub special: u32,
    pub controls: BTreeMap<u32, FlashControl>,
}
impl FlashFields {
    fn validate(&self) -> Result<(), String> {
        if self.groups.iter().map(Vec::len).collect::<Vec<_>>() != [6, 3, 4, 4]
            || self.special == 0
            || !self.controls.contains_key(&self.special)
            || self
                .groups
                .iter()
                .flatten()
                .flatten()
                .any(|id| *id == 0 || !self.controls.contains_key(id))
        {
            return Err("invalid Options flashing owner fields".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashInputs {
    pub interval: u32,
    pub people: Option<u32>,
    ///The already-guarded App+88+f4 receiver, read after People returns.
    pub auxiliary: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlashRequest {
    People { owner: u32, value: u32 },
    Auxiliary { owner: u32, value: u32 },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flash {
    value: u32,
    stage: u8,
    pending: Option<FlashRequest>,
}
impl Flash {
    pub fn new(value: u32) -> Self {
        Self {
            value,
            stage: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == 3 && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = match self.pending {
            None => true,
            Some(FlashRequest::People { owner, value }) => {
                self.stage == 2 && owner != 0 && value == self.value
            }
            Some(FlashRequest::Auxiliary { owner, value }) => {
                self.stage == 3 && owner != 0 && value == self.value
            }
        };
        if self.stage > 3 || !valid {
            return Err("invalid Options flashing continuation".into());
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut FlashFields,
        input: FlashInputs,
    ) -> Result<Option<FlashRequest>, String> {
        self.validate()?;
        fields.validate()?;
        if input.people == Some(0) || input.auxiliary == Some(0) {
            return Err("null flashing child identity".into());
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        if self.stage == 0 {
            for id in fields.groups.iter().flatten().flatten() {
                fields
                    .controls
                    .get_mut(id)
                    .unwrap()
                    .assign(input.interval, u32::from(self.value as u8 != 0));
            }
            fields
                .controls
                .get_mut(&fields.special)
                .unwrap()
                .assign(input.interval, 1);
            self.stage = 1;
        }
        if self.stage == 1 {
            self.stage = 2;
            if let Some(owner) = input.people {
                self.pending = Some(FlashRequest::People {
                    owner,
                    value: self.value,
                });
                return Ok(self.pending);
            }
        }
        if self.stage == 2 {
            self.stage = 3;
            if let Some(owner) = input.auxiliary {
                self.pending = Some(FlashRequest::Auxiliary {
                    owner,
                    value: self.value,
                });
                return Ok(self.pending);
            }
        }
        Ok(None)
    }
    pub fn acknowledge(&mut self, request: FlashRequest) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("Options flashing child return differs".into());
        }
        self.pending = None;
        Ok(())
    }
}
