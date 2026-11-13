//! Base Animator owner/profile/model initialization: 584cb5/584ba6/584a0d.
//! Each request is an actual service boundary. Acknowledgments follow completed
//! host work; model-bind failure retains the created model just as native does.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Native owner fields touched by these controllers. Other Animator storage is
/// retained by the caller. The flags are the raw +d8..+db bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub owner: u64,
    pub model: Option<u64>,
    pub profile: Option<u64>,
    pub flags: [u8; 4],
    pub appearance_index: i32,
    pub selection: i32,
    pub foreground: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Owner(u64),
    Profile,
    Model,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ClearAppearances,
    Profile {
        owner: u64,
    },
    ReloadAppearance {
        force: bool,
    },
    DestroyAccessories,
    /// 4704fa(profile, 1, -1), before defaulting an empty name to "root".
    Skeleton {
        profile: u64,
    },
    /// Allocate and construct 5a0646. Store its returned identity before Bind.
    ConstructModel,
    BindModel {
        model: u64,
        skeleton: String,
    },
    Reset,
    BindAccessories,
    Placement,
    /// The global camera's +28..+34 rectangle, not virtual +5c.
    CameraViewport,
    Visibility {
        viewport: [i32; 4],
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Profile(Option<u64>),
    Skeleton(String),
    Model(u64),
    Bound(i32),
    Viewport([i32; 4]),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Clear,
    Profile,
    Appearance,
    Destroy,
    ProfileEnd,
    Model,
    Skeleton,
    Construct,
    Bind,
    Register,
    Reset,
    Accessories,
    Finish,
    Placement,
    Viewport,
    Visibility,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    entry: Entry,
    graphic: u64,
    stage: Stage,
    skeleton: Option<String>,
    viewport: Option<[i32; 4]>,
    pending: Option<Request>,
    bind_status: Option<i32>,
}
impl Update {
    pub fn new(graphic: u64, entry: Entry) -> Self {
        Self {
            entry,
            graphic,
            stage: Stage::Start,
            skeleton: None,
            viewport: None,
            pending: None,
            bind_status: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    /// Owner binding's native return is always 1, even when model binding
    /// fails. Keep the actual bind code separately instead of conflating them.
    pub fn bind_status(&self) -> Option<i32> {
        self.bind_status
    }
    pub fn validate(&self) -> Result<(), String> {
        let bad = self
            .pending
            .as_ref()
            .is_some_and(|r| match (self.stage, r) {
                (Stage::Clear, Request::ClearAppearances)
                | (Stage::Destroy, Request::DestroyAccessories)
                | (Stage::Construct, Request::ConstructModel)
                | (Stage::Reset, Request::Reset)
                | (Stage::Accessories, Request::BindAccessories)
                | (Stage::Placement, Request::Placement)
                | (Stage::Viewport, Request::CameraViewport) => false,
                (Stage::Profile, Request::Profile { owner }) => *owner == 0,
                (Stage::Appearance, Request::ReloadAppearance { force }) => *force,
                (Stage::Skeleton, Request::Skeleton { profile }) => *profile == 0,
                (Stage::Bind, Request::BindModel { model, skeleton }) => {
                    *model == 0 || Some(skeleton) != self.skeleton.as_ref()
                }
                (Stage::Visibility, Request::Visibility { viewport }) => {
                    Some(*viewport) != self.viewport
                }
                _ => true,
            });
        if self.graphic == 0
            || matches!(self.entry, Entry::Owner(0))
            || bad
            || matches!(self.stage, Stage::Construct | Stage::Bind) && self.skeleton.is_none()
            || matches!(
                self.stage,
                Stage::Register
                    | Stage::Reset
                    | Stage::Accessories
                    | Stage::Finish
                    | Stage::Placement
                    | Stage::Viewport
                    | Stage::Visibility
            ) && self.bind_status != Some(0)
        {
            return Err("invalid base Animator initializer continuation".into());
        }
        Ok(())
    }
    /// `model_callbacks` is the retained model +14 owner storage, indexed by
    /// stable model identity. Registration is an inline store, not a callback.
    pub fn advance(
        &mut self,
        state: &mut State,
        model_callbacks: &mut BTreeMap<u64, Option<u64>>,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Start => {
                    if let Entry::Owner(owner) = self.entry {
                        state.owner = owner;
                    }
                    if self.entry == Entry::Model {
                        self.stage = Stage::Model;
                    } else {
                        state.flags[1] = 1;
                        self.stage = Stage::Clear;
                    }
                    continue;
                }
                Stage::Clear => Request::ClearAppearances,
                Stage::Profile => {
                    if state.owner == 0 {
                        return Err("base Animator profile owner absent".into());
                    }
                    Request::Profile { owner: state.owner }
                }
                Stage::Appearance => Request::ReloadAppearance { force: false },
                Stage::Destroy => Request::DestroyAccessories,
                Stage::ProfileEnd => {
                    state.appearance_index = -1;
                    state.flags[1] = 1;
                    self.stage = if self.entry == Entry::Profile {
                        Stage::Done
                    } else {
                        Stage::Model
                    };
                    continue;
                }
                Stage::Model => {
                    self.stage = if state.model.is_some() {
                        Stage::Done
                    } else {
                        Stage::Skeleton
                    };
                    continue;
                }
                Stage::Skeleton => Request::Skeleton {
                    profile: state
                        .profile
                        .filter(|p| *p != 0)
                        .ok_or("base Animator skeleton profile absent")?,
                },
                Stage::Construct => Request::ConstructModel,
                Stage::Bind => Request::BindModel {
                    model: state
                        .model
                        .filter(|p| *p != 0)
                        .ok_or("base Animator allocated model absent")?,
                    skeleton: self.skeleton.clone().unwrap(),
                },
                Stage::Register => {
                    let model = state.model.ok_or("base Animator bound model absent")?;
                    *model_callbacks
                        .get_mut(&model)
                        .ok_or("base Animator model callback storage absent")? = Some(self.graphic);
                    self.stage = Stage::Reset;
                    continue;
                }
                Stage::Reset => Request::Reset,
                Stage::Accessories => {
                    state.flags[0] = 1;
                    state.flags[1] = 1;
                    Request::BindAccessories
                }
                Stage::Finish => {
                    state.selection = -1;
                    state.foreground = None;
                    state.flags[1] = 1;
                    self.stage = Stage::Placement;
                    continue;
                }
                Stage::Placement => Request::Placement,
                Stage::Viewport => Request::CameraViewport,
                Stage::Visibility => Request::Visibility {
                    viewport: self.viewport.ok_or("base Animator viewport absent")?,
                },
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending.clone());
        }
    }
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(request) {
            return Err("stale base Animator initializer acknowledgment".into());
        }
        match (self.stage, request, reply) {
            (Stage::Clear, Request::ClearAppearances, Reply::Done) => self.stage = Stage::Profile,
            (Stage::Profile, Request::Profile { .. }, Reply::Profile(profile))
                if profile != Some(0) =>
            {
                state.profile = profile;
                self.stage = Stage::Appearance;
            }
            (Stage::Appearance, Request::ReloadAppearance { .. }, Reply::Done) => {
                self.stage = Stage::Destroy
            }
            (Stage::Destroy, Request::DestroyAccessories, Reply::Done) => {
                self.stage = Stage::ProfileEnd
            }
            (Stage::Skeleton, Request::Skeleton { .. }, Reply::Skeleton(name)) => {
                let name = name.split('\0').next().unwrap();
                self.skeleton = Some(if name.is_empty() {
                    "root".into()
                } else {
                    name.into()
                });
                self.stage = Stage::Construct;
            }
            (Stage::Construct, Request::ConstructModel, Reply::Model(model)) if model != 0 => {
                state.model = Some(model);
                self.stage = Stage::Bind;
            }
            (Stage::Bind, Request::BindModel { .. }, Reply::Bound(status)) => {
                self.bind_status = Some(status);
                self.stage = if status == 0 {
                    Stage::Register
                } else {
                    Stage::Done
                };
            }
            (Stage::Reset, Request::Reset, Reply::Done) => self.stage = Stage::Accessories,
            (Stage::Accessories, Request::BindAccessories, Reply::Done) => {
                self.stage = Stage::Finish
            }
            (Stage::Placement, Request::Placement, Reply::Done) => self.stage = Stage::Viewport,
            (Stage::Viewport, Request::CameraViewport, Reply::Viewport(viewport)) => {
                self.viewport = Some(viewport);
                self.stage = Stage::Visibility;
            }
            (Stage::Visibility, Request::Visibility { .. }, Reply::Done) => {
                self.stage = Stage::Done
            }
            _ => return Err("mismatched base Animator initializer reply".into()),
        }
        self.pending = None;
        Ok(())
    }
}
