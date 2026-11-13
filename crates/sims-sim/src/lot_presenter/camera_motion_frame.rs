use super::super::{centering, motion::frame};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Prepare(frame::Request),
    Displacement(frame::displacement::Request),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Coordinate(i32),
    Enabled(u8),
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Prepare(frame::Cursor),
    Displacement {
        cursor: frame::displacement::Cursor,
        passes: frame::displacement::Passes,
    },
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    stage: Stage,
}

impl Cursor {
    pub fn begin(state: &mut centering::State) -> Result<Self, String> {
        let mut cursor = Self {
            stage: Stage::Prepare(frame::Cursor::begin(state)),
        };
        cursor.advance(state)?;
        Ok(cursor)
    }

    fn advance(&mut self, state: &mut centering::State) -> Result<(), String> {
        match &self.stage {
            Stage::Prepare(cursor) if cursor.complete() => {
                if state.pending == [0, 0] {
                    state.motion_disabled = 0;
                    self.stage = Stage::Done;
                } else {
                    self.stage = Stage::Displacement {
                        cursor: frame::displacement::Cursor::begin(state)?,
                        passes: frame::displacement::Passes { count: 0, phase: 0 },
                    };
                }
            }
            Stage::Displacement { cursor, .. } if cursor.complete() => {
                state.motion_disabled = 0;
                self.stage = Stage::Done;
            }
            _ => (),
        }
        Ok(())
    }

    pub fn request(&self) -> Option<Request> {
        match &self.stage {
            Stage::Prepare(cursor) => cursor.request().cloned().map(Request::Prepare),
            Stage::Displacement { cursor, .. } => {
                cursor.request().cloned().map(Request::Displacement)
            }
            Stage::Done => None,
        }
    }

    pub fn complete(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    pub fn acknowledge(
        &mut self,
        state: &mut centering::State,
        request: &Request,
        reply: Reply,
    ) -> Result<(), String> {
        if self.request().as_ref() != Some(request) {
            return Err("native live motion frame acknowledgement mismatch".into());
        }
        match (&mut self.stage, request) {
            (Stage::Prepare(cursor), Request::Prepare(request)) => {
                let reply = match reply {
                    Reply::Coordinate(coordinate) => frame::Reply::Coordinate(coordinate),
                    Reply::Done => frame::Reply::Done,
                    _ => return Err("native live motion frame reply mismatch".into()),
                };
                cursor.acknowledge(state, request, reply)?;
            }
            (Stage::Displacement { cursor, passes }, Request::Displacement(request)) => {
                let reply = match reply {
                    Reply::Enabled(enabled) => frame::displacement::Reply::Enabled(enabled),
                    Reply::Done => frame::displacement::Reply::Done,
                    _ => return Err("native live displacement reply mismatch".into()),
                };
                cursor.acknowledge(state, passes, request, reply)?;
            }
            _ => return Err("native live motion frame stage mismatch".into()),
        }
        self.advance(state)
    }
}
