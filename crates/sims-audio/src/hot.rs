//! HOT files: the sound tables that go with a HIT program.
//!
//! A HOT file is an INI-style text file with seven sections. Every one of
//! the install's 27 HOT files uses only these (checked by the gated test):
//!
//! - `[Options]`: `Version` and `LoadPriority`.
//! - `[EventMappingEquate]`: named numbers (`kSndobPlay=1`) the other
//!   sections may use in place of numbers.
//! - `[TrackData]`: `0xID=0xADDRESS`. A *track data* ID names an entry
//!   point of the HIT program; the address is a byte offset into the HIT
//!   file. HIT branch and call operands are track data IDs too.
//! - `[Patch]`: `ID=0,name,"path",...`. A *patch* is one sample file. The
//!   path is relative to the install (see [`crate::files`]). 24 patches
//!   of one download give a second name before the path; the path is the
//!   first quoted field.
//! - `[GlobalHitList]`: `ID=items`. A *hit list* is a set of patches a
//!   script picks from at random. Items are patch IDs or `low-high`
//!   ranges, separated by commas; an empty list is allowed. Six items
//!   are neither: four names no patch carries (two expansion packs) and
//!   two dotted pairs such as `2370.2376` (the base game). What the
//!   original makes of them is **unverified**; they are kept apart in
//!   [`Hot::hit_list_unread`].
//! - `[Track]`: `ID=0,name,volume,args,duck priority,group,loudness,@entry,patch`.
//!   A *track* is a sound object: the HIT entry point that runs it (a
//!   `@name` from the HSM file, or a number, read as a track data ID) and
//!   the patch it starts with (`@name`, a number, or 0 for none).
//! - `[EventMapping]`: `name=kind,track,...`. An *event* is what the game
//!   asks for by name, such as `Phone_ring`. `kind` says what to do with
//!   it (`kSndobPlay` = 1 plays the track). Some events give only a kind.
//!
//! The field meanings come from the files' own names (the HSM lists
//! `vol`, `duckpri`, `patch` and so on) and public notes on the format
//! (the Niotso wiki's HOT page, FreeSO's HIT player). They are
//! **unverified** against the original.

use crate::hsm::Symbols;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// The most hit list entries one HOT file may hold, ranges expanded. The
/// install's largest holds 3,661; the cap keeps a hostile file from
/// expanding a few bytes of ranges into gigabytes.
pub const MAX_HIT_LIST_ENTRIES: usize = 100_000;

fn fail<T>(line: usize, message: impl Into<String>) -> Result<T> {
    Err(Error::text("HOT", line, message))
}

/// A field that names a number: a literal, an equate, or an HSM symbol
/// (`@name`). Kept as written; [`Hot::number`] resolves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Number(i64),
    Name(String),
    Symbol(String),
}

impl Value {
    fn parse(text: &str) -> Self {
        let text = text.trim();
        if let Some(symbol) = text.strip_prefix('@') {
            Self::Symbol(symbol.to_owned())
        } else if let Some(number) = number(text) {
            Self::Number(number)
        } else {
            Self::Name(text.to_owned())
        }
    }
}

/// Reads a decimal or `0x` hexadecimal number, with an optional sign.
pub fn number(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let value = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => i64::from_str_radix(hex, 16).ok()?,
        None if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            digits.parse().ok()?
        }
        None => return None,
    };
    Some(if negative { -value } else { value })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub name: String,
    /// The sample's path relative to the install.
    pub path: String,
    /// *(code)* Whether the patch's `IsStreamed` flag (register 38, `0x26`)
    /// is set: the second flag field after the quoted path is `1`, cleared
    /// when the path contains `"sounddata\\"` (`0x57b4a2`..`0x57b4bc`,
    /// `0x57b5b4`..`0x57b5d1`).
    pub is_streamed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub name: String,
    pub volume: Value,
    pub args: Value,
    pub duck_priority: Value,
    pub group: Value,
    pub loudness: Value,
    /// The HIT entry point.
    pub entry: Value,
    /// The patch the track starts with.
    pub patch: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// What to do: 1 (`kSndobPlay`) plays `track`.
    pub kind: Value,
    pub track: Option<Value>,
    /// Further numbers, unused by the events this crate plays.
    pub rest: Vec<Value>,
}

/// One HOT file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hot {
    pub options: BTreeMap<String, String>,
    pub equates: BTreeMap<String, i64>,
    /// Track data ID to HIT address.
    pub track_data: BTreeMap<u32, u32>,
    pub patches: BTreeMap<u32, Patch>,
    /// Hit list ID to patch IDs, ranges expanded, in the order written.
    pub hit_lists: BTreeMap<u32, Vec<u32>>,
    /// Hit list items that are neither a number nor a range, with their
    /// list's ID, in file order. Not part of `hit_lists`.
    pub hit_list_unread: Vec<(u32, String)>,
    pub tracks: BTreeMap<u32, Track>,
    /// Event names as written, in file order; lookups ignore case.
    pub events: Vec<(String, Event)>,
    /// The IDs of the `[Patch]` and `[Track]` lines, in file order, but
    /// for ID 0, which the original's handlers skip (0x57b377,
    /// 0x57b6b2): the order it registers their objects in (see
    /// [`crate::registry`]).
    pub registered: Vec<(u32, Object)>,
}

/// What a `[Patch]` or `[Track]` line registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Patch,
    Track,
}

impl Hot {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text: String = bytes.iter().map(|&b| char::from(b)).collect();
        let mut hot = Self::default();
        let mut section = String::new();
        // Hit list entries so far, ranges expanded.
        let mut entries = 0usize;
        for (index, line) in text.split('\n').enumerate() {
            let line = line.trim_end_matches('\r');
            let number_of = index + 1;
            if line.trim().is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[') {
                let Some(name) = name.trim_end().strip_suffix(']') else {
                    return fail(number_of, "unclosed section name");
                };
                section = name.to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return fail(number_of, "line without =");
            };
            let id = || {
                number(key)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| Error::text("HOT", number_of, format!("bad ID {key:?}")))
            };
            match section.as_str() {
                "Options" => {
                    hot.options.insert(key.to_owned(), value.trim().to_owned());
                }
                "EventMappingEquate" => match number(value) {
                    Some(n) => {
                        hot.equates.insert(key.to_owned(), n);
                    }
                    None => return fail(number_of, format!("equate {key} is not a number")),
                },
                "TrackData" => {
                    let address = number(value)
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or_else(|| Error::text("HOT", number_of, "bad address"))?;
                    hot.track_data.insert(id()?, address);
                }
                "Patch" => {
                    let fields: Vec<&str> = value.split(',').collect();
                    let Some(path_index) = fields.iter().position(|f| f.starts_with('"')) else {
                        return fail(number_of, "patch without a quoted path");
                    };
                    let path = fields[path_index].trim_matches('"').to_owned();
                    // *(code)* 0x57b4a2..0x57b4bc, 0x57b5b4..0x57b5d1: the
                    // second flag after the path sets `IsStreamed` when equal
                    // to 1, except paths containing `"sounddata\\"` clear it.
                    let is_streamed = fields
                        .get(path_index + 2)
                        .and_then(|f| number(f))
                        .is_some_and(|n| n == 1)
                        && !path.to_ascii_lowercase().contains("sounddata\\");
                    let id = id()?;
                    if id != 0 {
                        hot.registered.push((id, Object::Patch));
                    }
                    hot.patches.insert(
                        id,
                        Patch {
                            name: fields.get(1).unwrap_or(&"").to_string(),
                            path,
                            is_streamed,
                        },
                    );
                }
                "GlobalHitList" => {
                    let list = id()?;
                    let mut patches = Vec::new();
                    for item in value.split(',').filter(|i| !i.trim().is_empty()) {
                        let read = |t: &str| number(t).and_then(|n| u32::try_from(n).ok());
                        let range = item
                            .split_once('-')
                            .and_then(|(low, high)| Some((read(low)?, read(high)?)));
                        let more = match (read(item), range) {
                            (Some(patch), _) => vec![patch],
                            (None, Some((low, high))) if low <= high && high - low <= 10_000 => {
                                (low..=high).collect()
                            }
                            _ => {
                                hot.hit_list_unread.push((list, item.trim().to_owned()));
                                continue;
                            }
                        };
                        entries += more.len();
                        if entries > MAX_HIT_LIST_ENTRIES {
                            return fail(
                                number_of,
                                format!("hit lists hold more than {MAX_HIT_LIST_ENTRIES} entries"),
                            );
                        }
                        patches.extend(more);
                    }
                    hot.hit_lists.insert(list, patches);
                }
                "Track" => {
                    let fields: Vec<Value> = value.split(',').map(Value::parse).collect();
                    if fields.len() < 9 {
                        return fail(number_of, "track with fewer than nine fields");
                    }
                    let name = value.split(',').nth(1).unwrap_or_default().to_owned();
                    let mut fields = fields.into_iter().skip(2);
                    let mut next = || fields.next().expect("nine fields");
                    let id = id()?;
                    if id != 0 {
                        hot.registered.push((id, Object::Track));
                    }
                    hot.tracks.insert(
                        id,
                        Track {
                            name,
                            volume: next(),
                            args: next(),
                            duck_priority: next(),
                            group: next(),
                            loudness: next(),
                            entry: next(),
                            patch: next(),
                        },
                    );
                }
                "EventMapping" => {
                    let mut fields = value.split(',').map(Value::parse);
                    let kind = fields.next().expect("split gives one field");
                    let track = fields.next();
                    hot.events.push((
                        key.to_owned(),
                        Event {
                            kind,
                            track,
                            rest: fields.collect(),
                        },
                    ));
                }
                other => return fail(number_of, format!("unknown section [{other}]")),
            }
        }
        Ok(hot)
    }

    /// The number a value names: a literal, an equate of this file, or a
    /// symbol of `symbols`. None for a name neither defines.
    pub fn number(&self, value: &Value, symbols: &Symbols) -> Option<i64> {
        match value {
            Value::Number(n) => Some(*n),
            Value::Name(name) => self.equates.get(name).copied(),
            Value::Symbol(name) => symbols.get(name),
        }
    }

    /// The event of that name, ignoring ASCII case. The first of several
    /// with one name wins.
    pub fn event(&self, name: &str) -> Option<&Event> {
        self.events
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, e)| e)
    }
}
