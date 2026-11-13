//! cWinLive::Update (51ce79), including its nested refresh loops. This is the
//! ordering owner, not a replacement for the requested child operations. A
//! frontend must execute those operations before acknowledging them.
use super::Neighbor;
use serde::{Deserialize, Serialize};

/// Read live fields again after each callback. Dirty flags and the CPState
/// selected cache index are captured on entry; the other fields are not.
pub struct Inputs<'a> {
    pub dirty: u32,
    pub mode: i32,
    pub resident_slot: i32,
    pub offer_panel_present: bool,
    /// cWinLive+f0..f4. Unlike the rebuild loop, the mood loop is not capped at8.
    pub portrait_slots: &'a [bool],
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    /// CPState+1ec via4cf9a4; not4e986d's live registry selection.
    Selected,
    ///51ce99..51d05c: release/hide old owners and fetch up to8 actual family
    /// portraits through4e92a8, including names/tooltips. Not a no-op service.
    RebuildPortraits,
    Invalidate,
    Switch(i32),
    JobOffer(i32),
    /// Virtual+8c with argument1. Its byte result is not normalized to bool.
    OfferVisible,
    Visible {
        member: u16,
        shown: bool,
    },
    Motive {
        list: u16,
        index: u8,
    },
    ///473311 consumes an elapsed interval on CPState+200 (motives) or+210
    ///(relationships). It is not a pending-callback or interpolation queue.
    Elapsed(u16),
    Personality {
        index: u8,
        selected: i32,
    },
    LookupNeighbor(i32),
    Relationships,
    Inventory,
    Interests,
    ///51c5a6, including family matching and its nested category/publication work.
    PublishSelection(i32),
    ///4e97dc: current controlled family member -> image cache index.
    FamilyMember(u32),
    ///4ea6a6 is called twice: first for presence, then for actual mood. Do not
    /// cache its first result across a callback. Mood is the float at+0c.
    Motives(i32),
    PortraitMood {
        slot: u32,
        state: i32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Selected(i32),
    Bool(bool),
    Byte(u8),
    Neighbor(Option<Neighbor>),
    /// Float bits preserve NaNs and signed zero across a suspended refresh.
    Mood(Option<u32>),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Entry,
    Rebuild,
    Invalidate,
    Mode,
    Job,
    OfferQuery,
    OfferSwitch(u8),
    MotiveGate,
    Motive(u8),
    MotiveElapsed,
    Resident,
    PersonalityGate,
    Personality(u8),
    HumanLookup,
    HumanVisible,
    Extras,
    Relationships,
    RelationshipElapsed,
    Publish,
    MoodSlot,
    MoodPresence,
    MoodValue,
    MoodPublish,
    MoodNext,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Refresh {
    stage: Stage,
    pending: Option<Request>,
    flags: u32,
    selected: i32,
    offer: bool,
    neighbor: Option<Neighbor>,
    slot: u32,
    member: i32,
    mood: Option<u32>,
}
impl Default for Refresh {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
            flags: 0,
            selected: -1,
            offer: false,
            neighbor: None,
            slot: 0,
            member: -1,
            mood: None,
        }
    }
}
impl Refresh {
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        let valid = !matches!(self.stage, Stage::Motive(n) if n >= 16)
            && !matches!(self.stage, Stage::Personality(n) if n >= 5)
            && !matches!(self.stage, Stage::OfferSwitch(n) if n > 5)
            && match (&self.pending, self.stage) {
                (None, _) => true,
                (Some(Request::Selected), Stage::Rebuild)
                | (Some(Request::RebuildPortraits), Stage::Invalidate)
                | (Some(Request::Invalidate), Stage::Mode)
                | (Some(Request::Switch(0)), Stage::MotiveGate)
                | (Some(Request::Switch(1)), Stage::Job)
                | (Some(Request::OfferVisible), Stage::OfferSwitch(0)) => true,
                (Some(Request::JobOffer(n)), Stage::OfferQuery) => *n == self.selected,
                (Some(Request::Visible { member, shown }), Stage::OfferSwitch(n)) => {
                    n > 0 && Self::offer_visibility(n - 1, self.offer) == (*member, *shown)
                }
                (Some(Request::Motive { list, index }), stage) => {
                    let ordinal = match stage {
                        Stage::Motive(n) if n > 0 => n - 1,
                        Stage::MotiveElapsed => 15,
                        _ => return Err("invalid motive refresh continuation".into()),
                    };
                    (*list, *index) == Self::motive(ordinal)
                }
                (Some(Request::Elapsed(0x200)), Stage::Resident)
                | (Some(Request::Elapsed(0x210)), Stage::Publish)
                | (Some(Request::Relationships), Stage::RelationshipElapsed)
                | (Some(Request::Inventory | Request::Interests), Stage::Publish) => true,
                (Some(Request::Personality { index, selected }), stage) => {
                    *selected == self.selected
                        && match stage {
                            Stage::Personality(n) => n > 0 && *index == n - 1,
                            Stage::HumanLookup => *index == 4,
                            _ => false,
                        }
                }
                (Some(Request::LookupNeighbor(n)), Stage::HumanVisible) => *n == self.selected,
                (
                    Some(Request::Visible {
                        member: 0x1c4,
                        shown,
                    }),
                    Stage::Extras,
                ) => *shown == self.neighbor.is_some_and(|n| n.kind <= 1),
                (Some(Request::PublishSelection(n)), Stage::MoodSlot) => *n == self.selected,
                (Some(Request::FamilyMember(n)), Stage::MoodPresence) => *n == self.slot,
                (Some(Request::Motives(n)), Stage::MoodValue | Stage::MoodPublish) => {
                    *n == self.member
                }
                (Some(Request::PortraitMood { slot, state }), Stage::MoodNext) => {
                    *slot == self.slot && self.mood.is_some_and(|m| mood_state(m) == *state)
                }
                _ => false,
            };
        if valid {
            Ok(())
        } else {
            Err("live refresh lost its service".into())
        }
    }
    fn motive(n: u8) -> (u16, u8) {
        (if n % 2 == 0 { 0x128 } else { 0x134 }, n / 2)
    }
    fn offer_visibility(n: u8, offer: bool) -> (u16, bool) {
        match n {
            0 => (0x11c, offer),
            1 => (0x110, !offer),
            2 => (0x114, false),
            3 => (0x118, false),
            4 => (0x124, false),
            _ => unreachable!(),
        }
    }
    pub fn acknowledge(&mut self, expected: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("live refresh reply differs".into());
        }
        match (expected, reply) {
            (Request::Selected, Reply::Selected(n)) => self.selected = n,
            (Request::JobOffer(_), Reply::Bool(offer)) => self.offer = offer,
            (Request::OfferVisible, Reply::Byte(visible)) => {
                // Native tests zero for an offer, exactly1 for no offer.
                if !(if self.offer {
                    visible == 0
                } else {
                    visible == 1
                }) {
                    self.stage = Stage::MotiveGate;
                }
            }
            (Request::Elapsed(0x200), Reply::Bool(again)) => {
                if again {
                    self.stage = Stage::Motive(0);
                }
            }
            (Request::Elapsed(0x210), Reply::Bool(again)) => {
                if again {
                    self.stage = Stage::Relationships;
                }
            }
            (Request::LookupNeighbor(_), Reply::Neighbor(n)) => self.neighbor = n,
            (Request::FamilyMember(_), Reply::Selected(n)) => self.member = n,
            (Request::Motives(_), Reply::Mood(mood)) => self.mood = mood,
            (
                Request::RebuildPortraits
                | Request::Invalidate
                | Request::Switch(_)
                | Request::Visible { .. }
                | Request::Motive { .. }
                | Request::Personality { .. }
                | Request::Relationships
                | Request::Inventory
                | Request::Interests
                | Request::PublishSelection(_)
                | Request::PortraitMood { .. },
                Reply::Done,
            ) => (),
            _ => return Err("live refresh reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, input: Inputs<'_>) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Entry => {
                    self.flags = input.dirty;
                    self.stage = Stage::Rebuild;
                    Some(Request::Selected)
                }
                Stage::Rebuild => {
                    self.stage = Stage::Invalidate;
                    (self.flags & 0x20080000 != 0).then_some(Request::RebuildPortraits)
                }
                Stage::Invalidate => {
                    self.stage = Stage::Mode;
                    (self.flags & 9 != 0).then_some(Request::Invalidate)
                }
                Stage::Mode => {
                    if self.selected == -1 {
                        self.stage = Stage::MotiveGate;
                        Some(Request::Switch(0))
                    } else {
                        self.stage = Stage::Job;
                        (input.mode == 0).then_some(Request::Switch(1))
                    }
                }
                Stage::Job => {
                    if input.mode == 3 && input.offer_panel_present {
                        self.stage = Stage::OfferQuery;
                        Some(Request::JobOffer(self.selected))
                    } else {
                        self.stage = Stage::MotiveGate;
                        None
                    }
                }
                Stage::OfferQuery => {
                    self.stage = Stage::OfferSwitch(0);
                    Some(Request::OfferVisible)
                }
                Stage::OfferSwitch(n) => {
                    if n < 5 {
                        let (member, shown) = Self::offer_visibility(n, self.offer);
                        self.stage = Stage::OfferSwitch(n + 1);
                        Some(Request::Visible { member, shown })
                    } else {
                        self.stage = Stage::MotiveGate;
                        None
                    }
                }
                Stage::MotiveGate => {
                    self.stage = if self.flags & 9 != 0 {
                        Stage::Motive(0)
                    } else {
                        Stage::Resident
                    };
                    None
                }
                Stage::Motive(n) => {
                    let (list, index) = Self::motive(n);
                    self.stage = if n == 15 {
                        Stage::MotiveElapsed
                    } else {
                        Stage::Motive(n + 1)
                    };
                    Some(Request::Motive { list, index })
                }
                Stage::MotiveElapsed => {
                    self.stage = Stage::Resident;
                    Some(Request::Elapsed(0x200))
                }
                Stage::Resident => {
                    if input.resident_slot == -1 {
                        self.stage = Stage::MoodSlot;
                        Some(Request::PublishSelection(self.selected))
                    } else {
                        self.stage = Stage::PersonalityGate;
                        None
                    }
                }
                Stage::PersonalityGate => {
                    self.stage = if self.flags & 0x10008 == 0 {
                        Stage::Extras
                    } else if input.mode == 2 {
                        Stage::Personality(0)
                    } else {
                        Stage::HumanLookup
                    };
                    None
                }
                Stage::Personality(n) => {
                    self.stage = if n == 4 {
                        Stage::HumanLookup
                    } else {
                        Stage::Personality(n + 1)
                    };
                    Some(Request::Personality {
                        index: n,
                        selected: self.selected,
                    })
                }
                Stage::HumanLookup => {
                    self.stage = Stage::HumanVisible;
                    Some(Request::LookupNeighbor(self.selected))
                }
                Stage::HumanVisible => {
                    self.stage = Stage::Extras;
                    Some(Request::Visible {
                        member: 0x1c4,
                        shown: self.neighbor.is_some_and(|n| n.kind <= 1),
                    })
                }
                Stage::Extras => {
                    self.stage = Stage::Publish;
                    if self.flags & 0x6008008 == 0 {
                        None
                    } else {
                        match input.mode {
                            5 => {
                                self.stage = Stage::Relationships;
                                None
                            }
                            6 => Some(Request::Inventory),
                            7 => Some(Request::Interests),
                            _ => None,
                        }
                    }
                }
                Stage::Relationships => {
                    self.stage = Stage::RelationshipElapsed;
                    Some(Request::Relationships)
                }
                Stage::RelationshipElapsed => {
                    self.stage = Stage::Publish;
                    Some(Request::Elapsed(0x210))
                }
                Stage::Publish => {
                    self.stage = Stage::MoodSlot;
                    (self.flags & 8 != 0).then_some(Request::PublishSelection(self.selected))
                }
                Stage::MoodSlot => match input.portrait_slots.get(self.slot as usize) {
                    Some(true) => {
                        self.stage = Stage::MoodPresence;
                        Some(Request::FamilyMember(self.slot))
                    }
                    Some(false) => {
                        self.stage = Stage::MoodNext;
                        None
                    }
                    None => {
                        self.stage = Stage::Complete;
                        None
                    }
                },
                Stage::MoodPresence => {
                    if self.member == -1 {
                        self.stage = Stage::MoodNext;
                        None
                    } else {
                        self.stage = Stage::MoodValue;
                        Some(Request::Motives(self.member))
                    }
                }
                Stage::MoodValue => {
                    if self.mood.is_none() {
                        self.stage = Stage::MoodNext;
                        None
                    } else {
                        self.stage = Stage::MoodPublish;
                        Some(Request::Motives(self.member))
                    }
                }
                Stage::MoodPublish => {
                    let mood = self
                        .mood
                        .ok_or("native refresh would dereference missing second motives")?;
                    self.stage = Stage::MoodNext;
                    Some(Request::PortraitMood {
                        slot: self.slot,
                        state: mood_state(mood),
                    })
                }
                Stage::MoodNext => {
                    self.slot = self.slot.checked_add(1).ok_or("portrait slot overflow")?;
                    self.stage = Stage::MoodSlot;
                    None
                }
                Stage::Complete => return Ok(None),
            };
            if let Some(request) = request {
                self.pending = Some(request.clone());
                return Ok(Some(request));
            }
        }
    }
}

///51d295..51d2bb: x87 unordered comparison takes the same branch as <-25.
/// Constants66c7b8/667360 are exactly-25/+25 in the recovered memory image.
pub fn mood_state(bits: u32) -> i32 {
    let mood = f32::from_bits(bits);
    if mood.is_nan() || mood < -25. {
        2
    } else if mood > 25. {
        1
    } else {
        0
    }
}
