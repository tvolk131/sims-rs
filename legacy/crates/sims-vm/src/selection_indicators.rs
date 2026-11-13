//! Selection/star/date indicator orchestration, 44664c, 446ee9 and 4472a2.
//!
//! Physical and graphics calls are synchronous services: the host must finish
//! each request before acknowledging it. Registry fields are reread where the
//! original does so; graphics handles retain the identity returned by the
//! original getters. This controller does not implement those host services.
use crate::ObjectId;
use serde::{Deserialize, Serialize};

pub const STAR_GUID: u32 = 0xe9337020;
pub const SECONDARY_GUID: u32 = 0x0baa71c5;

/// Stable host identity for the graphic/animator, not a native memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graphic(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub flags: u32,
    pub occupant: i16,
}

pub trait View {
    fn enabled(&self) -> bool;
    fn primary(&self) -> Option<ObjectId>;
    fn secondary(&self) -> Option<ObjectId>;
    fn selected(&self) -> Result<Option<ObjectId>, String>;
    fn flags(&self, object: ObjectId) -> Result<u32, String>;
    fn is_person(&self, object: ObjectId) -> Result<bool, String>;
    fn person_data(&self, person: ObjectId, index: i16) -> Result<i16, String>;
    fn expansions(&self) -> u16;
    fn lookup(&self, script_id: i16) -> Option<ObjectId>;
    fn guid(&self, object: ObjectId) -> Result<u32, String>;
    fn outside(&self, object: ObjectId) -> Result<bool, String>;
    /// Script slot 1, native vector record 2 (record zero is the sentinel).
    fn slot(&self, object: ObjectId) -> Result<Option<Slot>, String>;
    /// Native X, Y, level, read after any release callback.
    fn position(&self, object: ObjectId) -> Result<[i32; 3], String>;
    /// Pure 4215c3/4594de getter; callers preserve its returned identity.
    fn graphic(&self, object: ObjectId) -> Result<Graphic, String>;
    /// None is empty; Some(None) is an actual null member in the native vector.
    fn spare(&self) -> Option<Option<ObjectId>>;
    fn pop_spare(&mut self) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphicCall {
    /// +b8: consume the transform owned by this graphic (+b0/58486d).
    BindTransform(Graphic),
    /// +64 / 5867bf, set the explicit-transform control byte to one.
    TransformControl,
    /// +d4, force the model/appearance refresh.
    ReloadModel,
    /// +54(0), including the person's animator refresh after attachment.
    Refresh,
    /// +50(0), after all preceding attachment/refresh services.
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// 43265e physical capture. This can normalize a native slot sentinel.
    Parent(ObjectId),
    /// Virtual RemoveFromLot(1).
    Detach(ObjectId),
    /// Registry 4481eb, including its pool and physical removal work.
    Release(ObjectId),
    /// Virtual +58(position, level, parent, slot=1, flags=1).
    Attach {
        indicator: ObjectId,
        person: ObjectId,
        position: [i32; 3],
    },
    Graphic {
        graphic: Graphic,
        call: GraphicCall,
    },
    /// 446356 must finish the original factories before acknowledgment.
    Refill,
    /// Append to the active star vector. The selection entry pops its spare
    /// only after this service; the star entry popped before placement.
    RecordStar(ObjectId),
}

impl Request {
    fn valid(self) -> bool {
        match self {
            Self::Parent(id) | Self::Detach(id) | Self::Release(id) | Self::RecordStar(id) => {
                id != 0
            }
            Self::Attach {
                indicator, person, ..
            } => indicator != 0 && person != 0,
            Self::Graphic { graphic, call } => {
                graphic != Graphic(0) && !matches!(call, GraphicCall::BindTransform(Graphic(0)))
            }
            Self::Refill => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Parent(Option<ObjectId>),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Selection,
    Star,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Kind {
    Primary,
    Secondary,
    Star,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    PrimaryParent,
    PrimaryDetach,
    PrimaryPlace,
    AfterPrimary,
    PrimaryClear,
    PrimaryRefresh,
    Secondary,
    SecondaryDetach,
    SecondaryPlace,
    AfterSecondary,
    SecondaryClear,
    Star,
    Refill,
    StarTake,
    StarPlace,
    Release,
    Attach,
    Bind,
    TransformControl,
    ReloadModel,
    SecondaryRefresh,
    PersonRefresh,
    Update,
    Record,
    Pop,
    Demote,
    DemoteDetach,
    DemoteRelease,
    DateDetach,
    DateTarget,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    object: Option<ObjectId>,
    entry: Entry,
    stage: Stage,
    selected_on_entry: bool,
    person_on_entry: bool,
    secondary_person: Option<ObjectId>,
    star: Option<ObjectId>,
    kind: Option<Kind>,
    recipient: Option<ObjectId>,
    occupant: Option<ObjectId>,
    graphic: Option<Graphic>,
    pending: Option<Request>,
}

impl Update {
    pub fn object(&self) -> Option<ObjectId> {
        self.object
    }
    pub fn entry(&self) -> Entry {
        self.entry
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn new(object: Option<ObjectId>, entry: Entry) -> Self {
        Self {
            object,
            entry,
            stage: Stage::Start,
            selected_on_entry: false,
            person_on_entry: false,
            secondary_person: None,
            star: None,
            kind: None,
            recipient: None,
            occupant: None,
            graphic: None,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Done
    }

    pub fn validate(&self) -> Result<(), String> {
        if [
            self.object,
            self.secondary_person,
            self.star,
            self.recipient,
            self.occupant,
        ]
        .contains(&Some(0))
            || self.graphic == Some(Graphic(0))
            || matches!(self.stage, Stage::DateDetach | Stage::DateTarget)
                && self.entry != Entry::Date
            || self.entry == Entry::Date
                && (self.kind.is_some_and(|kind| kind != Kind::Secondary)
                    || !matches!(
                        self.stage,
                        Stage::Start
                            | Stage::DateDetach
                            | Stage::DateTarget
                            | Stage::Release
                            | Stage::Attach
                            | Stage::Bind
                            | Stage::TransformControl
                            | Stage::ReloadModel
                            | Stage::SecondaryRefresh
                            | Stage::PersonRefresh
                            | Stage::Update
                            | Stage::Done
                    ))
            || self.stage != Stage::Start && self.stage != Stage::Done && self.object.is_none()
            || matches!(
                self.stage,
                Stage::Release
                    | Stage::Attach
                    | Stage::Bind
                    | Stage::TransformControl
                    | Stage::ReloadModel
                    | Stage::SecondaryRefresh
                    | Stage::PersonRefresh
                    | Stage::Update
            ) && (self.kind.is_none() || self.recipient.is_none())
            || matches!(
                self.stage,
                Stage::TransformControl
                    | Stage::ReloadModel
                    | Stage::SecondaryRefresh
                    | Stage::PersonRefresh
                    | Stage::Update
            ) && self.graphic.is_none()
            || matches!(self.stage, Stage::Record | Stage::Pop) && self.star.is_none()
            || matches!(
                self.stage,
                Stage::Release | Stage::DemoteDetach | Stage::DemoteRelease
            ) && self.occupant.is_none()
            || self.pending.is_some_and(|r| {
                !r.valid()
                    || !matches!(
                        (self.stage, r),
                        (Stage::PrimaryParent, Request::Parent(_))
                            | (
                                Stage::PrimaryDetach
                                    | Stage::PrimaryClear
                                    | Stage::SecondaryDetach
                                    | Stage::SecondaryClear
                                    | Stage::DemoteDetach
                                    | Stage::DateDetach,
                                Request::Detach(_)
                            )
                            | (Stage::Release | Stage::DemoteRelease, Request::Release(_))
                            | (Stage::Attach, Request::Attach { .. })
                            | (
                                Stage::Bind,
                                Request::Graphic {
                                    call: GraphicCall::BindTransform(_),
                                    ..
                                }
                            )
                            | (
                                Stage::TransformControl,
                                Request::Graphic {
                                    call: GraphicCall::TransformControl,
                                    ..
                                }
                            )
                            | (
                                Stage::ReloadModel,
                                Request::Graphic {
                                    call: GraphicCall::ReloadModel,
                                    ..
                                }
                            )
                            | (
                                Stage::SecondaryRefresh
                                    | Stage::PersonRefresh
                                    | Stage::PrimaryRefresh,
                                Request::Graphic {
                                    call: GraphicCall::Refresh,
                                    ..
                                }
                            )
                            | (
                                Stage::Update,
                                Request::Graphic {
                                    call: GraphicCall::Update,
                                    ..
                                }
                            )
                            | (Stage::Refill, Request::Refill)
                            | (Stage::Record, Request::RecordStar(_))
                    )
            })
        {
            return Err("invalid native indicator continuation".into());
        }
        Ok(())
    }

    fn primary(view: &impl View) -> Result<ObjectId, String> {
        view.primary()
            .ok_or_else(|| "native primary indicator disappeared".into())
    }
    fn secondary(view: &impl View) -> Result<ObjectId, String> {
        view.secondary()
            .ok_or_else(|| "native secondary indicator disappeared".into())
    }
    fn indicator(&self, view: &impl View) -> Result<ObjectId, String> {
        match self.kind.ok_or("indicator kind absent")? {
            Kind::Primary => Self::primary(view),
            Kind::Secondary => Self::secondary(view),
            Kind::Star => self.star.ok_or_else(|| "captured star absent".into()),
        }
    }
    fn place(
        &mut self,
        view: &impl View,
        person: ObjectId,
        kind: Kind,
        after: Stage,
    ) -> Result<(), String> {
        if view.outside(person)? {
            self.stage = after;
            return Ok(());
        }
        let slot = view.slot(person)?.ok_or("native indicator slot absent")?;
        if slot.flags & 2 == 0 {
            self.stage = after;
            return Ok(());
        }
        self.kind = Some(kind);
        self.recipient = Some(person);
        self.occupant = if slot.occupant == 0 {
            None
        } else {
            view.lookup(slot.occupant)
        };
        self.stage = if self.occupant.is_some() {
            Stage::Release
        } else {
            Stage::Attach
        };
        Ok(())
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
                    if self.object.is_none()
                        || matches!(self.entry, Entry::Selection | Entry::Date)
                            && (!view.enabled() || view.primary().is_none())
                    {
                        self.stage = Stage::Done;
                        continue;
                    }
                    self.selected_on_entry = view.flags(object)? & 2 != 0;
                    if self.entry == Entry::Star {
                        self.person_on_entry = true; // This entry is called for a cXPerson.
                        self.stage = Stage::Star;
                    } else if self.entry == Entry::Date {
                        self.stage = if view.secondary().is_some() {
                            Stage::DateDetach
                        } else {
                            Stage::Done
                        };
                    } else {
                        self.person_on_entry = view.is_person(object)?;
                        self.stage = if self.person_on_entry && self.selected_on_entry {
                            Stage::PrimaryParent
                        } else if self.person_on_entry {
                            Stage::AfterPrimary
                        } else {
                            Stage::Secondary
                        };
                    }
                    continue;
                }
                Stage::PrimaryParent => Request::Parent(Self::primary(view)?),
                Stage::DateDetach => Request::Detach(Self::secondary(view)?),
                Stage::DateTarget => {
                    // 4472a2 captures selection before detach, but reads the
                    // target afterwards, including callback changes.
                    let target = view.person_data(object, 77)?;
                    if self.selected_on_entry && target < 0 {
                        if let Some(person) = view.lookup(target.wrapping_neg()) {
                            self.place(view, person, Kind::Secondary, Stage::Done)?;
                            continue;
                        }
                    }
                    self.stage = Stage::Done;
                    continue;
                }
                Stage::PrimaryDetach => Request::Detach(Self::primary(view)?),
                Stage::PrimaryPlace => {
                    self.place(view, object, Kind::Primary, Stage::AfterPrimary)?;
                    continue;
                }
                Stage::AfterPrimary => {
                    self.stage = if view.selected()?.is_none() && view.primary().is_some() {
                        Stage::PrimaryClear
                    } else {
                        Stage::Secondary
                    };
                    continue;
                }
                Stage::PrimaryClear => Request::Detach(Self::primary(view)?),
                Stage::PrimaryRefresh => Request::Graphic {
                    graphic: view.graphic(object)?,
                    call: GraphicCall::Refresh,
                },
                Stage::Secondary => {
                    self.stage = Stage::AfterSecondary;
                    if self.selected_on_entry && self.person_on_entry && view.secondary().is_some()
                    {
                        let target = view.person_data(object, 77)?;
                        // Positive targets deliberately do not enter attachment.
                        if target < 0 {
                            self.secondary_person = view.lookup(target.wrapping_neg());
                            self.stage = Stage::SecondaryDetach;
                        }
                    }
                    continue;
                }
                Stage::SecondaryDetach => Request::Detach(Self::secondary(view)?),
                Stage::SecondaryPlace => {
                    if let Some(person) = self.secondary_person {
                        self.place(view, person, Kind::Secondary, Stage::AfterSecondary)?;
                    } else {
                        self.stage = Stage::AfterSecondary;
                    }
                    continue;
                }
                Stage::AfterSecondary => {
                    self.stage = if view.selected()?.is_none() && view.secondary().is_some() {
                        Stage::SecondaryClear
                    } else {
                        Stage::Star
                    };
                    continue;
                }
                Stage::SecondaryClear => Request::Detach(Self::secondary(view)?),
                Stage::Star => {
                    self.stage = if view.expansions() & 0x40 == 0
                        || self.selected_on_entry
                        || !self.person_on_entry
                    {
                        Stage::Done
                    } else if view.person_data(object, 81)? < 10 {
                        if self.entry == Entry::Star {
                            Stage::Demote
                        } else {
                            Stage::Done
                        }
                    } else if view.spare().is_none() {
                        Stage::Refill
                    } else {
                        Stage::StarTake
                    };
                    continue;
                }
                Stage::Refill => Request::Refill,
                Stage::StarTake => {
                    let Some(star) = view.spare() else {
                        self.stage = Stage::Done;
                        continue;
                    };
                    self.star = star;
                    if self.entry == Entry::Star {
                        view.pop_spare()?;
                    }
                    self.stage = if star.is_some() {
                        Stage::StarPlace
                    } else {
                        Stage::Done
                    };
                    continue;
                }
                Stage::StarPlace => {
                    self.place(view, object, Kind::Star, Stage::Done)?;
                    continue;
                }
                Stage::Release => Request::Release(self.occupant.unwrap()),
                Stage::Attach => Request::Attach {
                    indicator: self.indicator(view)?,
                    person: self.recipient.unwrap(),
                    position: view.position(self.recipient.unwrap())?,
                },
                Stage::Bind => {
                    let person = self.recipient.unwrap();
                    if !view.is_person(person)? {
                        return Err("indicator recipient is not a native person".into());
                    }
                    let graphic = view.graphic(self.indicator(view)?)?;
                    self.graphic = Some(graphic);
                    Request::Graphic {
                        graphic,
                        call: GraphicCall::BindTransform(view.graphic(person)?),
                    }
                }
                Stage::TransformControl => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::TransformControl,
                },
                Stage::ReloadModel => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::ReloadModel,
                },
                Stage::SecondaryRefresh => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::Refresh,
                },
                Stage::PersonRefresh => Request::Graphic {
                    graphic: view.graphic(self.recipient.unwrap())?,
                    call: GraphicCall::Refresh,
                },
                Stage::Update => Request::Graphic {
                    graphic: self.graphic.unwrap(),
                    call: GraphicCall::Update,
                },
                Stage::Record => Request::RecordStar(self.star.unwrap()),
                Stage::Pop => {
                    view.pop_spare()?;
                    self.stage = Stage::Done;
                    continue;
                }
                Stage::Demote => {
                    self.stage = Stage::Done;
                    if let Some(slot) = view.slot(object)? {
                        if slot.flags & 2 != 0 && slot.occupant != 0 {
                            if let Some(occupant) = view.lookup(slot.occupant) {
                                if view.guid(occupant)? == SECONDARY_GUID {
                                    self.occupant = Some(occupant);
                                    self.stage = Stage::DemoteDetach;
                                }
                            }
                        }
                    }
                    continue;
                }
                Stage::DemoteDetach => Request::Detach(self.occupant.unwrap()),
                Stage::DemoteRelease => Request::Release(self.occupant.unwrap()),
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
        {
            return Err("stale or mismatched native indicator acknowledgment".into());
        }
        self.stage = match self.stage {
            Stage::PrimaryParent => {
                if reply == Reply::Parent(self.object) {
                    Stage::PrimaryPlace
                } else {
                    Stage::PrimaryDetach
                }
            }
            Stage::PrimaryDetach => Stage::PrimaryPlace,
            Stage::DateDetach => Stage::DateTarget,
            Stage::PrimaryClear => Stage::PrimaryRefresh,
            Stage::PrimaryRefresh => Stage::Secondary,
            Stage::SecondaryDetach => Stage::SecondaryPlace,
            Stage::SecondaryClear => Stage::Star,
            Stage::Refill => Stage::StarTake,
            Stage::Release => Stage::Attach,
            Stage::Attach => Stage::Bind,
            Stage::Bind => Stage::TransformControl,
            Stage::TransformControl => Stage::ReloadModel,
            Stage::ReloadModel => {
                if self.kind == Some(Kind::Secondary) {
                    Stage::SecondaryRefresh
                } else {
                    Stage::PersonRefresh
                }
            }
            Stage::SecondaryRefresh => Stage::PersonRefresh,
            Stage::PersonRefresh => Stage::Update,
            Stage::Update => match self.kind.unwrap() {
                Kind::Primary => Stage::AfterPrimary,
                Kind::Secondary => {
                    if self.entry == Entry::Date {
                        Stage::Done
                    } else {
                        Stage::AfterSecondary
                    }
                }
                Kind::Star => Stage::Record,
            },
            Stage::Record => {
                if self.entry == Entry::Selection {
                    Stage::Pop
                } else {
                    Stage::Done
                }
            }
            Stage::DemoteDetach => Stage::DemoteRelease,
            Stage::DemoteRelease => Stage::Done,
            _ => return Err("indicator stage has no pending service".into()),
        };
        self.pending = None;
        Ok(())
    }
}
