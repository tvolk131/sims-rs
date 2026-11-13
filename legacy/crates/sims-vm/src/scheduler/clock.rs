//! Original timer4732c0..473334, speed46d697 and elapsed caller46d9d6.
//! These are saved caller kernels. The application/registry predicates, speed
//! overrides, lot update, queued requests and application publication remain
//! explicit child calls; this module alone is not a frontend clock binding.
use super::transport::Timing;
use serde::{Deserialize, Serialize};
pub mod application;
pub mod draw;
pub mod fps;
pub mod outer;
pub mod post_clock;
pub mod profiler;
pub mod speed_override;

/// 442f9b walks the live person vector. These are object[34], person[65],
/// motive[11] via45edc5, then person[32], in that short-circuit order.
pub fn needs_normal_speed(world: &crate::World) -> Result<bool, String> {
    let people = world
        .person
        .as_ref()
        .ok_or("clock person registry absent")?;
    for id in &people.person_order {
        let object = world.objects.get(id).ok_or("clock person absent")?;
        if *object
            .data
            .get(&34)
            .ok_or("clock person visibility absent")?
            != 0
        {
            continue;
        }
        let person = object.person.as_ref().ok_or("clock person fields absent")?;
        if *person.get(&65).ok_or("clock person65 absent")? > 1 {
            continue;
        }
        // FCOMP/SAHF/JAE also rejects unordered (NaN), unlike `mood < 0`.
        if !(f32::from_bits(object.motives.as_ref().ok_or("clock motives absent")?[11]) >= 0.0) {
            continue;
        }
        if *person.get(&32).ok_or("clock person32 absent")? == 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Four native timer words. Timer comparisons are unsigned, unlike the lot's
/// separate elapsed/period comparison in46d9d6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timer {
    pub elapsed: u32,
    pub interval: u32,
    pub last: u32,
    pub limit: u32,
}
impl Default for Timer {
    fn default() -> Self {
        Self {
            elapsed: 0,
            interval: 100,
            last: 0,
            limit: 0,
        }
    }
}
impl Timer {
    pub fn set_interval(&mut self, interval: u32) {
        self.interval = interval;
    }
    pub fn set_limit(&mut self, limit: i32) {
        self.limit = limit.max(0) as u32;
    }
    pub fn update(&mut self, now: u32) {
        self.elapsed = if self.last == 0 {
            self.interval
        } else {
            self.elapsed.wrapping_add(now.wrapping_sub(self.last))
        };
        self.last = now;
        if self.limit != 0 {
            self.elapsed = self.elapsed.min(self.interval.wrapping_mul(self.limit));
        }
    }
    pub fn consume(&mut self) -> bool {
        if self.elapsed < self.interval {
            return false;
        }
        self.elapsed = self.elapsed.wrapping_sub(self.interval);
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub timing: Timing,
    pub period: i32,
    pub timer: Timer,
    /// Lot+d8,+dc; retained without inferring meanings from their offsets.
    pub budget_d8: i32,
    pub budget_dc: i32,
    /// Simulator+d4: poll pending application messages after each lot update.
    /// Keep the historical wire name; pre-gameplay bytes misidentified this
    /// branch as a trap. Playable-game46da8c is TEST EAX/EAX; JNZ46dad0.
    #[serde(rename = "trap")]
    pub poll_messages: u8,
    pub publish: u8,
}

/// Speed is clamped before conversion. Every operation is exact in f64 over
/// this integer domain (the original constant is f32, denominator2^33), so this
/// retains the nearest53/64-bit x87 result used by the original offline oracle.
/// This is not a24-bit caller binding: actual instructions at speed500 with
/// FCW003f return87, while this helper returns86. A live source needs its actual
/// arithmetic-control owner before using this expression for a speed change.
pub fn period(speed: i32) -> i32 {
    let value = f64::from(speed.clamp(0, 1000));
    (172.0 - value * f64::from(f32::from_bits(0x3a83126f)) * 171.0 + 0.5) as i32
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum SpeedStage {
    Entry,
    AfterNotice,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speed {
    argument: i32,
    stage: SpeedStage,
    pending: bool,
}
impl Speed {
    pub fn new(argument: i32) -> Self {
        Self {
            argument,
            stage: SpeedStage::Entry,
            pending: false,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == SpeedStage::Complete && !self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.stage != SpeedStage::Entry && !(0..=1000).contains(&self.argument) {
            return Err("speed continuation has an unclamped argument".into());
        }
        if self.pending && self.stage != SpeedStage::AfterNotice {
            return Err("speed notification lost its caller".into());
        }
        Ok(())
    }
    /// True requests the existing synchronous C2,0 broadcast. Resume only after
    /// it returns; callback writes to the saved-speed word affect the final test.
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        speed: &mut i16,
        saved_speed: &mut i16,
    ) -> Result<bool, String> {
        self.validate()?;
        if self.pending {
            return Ok(true);
        }
        match self.stage {
            SpeedStage::Entry => {
                self.argument = self.argument.clamp(0, 1000);
                *speed = self.argument as i16;
                fields.period = period(self.argument);
                fields.timing = Timing::default();
                fields.timer.set_interval(fields.period as u32);
                fields.budget_d8 = (fields.period * 60 / 42).clamp(1, 60);
                //46d76c's consecutive upper/lower comparisons both use90.
                // Retain the observed instructions, not a guessed rate curve.
                fields.budget_dc = 90;
                self.stage = SpeedStage::AfterNotice;
                self.pending = true;
            }
            SpeedStage::AfterNotice => {
                if *saved_speed >= 0 {
                    *saved_speed = self.argument as i16;
                }
                self.stage = SpeedStage::Complete;
            }
            SpeedStage::Complete => (),
        }
        Ok(self.pending)
    }
    pub fn acknowledge(&mut self) -> Result<(), String> {
        self.validate()?;
        if !self.pending {
            return Err("speed has no notification".into());
        }
        self.pending = false;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    /// Global006e54e0; callbacks may change it before the next timer poll.
    pub now: u32,
    pub stopping: u8,
    /// Lot+26 / global11, cleared only after a completed update and stop check.
    pub step_word: i16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ApplicationState,
    RegistryPredicate,
    Boost,
    RestoreSpeed,
    UpdateLot,
    ProcessRequests,
    Publish,
    /// PeekMessageA(MSG*, NULL, 0, 0, PM_NOREMOVE). A nonzero BOOL returns
    /// from the elapsed caller before stop/step/request processing.
    PeekMessage,
    /// Archival pre-gameplay protected-byte boundary, rejected on restore.
    DebugBreak,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Entry,
    Registry,
    Accumulate,
    Poll,
    AfterUpdate,
    AfterMessage,
    AfterRequests,
    AfterPublish,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    stage: Stage,
    pending: Option<Request>,
    trapped: bool,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            stage: Stage::Entry,
            pending: None,
            trapped: false,
        }
    }
}
impl Frame {
    /// Resume46d9d6 at46da75, after its46d345 child has completed. The caller
    /// must establish this native boundary; entry/accumulation must not replay.
    pub fn after_completed_update() -> Self {
        Self {
            stage: Stage::AfterUpdate,
            pending: None,
            trapped: false,
        }
    }
    pub fn at_completed_update(&self) -> bool {
        self.stage == Stage::AfterUpdate && self.pending.is_none() && !self.trapped
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn trapped(&self) -> bool {
        self.trapped
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.trapped || self.pending == Some(Request::DebugBreak) {
            return Err("obsolete protected clock boundary requires an earlier checkpoint".into());
        }
        if let Some(request) = self.pending {
            let expected = match request {
                Request::ApplicationState => Stage::Entry,
                Request::RegistryPredicate => Stage::Registry,
                Request::Boost | Request::RestoreSpeed => Stage::Accumulate,
                Request::UpdateLot => Stage::AfterUpdate,
                Request::PeekMessage => Stage::AfterMessage,
                Request::ProcessRequests => Stage::AfterRequests,
                Request::Publish => Stage::AfterPublish,
                Request::DebugBreak => Stage::Complete,
            };
            if self.stage != expected {
                return Err("clock child lost its caller".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request, returned: u32) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("clock reply differs".into());
        }
        self.pending = None;
        match request {
            Request::ApplicationState => {
                self.stage = if returned as u8 == 1 {
                    Stage::Registry
                } else {
                    Stage::Accumulate
                }
            }
            Request::RegistryPredicate => {
                self.stage = Stage::Accumulate;
                self.pending = Some(if returned as u8 == 0 {
                    Request::Boost
                } else {
                    Request::RestoreSpeed
                });
            }
            Request::PeekMessage if returned != 0 => self.stage = Stage::Complete,
            _ => (),
        }
        Ok(())
    }
    pub fn advance(
        &mut self,
        fields: &mut Fields,
        inputs: &mut Inputs,
    ) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                Stage::Entry => self.pending = Some(Request::ApplicationState),
                Stage::Registry => self.pending = Some(Request::RegistryPredicate),
                Stage::Accumulate => {
                    fields.timing.elapsed = if fields.timing.last == 0 {
                        fields.period as u32
                    } else {
                        fields
                            .timing
                            .elapsed
                            .wrapping_add(inputs.now.wrapping_sub(fields.timing.last))
                    };
                    fields.timing.last = inputs.now;
                    self.stage = Stage::Poll;
                }
                Stage::Poll => {
                    fields.timer.update(inputs.now);
                    if (fields.timing.elapsed as i32) < fields.period || !fields.timer.consume() {
                        self.stage = Stage::Complete;
                    } else {
                        fields.timing.elapsed =
                            fields.timing.elapsed.wrapping_sub(fields.period as u32);
                        self.stage = Stage::AfterUpdate;
                        self.pending = Some(Request::UpdateLot);
                    }
                }
                Stage::AfterUpdate => {
                    self.stage = Stage::AfterMessage;
                    if fields.poll_messages != 0 {
                        self.pending = Some(Request::PeekMessage);
                    }
                }
                Stage::AfterMessage => {
                    if inputs.stopping != 0 {
                        self.stage = Stage::Complete;
                    } else if inputs.step_word != 0 {
                        inputs.step_word = 0;
                        self.stage = Stage::Complete;
                    } else {
                        self.stage = Stage::AfterRequests;
                        self.pending = Some(Request::ProcessRequests);
                    }
                }
                Stage::AfterRequests => {
                    if fields.publish == 1 {
                        self.stage = Stage::AfterPublish;
                        self.pending = Some(Request::Publish);
                    } else {
                        self.stage = Stage::Poll;
                    }
                }
                Stage::AfterPublish => {
                    // Even a callback's new publication request is cleared here.
                    fields.publish = 0;
                    self.stage = Stage::Poll;
                }
                Stage::Complete => return Ok(None),
            }
            if self.pending.is_some() {
                return Ok(self.pending);
            }
        }
    }
}

#[cfg(test)]
mod completed_boundary_tests {
    use super::*;
    #[test]
    fn retained_completed_child_consumes_next_period_without_reaccumulating() {
        let mut frame = Frame::after_completed_update();
        let bytes = serde_json::to_vec(&frame).unwrap();
        frame = serde_json::from_slice(&bytes).unwrap();
        let mut fields = Fields {
            timing: Timing {
                last: 343485,
                elapsed: 93,
            },
            period: 42,
            timer: Timer {
                elapsed: 1218,
                interval: 42,
                last: 343485,
                limit: 30,
            },
            budget_d8: 60,
            budget_dc: 90,
            poll_messages: 0,
            publish: 0,
        };
        let mut input = Inputs {
            now: 343485,
            stopping: 0,
            step_word: 0,
        };
        assert_eq!(
            frame.advance(&mut fields, &mut input).unwrap(),
            Some(Request::ProcessRequests)
        );
        assert_eq!(fields.timing.elapsed, 93);
        frame.acknowledge(Request::ProcessRequests, 0).unwrap();
        assert_eq!(
            frame.advance(&mut fields, &mut input).unwrap(),
            Some(Request::UpdateLot)
        );
        assert_eq!(
            fields.timing,
            Timing {
                last: 343485,
                elapsed: 51
            }
        );
        assert_eq!(
            fields.timer,
            Timer {
                elapsed: 1176,
                interval: 42,
                last: 343485,
                limit: 30
            }
        );
        assert_eq!(input.step_word, 0);
    }
}
