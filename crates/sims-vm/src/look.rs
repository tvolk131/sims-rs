//! Original Look Towards coordinate/continuation contract, excluding animator
//! progression. Native coordinates are sixteenths of a tile.
use crate::{Program, Status, Thread, TurnRequest, Vm};
use serde::{Deserialize, Serialize};
impl Vm {
    pub(crate) fn request_turn(
        &mut self,
        program: &Program,
        t: &mut Thread,
        ins: &sims_formats::behavior::Instruction,
    ) -> Result<String, String> {
        let mode = i16::from_le_bytes(ins.operands[..2].try_into().unwrap());
        if !(1..=3).contains(&mode) {
            return Err("invalid Look Towards mode (native error 0x27)".into());
        }
        if t.check {
            return Err("turning in a synchronous eligibility check is unsupported".into());
        }
        let f = t.stack.last_mut().ok_or("turn frame missing")?;
        let person = self.world.objects[&f.caller]
            .person
            .as_ref()
            .ok_or("turn caller is not a person")?;
        if f.continuation == 0 && *person.get(&0).ok_or("turn posture is missing")? != 0 {
            t.last_result = false;
            self.branch(program, t, ins.on_false)?;
            return Ok("Look Towards declined non-standing posture; branch false".into());
        }
        if f.continuation == 0 && mode != 1 && !self.world.objects.contains_key(&f.stack_object) {
            return Err("Look Towards target missing (native error 0x15)".into());
        }
        let id = self.next_request;
        self.next_request = id.checked_add(1).ok_or("request ID exhausted")?;
        t.status = Status::AwaitingTurn(TurnRequest {
            id,
            origin: f.location.clone(),
            caller: f.caller,
            target: f.stack_object,
            mode,
        });
        f.continuation = 1;
        Ok(format!("await native standing turn, request {id}"))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Preparation {
    Declined,
    Continue,
    Start {
        path: [[i32; 2]; 3],
        clear_data9: bool,
    },
}
pub fn prepare(
    mode: i16,
    continuation: u32,
    posture: i16,
    actor: [i32; 2],
    target: Option<[i32; 2]>,
    camera: u8,
) -> Result<Preparation, String> {
    if !(1..=3).contains(&mode) {
        return Err("invalid Look Towards mode (native error 0x27)".into());
    }
    if continuation != 0 {
        return Ok(Preparation::Continue);
    }
    if posture != 0 {
        return Ok(Preparation::Declined);
    }
    let destination = match mode {
        1 => {
            if camera > 3 {
                return Err("invalid native camera rotation".into());
            }
            let [x, y] = [[16, 16], [-16, 16], [-16, -16], [16, -16]][usize::from(camera)];
            [actor[0].wrapping_add(x), actor[1].wrapping_add(y)]
        }
        2 => target.ok_or("Look Towards target missing (native error 0x15)")?,
        3 => {
            let target = target.ok_or("Look Towards target missing (native error 0x15)")?;
            std::array::from_fn(|i| {
                target[i].wrapping_add(target[i].wrapping_sub(actor[i]).wrapping_mul(-2))
            })
        }
        _ => unreachable!(),
    };
    Ok(Preparation::Start {
        path: [actor, actor, destination],
        clear_data9: mode == 1,
    })
}
/// Return codes from 0x45a98e after animator status/finish boundaries. Status
/// zero is promoted to success, but pending finish forces another yielded tick.
pub fn complete(status: i32, finish: bool) -> i32 {
    let status = if status == 0 { 1 } else { status };
    if status != 2 && !finish { 2 } else { status }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StandingTurn {
    /// Posture index, resolved by 59db2f through the person's private/global
    /// animation profile (including child and special profiles).
    pub clip: u16,
    pub nominal_turn: f32,
    /// The clip contributes nominal root rotation; animator heading supplies
    /// the remainder. The renderer must not apply the clip's turn twice.
    pub base_heading: f32,
}
/// 0x58f4ee..0x58f688. The caller supplies its already-normalized heading delta,
/// in eighth-turns. A 135-degree turn selects the 90-degree clip plus correction.
pub fn standing_turn(heading: f32, delta: f32) -> Result<StandingTurn, String> {
    if !heading.is_finite() || !delta.is_finite() || !(-4.0..=4.0).contains(&delta) {
        return Err("invalid standing turn angle".into());
    }
    let (clip, nominal_turn) = if delta < -3. {
        (5, -4.)
    } else if delta > 3. {
        (4, 4.)
    } else if delta < -1.5 {
        (7, -2.)
    } else if delta > 1.5 {
        (6, 2.)
    } else if delta < -0.5 {
        (9, -1.)
    } else if delta > 0.5 {
        (8, 1.)
    } else {
        (10, 0.)
    };
    Ok(StandingTurn {
        clip,
        nominal_turn,
        base_heading: (f64::from(heading) + f64::from(delta) - f64::from(nominal_turn)) as f32,
    })
}

/// Native animator virtual clock (0x585483/0x585502). The alternate flag
/// bypasses the 1.2 scale before truncation. The clock retains the low word of the CRT i64 result.
pub fn animation_time(tick: i32, alternate: bool) -> i32 {
    let time = i64::from(tick) * if alternate { 42 } else { 50 };
    time as i32
}
/// Default forward, non-looping clip clock, bounded before skeleton blending.
/// The division spills to f32 before the native addition and completion test.
pub fn advance_progress(progress: f32, elapsed_ms: i32, duration_ms: f32) -> (f32, bool) {
    let increment = elapsed_ms as f32 / duration_ms;
    let next = f64::from(progress) + f64::from(increment);
    let ended = elapsed_ms != 0 && next >= 1.;
    (if ended { 1. } else { next as f32 }, ended)
}
/// Native final path segment direction, expressed in clockwise eighth-turns.
/// Its pi constant is stored as f32; replacing it with f64 PI changes north.
pub fn target_heading(native_delta: [i32; 2]) -> f32 {
    let pi = f64::from(std::f32::consts::PI);
    let mut angle = f64::from(native_delta[0] as f32).atan2(f64::from(native_delta[1] as f32));
    if angle < 0. {
        angle += 2. * pi;
    }
    ((angle * 8. / (2. * pi) + 2.).rem_euclid(8.)) as f32
}
pub fn heading_delta(from: f32, target: f32) -> f32 {
    (target - from + 4.).rem_euclid(8.) - 4.
}
/// The root pose contributes the nominal turn. This is only the animator's
/// heading correction, before advancing the clip for the current tick.
pub fn pose_heading(from: f32, base: f32, target: f32, progress: f32) -> f32 {
    if progress >= 1. {
        target
    } else {
        ((1. - f64::from(progress)) * f64::from(from) + f64::from(progress) * f64::from(base))
            as f32
    }
}
pub fn quantized_heading(heading: f32) -> u8 {
    ((f64::from(heading) + 0.5).trunc() as i32).rem_euclid(8) as u8
}
