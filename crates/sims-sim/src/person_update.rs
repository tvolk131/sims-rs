//! Resumable cXPerson::Update (0x45cc8c). Every external call is a service
//! boundary, including the ordinary animator update (not placement refresh).
//! The host owns VM execution, appearance, room membership and notifications.
use serde::{Deserialize, Serialize};
use sims_vm::{Object, motives};

/// Person storage outside the script-visible arrays: native +0xafc and +0x18c.
/// The label is an original byte string, not assumed to be UTF-8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub room: i16,
    pub displayed_animation: Vec<u8>,
}

/// Live world settings, re-read after services return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Native global 0x6e48c0 == 0.
    pub only_visible: bool,
    pub display_animation_names: bool,
    pub debug_recording: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ObjectUpdate {
        tick: i32,
    },
    /// Three identical f32 scale arguments to 0x5a0932, encoded as exact bits.
    Scale {
        bits: u32,
    },
    /// 0x45d145; the last-decay clock has already been written, even if hidden.
    Decay,
    Animator {
        only_visible: bool,
    },
    RoomMembership,
    Environment {
        room: i16,
    },
    /// Original 0x40745f query. This is not an actor-selection predicate.
    AllowNegativeOutdoorEnvironment,
    HasActionUse,
    Statistics {
        has_action: bool,
    },
    NotifyPerson {
        value: i16,
    },
    AnimationName,
    NotifyAnimationName {
        name: Vec<u8>,
    },
    RecordDebug,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Done,
    ObjectUpdated(u8),
    Environment(u32),
    Bool(bool),
    AnimationName(Vec<u8>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Object,
    Scale,
    Active,
    Decay,
    Animator,
    Membership,
    Environment,
    EnvironmentGuard,
    StoreEnvironment,
    Changes,
    Statistics,
    PersonNotification,
    AnimationName,
    CompareName,
    Debug,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    tick: i32,
    stage: Stage,
    result: Option<u8>,
    pending: Option<Request>,
    environment: Option<u32>,
    animation_name: Option<Vec<u8>>,
}
impl Execution {
    pub fn pending_request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn tick(&self) -> i32 {
        self.tick
    }
    pub fn new(tick: i32) -> Self {
        Self {
            tick,
            stage: Stage::Object,
            result: None,
            pending: None,
            environment: None,
            animation_name: None,
        }
    }
    pub fn result(&self) -> Option<u8> {
        (self.stage == Stage::Complete && self.pending.is_none())
            .then_some(self.result)
            .flatten()
    }
    /// Complete the actual motive service with the shared native-compared
    /// arithmetic. `next` already committed the clock; this must not re-test it.
    pub fn decay(
        &mut self,
        object: &mut Object,
        state: &mut State,
        hour: i16,
        tuning: &motives::Tuning,
    ) -> Result<(), String> {
        if self.pending != Some(Request::Decay) {
            return Err("no pending native periodic decay".into());
        }
        let disabled = field(object, 29)? != 0;
        if !disabled && !hidden(object)? {
            let mut person = [0; 256];
            for key in [3, 6, 29, 58, 65] {
                person[key] = field(object, key as i16)?;
            }
            object
                .motive_update
                .as_mut()
                .ok_or("missing native motive state")?
                .periodic(
                    object
                        .motives
                        .as_mut()
                        .ok_or("missing native person motives")?,
                    &person,
                    false,
                    hour,
                    tuning,
                );
        }
        self.acknowledge(object, state, Response::Done)
    }
    pub fn next(
        &mut self,
        object: &mut Object,
        state: &mut State,
        context: Context,
    ) -> Result<Option<Request>, String> {
        if let Some(request) = &self.pending {
            return Ok(Some(request.clone()));
        }
        loop {
            let request = match self.stage {
                Stage::Object => {
                    self.stage = Stage::Scale;
                    Some(Request::ObjectUpdate { tick: self.tick })
                }
                Stage::Scale => {
                    // Both comparisons use the unspilled x87 product. In
                    // particular field 85 == 1000 passes, then spills to 10f32.
                    let scale = if !hidden(object)? && self.tick % 10 == 0 {
                        let value = f64::from(field(object, 85)?) * f64::from(0.01_f32);
                        (value > 0.4 && value < 10.).then_some((value as f32).to_bits())
                    } else {
                        None
                    };
                    self.stage = Stage::Active;
                    scale.map(|bits| Request::Scale { bits })
                }
                Stage::Active => {
                    self.stage = if self.result.ok_or("missing native object update result")? == 0 {
                        Stage::Complete
                    } else {
                        Stage::Decay
                    };
                    None
                }
                Stage::Decay => {
                    let update = object
                        .motive_update
                        .as_mut()
                        .ok_or("missing native motive clock")?;
                    let due = self.tick >= update.last_decay.wrapping_add(60);
                    if due {
                        update.last_decay = self.tick;
                    }
                    self.stage = Stage::Animator;
                    due.then_some(Request::Decay)
                }
                Stage::Animator => {
                    self.stage = Stage::Membership;
                    Some(Request::Animator {
                        only_visible: context.only_visible,
                    })
                }
                Stage::Membership => {
                    self.stage = Stage::Environment;
                    Some(Request::RoomMembership)
                }
                Stage::Environment => {
                    if state.room == -5 {
                        self.stage = Stage::Changes;
                        None
                    } else {
                        self.stage = Stage::EnvironmentGuard;
                        Some(Request::Environment { room: state.room })
                    }
                }
                Stage::EnvironmentGuard => {
                    self.stage = Stage::StoreEnvironment;
                    // Native re-reads the room after the environment callback.
                    (state.room == 0).then_some(Request::AllowNegativeOutdoorEnvironment)
                }
                Stage::StoreEnvironment => {
                    object
                        .motives
                        .as_mut()
                        .ok_or("missing native person motives")?[13] = self
                        .environment
                        .ok_or("missing native environment result")?;
                    self.stage = Stage::Changes;
                    None
                }
                Stage::Changes => {
                    let counter = field(object, 27)?.wrapping_add(1);
                    let values = object
                        .motives
                        .as_mut()
                        .ok_or("missing native person motives")?;
                    object.person.as_mut().unwrap().insert(27, counter);
                    motives::apply_changes(values, &object.motive_changes);
                    self.stage = Stage::Statistics;
                    None
                }
                Stage::Statistics => {
                    // +0x5cc is person[32]; +0x7b8 is motive[11]. These are
                    // neither posture[0] nor social[14]. Test the post-change values.
                    let record = self.tick % 10 == 0
                        && field(object, 32)? == 0
                        && !hidden(object)?
                        && f32::from_bits(
                            object.motives.ok_or("missing native person motives")?[11],
                        ) >= 0.;
                    self.stage = Stage::PersonNotification;
                    record.then_some(Request::HasActionUse)
                }
                Stage::PersonNotification => {
                    let value = field(object, 1)?;
                    self.stage = Stage::AnimationName;
                    (value != 0).then_some(Request::NotifyPerson { value })
                }
                Stage::AnimationName => {
                    if context.display_animation_names {
                        self.stage = Stage::CompareName;
                        Some(Request::AnimationName)
                    } else {
                        self.stage = Stage::Debug;
                        None
                    }
                }
                Stage::CompareName => {
                    let name = self
                        .animation_name
                        .as_ref()
                        .ok_or("missing animator name")?;
                    self.stage = Stage::Debug;
                    (name != &state.displayed_animation)
                        .then(|| Request::NotifyAnimationName { name: name.clone() })
                }
                Stage::Debug => {
                    self.stage = Stage::Complete;
                    context.debug_recording.then_some(Request::RecordDebug)
                }
                Stage::Complete => return Ok(None),
            };
            if let Some(request) = request {
                self.pending = Some(request.clone());
                return Ok(Some(request));
            }
        }
    }
    /// Acknowledgment follows the service, so all live state may have changed.
    /// An incorrect response leaves the request pending and is safe to retry.
    pub fn acknowledge(
        &mut self,
        object: &mut Object,
        state: &mut State,
        response: Response,
    ) -> Result<(), String> {
        match (
            self.pending
                .as_ref()
                .ok_or("no pending person update service")?,
            response,
        ) {
            (Request::ObjectUpdate { .. }, Response::ObjectUpdated(value)) => {
                self.result = Some(value)
            }
            (Request::Environment { .. }, Response::Environment(bits)) => {
                self.environment = Some(bits)
            }
            (Request::AllowNegativeOutdoorEnvironment, Response::Bool(allow)) => {
                let bits = self.environment.ok_or("missing environment value")?;
                if !allow && f32::from_bits(bits) < 0. {
                    self.environment = Some(0f32.to_bits());
                }
            }
            (Request::HasActionUse, Response::Bool(has_action)) => {
                self.pending = Some(Request::Statistics { has_action });
                return Ok(());
            }
            (Request::NotifyPerson { .. }, Response::Done) => {
                object
                    .person
                    .as_mut()
                    .ok_or("missing native person fields")?
                    .insert(1, 0);
            }
            (Request::AnimationName, Response::AnimationName(name)) => {
                if name.len() >= 1024 || name.contains(&0) {
                    return Err("invalid native animator name".into());
                }
                self.animation_name = Some(name);
            }
            (Request::NotifyAnimationName { name }, Response::Done) => {
                state.displayed_animation = name.clone();
            }
            (
                Request::Scale { .. }
                | Request::Decay
                | Request::Animator { .. }
                | Request::RoomMembership
                | Request::Statistics { .. }
                | Request::RecordDebug,
                Response::Done,
            ) => (),
            _ => return Err("incorrect native person update response".into()),
        }
        self.pending = None;
        Ok(())
    }
}
fn field(object: &Object, key: i16) -> Result<i16, String> {
    object
        .person
        .as_ref()
        .and_then(|p| p.get(&key))
        .copied()
        .ok_or_else(|| format!("missing native person update field {key}"))
}
fn hidden(object: &Object) -> Result<bool, String> {
    object
        .data
        .get(&34)
        .map(|v| *v != 0)
        .ok_or("missing native person visibility".into())
}
