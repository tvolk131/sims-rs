//! Original44752e Placed caller. This is distinct from Selection/Star/Date.
//! Initialization, physical changes, graphics and pool services remain owned
//! synchronous children. Their partial writes precede acknowledgment and are
//! never rolled back by this continuation. No engine/GPU work is synthesized.
use crate::ObjectId;
pub use crate::selection_indicators::{Graphic, GraphicCall, Slot};
use serde::{Deserialize, Serialize};

pub trait View {
    /// Raw registry+11: any nonzero byte enables the caller.
    fn enabled_byte(&self) -> Result<u8, String>;
    fn primary(&self) -> Result<Option<ObjectId>, String>;
    fn secondary(&self) -> Result<Option<ObjectId>, String>;
    fn flags(&self, object: ObjectId) -> Result<u32, String>;
    /// Original RTTI boundary, not OBJD kind or script data.
    fn is_person(&self, object: ObjectId) -> Result<bool, String>;
    fn person_data(&self, person: ObjectId, index: i16) -> Result<i16, String>;
    /// [6e48bc]+66, distinct from the writable copy in VM global20. This read
    /// remains required even for a nonperson source. Unknown input is an error.
    fn installed_expansions(&self) -> Result<u16, String>;
    fn lookup(&self, id: i16) -> Result<Option<ObjectId>, String>;
    fn outside(&self, object: ObjectId) -> Result<bool, String>;
    /// Script slot1 = native record2. None means absent storage, not flags0.
    fn slot(&self, object: ObjectId) -> Result<Option<Slot>, String>;
    fn position(&self, object: ObjectId) -> Result<[i32; 3], String>;
    fn graphic(&self, object: ObjectId) -> Result<Graphic, String>;
    /// Person virtual+8c animator, then animator+b0 transform identity.
    fn transform(&self, person: ObjectId) -> Result<Graphic, String>;
    /// None = empty vector; Some(None) = actual null last member.
    fn spare(&self) -> Result<Option<Option<ObjectId>>, String>;
    fn pop_spare(&mut self) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// Run the retained445e4a child to completion before acknowledging. Cache
    /// changes during its factories are visible at the following Flags stage.
    Initialize,
    Parent(ObjectId),
    Detach(ObjectId),
    Release(ObjectId),
    Attach {
        indicator: ObjectId,
        person: ObjectId,
        position: [i32; 3],
    },
    Graphic {
        graphic: Graphic,
        call: GraphicCall,
    },
    Refill,
    RecordStar(ObjectId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Parent(Option<ObjectId>),
    Done,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Primary,
    Secondary,
    Star,
}
/// Explicit tagging preserves an observed null vector member through JSON.
/// Option<Option<ObjectId>> would collapse Some(None) and None to the same null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Member {
    Null,
    Object(ObjectId),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Enabled,
    Cache,
    Initialize,
    Flags,
    SourceRtti,
    Primary,
    PrimaryParent,
    PrimaryDetach,
    BeginPrimary,
    Secondary,
    SecondaryDetach,
    SecondaryLookup,
    SecondaryRtti,
    StarGuard,
    StarFame,
    StarPool,
    Refill,
    StarCapture,
    StarPop,
    BeginStar,
    Outside,
    Slot,
    Occupant,
    Release,
    Attach,
    Graphic,
    Bind,
    Control,
    Reload,
    PersonRefresh,
    Update,
    Record,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    object: Option<ObjectId>,
    stage: Stage,
    enabled: Option<u8>,
    selected: Option<bool>,
    person: Option<bool>,
    secondary_target: Option<i16>,
    installed_mask: Option<u16>,
    fame: Option<i16>,
    kind: Option<Kind>,
    recipient: Option<ObjectId>,
    recipient_person: Option<bool>,
    star: Option<Member>,
    slot_occupant: Option<i16>,
    occupant: Option<ObjectId>,
    graphic: Option<Graphic>,
    pending: Option<Request>,
}
impl Placed {
    pub fn new(object: Option<ObjectId>) -> Self {
        Self {
            object,
            stage: Stage::Start,
            enabled: None,
            selected: None,
            person: None,
            secondary_target: None,
            installed_mask: None,
            fame: None,
            kind: None,
            recipient: None,
            recipient_person: None,
            star: None,
            slot_occupant: None,
            occupant: None,
            graphic: None,
            pending: None,
        }
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn validate(&self) -> Result<(), String> {
        let early = matches!(
            self.stage,
            Stage::Start | Stage::Enabled | Stage::Cache | Stage::Initialize | Stage::Flags
        );
        let placement = matches!(
            self.stage,
            Stage::Outside
                | Stage::Slot
                | Stage::Occupant
                | Stage::Release
                | Stage::Attach
                | Stage::Graphic
                | Stage::Bind
                | Stage::Control
                | Stage::Reload
                | Stage::PersonRefresh
                | Stage::Update
                | Stage::Record
        );
        let star = matches!(
            self.stage,
            Stage::StarFame
                | Stage::StarPool
                | Stage::Refill
                | Stage::StarCapture
                | Stage::StarPop
                | Stage::BeginStar
                | Stage::Record
        );
        let secondary = matches!(
            self.stage,
            Stage::SecondaryDetach | Stage::SecondaryLookup | Stage::SecondaryRtti
        );
        let invalid = self.object.is_some_and(|id| id <= 0)
            || self.object.is_none() && !matches!(self.stage, Stage::Start | Stage::Done)
            || [self.recipient, self.occupant, self.star_id()]
                .into_iter()
                .flatten()
                .any(|id| id <= 0)
            || self.graphic == Some(Graphic(0))
            || matches!(self.stage, Stage::Start | Stage::Enabled) && self.enabled.is_some()
            || !matches!(self.stage, Stage::Start | Stage::Enabled | Stage::Done)
                && self.enabled.is_none_or(|value| value == 0)
            || early && (self.selected.is_some() || self.person.is_some())
            || self.stage == Stage::SourceRtti
                && (self.selected.is_none() || self.person.is_some())
            || !early
                && !matches!(self.stage, Stage::SourceRtti | Stage::Done)
                && (self.selected.is_none() || self.person.is_none())
            || self.stage == Stage::Done && (self.selected.is_some() != self.person.is_some())
            || placement
                && (self.kind.is_none()
                    || self.recipient.is_none()
                    || self.recipient_person.is_none())
            || matches!(
                self.stage,
                Stage::Bind
                    | Stage::Control
                    | Stage::Reload
                    | Stage::PersonRefresh
                    | Stage::Update
                    | Stage::Record
            ) && self.graphic.is_none()
            || matches!(
                self.stage,
                Stage::PrimaryParent | Stage::PrimaryDetach | Stage::BeginPrimary
            ) && (self.person != Some(true) || self.selected != Some(true))
            || secondary
                && (self.person != Some(true) || self.secondary_target.is_none_or(|id| id <= 0))
            || self.stage == Stage::SecondaryDetach && self.selected != Some(true)
            || self.stage == Stage::SecondaryRtti
                && (self.kind != Some(Kind::Secondary)
                    || self.recipient.is_none()
                    || self.recipient_person.is_some())
            || star
                && (self.person != Some(true)
                    || self.selected != Some(false)
                    || self.installed_mask.is_none_or(|m| m & 0x40 == 0))
            || star && self.stage != Stage::StarFame && self.fame.is_none_or(|value| value < 10)
            || self.stage == Stage::StarPop && self.star.is_none()
            || matches!(self.stage, Stage::BeginStar | Stage::Record) && self.star_id().is_none()
            || self.stage == Stage::Occupant && self.slot_occupant.is_none()
            || self.stage == Stage::Release && self.occupant.is_none()
            || self.stage == Stage::Record && self.kind != Some(Kind::Star)
            || self
                .pending
                .is_some_and(|request| !self.request_valid(request));
        if invalid {
            return Err("invalid native Placed indicator continuation".into());
        }
        Ok(())
    }
    fn request_valid(&self, request: Request) -> bool {
        match (self.stage, request) {
            (Stage::Initialize, Request::Initialize) | (Stage::Refill, Request::Refill) => true,
            (Stage::PrimaryParent, Request::Parent(id))
            | (Stage::PrimaryDetach | Stage::SecondaryDetach, Request::Detach(id)) => id > 0,
            (Stage::Release, Request::Release(id)) => Some(id) == self.occupant,
            (
                Stage::Attach,
                Request::Attach {
                    indicator, person, ..
                },
            ) => indicator > 0 && Some(person) == self.recipient,
            (Stage::Record, Request::RecordStar(id)) => Some(id) == self.star_id(),
            (
                Stage::Bind,
                Request::Graphic {
                    graphic,
                    call: GraphicCall::BindTransform(source),
                },
            ) => Some(graphic) == self.graphic && source.0 != 0,
            (
                Stage::Control,
                Request::Graphic {
                    graphic,
                    call: GraphicCall::TransformControl,
                },
            )
            | (
                Stage::Reload,
                Request::Graphic {
                    graphic,
                    call: GraphicCall::ReloadModel,
                },
            )
            | (
                Stage::Update,
                Request::Graphic {
                    graphic,
                    call: GraphicCall::Update,
                },
            ) => Some(graphic) == self.graphic,
            (
                Stage::PersonRefresh,
                Request::Graphic {
                    graphic,
                    call: GraphicCall::Refresh,
                },
            ) => graphic.0 != 0,
            _ => false,
        }
    }
    fn primary(view: &impl View) -> Result<ObjectId, String> {
        view.primary()?
            .ok_or_else(|| "native Placed primary cache absent".into())
    }
    fn star_id(&self) -> Option<ObjectId> {
        match self.star {
            Some(Member::Object(id)) => Some(id),
            Some(Member::Null) | None => None,
        }
    }
    fn secondary(view: &impl View) -> Result<ObjectId, String> {
        view.secondary()?
            .ok_or_else(|| "native Placed secondary cache absent".into())
    }
    fn indicator(&self, view: &impl View) -> Result<ObjectId, String> {
        match self.kind.ok_or("native Placed kind absent")? {
            Kind::Primary => Self::primary(view),
            Kind::Secondary => Self::secondary(view),
            Kind::Star => self
                .star_id()
                .ok_or_else(|| "native Placed star absent".into()),
        }
    }
    fn begin_place(&mut self, kind: Kind, recipient: ObjectId, person: bool) {
        self.kind = Some(kind);
        self.recipient = Some(recipient);
        self.recipient_person = Some(person);
        self.slot_occupant = None;
        self.occupant = None;
        self.graphic = None;
        self.stage = Stage::Outside;
    }
    fn after_place(&self) -> Stage {
        match self.kind.expect("validated Placed kind") {
            Kind::Primary => Stage::Secondary,
            Kind::Secondary => Stage::StarGuard,
            Kind::Star => Stage::Done,
        }
    }
    pub fn advance(&mut self, view: &mut impl View) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            let object = self.object.unwrap_or(0);
            let request = match self.stage {
                Stage::Start => {
                    self.stage = if self.object.is_none() {
                        Stage::Done
                    } else {
                        Stage::Enabled
                    };
                    continue;
                }
                Stage::Enabled => {
                    let enabled = view.enabled_byte()?;
                    self.enabled = Some(enabled);
                    self.stage = if enabled == 0 {
                        Stage::Done
                    } else {
                        Stage::Cache
                    };
                    continue;
                }
                Stage::Cache => {
                    self.stage = if view.primary()?.is_none() {
                        Stage::Initialize
                    } else {
                        Stage::Flags
                    };
                    continue;
                }
                Stage::Initialize => Request::Initialize,
                Stage::Flags => {
                    self.selected = Some(view.flags(object)? & 2 != 0);
                    self.stage = Stage::SourceRtti;
                    continue;
                }
                Stage::SourceRtti => {
                    self.person = Some(view.is_person(object)?);
                    self.stage = Stage::Primary;
                    continue;
                }
                Stage::Primary => {
                    self.stage = if self.person == Some(true)
                        && self.selected == Some(true)
                        && view.primary()?.is_some()
                    {
                        Stage::PrimaryParent
                    } else {
                        Stage::Secondary
                    };
                    continue;
                }
                Stage::PrimaryParent => Request::Parent(Self::primary(view)?),
                Stage::PrimaryDetach => Request::Detach(Self::primary(view)?),
                Stage::BeginPrimary => {
                    self.begin_place(Kind::Primary, object, true);
                    continue;
                }
                Stage::Secondary => {
                    if self.person != Some(true) || view.secondary()?.is_none() {
                        self.stage = Stage::StarGuard;
                        continue;
                    }
                    let target = view.person_data(object, 77)?;
                    self.secondary_target = Some(target);
                    self.stage = if target <= 0 {
                        Stage::StarGuard
                    } else if self.selected == Some(true) {
                        Stage::SecondaryDetach
                    } else {
                        Stage::SecondaryLookup
                    };
                    continue;
                }
                Stage::SecondaryDetach => Request::Detach(Self::secondary(view)?),
                Stage::SecondaryLookup => {
                    if let Some(recipient) = view.lookup(self.secondary_target.unwrap())? {
                        self.kind = Some(Kind::Secondary);
                        self.recipient = Some(recipient);
                        self.recipient_person = None;
                        self.stage = Stage::SecondaryRtti;
                    } else {
                        self.stage = Stage::StarGuard;
                    }
                    continue;
                }
                Stage::SecondaryRtti => {
                    // A failed cast does not skip physical placement. Native
                    // faults only after attach and the indicator graphic getter.
                    let person = view.is_person(self.recipient.unwrap())?;
                    self.begin_place(Kind::Secondary, self.recipient.unwrap(), person);
                    continue;
                }
                Stage::StarGuard => {
                    let mask = view.installed_expansions()?;
                    self.installed_mask = Some(mask);
                    self.stage = if mask & 0x40 == 0
                        || self.selected == Some(true)
                        || self.person != Some(true)
                    {
                        Stage::Done
                    } else {
                        Stage::StarFame
                    };
                    continue;
                }
                Stage::StarFame => {
                    let fame = view.person_data(object, 81)?;
                    self.fame = Some(fame);
                    self.stage = if fame < 10 {
                        Stage::Done
                    } else {
                        Stage::StarPool
                    };
                    continue;
                }
                Stage::StarPool => {
                    self.stage = if view.spare()?.is_none() {
                        Stage::Refill
                    } else {
                        Stage::StarCapture
                    };
                    continue;
                }
                Stage::Refill => Request::Refill,
                Stage::StarCapture => {
                    self.star = view
                        .spare()?
                        .map(|member| member.map_or(Member::Null, Member::Object));
                    self.stage = if self.star.is_some() {
                        Stage::StarPop
                    } else {
                        Stage::Done
                    };
                    continue;
                }
                Stage::StarPop => {
                    //447b40..447b63 changes end BEFORE null/outside/slot checks.
                    view.pop_spare()?;
                    self.stage = if self.star_id().is_some() {
                        Stage::BeginStar
                    } else {
                        Stage::Done
                    };
                    continue;
                }
                Stage::BeginStar => {
                    self.begin_place(Kind::Star, object, true);
                    continue;
                }
                Stage::Outside => {
                    self.stage = if view.outside(self.recipient.unwrap())? {
                        self.after_place()
                    } else {
                        Stage::Slot
                    };
                    continue;
                }
                Stage::Slot => {
                    let slot = view
                        .slot(self.recipient.unwrap())?
                        .ok_or("native Placed indicator slot absent")?;
                    self.slot_occupant = Some(slot.occupant);
                    self.stage = if slot.flags & 2 == 0 {
                        self.after_place()
                    } else {
                        Stage::Occupant
                    };
                    continue;
                }
                Stage::Occupant => {
                    let occupant = self.slot_occupant.unwrap();
                    self.occupant = if occupant == 0 {
                        None
                    } else {
                        view.lookup(occupant)?
                    };
                    self.stage = if self.occupant.is_some() {
                        Stage::Release
                    } else {
                        Stage::Attach
                    };
                    continue;
                }
                Stage::Release => Request::Release(self.occupant.unwrap()),
                Stage::Attach => {
                    let position = view.position(self.recipient.unwrap())?;
                    Request::Attach {
                        indicator: self.indicator(view)?,
                        person: self.recipient.unwrap(),
                        position,
                    }
                }
                Stage::Graphic => {
                    self.graphic = Some(view.graphic(self.indicator(view)?)?);
                    self.stage = Stage::Bind;
                    continue;
                }
                Stage::Bind => {
                    if self.recipient_person != Some(true) {
                        return Err("native Placed recipient RTTI absent after attachment".into());
                    }
                    Request::Graphic {
                        graphic: self.graphic.unwrap(),
                        call: GraphicCall::BindTransform(view.transform(self.recipient.unwrap())?),
                    }
                }
                Stage::Control => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::TransformControl,
                },
                Stage::Reload => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::ReloadModel,
                },
                Stage::PersonRefresh => Request::Graphic {
                    graphic: view.graphic(self.recipient.unwrap())?,
                    call: GraphicCall::Refresh,
                },
                Stage::Update => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::Update,
                },
                Stage::Record => Request::RecordStar(self.star_id().unwrap()),
                Stage::Done => return Ok(None),
            };
            self.pending = Some(request);
            self.validate()?;
            return Ok(self.pending);
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request)
            || matches!(request, Request::Parent(_)) != matches!(reply, Reply::Parent(_))
            || reply == Reply::Parent(Some(0))
            || matches!(reply, Reply::Parent(Some(id)) if id < 0)
        {
            return Err("stale or mismatched native Placed acknowledgment".into());
        }
        self.stage = match self.stage {
            Stage::Initialize => Stage::Flags,
            Stage::PrimaryParent => {
                if reply == Reply::Parent(self.object) {
                    Stage::BeginPrimary
                } else {
                    Stage::PrimaryDetach
                }
            }
            Stage::PrimaryDetach => Stage::BeginPrimary,
            Stage::SecondaryDetach => Stage::SecondaryLookup,
            Stage::Refill => Stage::StarCapture,
            Stage::Release => Stage::Attach,
            Stage::Attach => Stage::Graphic,
            Stage::Bind => Stage::Control,
            Stage::Control => Stage::Reload,
            Stage::Reload => Stage::PersonRefresh,
            Stage::PersonRefresh => Stage::Update,
            Stage::Update => {
                if self.kind == Some(Kind::Star) {
                    Stage::Record
                } else {
                    self.after_place()
                }
            }
            Stage::Record => Stage::Done,
            _ => return Err("native Placed stage has no pending child".into()),
        };
        self.pending = None;
        self.validate()?;
        Ok(())
    }
}
