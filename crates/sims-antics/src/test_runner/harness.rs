//! The `sims-vm` side of [`super`]: builds a lot for a test file and runs its cases.
//!
//! Every file gets one linked program: the subject's (and `[lot].also`'s) compiled files,
//! a `Global.iff` holding the global stubs, a semi-global file when there are semi-global
//! stubs, and a synthetic Sim type. The object files and the stub files are loaded through
//! `sims_content::Index` (as in-memory files), `sims_content::Catalog::build`, and
//! `sims_vm::load_library`: the same registration and loading the game's install goes
//! through. Every case then gets a fresh world, built like the `open-pack` behavioural
//! suite's (`tests/integration/open_pack_behavioral.rs`): a flat 64-by-64 lot with one indoor room,
//! the Sim (if any) as object 1, and the subject made by the VM's own factory
//! (`Interpreter::create`, which runs its Init). With `[lot] room = true` the room is counted
//! and the objects the case makes stand on its tiles, as that suite's placed-room tests set
//! them up ([`place_on_tile`]).

use super::{
    Actor, AttrKey, CaseReport, CreateRef, DEFAULT_BUDGET, Expect, FileReport, ObjRef, Outcome,
    ROOM_TILES, RoomExpect, SimState, Source, Stub, Target, TestCase, TestFile, TreeRef,
    dialog_kind_name,
};
use crate::compile::compile_antics;
use crate::encode::{encode_bcon, encode_bhav, encode_iff_with_rsmp};
use crate::error::{Error, Result};
use crate::ir::IFF_CONVERTED_FLAG;
use sims_content::{ArchivePool, Catalog, Index};
use sims_formats::behavior::Behavior;
use sims_formats::iff::{ChunkData, Iff};
use sims_formats::lot::Array as LotArray;
use sims_formats::saved_objects::{Frame, Interaction as SavedInteraction, Person, Slot};
use sims_formats::strings::StringTable;
use sims_sim::motives::{Rates, Tuning};
use sims_sim::rooms::Room;
use sims_sim::{Object, World};
use sims_vm::orders::MenuEntry;
use sims_vm::{
    Answers, DEFINITION_WORDS, Interpreter, Library, Program, SoundEvent, Status, Stream,
    TICK_BUDGET, TickError, Trap, Update,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The synthetic Sim type's GUID. Not an ID any `open-pack` object uses (they are
/// `0x7F00_xxxx`).
pub(super) const SIM_GUID: u32 = 0x7E57_0051;
/// The Sim's object ID: it is put on the lot first.
const SIM_ID: u16 = 1;
/// The Sim's type ID in the world's type table.
const SIM_TYPE: u16 = 1;
/// The Sim's main tree: an endless Idle For Input loop, as a person's main runs the
/// interactions queued on it.
const SIM_MAIN: u16 = 4097;
const SIM_MAIN_SOURCE: &str = r#"
tree 4097 "TestSimMain" (args: 1) {
    while 0 == 0 {
        param[0] := 1;
        idle_for_input(param[0], 1);
    }
}
"#;
/// The idle loop `idle_main` puts in place of a Main that returns: `loop { sleep(1); }`.
/// It is the only tree of a library of its own ([`IDLE_GUID`]'s), so no call in the lot's
/// code reaches it: a call resolves a private tree in the caller's own file, and nothing
/// names this file but the replaced frames.
const IDLE_MAIN: u16 = 4096;
const IDLE_MAIN_SOURCE: &str = r#"
tree 4096 "TestRunnerIdleMain" (args: 1) {
    while 0 == 0 {
        param[0] := 1;
        sleep(param[0]);
    }
}
"#;
/// The GUID of the idle loop's owner: a definition with no objects, which the frames that
/// run the loop name as their code's owner. Not an ID any `open-pack` object uses.
pub(super) const IDLE_GUID: u32 = 0x7E57_1D1E;
/// The idle loop owner's type ID in the world's type table. The factory gives a new type
/// the lowest unused ID, so it reaches this one only after 32,766 other types, far more
/// than a test lot has.
const IDLE_TYPE: u16 = i16::MAX as u16;
/// The assumption `idle_main` makes for one object definition whose Main a case replaced,
/// reported with that case.
pub(super) fn idle_main_assumption(owner: &str, chunk: u16, guid: u32) -> String {
    format!("{owner} Main (OBJD #{chunk}, GUID {guid:#010x}) replaced by an idle loop (idle_main)")
}
/// Turns an interaction may take when the case does not say: as many as run before the
/// Sim's first motive decay, which stops a run ([`real_turn`]).
const DEFAULT_INTERACTION_TURNS: u32 = 60;
/// The TTAB entry flag that runs an action at once, inside the pick (0x45d235).
const AT_ONCE: u32 = 4;
/// The TTAB entry flag of a debug entry, which the menu shows with "*" (0x41f167).
const DEBUG: u32 = 0x80;
/// The language the menu's TTAs strings are read in (`sims-game`'s menu).
const LANGUAGE: u8 = 1;
/// An action's `join` for the offer to join a running interaction (0x41efc1).
const JOINS: i32 = 1;
/// The most entry arguments a tree call takes (`sims-vm`'s `call_as`).
const MAX_ENTRY_ARGS: usize = 255;
/// The motives' names, for messages.
const MOTIVE_NAMES: [(u8, &str); 10] = [
    (3, "mood"),
    (5, "energy"),
    (6, "comfort"),
    (7, "hunger"),
    (8, "hygiene"),
    (9, "bladder"),
    (11, "alertness"),
    (13, "room"),
    (14, "social"),
    (15, "fun"),
];

/// One compiled or read object file.
struct ObjectFile {
    slug: String,
    bytes: Vec<u8>,
    /// The GUID `given.create` and the subject use: the compiled object's own, or an
    /// `.iff`'s first definition in resource order.
    guid: Option<u32>,
    /// `.antics` state names, from the compiled `.air` header.
    attr_names: BTreeMap<String, u16>,
}

/// What every case of a file shares.
struct Lot {
    program: Program,
    catalog: Catalog,
    subject_guid: u32,
    subject_resource: usize,
    subject_attr_names: BTreeMap<String, u16>,
    /// Every loaded object's state names, by slug, for `expect.objects`.
    attr_names: BTreeMap<String, BTreeMap<String, u16>>,
    /// Slug to GUID, for `given.create`.
    slugs: BTreeMap<String, u32>,
    /// Interaction names by object definition (resource and OBJD chunk) and menu number
    /// (TTAB entry string index). Each definition names its own table: two definitions of
    /// one file may give one menu number different names.
    menus: BTreeMap<(usize, u16), BTreeMap<u32, String>>,
    /// What the game's pie menu shows for each entry without a name of its own, by object
    /// definition and menu number ([`EntryLabel`]).
    labels: BTreeMap<(usize, u16), BTreeMap<u32, EntryLabel>>,
    /// The subject's definition and the tiles of its group: the definitions its pie menu
    /// can offer entries of.
    subject_group: Vec<(usize, u16)>,
    /// The definitions given `idle_main` (the subject's, `[lot].also`'s, each with the tiles
    /// of its group), by resource and OBJD chunk, with the object they belong to.
    idle_defs: BTreeMap<(usize, u16), IdleDef>,
    /// The idle loop's frame: its arguments and locals (`Behavior::frame_dimensions`).
    idle_frame: (usize, usize),
}

/// A definition given `idle_main`.
struct IdleDef {
    /// "subject", or the `[lot].also` object's name.
    owner: String,
    guid: u32,
}

/// Behaviors of a module compiled with [`compile_antics`], as a library holds them.
fn library_behaviors(source: &str) -> Result<Vec<(u16, Behavior, String)>> {
    let module = compile_antics(source)?;
    Ok(module
        .behaviors
        .iter()
        .map(|(&id, bhav)| {
            (
                id,
                Behavior {
                    format: bhav.format,
                    kind: bhav.kind,
                    args: bhav.args,
                    locals: bhav.locals,
                    revision: bhav.revision,
                    instructions: bhav.instructions.clone(),
                },
                bhav.label.clone(),
            )
        })
        .collect())
}

/// A TTAB entry as the game's pie menu names it when its check gave it no name
/// (`sims-game`'s `menu::entry_label`, 0x41f167).
#[derive(Debug, Clone)]
struct EntryLabel {
    /// Its TTAs string, if the file has the table's TTAs and the string is not blank.
    label: Option<String>,
    /// Whether the table's TTAs loaded at all (the offer to join then carries its stored
    /// name, 0x475bf6).
    strings: bool,
    /// TTAB flag 0x80 (`debug`).
    debug: bool,
}

impl EntryLabel {
    /// The name shown: the label, or (a blank one too) "Interaction" and the menu number,
    /// with "*" before it for a debug entry unless the action carries the entry's stored
    /// name, as the offer to join does (the getter adds the "*" only to an empty name).
    fn shown(&self, menu: u32, joins: bool) -> String {
        let name = self
            .label
            .clone()
            .unwrap_or_else(|| format!("Interaction {menu}"));
        if self.debug && !(joins && self.strings) {
            format!("*{name}")
        } else {
            name
        }
    }
}

pub(super) fn run(file: &TestFile, path: &Path) -> Result<FileReport> {
    let lot = Lot::build(file).map_err(|e| e.with_context(path.display()))?;
    let cases = file
        .cases
        .iter()
        .map(|case| {
            let ((result, mut assumptions), own_thread) = on_case_thread(|| {
                let mut assumptions = Vec::new();
                (run_case(&lot, file, case, &mut assumptions), assumptions)
            });
            if !own_thread {
                assumptions.push(format!(
                    "ran on the caller's stack: no thread with a {} GiB stack could be made, \
                     so a Main that creates its own kind may overflow it",
                    CASE_STACK >> 30
                ));
            }
            let failures = match result {
                Ok(failures) => failures,
                Err(e) => vec![format!("setup: {e}")],
            };
            CaseReport {
                name: case.name.clone(),
                assumptions,
                failures: failures.into_iter().map(elide).collect(),
            }
        })
        .collect();
    Ok(FileReport {
        path: path.to_path_buf(),
        cases,
    })
}

/// The stack a case runs on. `sims-vm`'s factory runs a new object's Main for its first
/// turn inside the creation, so a Main (or Init) that makes another object of its own kind
/// nests creations until the lot runs out of object IDs (32,767), and the factory has no
/// depth limit of its own. That takes about 0.5 GiB of stack in a release build and 1 GiB
/// in a debug one: the case then fails on the factory's "no unused object ID" rather than
/// aborting the process on a stack overflow. Only the pages used are committed.
const CASE_STACK: usize = 2 << 30;

/// Runs `f` on a thread with [`CASE_STACK`] of stack, or on this one if no such thread can
/// be made; the flag says whether it got its own thread. A panic in `f` is the caller's.
fn on_case_thread<T: Send>(f: impl FnOnce() -> T + Send) -> (T, bool) {
    let mut f = Some(f);
    let run = std::thread::scope(|scope| {
        let thread = std::thread::Builder::new()
            .name("sims-antics-test-case".to_owned())
            .stack_size(CASE_STACK)
            .spawn_scoped(scope, || (f.take().expect("taken once"))());
        thread.ok().map(|t| match t.join() {
            Ok(v) => v,
            Err(panic) => std::panic::resume_unwind(panic),
        })
    });
    match run {
        Some(v) => (v, true),
        None => ((f.take().expect("the thread did not start"))(), false),
    }
}

/// The longest failure line reported in full; a longer one (a fault nested through
/// thousands of creations) keeps its start and end.
const MAX_LINE: usize = 4000;

fn elide(line: String) -> String {
    let chars = line.chars().count();
    if chars <= MAX_LINE {
        return line;
    }
    let keep = MAX_LINE / 2;
    let head: String = line.chars().take(keep).collect();
    let tail: String = line.chars().skip(chars - keep).collect();
    format!(
        "{head} [... {} characters elided ...] {tail}",
        chars - 2 * keep
    )
}

// ---------------------------------------------------------------------------------------
// The program
// ---------------------------------------------------------------------------------------

fn compile_source(source: &Source) -> Result<ObjectFile> {
    let slug = source.slug();
    match source {
        Source::Antics { path, pack } => {
            // Refused before the compiler sees it (the v2 loader opens its files
            // nonblocking and checks them again).
            drop(super::open_regular_file(path)?);
            let compiled = crate::v2::compile_v2_object_file(path, pack.as_deref())
                .map_err(|e| e.with_context(path.display()))?;
            let air = crate::v2::expand_v2_to_air(path, pack.as_deref())
                .map_err(|e| e.with_context(path.display()))?;
            Ok(ObjectFile {
                slug,
                bytes: compiled.iff_bytes,
                guid: Some(compiled.guid),
                attr_names: attribute_names(&air),
            })
        }
        Source::Iff { path } => {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut super::open_regular_file(path)?, &mut bytes)
                .map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
            Ok(ObjectFile {
                slug,
                bytes,
                guid: None,
                attr_names: BTreeMap::new(),
            })
        }
    }
}

/// The state names an `.antics` v2 expansion lists in its header
/// (`// Attributes: attr[0] = occupied, attr[1] = power`).
pub(super) fn attribute_names(air: &str) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    for line in air.lines().take(8) {
        let Some(rest) = line.trim().strip_prefix("// Attributes:") else {
            continue;
        };
        for item in rest.split(',') {
            let Some((lhs, name)) = item.split_once('=') else {
                continue;
            };
            let Some(idx) = lhs
                .trim()
                .strip_prefix("attr[")
                .and_then(|s| s.strip_suffix(']'))
                .and_then(|s| s.parse::<u16>().ok())
            else {
                continue;
            };
            out.insert(name.trim().to_owned(), idx);
        }
    }
    out
}

/// The stub files' chunks: global trees and constants, and semi-global ones.
fn stub_chunks(stubs: &[Stub]) -> Result<(Vec<ChunkData>, Vec<ChunkData>)> {
    let mut global = Vec::new();
    let mut semi = Vec::new();
    let mut seen = BTreeSet::new();
    for stub in stubs {
        let (origin, text) = match stub {
            Stub::Returns {
                id,
                label,
                args,
                result,
            } => (
                format!("stub {id}"),
                format!("tree {id} \"{label}\" (args: {args}) {{\n    return {result};\n}}\n"),
            ),
            Stub::Source { origin, text } => (origin.clone(), text.clone()),
        };
        let module = compile_antics(&text).map_err(|e| e.with_context(&origin))?;
        for bhav in module.behaviors.values() {
            if !super::stub_range(bhav.id) {
                return Err(Error::new(format!(
                    "{origin}: tree {} is not a global (256 to 4095) or semi-global (8192 and \
                     up) ID; the subject's own file holds 4096 to 8191",
                    bhav.id
                )));
            }
            if !seen.insert((*b"BHAV", bhav.id)) {
                return Err(Error::new(format!(
                    "{origin}: stub tree {} is defined twice",
                    bhav.id
                )));
            }
            let chunk = ChunkData {
                tag: *b"BHAV",
                id: bhav.id,
                flags: IFF_CONVERTED_FLAG,
                label: bhav.label.clone(),
                data: encode_bhav(bhav)?,
            };
            if bhav.id < 4096 {
                global.push(chunk);
            } else {
                semi.push(chunk);
            }
        }
        for bcon in module.constants.values() {
            if !super::stub_range(bcon.id) {
                return Err(Error::new(format!(
                    "{origin}: constants {} are not a global (256 to 4095) or semi-global \
                     (8192 and up) BCON ID",
                    bcon.id
                )));
            }
            if !seen.insert((*b"BCON", bcon.id)) {
                return Err(Error::new(format!(
                    "{origin}: stub constants {} are defined twice",
                    bcon.id
                )));
            }
            let chunk = ChunkData {
                tag: *b"BCON",
                id: bcon.id,
                flags: IFF_CONVERTED_FLAG,
                label: bcon.label.clone(),
                data: encode_bcon(bcon)?,
            };
            if bcon.id < 4096 {
                global.push(chunk);
            } else {
                semi.push(chunk);
            }
        }
    }
    Ok((global, semi))
}

fn content_error(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}

impl Lot {
    fn build(file: &TestFile) -> Result<Self> {
        let subject = compile_source(&file.subject)?;
        let also = file
            .also
            .iter()
            .map(compile_source)
            .collect::<Result<Vec<_>>>()?;
        let (global_chunks, semi_chunks) = stub_chunks(&file.stubs)?;

        // The files, as the loader sees them: Global.iff at the top, objects in a folder.
        const GLOBAL_FILE: &str = "Global.iff";
        const SEMI_FILE: &str = "Stubs/TestSemiGlobals.iff";
        let object_file = |slug: &str| format!("Objects/{slug}.iff");
        let mut files = BTreeMap::new();
        files.insert(
            GLOBAL_FILE.to_owned(),
            encode_iff_with_rsmp(&global_chunks)?,
        );
        if !semi_chunks.is_empty() {
            files.insert(SEMI_FILE.to_owned(), encode_iff_with_rsmp(&semi_chunks)?);
        }
        for o in std::iter::once(&subject).chain(&also) {
            files.insert(object_file(&o.slug), o.bytes.clone());
        }
        let mut index = Index {
            root: std::path::PathBuf::from("sims-antics-test"),
            mounts: Vec::new(),
            resources: Vec::new(),
            diagnostics: Vec::new(),
        };
        index.add_character_files(&files).map_err(content_error)?;
        let resource_of = |name: &str| {
            index
                .resources
                .iter()
                .position(|r| r.member.is_none() && r.path.ends_with(name))
                .ok_or_else(|| Error::new(format!("the test lot lost its file {name}")))
        };
        let mut catalog = Catalog::build(&index).map_err(content_error)?;
        let global = catalog.global;
        let semi_resource = if semi_chunks.is_empty() {
            None
        } else {
            Some(resource_of(SEMI_FILE)?)
        };

        let pool = ArchivePool::new();
        let mut libraries = BTreeMap::new();
        let load = |res: usize, is_global: bool| {
            sims_vm::load_library(&index, res, is_global, &pool).map_err(|e| {
                Error::new(format!(
                    "sims-vm cannot load {}: {e}",
                    index.resources[res].name()
                ))
            })
        };
        libraries.insert(global, load(global, true)?);
        if let Some(res) = semi_resource {
            libraries.insert(res, load(res, false)?);
        }
        let mut semi_globals = BTreeMap::new();
        let mut slugs = BTreeMap::new();
        let mut subject_resource = 0;
        let mut subject_guid = 0;
        // The objects given `idle_main`: resource, GUID, and the name reports give them.
        let mut idle_objects = Vec::new();
        for (n, o) in std::iter::once(&subject).chain(&also).enumerate() {
            let res = resource_of(&object_file(&o.slug))?;
            let lib = load(res, false)?;
            let guid = match o.guid {
                Some(g) => g,
                None => {
                    let first = lib.definition_order.first().ok_or_else(|| {
                        Error::new(format!("{} has no object definition (OBJD)", o.slug))
                    })?;
                    definition_guid(&lib.definitions[first])
                }
            };
            if catalog.object(guid).is_none_or(|c| c.resource != res) {
                return Err(Error::new(format!(
                    "{}: GUID {guid:#010x} does not resolve to this file (another file \
                     registers it too)",
                    o.slug
                )));
            }
            libraries.insert(res, lib);
            // Semi-global stubs stand in for the file's own semi-global link.
            let semi = semi_resource.or_else(|| catalog.semi_globals.get(&res).copied().flatten());
            if let Some(s) = semi
                && !libraries.contains_key(&s)
            {
                libraries.insert(s, load(s, false)?);
            }
            semi_globals.insert(res, semi);
            slugs.insert(o.slug.clone(), guid);
            if if n == 0 {
                file.idle_main
            } else {
                file.also_idle_main.contains(&o.slug)
            } {
                let owner = if n == 0 { "subject" } else { o.slug.as_str() };
                idle_objects.push((res, guid, owner.to_owned()));
            }
            if n == 0 {
                subject_resource = res;
                subject_guid = guid;
            }
        }

        // The Sim's type: a person definition whose main tree waits for interactions.
        for (guid, what) in [(SIM_GUID, "the test Sim"), (IDLE_GUID, "the idle loop")] {
            if catalog.by_guid.contains_key(&guid) {
                return Err(Error::new(format!(
                    "GUID {guid:#010x} is reserved for {what}"
                )));
            }
        }
        let sim_resource = index.resources.len();
        let mut sim_lib = Library::default();
        for (id, behavior, label) in library_behaviors(SIM_MAIN_SOURCE)? {
            sim_lib.behaviors.insert(id, behavior);
            sim_lib.names.insert(id, label);
        }
        let mut words = [0i16; DEFINITION_WORDS];
        words[9] = 2;
        // Definition word 102 bit 2, which keeps the person's animator from starting its
        // posture's base clip (0x590430, `sims-vm`'s `update_base`), as `sims-vm`'s own
        // fixtures' people have it: the test Sim's type has no animations or profile, so its
        // animator ticks with no layer to play.
        words[102] = 4;
        words[14] = SIM_GUID as u16 as i16;
        words[15] = (SIM_GUID >> 16) as u16 as i16;
        sim_lib.definitions.insert(128, words);
        sim_lib.definition_order.push(128);
        sim_lib.callbacks.insert(
            128,
            vec![
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: 0,
                },
                sims_formats::execution::ObjectFunction {
                    condition: 0,
                    action: SIM_MAIN,
                },
            ],
        );
        libraries.insert(sim_resource, sim_lib);
        semi_globals.insert(sim_resource, None);

        // `idle_main`: the idle loop, the only tree of a library of its own whose one
        // definition no object is made from. A frame running it names that definition's
        // type as its code's owner; every call resolves a private tree in its caller's own
        // file, so nothing in the lot can call it.
        let idle_resource = sim_resource + 1;
        let mut idle_lib = Library::default();
        let mut idle_frame = None;
        for (id, behavior, label) in library_behaviors(IDLE_MAIN_SOURCE)? {
            idle_frame = Some(behavior.frame_dimensions().map_err(|e| {
                Error::new(format!("idle_main: the idle loop cannot be called: {e:?}"))
            })?);
            idle_lib.behaviors.insert(id, behavior);
            idle_lib.names.insert(id, label);
        }
        let idle_frame = idle_frame.expect("the idle loop is one tree");
        let mut words = [0i16; DEFINITION_WORDS];
        words[14] = IDLE_GUID as u16 as i16;
        words[15] = (IDLE_GUID >> 16) as u16 as i16;
        idle_lib.definitions.insert(128, words);
        idle_lib.definition_order.push(128);
        libraries.insert(idle_resource, idle_lib);
        semi_globals.insert(idle_resource, None);
        catalog.by_guid.insert(IDLE_GUID, catalog.objects.len());
        catalog.objects.push(sims_content::Object {
            guid: IDLE_GUID,
            resource: idle_resource,
            chunk: 128,
            kind: 0,
            master: 0,
        });
        catalog.by_guid.insert(SIM_GUID, catalog.objects.len());
        catalog.objects.push(sims_content::Object {
            guid: SIM_GUID,
            resource: sim_resource,
            chunk: 128,
            kind: 2,
            master: 0,
        });

        let program = Program {
            global,
            libraries,
            semi_globals,
            careers: None,
            career_texts_end: true,
            labels_hold_no_token: true,
            zoning: BTreeMap::new(),
            animations: Default::default(),
            people: Default::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: Default::default(),
        };

        // The definitions given `idle_main`: each object's own, with the tiles of its group.
        let mut idle_defs = BTreeMap::new();
        for (res, guid, owner) in idle_objects {
            let def = catalog
                .object(guid)
                .ok_or_else(|| Error::new(format!("{owner}: GUID {guid:#010x} is not loaded")))?;
            for chunk in group_definitions(&program, res, def.chunk) {
                let words = program
                    .definition(res, chunk)
                    .expect("listed by its library");
                idle_defs.insert(
                    (res, chunk),
                    IdleDef {
                        owner: owner.clone(),
                        guid: definition_guid(words),
                    },
                );
            }
        }
        let subject_chunk = catalog
            .object(subject_guid)
            .ok_or_else(|| Error::new("the subject is not in the catalog"))?
            .chunk;
        let subject_group = group_definitions(&program, subject_resource, subject_chunk)
            .into_iter()
            .map(|chunk| (subject_resource, chunk))
            .collect();

        // Interaction names: each definition's effective TTAB (its own, or its group
        // master's for a tile whose word 7 is -1; in its file or its semi-global file),
        // named by the TTAs of the same ID in the file that holds the table.
        let mut bytes_of = BTreeMap::new();
        for (name, bytes) in &files {
            bytes_of.insert(resource_of(name)?, bytes.as_slice());
        }
        let mut menus = BTreeMap::new();
        let mut labels = BTreeMap::new();
        let attr_names = std::iter::once(&subject)
            .chain(&also)
            .map(|o| (o.slug.clone(), o.attr_names.clone()))
            .collect();
        for o in std::iter::once(&subject).chain(&also) {
            let res = resource_of(&object_file(&o.slug))?;
            for (chunk, (names, shown)) in interaction_names(&program, res, &bytes_of)? {
                menus.insert((res, chunk), names);
                labels.insert((res, chunk), shown);
            }
        }

        Ok(Self {
            program,
            catalog,
            subject_guid,
            subject_resource,
            subject_attr_names: subject.attr_names,
            attr_names,
            slugs,
            menus,
            labels,
            subject_group,
            idle_defs,
            idle_frame,
        })
    }

    /// The subject's tree with this ID or label: its own file first, then the stubs.
    fn tree(&self, r: &TreeRef) -> Result<u16> {
        match r {
            TreeRef::Id(id) => Ok(*id),
            TreeRef::Label(label) => {
                let mut libs = vec![self.subject_resource, self.program.global];
                if let Some(Some(s)) = self.program.semi_globals.get(&self.subject_resource) {
                    libs.push(*s);
                }
                for res in libs {
                    let Some(lib) = self.program.libraries.get(&res) else {
                        continue;
                    };
                    let ids: Vec<u16> = lib
                        .names
                        .iter()
                        .filter(|(_, n)| *n == label)
                        .map(|(id, _)| *id)
                        .collect();
                    match ids.as_slice() {
                        [] => {}
                        [id] => return Ok(*id),
                        many => {
                            return Err(Error::new(format!(
                                "tree label \"{label}\" names trees {many:?}; give the ID"
                            )));
                        }
                    }
                }
                let known: Vec<&str> = self.program.libraries[&self.subject_resource]
                    .names
                    .values()
                    .map(String::as_str)
                    .collect();
                Err(Error::new(format!(
                    "no tree labelled \"{label}\" (the subject has: {})",
                    known.join(", ")
                )))
            }
        }
    }

    /// The subject's own definition: its resource and OBJD chunk.
    fn subject_definition(&self) -> Result<(usize, u16)> {
        let def = self
            .catalog
            .object(self.subject_guid)
            .ok_or_else(|| Error::new("the subject is not in the catalog"))?;
        Ok((def.resource, def.chunk))
    }

    /// The subject's interaction entry with this name, from its own definition's table: its
    /// menu number, check tree, and action tree.
    fn interaction(&self, name: &str) -> Result<(u32, u16, u16)> {
        let (resource, chunk) = self.subject_definition()?;
        let empty = BTreeMap::new();
        let names = self.menus.get(&(resource, chunk)).unwrap_or(&empty);
        let menu = names
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(m, _)| *m)
            .ok_or_else(|| {
                Error::new(format!(
                    "the subject has no interaction \"{name}\" (it has: {})",
                    names
                        .values()
                        .map(|n| format!("\"{n}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        let table = self
            .program
            .interactions(resource, chunk)
            .map_err(Error::new)?
            .ok_or_else(|| Error::new("the subject has no interaction table"))?;
        let entry = table
            .interactions
            .iter()
            .find(|i| i.string_index == menu)
            .ok_or_else(|| Error::new(format!("no TTAB entry for menu number {menu}")))?;
        Ok((menu, entry.test, entry.action))
    }

    /// Whether the subject's definition, or a tile of its group, names an interaction
    /// `name`, by its TTAs name or by a name the game's menu shows for it (a pie menu on a
    /// multi-tile object may offer another tile's entry). Other definitions of the file are
    /// not the subject's menu.
    fn menu_names_interaction(&self, name: &str) -> bool {
        self.subject_group.iter().any(|def| {
            self.menus
                .get(def)
                .is_some_and(|names| names.values().any(|n| n == name))
                || self.labels.get(def).is_some_and(|labels| {
                    labels.iter().any(|(&menu, l)| {
                        l.shown(menu, false) == name || l.shown(menu, true) == name
                    })
                })
        })
    }

    fn attr_index(&self, key: &AttrKey) -> Result<u16> {
        match key {
            AttrKey::Index(i) => Ok(*i),
            AttrKey::Name(n) => self.subject_attr_names.get(n).copied().ok_or_else(|| {
                if self.subject_attr_names.is_empty() {
                    Error::new(format!(
                        "attribute '{n}': the subject has no named states (name attributes by \
                         index)"
                    ))
                } else {
                    Error::new(format!(
                        "the subject has no state '{n}' (it has: {})",
                        self.subject_attr_names
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }),
        }
    }

    fn attr_label(&self, key: &AttrKey, index: u16) -> String {
        match key {
            AttrKey::Index(i) => match self.subject_attr_names.iter().find(|(_, v)| **v == *i) {
                Some((n, _)) => format!("attrs.{i} ({n})"),
                None => format!("attrs.{i}"),
            },
            AttrKey::Name(n) => format!("attrs.{n} (attr[{index}])"),
        }
    }

    /// Resolves a table of attribute keys to indices, refusing two keys for one attribute
    /// (a state name and its index: the table would keep one value and drop the other).
    fn attr_indices<'k>(
        &self,
        map: &'k BTreeMap<AttrKey, i16>,
        what: &str,
    ) -> Result<Vec<(&'k AttrKey, u16, i16)>> {
        let mut seen: BTreeMap<u16, &AttrKey> = BTreeMap::new();
        let mut out = Vec::new();
        for (key, &v) in map {
            let i = self.attr_index(key)?;
            if let Some(prev) = seen.insert(i, key) {
                return Err(Error::new(format!(
                    "{what}: '{prev}' and '{key}' both set attribute {i}; give it once"
                )));
            }
            out.push((key, i, v));
        }
        Ok(out)
    }

    /// Whether the entry with menu number `menu` of `target`'s table runs at once (TTAB
    /// flag 4), as `sims-vm`'s `pick` decides (0x45d235); None when the table has no such
    /// entry.
    fn runs_at_once(&self, world: &World, target: i16, menu: i32) -> Option<bool> {
        let guid = world.objects.get(&u16::try_from(target).ok()?)?.guid;
        let def = self.catalog.object(guid)?;
        let table = self.program.interactions(def.resource, def.chunk).ok()??;
        let menu = u32::try_from(menu).ok()?;
        table
            .interactions
            .iter()
            .find(|i| i.string_index == menu)
            .map(|i| i.flags & AT_ONCE != 0)
    }

    /// The name the pie menu shows for `entry`, as `sims-game`'s menu names it: the one a
    /// check tree gave it (Add/Change Action String), or else (an empty one too) its TTAs
    /// name, "Interaction" and its menu number for a blank or missing one, with "*" before
    /// a debug entry's (0x41f167, [`EntryLabel::shown`]).
    fn displayed_name(&self, world: &World, entry: &MenuEntry) -> Option<String> {
        if let Some(name) = entry.name.clone().filter(|n| !n.is_empty()) {
            return Some(name);
        }
        let guid = world
            .objects
            .get(&u16::try_from(entry.action.target).ok()?)?
            .guid;
        let def = self.catalog.object(guid)?;
        let menu = u32::try_from(entry.action.table_index).ok()?;
        // The offer to join keeps its entry's stored name unless a check named it.
        let joins = entry.action.join == JOINS && entry.name.is_none();
        Some(
            self.labels
                .get(&(def.resource, def.chunk))?
                .get(&menu)?
                .shown(menu, joins),
        )
    }

    /// The name of menu number `menu` in the table of `target`'s own definition.
    fn menu_name(&self, world: &World, target: i16, menu: i32) -> Option<String> {
        let guid = world.objects.get(&u16::try_from(target).ok()?)?.guid;
        let def = self.catalog.object(guid)?;
        self.menus
            .get(&(def.resource, def.chunk))?
            .get(&u32::try_from(menu).ok()?)
            .cloned()
    }
}

/// An object definition's GUID (words 14 and 15).
fn definition_guid(words: &[i16; DEFINITION_WORDS]) -> u32 {
    u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16)
}

/// The definitions of `res` one creation of definition `chunk` makes: itself, or for a tile
/// of a multi-tile object (word 10, its group's ID, not 0) every definition of the file in
/// its group (the factory makes the whole group: `sims-vm`'s `group_plan`, 0x440b2d).
fn group_definitions(program: &Program, res: usize, chunk: u16) -> Vec<u16> {
    let Some(lib) = program.libraries.get(&res) else {
        return vec![chunk];
    };
    let group = lib.definitions.get(&chunk).map_or(0, |w| w[10]);
    if group == 0 {
        return vec![chunk];
    }
    lib.definition_order
        .iter()
        .copied()
        .filter(|c| lib.definitions.get(c).is_some_and(|w| w[10] == group))
        .collect()
}

/// Where a definition's interaction table is, as `Program::interactions` finds it: the
/// resource holding the TTAB and its ID. Word 7 names the table; a tile whose word 7 is -1
/// uses its group master's (0x45726a, 0x43ae0a). The table is in the definition's own file,
/// or else in its semi-global file.
fn table_source(program: &Program, res: usize, chunk: u16) -> Option<(usize, u16)> {
    let words = program.definition(res, chunk)?;
    let mut id = words[7];
    if id == -1 {
        id = (words[10] != 0)
            .then(|| program.master_definition(res, chunk).ok().flatten())
            .flatten()
            .map_or(0, |master| master[7]);
    }
    let id = u16::try_from(id).ok().filter(|&id| id != 0)?;
    if program.libraries.get(&res)?.interactions.contains_key(&id) {
        return Some((res, id));
    }
    let semi = program.semi_globals.get(&res).copied().flatten()?;
    program
        .libraries
        .get(&semi)?
        .interactions
        .contains_key(&id)
        .then_some((semi, id))
}

/// Each definition's interaction names, by OBJD chunk and menu number: its effective table
/// ([`table_source`]), named by the TTAs of the table's ID in the file that holds it; and
/// what the game's menu shows for each of the table's entries ([`EntryLabel`]).
#[allow(clippy::type_complexity)]
fn interaction_names(
    program: &Program,
    res: usize,
    bytes_of: &BTreeMap<usize, &[u8]>,
) -> Result<BTreeMap<u16, (BTreeMap<u32, String>, BTreeMap<u32, EntryLabel>)>> {
    let mut parsed: BTreeMap<usize, Iff> = BTreeMap::new();
    let mut by_definition = BTreeMap::new();
    let lib = &program.libraries[&res];
    for &chunk in &lib.definition_order {
        let mut out = BTreeMap::new();
        let mut shown = BTreeMap::new();
        let table = match program.interactions(res, chunk) {
            Ok(Some(table)) => table,
            Ok(None) => continue,
            Err(e) => return Err(Error::new(e)),
        };
        let (holder, id) = table_source(program, res, chunk).ok_or_else(|| {
            Error::new(format!(
                "OBJD #{chunk}: sims-vm finds an interaction table the runner cannot locate"
            ))
        })?;
        let iff = match parsed.entry(holder) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let bytes = bytes_of.get(&holder).ok_or_else(|| {
                    Error::new(format!(
                        "OBJD #{chunk}: its interaction table's file is not one the runner \
                         loaded"
                    ))
                })?;
                e.insert(Iff::parse(bytes.to_vec()).map_err(|e| Error::new(e.to_string()))?)
            }
        };
        let ttas = iff
            .get(b"TTAs", id)
            .and_then(|c| StringTable::parse(iff.data(c)).ok());
        for entry in &table.interactions {
            let name = ttas
                .as_ref()
                .and_then(|t| t.get(entry.string_index as usize, LANGUAGE));
            if let Some(name) = name {
                out.entry(entry.string_index)
                    .or_insert_with(|| name.to_owned());
            }
            shown
                .entry(entry.string_index)
                .or_insert_with(|| EntryLabel {
                    label: name.filter(|n| !n.trim().is_empty()).map(str::to_owned),
                    strings: ttas.is_some(),
                    debug: entry.flags & DEBUG != 0,
                });
        }
        by_definition.insert(chunk, (out, shown));
    }
    Ok(by_definition)
}

// ---------------------------------------------------------------------------------------
// The world
// ---------------------------------------------------------------------------------------

fn lot_array(stride: usize, fill: u8) -> LotArray {
    LotArray {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![fill; 64 * 64 * stride],
    }
}

fn empty_interaction() -> SavedInteraction {
    SavedInteraction {
        join: 0,
        uid: 0,
        caller: 0,
        target: 0,
        icon: 0,
        table_index: -1,
        args: [0; 4],
        priority: 0,
        action: 0,
        attenuation: 0.0,
        flags: 0,
    }
}

fn sim_object(state: &SimState) -> Result<Object> {
    let mut motives = [0.0f32; 16];
    for (&i, &v) in &state.motives {
        *motives
            .get_mut(usize::from(i))
            .ok_or_else(|| Error::new(format!("sim.motives.{i}: the Sim has 16 motives")))? = v;
    }
    let mut data = vec![0i16; 72];
    data[11] = SIM_ID as i16;
    data[14] = -1;
    data[29] = 1; // room 1 (the outdoor room 2 with `room = true`: `empty_world`)
    data[67] = 5;
    let mut person = vec![0i16; 256];
    person[58] = 25; // an adult's age
    let set = |out: &mut Vec<i16>, values: &BTreeMap<u16, i16>, what: &str| -> Result<()> {
        let len = out.len();
        for (&i, &v) in values {
            *out.get_mut(usize::from(i)).ok_or_else(|| {
                Error::new(format!("sim.{what}.{i}: the Sim has {len} {what} words"))
            })? = v;
        }
        Ok(())
    };
    set(&mut person, &state.person, "person")?;
    set(&mut data, &state.data, "data")?;
    let mut attributes = vec![0i16; 8];
    set(&mut attributes, &state.attrs, "attrs")?;
    let mut temps = vec![0i16; 8];
    set(&mut temps, &state.temps, "temps")?;
    Ok(Object {
        id: SIM_ID,
        guid: SIM_GUID,
        footprint: [0; 4],
        position: [32, 32, 1],
        unknown_short: 0,
        attributes,
        temps,
        data,
        obsolete_word: 0,
        stack_capacity_bytes: 512,
        // Its main loop, which runs the interactions queued on it.
        stack: vec![Frame {
            stack_object: 0,
            tree: SIM_MAIN,
            node: 0,
            parameters: vec![0],
            locals: Vec::new(),
            primitive_state: 0,
            code_owner_type: SIM_TYPE as i16,
        }],
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: vec![
            Slot {
                unknown: 0,
                object: 0,
            };
            6
        ],
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: Some(Person {
            animation_events: 0,
            engaged: 0,
            appearance: Default::default(),
            accessories: Vec::new(),
            animations: Default::default(),
            routing_state: 0,
            floats: [0.0; 9],
            previous_motives: [0.0; 16],
            motives,
            data: person,
            routing_frames: 0,
            active: empty_interaction(),
            last: empty_interaction(),
            queue: Vec::new(),
            object_uses: Vec::new(),
            motive_deltas: Vec::new(),
        }),
        // Scheduler flag 1 (the factory's "ready"), no countdown: the lot update ticks it.
        registry_word: Some(0x0001_ffff),
        last_result: false,
        removal_skips_resets: false,
        // The Sim the player orders about is the selected one.
        selected: true,
    })
}

fn set_funds(world: &mut World, funds: i32) {
    world.budget.funds = funds;
    world.globals[14] = (funds / 10_000) as i16;
    world.globals[13] = (funds % 10_000) as i16;
}

fn empty_world(sim: Option<&SimState>, room: bool) -> Result<World> {
    let mut world = World::empty();
    world.size = 16;
    world.globals = vec![0; 64];
    world.globals[1] = 1; // day
    world.globals[7] = 1; // month
    world.globals[8] = 2000; // year
    world.globals[10] = 1; // lot number
    world.budget.days = vec![Some([0; 8])];
    set_funds(&mut world, 0);
    // A flat lot with one indoor room, so placement and removal run their real code.
    world.arrays.insert(0, lot_array(4, 0));
    for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
        world.arrays.insert(floors, lot_array(2, 0));
        world.arrays.insert(walls, lot_array(14, 0));
        world.arrays.insert(flags, lot_array(1, 0));
    }
    world.arrays.insert(3, lot_array(2, 0));
    world.arrays.insert(103, lot_array(2, 0));
    world.arrays.insert(9, lot_array(1, 0xff));
    world.arrays.insert(10, lot_array(1, 0xff));
    world.rooms.map = [vec![1; 64 * 64], vec![1024; 64 * 64]];
    world.rooms.rooms.insert(
        1,
        Room {
            outdoors: false,
            ..Room::default()
        },
    );
    if room {
        // `[lot] room = true`: room 1 lists the tiles of [`ROOM_TILES`] on level 1, and its
        // first tile has a covered west wall (segment bit 1, style 1 and pattern 1, as the
        // behavioural suite's placed rooms have), so its score is finite.
        // The rest of the ground level is an outdoor room 2, so an object that leaves the
        // room's tiles is in a room that counts it, not in room 1 uncounted.
        let mut outside = Vec::new();
        for x in 0..64u8 {
            for y in 0..64u8 {
                if ROOM_TILES.contains(&x) && ROOM_TILES.contains(&y) {
                    continue;
                }
                world.rooms.map[0][usize::from(x) * 64 + usize::from(y)] = 2;
                if x < 16 && y < 16 {
                    outside.push([x, y, 1]);
                }
            }
        }
        world.rooms.rooms.insert(
            2,
            Room {
                outdoors: true,
                tiles: outside,
                ..Room::default()
            },
        );
        let tiles = &mut world.rooms.rooms.get_mut(&1).expect("made above").tiles;
        for x in ROOM_TILES {
            for y in ROOM_TILES {
                tiles.push([x, y, 1]);
            }
        }
        let cell = (usize::from(ROOM_TILES.start) * 64 + usize::from(ROOM_TILES.start)) * 14;
        let walls = &mut world.arrays.get_mut(&2).expect("made above").data;
        walls[cell] = 1;
        walls[cell + 2] = 1;
        walls[cell + 6] = 1;
    }
    world.indicators = [u16::MAX, u16::MAX];
    if let Some(state) = sim {
        world.order.push(SIM_ID);
        world.people.push(SIM_ID);
        world.types.insert(SIM_TYPE, SIM_GUID);
        let mut sim = sim_object(state)?;
        // The Sim stands on tile [2, 2]: with the placed room, that is the outdoor room 2
        // (unless the case gives its room word).
        if room && !state.data.contains_key(&29) {
            sim.data[29] = 2;
        }
        world.objects.insert(SIM_ID, sim);
    }
    Ok(world)
}

// ---------------------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------------------

/// What a run left that the world does not hold.
struct Observed {
    outcome: Outcome,
    fault: Option<String>,
    turns: u32,
    /// The stack object the outermost frame returned with, or why there is none.
    stack_obj: std::result::Result<i16, String>,
    /// Why the run stopped short, when it did (it was still running).
    note: Option<String>,
    /// Why the game could not have gone on, or why the runner cannot say what it does next
    /// (a dialog without an answer it accepts, a lot update that stopped on something
    /// `sims-vm` does not port): the case fails with this, whatever it expects.
    stopped: Option<String>,
    /// The Sim had motive changes over time pending (`person.motive_deltas`) at the end of a
    /// run in which no lot update ran: the person tick that applies them every tick did not
    /// run after the code that set them.
    motive_changes: bool,
}

impl Observed {
    fn new(outcome: Outcome, turns: u32) -> Self {
        Self {
            outcome,
            fault: None,
            turns,
            stack_obj: Err("the run did not return".to_owned()),
            note: None,
            stopped: None,
            motive_changes: false,
        }
    }

    fn fault(fault: String, turns: u32) -> Self {
        Self {
            fault: Some(fault),
            ..Self::new(Outcome::Error, turns)
        }
    }

    fn stopped(why: String, turns: u32) -> Self {
        Self {
            stopped: Some(why),
            ..Self::new(Outcome::Error, turns)
        }
    }
}

/// A dialog record's status while it is new: made and not shown yet
/// (`sims-vm/src/primitives/dialog.rs`).
const DIALOG_NEW: u8 = 0;

/// The kinds of the dialogs made and not shown yet, in the order the end of an update shows
/// them.
fn new_dialog_kinds(world: &World) -> Vec<u8> {
    world
        .dialogs
        .records
        .iter()
        .filter(|r| r.status == DIALOG_NEW)
        .map(|r| r.operands[5] & 0x7f)
        .collect()
}

/// The motive tuning the lot updates run with: the rates the original uses where the global
/// file's tuning tables lack a value (`Rates::from_lookup`), and no mood curves (the test
/// lot's global file has none). A decay, which reads both, stops the run ([`real_turn`]): so
/// does a decay point the person tick reaches with the decay switched off.
fn tuning() -> Tuning {
    Tuning {
        rates: [0, 1, 2].map(|kind| Rates::from_lookup(kind, |_| None)),
        adult_mood: Vec::new(),
        child_mood: Vec::new(),
    }
}

/// The game around a run: the lot update each turn is (`sims-vm`'s
/// `Interpreter::lot_update_kept`, in strict mode), and what runs between updates.
struct Game<'a> {
    /// Runs the lot updates, the factory, and synchronous calls.
    lot: Interpreter<'a>,
    /// Lists, checks and picks the pie menu's entries and shows the dialogs made between
    /// updates, on the game's stream, as `sims-vm`'s `boot::Lot` does.
    between: Interpreter<'a>,
    /// Runs the runner's look-ahead on copies of the world, with a sound sink of its own.
    ahead: Interpreter<'a>,
    tuning: Tuning,
    /// The case's `answers`, as `sims_vm::Answers::choices` gives them: a one-button dialog
    /// that does not ask for text answers itself.
    player: Answers,
    /// The kinds of the dialogs shown, in order, when `track_kinds`.
    kinds: Vec<u8>,
    /// Whether the case checks the dialogs' kinds, which takes a look-ahead each update.
    track_kinds: bool,
    /// `[lot] room = true`: the rooms marked are counted after each update, as the game
    /// counts them before it draws a frame (`Interpreter::draw_frame_in`).
    rooms: bool,
}

impl Game<'_> {
    /// Shows the dialogs made and not shown yet, as the end of the next lot update would
    /// (`Interpreter::end_of_update`, as after a partial pass, so the queued removals stay
    /// queued): for a run that ends between updates, or where an update stopped.
    fn settle(&mut self, world: &mut World) -> std::result::Result<(), String> {
        let pending = new_dialog_kinds(world);
        if pending.is_empty() {
            return Ok(());
        }
        self.kinds.extend(&pending);
        let mut update = Update::default();
        self.between
            .end_of_update(world, true, &mut self.player, &mut update)
            .map_err(|e| update_stopped(&e))
    }

    /// One lot update.
    fn update(&mut self, world: &mut World) -> (Update, std::result::Result<(), TickError>) {
        // A dialog that only informs is made, shown and done within one update, leaving no
        // record: the look-ahead runs the update's scheduler pass on a copy to see what the
        // end of the update will show.
        let ahead = self.track_kinds.then(|| {
            let mut copy = world.clone();
            let _ = self
                .ahead
                .scheduler_pass(&mut copy, &self.tuning, &mut Update::default());
            new_dialog_kinds(&copy)
        });
        let (update, mut result) =
            self.lot
                .lot_update_kept(world, &self.tuning, &mut self.player, None);
        if result.is_ok()
            && self.rooms
            && let Err(f) = self.lot.update_pending_rooms(world)
        {
            result = Err(TickError::Unsupported(format!(
                "the room update after the lot update faulted: {f}"
            )));
        }
        if result.is_ok()
            && let Some(kinds) = ahead
        {
            let shown = update.answered.len() + update.special_panels_answered.len();
            if shown != kinds.len() {
                let why = format!(
                    "the runner's look-ahead saw {} dialog(s) for the update to show, and it \
                     showed {shown}; their kinds are not known",
                    kinds.len()
                );
                return (update, Err(TickError::Unsupported(why)));
            }
            self.kinds.extend(kinds);
        }
        (update, result)
    }
}

/// One turn of `thread` as an object's tick runs it (`Interpreter::turn`, under the tick's
/// limit), an instruction at a time so `after_step` can look at the world after each one.
fn turn(vm: &Interpreter<'_>, world: &mut World, thread: u16, mut after_step: impl FnMut(&World)) {
    for _ in 0..TICK_BUDGET {
        let status = vm.step(world, thread);
        after_step(world);
        if !matches!(status, Ok(Status::Running)) {
            return;
        }
    }
}

/// How a turn's lot update went for the thread a run follows.
enum Step {
    /// The update ran to its end, and the thread's object is still on the lot.
    Went,
    /// The thread returned from its outermost frame with this result. The update stops
    /// there: it is error 0x3e9 in the original, whose handling `sims-vm` does not port.
    Returned(bool),
    /// The thread faulted, or its object left the lot.
    Fault(String),
    /// The case fails, whatever it expects.
    Stopped(String),
}

/// Why a lot update (or the end of one) stopped, in the spec's terms where it is a dialog:
/// its kind (by the answers it offers) and its buttons (a status less 1).
fn update_stopped(e: &TickError) -> String {
    let TickError::Unanswered { shown, given } = e else {
        return format!("the lot update stopped: {e}");
    };
    let kind = match (shown.text_entry, shown.answers.len()) {
        (true, _) => "text",
        (false, 1) => "ok",
        (false, 2) => "yes_no",
        _ => "yes_no_cancel",
    };
    let buttons = shown
        .answers
        .iter()
        .map(|a| a.saturating_sub(1).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    match given {
        None => format!(
            "dialogs: a {kind} dialog of object {} waits for an answer (button(s) {buttons}), \
             and `answers` has none left",
            shown.object
        ),
        Some(a) => format!(
            "dialogs: a {kind} dialog of object {} offers button(s) {buttons}; the game refuses \
             answer {}",
            shown.object,
            a.saturating_sub(1)
        ),
    }
}

/// The original's error for a thread that returns from its outermost frame.
const RETURNED: u16 = 0x3e9;
/// One turn: one lot update, and what it did to `thread`.
fn real_turn(
    game: &mut Game<'_>,
    world: &mut World,
    ids: &Ids,
    thread: u16,
    t: u32,
) -> (Update, Step) {
    // The tick at which the Sim's person tick last reached its motive decay.
    let last_decay = |world: &World| {
        let sim = ids.sim?;
        Some(world.person_ticks.get(&sim).map_or(0, |p| p.last_decay))
    };
    let before = last_decay(world);
    let (update, result) = game.update(world);
    // Any decay point stops the run, whatever person word 29 and data word 34 (which keep
    // the motives from decaying) were before the update: the person tick reads them after
    // the Sim's own turn in the same tick (`sims-vm`'s `tick_person`), so a script can open
    // them just before the decay.
    if before != last_decay(world) {
        let why = format!(
            "turn {t}: the Sim's person tick reached its motive decay (every 60 ticks); the \
             runner's lot has no global tuning (motive rates and mood curves), so its motives \
             are not the game's from here, whether or not they decayed (give the case at most \
             60 turns)"
        );
        return (update, Step::Stopped(why));
    }
    let step = match result {
        Ok(()) if world.objects.contains_key(&thread) => Step::Went,
        Ok(()) => Step::Fault(format!(
            "{} left the lot at the end of turn {t} (the lot update removes the objects queued \
             for removal), before its tree returned",
            ids.who(thread)
        )),
        Err(TickError::Fault(f)) if f.object == thread => match world.objects.get(&thread) {
            Some(o)
                if o.stack.is_empty() && matches!(f.trap, Trap::Native { code: RETURNED, .. }) =>
            {
                Step::Returned(o.last_result)
            }
            _ => Step::Fault(f.to_string()),
        },
        // An object with no thread to run: its Main returned on its first turn, when it was
        // created.
        Err(TickError::Fault(f))
            if world
                .objects
                .get(&f.object)
                .is_some_and(|o| o.stack.is_empty()) =>
        {
            Step::Stopped(format!(
                "turn {t}: the lot update stopped on {}, which has no thread to run: its Main \
                 returned on its first turn, when it was created, so its tick faults ({f}); \
                 live play resets the object and parks it after a second error, which the \
                 runner does not model; give it a Main that does not return{}",
                ids.who(f.object),
                if f.object == ids.subject {
                    " (or set `idle_main = true` under [subject] to run the case with an idle \
                     loop in its place, an assumption the run reports)"
                } else {
                    ""
                }
            ))
        }
        Err(TickError::Fault(f)) => Step::Stopped(format!(
            "turn {t}: the lot update stopped on a fault of {}, which the run does not \
             follow: {f}",
            ids.who(f.object)
        )),
        Err(e) => Step::Stopped(format!("turn {t}: {}", update_stopped(&e))),
    };
    (update, step)
}

/// The stack object `thread`'s outermost frame, running `tree`, returned with. A popped
/// frame's block stays in the frame storage (`sims_sim::FrameStorage`), written from the
/// frame as it was when it returned, after its last instruction (`sims-vm`'s `pop_frame`);
/// its header holds the tree (word 0) and the stack object (word 2). A threaded run starts
/// with no frame storage, so the outermost frame's block is at word 0.
fn returned_stack_obj(world: &World, thread: u16, tree: u16) -> std::result::Result<i16, String> {
    let words = world
        .frame_storage
        .get(&thread)
        .map_or(&[][..], |s| &s.words[..]);
    match (words.first(), words.get(2)) {
        (Some(Some(t)), Some(Some(stack_obj))) if (*t as u16) & !Frame::TREE_FLAG == tree => {
            Ok(*stack_obj)
        }
        _ => Err("the returned frame could not be read from the frame storage".to_owned()),
    }
}

/// Whether the Sim has motive changes over time pending, which its person tick applies.
fn motive_changes_pending(world: &World, sim: Option<u16>) -> bool {
    sim.and_then(|s| world.objects.get(&s))
        .and_then(|o| o.person.as_ref())
        .is_some_and(|p| !p.motive_deltas.is_empty())
}

/// `observed` once the dialogs made and not shown yet are shown ([`Game::settle`]), for a
/// run that ends between updates or where an update stopped.
fn settled(game: &mut Game<'_>, world: &mut World, observed: Observed) -> Observed {
    match game.settle(world) {
        Ok(()) => observed,
        Err(why) => Observed::stopped(why, observed.turns),
    }
}

struct Ids {
    subject: u16,
    sim: Option<u16>,
}

impl Ids {
    fn resolve(&self, r: ObjRef) -> Result<i16> {
        Ok(match r {
            ObjRef::Object => self.subject as i16,
            ObjRef::Sim => {
                self.sim
                    .ok_or_else(|| Error::new("there is no Sim on the lot"))? as i16
            }
            ObjRef::None => 0,
            ObjRef::Id(id) => id,
        })
    }

    fn name(&self, id: i16) -> String {
        if id == self.subject as i16 {
            "object".to_owned()
        } else if self.sim.is_some_and(|s| s as i16 == id) {
            "sim".to_owned()
        } else if id == 0 {
            "none".to_owned()
        } else {
            id.to_string()
        }
    }

    /// An object, for messages.
    fn who(&self, id: u16) -> String {
        if id == self.subject {
            format!("the subject (object {id})")
        } else if self.sim == Some(id) {
            format!("the Sim (object {id})")
        } else {
            format!("object {id}")
        }
    }
}

fn run_case(
    lot: &Lot,
    file: &TestFile,
    case: &TestCase,
    assumptions: &mut Vec<String>,
) -> Result<Vec<String>> {
    let sim_state = case.sim_state(file);
    let room = case.in_room(file);
    let mut world = empty_world(sim_state.as_ref(), room)?;
    let sounds = RefCell::new(Vec::new());
    let ahead_sounds = RefCell::new(Vec::new());
    let interpreter = |stream, sink| {
        let mut vm = Interpreter::new(&lot.program, &lot.catalog, stream);
        vm.sounds = Some(sink);
        vm
    };
    let mut game = Game {
        lot: interpreter(Stream::Lot, &sounds),
        between: interpreter(Stream::Global, &sounds),
        ahead: interpreter(Stream::Lot, &ahead_sounds),
        tuning: tuning(),
        player: Answers::choices(
            &case
                .answers
                .iter()
                .map(|a| a.saturating_add(1))
                .collect::<Vec<_>>(),
        ),
        kinds: Vec::new(),
        track_kinds: case.expect.dialogs.is_some(),
        rooms: room,
    };

    let tiles = if room { case.tiles(file)? } else { Vec::new() };
    let subject = create(lot, &game.lot, &mut world, lot.subject_guid, assumptions)
        .map_err(|e| e.with_context("creating the subject"))?;
    if let Some(&tile) = tiles.first() {
        place_on_tile(lot, &mut world, subject, tile)
            .map_err(|e| e.with_context("placing the subject"))?;
    }
    for (i, c) in case.given.create.iter().enumerate() {
        let guid = match &c.object {
            CreateRef::Guid(g) if [SIM_GUID, IDLE_GUID].contains(g) => {
                return Err(Error::new(format!(
                    "given.create: GUID {g:#010x} is reserved for the runner's {}",
                    if *g == SIM_GUID {
                        "test Sim"
                    } else {
                        "idle loop"
                    }
                )));
            }
            CreateRef::Guid(g) => *g,
            CreateRef::Slug(s) => *lot.slugs.get(s).ok_or_else(|| {
                Error::new(format!(
                    "given.create: no object '{s}' is loaded (add it to [lot].also)"
                ))
            })?,
        };
        let id = create(lot, &game.lot, &mut world, guid, assumptions)
            .map_err(|e| e.with_context(format!("creating {:?}", c.object)))?;
        if let Some(&tile) = tiles.get(i + 1) {
            place_on_tile(lot, &mut world, id, tile)
                .map_err(|e| e.with_context(format!("placing {:?}", c.object)))?;
        }
    }
    let ids = Ids {
        subject,
        sim: sim_state.as_ref().map(|_| SIM_ID),
    };

    // The case's inputs.
    for (key, i, v) in lot.attr_indices(&case.given.attrs, "given.attrs")? {
        let o = world.objects.get_mut(&subject).expect("made above");
        let len = o.attributes.len();
        *o.attributes.get_mut(usize::from(i)).ok_or_else(|| {
            Error::new(format!(
                "given.attrs.{key}: attribute {i} is past the subject's {len}"
            ))
        })? = v;
    }
    for (what, values, pick) in [
        ("temps", &case.given.temps, 0),
        ("data", &case.given.data, 1),
    ] {
        let o = world.objects.get_mut(&subject).expect("made above");
        let words = if pick == 0 { &mut o.temps } else { &mut o.data };
        for (&i, &v) in values {
            let len = words.len();
            *words.get_mut(usize::from(i)).ok_or_else(|| {
                Error::new(format!(
                    "given.{what}.{i}: the subject has {len} {what} words"
                ))
            })? = v;
        }
    }
    for (&i, &v) in &case.given.globals {
        let len = world.globals.len();
        *world
            .globals
            .get_mut(usize::from(i))
            .ok_or_else(|| Error::new(format!("given.globals.{i}: the lot has {len} globals")))? =
            v;
    }
    if let Some(f) = case.given.funds {
        set_funds(&mut world, f);
    }

    // What the run starts from: the dialogs the factory made are the game's before the case.
    game.settle(&mut world)
        .map_err(|why| Error::new(format!("making the lot's objects: {why}")))?;
    if room {
        // The run starts from counted rooms, as a loaded lot's rooms are: the room and the
        // outdoor room around it.
        world.mark_room(1);
        world.mark_room(2);
        game.lot
            .update_pending_rooms(&mut world)
            .map_err(|f| Error::new(format!("counting the room before the run: {f}")))?;
    }
    game.kinds.clear();
    sounds.borrow_mut().clear();
    let queue_before: BTreeSet<i32> = ids
        .sim
        .and_then(|s| world.objects.get(&s))
        .and_then(|o| o.person.as_ref())
        .map(|p| p.queue.iter().map(|a| a.uid).collect())
        .unwrap_or_default();

    let (observed, picked) = match &case.target {
        Target::Interaction(name) => run_interaction(lot, &mut game, &mut world, case, &ids, name)?,
        _ => (run_tree(lot, &mut game, &mut world, case, &ids)?, None),
    };
    if let Some(why) = observed.stopped {
        return Ok(vec![why]);
    }
    // The frame after the run: the rooms its code marked are counted. A room nothing marked
    // keeps its count from before, as in the game.
    if room && let Err(f) = game.lot.update_pending_rooms(&mut world) {
        return Ok(vec![format!("the room update after the run faulted: {f}")]);
    }

    let mut failures = Vec::new();
    check(
        lot,
        &world,
        &case.expect,
        &ids,
        &observed,
        &sounds.borrow(),
        &game.kinds,
        &queue_before,
        picked,
        &mut failures,
    )?;
    Ok(failures)
}

/// Stands a just-made object on `tile` of room 1 on level 1, facing north, as the
/// behavioural suite's placed-room tests do (`PackVmFixture::place_on_tile`): its position
/// at the tile's centre, its footprint the tile, data word 1 (facing) 0, data word 29 (its
/// room) 1, and the tile's head in the level's object grid (array 3) the object. A
/// multi-tile object, whose tiles stand together, is refused.
fn place_on_tile(lot: &Lot, world: &mut World, id: u16, tile: [u8; 2]) -> Result<()> {
    let object = world
        .objects
        .get(&id)
        .ok_or_else(|| Error::new(format!("object {id} left the lot when it was made")))?;
    let group = lot
        .catalog
        .object(object.guid)
        .and_then(|def| lot.program.definition(def.resource, def.chunk))
        .map_or(0, |words| words[10]);
    if group != 0 {
        return Err(Error::new(format!(
            "it is a tile of a multi-tile object (group {group}); [lot] room = true places \
             one-tile objects only"
        )));
    }
    let cell = (usize::from(tile[0]) * 64 + usize::from(tile[1])) * 2;
    let grid = &world
        .arrays
        .get(&3)
        .expect("the test lot has an object grid")
        .data;
    // An Init that placed the object itself left its grid head elsewhere: moving it here
    // would leave that head behind, and the room would count the object twice.
    if let Some(other) = (0..64 * 64)
        .filter(|&c| c * 2 != cell)
        .find(|&c| i16::from_le_bytes([grid[c * 2], grid[c * 2 + 1]]) == id as i16)
    {
        return Err(Error::new(format!(
            "its Init already placed it on tile [{}, {}]; place it there (`place`) or make \
             it without placing itself",
            other / 64,
            other % 64
        )));
    }
    let head = i16::from_le_bytes([grid[cell], grid[cell + 1]]);
    if head != 0 && head != id as i16 {
        return Err(Error::new(format!(
            "tile [{}, {}] already holds object {head} (made by a script before it was \
             placed); place the object on another tile",
            tile[0], tile[1]
        )));
    }
    let [x, y] = tile.map(i32::from);
    let (px, py) = (x * 16 + 8, y * 16 + 8);
    let object = world.objects.get_mut(&id).expect("looked up above");
    object.position = [px, py, 1];
    object.footprint = [px - 8, py - 8, px + 8, py + 8];
    for (word, value) in [(1, 0), (29, 1)] {
        let len = object.data.len();
        *object
            .data
            .get_mut(word)
            .ok_or_else(|| Error::new(format!("object {id} has {len} data words")))? = value;
    }
    let grid = &mut world
        .arrays
        .get_mut(&3)
        .expect("the test lot has an object grid")
        .data;
    grid[cell..cell + 2].copy_from_slice(&(id as i16).to_le_bytes());
    Ok(())
}

/// Makes an object with the VM's factory (`Interpreter::create`, which runs its Init and its
/// Main's first turn), then applies `idle_main` to what it made of a definition given it,
/// noting each definition replaced in `assumptions`.
fn create(
    lot: &Lot,
    vm: &Interpreter<'_>,
    world: &mut World,
    guid: u32,
    assumptions: &mut Vec<String>,
) -> Result<u16> {
    let before: BTreeSet<u16> = world.objects.keys().copied().collect();
    let id = vm
        .create(world, guid)
        .map_err(|f| Error::new(format!("faulted: {f}")))?;
    // What the creation made of a definition given `idle_main`: the object, or a multi-tile
    // object's tiles (and anything of such a definition its Init or Main made).
    let made: Vec<(u16, (usize, u16))> = world
        .objects
        .iter()
        .filter(|(id, _)| !before.contains(id))
        .filter_map(|(&id, o)| {
            let def = lot.catalog.object(o.guid)?;
            let key = (def.resource, def.chunk);
            lot.idle_defs.contains_key(&key).then_some((id, key))
        })
        .collect();
    if !made.is_empty() {
        replace_returned_mains(lot, world, &made, assumptions)?;
    }
    Ok(id)
}

/// `idle_main`: gives each of `made` (one creation's objects of definitions given the key)
/// whose Main returned on its first turn, in the factory, the idle loop as its thread, as a
/// threaded run gives a thread its tree. A creation whose such objects all still run is a
/// setup error: the key is only for a Main that returns, so it cannot hide another one.
fn replace_returned_mains(
    lot: &Lot,
    world: &mut World,
    made: &[(u16, (usize, u16))],
    assumptions: &mut Vec<String>,
) -> Result<()> {
    let returned: Vec<(u16, (usize, u16))> = made
        .iter()
        .copied()
        .filter(|(id, _)| world.objects.get(id).is_some_and(|o| o.stack.is_empty()))
        .collect();
    if returned.is_empty() {
        let names: BTreeSet<String> = made
            .iter()
            .map(|(_, key)| {
                let def = &lot.idle_defs[key];
                format!("{} (OBJD #{}, GUID {:#010x})", def.owner, key.1, def.guid)
            })
            .collect();
        return Err(Error::new(format!(
            "idle_main is only for a Main that returns: {}'s Main is still running after its \
             first turn; drop idle_main",
            names.into_iter().collect::<Vec<_>>().join(", ")
        )));
    }
    world.insert_type(IDLE_TYPE, IDLE_GUID);
    let (args, locals) = lot.idle_frame;
    for (id, key) in returned {
        world.frame_storage.remove(&id);
        let o = world.objects.get_mut(&id).expect("listed above");
        // As the factory installs a Main (0x47480c): four zero arguments, no stack object.
        o.stack = vec![Frame {
            stack_object: 0,
            tree: IDLE_MAIN,
            node: 0,
            parameters: vec![0; args.max(4)],
            locals: vec![0; locals],
            primitive_state: 0,
            code_owner_type: IDLE_TYPE as i16,
        }];
        o.set_countdown(-1);
        let def = &lot.idle_defs[&key];
        let line = idle_main_assumption(&def.owner, key.1, def.guid);
        if !assumptions.contains(&line) {
            assumptions.push(line);
        }
    }
    Ok(())
}

fn run_tree(
    lot: &Lot,
    game: &mut Game<'_>,
    world: &mut World,
    case: &TestCase,
    ids: &Ids,
) -> Result<Observed> {
    let (tree, default_me) = match &case.target {
        Target::Tree(r) => (lot.tree(r)?, Actor::Object),
        Target::Guard(name) => {
            let (_, test, _) = lot.interaction(name)?;
            if test == 0 {
                return Err(Error::new(format!(
                    "interaction \"{name}\" has no check tree"
                )));
            }
            (test, Actor::Sim)
        }
        Target::Action(name) => (lot.interaction(name)?.2, Actor::Sim),
        Target::Interaction(_) => unreachable!("run_interaction"),
    };
    let me = match case.me {
        Some(a) => a,
        None if ids.sim.is_some() => default_me,
        None => Actor::Object,
    };
    let thread = match me {
        Actor::Object => ids.subject,
        Actor::Sim => ids
            .sim
            .ok_or_else(|| Error::new("me = \"sim\" needs a Sim"))?,
    };
    let stack_obj = ids.resolve(case.stack_obj.unwrap_or(ObjRef::Object))?;

    // The tree, as the subject's code finds it, whichever way it runs: a tree that is not
    // there is a mistake in the spec, not a fault of the code under test.
    let behavior = match lot.program.tree(lot.subject_resource, tree) {
        sims_vm::Tree::Behavior(_, b) => b,
        sims_vm::Tree::Primitive(id) => {
            return Err(Error::new(format!("tree {id} is a primitive, not a BHAV")));
        }
        sims_vm::Tree::Missing => {
            return Err(Error::new(format!("the subject's code has no tree {tree}")));
        }
    };

    // A synchronous call: the subject runs its own tree as the game runs a callback.
    if me == Actor::Object && case.turns.is_none() {
        if case.expect.stack_obj.is_some() {
            return Err(Error::new(
                "expect.stack_obj needs the tree to run as a thread (set `turns` or me = \"sim\")",
            ));
        }
        let result = game
            .lot
            .run_tree(world, thread, tree, stack_obj, &case.params, case.budget);
        let mut observed = match result {
            Ok(b) => Observed::new(if b { Outcome::True } else { Outcome::False }, 0),
            Err(f) => Observed::fault(f.to_string(), 0),
        };
        observed.motive_changes = motive_changes_pending(world, ids.sim);
        return Ok(settled(game, world, observed));
    }

    // A thread: its turns are lot updates, which run every object's turn under the game's
    // own limit.
    if case.budget != DEFAULT_BUDGET {
        return Err(Error::new(format!(
            "budget: a threaded run's turns are lot updates, whose limit is the game's \
             ({TICK_BUDGET} instructions a turn); budget is for synchronous calls"
        )));
    }
    // The tree is the only frame on `thread`'s stack, with the subject's code, sized as a
    // call to it is (`Behavior::frame_dimensions`).
    if behavior.kind == 4 {
        return Err(Error::new(format!(
            "tree {tree} is indexed (kind 4); run it as a synchronous call (me = \"object\", \
             no `turns`)"
        )));
    }
    let (args, locals) = behavior
        .frame_dimensions()
        .map_err(|e| Error::new(format!("tree {tree} cannot be called: {e:?}")))?;
    let owner_type = world
        .types
        .iter()
        .find(|(_, g)| **g == world.objects[&ids.subject].guid)
        .map(|(t, _)| *t)
        .ok_or_else(|| Error::new("the subject's type is not registered"))?;
    // The VM refuses this before it sizes the frame (`call_as`); so does a thread.
    if case.params.len() > MAX_ENTRY_ARGS {
        return Ok(Observed::fault(
            format!("entry arguments exceed the native limit of {MAX_ENTRY_ARGS}"),
            0,
        ));
    }
    let mut parameters = case.params.clone();
    parameters.resize(args, 0);
    world.frame_storage.remove(&thread);
    let o = world.objects.get_mut(&thread).expect("thread exists");
    o.stack = vec![Frame {
        stack_object: stack_obj,
        tree,
        node: 0,
        parameters,
        locals: vec![0; locals],
        primitive_state: 0,
        code_owner_type: owner_type as i16,
    }];
    // A countdown the replaced thread's Sleep left goes with that thread: kept, it would end
    // the tree's first Sleep at once (a countdown of 0 is a finished one). -1 is no countdown,
    // as the factory leaves a new object (`sims-vm`'s `reset`).
    o.set_countdown(-1);

    let max_turns = case.turns.unwrap_or(1);
    for t in 1..=max_turns {
        let observed = match real_turn(game, world, ids, thread, t).1 {
            Step::Went => continue,
            Step::Returned(b) => Observed {
                stack_obj: returned_stack_obj(world, thread, tree),
                ..Observed::new(if b { Outcome::True } else { Outcome::False }, t)
            },
            Step::Fault(msg) => Observed::fault(msg, t),
            Step::Stopped(why) => return Ok(Observed::stopped(why, t)),
        };
        // The update stopped where the thread returned or faulted, before its end showed
        // the dialogs made in it.
        return Ok(settled(game, world, observed));
    }
    let note = match case.turns {
        None => "the tree yielded; a synchronous run must finish in one turn (set `turns` to \
                 let it run longer)"
            .to_owned(),
        Some(n) => format!("still running after {n} turns"),
    };
    Ok(Observed {
        note: Some(note),
        ..Observed::new(Outcome::Yield, max_turns)
    })
}

/// How the Sim's picked action has gone, as seen after each instruction of its thread: it
/// started once it was the running action, and it finished once something else took that
/// place (or it became the last action). Looking after every instruction catches an action
/// that starts and finishes within one turn, and one whose successor (queued at the front)
/// also finishes in that turn, taking its place as the last action.
#[derive(Default, Clone)]
struct Progress {
    started: bool,
    finished: bool,
}

impl Progress {
    fn note(&mut self, world: &World, sim: u16, uid: i32) {
        let Some(p) = world.objects.get(&sim).and_then(|o| o.person.as_ref()) else {
            return;
        };
        if p.active.uid == uid {
            self.started = true;
        } else if self.started || p.last.uid == uid {
            self.started = true;
            self.finished = true;
        }
    }
}

fn run_interaction(
    lot: &Lot,
    game: &mut Game<'_>,
    world: &mut World,
    case: &TestCase,
    ids: &Ids,
    name: &str,
) -> Result<(Observed, Option<i32>)> {
    let sim = ids
        .sim
        .ok_or_else(|| Error::new("an interaction needs a Sim"))?;
    // An outcome decided before the first turn: no person tick runs after it.
    let before_turns = |world: &World, observed: Observed| Observed {
        motive_changes: motive_changes_pending(world, Some(sim)),
        ..observed
    };
    let unavailable = |why: String| Observed {
        note: Some(why),
        ..Observed::new(Outcome::Unavailable, 0)
    };
    // Listing runs the entries' check trees, which may make dialogs: every outcome is
    // reported once they are shown, so a refusing check cannot hide one.
    let entries = match game.between.pie_menu(world, sim, ids.subject) {
        Ok(e) => e,
        Err(f) => {
            // A name the subject's menu cannot have is a mistake in the spec, whatever the
            // listing did.
            if !lot.menu_names_interaction(name) {
                lot.interaction(name)
                    .map_err(|e| Error::new(format!("{e}; listing the pie menu faulted: {f}")))?;
            }
            let o = before_turns(world, Observed::fault(f.to_string(), 0));
            return Ok((settled(game, world, o), None));
        }
    };
    // An entry is selected by the name the menu shows (`Lot::displayed_name`). Its target is
    // the entry's own: a multi-tile object's menu may offer another tile's entry.
    let shown: Vec<Option<String>> = entries
        .iter()
        .map(|e| lot.displayed_name(world, e))
        .collect();
    let statics: Vec<Option<String>> = entries
        .iter()
        .map(|e| lot.menu_name(world, e.action.target, e.action.table_index))
        .collect();
    let mut offered: Vec<_> = entries
        .into_iter()
        .zip(&shown)
        .filter(|(_, n)| n.as_deref() == Some(name))
        .map(|(e, _)| e)
        .collect();
    let entry = match offered.len() {
        0 => {
            if !lot.menu_names_interaction(name) {
                let listed: Vec<String> =
                    shown.iter().flatten().map(|n| format!("\"{n}\"")).collect();
                // Its error lists the names the subject's table has.
                lot.interaction(name).map_err(|e| {
                    Error::new(format!("{e}; its pie menu shows: {}", listed.join(", ")))
                })?;
            }
            // An entry whose table names it `name` but that the menu shows by another name
            // (one its check gave it, or a debug entry's "*").
            let renamed = shown.iter().zip(&statics).find_map(|(shown, table)| {
                (table.as_deref() == Some(name))
                    .then_some(shown.as_deref())
                    .flatten()
                    .filter(|n| *n != name)
            });
            let why = match renamed {
                Some(n) => format!(
                    "the pie menu does not offer \"{name}\"; it shows that entry as \"{n}\""
                ),
                None => format!("the pie menu does not offer \"{name}\""),
            };
            let o = before_turns(world, unavailable(why));
            return Ok((settled(game, world, o), None));
        }
        1 => offered.remove(0),
        n => {
            let targets: Vec<String> = offered.iter().map(|e| ids.name(e.action.target)).collect();
            return Err(Error::new(format!(
                "the pie menu offers \"{name}\" {n} times (on {}); the runner cannot tell \
                 which to pick",
                targets.join(", ")
            )));
        }
    };
    // Whether the entry runs at once is the target table's flag, as `pick` reads it. An
    // at-once action that queues a successor hands `pick` the successor's number (the
    // successor is copied over the running action, 0x45d5a5), so the number alone cannot
    // tell the selection from what it queued.
    let at_once = lot
        .runs_at_once(world, entry.action.target, entry.action.table_index)
        .ok_or_else(|| {
            Error::new(format!(
                "\"{name}\": the target's table has no entry with menu number {}",
                entry.action.table_index
            ))
        })?;
    // Choosing an entry checks it again first (`sims-game`: 0x5eb433 -> 0x48df28), and the
    // action is picked as that check left it.
    let action = match game.between.highlight(world, sim, entry.action) {
        Ok(a) => a,
        // The game checks the selection on a copy of the lot, drops the copy when the check
        // fails, and still picks the entry as the menu listed it (`sims-vm`'s
        // `Lot::highlight`, `sims-game`'s `check_entry`); the original's handling of the
        // error is not known. The runner follows neither.
        Err(f) => {
            let why = format!(
                "\"{name}\": the check run when the entry is selected (0x48df28) faulted ({f}); \
                 the game drops what that check did and picks the entry as listed, which the \
                 runner does not follow"
            );
            return Ok((Observed::stopped(why, 0), None));
        }
    };
    let uid = match game.between.pick(world, sim, action) {
        Ok(Some(uid)) => uid,
        Ok(None) => {
            let o = before_turns(
                world,
                unavailable(format!("the Sim's queue did not take \"{name}\"")),
            );
            return Ok((settled(game, world, o), None));
        }
        Err(f) => {
            let o = before_turns(world, Observed::fault(f.to_string(), 0));
            return Ok((settled(game, world, o), None));
        }
    };
    // An at-once entry ran inside the pick and is done: what it queued stays queued (and
    // is listed by `queue`), and no turn runs it.
    if at_once {
        let o = before_turns(world, Observed::new(Outcome::Ok, 0));
        return Ok((settled(game, world, o), None));
    }
    let mut progress = Progress::default();
    progress.note(world, sim, uid);
    let queued = |world: &World| {
        world.objects[&sim]
            .person
            .as_ref()
            .is_some_and(|p| p.queue.iter().any(|a| a.uid == uid))
    };
    if !queued(world) && !progress.started {
        return Ok((
            Observed::stopped(
                format!(
                    "\"{name}\": sims-vm's pick returned action {uid} for an entry that does \
                     not run at once, and the Sim's queue does not hold it"
                ),
                0,
            ),
            Some(uid),
        ));
    }

    let max_turns = case.turns.unwrap_or(DEFAULT_INTERACTION_TURNS);
    for t in 1..=max_turns {
        // The Sim has the lowest ID, so the update's pass ticks it first: the look-ahead runs
        // its turn on a copy an instruction at a time, to see an action that starts and
        // finishes within the turn.
        let mut ahead = progress.clone();
        let mut copy = world.clone();
        turn(&game.ahead, &mut copy, sim, |w| ahead.note(w, sim, uid));
        let (update, step) = real_turn(game, world, ids, sim, t);
        if update.ticked.first() == Some(&sim) {
            progress = ahead;
        } else if update.ticked.contains(&sim) {
            let why = format!(
                "turn {t}: the lot update ticked another object before the Sim; the runner \
                 cannot follow its action within the turn"
            );
            return Ok((Observed::stopped(why, t), Some(uid)));
        }
        match step {
            Step::Went => {}
            Step::Returned(_) => {
                let o = Observed::fault("the test Sim's main loop returned".to_owned(), t);
                return Ok((settled(game, world, o), Some(uid)));
            }
            Step::Fault(msg) => {
                return Ok((settled(game, world, Observed::fault(msg, t)), Some(uid)));
            }
            Step::Stopped(why) => return Ok((Observed::stopped(why, t), Some(uid))),
        }
        progress.note(world, sim, uid);
        if progress.finished {
            return Ok((Observed::new(Outcome::Ok, t), Some(uid)));
        }
        if !progress.started && !queued(world) {
            return Ok((
                Observed {
                    note: Some(format!(
                        "the Sim dropped \"{name}\" from its queue without starting it"
                    )),
                    ..Observed::new(Outcome::Unavailable, t)
                },
                Some(uid),
            ));
        }
    }
    // An action the Sim never started is not running: `yield` is only for one that started.
    let (outcome, note) = if progress.started {
        (
            Outcome::Yield,
            format!("\"{name}\" was still running after {max_turns} turns"),
        )
    } else {
        (
            Outcome::Unavailable,
            format!(
                "the Sim never started \"{name}\" in {max_turns} turns (it is still queued; a \
                 Sim whose person.priority is not 0 is busy)"
            ),
        )
    };
    Ok((
        Observed {
            note: Some(note),
            ..Observed::new(outcome, max_turns)
        },
        Some(uid),
    ))
}

// ---------------------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------------------

fn words(failures: &mut Vec<String>, what: &str, expected: &BTreeMap<u16, i16>, actual: &[i16]) {
    for (&i, &want) in expected {
        match actual.get(usize::from(i)) {
            Some(&got) if got == want => {}
            Some(&got) => failures.push(format!("{what}.{i}: expected {want}, got {got}")),
            None => failures.push(format!(
                "{what}.{i}: expected {want}, but there are only {} words",
                actual.len()
            )),
        }
    }
}

fn motive_name(i: u8) -> String {
    MOTIVE_NAMES
        .iter()
        .find(|(m, _)| *m == i)
        .map_or_else(|| i.to_string(), |(_, n)| (*n).to_owned())
}

#[allow(clippy::too_many_arguments)]
fn check(
    lot: &Lot,
    world: &World,
    e: &Expect,
    ids: &Ids,
    obs: &Observed,
    sounds: &[SoundEvent],
    dialog_kinds: &[u8],
    queue_before: &BTreeSet<i32>,
    picked: Option<i32>,
    failures: &mut Vec<String>,
) -> Result<()> {
    let got = match (&obs.fault, &obs.note) {
        (Some(f), _) => format!("{} ({f})", obs.outcome),
        (None, Some(n)) => format!("{} ({n})", obs.outcome),
        (None, None) => obs.outcome.to_string(),
    };
    match e.result {
        Some(Outcome::Ok) => {
            if !matches!(obs.outcome, Outcome::Ok | Outcome::True | Outcome::False) {
                failures.push(format!("result: expected ok (no fault), got {got}"));
            }
        }
        Some(want) => {
            if want != obs.outcome {
                failures.push(format!("result: expected {want}, got {got}"));
            }
        }
        None => {
            if matches!(
                obs.outcome,
                Outcome::Error | Outcome::Yield | Outcome::Unavailable
            ) {
                failures.push(format!("result: the run did not finish: {got}"));
            }
        }
    }
    if let Some(want) = &e.error {
        match &obs.fault {
            Some(f) if f.contains(want.as_str()) => {}
            Some(f) => failures.push(format!(
                "error: expected a fault containing \"{want}\", got \"{f}\""
            )),
            None => failures.push(format!(
                "error: expected a fault containing \"{want}\", got none"
            )),
        }
    }
    if let Some(want) = e.turns
        && want != obs.turns
    {
        failures.push(format!("turns: expected {want}, got {}", obs.turns));
    }

    // The subject.
    if let Some(o) = world.objects.get(&ids.subject) {
        for (key, i, want) in lot.attr_indices(&e.attrs, "expect.attrs")? {
            let label = lot.attr_label(key, i);
            match o.attributes.get(usize::from(i)) {
                Some(&got) if got == want => {}
                Some(&got) => failures.push(format!("{label}: expected {want}, got {got}")),
                None => failures.push(format!(
                    "{label}: expected {want}, but the subject has {} attributes",
                    o.attributes.len()
                )),
            }
        }
        words(failures, "temps", &e.temps, &o.temps);
        words(failures, "data", &e.data, &o.data);
    } else if !(e.attrs.is_empty() && e.temps.is_empty() && e.data.is_empty()) {
        failures.push("the subject is no longer on the lot".to_owned());
    }
    words(failures, "globals", &e.globals, &world.globals);
    if let Some(want) = e.funds
        && world.budget.funds != want
    {
        failures.push(format!(
            "funds: expected {want}, got {}",
            world.budget.funds
        ));
    }

    // The Sim.
    if let Some(want) = &e.sim {
        let sim = ids
            .sim
            .and_then(|s| world.objects.get(&s))
            .ok_or_else(|| Error::new("expect.sim: there is no Sim on the lot"))?;
        let person = sim.person.as_ref().expect("the Sim is a person");
        if obs.motive_changes && !want.motives.is_empty() {
            failures.push(
                "sim.motives: the Sim has motive changes over time pending \
                 (person.motive_deltas), and no lot update ran after the code that set them \
                 (a synchronous call, or an interaction that ended before its first turn); the \
                 game's person tick applies them every tick, so its motives are not observable \
                 on this run"
                    .to_owned(),
            );
        }
        for (&m, &v) in &want.motives {
            let got = person.motives[usize::from(m)];
            if got.to_bits() != v.to_bits() {
                failures.push(format!(
                    "sim.motives.{}: expected {v}, got {got}",
                    motive_name(m)
                ));
            }
        }
        words(failures, "sim.person", &want.person, &person.data);
        words(failures, "sim.attrs", &want.attrs, &sim.attributes);
        words(failures, "sim.temps", &want.temps, &sim.temps);
        words(failures, "sim.data", &want.data, &sim.data);
    }

    if let Some(want) = e.stack_obj {
        let want_id = ids.resolve(want)?;
        match &obs.stack_obj {
            Ok(got) if *got == want_id => {}
            Ok(got) => failures.push(format!(
                "stack_obj: expected {want}, got {}",
                ids.name(*got)
            )),
            Err(why) => failures.push(format!("stack_obj: expected {want}, but {why}")),
        }
    }

    if let Some(want) = &e.sounds {
        let got: Vec<&str> = sounds
            .iter()
            .filter_map(|s| match s {
                SoundEvent::Play { name, .. } => Some(name.as_str()),
                SoundEvent::Stop { .. } => None,
            })
            .collect();
        if got != want.iter().map(String::as_str).collect::<Vec<_>>() {
            failures.push(format!("sounds: expected {want:?}, got {got:?}"));
        }
    }

    if let Some(want) = &e.dialogs {
        let fmt_kind = |k: u8| dialog_kind_name(k).map_or_else(|| k.to_string(), |n| n.to_owned());
        let matches = want.len() == dialog_kinds.len()
            && want
                .iter()
                .zip(dialog_kinds)
                .all(|(w, &g)| w.kind == 0xFF || w.kind == g);
        if !matches {
            let want_s: Vec<String> = want
                .iter()
                .map(|w| {
                    if w.kind == 0xFF {
                        "any".to_owned()
                    } else {
                        fmt_kind(w.kind)
                    }
                })
                .collect();
            let got_s: Vec<String> = dialog_kinds.iter().map(|&k| fmt_kind(k)).collect();
            failures.push(format!("dialogs: expected {want_s:?}, got {got_s:?}"));
        }
    }

    if let Some(want) = &e.queue {
        let queue: Vec<&SavedInteraction> = ids
            .sim
            .and_then(|s| world.objects.get(&s))
            .and_then(|o| o.person.as_ref())
            .map(|p| {
                p.queue
                    .iter()
                    .filter(|a| !queue_before.contains(&a.uid) && Some(a.uid) != picked)
                    .collect()
            })
            .unwrap_or_default();
        let describe = |a: &SavedInteraction| {
            let name = lot
                .menu_name(world, a.target, a.table_index)
                .map_or_else(|| format!("menu {}", a.table_index), |n| format!("\"{n}\""));
            format!("{name} on {} (priority {})", ids.name(a.target), a.priority)
        };
        let mut ok = want.len() == queue.len();
        if ok {
            for (w, a) in want.iter().zip(&queue) {
                if let Some(n) = &w.interaction
                    && lot.menu_name(world, a.target, a.table_index).as_deref() != Some(n.as_str())
                {
                    ok = false;
                }
                if let Some(m) = w.menu
                    && i64::from(a.table_index) != i64::from(m)
                {
                    ok = false;
                }
                if let Some(t) = w.target
                    && ids.resolve(t)? != a.target
                {
                    ok = false;
                }
                if let Some(p) = w.priority
                    && i32::from(p) != a.priority
                {
                    ok = false;
                }
            }
        }
        if !ok {
            let got: Vec<String> = queue.iter().map(|a| describe(a)).collect();
            let want_s: Vec<String> = want
                .iter()
                .map(|w| {
                    let mut parts = Vec::new();
                    if let Some(n) = &w.interaction {
                        parts.push(format!("\"{n}\""));
                    }
                    if let Some(m) = w.menu {
                        parts.push(format!("menu {m}"));
                    }
                    if let Some(t) = w.target {
                        parts.push(format!("on {t}"));
                    }
                    if let Some(p) = w.priority {
                        parts.push(format!("(priority {p})"));
                    }
                    if parts.is_empty() {
                        "any".to_owned()
                    } else {
                        parts.join(" ")
                    }
                })
                .collect();
            failures.push(format!(
                "queue: expected [{}], got [{}]",
                want_s.join(", "),
                got.join(", ")
            ));
        }
    }

    check_objects(lot, world, e, ids, failures)?;
    if let Some(want) = &e.room {
        check_room(world, want, failures);
    }
    Ok(())
}

/// `expect.objects`: each slug's objects on the lot, in ID order.
fn check_objects(
    lot: &Lot,
    world: &World,
    e: &Expect,
    ids: &Ids,
    failures: &mut Vec<String>,
) -> Result<()> {
    for (slug, want) in &e.objects {
        let what = format!("objects.{slug}");
        let guid = *lot.slugs.get(slug).ok_or_else(|| {
            Error::new(format!(
                "expect.{what}: no object '{slug}' is loaded (the subject, or add it to \
                 [lot].also)"
            ))
        })?;
        // A multi-tile object is made of tiles with GUIDs of their own: counting the slug's
        // GUID would count one tile, or none, of each instance.
        let group = lot
            .catalog
            .object(guid)
            .and_then(|def| lot.program.definition(def.resource, def.chunk))
            .map_or(0, |words| words[10]);
        if group != 0 {
            return Err(Error::new(format!(
                "expect.{what}: '{slug}' is a multi-tile object (group {group}); \
                 expect.objects counts one-tile objects only"
            )));
        }
        let names = &lot.attr_names[slug];
        let mut attrs = Vec::new();
        let mut seen: BTreeMap<u16, &AttrKey> = BTreeMap::new();
        for (key, &v) in &want.attrs {
            let i = match key {
                AttrKey::Index(i) => *i,
                AttrKey::Name(n) => *names.get(n).ok_or_else(|| {
                    Error::new(format!(
                        "expect.{what}.attrs: '{slug}' has no state '{n}'{}",
                        if names.is_empty() {
                            " (it has no named states; name attributes by index)".to_owned()
                        } else {
                            format!(
                                " (it has: {})",
                                names.keys().cloned().collect::<Vec<_>>().join(", ")
                            )
                        }
                    ))
                })?,
            };
            if let Some(prev) = seen.insert(i, key) {
                return Err(Error::new(format!(
                    "expect.{what}.attrs: '{prev}' and '{key}' both set attribute {i}; give it \
                     once"
                )));
            }
            attrs.push((key, i, v));
        }
        let found: Vec<(u16, &Object)> = world
            .objects
            .iter()
            .filter(|(_, o)| o.guid == guid)
            .map(|(&id, o)| (id, o))
            .collect();
        match want.count {
            Some(n) if usize::from(n) != found.len() => {
                failures.push(format!(
                    "{what}: expected {n}, got {}{}",
                    found.len(),
                    listed(&found, ids)
                ));
                continue;
            }
            None if found.is_empty() => {
                failures.push(format!("{what}: expected at least one, got none"));
                continue;
            }
            _ => {}
        }
        for (id, o) in &found {
            let who = format!("{what} ({})", ids.who(*id));
            for &(key, i, want) in &attrs {
                let label = match key {
                    AttrKey::Index(_) => format!("{who}.attrs.{i}"),
                    AttrKey::Name(n) => format!("{who}.attrs.{n} (attr[{i}])"),
                };
                match o.attributes.get(usize::from(i)) {
                    Some(&got) if got == want => {}
                    Some(&got) => failures.push(format!("{label}: expected {want}, got {got}")),
                    None => failures.push(format!(
                        "{label}: expected {want}, but it has {} attributes",
                        o.attributes.len()
                    )),
                }
            }
            words(failures, &format!("{who}.temps"), &want.temps, &o.temps);
            words(failures, &format!("{who}.data"), &want.data, &o.data);
        }
    }
    Ok(())
}

/// The objects found, for a count message: ` (objects 3, 4)`.
fn listed(found: &[(u16, &Object)], ids: &Ids) -> String {
    if found.is_empty() {
        return String::new();
    }
    let names: Vec<String> = found.iter().map(|(id, _)| ids.who(*id)).collect();
    format!(" ({})", names.join(", "))
}

/// `expect.room`: room 1 as the last room update counted it.
fn check_room(world: &World, want: &RoomExpect, failures: &mut Vec<String>) {
    let Some(room) = world.rooms.rooms.get(&1) else {
        failures.push("room: the lot has no room 1".to_owned());
        return;
    };
    for (key, want, got) in [
        ("score", want.score, room.score.to_f32()),
        ("light", want.light, room.light.to_f32()),
    ] {
        if let Some(want) = want
            && want.to_bits() != got.to_bits()
        {
            failures.push(format!("room.{key}: expected {want}, got {got}"));
        }
    }
    for (key, want, got) in [
        ("lamp_light", want.lamp_light, room.lamp_light),
        ("light_sources", want.light_sources, room.lights),
        ("impact", want.impact, room.impact),
        ("objects", want.objects, room.objects),
    ] {
        if let Some(want) = want
            && want != got
        {
            failures.push(format!("room.{key}: expected {want}, got {got}"));
        }
    }
}
