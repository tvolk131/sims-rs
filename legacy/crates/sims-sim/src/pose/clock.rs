//! Original layer clock prefix and fade processing from 0x597c4c. This state is
//! separate from the legacy foreground fifth-frame clock until the enclosing
//! animator update/event order is integrated.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clock {
    pub current: i32,
    pub previous: i32,
    /// Already scaled for playback speed by the native layer initializer.
    pub duration: f32,
    /// Only sign/near-zero status are used here; magnitude is not applied twice.
    pub speed: f32,
    pub phase: f32,
    pub weight: f32,
    pub event_time: i32,
    pub previous_event_time: i32,
    pub ended: bool,
    /// 0 stop, 1 repeat, 2 reverse, 3 stop and fade out.
    pub mode: i32,
    pub fading: bool,
    pub fade_start: i32,
    pub fade_duration: i32,
    pub fade_from: f32,
    pub fade_to: f32,
}
impl Clock {
    pub fn validate(&self) -> Result<(), String> {
        if [
            self.duration,
            self.speed,
            self.phase,
            self.weight,
            self.fade_from,
            self.fade_to,
        ]
        .iter()
        .any(|v| !v.is_finite())
            || self.duration < 0.
            || (self.fading && self.fade_duration <= 0)
            || !(0..=3).contains(&self.mode)
            || self.phase.abs() > 1_000_000.
        {
            return Err("invalid native layer clock".into());
        }
        Ok(())
    }
    /// Advance through the sample boundary. Event handlers may change layer
    /// state, so the caller commits previous_event_time separately afterward.
    pub fn advance(
        &mut self,
        now: i32,
        original_duration: f32,
        has_bound_frames: bool,
    ) -> Result<f32, String> {
        self.advance_with_control(now, original_duration, has_bound_frames, 0x037f)
    }
    /// The captured game's003f control rounds x87 arithmetic to24 bits;
    /// historical isolated native probes used037f extended precision.
    pub fn advance_with_control(
        &mut self,
        now: i32,
        original_duration: f32,
        has_bound_frames: bool,
        fp_control: u16,
    ) -> Result<f32, String> {
        if !matches!(fp_control, 0x003f | 0x037f) {
            return Err("unsupported native animation floating-point control".into());
        }
        self.validate()?;
        if !original_duration.is_finite() || original_duration < 0. {
            return Err("invalid original clip duration".into());
        }
        self.previous = self.current;
        self.current = now;
        let elapsed = now.wrapping_sub(self.previous) as f32;
        if !has_bound_frames {
            return Ok(elapsed);
        }
        let mut increment = if self.speed.abs() > 0.000001 {
            if self.duration == 0. {
                return Err("active native layer has zero duration".into());
            }
            elapsed / self.duration
        } else {
            0.
        };
        if self.speed < 0. {
            increment = -increment;
        }
        let next = f64::from(self.phase) + f64::from(increment);
        self.phase = next as f32;
        if elapsed.abs() > 0.000001 {
            self.ended = if self.speed >= 0. {
                next >= 1.
            } else {
                next <= 0.
            };
            if self.ended {
                match self.mode {
                    0 => {
                        self.duration = f32::from_bits(0x4cbebc20);
                        self.speed = 0.;
                        self.phase = next.clamp(0., 1.) as f32;
                    }
                    1 | 2 => {
                        if self.mode == 2 {
                            self.speed = -self.speed;
                        }
                        if self.phase < 0. || self.phase > 1. {
                            let part = (f64::from(self.phase) - next.floor()) as f32;
                            self.phase = if self.mode == 2 { 1. - part } else { part };
                        }
                    }
                    3 => {
                        self.fading = true;
                        self.speed = 0.;
                        self.fade_start = now;
                        self.fade_from = self.weight;
                        self.duration = 0.;
                        self.fade_to = 0.;
                        self.phase = next.clamp(0., 1.) as f32;
                    }
                    _ => unreachable!(),
                }
            }
        }
        if self.fading {
            let elapsed = now.wrapping_sub(self.fade_start) as f32;
            let duration = self.fade_duration as f32;
            let fraction = (f64::from(elapsed) / f64::from(self.fade_duration)).clamp(0., 1.);
            if elapsed > duration {
                self.fading = false;
                self.weight = self.fade_to;
            } else {
                self.weight = ((f64::from(self.fade_to) - f64::from(self.fade_from)) * fraction
                    + f64::from(self.fade_from)) as f32;
            }
        }
        // 597e9d..597ea3 multiplies under the game's observed003f x87
        // control (24-bit significand), then __ftol truncates. Retaining a
        // double product here makes the captured200 boundary become199.
        let event_time = if fp_control == 0x003f {
            f64::from(original_duration * self.phase).trunc()
        } else {
            (f64::from(original_duration) * f64::from(self.phase)).trunc()
        };
        if !event_time.is_finite() || event_time < i32::MIN as f64 || event_time > i32::MAX as f64 {
            return Err("native layer event time overflow".into());
        }
        self.event_time = event_time as i32;
        self.validate()?;
        Ok(elapsed)
    }
    /// Native tail, after event callbacks and optional bone samples.
    pub fn finish(&mut self) {
        self.previous_event_time = self.event_time;
    }
}
