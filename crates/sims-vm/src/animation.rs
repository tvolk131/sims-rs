//! Foreground Animate Sim event contract from native 0x0058950b.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    Waiting,
    Event(i16),
    Completed,
}

/// 0x00592904 selects the default animator posture after foreground playback.
/// Unrecognized person postures leave the previous animator mode unchanged.
pub fn posture_mode(posture: i16, previous: u32) -> u32 {
    match posture {
        0 => 3,
        1 => 1,
        2 => 2,
        _ => previous,
    }
}

/// Global action-animation cache selection (0x59dd16, initialized by 0x59d860).
/// This signed age test treats age zero as a child, unlike motive-score tables.
pub fn global_action_table(age: i16) -> u16 {
    if age < 18 { 157 } else { 156 }
}

/// 0x59dd6d selects fixed object STR# resources, not the OBJD animation link.
/// The action icon's age participates only for foreground mode and when the
/// active action target is this animation owner; otherwise use the actor's age.
/// `icon_age` is present only after the native person-kind/RTTI checks.
pub fn object_action_table(
    age: i16,
    foreground: bool,
    active_target_matches: bool,
    icon_age: Option<i16>,
) -> u16 {
    let partner = if foreground && active_target_matches {
        icon_age.unwrap_or(age)
    } else {
        age
    };
    match (age < 18, partner < 18) {
        (false, false) => 129,
        (true, true) => 130,
        (false, true) => 131,
        (true, false) => 132,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameTable {
    Private(u16),
    Global(u16),
}

/// Default posture/walk name selection (0x59db2f). Lookups receive zero-based
/// indices; native StringSet receives index+1. The special profile replaces a
/// private override even when its entry is absent or empty, then uses the normal
/// age/species profile and finally Global 150. No clip is fabricated if all miss.
pub fn posture_name<'a>(
    age: i16,
    special: i16,
    species: i16,
    index: usize,
    mut lookup: impl FnMut(NameTable, usize) -> Option<&'a str>,
) -> Option<&'a str> {
    let nonempty = |s: Option<&'a str>| s.filter(|s| !s.is_empty());
    let mut name = nonempty(lookup(NameTable::Private(150), index));
    if special != 0 {
        let table = if age < 18 {
            160
        } else {
            158 + (species as u16 & 1)
        };
        name = nonempty(lookup(NameTable::Global(table), index));
    }
    if name.is_none() {
        let table = if age < 18 {
            151
        } else {
            152 + (species as u16 & 1)
        };
        name = nonempty(lookup(NameTable::Global(table), index));
    }
    if name.is_none() {
        name = nonempty(lookup(NameTable::Global(150), index));
    }
    name
}

/// The event-count operand is a signed byte. Numbered events (<100) must arrive
/// in sequence; higher events are independent. Ended clips synthesize missing
/// numbered events one per primitive invocation before returning true.
pub fn poll_events(
    count: i8,
    reverse: bool,
    seen: &mut i32,
    event: Option<i16>,
    ended: bool,
) -> Poll {
    let count = i32::from(count);
    let expected = if reverse { count - *seen - 1 } else { *seen };
    if let Some(event) = event {
        if event >= 100 {
            return Poll::Event(event);
        }
        if expected < 0 || expected >= count || i32::from(event) != expected || event < 0 {
            return Poll::Waiting;
        }
        *seen += 1;
        Poll::Event(expected as i16)
    } else if !ended {
        Poll::Waiting
    } else if *seen >= count {
        Poll::Completed
    } else {
        *seen += 1;
        Poll::Event(expected as i16)
    }
}

/// Original 0x47ec4c: bits 0 and 4 select foreground, background, carry, pickup.
pub fn mode(flags: u8) -> u8 {
    (flags & 1) | ((flags >> 3) & 2)
}

/// Native automatic carry selection. Mode 6 is an explicit script carry clip;
/// pickup suppresses automatic replacement without changing the saved mode.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CarrySelector {
    pub mode: i32,
    pub selection: i32,
}
impl Default for CarrySelector {
    fn default() -> Self {
        Self {
            mode: 0,
            selection: -1,
        }
    }
}
impl CarrySelector {
    /// 0x58950b mode 2: installing a resolved script clip or explicitly clearing it.
    pub fn script_clip(&mut self, present: bool) {
        self.mode = if present { 6 } else { 0 };
        self.selection = -1;
    }

    /// None means no change. Some(None) removes the old layer; Some(Some(38))
    /// removes it and attempts original global carry clip 38. Selection changes
    /// even when that resource is absent or playback cannot start.
    pub fn update(&mut self, held: bool, pickup: bool) -> Option<Option<u16>> {
        if self.mode == 6 {
            return None;
        }
        let mut next = -1;
        if !pickup {
            self.mode = i32::from(held);
            if self.mode != 0 && self.mode < 6 {
                next = 0;
            }
        }
        if self.selection == next {
            return None;
        }
        self.selection = next;
        Some((next == 0).then_some(38))
    }
}

/// The original timed-property dispatcher (0x5912fd) recognizes a fixed,
/// case-sensitive vocabulary and ignores other keys, including `translation`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Property {
    Event(i32),
    Hand(usize, i16),
    Censor(i16),
    Interruptible(i32),
    Footstep(i32),
    Sound,
    Unsupported,
    Ignored,
}
pub fn property(key: &str, value: &str) -> Result<Property, String> {
    let numeric = || -> Result<i32, String> {
        let text = value.trim_start_matches(|c: char| c.is_ascii_whitespace());
        let (sign, text) = if let Some(t) = text.strip_prefix('-') {
            (-1i64, t)
        } else {
            (1i64, text.strip_prefix('+').unwrap_or(text))
        };
        let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return Ok(0i32);
        }
        let n = digits
            .parse::<i64>()
            .map_err(|_| "animation integer overflow")?;
        i32::try_from(sign * n).map_err(|_| "animation integer overflow".into())
    };
    Ok(match key {
        "xevt" => Property::Event(numeric()?),
        "lefthand" => Property::Hand(0, numeric()? as i16),
        "righthand" => Property::Hand(1, numeric()? as i16),
        "censor" => Property::Censor(numeric()? as i16),
        "interruptable" | "interruptible" => Property::Interruptible(numeric()?),
        "footstep" => Property::Footstep(numeric()?),
        "sound" => Property::Sound,
        "anchor" | "dress" | "undress" | "selectedsound" | "deselectedsound" | "discontinuity" => {
            Property::Unsupported
        }
        _ => Property::Ignored,
    })
}
