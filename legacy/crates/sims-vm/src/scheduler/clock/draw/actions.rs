//! Original4d28df action-window reconciliation. UI callees remain explicit
//! children; the caller's fields/cursor survive a binary save at every child.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod image;
pub mod tail;
pub mod clear;
pub mod collect;
pub mod record;

pub type Record = [u8; 28];
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub initialized: u8,
    pub windows: Vec<u32>,
    pub records: Vec<Record>,
    pub scratch: Vec<u32>,
    /// Complete0x124-byte native action-window owners, including inactive ones.
    pub headers: BTreeMap<u32, Vec<u8>>,
    pub hint_pending: u8,
    pub hint_token: u32,
    pub hint_point: [u32; 2],
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if self.windows.len() > 4096
            || self.records.len() > 4096
            || self.scratch.len() > 4096
            || self.headers.values().any(|h| h.len() != 0x124)
            || self
                .windows
                .iter()
                .chain(&self.scratch)
                .any(|w| *w == 0 || !self.headers.contains_key(w))
        {
            return Err("action-window owner/vector extent differs".into());
        }
        Ok(())
    }
    fn header(&self, window: u32) -> Result<&[u8], String> {
        self.headers
            .get(&window)
            .map(Vec::as_slice)
            .ok_or_else(|| "action-window owner absent".into())
    }
    fn flag(&self, window: u32, offset: usize) -> Result<u8, String> {
        Ok(self.header(window)?[offset])
    }
    fn word(&self, window: u32, offset: usize) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.header(window)?[offset..offset + 4].try_into().unwrap(),
        ))
    }
    fn set_flag(&mut self, window: u32, offset: usize, value: u8) -> Result<(), String> {
        self.headers
            .get_mut(&window)
            .ok_or("action-window owner absent")?[offset] = value;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Vector {
    Records,
    Scratch,
    Windows,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Mode,
    Parent {
        window: u32,
    },
    Show {
        window: u32,
        visible: bool,
    },
    Attach {
        window: u32,
        attach: bool,
    },
    Initialize {
        vector: Vector,
    },
    ClearRecords,
    SelectedPerson,
    Collect {
        selected: u32,
    },
    Token {
        record: Record,
    },
    Create {
        record: Record,
    },
    Image {
        window: u32,
        image: u32,
    },
    CaptionDirty {
        record: Record,
    },
    Caption {
        window: u32,
        record: Record,
    },
    Move {
        window: u32,
        point: [u32; 2],
    },
    Tween {
        window: u32,
        point: [u32; 2],
        duration: u32,
    },
    Push {
        vector: Vector,
        window: u32,
    },
    Remove {
        window: u32,
    },
    Advance {
        window: u32,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Done,
    Value(u32),
    Records(Vec<Record>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Parents {
        index: i32,
        live: bool,
    },
    Show {
        index: i32,
        live: bool,
        window: u32,
    },
    Attach {
        index: i32,
        live: bool,
        window: u32,
    },
    InitRecords,
    ClearRecords,
    Selected,
    Collect {
        selected: u32,
    },
    ClearMarks,
    InitScratch,
    ClearScratch,
    Item {
        index: usize,
    },
    Match {
        index: usize,
        token: u32,
    },
    Create {
        index: usize,
    },
    Hint {
        index: usize,
        window: u32,
    },
    HintToken {
        index: usize,
        window: u32,
    },
    HintMove {
        index: usize,
        window: u32,
    },
    Image {
        index: usize,
        window: u32,
    },
    Dirty {
        index: usize,
        window: u32,
    },
    Caption {
        index: usize,
        window: u32,
    },
    Scratch {
        index: usize,
        window: u32,
    },
    RemoveScan,
    Remove {
        window: u32,
    },
    ClearWindows,
    Layout {
        index: usize,
        x: u32,
    },
    Move {
        index: usize,
        x: u32,
        window: u32,
        tween: bool,
    },
    Append {
        index: usize,
        x: u32,
        window: u32,
    },
    Advance {
        index: usize,
    },
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    phase: Phase,
    pending: Option<Request>,
}
impl Frame {
    pub fn at_entry() -> Self {
        Self {
            phase: Phase::Entry,
            pending: None,
        }
    }
    pub fn is_at_entry(&self) -> bool {
        self.phase == Phase::Entry
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete
    }
    pub fn pending(&self) -> Option<&Request> {
        self.pending.as_ref()
    }
    fn record(state: &State, index: usize) -> Result<Record, String> {
        state
            .records
            .get(index)
            .copied()
            .ok_or_else(|| "action-image iterator lost its record".into())
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(request) = &self.pending {
            let valid = matches!(
                (self.phase, request),
                (Phase::Entry, Request::Mode)
                    | (Phase::Parents { .. }, Request::Parent { .. })
                    | (Phase::Show { .. }, Request::Show { .. })
                    | (Phase::Attach { .. }, Request::Attach { .. })
                    | (
                        Phase::InitRecords | Phase::InitScratch,
                        Request::Initialize { .. }
                    )
                    | (Phase::ClearRecords, Request::ClearRecords)
                    | (Phase::Selected, Request::SelectedPerson)
                    | (Phase::Collect { .. }, Request::Collect { .. })
                    | (
                        Phase::Item { .. } | Phase::HintToken { .. },
                        Request::Token { .. }
                    )
                    | (Phase::Create { .. }, Request::Create { .. })
                    | (Phase::Image { .. }, Request::Image { .. })
                    | (Phase::Dirty { .. }, Request::CaptionDirty { .. })
                    | (Phase::Caption { .. }, Request::Caption { .. })
                    | (
                        Phase::HintMove { .. } | Phase::Move { tween: false, .. },
                        Request::Move { .. }
                    )
                    | (Phase::Move { tween: true, .. }, Request::Tween { .. })
                    | (
                        Phase::Scratch { .. } | Phase::Append { .. },
                        Request::Push { .. }
                    )
                    | (Phase::Remove { .. }, Request::Remove { .. })
                    | (Phase::Advance { .. }, Request::Advance { .. })
            );
            if !valid {
                return Err("action-window child disagrees with saved cursor".into());
            }
        }
        Ok(())
    }
    pub fn validate_state(&self, state: &State) -> Result<(), String> {
        self.validate()?;
        state.validate()?;
        if let Some(request) = &self.pending {
            self.check_child(state, request)?;
        }
        Ok(())
    }
    fn check_child(&self, state: &State, request: &Request) -> Result<(), String> {
        let same_record =
            |index, record: &Record| Self::record(state, index).is_ok_and(|r| r == *record);
        let valid = match (self.phase, request) {
            (Phase::Entry, Request::Mode)
            | (Phase::ClearRecords, Request::ClearRecords)
            | (Phase::Selected, Request::SelectedPerson) => true,
            (Phase::Parents { index, .. }, Request::Parent { window }) => {
                index >= 0 && state.windows.get(index as usize) == Some(window)
            }
            (Phase::Show { window, live, .. }, Request::Show { window: w, visible }) => {
                window == *w && live == *visible
            }
            (Phase::Attach { window, live, .. }, Request::Attach { window: w, attach }) => {
                window == *w && live == *attach
            }
            (
                Phase::InitRecords,
                Request::Initialize {
                    vector: Vector::Records,
                },
            ) => state.initialized & 1 != 0,
            (
                Phase::InitScratch,
                Request::Initialize {
                    vector: Vector::Scratch,
                },
            ) => state.initialized & 2 != 0,
            (Phase::Collect { selected }, Request::Collect { selected: s }) => selected == *s,
            (Phase::Item { index } | Phase::HintToken { index, .. }, Request::Token { record }) => {
                same_record(index, record)
            }
            (Phase::Create { index }, Request::Create { record }) => same_record(index, record),
            (Phase::Image { index, window }, Request::Image { window: w, image }) => {
                window == *w
                    && Self::record(state, index)
                        .is_ok_and(|r| u32::from_le_bytes(r[4..8].try_into().unwrap()) == *image)
            }
            (Phase::Dirty { index, .. }, Request::CaptionDirty { record }) => {
                same_record(index, record)
            }
            (Phase::Caption { index, window }, Request::Caption { window: w, record }) => {
                window == *w && same_record(index, record)
            }
            (Phase::HintMove { window, .. }, Request::Move { window: w, point }) => {
                window == *w && *point == state.hint_point.map(|v| v.wrapping_sub(16))
            }
            (
                Phase::Move {
                    window,
                    x,
                    tween: false,
                    ..
                },
                Request::Move { window: w, point },
            ) => window == *w && *point == [x, 32],
            (
                Phase::Move {
                    window,
                    x,
                    tween: true,
                    ..
                },
                Request::Tween {
                    window: w,
                    point,
                    duration,
                },
            ) => window == *w && *point == [x, 32] && *duration == 500,
            (
                Phase::Scratch { window, .. },
                Request::Push {
                    vector: Vector::Scratch,
                    window: w,
                },
            )
            | (
                Phase::Append { window, .. },
                Request::Push {
                    vector: Vector::Windows,
                    window: w,
                },
            ) => window == *w,
            (Phase::Remove { window }, Request::Remove { window: w }) => window == *w,
            (Phase::Advance { index }, Request::Advance { window }) => {
                state.windows.get(index) == Some(window)
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err("action-window child arguments disagree with caller owners".into())
        }
    }
    pub fn advance(&mut self, state: &mut State) -> Result<Option<Request>, String> {
        self.validate()?;
        state.validate()?;
        if let Some(request) = &self.pending {
            self.check_child(state, request)?;
            return Ok(Some(request.clone()));
        }
        for _ in 0..20000 {
            let request = match self.phase {
                Phase::Entry => Request::Mode,
                Phase::Parents { index, live } => {
                    if index < 0 {
                        self.phase = if live {
                            Phase::InitRecords
                        } else {
                            Phase::Complete
                        };
                        continue;
                    }
                    let window = *state
                        .windows
                        .get(index as usize)
                        .ok_or("action-window parent iterator invalid")?;
                    Request::Parent { window }
                }
                Phase::Show { window, live, .. } => Request::Show {
                    window,
                    visible: live,
                },
                Phase::Attach { window, live, .. } => Request::Attach {
                    window,
                    attach: live,
                },
                Phase::InitRecords => {
                    if state.initialized & 1 != 0 {
                        self.phase = Phase::ClearRecords;
                        continue;
                    }
                    state.initialized |= 1;
                    state.records.clear();
                    Request::Initialize {
                        vector: Vector::Records,
                    }
                }
                Phase::ClearRecords => Request::ClearRecords,
                Phase::Selected => Request::SelectedPerson,
                Phase::Collect { selected } => Request::Collect { selected },
                Phase::ClearMarks => {
                    for i in 0..state.windows.len() {
                        state.set_flag(state.windows[i], 0x104, 0)?;
                    }
                    self.phase = Phase::InitScratch;
                    continue;
                }
                Phase::InitScratch => {
                    if state.initialized & 2 != 0 {
                        self.phase = Phase::ClearScratch;
                        continue;
                    }
                    state.initialized |= 2;
                    state.scratch.clear();
                    Request::Initialize {
                        vector: Vector::Scratch,
                    }
                }
                Phase::ClearScratch => {
                    state.scratch.clear();
                    self.phase = Phase::Item { index: 0 };
                    continue;
                }
                Phase::Item { index } => {
                    if index >= state.records.len() {
                        self.phase = Phase::RemoveScan;
                        continue;
                    }
                    Request::Token {
                        record: Self::record(state, index)?,
                    }
                }
                Phase::Match { index, token } => {
                    let mut found = None;
                    for &window in &state.windows {
                        if state.word(window, 0x108)? == token {
                            found = Some(window);
                            break;
                        }
                    }
                    self.phase = if let Some(window) = found {
                        state.set_flag(window, 0x104, 1)?;
                        Phase::Image { index, window }
                    } else {
                        Phase::Create { index }
                    };
                    continue;
                }
                Phase::Create { index } => Request::Create {
                    record: Self::record(state, index)?,
                },
                Phase::Hint { index, window } => {
                    if state.hint_pending != 0 {
                        self.phase = Phase::HintToken { index, window };
                    } else {
                        state.set_flag(window, 0x105, 0)?;
                        state.set_flag(window, 0x104, 1)?;
                        self.phase = Phase::Scratch { index, window };
                    }
                    continue;
                }
                Phase::HintToken { index, .. } => Request::Token {
                    record: Self::record(state, index)?,
                },
                Phase::HintMove { window, .. } => Request::Move {
                    window,
                    point: state.hint_point.map(|v| v.wrapping_sub(16)),
                },
                Phase::Image { index, window } => {
                    let r = Self::record(state, index)?;
                    Request::Image {
                        window,
                        image: u32::from_le_bytes(r[4..8].try_into().unwrap()),
                    }
                }
                Phase::Dirty { index, .. } => Request::CaptionDirty {
                    record: Self::record(state, index)?,
                },
                Phase::Caption { index, window } => Request::Caption {
                    window,
                    record: Self::record(state, index)?,
                },
                Phase::Scratch { window, .. } => Request::Push {
                    vector: Vector::Scratch,
                    window,
                },
                Phase::RemoveScan => {
                    let mut unused = None;
                    for &window in &state.windows {
                        if state.flag(window, 0x104)? == 0 {
                            unused = Some(window);
                            break;
                        }
                    }
                    self.phase =
                        unused.map_or(Phase::ClearWindows, |window| Phase::Remove { window });
                    continue;
                }
                Phase::Remove { window } => Request::Remove { window },
                Phase::ClearWindows => {
                    state.windows.clear();
                    self.phase = Phase::Layout { index: 0, x: 32 };
                    continue;
                }
                Phase::Layout { index, x } => {
                    let Some(&window) = state.scratch.get(index) else {
                        self.phase = Phase::Advance { index: 0 };
                        continue;
                    };
                    if state.flag(window, 0x105)? == 0 {
                        self.phase = Phase::Move {
                            index,
                            x,
                            window,
                            tween: false,
                        };
                    } else {
                        let at = if state.flag(window, 0xf0)? != 0 {
                            0xfc
                        } else {
                            0x7c
                        };
                        self.phase =
                            if [state.word(window, at)?, state.word(window, at + 4)?] == [x, 32] {
                                Phase::Append { index, x, window }
                            } else {
                                Phase::Move {
                                    index,
                                    x,
                                    window,
                                    tween: true,
                                }
                            };
                    }
                    continue;
                }
                Phase::Move {
                    window, x, tween, ..
                } => {
                    if tween {
                        Request::Tween {
                            window,
                            point: [x, 32],
                            duration: 500,
                        }
                    } else {
                        Request::Move {
                            window,
                            point: [x, 32],
                        }
                    }
                }
                Phase::Append { window, .. } => Request::Push {
                    vector: Vector::Windows,
                    window,
                },
                Phase::Advance { index } => {
                    let Some(&window) = state.windows.get(index) else {
                        self.phase = Phase::Complete;
                        continue;
                    };
                    Request::Advance { window }
                }
                Phase::Complete => return Ok(None),
            };
            self.pending = Some(request.clone());
            return Ok(Some(request));
        }
        Err("action-window caller exceeded its bounded owner profile".into())
    }
    /// The child executor owns the effects of Show/Attach/Create/Image/Caption,
    /// Move/Tween/Push/Remove/Advance. Done is never a substitute for executing
    /// those callees. Collection returns its rebuilt semantic28-byte records.
    pub fn acknowledge(
        &mut self,
        state: &mut State,
        request: Request,
        reply: Reply,
    ) -> Result<(), String> {
        self.validate()?;
        state.validate()?;
        if self.pending.as_ref() != Some(&request) {
            return Err("action-window reply is not its pending child".into());
        }
        self.check_child(state, &request)?;
        let value = if let Reply::Value(v) = reply {
            Some(v)
        } else {
            None
        };
        let expects_value = matches!(
            request,
            Request::Mode
                | Request::Parent { .. }
                | Request::SelectedPerson
                | Request::Token { .. }
                | Request::Create { .. }
                | Request::CaptionDirty { .. }
        );
        if (expects_value && value.is_none())
            || (!expects_value
                && !matches!(request, Request::Collect { .. })
                && !matches!(reply, Reply::Done))
            || (matches!(request, Request::Collect { .. }) && !matches!(reply, Reply::Records(_)))
        {
            return Err("action-window child reply type differs".into());
        }
        self.phase = match self.phase {
            Phase::Entry => Phase::Parents {
                index: state.windows.len() as i32 - 1,
                live: value == Some(2),
            },
            Phase::Parents { index, live } => {
                let Request::Parent { window } = request else {
                    unreachable!()
                };
                if (value != Some(0)) == live {
                    Phase::Parents {
                        index: index - 1,
                        live,
                    }
                } else {
                    Phase::Show {
                        index,
                        live,
                        window,
                    }
                }
            }
            Phase::Show {
                index,
                live,
                window,
            } => Phase::Attach {
                index,
                live,
                window,
            },
            Phase::Attach { index, live, .. } => Phase::Parents {
                index: index - 1,
                live,
            },
            Phase::InitRecords => Phase::ClearRecords,
            Phase::ClearRecords => {
                state.records.clear();
                Phase::Selected
            }
            Phase::Selected => Phase::Collect {
                selected: value.unwrap(),
            },
            Phase::Collect { .. } => {
                let Reply::Records(records) = reply else {
                    unreachable!()
                };
                state.records = records;
                Phase::ClearMarks
            }
            Phase::InitScratch => Phase::ClearScratch,
            Phase::Item { index } => Phase::Match {
                index,
                token: value.unwrap(),
            },
            Phase::Create { index } => {
                let window = value.unwrap();
                state.header(window)?;
                Phase::Hint { index, window }
            }
            Phase::HintToken { index, window } => {
                if value == Some(state.hint_token) {
                    state.hint_pending = 0;
                    Phase::HintMove { index, window }
                } else {
                    state.set_flag(window, 0x105, 0)?;
                    state.set_flag(window, 0x104, 1)?;
                    Phase::Scratch { index, window }
                }
            }
            Phase::HintMove { index, window } => {
                state.set_flag(window, 0x105, 1)?;
                state.set_flag(window, 0x104, 1)?;
                Phase::Scratch { index, window }
            }
            Phase::Image { index, window } => {
                state.set_flag(window, 0x120, Self::record(state, index)?[8])?;
                Phase::Dirty { index, window }
            }
            Phase::Dirty { index, window } => {
                if value.unwrap() & 255 == 0 {
                    Phase::Scratch { index, window }
                } else {
                    Phase::Caption { index, window }
                }
            }
            Phase::Caption { index, window } => Phase::Scratch { index, window },
            Phase::Scratch { index, .. } => Phase::Item { index: index + 1 },
            Phase::Remove { .. } => Phase::RemoveScan,
            Phase::Move {
                index, x, window, ..
            } => Phase::Append { index, x, window },
            Phase::Append { index, x, .. } => Phase::Layout {
                index: index + 1,
                x: x.wrapping_add(50),
            },
            Phase::Advance { index } => Phase::Advance { index: index + 1 },
            _ => return Err("action-window caller replied outside a child".into()),
        };
        self.pending = None;
        self.validate()?;
        state.validate()
    }
}
