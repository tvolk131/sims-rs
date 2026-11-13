//! Lot calendar advancement from Sims 1.91 0x0046d345.
use serde::{Deserialize, Serialize};

pub fn host_ticks_from_milliseconds(milliseconds: u32) -> u32 {
    milliseconds.wrapping_mul(60) / 1000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LotClock {
    pub hour: i16,
    pub day: i16,
    pub period: i16,
    pub minute: i16,
    pub second: i16,
    pub month: i16,
    pub year: i16,
    pub elapsed_days: i16,
    pub month_days: i16,
    pub ticks: u32,
    /// Native six adjacent eight-word daily-statistic blocks, in memory order.
    pub statistics: [[i32; 8]; 6],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockEvent {
    DailyObjects,
    DailyNeighborhood,
    WorldUpdate { partial: bool },
    Lighting,
    Signal { kind: u16, argument: i32 },
}

impl LotClock {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..24).contains(&self.hour)
            || !(0..60).contains(&self.minute)
            || !(0..60).contains(&self.second)
            || !(1..=30).contains(&self.day)
            || !(1..=12).contains(&self.month)
            || self.month_days != 30
            || !(0..4).contains(&self.period)
        {
            return Err("invalid native lot calendar".into());
        }
        Ok(())
    }

    pub fn period_for_hour(hour: i16) -> i16 {
        match hour {
            i16::MIN..=5 => 2,
            6 => 3,
            7..=17 => 0,
            18 => 1,
            _ => 2,
        }
    }

    /// Finish a scheduler pass. A partial pass does not advance the clock.
    /// Each completed tick advances two game seconds; 43,200 ticks make a day.
    /// The original calendar helper 0x00473544 returns 30 for every month.
    pub fn finish_tick(&mut self, complete: bool) -> Vec<ClockEvent> {
        let mut events = Vec::new();
        let mut signal = None;
        let mut hour_changed = false;
        if complete {
            self.ticks = self.ticks.wrapping_add(1);
            self.second = self.second.wrapping_add(2);
            if self.second > 59 {
                self.minute = self.minute.wrapping_add(1);
                self.second = self.second.wrapping_sub(60);
                signal = Some(0xdb);
                if self.minute > 59 {
                    self.hour = self.hour.wrapping_add(1);
                    self.minute = self.minute.wrapping_sub(60);
                    hour_changed = true;
                    if self.hour > 23 {
                        self.elapsed_days = self.elapsed_days.wrapping_add(1);
                        self.day = self.day.wrapping_add(1);
                        self.hour = self.hour.wrapping_sub(24);
                        signal = Some(0xdc);
                        events.push(ClockEvent::DailyObjects);
                        events.push(ClockEvent::DailyNeighborhood);
                        // Preserve the original forward-copy order, including its
                        // propagation of block zero through all four history blocks.
                        for i in 1..5 {
                            self.statistics[i] = self.statistics[i - 1];
                        }
                        self.statistics[0] = self.statistics[5];
                        self.statistics[5] = [0; 8];
                        if self.day > 30 {
                            self.month = self.month.wrapping_add(1);
                            self.day = 1;
                            if self.month > 12 {
                                self.year = self.year.wrapping_add(1);
                                self.month = 1;
                            }
                            self.month_days = 30;
                        }
                    }
                }
            }
        }
        events.push(ClockEvent::WorldUpdate { partial: !complete });
        if let Some(kind) = signal {
            events.push(ClockEvent::Signal { kind, argument: 0 });
        }
        if hour_changed && self.period != Self::period_for_hour(self.hour) {
            self.period = Self::period_for_hour(self.hour);
            events.push(ClockEvent::Lighting);
            events.push(ClockEvent::Signal {
                kind: 0xf8,
                argument: 5,
            });
            events.push(ClockEvent::Signal {
                kind: 0xdd,
                argument: 0,
            });
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::host_ticks_from_milliseconds;

    #[test]
    fn native_host_tick_millisecond_boundaries_preserve_integer_truncation() {
        for (milliseconds, ticks) in [
            (0, 0),
            (1, 0),
            (16, 0),
            (17, 1),
            (999, 59),
            (1000, 60),
            (1001, 60),
            (65535, 3932),
        ] {
            assert_eq!(host_ticks_from_milliseconds(milliseconds), ticks);
        }
    }

    #[test]
    fn native_host_tick_product_wraps_before_unsigned_division() {
        for (milliseconds, ticks) in [
            (71582788, 4294967),
            (71582789, 0),
            (i32::MAX as u32, 4294967),
            (1 << 31, 0),
            (u32::MAX - 1, 4294967),
            (u32::MAX, 4294967),
        ] {
            assert_eq!(host_ticks_from_milliseconds(milliseconds), ticks);
        }
        assert_ne!(
            u64::from(host_ticks_from_milliseconds(u32::MAX)),
            u64::from(u32::MAX) * 60 / 1000,
        );
    }
}
