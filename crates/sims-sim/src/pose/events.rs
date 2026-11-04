//! Native timed-property construction and cursor scanning. The property callback
//! is a separate service: a yielded event must be handled before scanning resumes.
use serde::{Deserialize, Serialize};
use sims_formats::animation::{Motion, TimedProperty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub time: i32,
    pub properties: Vec<(String, String)>,
}
/// Original 59431a / 594390 with the captured C-locale folding policy. Lookup
/// precedes lowercase copying, so differently cased input keys can create
/// duplicate lowercase keys. Values are folded too. Keep first insertion order.
pub fn properties(input: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for (key, value) in input {
        if let Some((_, old)) = out.iter_mut().find(|(k, _)| k == key) {
            *old = value.to_ascii_lowercase();
        } else {
            out.push((key.to_ascii_lowercase(), value.to_ascii_lowercase()));
        }
    }
    out
}
/// Original 5946fc: sorted insertion, replacing the whole property list at an
/// equal timestamp. This is deliberately different from a stable sorted concat.
pub fn ordered<T>(input: impl IntoIterator<Item = (i32, T)>) -> Vec<(i32, T)> {
    let mut out = std::collections::BTreeMap::new();
    for (time, value) in input {
        out.insert(time, value);
    }
    out.into_iter().collect()
}
pub fn stream(motion: &Motion) -> Result<Vec<Entry>, String> {
    if motion.event_lists.len() > 1 {
        return Err("native motion has one optional timed-property list".into());
    }
    let input = motion
        .event_lists
        .iter()
        .flatten()
        .map(|e: &TimedProperty| (e.time_ms, properties(&e.pairs)));
    Ok(ordered(input)
        .into_iter()
        .map(|(time, properties)| Entry { time, properties })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    ForwardTail,
    Forward,
    ReverseTail,
    Reverse,
    Done,
}
/// Serializable control flow for the native cursor scan within one binding.
/// The cursor itself belongs to Layer::bindings. No event is sorted across bones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scan {
    phase: Phase,
    /// Original cursor step happens after the callback returns, not before it.
    after_callback: i32,
    upper: i32,
}
impl Scan {
    pub fn new(
        speed: f32,
        current: i32,
        previous: i32,
        original_duration: f32,
    ) -> Result<Self, String> {
        if !speed.is_finite() || !original_duration.is_finite() || original_duration < 0. {
            return Err("invalid native event scan input".into());
        }
        // Original 597ca8 adds 40 ms in extended precision, then truncates.
        let upper = (f64::from(original_duration) + 40.).trunc();
        if upper > i32::MAX as f64 {
            return Err("native event duration overflow".into());
        }
        let phase = if speed >= 0. {
            if current < previous {
                Phase::ForwardTail
            } else {
                Phase::Forward
            }
        } else if previous < current {
            Phase::ReverseTail
        } else {
            Phase::Reverse
        };
        Ok(Self {
            phase,
            after_callback: 0,
            upper: upper as i32,
        })
    }
    /// Return the next eligible property-list index, keeping the cursor on that
    /// entry throughout its callback. Call again only after the callback completes.
    /// Callback absence still consumes entries. Empty lists are gated by the caller.
    pub fn next(
        &mut self,
        cursor: &mut i32,
        entries: &[Entry],
        current: i32,
        callback: bool,
    ) -> Option<usize> {
        if self.after_callback != 0 {
            *cursor = cursor.wrapping_add(self.after_callback);
            self.after_callback = 0;
        }
        loop {
            if self.phase == Phase::Done {
                return None;
            }
            if self.phase == Phase::ReverseTail {
                *cursor = cursor.wrapping_sub(1);
            }
            let valid = *cursor >= 0 && (*cursor as usize) < entries.len();
            if !valid {
                match self.phase {
                    Phase::ForwardTail => {
                        *cursor = 0;
                        self.phase = Phase::Forward;
                        continue;
                    }
                    Phase::ReverseTail => {
                        *cursor = entries.len() as i32 - 1;
                        self.phase = Phase::Reverse;
                        continue;
                    }
                    _ => {
                        self.phase = Phase::Done;
                        return None;
                    }
                }
            }
            let index = *cursor as usize;
            let event = &entries[index];
            let in_range = event.time >= 0 && event.time <= self.upper;
            if in_range
                && ((self.phase == Phase::Forward && event.time > current)
                    || (self.phase == Phase::Reverse && event.time < current))
            {
                self.phase = Phase::Done;
                return None;
            }
            let step = match self.phase {
                Phase::ForwardTail | Phase::Forward => 1,
                Phase::Reverse => -1,
                Phase::ReverseTail => 0, // pre-decrement at the next iteration
                Phase::Done => unreachable!(),
            };
            if in_range && !event.properties.is_empty() && callback {
                self.after_callback = step;
                return Some(index);
            }
            *cursor = cursor.wrapping_add(step);
        }
    }
}
