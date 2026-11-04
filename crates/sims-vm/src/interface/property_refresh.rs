//!4ce81c and4ce41b. Borrow actual CPState fields at every advance; children
//!remain explicit and must run before their return is acknowledged.
use super::state::State;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub undo_available: u8, //+64
    pub zoom: u32,          //+34
    pub rotation: u32,      //+38
    pub floor: i32,         //+3c
}
///Fresh owner projections, not cached in this continuation. Clock words alone
///are captured before the renderer getter, as in4ce853..4ce883.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub undo_vector_bytes: u32,
    pub clock: [i16; 2],
    pub view: u32,
    pub panel: Option<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Query {
    Zoom,
    Rotation,
    Floor,
    WallsDown,
    Cutaway,
    Roof,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Renderer,
    Clock {
        renderer: u32,
        state: u32,
        words: [i16; 2],
    },
    Funds,
    SetFunds(i32),
    Camera {
        view: u32,
        query: Query,
    },
    SelectedTool, //4cee3d, not4ce41b
    Availability, //46fb64
    Speed,
    SetSpeed(i32),
    Paused,
    SetPaused(u8),
    SetWalls {
        value: u32,
        deferred: u8,
    },
    SelectedPerson,
    Panels {
        panel: u32,
        state: u32,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Done,
    Number(u32),
}
impl Request {
    fn accepts(self, r: Reply) -> bool {
        matches!(
            (self, r),
            (
                Self::Renderer
                    | Self::Funds
                    | Self::Camera { .. }
                    | Self::Speed
                    | Self::Paused
                    | Self::SelectedPerson,
                Reply::Number(_)
            ) | (
                Self::Clock { .. }
                    | Self::SetFunds(_)
                    | Self::SelectedTool
                    | Self::Availability
                    | Self::SetSpeed(_)
                    | Self::SetPaused(_)
                    | Self::SetWalls { .. }
                    | Self::Panels { .. },
                Reply::Done
            )
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Entry,
    Renderer,
    Clock,
    AfterClock,
    Funds,
    FundsSet,
    Zoom,
    ZoomValue,
    Rotation,
    RotationValue,
    RotationTool,
    Floor,
    FloorValue,
    FloorTool,
    FloorAvailability,
    Speed,
    SpeedValue,
    Paused,
    PausedValue,
    Walls,
    DownValue,
    CutawayValue,
    RoofValue,
    WallsAvailability,
    Selected,
    SelectedValue,
    Panels,
    Returned,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    state: u32,
    panels: bool,
    phase: Phase,
    clock: Option<[i16; 2]>,
    value: Option<u32>,
    pending: Option<Request>,
}
impl Call {
    pub fn new(state: u32, panels: bool) -> Result<Self, String> {
        if state == 0 {
            return Err("property refresh CPState identity absent".into());
        }
        Ok(Self {
            state,
            panels,
            phase: Phase::Entry,
            clock: None,
            value: None,
            pending: None,
        })
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn matches(&self, state: u32) -> bool {
        self.state == state
    }
    pub fn complete(&self) -> bool {
        self.phase == Phase::Complete && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        use Phase as P;
        use Request as R;
        let valid = match (self.phase, self.pending) {
            (_, None) => true,
            (P::Renderer, Some(R::Renderer)) => self.clock.is_some(),
            (
                P::AfterClock,
                Some(R::Clock {
                    renderer,
                    state,
                    words,
                }),
            ) => renderer != 0 && state == self.state && Some(words) == self.clock,
            (P::FundsSet, Some(R::Funds))
            | (
                P::ZoomValue,
                Some(R::Camera {
                    query: Query::Zoom, ..
                }),
            )
            | (
                P::RotationValue,
                Some(R::Camera {
                    query: Query::Rotation,
                    ..
                }),
            )
            | (
                P::FloorValue,
                Some(R::Camera {
                    query: Query::Floor,
                    ..
                }),
            )
            | (P::SpeedValue, Some(R::Speed))
            | (P::PausedValue, Some(R::Paused))
            | (
                P::DownValue,
                Some(R::Camera {
                    query: Query::WallsDown,
                    ..
                }),
            )
            | (
                P::CutawayValue,
                Some(R::Camera {
                    query: Query::Cutaway,
                    ..
                }),
            )
            | (
                P::RoofValue,
                Some(R::Camera {
                    query: Query::Roof, ..
                }),
            )
            | (P::SelectedValue, Some(R::SelectedPerson)) => true,
            (P::Zoom, Some(R::SetFunds(_)))
            | (P::Floor, Some(R::SelectedTool))
            | (P::FloorAvailability, Some(R::SelectedTool))
            | (P::Speed, Some(R::Availability))
            | (P::Paused, Some(R::SetSpeed(_)))
            | (P::Walls, Some(R::SetPaused(_)))
            | (P::WallsAvailability, Some(R::SetWalls { deferred: 1, .. }))
            | (P::Selected, Some(R::Availability)) => true,
            (P::Returned, Some(R::Panels { panel, state })) => {
                self.panels && panel != 0 && state == self.state
            }
            _ => false,
        };
        let number = self.pending.is_none()
            && matches!(
                self.phase,
                P::Clock
                    | P::FundsSet
                    | P::ZoomValue
                    | P::RotationValue
                    | P::FloorValue
                    | P::SpeedValue
                    | P::PausedValue
                    | P::DownValue
                    | P::CutawayValue
                    | P::RoofValue
                    | P::SelectedValue
            );
        if self.state == 0
            || !valid
            || matches!(self.pending, Some(R::Camera { view: 0, .. }))
            || self.clock.is_some() != matches!(self.phase, P::Renderer | P::Clock | P::AfterClock)
            || self.value.is_some() != number
        {
            return Err("property refresh lost its child".into());
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, reply: Reply) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) || !request.accepts(reply) {
            return Err("property refresh reply differs".into());
        }
        self.value = match reply {
            Reply::Number(n) => Some(n),
            Reply::Done => None,
        };
        self.pending = None;
        if self.phase == Phase::Renderer {
            self.phase = Phase::Clock;
        }
        Ok(())
    }
    fn number(&mut self) -> Result<u32, String> {
        self.value
            .take()
            .ok_or("property getter reply absent".into())
    }
    pub fn advance(
        &mut self,
        state: &mut State,
        fields: &mut Fields,
        input: Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() || self.complete() {
            return Ok(self.pending);
        }
        use Phase as P;
        use Request as R;
        let request = loop {
            match self.phase {
                P::Entry => {
                    if state.buy.dirty & 0x1000000 != 0 {
                        fields.undo_available = u8::from(input.undo_vector_bytes & !3 != 0);
                    }
                    self.phase = P::Funds;
                    if state.buy.dirty & 0x2000 != 0 {
                        self.clock = Some(input.clock);
                        self.phase = P::Renderer;
                        break R::Renderer;
                    }
                }
                P::Renderer => return Err("property renderer reply absent".into()),
                P::Clock => {
                    let renderer = self.number()?;
                    if renderer == 0 {
                        return Err("property clock renderer absent".into());
                    }
                    self.phase = P::AfterClock;
                    break R::Clock {
                        renderer,
                        state: self.state,
                        words: self.clock.unwrap(),
                    };
                }
                P::AfterClock => {
                    state.mark_dirty(0x2000);
                    self.clock = None;
                    self.phase = P::Funds;
                }
                P::Funds => {
                    self.phase = P::Zoom;
                    if state.buy.dirty & 0x4000 != 0 {
                        self.phase = P::FundsSet;
                        break R::Funds;
                    }
                }
                P::FundsSet => {
                    let n = self.number()?;
                    self.phase = P::Zoom;
                    break R::SetFunds(n as i32);
                }
                P::Zoom => {
                    self.phase = P::Rotation;
                    if state.buy.dirty & 0x1000 != 0 {
                        self.phase = P::ZoomValue;
                        break R::Camera {
                            view: input.view,
                            query: Query::Zoom,
                        };
                    }
                }
                P::ZoomValue => {
                    let n = self.number()?;
                    state.mark_dirty(0x1000);
                    fields.zoom = n;
                    self.phase = P::Rotation;
                }
                P::Rotation => {
                    self.phase = P::Floor;
                    if state.buy.dirty & 0x800 != 0 {
                        self.phase = P::RotationValue;
                        break R::Camera {
                            view: input.view,
                            query: Query::Rotation,
                        };
                    }
                }
                P::RotationValue => {
                    let n = self.number()?;
                    state.mark_dirty(0x800);
                    fields.rotation = n;
                    self.phase = P::RotationTool;
                }
                P::RotationTool => {
                    self.phase = P::Floor;
                    break R::SelectedTool;
                }
                P::Floor => {
                    self.phase = P::Speed;
                    if state.buy.dirty & 0x100 != 0 {
                        self.phase = P::FloorValue;
                        break R::Camera {
                            view: input.view,
                            query: Query::Floor,
                        };
                    }
                }
                P::FloorValue => {
                    let n = self.number()?;
                    state.mark_dirty(0x100);
                    fields.floor = if n == 0 { 1 } else { n as i32 };
                    self.phase = P::FloorTool;
                }
                P::FloorTool => {
                    self.phase = P::FloorAvailability;
                    break R::SelectedTool;
                }
                P::FloorAvailability => {
                    self.phase = P::Speed;
                    break R::Availability;
                }
                P::Speed => {
                    self.phase = P::Paused;
                    if state.buy.dirty & 0x40 != 0 {
                        self.phase = P::SpeedValue;
                        break R::Speed;
                    }
                }
                P::SpeedValue => {
                    let n = self.number()?;
                    self.phase = P::Paused;
                    break R::SetSpeed(n as i32);
                }
                P::Paused => {
                    self.phase = P::Walls;
                    if state.buy.dirty & 0x800000 != 0 {
                        self.phase = P::PausedValue;
                        break R::Paused;
                    }
                }
                P::PausedValue => {
                    let n = self.number()?;
                    self.phase = P::Walls;
                    break R::SetPaused(n as u8);
                }
                P::Walls => {
                    self.phase = P::Selected;
                    if state.buy.dirty & 0x80 != 0 {
                        self.phase = P::DownValue;
                        break R::Camera {
                            view: input.view,
                            query: Query::WallsDown,
                        };
                    }
                }
                P::DownValue => {
                    if self.number()? as u8 != 0 {
                        self.phase = P::WallsAvailability;
                        break R::SetWalls {
                            value: 2,
                            deferred: 1,
                        };
                    }
                    self.phase = P::CutawayValue;
                    break R::Camera {
                        view: input.view,
                        query: Query::Cutaway,
                    };
                }
                P::CutawayValue => {
                    if self.number()? as u8 != 0 {
                        self.phase = P::WallsAvailability;
                        break R::SetWalls {
                            value: 4,
                            deferred: 1,
                        };
                    }
                    self.phase = P::RoofValue;
                    break R::Camera {
                        view: input.view,
                        query: Query::Roof,
                    };
                }
                P::RoofValue => {
                    let value = if self.number()? as u8 != 0 { 8 } else { 1 };
                    self.phase = P::WallsAvailability;
                    break R::SetWalls { value, deferred: 1 };
                }
                P::WallsAvailability => {
                    self.phase = P::Selected;
                    break R::Availability;
                }
                P::Selected => {
                    self.phase = P::Panels;
                    if state.buy.dirty & 8 != 0 {
                        self.phase = P::SelectedValue;
                        break R::SelectedPerson;
                    }
                }
                P::SelectedValue => {
                    state.selected_person_index = Some(self.number()? as i32);
                    self.phase = P::Panels;
                }
                P::Panels => {
                    self.phase = P::Returned;
                    if self.panels
                        && let Some(panel) = input.panel
                    {
                        if panel == 0 {
                            return Err("property panel identity absent".into());
                        }
                        break R::Panels {
                            panel,
                            state: self.state,
                        };
                    }
                }
                P::Returned => {
                    self.phase = P::Complete;
                    return Ok(None);
                }
                P::Complete => return Ok(None),
            }
        };
        self.pending = Some(request);
        self.validate()?;
        Ok(self.pending)
    }
}
