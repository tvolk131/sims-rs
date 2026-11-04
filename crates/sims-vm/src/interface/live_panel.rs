//! Recovered cWinLive panel selection51d375. This resumable call owns the
//! original ordering; its widget/neighbor/refresh services must really run.
//! It is not the51ce79 refresh, nor a substitute for constructing live widgets.
use serde::{Deserialize, Serialize};
pub mod history;
pub mod refresh;

/// Actual cWinLive member offsets, preserving the original hide order.
pub const PANELS: [u16; 13] = [
    0x100, 0x104, 0x108, 0x1c4, 0x110, 0x11c, 0x114, 0x118, 0x10c, 0x120, 0x124, 0x338, 0x410,
];
/// Mode -> cWinLive button member. Modes are not bitmap IDs.
pub fn button_member(mode: i32) -> Option<u16> {
    match mode {
        1 => Some(0x15c),
        2 => Some(0x16c),
        3 => Some(0x164),
        4 => Some(0x168),
        5 => Some(0x160),
        6 => Some(0x170),
        7 => Some(0x174),
        _ => None,
    }
}
/// Resource table66c5c4, also named by the original Res_CPanel.h/.RT.
pub fn button_resource(mode: i32) -> Option<i32> {
    match mode {
        1 => Some(4500),
        2 => Some(4501),
        3 => Some(4503),
        4 => Some(4502),
        5 => Some(4504),
        6 => Some(4505),
        7 => Some(4507),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
/// Live person fields returned through4e9608's cache-index lookup. These are
/// not copied NBRS values; the application's current object supplies them.
pub struct Neighbor {
    /// Signed word+600 (person data101); the original tests nonzero and <18.
    pub career: i16,
    /// Signed word+60e (person data108), including original species flags.
    pub kind: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Request {
    ClosePopup,
    Button {
        member: u16,
        state: i32,
    },
    HouseAllowed,
    Visible {
        member: u16,
        shown: bool,
    },
    ///4e986d returns the image/person cache index, not a neighborhood ID.
    SelectedNeighbor,
    LookupNeighbor(i32),
    JobOffer(i32),
    ///51d375 only shows the potentially absent offer panel when nonnull.
    OfferPanelPresent,
    ///51d375 fetches the selected neighbor again before every binding.
    BindMotive {
        list: u16,
        index: u8,
        neighbor: i32,
    },
    Dirty(u32),
    Refresh,
    Invalidate,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Bool(bool),
    Selected(i32),
    Neighbor(Option<Neighbor>),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum Stage {
    Entry,
    OldButton,
    Gate,
    Publish,
    Hide(u8),
    Branch,
    Selected,
    Lookup,
    JobSelected,
    JobOffer,
    JobPresent,
    ShowFirst,
    ShowSecond,
    MotiveSelected(u8),
    MotiveBind(u8),
    Dirty,
    NewButton,
    Refresh,
    Invalidate,
    Complete,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Switch {
    requested: i32,
    stage: Stage,
    pending: Option<Request>,
    branch: i32,
    neighbor: i32,
    data: Option<Neighbor>,
    show: Option<u16>,
}
impl Switch {
    pub fn new(requested: i32) -> Self {
        Self {
            requested,
            stage: Stage::Entry,
            pending: None,
            branch: 0,
            neighbor: -1,
            data: None,
            show: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    pub fn validate(&self) -> Result<(), String> {
        if matches!(self.stage,Stage::Hide(n) if n as usize > PANELS.len())
            || matches!(self.stage,Stage::MotiveSelected(n)|Stage::MotiveBind(n) if n >= 16)
            || self.show.is_some_and(|s| !PANELS.contains(&s))
        {
            return Err("invalid live panel continuation".into());
        }
        let valid = match (&self.pending, self.stage) {
            (None, _) => true,
            (Some(Request::ClosePopup), Stage::OldButton) => true,
            (Some(Request::Button { member, state: 0 }), Stage::Gate) => button_member(1)
                .into_iter()
                .chain((2..=7).filter_map(button_member))
                .any(|m| m == *member),
            (Some(Request::HouseAllowed), Stage::Publish) => self.requested == 4,
            (
                Some(Request::Visible {
                    member,
                    shown: false,
                }),
                Stage::Hide(n),
            ) => n > 0 && PANELS[n as usize - 1] == *member,
            (
                Some(Request::SelectedNeighbor),
                Stage::Lookup | Stage::JobOffer | Stage::MotiveBind(_),
            ) => true,
            (Some(Request::LookupNeighbor(n)), Stage::JobSelected) => *n == self.neighbor,
            (Some(Request::LookupNeighbor(n)), Stage::ShowFirst) => *n == self.neighbor,
            (Some(Request::JobOffer(n)), Stage::JobPresent) => *n == self.neighbor,
            (Some(Request::OfferPanelPresent), Stage::ShowFirst) => true,
            (Some(Request::Visible { member, .. }), Stage::ShowSecond) => {
                matches!(self.branch, 1 | 2) && matches!(member, 0x100 | 0x1c4)
            }
            (Some(Request::Visible { member, .. }), Stage::MotiveSelected(0)) => {
                self.branch == 1 && *member == 0x104
            }
            (
                Some(Request::Visible {
                    member,
                    shown: true,
                }),
                Stage::Dirty,
            ) => self.show == Some(*member),
            (
                Some(Request::BindMotive {
                    list,
                    index,
                    neighbor,
                }),
                Stage::MotiveSelected(n),
            ) => {
                n > 0
                    && *list == if (n - 1) % 2 == 0 { 0x128 } else { 0x134 }
                    && *index == (n - 1) / 2
                    && *neighbor == self.neighbor
            }
            (
                Some(Request::BindMotive {
                    list: 0x134,
                    index: 7,
                    neighbor,
                }),
                Stage::Dirty,
            ) => *neighbor == self.neighbor && self.branch == 1,
            (Some(Request::Dirty(flags)), Stage::NewButton) => {
                Self::dirty(self.branch) == Some(*flags)
            }
            (Some(Request::Button { member, state: 1 }), Stage::Refresh) => {
                button_member(self.branch) == Some(*member)
            }
            (Some(Request::Refresh), Stage::Invalidate)
            | (Some(Request::Invalidate), Stage::Complete) => true,
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("live panel lost its pending service".into())
        }
    }
    fn dirty(mode: i32) -> Option<u32> {
        match mode {
            1 => Some(1),
            2 => Some(0x10000),
            3 => Some(0x20000),
            4 => Some(0x40000),
            5 => Some(0x8000),
            6 => Some(0x2000000),
            7 => Some(0x4000000),
            _ => None,
        }
    }
    pub fn acknowledge(&mut self, expected: &Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending.as_ref() != Some(expected) {
            return Err("live panel reply differs".into());
        }
        match (expected, reply) {
            (Request::HouseAllowed, Reply::Bool(allowed)) => {
                if !allowed {
                    self.requested = 0;
                }
            }
            (Request::SelectedNeighbor, Reply::Selected(n)) => self.neighbor = n,
            (Request::LookupNeighbor(_), Reply::Neighbor(n)) => self.data = n,
            (Request::JobOffer(_), Reply::Bool(offer)) => {
                self.show = Some(if offer { 0x11c } else { 0x110 });
                if !offer {
                    self.stage = Stage::ShowFirst;
                }
            }
            (Request::OfferPanelPresent, Reply::Bool(present)) => {
                if !present {
                    self.show = None;
                }
            }
            (
                Request::ClosePopup
                | Request::Button { .. }
                | Request::Visible { .. }
                | Request::BindMotive { .. }
                | Request::Dirty(_)
                | Request::Refresh
                | Request::Invalidate,
                Reply::Done,
            ) => (),
            _ => return Err("live panel reply kind differs".into()),
        }
        self.pending = None;
        Ok(())
    }
    /// `mode` is cWinLive+fc, reread after synchronous callbacks. The caller
    /// supplies the actual current popup presence; no widget is synthesized.
    pub fn advance(
        &mut self,
        mode: &mut i32,
        popup_present: bool,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending.clone());
        }
        loop {
            let request = match self.stage {
                Stage::Entry => {
                    self.stage = Stage::OldButton;
                    (*mode != self.requested && popup_present).then_some(Request::ClosePopup)
                }
                Stage::OldButton => {
                    self.stage = Stage::Gate;
                    button_member(*mode).map(|member| Request::Button { member, state: 0 })
                }
                Stage::Gate => {
                    self.stage = Stage::Publish;
                    (self.requested == 4).then_some(Request::HouseAllowed)
                }
                Stage::Publish => {
                    *mode = self.requested;
                    self.stage = Stage::Hide(0);
                    None
                }
                Stage::Hide(n) => {
                    if let Some(&member) = PANELS.get(n as usize) {
                        self.stage = Stage::Hide(n + 1);
                        Some(Request::Visible {
                            member,
                            shown: false,
                        })
                    } else {
                        self.stage = Stage::Branch;
                        None
                    }
                }
                Stage::Branch => {
                    self.branch = *mode;
                    self.show = match self.branch {
                        2 => Some(0x108),
                        4 => Some(0x120),
                        5 => Some(0x10c),
                        6 => Some(0x338),
                        7 => Some(0x410),
                        _ => None,
                    };
                    self.stage = match self.branch {
                        0 => Stage::Complete,
                        1..=3 => Stage::Selected,
                        4..=7 => Stage::ShowFirst,
                        _ => Stage::Refresh,
                    };
                    None
                }
                Stage::Selected => {
                    self.stage = Stage::Lookup;
                    Some(Request::SelectedNeighbor)
                }
                Stage::Lookup => {
                    self.stage = if self.branch == 3 {
                        Stage::JobSelected
                    } else {
                        Stage::ShowFirst
                    };
                    Some(Request::LookupNeighbor(self.neighbor))
                }
                Stage::JobSelected => {
                    self.stage = Stage::JobOffer;
                    Some(Request::SelectedNeighbor)
                }
                Stage::JobOffer => {
                    // Assembly permits negative career words too; it tests
                    // !=0 and signed <18, not a conventional positive range.
                    let d = self
                        .data
                        .ok_or("native job panel would dereference a missing neighbor")?;
                    self.show = if d.career != 0 && d.career < 18 {
                        Some(0x124)
                    } else if d.kind as u16 & 2 != 0 {
                        Some(0x114)
                    } else if d.kind as u16 & 4 != 0 {
                        Some(0x118)
                    } else {
                        None
                    };
                    if self.show.is_some() {
                        self.stage = Stage::ShowFirst;
                        None
                    } else {
                        self.stage = Stage::JobPresent;
                        Some(Request::JobOffer(self.neighbor))
                    }
                }
                Stage::JobPresent => {
                    self.stage = Stage::ShowFirst;
                    Some(Request::OfferPanelPresent)
                }
                Stage::ShowFirst => {
                    if matches!(self.branch, 1 | 2) {
                        let shown = self.data.is_some_and(|d| d.kind <= 1);
                        self.stage = Stage::ShowSecond;
                        Some(Request::Visible {
                            member: if self.branch == 1 { 0x100 } else { 0x1c4 },
                            shown,
                        })
                    } else {
                        self.stage = Stage::Dirty;
                        self.show.map(|member| Request::Visible {
                            member,
                            shown: true,
                        })
                    }
                }
                Stage::ShowSecond => {
                    self.stage = if self.branch == 1 {
                        Stage::MotiveSelected(0)
                    } else {
                        Stage::Dirty
                    };
                    Some(Request::Visible {
                        member: if self.branch == 1 { 0x104 } else { 0x108 },
                        shown: self.branch == 2 || !self.data.is_some_and(|d| d.kind <= 1),
                    })
                }
                Stage::MotiveSelected(n) => {
                    self.stage = Stage::MotiveBind(n);
                    Some(Request::SelectedNeighbor)
                }
                Stage::MotiveBind(n) => {
                    self.stage = if n == 15 {
                        Stage::Dirty
                    } else {
                        Stage::MotiveSelected(n + 1)
                    };
                    Some(Request::BindMotive {
                        list: if n % 2 == 0 { 0x128 } else { 0x134 },
                        index: n / 2,
                        neighbor: self.neighbor,
                    })
                }
                Stage::Dirty => {
                    self.stage = Stage::NewButton;
                    Self::dirty(self.branch).map(Request::Dirty)
                }
                Stage::NewButton => {
                    self.stage = Stage::Refresh;
                    button_member(self.branch).map(|member| Request::Button { member, state: 1 })
                }
                Stage::Refresh => {
                    self.stage = Stage::Invalidate;
                    Some(Request::Refresh)
                }
                Stage::Invalidate => {
                    self.stage = Stage::Complete;
                    Some(Request::Invalidate)
                }
                Stage::Complete => return Ok(None),
            };
            if let Some(request) = request {
                self.pending = Some(request.clone());
                self.validate()?;
                return Ok(Some(request));
            }
        }
    }
}
