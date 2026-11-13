//! Native 589c85 / 589e2d route start and stop over the shared animator state.
use super::{Movement, Placement, Request, ordinary, path};
use serde::{Deserialize, Serialize};

/// 45ad63 states 9/10/11 retain their chosen outcome while 589e2d yields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stop {
    Success,
    Retry,
    Failure,
    /// House refresh selects state15: stop, then re-enter state1 without a
    /// failure result, success placement, or contention retry.
    Refresh,
}
impl Stop {
    /// 45ad63 retains this state while 589e2d has not finished stopping.
    pub fn frame_continuation(self) -> u32 {
        match self {
            Self::Success => 9,
            Self::Retry => 10,
            Self::Failure => 11,
            Self::Refresh => 15,
        }
    }
    /// State 3 checks effective interruption before path presence or polling.
    /// A missing path retains the previous result; cancellation writes 4 now.
    pub fn before_poll(interrupted: bool, has_path: bool, result: &mut i16) -> Option<Self> {
        if interrupted {
            *result = 4;
            Some(Self::Failure)
        } else if !has_path {
            Some(Self::Failure)
        } else {
            None
        }
    }
    pub fn after_poll(result: i32) -> Result<Option<Self>, String> {
        match result {
            0 => Ok(Some(Self::Retry)),
            1 => Ok(Some(Self::Success)),
            2 => Ok(None),
            _ => Err("invalid native walking poll result".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Accept,
    Move,
    Done,
}

/// 589d47 captures the original physical coordinate and level before invoking
/// acceptance. Rejection still invokes SetObject, using that captured position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    phase: Phase,
    position: [i32; 2],
    previous: [i32; 2],
    level: i32,
    result: i32,
}
impl Poll {
    pub fn begin(
        movement: &Movement,
        placement: &mut Placement,
        facing: &mut i16,
        physical: [i32; 2],
        level: i32,
    ) -> Result<Self, String> {
        let status = movement.state.status;
        let mut state = Self {
            phase: Phase::Done,
            position: physical,
            previous: physical,
            level,
            result: if matches!(status, -1 | 1) { 2 } else { 0 },
        };
        if status == 2 {
            super::heading(placement, facing, placement.base[2])?;
            let points = &movement
                .path
                .as_ref()
                .ok_or("native route result path absent")?
                .points;
            state.position = path::project(
                points,
                [
                    path::round(f64::from(placement.base[1]))?,
                    path::round(f64::from(placement.base[0]))?,
                ],
                -1,
            )?;
            state.phase = Phase::Accept;
            state.result = 1;
        }
        Ok(state)
    }
    pub fn request(&self) -> Option<Request> {
        match self.phase {
            Phase::Accept => Some(Request::AcceptStep {
                position: self.position,
                level: self.level,
            }),
            Phase::Move => Some(Request::MoveStep {
                position: self.position,
                level: self.level,
            }),
            Phase::Done => None,
        }
    }
    pub fn accept(&mut self, placement: &mut Placement, accepted: bool) -> Result<(), String> {
        if self.phase != Phase::Accept {
            return Err("native route result acceptance not pending".into());
        }
        if !accepted {
            self.position = self.previous;
            placement.invalidate();
            self.result = 0;
        }
        self.phase = Phase::Move;
        Ok(())
    }
    pub fn moved(&mut self) -> Result<(), String> {
        if self.phase != Phase::Move {
            return Err("native route result move not pending".into());
        }
        self.phase = Phase::Done;
        Ok(())
    }
    pub fn result(&self) -> Option<i32> {
        (self.phase == Phase::Done).then_some(self.result)
    }
}

pub(super) fn initial_scalar_7c() -> f32 {
    0.
}

impl Movement {
    /// Position is the original physical pair (+f4, +f8), not animator order.
    /// Returns the 59209f write to object data 22. Layers and calibration are
    /// retained; route start does not clear the foreground or reset its phase.
    pub fn start_route(
        &mut self,
        placement: &mut Placement,
        control: &mut ordinary::State,
        position: [i32; 2],
        facing: i16,
    ) -> Result<i16, String> {
        if self.busy() {
            return Err("native route start during movement update".into());
        }
        let path = self
            .path
            .as_mut()
            .ok_or("native route calibration absent")?;
        if path.points.len() < 3 {
            return Err("native route lacks destination/heading points".into());
        }
        placement.base = [position[1] as f32, position[0] as f32, f32::from(facing)];
        placement.previous = placement.base;
        placement.offset = [0.; 3];
        self.scalar_7c = 1.;
        self.state.status = 1;
        path.progress = 0.;
        path.index = 0;
        path.stop_mode = 0;
        control.mode = 3;
        path.interrupt = 0;
        self.state.reason = 0;
        Ok(256)
    }

    /// A busy walk requests interruption and yields. Only the immediate branch
    /// changes the mode/target and emits the native route diagnostic 7.
    pub fn stop_route(
        &mut self,
        placement: &mut Placement,
        control: &mut ordinary::State,
    ) -> Result<bool, String> {
        if self.busy() {
            return Err("native route stop during movement update".into());
        }
        let path = self
            .path
            .as_mut()
            .ok_or("native route calibration absent")?;
        if matches!(self.state.status, -1 | 1) {
            path.interrupt = 1;
            return Ok(false);
        }
        self.state.status = 0;
        path.index = -1;
        path.deadline = 0;
        placement.previous = placement.base;
        path.stop_mode = 1;
        control.mode = 3;
        Ok(true)
    }
}
