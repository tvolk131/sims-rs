//! 53ab6d ViewControl publication. This is the caller, not an implementation of
//! its widget, CPState or lot children. Owners and string references survive
//! synchronous callbacks; a backend must perform each requested child itself.
use super::state::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod buttons;
pub mod expansion;
pub mod timer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Widget {
    ZoomIn,
    ZoomOut,
    WallsDown,
    WallsUp,
    Cutaway,
    Live,
    Buy,
    Build,
    Options,
    Camera,
    Status,
    FloorOne,
    FloorTwo,
    Roof,
    Normal,
    Fast,
    Ultra,
    Label,
}
impl Widget {
    pub fn offset(self) -> u16 {
        [
            0xe0, 0xe4, 0xe8, 0xec, 0xf0, 0xf4, 0xf8, 0xfc, 0x100, 0x104, 0x108, 0x10c, 0x110,
            0x114, 0x11c, 0x120, 0x124, 0x1d0,
        ][self as usize]
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    /// Stable owner identity and field, not a copied string or a process pointer.
    pub owner: u32,
    pub offset: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blink {
    pub phase: u32,    // +17c
    pub interval: u32, // +180
    pub deadline: u32, // +184
    pub last: u32,     // +188
    pub first: u32,    // +18c
    pub second: u32,   // +190
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    /// ViewControl+d4, distinct from the retained update argument.
    pub registered: u32,
    pub widgets: [u32; 18],
    pub blink: Blink,
}
impl Fields {
    pub fn widget(&self, w: Widget) -> u32 {
        self.widgets[w as usize]
    }
}
#[derive(Debug, Clone, Copy)]
pub struct Inputs {
    /// Argument CPState+54; captured only on entry.
    pub panel: u32,
    /// Current process lot owner; reread at each lot call.
    pub lot: u32,
    pub now: u32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Query {
    Mode,
    Floor,
    BuildFloorAvailable,
    UpperFloorAvailable,
    Walls,
    AvailableWalls,
    ZoomIn,
    ZoomOut,
    Status,
    Title,
    LotPaused,
    SelectionName,
    SavedSpeedActive,
    SavedSpeed,
    Speed,
    Expansion,
    BuyBlocked,
    LiveBlocked,
    LabelsHidden,
}
impl Query {
    pub fn address(self) -> u32 {
        match self {
            Self::Mode => 0x416920,
            Self::Floor => 0x4cea19,
            Self::BuildFloorAvailable => 0x4cfabd,
            Self::UpperFloorAvailable => 0x4cfab1,
            Self::Walls => 0x5ecdca,
            Self::AvailableWalls => 0x4cfa9c,
            Self::ZoomIn => 0x4ce9ee,
            Self::ZoomOut => 0x4ce9f8,
            Self::Status => 0x4ce9ea,
            Self::Title => 0x5c184d,
            Self::LotPaused => 0x46d866,
            Self::SelectionName => 0x4cfda4,
            Self::SavedSpeedActive => 0x46d7cc,
            Self::SavedSpeed => 0x46d7d6,
            Self::Speed => 0x46d78b,
            Self::Expansion => 0x4cfc6e,
            Self::BuyBlocked => 0x4cea1d,
            Self::LiveBlocked => 0x4cea2f,
            Self::LabelsHidden => 0x4cfe18,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Get {
        owner: u32,
        query: Query,
    },
    Invalidate(u32),
    Buttons {
        view: u32,
        selector: i32,
    }, //53b536
    Panel {
        panel: u32,
        selector: i32,
    }, //50a7ec
    Selected {
        widget: u32,
        value: i32,
    }, //5e480a, not the enable byte
    Floor {
        state: u32,
        value: i32,
        refresh: u8,
    }, //4cee08
    Speed {
        state: u32,
        value: i32,
    }, //4cee89, including both timers
    Paused {
        state: u32,
        value: u8,
    }, //4cee4a
    Expansion {
        view: u32,
        value: i32,
    }, //53b2eb
    Length(Text), //5d4b41
    Equal {
        left: Text,
        right: Text,
    }, //5b8897, low byte result
    Copy {
        target: Text,
        source: Text,
    }, //5d4b12
    CopyCString {
        target: Text,
        source: u32,
    }, //5d4cf9
    SetText {
        widget: u32,
        source: Text,
    }, //virtual+9c
    /// Preserve the original virtual slot until the concrete widget is bound.
    Visibility {
        widget: u32,
        slot: u16,
    }, //virtual+94/+98
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Number(u32),
    Text(Text),
}
impl Request {
    fn accepts(self, reply: Reply) -> bool {
        match (self, reply) {
            (
                Self::Get {
                    query: Query::Status | Query::Title,
                    ..
                },
                Reply::Text(_),
            ) => true,
            (
                Self::Get {
                    query: Query::Status | Query::Title,
                    ..
                },
                _,
            ) => false,
            (Self::Get { .. } | Self::Length(_) | Self::Equal { .. }, Reply::Number(_)) => true,
            (Self::Get { .. } | Self::Length(_) | Self::Equal { .. }, _) => false,
            (_, Reply::Done) => true,
            _ => false,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    ModeButtons,
    ModeRead,
    ModeResult,
    ModeSelected(i32, u8),
    Floor,
    FloorOne,
    FloorReadTwo,
    FloorTwo,
    EnableFloorOne,
    FloorMode,
    FloorModeResult,
    BuildAvailable,
    UpperAvailable,
    EnableUpper(bool),
    DisabledFloor,
    DisabledFloorResult,
    SelectFloorOne,
    Walls,
    WallsResult,
    WallSelected(u32, u8),
    WallAvailability,
    WallAvailabilityResult,
    WallEnabled(u32, u8),
    Zoom,
    ZoomFirst,
    ZoomSecond(u8),
    ZoomEnabled([u8; 2], u8),
    Status,
    StatusLength,
    StatusWidgetLength,
    StatusCompare,
    StatusResult,
    StatusCopied,
    StatusTextDone(u32, TextFinish),
    StatusInvalidated(TextFinish),
    StatusDeselected,
    Title,
    TitleCompare,
    TitleResult,
    TitleCopied,
    Selection,
    SelectionPaused,
    SelectionPausedResult,
    SelectionName,
    SelectionCopy,
    Speed,
    SpeedOverride,
    SpeedRead,
    SpeedResult,
    SpeedSelected(u32, u8),
    Pause,
    Expansion,
    ExpansionFirst,
    ExpansionSecond,
    Buy,
    BuyResult,
    BuyEnabled(u8, u8),
    Live,
    LiveResult,
    LiveEnabled(u8, u8),
    Labels,
    LabelsResult,
    LabelSecond(u16),
    Complete,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum TextFinish {
    Reset,
    Second,
    Preserve,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Update {
    view: u32,
    argument: u32,
    mask: u32,
    panel: u32,
    stage: Stage,
    pending: Option<Request>,
    answer: Reply,
    source: Text,
}
impl Update {
    pub fn matches(&self, view: u32, argument: u32) -> bool {
        self.view == view && self.argument == argument
    }
    pub fn new(view: u32, argument: u32) -> Self {
        Self {
            view,
            argument,
            mask: 0,
            panel: 0,
            stage: Stage::Entry,
            pending: None,
            answer: Reply::Done,
            source: Text {
                owner: 0,
                offset: 0,
            },
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        use Stage as S;
        let valid_index = match self.stage {
            S::ModeSelected(_, n) => n <= 5,
            S::WallSelected(_, n) => n <= 4,
            S::WallEnabled(_, n) | S::SpeedSelected(_, n) => n <= 3,
            S::ZoomEnabled(_, n) | S::BuyEnabled(_, n) | S::LiveEnabled(_, n) => n <= 2,
            _ => true,
        };
        if !valid_index {
            return Err("ViewControl continuation index invalid".into());
        }
        let required = match self.stage {
            S::ModeButtons | S::ModeRead | S::ModeResult | S::ModeSelected(..) => 2,
            S::FloorOne
            | S::FloorReadTwo
            | S::FloorTwo
            | S::EnableFloorOne
            | S::FloorMode
            | S::FloorModeResult
            | S::BuildAvailable
            | S::UpperAvailable
            | S::EnableUpper(_)
            | S::DisabledFloor
            | S::DisabledFloorResult
            | S::SelectFloorOne => 0x100,
            S::WallsResult
            | S::WallSelected(..)
            | S::WallAvailability
            | S::WallAvailabilityResult
            | S::WallEnabled(..) => 0x80,
            S::ZoomFirst | S::ZoomSecond(_) | S::ZoomEnabled(..) => 0x1000,
            S::StatusLength
            | S::StatusWidgetLength
            | S::StatusCompare
            | S::StatusResult
            | S::StatusCopied
            | S::StatusTextDone(..)
            | S::StatusInvalidated(_)
            | S::StatusDeselected => 0x4000,
            S::TitleCompare | S::TitleResult | S::TitleCopied => 0x2000,
            S::SelectionPaused | S::SelectionPausedResult | S::SelectionName | S::SelectionCopy => {
                0x80000
            }
            S::SpeedOverride | S::SpeedRead | S::SpeedResult | S::SpeedSelected(..) => 0x42,
            S::ExpansionFirst | S::ExpansionSecond => 0x100000,
            S::BuyResult | S::BuyEnabled(..) => 0x1000000,
            S::LiveResult
            | S::LiveEnabled(..)
            | S::Labels
            | S::LabelsResult
            | S::LabelSecond(_) => 0x8000000,
            _ => 0,
        };
        if required != 0 && self.mask & required == 0 {
            return Err("ViewControl continuation lost its captured dirty mask".into());
        }
        if let Some(r) = self.pending {
            if !self.expected(r) {
                return Err("ViewControl lost its child continuation".into());
            }
            if let Request::Get { owner, query } = r {
                let argument_query = matches!(
                    query,
                    Query::Mode
                        | Query::Floor
                        | Query::BuildFloorAvailable
                        | Query::UpperFloorAvailable
                        | Query::Walls
                        | Query::AvailableWalls
                        | Query::Status
                        | Query::Title
                        | Query::Expansion
                        | Query::BuyBlocked
                        | Query::LiveBlocked
                        | Query::LabelsHidden
                );
                if argument_query && owner != self.argument {
                    return Err("ViewControl getter lost its argument owner".into());
                }
            }
        } else {
            match self.stage {
                S::ModeResult
                | S::FloorOne
                | S::FloorTwo
                | S::FloorModeResult
                | S::BuildAvailable
                | S::UpperAvailable
                | S::DisabledFloorResult
                | S::WallsResult
                | S::WallAvailabilityResult
                | S::ZoomFirst
                | S::ZoomSecond(_)
                | S::StatusWidgetLength
                | S::StatusCompare
                | S::StatusResult
                | S::TitleResult
                | S::SelectionPausedResult
                | S::SelectionCopy
                | S::SpeedRead
                | S::SpeedResult
                | S::ExpansionFirst
                | S::ExpansionSecond
                | S::BuyResult
                | S::LiveResult
                | S::LabelsResult => {
                    self.number()?;
                }
                S::StatusLength | S::TitleCompare => {
                    self.text()?;
                }
                _ => (),
            }
        }
        Ok(())
    }
    fn expected(&self, r: Request) -> bool {
        use Request as R;
        use Stage as S;
        // The continuation is stored before entering the child. Keep the
        // request itself, including owner identities, stable until it returns.
        match (self.stage, r) {
            (S::ModeButtons | S::FloorMode, R::Invalidate(_)) => true,
            (S::ModeRead, R::Buttons { selector: 3, .. })
            | (S::SpeedOverride, R::Buttons { selector: 0, .. })
            | (S::Expansion, R::Buttons { selector: 1, .. }) => true,
            (
                S::ModeResult | S::FloorModeResult,
                R::Get {
                    query: Query::Mode, ..
                },
            ) => true,
            (S::ModeSelected(_, 0), R::Panel { .. }) => true,
            (
                S::ModeSelected(_, 1..=5)
                | S::FloorReadTwo
                | S::EnableFloorOne
                | S::WallSelected(_, 1..=4)
                | S::SpeedSelected(_, 1..=3)
                | S::StatusDeselected,
                R::Selected { .. },
            ) => true,
            (
                S::FloorOne | S::FloorTwo | S::DisabledFloorResult,
                R::Get {
                    query: Query::Floor,
                    ..
                },
            ) => true,
            (
                S::BuildAvailable,
                R::Get {
                    query: Query::BuildFloorAvailable,
                    ..
                },
            ) => true,
            (
                S::UpperAvailable,
                R::Get {
                    query: Query::UpperFloorAvailable,
                    ..
                },
            ) => true,
            (
                S::Walls
                | S::DisabledFloor
                | S::WallEnabled(_, 1..=3)
                | S::ZoomEnabled(_, 1..=2)
                | S::BuyEnabled(_, 1..=2)
                | S::LiveEnabled(_, 1..=2),
                R::Invalidate(_),
            ) => true,
            (
                S::SelectFloorOne,
                R::Floor {
                    value: 1,
                    refresh: 0,
                    ..
                },
            ) => true,
            (S::Walls, R::Selected { value: 1, .. }) => true,
            (
                S::WallsResult,
                R::Get {
                    query: Query::Walls,
                    ..
                },
            ) => true,
            (
                S::WallAvailabilityResult,
                R::Get {
                    query: Query::AvailableWalls,
                    ..
                },
            ) => true,
            (
                S::ZoomFirst,
                R::Get {
                    query: Query::ZoomIn,
                    ..
                },
            ) => true,
            (
                S::ZoomSecond(_),
                R::Get {
                    query: Query::ZoomOut,
                    ..
                },
            ) => true,
            (
                S::StatusLength,
                R::Get {
                    query: Query::Status,
                    ..
                },
            ) => true,
            (S::StatusWidgetLength | S::StatusCompare, R::Length(_)) => true,
            (S::StatusResult | S::TitleResult, R::Equal { .. }) => true,
            (S::StatusCopied | S::Title | S::TitleCopied, R::Copy { .. }) => true,
            (S::StatusTextDone(_, _), R::SetText { .. }) => true,
            (S::StatusInvalidated(_), R::Invalidate(_)) => true,
            (S::Title, R::Selected { value: 1, .. }) => true,
            (
                S::TitleCompare,
                R::Get {
                    query: Query::Title,
                    ..
                },
            ) => true,
            (S::Selection, R::Invalidate(_)) => true,
            (S::SelectionPaused, R::Speed { value: 760, .. }) => true,
            (
                S::SelectionPausedResult,
                R::Get {
                    query: Query::LotPaused,
                    ..
                },
            ) => true,
            (S::SelectionName, R::Paused { value: 1, .. }) => true,
            (
                S::SelectionCopy,
                R::Get {
                    query: Query::SelectionName,
                    ..
                },
            ) => true,
            (S::Speed, R::CopyCString { .. }) => true,
            (
                S::SpeedRead,
                R::Get {
                    query: Query::SavedSpeedActive,
                    ..
                },
            ) => true,
            (
                S::SpeedResult,
                R::Get {
                    query: Query::SavedSpeed | Query::Speed,
                    ..
                },
            ) => true,
            (
                S::ExpansionFirst | S::ExpansionSecond,
                R::Get {
                    query: Query::Expansion,
                    ..
                },
            ) => true,
            (S::Buy, R::Expansion { value: 0 | 1, .. }) => true,
            (
                S::BuyResult,
                R::Get {
                    query: Query::BuyBlocked,
                    ..
                },
            ) => true,
            (
                S::LiveResult,
                R::Get {
                    query: Query::LiveBlocked,
                    ..
                },
            ) => true,
            (
                S::LabelsResult,
                R::Get {
                    query: Query::LabelsHidden,
                    ..
                },
            ) => true,
            (
                S::LabelSecond(0x94 | 0x98) | S::Complete,
                R::Visibility {
                    slot: 0x94 | 0x98, ..
                },
            ) => true,
            _ => false,
        }
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) || !request.accepts(reply) {
            return Err("ViewControl reply differs".into());
        }
        self.pending = None;
        self.answer = reply;
        Ok(())
    }
    fn number(&self) -> Result<u32, String> {
        match self.answer {
            Reply::Number(n) => Ok(n),
            _ => Err("ViewControl numeric result absent".into()),
        }
    }
    fn text(&self) -> Result<Text, String> {
        match self.answer {
            Reply::Text(t) => Ok(t),
            _ => Err("ViewControl text reference absent".into()),
        }
    }
    fn get(&self, owner: u32, query: Query) -> Request {
        Request::Get { owner, query }
    }
    fn own_text(&self, offset: u16) -> Text {
        Text {
            owner: self.view,
            offset,
        }
    }
    fn selected(f: &Fields, w: Widget, value: i32) -> Request {
        Request::Selected {
            widget: f.widget(w),
            value,
        }
    }
    fn enable(
        f: &Fields,
        widgets: &mut BTreeMap<u32, u8>,
        w: Widget,
        value: u8,
    ) -> Result<Option<Request>, String> {
        let id = f.widget(w);
        let old = widgets
            .get_mut(&id)
            .filter(|_| id != 0)
            .ok_or("ViewControl enable-byte owner absent")?;
        if *old == value {
            return Ok(None);
        }
        *old = value;
        Ok(Some(Request::Invalidate(id)))
    }
    fn set_text(&mut self, f: &Fields, source: Text, finish: TextFinish) -> Request {
        let widget = f.widget(Widget::Status);
        self.stage = Stage::StatusTextDone(widget, finish);
        Request::SetText { widget, source }
    }
    /// The passed State is the original argument's actual shared CPState.
    /// The enable-byte map is owned by the widget backend, not a local cache.
    pub fn advance(
        &mut self,
        f: &mut Fields,
        state: &mut State,
        widgets: &mut BTreeMap<u32, u8>,
        i: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        use Query as Q;
        use Stage as S;
        use Widget as W;
        let r = loop {
            match self.stage {
                S::Entry => {
                    self.mask = state.buy.dirty;
                    self.panel = i.panel;
                    self.stage = S::Floor;
                    if self.mask & 2 != 0 {
                        self.stage = S::ModeButtons;
                        break Request::Invalidate(self.view);
                    }
                }
                S::ModeButtons => {
                    self.stage = S::ModeRead;
                    break Request::Buttons {
                        view: self.view,
                        selector: 3,
                    };
                }
                S::ModeRead => {
                    self.stage = S::ModeResult;
                    break self.get(self.argument, Q::Mode);
                }
                S::ModeResult => {
                    let mode = self.number()? as i32;
                    self.stage = S::ModeSelected(mode, 0);
                    break Request::Panel {
                        panel: self.panel,
                        selector: match mode {
                            0 => 2,
                            1 => 1,
                            3 => 3,
                            4 => 5,
                            _ => 0,
                        },
                    };
                }
                S::ModeSelected(mode, n) => {
                    if n == 5 {
                        self.stage = S::Floor;
                        continue;
                    }
                    self.stage = S::ModeSelected(mode, n + 1);
                    let (w, m) = [
                        (W::Live, 2),
                        (W::Buy, 1),
                        (W::Build, 0),
                        (W::Options, 3),
                        (W::Camera, 4),
                    ][n as usize];
                    break Self::selected(f, w, i32::from(mode == m));
                }
                S::Floor => {
                    self.stage = S::Walls;
                    if self.mask & 0x100 != 0 {
                        self.stage = S::FloorOne;
                        break self.get(self.argument, Q::Floor);
                    }
                }
                S::FloorOne => {
                    let v = i32::from(self.number()? == 1);
                    self.stage = S::FloorReadTwo;
                    break Self::selected(f, W::FloorOne, v);
                }
                S::FloorReadTwo => {
                    self.stage = S::FloorTwo;
                    break self.get(self.argument, Q::Floor);
                }
                S::FloorTwo => {
                    let v = i32::from(self.number()? == 2);
                    self.stage = S::EnableFloorOne;
                    break Self::selected(f, W::FloorTwo, v);
                }
                S::EnableFloorOne => {
                    let r = Self::enable(f, widgets, W::FloorOne, 1)?;
                    self.stage = S::FloorMode;
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::FloorMode => {
                    self.stage = S::FloorModeResult;
                    break self.get(self.argument, Q::Mode);
                }
                S::FloorModeResult => {
                    if self.number()? == 0 {
                        self.stage = S::BuildAvailable;
                        break self.get(self.argument, Q::BuildFloorAvailable);
                    }
                    self.stage = S::UpperAvailable;
                    break self.get(self.argument, Q::UpperFloorAvailable);
                }
                S::BuildAvailable => {
                    if self.number()? as u8 != 0 {
                        self.stage = S::EnableUpper(true);
                    } else {
                        self.stage = S::UpperAvailable;
                        break self.get(self.argument, Q::UpperFloorAvailable);
                    }
                }
                S::UpperAvailable => self.stage = S::EnableUpper(self.number()? as u8 != 0),
                S::EnableUpper(enabled) => {
                    let r = Self::enable(f, widgets, W::FloorTwo, u8::from(enabled))?;
                    self.stage = if enabled { S::Walls } else { S::DisabledFloor };
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::DisabledFloor => {
                    self.stage = S::DisabledFloorResult;
                    break self.get(self.argument, Q::Floor);
                }
                S::DisabledFloorResult => {
                    self.stage = S::Walls;
                    if self.number()? == 2 {
                        self.stage = S::SelectFloorOne;
                        break Request::Floor {
                            state: self.argument,
                            value: 1,
                            refresh: 0,
                        };
                    }
                }
                S::SelectFloorOne => {
                    self.stage = S::Walls;
                    break Self::selected(f, W::FloorOne, 1);
                }
                S::Walls => {
                    self.stage = S::Zoom;
                    if self.mask & 0x80 != 0 {
                        self.stage = S::WallsResult;
                        break self.get(self.argument, Q::Walls);
                    }
                }
                S::WallsResult => self.stage = S::WallSelected(self.number()?, 0),
                S::WallSelected(mode, n) => {
                    if n == 4 {
                        self.stage = S::WallAvailability;
                        continue;
                    }
                    self.stage = S::WallSelected(mode, n + 1);
                    let (w, m) = [
                        (W::WallsDown, 1),
                        (W::WallsUp, 4),
                        (W::Cutaway, 2),
                        (W::Roof, 8),
                    ][n as usize];
                    break Self::selected(f, w, i32::from(mode == m));
                }
                S::WallAvailability => {
                    self.stage = S::WallAvailabilityResult;
                    break self.get(self.argument, Q::AvailableWalls);
                }
                S::WallAvailabilityResult => self.stage = S::WallEnabled(self.number()?, 0),
                S::WallEnabled(mask, n) => {
                    if n == 3 {
                        self.stage = S::Zoom;
                        continue;
                    }
                    let (w, bit) =
                        [(W::WallsDown, 1), (W::WallsUp, 4), (W::Cutaway, 2)][n as usize];
                    let r = Self::enable(f, widgets, w, u8::from(mask & bit != 0))?;
                    self.stage = S::WallEnabled(mask, n + 1);
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::Zoom => {
                    self.stage = S::Status;
                    if self.mask & 0x1000 != 0 {
                        self.stage = S::ZoomFirst;
                        break self.get(f.registered, Q::ZoomIn);
                    }
                }
                S::ZoomFirst => {
                    self.stage = S::ZoomSecond(self.number()? as u8);
                    break self.get(f.registered, Q::ZoomOut);
                }
                S::ZoomSecond(a) => self.stage = S::ZoomEnabled([a, self.number()? as u8], 0),
                S::ZoomEnabled(values, n) => {
                    if n == 2 {
                        self.stage = S::Status;
                        continue;
                    }
                    let r = Self::enable(
                        f,
                        widgets,
                        [W::ZoomIn, W::ZoomOut][n as usize],
                        values[n as usize],
                    )?;
                    self.stage = S::ZoomEnabled(values, n + 1);
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::Status => {
                    self.stage = S::Title;
                    if self.mask & 0x4000 != 0 {
                        self.stage = S::StatusLength;
                        break self.get(self.argument, Q::Status);
                    }
                }
                S::StatusLength => {
                    self.source = self.text()?;
                    self.stage = S::StatusWidgetLength;
                    break Request::Length(self.source);
                }
                S::StatusWidgetLength => {
                    if self.number()? == 0 {
                        break self.set_text(f, self.source, TextFinish::Reset);
                    }
                    self.stage = S::StatusCompare;
                    break Request::Length(Text {
                        owner: f.widget(W::Status),
                        offset: 0x4c,
                    });
                }
                S::StatusCompare => {
                    if self.number()? == 0 {
                        break self.set_text(f, self.source, TextFinish::Reset);
                    }
                    self.stage = S::StatusResult;
                    break Request::Equal {
                        left: self.source,
                        right: Text {
                            owner: f.widget(W::Status),
                            offset: 0x4c,
                        },
                    };
                }
                S::StatusResult => {
                    let changed = self.number()? as u8 == 0;
                    let phase = f.blink.phase;
                    self.stage = S::Title;
                    if phase == 0 {
                        if changed {
                            self.stage = S::StatusCopied;
                            break Request::Copy {
                                target: self.own_text(0x194),
                                source: self.source,
                            };
                        }
                    } else if i.now < f.blink.deadline {
                        if changed {
                            if phase == 1 {
                                break Request::Copy {
                                    target: self.own_text(0x194),
                                    source: self.source,
                                };
                            }
                            break self.set_text(f, self.source, TextFinish::Preserve);
                        }
                    } else {
                        f.blink.deadline = u32::MAX;
                        match f.blink.phase {
                            1 => break self.set_text(f, self.own_text(0x194), TextFinish::Second),
                            2 => {
                                self.stage = S::StatusDeselected;
                                break Self::selected(f, W::Status, 0);
                            }
                            _ => (),
                        }
                    }
                }
                S::StatusCopied => {
                    let p = state
                        .publication
                        .as_mut()
                        .ok_or("ViewControl after-dirty owner absent")?;
                    f.blink.phase = 1;
                    f.blink.interval = f.blink.first;
                    f.blink.deadline = i.now.wrapping_add(f.blink.first);
                    f.blink.last = f.blink.deadline;
                    p.after_dirty |= 0x4000;
                    self.stage = S::Title;
                    break Self::selected(f, W::Status, 1);
                }
                S::StatusTextDone(widget, finish) => {
                    self.stage = S::StatusInvalidated(finish);
                    break Request::Invalidate(widget);
                }
                S::StatusInvalidated(finish) => {
                    match finish {
                        TextFinish::Reset => f.blink.phase = 0,
                        TextFinish::Preserve => (),
                        TextFinish::Second => {
                            f.blink.phase = 2;
                            f.blink.interval = f.blink.second;
                            f.blink.deadline = i.now.wrapping_add(f.blink.second);
                            f.blink.last = f.blink.deadline;
                        }
                    }
                    self.stage = S::Title;
                }
                S::StatusDeselected => {
                    let p = state
                        .publication
                        .as_mut()
                        .ok_or("ViewControl after-dirty owner absent")?;
                    f.blink.phase = 0;
                    p.after_dirty &= !0x4000;
                    self.stage = S::Title;
                }
                S::Title => {
                    self.stage = S::Selection;
                    if self.mask & 0x2000 != 0 {
                        self.stage = S::TitleCompare;
                        break self.get(self.argument, Q::Title);
                    }
                }
                S::TitleCompare => {
                    self.source = self.text()?;
                    self.stage = S::TitleResult;
                    break Request::Equal {
                        left: self.source,
                        right: self.own_text(0x170),
                    };
                }
                S::TitleResult => {
                    self.stage = S::Selection;
                    if self.number()? as u8 == 0 {
                        self.stage = S::TitleCopied;
                        break Request::Copy {
                            target: self.own_text(0x170),
                            source: self.source,
                        };
                    }
                }
                S::TitleCopied => {
                    self.stage = S::Selection;
                    break Request::Invalidate(self.view);
                }
                S::Selection => {
                    self.stage = S::Speed;
                    if self.mask & 0x80000 != 0 {
                        self.stage = S::SelectionPaused;
                        break Request::Speed {
                            state: f.registered,
                            value: 760,
                        };
                    }
                }
                S::SelectionPaused => {
                    self.stage = S::SelectionPausedResult;
                    break self.get(i.lot, Q::LotPaused);
                }
                S::SelectionPausedResult => {
                    self.stage = S::SelectionName;
                    if self.number()? as u8 != 0 {
                        break Request::Paused {
                            state: f.registered,
                            value: 1,
                        };
                    }
                }
                S::SelectionName => {
                    self.stage = S::SelectionCopy;
                    break self.get(f.registered, Q::SelectionName);
                }
                S::SelectionCopy => {
                    let source = self.number()?;
                    self.stage = S::Speed;
                    break Request::CopyCString {
                        target: self.own_text(0x1d4),
                        source,
                    };
                }
                S::Speed => {
                    self.stage = S::Pause;
                    if self.mask & 0x42 != 0 {
                        self.stage = S::SpeedOverride;
                        break Request::Buttons {
                            view: self.view,
                            selector: 0,
                        };
                    }
                }
                S::SpeedOverride => {
                    self.stage = S::SpeedRead;
                    break self.get(i.lot, Q::SavedSpeedActive);
                }
                S::SpeedRead => {
                    let q = if self.number()? as u8 != 0 {
                        Q::SavedSpeed
                    } else {
                        Q::Speed
                    };
                    self.stage = S::SpeedResult;
                    break self.get(i.lot, q);
                }
                S::SpeedResult => self.stage = S::SpeedSelected(self.number()?, 0),
                S::SpeedSelected(speed, n) => {
                    if n == 3 {
                        self.stage = S::Pause;
                        continue;
                    }
                    let (w, v) = [(W::Normal, 760), (W::Fast, 924), (W::Ultra, 994)][n as usize];
                    self.stage = S::SpeedSelected(speed, n + 1);
                    break Self::selected(f, w, i32::from(speed == v));
                }
                S::Pause => {
                    self.stage = S::Expansion;
                    if self.mask & 0x800000 != 0 {
                        break Request::Buttons {
                            view: self.view,
                            selector: 1,
                        };
                    }
                }
                S::Expansion => {
                    self.stage = S::Buy;
                    if self.mask & 0x100000 != 0 {
                        self.stage = S::ExpansionFirst;
                        break self.get(self.argument, Q::Expansion);
                    }
                }
                S::ExpansionFirst => {
                    if self.number()? == 0 {
                        self.stage = S::Buy;
                        break Request::Expansion {
                            view: self.view,
                            value: 0,
                        };
                    }
                    self.stage = S::ExpansionSecond;
                    break self.get(self.argument, Q::Expansion);
                }
                S::ExpansionSecond => {
                    self.stage = S::Buy;
                    if self.number()? == 1 {
                        break Request::Expansion {
                            view: self.view,
                            value: 1,
                        };
                    }
                }
                S::Buy => {
                    self.stage = S::Live;
                    if self.mask & 0x1000000 != 0 {
                        self.stage = S::BuyResult;
                        break self.get(self.argument, Q::BuyBlocked);
                    }
                }
                S::BuyResult => self.stage = S::BuyEnabled(u8::from(self.number()? as u8 == 0), 0),
                S::BuyEnabled(v, n) => {
                    if n == 2 {
                        self.stage = S::Live;
                        continue;
                    }
                    let r = Self::enable(f, widgets, [W::Buy, W::Build][n as usize], v)?;
                    self.stage = S::BuyEnabled(v, n + 1);
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::Live => {
                    self.stage = S::Complete;
                    if self.mask & 0x8000000 != 0 {
                        self.stage = S::LiveResult;
                        break self.get(self.argument, Q::LiveBlocked);
                    }
                }
                S::LiveResult => {
                    self.stage = S::LiveEnabled(u8::from(self.number()? as u8 != 1), 0)
                }
                S::LiveEnabled(v, n) => {
                    if n == 2 {
                        self.stage = S::Labels;
                        continue;
                    }
                    let r = Self::enable(f, widgets, [W::Live, W::Camera][n as usize], v)?;
                    self.stage = S::LiveEnabled(v, n + 1);
                    if let Some(r) = r {
                        break r;
                    }
                }
                S::Labels => {
                    self.stage = S::LabelsResult;
                    break self.get(self.argument, Q::LabelsHidden);
                }
                S::LabelsResult => {
                    let slot = if self.number()? as u8 == 1 {
                        0x98
                    } else {
                        0x94
                    };
                    self.stage = S::LabelSecond(slot);
                    break Request::Visibility {
                        widget: f.widget(W::Status),
                        slot,
                    };
                }
                S::LabelSecond(slot) => {
                    self.stage = S::Complete;
                    break Request::Visibility {
                        widget: f.widget(W::Label),
                        slot,
                    };
                }
                S::Complete => return Ok(None),
            }
        };
        self.pending = Some(r);
        Ok(Some(r))
    }
}
