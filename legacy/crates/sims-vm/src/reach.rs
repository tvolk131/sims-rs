//! Person primitive 0x2f (0x45c9ba). The host owns animation and placement.
use crate::{Location, ObjectId, Program, Status, ThreadId, Vm, current};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub origin: Location,
    pub caller: ObjectId,
    pub target: ObjectId,
    pub operands: [u8; 8],
}
impl Vm {
    pub fn finish_reach(
        &mut self,
        p: &Program,
        id: ThreadId,
        request: u64,
        success: bool,
    ) -> Result<(), String> {
        let t = self.threads.get(&id).ok_or("missing Reach thread")?;
        if !matches!(&t.status,Status::AwaitingReach(r) if r.id==request) {
            return Err("stale Reach response".into());
        }
        let ins = current(p, t)?.clone();
        self.complete_host_primitive(p, id, success, ins.on_true, ins.on_false)
    }
}
/// 0x59265c: standing/seated pairs in the adult/child Reach STR# table.
pub fn animation_index(height: f32, posture: i16) -> i16 {
    let index = if height < 0. {
        9
    } else if height < 0.75 {
        1
    } else if height < 2. {
        3
    } else if height < 2.666_666_7 {
        5
    } else if height < 4. {
        7
    } else {
        return -1;
    };
    index + i16::from(posture == 1)
}
/// Native continuation decision, with animation/placement results supplied.
/// Return 2 yields; 0/1 returns through the instruction's branches.
pub fn transition(
    phase: u32,
    mode: i16,
    transfer: bool,
    prepare: bool,
    start: bool,
    ended: bool,
    event_zero: bool,
    place: bool,
) -> (i32, u32) {
    match phase {
        0 => {
            if !prepare {
                (0, 0)
            } else if start {
                (2, 1)
            } else {
                (i32::from(!transfer), 0)
            }
        }
        1 => {
            if !ended && !event_zero {
                (2, 1)
            } else if !prepare {
                (i32::from(!transfer), 1)
            } else if !transfer {
                (2, 2)
            } else if mode != 0 && mode != 1 {
                (2, 1)
            } else {
                (2, if place { 2 } else { 3 })
            }
        }
        2 | 3 => {
            if ended {
                (i32::from(phase == 2), phase)
            } else {
                (2, phase)
            }
        }
        _ => (-1, phase),
    }
}
