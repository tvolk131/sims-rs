//! `sims-antics test`: live-VM unit tests for `.antics` objects.
//!
//! A test file (`*.test.toml`, read with the crate's own TOML subset parser,
//! [`crate::manifest::parse_toml`]) names a subject object (an `.antics` v2 source compiled
//! in memory, or a compiled `.iff`), optional stub trees for the global and semi-global
//! ranges, and a list of `[[test]]` cases. Each case boots a small deterministic `sims-vm`
//! lot holding the subject (and, when asked, a Sim and other objects), runs one tree or one
//! interaction, and compares what it left with the case's expectations.
//!
//! ```toml
//! [subject]
//! antics = "../objects/open-tv.antics"   # relative to this file
//! pack = ".."                             # for `use crate::...` imports
//! # idle_main = true                      # replace a Main that returns at once by an idle
//!                                         # loop (reported as an ASSUME line per case)
//!
//! [sim]                                   # a Sim on the lot, for every case
//! motives = { fun = -20 }
//!
//! [[test]]
//! name = "turning the set on clicks"
//! action = "Turn On"                      # the interaction's action tree
//! given = { attrs = { power = 0 } }
//! expect = { result = true, attrs = { power = 1 }, sounds = ["open_tv_click"] }
//! ```
//!
//! The format is documented in full in `docs/decisions/20261124-antics-test-runner.md`.
//! Parsing ([`TestFile::parse`]) needs nothing but this crate; running a file
//! ([`run_test_file`]) needs `sims-vm`, behind the default `vm-replay` feature.

use crate::error::{Error, Result};
use crate::manifest::{TomlValue, parse_toml};
use crate::parser::{motive_by_name, person_field_by_name, skill_by_name};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(feature = "vm-replay")]
mod harness;

/// The instructions a synchronous call may run before it counts as a fault: the VM's own
/// per-turn guard (`sims_vm::TURN_BUDGET`). A threaded run's turns are lot updates, under
/// the game's own limit (`sims_vm::TICK_BUDGET`), and take no budget.
pub const DEFAULT_BUDGET: usize = 100_000;

/// The most turns a test may ask for. A turn is one lot update (`sims-vm`'s
/// `Interpreter::lot_update_kept`), which runs every object's tick.
pub const MAX_TURNS: u32 = 100_000;

/// One parsed `*.test.toml` file.
#[derive(Debug, Clone, PartialEq)]
pub struct TestFile {
    /// The object under test.
    pub subject: Source,
    /// `[subject] idle_main = true`: the subject's Main, which returns at once (as v2's
    /// default Main does), is replaced after the subject is made by an idle loop, so the lot
    /// update between turns can tick it. Only the subject's definition (and the tiles of its
    /// group) is replaced, and a case reports each replacement it made
    /// ([`CaseReport::assumptions`]); a Main that does not return is a setup error with it.
    pub idle_main: bool,
    /// Other objects loaded into the lot's catalog (not created unless a case's
    /// `given.create` names them).
    pub also: Vec<Source>,
    /// The `[lot].also` objects, by name ([`Source::slug`]), given `idle_main = true` in their
    /// table: as the subject's, their returning Main is replaced when a case creates them.
    pub also_idle_main: BTreeSet<String>,
    /// Stub trees for the global (256 to 4095) and semi-global (8192 and up) ranges.
    pub stubs: Vec<Stub>,
    /// The file-wide Sim, which every case gets (and may change with `given.sim`).
    pub sim: Option<SimState>,
    /// `[lot] room = true`: the lot's room 1 is a counted room of [`ROOM_TILES`] tiles, and
    /// the subject and every `given.create` object stand on a tile of it ([`TestCase::tiles`]).
    /// The room is counted before the run and after each lot update, as the game counts a
    /// marked room before it draws a frame. Off, every object is made off the tile grid and
    /// no room is counted. A case's own `room` key overrides it ([`TestCase::in_room`]).
    pub room: bool,
    /// `[subject] place = [x, y]`: the subject's tile in the room (with `room`).
    pub subject_place: Option<[u8; 2]>,
    pub cases: Vec<TestCase>,
}

/// The tiles of the room `[lot] room = true` gives the lot, each coordinate in this range:
/// an 8 by 8 room on level 1, inside the test lot's 16 tiles. Its one wall, a covered west
/// wall on the first tile, gives its score a finite value.
pub const ROOM_TILES: std::ops::Range<u8> = 4..12;

/// Where an object comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// An `.antics` v2 object source, compiled in memory, with the pack directory its
    /// `use crate::...` imports resolve against.
    Antics {
        path: PathBuf,
        pack: Option<PathBuf>,
    },
    /// A compiled object file.
    Iff { path: PathBuf },
}

impl Source {
    /// The file the source reads.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Antics { path, .. } | Self::Iff { path } => path,
        }
    }

    /// The name a case's `given.create` uses for it: the file stem (`open-counter`).
    #[must_use]
    pub fn slug(&self) -> String {
        self.path()
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// A stub library entry: one tree that always returns a fixed result, or a module of trees
/// written in `.air` / legacy `.antics` syntax (compiled with [`crate::compile_antics`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stub {
    Returns {
        id: u16,
        label: String,
        args: u8,
        result: bool,
    },
    Source {
        /// Where the source came from, for messages.
        origin: String,
        text: String,
    },
}

/// Which tree or interaction a case runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A tree of the subject's file, by BHAV ID or label.
    Tree(TreeRef),
    /// The check tree of the subject's interaction with this name.
    Guard(String),
    /// The action tree of the subject's interaction with this name, run directly.
    Action(String),
    /// The interaction with this name, picked from the subject's pie menu by the Sim and
    /// run on the Sim's thread over up to `turns` turns.
    Interaction(String),
}

/// A tree named by ID or label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRef {
    Id(u16),
    Label(String),
}

impl fmt::Display for TreeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Id(id) => write!(f, "#{id}"),
            Self::Label(l) => write!(f, "\"{l}\""),
        }
    }
}

/// Whose thread a tree runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Object,
    Sim,
}

/// An object reference in a case: the subject, the Sim, no object (0), or a raw ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjRef {
    Object,
    Sim,
    None,
    Id(i16),
}

impl fmt::Display for ObjRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Object => f.write_str("object"),
            Self::Sim => f.write_str("sim"),
            Self::None => f.write_str("none"),
            Self::Id(id) => write!(f, "{id}"),
        }
    }
}

/// An attribute named by index or by `.antics` state name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttrKey {
    Index(u16),
    Name(String),
}

impl fmt::Display for AttrKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index(i) => write!(f, "{i}"),
            Self::Name(n) => f.write_str(n),
        }
    }
}

/// A Sim's inputs or expected state. Motives are the VM's floats; the rest are words.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimState {
    /// By motive index (`hunger` is 7).
    pub motives: BTreeMap<u8, f32>,
    /// Person data words, by index: skills, personality, `age`, `priority` and raw
    /// `person = { 58 = 25 }` all land here.
    pub person: BTreeMap<u16, i16>,
    pub attrs: BTreeMap<u16, i16>,
    pub temps: BTreeMap<u16, i16>,
    pub data: BTreeMap<u16, i16>,
}

impl SimState {
    fn merged_over(&self, base: &Self) -> Self {
        let mut out = base.clone();
        out.motives.extend(&self.motives);
        out.person.extend(&self.person);
        out.attrs.extend(&self.attrs);
        out.temps.extend(&self.temps);
        out.data.extend(&self.data);
        out
    }
}

/// What a case sets up before its run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Given {
    pub attrs: BTreeMap<AttrKey, i16>,
    pub temps: BTreeMap<u16, i16>,
    pub data: BTreeMap<u16, i16>,
    pub globals: BTreeMap<u16, i16>,
    pub funds: Option<i32>,
    pub sim: Option<SimState>,
    /// Objects to create after the subject, by slug of a loaded source or by GUID.
    pub create: Vec<Create>,
}

/// An object `given.create` makes: `"open-fridge"`, a GUID, or
/// `{ object = "open-fridge", place = [7, 6] }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Create {
    pub object: CreateRef,
    /// Its tile in the room (with `[lot] room = true`); the next free one when left out.
    pub place: Option<[u8; 2]>,
}

/// Which object `given.create` makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateRef {
    Slug(String),
    Guid(u32),
}

/// `expect.objects.<slug>`: the objects of a loaded source on the lot after the run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectsExpect {
    /// How many there are. Left out, at least one; a bare number is a count alone.
    pub count: Option<u16>,
    /// What every one of them holds.
    pub attrs: BTreeMap<AttrKey, i16>,
    pub temps: BTreeMap<u16, i16>,
    pub data: BTreeMap<u16, i16>,
}

/// `expect.room`: the counted room 1 (`[lot] room = true`) after the run, as
/// `sims_sim::rooms::Room` holds it. Every field left out is not checked.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RoomExpect {
    /// The room's score, -100 to 100, compared exactly.
    pub score: Option<f32>,
    /// The room's light, 0 to 1, compared exactly.
    pub light: Option<f32>,
    /// The sum of data word 51 (`light`) of its light sources that are not windows.
    pub lamp_light: Option<i32>,
    /// Its light sources (data word 40 flag 0x400) that stand on the lot and are not windows.
    pub light_sources: Option<i32>,
    /// The sum of its objects' data word 7 (`room_impact`).
    pub impact: Option<i32>,
    /// The objects counted in it.
    pub objects: Option<i32>,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The tree returned true.
    True,
    /// The tree returned false.
    False,
    /// The run faulted (any VM trap, including the instruction budget).
    Error,
    /// The run finished without a fault, whatever it returned.
    Ok,
    /// The thread was still running when its turns ran out.
    Yield,
    /// The pie menu did not offer the interaction.
    Unavailable,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::True => "true",
            Self::False => "false",
            Self::Error => "error",
            Self::Ok => "ok",
            Self::Yield => "yield",
            Self::Unavailable => "unavailable",
        })
    }
}

/// An expected dialog: its kind (operand 5, bits 0 to 6) and whether it only informs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogExpect {
    pub kind: u8,
}

/// An expected entry of the Sim's action queue: an interaction pushed during the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueExpect {
    pub interaction: Option<String>,
    pub menu: Option<u32>,
    pub target: Option<ObjRef>,
    pub priority: Option<i16>,
}

/// What a case checks after its run. Every field left out is not checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Expect {
    pub result: Option<Outcome>,
    /// A substring the fault's message must contain (implies `result = "error"`).
    pub error: Option<String>,
    pub attrs: BTreeMap<AttrKey, i16>,
    pub temps: BTreeMap<u16, i16>,
    pub data: BTreeMap<u16, i16>,
    pub globals: BTreeMap<u16, i16>,
    pub funds: Option<i32>,
    pub sim: Option<SimState>,
    pub stack_obj: Option<ObjRef>,
    /// Sound events played during the run, in order.
    pub sounds: Option<Vec<String>>,
    /// Dialogs made during the run, in order.
    pub dialogs: Option<Vec<DialogExpect>>,
    /// Interactions pushed onto the Sim's queue during the run, in queue order.
    pub queue: Option<Vec<QueueExpect>>,
    /// The turns the run took.
    pub turns: Option<u32>,
    /// Objects on the lot after the run, by slug of a loaded source.
    pub objects: BTreeMap<String, ObjectsExpect>,
    /// The counted room after the run (needs `[lot] room = true`).
    pub room: Option<RoomExpect>,
}

/// One `[[test]]` case.
#[derive(Debug, Clone, PartialEq)]
pub struct TestCase {
    pub name: String,
    pub target: Target,
    pub me: Option<Actor>,
    pub stack_obj: Option<ObjRef>,
    pub params: Vec<i16>,
    /// Run the tree as a thread for up to this many turns. Without it a tree runs as a
    /// synchronous call (`Interpreter::run_tree`) when `me` is the subject, and must not
    /// yield when `me` is the Sim. An interaction defaults to 60.
    pub turns: Option<u32>,
    pub budget: usize,
    /// Dialog buttons (1 to 3), answered in order as dialogs wait for an answer.
    pub answers: Vec<u8>,
    /// `room = true` or `false`: the case's own choice of `[lot] room` ([`TestCase::in_room`]).
    pub room: Option<bool>,
    pub given: Given,
    pub expect: Expect,
}

impl TestCase {
    /// Whether this case has a Sim on the lot (the file's or its own).
    #[must_use]
    pub fn has_sim(&self, file: &TestFile) -> bool {
        file.sim.is_some() || self.given.sim.is_some()
    }

    /// The Sim this case starts with: the file's, with the case's on top.
    #[must_use]
    pub fn sim_state(&self, file: &TestFile) -> Option<SimState> {
        match (&file.sim, &self.given.sim) {
            (None, None) => None,
            (Some(s), None) | (None, Some(s)) => Some(s.clone()),
            (Some(base), Some(over)) => Some(over.merged_over(base)),
        }
    }

    /// Whether the case runs in the counted room: its own `room`, or else the file's
    /// `[lot] room`.
    #[must_use]
    pub fn in_room(&self, file: &TestFile) -> bool {
        self.room.unwrap_or(file.room)
    }

    /// The tiles the subject (first) and each `given.create` object stand on in the room
    /// ([`TestCase::in_room`]): each its `place`, or else the next tile of the room no object
    /// claims, in reading order (row by row from `[4, 4]`, column by column within a row).
    /// Refuses a place outside the room, two objects on one tile, and more objects than tiles.
    pub fn tiles(&self, file: &TestFile) -> Result<Vec<[u8; 2]>> {
        let what = format!("test '{}'", self.name);
        let places: Vec<Option<[u8; 2]>> = std::iter::once(file.subject_place)
            .chain(self.given.create.iter().map(|c| c.place))
            .collect();
        let label = |i: usize| {
            if i == 0 {
                "the subject".to_owned()
            } else {
                format!("given.create[{}]", i - 1)
            }
        };
        let mut taken: BTreeMap<[u8; 2], usize> = BTreeMap::new();
        for (i, place) in places.iter().enumerate() {
            let Some(tile) = *place else { continue };
            if !tile.iter().all(|c| ROOM_TILES.contains(c)) {
                return Err(Error::new(format!(
                    "{what}: {} is placed at {tile:?}, outside the room (each coordinate {} \
                     to {})",
                    label(i),
                    ROOM_TILES.start,
                    ROOM_TILES.end - 1
                )));
            }
            if let Some(prev) = taken.insert(tile, i) {
                return Err(Error::new(format!(
                    "{what}: {} and {} are both placed at {tile:?}",
                    label(prev),
                    label(i)
                )));
            }
        }
        let mut free = ROOM_TILES
            .flat_map(|y| ROOM_TILES.map(move |x| [x, y]))
            .filter(|t| !taken.contains_key(t));
        places
            .iter()
            .enumerate()
            .map(|(i, place)| {
                place.or_else(|| free.next()).ok_or_else(|| {
                    Error::new(format!(
                        "{what}: the room has no free tile left for {}",
                        label(i)
                    ))
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------------------

type Table = BTreeMap<String, TomlValue>;

fn kind_of(v: &TomlValue) -> &'static str {
    match v {
        TomlValue::String(_) => "a string",
        TomlValue::Int(_) => "an integer",
        TomlValue::Float(_) => "a float",
        TomlValue::Bool(_) => "a boolean",
        TomlValue::Array(_) => "an array",
        TomlValue::Table(_) => "a table",
    }
}

fn check_keys(t: &Table, allowed: &[&str], what: &str) -> Result<()> {
    for k in t.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(Error::new(format!(
                "unknown key '{k}' in {what} (expected one of: {})",
                allowed.join(", ")
            )));
        }
    }
    Ok(())
}

fn as_table<'a>(v: &'a TomlValue, what: &str) -> Result<&'a Table> {
    match v {
        TomlValue::Table(t) => Ok(t),
        other => Err(Error::new(format!(
            "{what} must be a table, not {}",
            kind_of(other)
        ))),
    }
}

fn as_str<'a>(v: &'a TomlValue, what: &str) -> Result<&'a str> {
    match v {
        TomlValue::String(s) => Ok(s),
        other => Err(Error::new(format!(
            "{what} must be a string, not {}",
            kind_of(other)
        ))),
    }
}

fn as_int(v: &TomlValue, what: &str) -> Result<i64> {
    match v {
        TomlValue::Int(i) => Ok(*i),
        other => Err(Error::new(format!(
            "{what} must be an integer, not {}",
            kind_of(other)
        ))),
    }
}

fn as_i16(v: &TomlValue, what: &str) -> Result<i16> {
    let i = as_int(v, what)?;
    i16::try_from(i).map_err(|_| Error::new(format!("{what} = {i} is out of the 16-bit range")))
}

fn as_u16(v: &TomlValue, what: &str) -> Result<u16> {
    let i = as_int(v, what)?;
    u16::try_from(i).map_err(|_| Error::new(format!("{what} = {i} is out of range 0 to 65535")))
}

fn as_f32(v: &TomlValue, what: &str) -> Result<f32> {
    match v {
        TomlValue::Int(i) => {
            if i.unsigned_abs() > 1 << 24 {
                return Err(Error::new(format!("{what} = {i} is not exact as a float")));
            }
            Ok(*i as f32)
        }
        TomlValue::Float(f) => Ok(*f as f32),
        other => Err(Error::new(format!(
            "{what} must be a number, not {}",
            kind_of(other)
        ))),
    }
}

fn index_key(k: &str, what: &str) -> Result<u16> {
    k.parse::<u16>()
        .map_err(|_| Error::new(format!("{what}: '{k}' is not an index (0 to 65535)")))
}

/// Notes that the spec key `label` sets `key`, refusing a second spelling of the same key in
/// one table (`15` and `fun`, `0` and `00`, `age` and `person.58`): a table keeps one value
/// per key, so the later spelling would silently drop the earlier one's value.
fn claim<K: Ord>(
    seen: &mut BTreeMap<K, String>,
    key: K,
    label: String,
    what: &str,
    target: impl FnOnce() -> String,
) -> Result<()> {
    if let Some(prev) = seen.get(&key) {
        return Err(Error::new(format!(
            "{what}: '{prev}' and '{label}' both set {}; give it once",
            target()
        )));
    }
    seen.insert(key, label);
    Ok(())
}

/// `{ 0 = 5, 3 = -1 }`: words by index.
fn word_map(v: &TomlValue, what: &str) -> Result<BTreeMap<u16, i16>> {
    let t = as_table(v, what)?;
    let mut out = BTreeMap::new();
    let mut seen = BTreeMap::new();
    for (k, v) in t {
        let i = index_key(k, what)?;
        claim(&mut seen, i, k.clone(), what, || format!("word {i}"))?;
        out.insert(i, as_i16(v, &format!("{what}.{k}"))?);
    }
    Ok(out)
}

/// `{ stock = 4, 2 = 1 }`: attributes by index or state name. Two spellings of one index are
/// refused here; a name and the index it stands for are refused once the subject's state
/// names are known (by the harness).
fn attr_map(v: &TomlValue, what: &str) -> Result<BTreeMap<AttrKey, i16>> {
    let t = as_table(v, what)?;
    let mut out = BTreeMap::new();
    let mut seen = BTreeMap::new();
    for (k, v) in t {
        let key = if k.bytes().all(|b| b.is_ascii_digit()) {
            let i = index_key(k, what)?;
            claim(&mut seen, i, k.clone(), what, || format!("attribute {i}"))?;
            AttrKey::Index(i)
        } else {
            AttrKey::Name(k.clone())
        };
        out.insert(key, as_i16(v, &format!("{what}.{k}"))?);
    }
    Ok(out)
}

/// The person data word behind `sim.personality.<trait>` (as `.antics` v2 lowers it).
fn personality_word(name: &str) -> Option<u16> {
    match name {
        "nice" => Some(2),
        "active" => Some(3),
        "playful" => Some(5),
        "outgoing" => Some(6),
        "neat" => Some(7),
        _ => None,
    }
}

fn sim_state(v: &TomlValue, what: &str) -> Result<SimState> {
    let t = as_table(v, what)?;
    check_keys(
        t,
        &[
            "motives",
            "skills",
            "personality",
            "person",
            "age",
            "priority",
            "attrs",
            "temps",
            "data",
        ],
        what,
    )?;
    let mut s = SimState::default();
    let mut motives_seen = BTreeMap::new();
    if let Some(m) = t.get("motives") {
        let w = format!("{what}.motives");
        for (k, v) in as_table(m, &w)? {
            let idx = motive_by_name(k)
                .or_else(|| k.parse::<u8>().ok().filter(|&i| i < 16))
                .ok_or_else(|| {
                    Error::new(format!(
                        "{what}.motives: unknown motive '{k}' (expected energy, comfort, hunger, \
                         hygiene, bladder, alertness, room, social, fun, mood, or an index 0 to 15)"
                    ))
                })?;
            claim(&mut motives_seen, idx, k.clone(), &w, || {
                format!("motive {idx}")
            })?;
            s.motives
                .insert(idx, as_f32(v, &format!("{what}.motives.{k}"))?);
        }
    }
    // Skills, personality, `person`, `age` and `priority` all land in the person words.
    let mut person_seen = BTreeMap::new();
    let mut person_word = |s: &mut SimState, idx: u16, label: String, v: &TomlValue| {
        claim(&mut person_seen, idx, label.clone(), what, || {
            format!("person word {idx}")
        })?;
        s.person.insert(idx, as_i16(v, &format!("{what}.{label}"))?);
        Ok::<(), Error>(())
    };
    if let Some(m) = t.get("skills") {
        for (k, v) in as_table(m, &format!("{what}.skills"))? {
            let idx = skill_by_name(k).ok_or_else(|| {
                Error::new(format!(
                    "{what}.skills: unknown skill '{k}' (expected cooking, charisma, mechanical, \
                     creativity, body, or logic)"
                ))
            })?;
            person_word(&mut s, u16::from(idx), format!("skills.{k}"), v)?;
        }
    }
    if let Some(m) = t.get("personality") {
        for (k, v) in as_table(m, &format!("{what}.personality"))? {
            let idx = personality_word(k).ok_or_else(|| {
                Error::new(format!(
                    "{what}.personality: unknown trait '{k}' (expected nice, active, playful, \
                     outgoing, or neat)"
                ))
            })?;
            person_word(&mut s, idx, format!("personality.{k}"), v)?;
        }
    }
    if let Some(m) = t.get("person") {
        for (k, v) in as_table(m, &format!("{what}.person"))? {
            let idx = match person_field_by_name(k) {
                Some(i) => u16::from(i),
                None if k == "age" => 58,
                None => index_key(k, &format!("{what}.person"))?,
            };
            person_word(&mut s, idx, format!("person.{k}"), v)?;
        }
    }
    if let Some(v) = t.get("age") {
        person_word(&mut s, 58, "age".to_owned(), v)?;
    }
    if let Some(v) = t.get("priority") {
        person_word(&mut s, 33, "priority".to_owned(), v)?;
    }
    if let Some(v) = t.get("attrs") {
        s.attrs = word_map(v, &format!("{what}.attrs"))?;
    }
    if let Some(v) = t.get("temps") {
        s.temps = word_map(v, &format!("{what}.temps"))?;
    }
    if let Some(v) = t.get("data") {
        s.data = word_map(v, &format!("{what}.data"))?;
    }
    Ok(s)
}

fn obj_ref(v: &TomlValue, what: &str) -> Result<ObjRef> {
    match v {
        TomlValue::String(s) => match s.as_str() {
            "object" | "self" | "me" => Ok(ObjRef::Object),
            "sim" => Ok(ObjRef::Sim),
            "none" => Ok(ObjRef::None),
            other => Err(Error::new(format!(
                "{what}: unknown object '{other}' (expected \"object\", \"sim\", \"none\", or an ID)"
            ))),
        },
        TomlValue::Int(_) => Ok(ObjRef::Id(as_i16(v, what)?)),
        other => Err(Error::new(format!(
            "{what} must be \"object\", \"sim\", \"none\", or an ID, not {}",
            kind_of(other)
        ))),
    }
}

fn outcome(v: &TomlValue, what: &str) -> Result<Outcome> {
    match v {
        TomlValue::Bool(true) => Ok(Outcome::True),
        TomlValue::Bool(false) => Ok(Outcome::False),
        TomlValue::String(s) => match s.as_str() {
            "true" => Ok(Outcome::True),
            "false" => Ok(Outcome::False),
            "error" => Ok(Outcome::Error),
            "ok" => Ok(Outcome::Ok),
            "yield" => Ok(Outcome::Yield),
            "unavailable" => Ok(Outcome::Unavailable),
            other => Err(Error::new(format!(
                "{what}: unknown result '{other}' (expected true, false, \"error\", \"ok\", \
                 \"yield\", or \"unavailable\")"
            ))),
        },
        other => Err(Error::new(format!(
            "{what} must be a boolean or a string, not {}",
            kind_of(other)
        ))),
    }
}

/// The `Dialog` kinds by the names `.antics` v2 gives them.
#[must_use]
pub fn dialog_kind_name(kind: u8) -> Option<&'static str> {
    match kind {
        0 => Some("ok"),
        1 => Some("yes_no"),
        2 => Some("yes_no_cancel"),
        3 => Some("text"),
        _ => None,
    }
}

fn dialog_kind(v: &TomlValue, what: &str) -> Result<u8> {
    match v {
        TomlValue::String(s) => (0..4)
            .find(|&k| dialog_kind_name(k) == Some(s.as_str()))
            .ok_or_else(|| {
                Error::new(format!(
                    "{what}: unknown dialog kind '{s}' (expected ok, yes_no, yes_no_cancel, \
                     text, or a kind number)"
                ))
            }),
        TomlValue::Int(i) => u8::try_from(*i)
            .ok()
            .filter(|&k| k < 0x80)
            .ok_or_else(|| Error::new(format!("{what}: dialog kind {i} is out of range 0 to 127"))),
        other => Err(Error::new(format!(
            "{what} must be a dialog kind name or number, not {}",
            kind_of(other)
        ))),
    }
}

fn queue_entry(v: &TomlValue, what: &str) -> Result<QueueExpect> {
    match v {
        TomlValue::String(s) => Ok(QueueExpect {
            interaction: Some(s.clone()),
            menu: None,
            target: None,
            priority: None,
        }),
        TomlValue::Table(t) => {
            check_keys(t, &["interaction", "menu", "target", "priority"], what)?;
            let q = QueueExpect {
                interaction: t
                    .get("interaction")
                    .map(|v| as_str(v, &format!("{what}.interaction")).map(str::to_owned))
                    .transpose()?,
                menu: t
                    .get("menu")
                    .map(|v| as_u16(v, &format!("{what}.menu")).map(u32::from))
                    .transpose()?,
                target: t
                    .get("target")
                    .map(|v| obj_ref(v, &format!("{what}.target")))
                    .transpose()?,
                priority: t
                    .get("priority")
                    .map(|v| as_i16(v, &format!("{what}.priority")))
                    .transpose()?,
            };
            if q.interaction.is_some() && q.menu.is_some() {
                return Err(Error::new(format!(
                    "{what}: give 'interaction' or 'menu', not both"
                )));
            }
            Ok(q)
        }
        other => Err(Error::new(format!(
            "{what} must be an interaction name or a table, not {}",
            kind_of(other)
        ))),
    }
}

fn array<'a>(v: &'a TomlValue, what: &str) -> Result<&'a [TomlValue]> {
    match v {
        TomlValue::Array(a) => Ok(a),
        other => Err(Error::new(format!(
            "{what} must be an array, not {}",
            kind_of(other)
        ))),
    }
}

fn given(v: &TomlValue, what: &str) -> Result<Given> {
    let t = as_table(v, what)?;
    check_keys(
        t,
        &[
            "attrs", "temps", "data", "globals", "funds", "sim", "create",
        ],
        what,
    )?;
    let mut g = Given::default();
    if let Some(v) = t.get("attrs") {
        g.attrs = attr_map(v, &format!("{what}.attrs"))?;
    }
    if let Some(v) = t.get("temps") {
        g.temps = word_map(v, &format!("{what}.temps"))?;
    }
    if let Some(v) = t.get("data") {
        g.data = word_map(v, &format!("{what}.data"))?;
    }
    if let Some(v) = t.get("globals") {
        g.globals = word_map(v, &format!("{what}.globals"))?;
    }
    if let Some(v) = t.get("funds") {
        g.funds = Some(funds(v, &format!("{what}.funds"))?);
    }
    if let Some(v) = t.get("sim") {
        g.sim = Some(sim_state(v, &format!("{what}.sim"))?);
    }
    if let Some(v) = t.get("create") {
        for (i, item) in array(v, &format!("{what}.create"))?.iter().enumerate() {
            let w = format!("{what}.create[{i}]");
            g.create.push(match item {
                TomlValue::Table(t) => {
                    check_keys(t, &["object", "place"], &w)?;
                    Create {
                        object: create_ref(
                            t.get("object").ok_or_else(|| {
                                Error::new(format!("{w}: give 'object', a slug or a GUID"))
                            })?,
                            &format!("{w}.object"),
                        )?,
                        place: t
                            .get("place")
                            .map(|v| tile(v, &format!("{w}.place")))
                            .transpose()?,
                    }
                }
                _ => Create {
                    object: create_ref(item, &w)?,
                    place: None,
                },
            });
        }
    }
    Ok(g)
}

fn create_ref(v: &TomlValue, what: &str) -> Result<CreateRef> {
    match v {
        TomlValue::String(s) => Ok(CreateRef::Slug(s.clone())),
        TomlValue::Int(n) => {
            Ok(CreateRef::Guid(u32::try_from(*n).map_err(|_| {
                Error::new(format!("{what}: GUID {n} is out of range"))
            })?))
        }
        other => Err(Error::new(format!(
            "{what} must be an object slug or a GUID, not {}",
            kind_of(other)
        ))),
    }
}

/// `[x, y]`: a tile, column and row. Whether it is in the room is checked with the case
/// ([`TestCase::tiles`]).
fn tile(v: &TomlValue, what: &str) -> Result<[u8; 2]> {
    let items = array(v, what)?;
    let [x, y] = items else {
        return Err(Error::new(format!(
            "{what} must be a tile [x, y], not {} values",
            items.len()
        )));
    };
    let coord = |v: &TomlValue, axis: &str| -> Result<u8> {
        let n = as_int(v, &format!("{what}.{axis}"))?;
        u8::try_from(n).map_err(|_| Error::new(format!("{what}: {axis} = {n} is not a tile")))
    };
    Ok([coord(x, "x")?, coord(y, "y")?])
}

/// `expect.objects`: `{ open-meal-plate = 1 }` or
/// `{ open-meal-plate = { count = 1, attrs = { servings = 3 } } }`.
fn objects_expect(v: &TomlValue, what: &str) -> Result<BTreeMap<String, ObjectsExpect>> {
    let mut out = BTreeMap::new();
    for (slug, v) in as_table(v, what)? {
        let w = format!("{what}.{slug}");
        let want = match v {
            TomlValue::Int(_) => ObjectsExpect {
                count: Some(as_u16(v, &w)?),
                ..ObjectsExpect::default()
            },
            _ => {
                let t = as_table(v, &w)?;
                check_keys(t, &["count", "attrs", "temps", "data"], &w)?;
                let want = ObjectsExpect {
                    count: t
                        .get("count")
                        .map(|v| as_u16(v, &format!("{w}.count")))
                        .transpose()?,
                    attrs: t
                        .get("attrs")
                        .map(|v| attr_map(v, &format!("{w}.attrs")))
                        .transpose()?
                        .unwrap_or_default(),
                    temps: t
                        .get("temps")
                        .map(|v| word_map(v, &format!("{w}.temps")))
                        .transpose()?
                        .unwrap_or_default(),
                    data: t
                        .get("data")
                        .map(|v| word_map(v, &format!("{w}.data")))
                        .transpose()?
                        .unwrap_or_default(),
                };
                if want.count == Some(0)
                    && !(want.attrs.is_empty() && want.temps.is_empty() && want.data.is_empty())
                {
                    return Err(Error::new(format!(
                        "{w}: count = 0 leaves no object for attrs, temps or data to check"
                    )));
                }
                want
            }
        };
        out.insert(slug.clone(), want);
    }
    Ok(out)
}

/// `expect.room`: `{ score = 12.5, lamp_light = 50, light_sources = 1 }`.
fn room_expect(v: &TomlValue, what: &str) -> Result<RoomExpect> {
    let t = as_table(v, what)?;
    check_keys(
        t,
        &[
            "score",
            "light",
            "lamp_light",
            "light_sources",
            "impact",
            "objects",
        ],
        what,
    )?;
    let float = |k: &str| {
        t.get(k)
            .map(|v| as_f32(v, &format!("{what}.{k}")))
            .transpose()
    };
    let int = |k: &str| {
        t.get(k)
            .map(|v| {
                let n = as_int(v, &format!("{what}.{k}"))?;
                i32::try_from(n)
                    .map_err(|_| Error::new(format!("{what}.{k} = {n} is out of the 32-bit range")))
            })
            .transpose()
    };
    Ok(RoomExpect {
        score: float("score")?,
        light: float("light")?,
        lamp_light: int("lamp_light")?,
        light_sources: int("light_sources")?,
        impact: int("impact")?,
        objects: int("objects")?,
    })
}

fn funds(v: &TomlValue, what: &str) -> Result<i32> {
    let i = as_int(v, what)?;
    // The lot keeps funds in two globals of four decimal digits each (13 and 14).
    if !(0..=99_999_999).contains(&i) {
        return Err(Error::new(format!(
            "{what} = {i} is out of range 0 to 99999999"
        )));
    }
    Ok(i as i32)
}

fn expect(v: &TomlValue, what: &str) -> Result<Expect> {
    let t = as_table(v, what)?;
    check_keys(
        t,
        &[
            "result",
            "error",
            "attrs",
            "temps",
            "data",
            "globals",
            "funds",
            "sim",
            "stack_obj",
            "sounds",
            "dialogs",
            "queue",
            "turns",
            "objects",
            "room",
        ],
        what,
    )?;
    let mut e = Expect::default();
    if let Some(v) = t.get("result") {
        e.result = Some(outcome(v, &format!("{what}.result"))?);
    }
    if let Some(v) = t.get("error") {
        e.error = Some(as_str(v, &format!("{what}.error"))?.to_owned());
        match e.result {
            None => e.result = Some(Outcome::Error),
            Some(Outcome::Error) => {}
            Some(other) => {
                return Err(Error::new(format!(
                    "{what}: 'error' needs result = \"error\", not {other}"
                )));
            }
        }
    }
    if let Some(v) = t.get("attrs") {
        e.attrs = attr_map(v, &format!("{what}.attrs"))?;
    }
    if let Some(v) = t.get("temps") {
        e.temps = word_map(v, &format!("{what}.temps"))?;
    }
    if let Some(v) = t.get("data") {
        e.data = word_map(v, &format!("{what}.data"))?;
    }
    if let Some(v) = t.get("globals") {
        e.globals = word_map(v, &format!("{what}.globals"))?;
    }
    if let Some(v) = t.get("funds") {
        e.funds = Some(funds(v, &format!("{what}.funds"))?);
    }
    if let Some(v) = t.get("sim") {
        e.sim = Some(sim_state(v, &format!("{what}.sim"))?);
    }
    if let Some(v) = t.get("stack_obj") {
        e.stack_obj = Some(obj_ref(v, &format!("{what}.stack_obj"))?);
    }
    if let Some(v) = t.get("sounds") {
        let mut out = Vec::new();
        for (i, s) in array(v, &format!("{what}.sounds"))?.iter().enumerate() {
            out.push(as_str(s, &format!("{what}.sounds[{i}]"))?.to_owned());
        }
        e.sounds = Some(out);
    }
    if let Some(v) = t.get("dialogs") {
        let w = format!("{what}.dialogs");
        e.dialogs = Some(match v {
            // A count: that many dialogs, of any kind. Kept as kinds of 0xFF ("any").
            TomlValue::Int(_) => {
                let n = as_u16(v, &w)?;
                vec![DialogExpect { kind: 0xFF }; usize::from(n)]
            }
            _ => array(v, &w)?
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    dialog_kind(k, &format!("{w}[{i}]")).map(|kind| DialogExpect { kind })
                })
                .collect::<Result<_>>()?,
        });
    }
    if let Some(v) = t.get("queue") {
        let w = format!("{what}.queue");
        e.queue = Some(match v {
            TomlValue::Int(_) => {
                let n = as_u16(v, &w)?;
                vec![
                    QueueExpect {
                        interaction: None,
                        menu: None,
                        target: None,
                        priority: None,
                    };
                    usize::from(n)
                ]
            }
            _ => array(v, &w)?
                .iter()
                .enumerate()
                .map(|(i, q)| queue_entry(q, &format!("{w}[{i}]")))
                .collect::<Result<_>>()?,
        });
    }
    if let Some(v) = t.get("turns") {
        let n = as_int(v, &format!("{what}.turns"))?;
        e.turns = Some(
            u32::try_from(n)
                .ok()
                .filter(|&n| n <= MAX_TURNS)
                .ok_or_else(|| Error::new(format!("{what}.turns = {n} is out of range")))?,
        );
    }
    if let Some(v) = t.get("objects") {
        e.objects = objects_expect(v, &format!("{what}.objects"))?;
    }
    if let Some(v) = t.get("room") {
        e.room = Some(room_expect(v, &format!("{what}.room"))?);
    }
    Ok(e)
}

fn tree_ref(v: &TomlValue, what: &str) -> Result<TreeRef> {
    match v {
        TomlValue::Int(_) => Ok(TreeRef::Id(as_u16(v, what)?)),
        TomlValue::String(s) if !s.is_empty() => Ok(TreeRef::Label(s.clone())),
        other => Err(Error::new(format!(
            "{what} must be a BHAV ID or a non-empty label, not {}",
            kind_of(other)
        ))),
    }
}

fn case(t: &Table, index: usize) -> Result<TestCase> {
    let name = match t.get("name") {
        Some(v) => as_str(v, &format!("[[test]] #{}.name", index + 1))?.to_owned(),
        None => format!("test #{}", index + 1),
    };
    let what = format!("test '{name}'");
    check_keys(
        t,
        &[
            "name",
            "tree",
            "guard",
            "action",
            "interaction",
            "me",
            "stack_obj",
            "params",
            "turns",
            "budget",
            "answers",
            "room",
            "given",
            "expect",
        ],
        &what,
    )?;
    let room = match t.get("room") {
        None => None,
        Some(TomlValue::Bool(b)) => Some(*b),
        Some(other) => {
            return Err(Error::new(format!(
                "{what}: room must be a boolean, not {}",
                kind_of(other)
            )));
        }
    };
    let mut targets = Vec::new();
    if let Some(v) = t.get("tree") {
        targets.push(Target::Tree(tree_ref(v, &format!("{what}: tree"))?));
    }
    for (key, make) in [
        ("guard", Target::Guard as fn(String) -> Target),
        ("action", Target::Action),
        ("interaction", Target::Interaction),
    ] {
        if let Some(v) = t.get(key) {
            let s = as_str(v, &format!("{what}: {key}"))?;
            if s.is_empty() {
                return Err(Error::new(format!("{what}: {key} must not be empty")));
            }
            targets.push(make(s.to_owned()));
        }
    }
    let target = match targets.len() {
        1 => targets.remove(0),
        0 => {
            return Err(Error::new(format!(
                "{what}: give one of 'tree', 'guard', 'action', or 'interaction'"
            )));
        }
        _ => {
            return Err(Error::new(format!(
                "{what}: give only one of 'tree', 'guard', 'action', or 'interaction'"
            )));
        }
    };
    let me = t
        .get("me")
        .map(|v| match as_str(v, &format!("{what}: me"))? {
            "object" | "self" => Ok(Actor::Object),
            "sim" => Ok(Actor::Sim),
            other => Err(Error::new(format!(
                "{what}: me = '{other}' (expected \"object\" or \"sim\")"
            ))),
        })
        .transpose()?;
    let stack_obj = t
        .get("stack_obj")
        .map(|v| obj_ref(v, &format!("{what}: stack_obj")))
        .transpose()?;
    let params = match t.get("params") {
        Some(v) => array(v, &format!("{what}: params"))?
            .iter()
            .enumerate()
            .map(|(i, p)| as_i16(p, &format!("{what}: params[{i}]")))
            .collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let turns = t
        .get("turns")
        .map(|v| {
            let n = as_int(v, &format!("{what}: turns"))?;
            u32::try_from(n)
                .ok()
                .filter(|&n| (1..=MAX_TURNS).contains(&n))
                .ok_or_else(|| {
                    Error::new(format!(
                        "{what}: turns = {n} is out of range 1 to {MAX_TURNS}"
                    ))
                })
        })
        .transpose()?;
    let budget = match t.get("budget") {
        Some(v) => {
            let n = as_int(v, &format!("{what}: budget"))?;
            usize::try_from(n)
                .ok()
                .filter(|&n| (1..=10_000_000).contains(&n))
                .ok_or_else(|| {
                    Error::new(format!(
                        "{what}: budget = {n} is out of range 1 to 10000000"
                    ))
                })?
        }
        None => DEFAULT_BUDGET,
    };
    let answers = match t.get("answers") {
        Some(v) => array(v, &format!("{what}: answers"))?
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let n = as_int(a, &format!("{what}: answers[{i}]"))?;
                u8::try_from(n)
                    .ok()
                    .filter(|&n| (1..=3).contains(&n))
                    .ok_or_else(|| {
                        Error::new(format!(
                            "{what}: answers[{i}] = {n} is not a button (1, 2, or 3)"
                        ))
                    })
            })
            .collect::<Result<_>>()?,
        None => Vec::new(),
    };
    let given = match t.get("given") {
        Some(v) => given(v, &format!("{what}: given"))?,
        None => Given::default(),
    };
    let expect = match t.get("expect") {
        Some(v) => expect(v, &format!("{what}: expect"))?,
        None => Expect::default(),
    };
    if matches!(target, Target::Interaction(_)) {
        if me.is_some() || stack_obj.is_some() || !params.is_empty() {
            return Err(Error::new(format!(
                "{what}: an interaction runs on the Sim's thread as picked from the pie menu; \
                 'me', 'stack_obj', and 'params' do not apply"
            )));
        }
        if matches!(expect.result, Some(Outcome::True | Outcome::False)) {
            return Err(Error::new(format!(
                "{what}: an interaction's return value is not observable; expect result \
                 \"ok\", \"error\", \"yield\", or \"unavailable\""
            )));
        }
        if expect.stack_obj.is_some() {
            return Err(Error::new(format!(
                "{what}: expect.stack_obj applies to tree runs only"
            )));
        }
    } else if expect.result == Some(Outcome::Unavailable) {
        return Err(Error::new(format!(
            "{what}: result \"unavailable\" applies to interactions only"
        )));
    }
    Ok(TestCase {
        name,
        target,
        me,
        stack_obj,
        params,
        turns,
        budget,
        answers,
        room,
        given,
        expect,
    })
}

/// An object table's `idle_main` key (false when absent).
fn idle_main_key(t: &Table, what: &str) -> Result<bool> {
    match t.get("idle_main") {
        None => Ok(false),
        Some(TomlValue::Bool(b)) => Ok(*b),
        Some(other) => Err(Error::new(format!(
            "{what}.idle_main must be a boolean, not {}",
            kind_of(other)
        ))),
    }
}

/// An object source table; `extra` are further keys the caller reads itself.
fn source(t: &Table, base: &Path, what: &str, extra: &[&str]) -> Result<Source> {
    let mut allowed = vec!["antics", "iff", "pack"];
    allowed.extend_from_slice(extra);
    check_keys(t, &allowed, what)?;
    let pack = t
        .get("pack")
        .map(|v| as_str(v, &format!("{what}.pack")).map(|p| base.join(p)))
        .transpose()?;
    match (t.get("antics"), t.get("iff")) {
        (Some(a), None) => Ok(Source::Antics {
            path: base.join(as_str(a, &format!("{what}.antics"))?),
            pack,
        }),
        (None, Some(i)) => {
            if pack.is_some() {
                return Err(Error::new(format!(
                    "{what}: 'pack' applies to an 'antics' source only"
                )));
            }
            Ok(Source::Iff {
                path: base.join(as_str(i, &format!("{what}.iff"))?),
            })
        }
        (Some(_), Some(_)) => Err(Error::new(format!(
            "{what}: give 'antics' or 'iff', not both"
        ))),
        (None, None) => Err(Error::new(format!("{what}: give 'antics' or 'iff'"))),
    }
}

fn stub(t: &Table, base: &Path, index: usize) -> Result<Stub> {
    let what = format!("[[stub]] #{}", index + 1);
    check_keys(
        t,
        &["id", "label", "args", "returns", "source", "file"],
        &what,
    )?;
    let has_returns = t.contains_key("returns");
    let sources = usize::from(t.contains_key("source")) + usize::from(t.contains_key("file"));
    match (has_returns, sources) {
        (true, 0) => {
            let id_v = t
                .get("id")
                .ok_or_else(|| Error::new(format!("{what}: 'returns' needs an 'id'")))?;
            let id = as_u16(id_v, &format!("{what}.id"))?;
            if !stub_range(id) {
                return Err(Error::new(format!(
                    "{what}: tree {id} is not a global (256 to 4095) or semi-global \
                     (8192 and up) ID"
                )));
            }
            let result = match t.get("returns") {
                Some(TomlValue::Bool(b)) => *b,
                Some(other) => {
                    return Err(Error::new(format!(
                        "{what}.returns must be a boolean, not {}",
                        kind_of(other)
                    )));
                }
                None => unreachable!("checked above"),
            };
            let label = match t.get("label") {
                Some(v) => as_str(v, &format!("{what}.label"))?.to_owned(),
                None => format!("Stub{id}"),
            };
            if label.contains(['"', '\\', '\n']) {
                return Err(Error::new(format!(
                    "{what}.label must not hold quotes, backslashes, or newlines"
                )));
            }
            let args = match t.get("args") {
                Some(v) => {
                    let n = as_int(v, &format!("{what}.args"))?;
                    u8::try_from(n).ok().filter(|&n| n <= 4).ok_or_else(|| {
                        Error::new(format!("{what}.args = {n} is out of range 0 to 4"))
                    })?
                }
                None => 0,
            };
            Ok(Stub::Returns {
                id,
                label,
                args,
                result,
            })
        }
        (false, 1) => {
            for k in ["id", "label", "args"] {
                if t.contains_key(k) {
                    return Err(Error::new(format!(
                        "{what}: '{k}' applies to a 'returns' stub; a source stub names its \
                         trees itself"
                    )));
                }
            }
            if let Some(v) = t.get("source") {
                let text = match v {
                    TomlValue::String(s) => s.clone(),
                    TomlValue::Array(lines) => {
                        let mut text = String::new();
                        for (i, l) in lines.iter().enumerate() {
                            text.push_str(as_str(l, &format!("{what}.source[{i}]"))?);
                            text.push('\n');
                        }
                        text
                    }
                    other => {
                        return Err(Error::new(format!(
                            "{what}.source must be a string or an array of lines, not {}",
                            kind_of(other)
                        )));
                    }
                };
                Ok(Stub::Source {
                    origin: format!("{what}.source"),
                    text,
                })
            } else {
                let rel = as_str(&t["file"], &format!("{what}.file"))?;
                let path = base.join(rel);
                let text =
                    read_regular_file(&path).map_err(|e| Error::new(format!("{what}: {e}")))?;
                Ok(Stub::Source {
                    origin: path.display().to_string(),
                    text,
                })
            }
        }
        _ => Err(Error::new(format!(
            "{what}: give exactly one of 'returns' (with 'id'), 'source', or 'file'"
        ))),
    }
}

/// Whether a stub may hold tree `id`: the global and semi-global ranges only, since the
/// subject's own file holds 4096 to 8191.
#[must_use]
pub fn stub_range(id: u16) -> bool {
    (256..4096).contains(&id) || id >= 8192
}

impl TestFile {
    /// Parses a test file. Relative paths in it resolve against `base` (the file's
    /// directory).
    pub fn parse(source: &str, base: &Path) -> Result<Self> {
        let root = parse_toml(source)?;
        check_keys(
            &root,
            &["subject", "lot", "stub", "sim", "test"],
            "the test file",
        )?;
        let subject_table = as_table(
            root.get("subject")
                .ok_or_else(|| Error::new("the test file needs a [subject] table"))?,
            "[subject]",
        )?;
        let subject = self::source(subject_table, base, "[subject]", &["idle_main", "place"])?;
        let idle_main = idle_main_key(subject_table, "[subject]")?;
        let subject_place = subject_table
            .get("place")
            .map(|v| tile(v, "[subject].place"))
            .transpose()?;
        let mut also = Vec::new();
        let mut also_idle_main = BTreeSet::new();
        let mut room = false;
        if let Some(lot) = root.get("lot") {
            let t = as_table(lot, "[lot]")?;
            check_keys(t, &["also", "room"], "[lot]")?;
            match t.get("room") {
                None => {}
                Some(TomlValue::Bool(b)) => room = *b,
                Some(other) => {
                    return Err(Error::new(format!(
                        "[lot].room must be a boolean, not {}",
                        kind_of(other)
                    )));
                }
            }
            if let Some(v) = t.get("also") {
                for (i, item) in array(v, "[lot].also")?.iter().enumerate() {
                    let w = format!("[lot].also[{i}]");
                    also.push(match item {
                        // A bare path: an `.antics` source with the subject's pack, or an `.iff`.
                        TomlValue::String(p) => {
                            let path = base.join(p);
                            if p.to_ascii_lowercase().ends_with(".iff") {
                                Source::Iff { path }
                            } else {
                                let pack = match &subject {
                                    Source::Antics { pack, .. } => pack.clone(),
                                    Source::Iff { .. } => None,
                                };
                                Source::Antics { path, pack }
                            }
                        }
                        TomlValue::Table(t) => {
                            let s = self::source(t, base, &w, &["idle_main"])?;
                            if idle_main_key(t, &w)? {
                                also_idle_main.insert(s.slug());
                            }
                            s
                        }
                        other => {
                            return Err(Error::new(format!(
                                "{w} must be a path or a table, not {}",
                                kind_of(other)
                            )));
                        }
                    });
                }
            }
        }
        let mut slugs = BTreeMap::new();
        for s in std::iter::once(&subject).chain(&also) {
            if let Some(prev) = slugs.insert(s.slug(), s.path().to_path_buf()) {
                return Err(Error::new(format!(
                    "two objects share the name '{}' ({} and {})",
                    s.slug(),
                    prev.display(),
                    s.path().display()
                )));
            }
        }
        let mut stubs = Vec::new();
        if let Some(v) = root.get("stub") {
            for (i, item) in array(v, "[[stub]]")?.iter().enumerate() {
                stubs.push(stub(
                    as_table(item, &format!("[[stub]] #{}", i + 1))?,
                    base,
                    i,
                )?);
            }
        }
        let sim = root.get("sim").map(|v| sim_state(v, "[sim]")).transpose()?;
        let mut cases = Vec::new();
        if let Some(v) = root.get("test") {
            let items = match v {
                TomlValue::Array(a) => a,
                _ => {
                    return Err(Error::new(
                        "'test' must be declared as an array of tables '[[test]]'",
                    ));
                }
            };
            for (i, item) in items.iter().enumerate() {
                cases.push(case(as_table(item, &format!("[[test]] #{}", i + 1))?, i)?);
            }
        }
        if cases.is_empty() {
            return Err(Error::new("the test file has no [[test]] cases"));
        }
        let mut names = BTreeMap::new();
        for c in &cases {
            if names.insert(c.name.clone(), ()).is_some() {
                return Err(Error::new(format!("two tests are named '{}'", c.name)));
            }
        }
        let file = Self {
            subject,
            idle_main,
            also,
            also_idle_main,
            stubs,
            sim,
            room,
            subject_place,
            cases,
        };
        if file.subject_place.is_some() && !file.cases.iter().any(|c| c.in_room(&file)) {
            return Err(Error::new(
                "[subject].place needs a room to stand in ([lot] room = true, or room = true \
                 on a case)",
            ));
        }
        for c in &file.cases {
            let what = format!("test '{}'", c.name);
            if c.in_room(&file) {
                c.tiles(&file)?;
            } else if let Some(i) = c.given.create.iter().position(|c| c.place.is_some()) {
                return Err(Error::new(format!(
                    "{what}: given.create[{i}].place needs a room to stand in ([lot] room = \
                     true, or room = true on the case)"
                )));
            } else if c.expect.room.is_some() {
                return Err(Error::new(format!(
                    "{what}: expect.room needs a counted room ([lot] room = true, or room = \
                     true on the case)"
                )));
            }
            let has_sim = c.has_sim(&file);
            if matches!(c.target, Target::Interaction(_)) && !has_sim {
                return Err(Error::new(format!(
                    "{what}: an interaction needs a Sim ([sim] or given.sim)"
                )));
            }
            if c.me == Some(Actor::Sim) && !has_sim {
                return Err(Error::new(format!(
                    "{what}: me = \"sim\" needs a Sim ([sim] or given.sim)"
                )));
            }
            if (c.stack_obj == Some(ObjRef::Sim)
                || c.expect.stack_obj == Some(ObjRef::Sim)
                || c.expect.sim.is_some()
                || c.expect.queue.is_some())
                && !has_sim
            {
                return Err(Error::new(format!(
                    "{what}: refers to the Sim, but there is none ([sim] or given.sim)"
                )));
            }
        }
        Ok(file)
    }

    /// Reads and parses a test file from disk.
    pub fn read(path: &Path) -> Result<Self> {
        let text = read_regular_file(path)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Self::parse(&text, base).map_err(|e| e.with_context(path.display()))
    }
}

/// Opens a file a spec names for reading, refusing anything but a regular file: a FIFO or a
/// device would block the run or never end. The path is checked first, then opened (without
/// blocking, `O_NONBLOCK`, on macOS, iOS and the BSDs, and on Linux and Android for x86,
/// x86-64, ARM, AArch64 and RISC-V 64, the targets whose flag value is set below, as the v2
/// loader's `read_regular_file_checked` does, so a FIFO swapped in after the check cannot
/// block the open; elsewhere such a swap can still block it) and the opened handle is
/// checked again, so what is read is what was checked.
pub(crate) fn open_regular_file(path: &Path) -> Result<std::fs::File> {
    let meta = std::fs::metadata(path)
        .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(not_regular(path));
    }
    open_checked(path)
}

fn not_regular(path: &Path) -> Error {
    Error::new(format!(
        "cannot read {}: not a regular file",
        path.display()
    ))
}

/// The open half of [`open_regular_file`]: opens without blocking where the platform allows
/// it and checks the opened handle is a regular file.
pub(crate) fn open_checked(path: &Path) -> Result<std::fs::File> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0004;
        opts.custom_flags(O_NONBLOCK);
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "x86",
            target_arch = "x86_64",
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "riscv64"
        )
    ))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NONBLOCK: i32 = 0x0800;
        opts.custom_flags(O_NONBLOCK);
    }
    let file = opts
        .open(path)
        .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    let meta = file
        .metadata()
        .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(not_regular(path));
    }
    Ok(file)
}

/// Reads a text file a spec names through [`open_regular_file`].
fn read_regular_file(path: &Path) -> Result<String> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut open_regular_file(path)?, &mut text)
        .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    Ok(text)
}

// ---------------------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------------------

/// One case's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseReport {
    pub name: String,
    /// What the case assumed that the game would not do: one line each (`subject Main (OBJD
    /// #128, GUID 0x7f000123) replaced by an idle loop (idle_main)`), for what the run did
    /// (an `idle_main` definition whose Main it replaced). Its PASS holds only under them; the
    /// CLI prints them as `ASSUME` lines before the case's own.
    pub assumptions: Vec<String>,
    /// One line per mismatch (`field: expected X, got Y`); empty when the case passed.
    pub failures: Vec<String>,
}

impl CaseReport {
    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// One file's results, case by case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: PathBuf,
    pub cases: Vec<CaseReport>,
}

impl FileReport {
    #[must_use]
    pub fn passed(&self) -> usize {
        self.cases.iter().filter(|c| c.passed()).count()
    }

    #[must_use]
    pub fn failed(&self) -> usize {
        self.cases.len() - self.passed()
    }
}

/// Runs every case of a parsed test file in its own fresh lot. An error is a problem with
/// the file or its subject (a compile error, an unknown tree, an attribute name the subject
/// does not have) rather than a failing case.
#[cfg(feature = "vm-replay")]
pub fn run_test_file(file: &TestFile, path: &Path) -> Result<FileReport> {
    harness::run(file, path)
}

/// Without `vm-replay` there is no VM to run cases in.
#[cfg(not(feature = "vm-replay"))]
pub fn run_test_file(_file: &TestFile, path: &Path) -> Result<FileReport> {
    Err(Error::new(format!(
        "{}: running tests needs sims-vm; build sims-antics with the `vm-replay` feature",
        path.display()
    )))
}

/// Whether this build can run tests (the `vm-replay` feature).
#[must_use]
pub const fn can_run() -> bool {
    cfg!(feature = "vm-replay")
}

/// The test files under `path`: the file itself, or every `*.test.toml` below a directory,
/// sorted by path.
// The listing is sorted before use, so its order never depends on the filesystem.
#[allow(clippy::disallowed_methods)]
pub fn collect_test_files(path: &Path) -> Result<Vec<PathBuf>> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| Error::new(format!("cannot access {}: {e}", path.display())))?;
    if meta.file_type().is_symlink() {
        return Err(Error::new(format!("refusing symlink {}", path.display())));
    }
    if meta.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !meta.is_dir() {
        return Err(not_regular(path));
    }
    let mut out = Vec::new();
    let mut dirs = vec![path.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| Error::new(format!("cannot read {}: {e}", dir.display())))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| Error::new(format!("cannot read {}: {e}", dir.display())))?;
            let ty = entry
                .file_type()
                .map_err(|e| Error::new(format!("cannot read {}: {e}", dir.display())))?;
            let p = entry.path();
            if ty.is_dir() {
                dirs.push(p);
            } else if ty.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".test.toml"))
            {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests;
