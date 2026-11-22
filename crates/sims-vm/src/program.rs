use crate::{Error, Result};
use sims_content::{Animations, Catalog, Index, Object, People};
use sims_formats::behavior::{Behavior, InteractionTable};
use sims_formats::career::Level;
use sims_formats::execution::{self, ObjectFunction, ObjectFunctions, Slots};
use sims_formats::iff::Iff;
use sims_sim::TypeAttributes;
use sims_sim::rooms::Tuning;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The SLOT entry kind that makes a routing slot (0x46e3a0).
const ROUTING_SLOT: u16 = 3;
/// The global file's SLOT chunk that holds the lot's global routing slots
/// (0x440630).
const GLOBAL_SLOT_CHUNK: u16 = 100;

/// The code and constants one IFF file provides.
#[derive(Debug, Clone, Default)]
pub struct Library {
    /// BHAV trees by chunk ID.
    pub behaviors: BTreeMap<u16, Behavior>,
    /// BCON constant tables by chunk ID.
    pub constants: BTreeMap<u16, Vec<i16>>,
    /// Each OBJD's callback table, by OBJD chunk ID.
    pub callbacks: BTreeMap<u16, Vec<ObjectFunction>>,
    /// Each OBJD as the original holds it once registered, as 16-bit words,
    /// by OBJD chunk ID. See [`definition_words`].
    pub definitions: BTreeMap<u16, [i16; DEFINITION_WORDS]>,
    /// OBJD chunk IDs in the file's native resource order.
    pub definition_order: Vec<u16>,
    /// Interaction tables (TTAB) by chunk ID. None is a table this reader
    /// cannot decode; using one is an error, having one is not.
    pub interactions: BTreeMap<u16, Option<InteractionTable>>,
    /// How many slots of each SLOT chunk can hold an object (kind 0), by
    /// chunk ID.
    pub containers: BTreeMap<u16, usize>,
    /// Each SLOT chunk's entries, by chunk ID: what an object of a type
    /// whose word 20 names the chunk copies into its slot table (object
    /// +0x13c) and Go To Routing Slot reads.
    pub slots: BTreeMap<u16, Vec<sims_formats::execution::Slot>>,
    /// Each BHAV's name, by chunk ID: what Run Tree By Name looks trees up
    /// by.
    pub names: BTreeMap<u16, String>,
    /// The names in string table 303, the table Run Tree By Name reads
    /// tree names from: its entries in the first language, in order.
    pub tree_names: Vec<String>,
    /// Named tuning tables (FCNS) by chunk ID.
    pub tuning: BTreeMap<u16, sims_formats::tuning::Constants>,
    /// Each string table (STR#) by chunk ID: its entries in the first
    /// language, in order.
    pub strings: BTreeMap<u16, Vec<String>>,
    /// Each catalog string table (CTSS) by chunk ID: every entry, with its
    /// language. A definition's word 41 names its table, whose name the
    /// dialog formatter writes for `$Object` (0x44c1ca, 0x457d3b).
    pub catalog_strings: BTreeMap<u16, Vec<(u8, String)>>,
    /// The string tables (STR#) as the original's loader finds them
    /// (0x46feac), by ID, where that differs from [`Library::strings`]
    /// (the chunks in file order, the last of an ID kept, each table's
    /// first-language entries in file order): the resource of the ID
    /// ([`Iff::get`]: deleted chunks skipped, 0x41c542), with its
    /// first-language entries in the loader's order ([`loaded_order`]), or
    /// None when the file has chunks of the ID but no resource. Only the
    /// dialog's strings read it; whether the other readers of
    /// [`Library::strings`] load through the same loader is **unverified**.
    pub loaded_strings: BTreeMap<u16, Option<Vec<String>>>,
    /// The string tables (STR#) whose resource the loader (0x46feac, the
    /// dialog's call with argument 3 zero) reads in a way not ported, by
    /// ID, each with why ([`unported_strings`]). Only the dialog reads it.
    pub unported_strings: BTreeMap<u16, &'static str>,
    /// The IDs of the file's legacy string tables (CST resources), which
    /// are not read: the original's string loader falls back to one when
    /// the file has no STR# of the ID (0x4702c7, 0x47018e to 0x4701e6).
    /// The loader compares the type with 0x435354, and the header reader
    /// reads the type's bytes big-endian (0x41a5af, 0x472fbf), so the tag
    /// is `\0CST`.
    pub legacy_strings: BTreeSet<u16>,
    /// The IDs of the file's legacy catalog string tables (CATS), which
    /// are not read: the catalog's name lookup (0x4061e3) checks for one
    /// before the CTSS table (**unverified**, from a reading of the
    /// disassembly not checked here).
    pub legacy_catalog_strings: BTreeSet<u16>,
    /// The sound event each FWAV chunk names, by chunk ID: what Play
    /// Sound Event plays (0x582c57). The name is the chunk's bytes up to
    /// the first zero, read as Latin-1.
    pub sounds: BTreeMap<u16, String>,
    /// Whether the file holds a decodable neutral-language `BMP_` 2002
    /// (`0x7d2`, high byte of flags 0) portrait image that `0x4e92a8` →
    /// `0x4e9260` → `0x4dece6` → `0x41cb2f` → `0x41bab1` can load without
    /// calling `0x4e8c5d`.
    pub has_portrait_2002: bool,
    /// Whether the underlying resource file is read-only (e.g. a `.far`
    /// archive member or a read-only file on disk). `0x4e8c5d` checks
    /// writability at `0x4e8cc6` (`0x4067b3` → `0x4174dc`) before calling
    /// `0x4e9652` for an off-lot person, and `0x4e9000` only persists a
    /// generated `BMP_` 2002 to a writable file.
    pub read_only: bool,
}

/// The string table Run Tree By Name instructions name their trees in.
pub const TREE_NAMES: u16 = 303;

/// The global file's tuning table that holds the room score constants.
const ROOM_TUNING: u16 = 3;

/// The global file's tuning table with Find Best Interaction's constants.
const AUTONOMY_TUNING: u16 = 2;

/// The global file's tuning tables with the motive decay rates of the
/// three kinds of person, in kind order.
const MOTIVE_TUNING: [u16; 3] = [1, 5, 6];

/// The global file's string tables with the curves that weigh an adult's
/// and a child's motives in the mood.
const ADULT_MOOD: u16 = 502;
const CHILD_MOOD: u16 = 504;

/// The words in a registered object definition: the original keeps every
/// OBJD as a 216-byte record.
pub const DEFINITION_WORDS: usize = 108;

/// What a tree ID names, seen from an object's file.
#[derive(Debug, Clone, Copy)]
pub enum Tree<'a> {
    /// IDs below 256 are primitives, built into the interpreter.
    Primitive(u16),
    /// A BHAV, with the resource that holds it.
    Behavior(usize, &'a Behavior),
    /// No tree with this ID where the ID points.
    Missing,
}

/// The callbacks an object definition names, in table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Callback {
    Init = 0,
    Main = 1,
    /// Function 7: asked of an object when the lot's rooms have been
    /// worked out again (0x442659).
    RoomsChanged = 7,
}

/// When an OBJD has no OBJf table, the original reads its callbacks from
/// these OBJD byte offsets, in callback order (init, main, load, ...).
const DEFINITION_CALLBACKS: [usize; 30] = [
    0x56, 0x0a, 0x5e, 0x68, 0x78, 0x2a, 0x7c, 0x64, 0x9c, 0x58, 0x94, 0x60, 0x5a, 0x6a, 0x6e, 0x22,
    0x0c, 0x18, 0x30, 0x32, 0x34, 0x36, 0x38, 0x3a, 0x3c, 0x3e, 0x40, 0x42, 0x76, 0xa0,
];

/// The code a set of objects can run.
#[derive(Debug, Clone)]
pub struct Program {
    /// The resource holding `Global.iff`.
    pub global: usize,
    /// Libraries by resource number in the index.
    pub libraries: BTreeMap<usize, Library>,
    /// Each object file's semi-global file, from the catalog.
    pub semi_globals: BTreeMap<usize, Option<usize>>,
    /// The careers in the original's order: one for each CARR chunk of
    /// `work.iff`, in resource order (0x403e3f to 0x403f1f). None when the
    /// install has no `work.iff`, or has a `Careers.txt`, which overrides
    /// the table through a loader that is not ported (0x4036e8).
    pub careers: Option<Vec<Career>>,
    /// Whether the texts a dialog's career tokens may write are known to
    /// hold no career token past their first character, so the career
    /// passes end ([`crate::primitives::dialog::may_hold_career_token`]):
    /// true only when `work.iff` was read for [`Program::careers`] and no
    /// string of it (every STR# entry in every language, and every
    /// career's name and level strings) holds one. That the career
    /// tokens' texts all come from `work.iff` (0x4035d0, 0x4033d2, and
    /// the level's +0x3b4 that 0x404328 fills from its STR# tables) is
    /// **unverified**.
    pub career_texts_end: bool,
    /// Whether the default button labels a dialog shows for a string it
    /// does not name (STR# 142 of `GameData/UIText.iff`, 0x435fee,
    /// 0x43600a) are known to hold no `$`, so no token
    /// ([`labels_hold_no_token`]). When they are not, a dialog with a
    /// default string fails loudly.
    pub labels_hold_no_token: bool,
    /// Each lot's zoning, by lot number: 1 residential, 2 community. It
    /// comes from the neighborhood ([`sims_content::Neighborhood::zoning`]),
    /// so [`Program::link`] leaves it empty for the caller to fill.
    pub zoning: BTreeMap<i16, i32>,
    /// The animation clips persons play, by name. They come from the
    /// install's animation files ([`sims_content::Animations::load`]), so
    /// [`Program::link`] leaves this empty for the caller to fill.
    pub animations: Animations,
    /// The skeletons and profiles of the person types the lot may make or
    /// reset: what a new person's constructor builds and what reading a
    /// person's appearance again reads (0x5888d3). They come from the
    /// install ([`sims_content::People::load`]), so [`Program::link`]
    /// leaves this empty for the caller to fill.
    pub people: People,
    /// What the sprite cache the headlines draw from (\[0x6f0c08\],
    /// made at 0x4144c4 from `GameData/Sprites.iff`) gives as each
    /// sprite's frame count (0x4c8deb), by sprite ID: see
    /// [`icon_frames`]. A sprite the file does not have has 0 frames.
    /// None when it was not read: showing an icon then fails.
    /// [`Program::link`] leaves it None for the caller to fill.
    pub icon_frames: Option<BTreeMap<u16, i32>>,
    /// The suit and texture names Make New Character picks a new
    /// person's body and head from ([`sims_content::Wardrobe`]). None when
    /// they were not read: making a character then fails.
    /// [`Program::link`] leaves it None for the caller to fill.
    pub wardrobe: Option<sims_content::Wardrobe>,
    /// Native-ordered suit and texture lists (`0x409255`, `0x40895a`) for
    /// exact index selection in Make New Character (`0x44b151`, `0x44b167`,
    /// `0x44b07e`), or the error that blocked loading them on a real install
    /// (failing Make New Character loudly rather than falling back to sorted
    /// `wardrobe`). [`Program::link`] leaves it None for synthetic fixtures.
    pub ordered_wardrobe: Option<std::result::Result<sims_content::ImportWardrobe, String>>,
    /// The numbers of the `User%05d.iff` files in the neighborhood's
    /// `Characters` folder: the file numbers Make New Character's copy
    /// finds taken (0x4722b8), besides those of the types it makes.
    /// [`Program::link`] leaves it empty for the caller to fill.
    pub character_files: BTreeSet<i32>,
    /// The installed-expansions mask: the application's word at +0x66
    /// (\[0x6e48bc\]+0x66), which the original sets from the registry at
    /// startup (0x43ae81). The person factory's pet guard reads it
    /// (0x440d8b). [`Program::link`] leaves it 0, no pack installed, for
    /// the caller to fill.
    pub expansions: u16,
    /// The GUIDs of the types registered from the neighborhood's
    /// `Characters` folder, which the original registers with kind 1
    /// (bits 2 and 3 of the type record's +0xb4, 0x424a8b, 0x43c228,
    /// 0x43bd19). [`Program::link`] leaves it empty for the caller to
    /// fill.
    pub characters: BTreeSet<u32>,
}

impl Program {
    /// Loads Global.iff and, for each GUID, its object's file and that
    /// file's semi-global file.
    pub fn link(
        index: &Index,
        catalog: &Catalog,
        guids: impl IntoIterator<Item = u32>,
    ) -> Result<Self> {
        let mut program = Self {
            global: catalog.global,
            libraries: BTreeMap::new(),
            semi_globals: BTreeMap::new(),
            careers: careers(index)?,
            career_texts_end: career_texts_end(index)?,
            labels_hold_no_token: labels_hold_no_token(&index.root)?,
            zoning: BTreeMap::new(),
            animations: Animations::default(),
            people: People::default(),
            icon_frames: None,
            wardrobe: None,
            ordered_wardrobe: None,
            character_files: Default::default(),
            expansions: 0,
            characters: BTreeSet::new(),
        };
        program.load(index, catalog.global)?;
        for guid in guids {
            let object = catalog.object(guid).ok_or(Error::MissingDefinition(guid))?;
            program.load(index, object.resource)?;
            let semi_global = catalog
                .semi_globals
                .get(&object.resource)
                .copied()
                .flatten();
            program.semi_globals.insert(object.resource, semi_global);
            if let Some(resource) = semi_global {
                program.load(index, resource)?;
            }
        }
        Ok(program)
    }

    fn load(&mut self, index: &Index, resource: usize) -> Result<()> {
        if self.libraries.contains_key(&resource) {
            return Ok(());
        }
        let file = &index.resources[resource];
        let location = file.location(&index.root);
        let format = |tag: &str, id: u16, e| {
            sims_content::Error::format(&format!("{location} {tag} #{id}"), e)
        };
        let iff =
            Iff::parse(file.read()?).map_err(|e| sims_content::Error::format(&location, e))?;
        let mut library = Library::default();
        for chunk in &iff.chunks {
            let data = iff.data(chunk);
            match &chunk.tag {
                b"BHAV" => {
                    let behavior =
                        Behavior::parse(data).map_err(|e| format("BHAV", chunk.id, e))?;
                    library.behaviors.insert(chunk.id, behavior);
                    library.names.insert(chunk.id, chunk.label.clone());
                }
                b"STR#" => {
                    let table = match string_table(data, iff.resource_flags(chunk)) {
                        Ok(table) => table,
                        // The loader skips a deleted chunk (0x41c542), so
                        // one that does not parse does not stop the link.
                        Err(_) if chunk.flags & sims_formats::iff::DELETED != 0 => continue,
                        Err(e) => return Err(format("STR#", chunk.id, e).into()),
                    };
                    let entries: Vec<String> = table
                        .entries
                        .iter()
                        .filter(|entry| entry.language == 1)
                        .map(|entry| entry.text.clone())
                        .collect();
                    if chunk.id == TREE_NAMES {
                        library.tree_names.clone_from(&entries);
                    }
                    library.strings.insert(chunk.id, entries);
                }
                b"\0CST" => {
                    if iff.get(b"\0CST", chunk.id).is_some() {
                        library.legacy_strings.insert(chunk.id);
                    }
                }
                b"CTSS" => {
                    let table = sims_formats::strings::StringTable::parse(data)
                        .map_err(|e| format("CTSS", chunk.id, e))?;
                    let entries = table
                        .entries
                        .iter()
                        .map(|entry| (entry.language, entry.text.clone()))
                        .collect();
                    library.catalog_strings.insert(chunk.id, entries);
                }
                b"CATS" => {
                    library.legacy_catalog_strings.insert(chunk.id);
                }
                b"FWAV" => {
                    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                    let name = data[..end].iter().map(|&b| char::from(b)).collect();
                    library.sounds.insert(chunk.id, name);
                }
                b"BCON" => {
                    let constants =
                        execution::constants(data).map_err(|e| format("BCON", chunk.id, e))?;
                    library.constants.insert(chunk.id, constants);
                }
                b"FCNS" => {
                    let table = sims_formats::tuning::Constants::parse(data)
                        .map_err(|e| format("FCNS", chunk.id, e))?;
                    library.tuning.insert(chunk.id, table);
                }
                b"TTAB" => {
                    let table = InteractionTable::parse(data).ok();
                    library.interactions.insert(chunk.id, table);
                }
                b"SLOT" => {
                    let slots = Slots::parse(data).map_err(|e| format("SLOT", chunk.id, e))?;
                    let count = slots.entries.iter().filter(|s| s.kind == 0).count();
                    library.containers.insert(chunk.id, count);
                    library.slots.insert(chunk.id, slots.entries);
                }
                _ => {}
            }
        }
        // The string tables as the original's loader finds them.
        let ids: BTreeSet<u16> = iff
            .chunks
            .iter()
            .filter(|chunk| &chunk.tag == b"STR#")
            .map(|chunk| chunk.id)
            .collect();
        for id in ids {
            let Some(chunk) = iff.get(b"STR#", id) else {
                library.loaded_strings.insert(id, None);
                continue;
            };
            // A file whose resource map does not match its chunks, which
            // the original reads as the map has it (0x41bfdd to 0x41c032),
            // fails the link below (`resource_order`).
            let data = iff.data(chunk);
            if let Some(why) = unported_strings(iff.resource_flags(chunk), data) {
                library.unported_strings.insert(id, why);
                continue;
            }
            let table = sims_formats::strings::StringTable::parse(data)
                .map_err(|e| format("STR#", id, e))?;
            if english_fallback(&table.entries) {
                library.unported_strings.insert(
                    id,
                    "has entries with a language and none in US English, which the loader \
                     reads again for English (0x47006b to 0x4700d0)",
                );
                continue;
            }
            let loaded = loaded_order(&table.entries);
            if library.strings.get(&id) != Some(&loaded) {
                library.loaded_strings.insert(id, Some(loaded));
            }
        }
        // Portrait bitmap 2002 (0x7d2) read by 0x4e92a8 -> 0x4e9260 -> 0x4dece6
        // -> 0x4ded4a -> 0x41cb2f -> 0x41bab1: 0x41bad8 zeros the lookup key's
        // high byte, so only a neutral-language (high byte 0) BMP_ 2002 matches.
        library.has_portrait_2002 = iff
            .get(b"BMP_", 2002)
            .is_some_and(|chunk| is_native_portrait_bmp(iff.data(chunk)));
        // Archive members (.far) and read-only loose files report IsWritable ==
        // false at 0x4067b3 -> 0x4174dc.
        library.read_only = file.member.is_some()
            || std::fs::metadata(&file.path)
                .map(|m| m.permissions().readonly())
                .unwrap_or(true);
        // The native resource order (the rsmp map when there is one), as the
        // catalog registers definitions. The old prototype kept physical
        // file order instead (legacy/crates/sims-content/src/vm.rs:187).
        let definitions = iff
            .resource_order(b"OBJD")
            .map_err(|e| sims_content::Error::format(&location, e))?;
        for chunk in definitions {
            let words =
                definition_words(iff.data(chunk)).map_err(|e| format("OBJD", chunk.id, e))?;
            library.definition_order.push(chunk.id);
            library.definitions.insert(chunk.id, words);
            let callbacks = match iff.get(b"OBJf", chunk.id) {
                Some(table) => {
                    ObjectFunctions::parse(iff.data(table))
                        .map_err(|e| format("OBJf", table.id, e))?
                        .entries
                }
                // The fallback reads the normalized record
                // (legacy/crates/sims-content/src/vm.rs:351-370).
                None => DEFINITION_CALLBACKS
                    .iter()
                    .map(|&offset| ObjectFunction {
                        condition: 0,
                        action: words[offset / 2] as u16,
                    })
                    .collect(),
            };
            library.callbacks.insert(chunk.id, callbacks);
        }
        self.libraries.insert(resource, library);
        Ok(())
    }

    /// The constants the room scores are worked out with (0x4665d1): the
    /// global file's tuning table 3, read by name, with the original's
    /// default for each name the table lacks.
    ///
    /// Ported from the old prototype's `Tuning::from_program`
    /// (legacy/crates/sims-sim/src/lot_environment.rs).
    pub fn room_tuning(&self) -> Tuning {
        let table = self
            .libraries
            .get(&self.global)
            .and_then(|library| library.tuning.get(&ROOM_TUNING));
        Tuning::from_lookup(|name| table.and_then(|table| table.get(name)))
    }

    /// The constants a person's motives are updated with
    /// ([`sims_sim::motives::Tuning`]): the decay rates from the global
    /// file's tuning tables 1, 5 and 6, read by name, and the mood curves
    /// from its string tables 502 (adult) and 504 (child), one curve per
    /// entry for the motives in [`sims_sim::motives::MOOD_MOTIVES`] order.
    ///
    /// Ported from the old prototype's `Tuning::from_program`
    /// (legacy/crates/sims-vm/src/motives.rs). The original loads these
    /// when the game starts (rates: 0x420c36). That the curves come from
    /// these tables in this order is the prototype's finding, compared
    /// there with the original's tables at 0x6e51f8 and 0x6e5160 in a
    /// memory image; here it is checked only through the mood a decay
    /// works out, which equals the original's (`docs/decisions/`).
    pub fn motive_tuning(&self) -> Result<sims_sim::motives::Tuning> {
        use sims_sim::motives::{Curve, MOOD_MOTIVES, MOOD_POINTS, Rates, Tuning};
        let global = self.libraries.get(&self.global);
        let rates = MOTIVE_TUNING.map(|id| {
            let table = global.and_then(|library| library.tuning.get(&id));
            (id, table)
        });
        if let Some((id, _)) = rates.iter().find(|(_, table)| table.is_none()) {
            return Err(Error::Unsupported(format!(
                "the global file has no tuning table {id} with motive rates"
            )));
        }
        let curves = |id: u16| -> Result<Vec<Curve>> {
            let strings = global
                .and_then(|library| library.strings.get(&id))
                .ok_or_else(|| {
                    Error::Unsupported(format!("the global file has no string table {id}"))
                })?;
            MOOD_MOTIVES
                .iter()
                .enumerate()
                .map(|(i, &motive)| {
                    let text = strings.get(i).ok_or_else(|| {
                        Error::Unsupported(format!("string table {id} has no entry {i}"))
                    })?;
                    Ok(Curve::parse(motive, text, MOOD_POINTS))
                })
                .collect()
        };
        let mut profile = 0;
        let rates = rates.map(|(_, table)| {
            let made = Rates::from_lookup(profile, |name| table.and_then(|t| t.get(name)));
            profile += 1;
            made
        });
        Ok(Tuning {
            rates,
            adult_mood: curves(ADULT_MOOD)?,
            child_mood: curves(CHILD_MOOD)?,
        })
    }

    /// The constants Find Best Interaction works with
    /// ([`crate::autonomy::Constants`]): the global file's tuning table 2,
    /// read by name, with the original's default for each name the table
    /// lacks.
    ///
    /// The original reads them once (0x459120): it looks each name up
    /// ignoring the case of A to Z (0x4131f4), adds a missing one with its
    /// default, and stores the values at 0x6c4384 to 0x6c4d94. The
    /// random selection count is stored as a whole number, truncated
    /// (`__ftol`). Names, addresses and defaults are the old prototype's
    /// (legacy/crates/sims-vm/src/autonomy_constants.rs, `SPECS`). The
    /// values read here were compared with the stored ones in a memory
    /// image of the original (`docs/decisions/`).
    pub fn autonomy_constants(&self) -> crate::autonomy::Constants {
        use crate::autonomy::Constants;
        use sims_core::{Ext, F32};
        let table = self
            .libraries
            .get(&self.global)
            .and_then(|library| library.tuning.get(&AUTONOMY_TUNING));
        let value = |name: &str, default: u32| {
            table
                .and_then(|table| {
                    table
                        .entries
                        .iter()
                        .find(|entry| entry.name.eq_ignore_ascii_case(name))
                })
                .map_or(F32::from_bits(default), |entry| F32::from_f32(entry.value))
        };
        Constants {
            family_threshold: value("min autonomy score for family", 0x3e4c_cccd),
            visitor_threshold: value("min autonomy score for visitors", 0x3d4c_cccd),
            attenuation: [
                value("low attenuation", 0x3b03_126f),
                value("moderate attenuation", 0x3ca3_d70a),
                value("high attenuation", 0x3dcc_cccd),
            ],
            visitor_attenuation: [
                value("visitor low attenuation", 0x3b03_126f),
                value("visitor moderate attenuation", 0x3ca3_d70a),
                value("visitor high attenuation", 0x3dcc_cccd),
            ],
            sitting_threshold: value("min autonomy score for sitting", 0x3e4c_cccd),
            random_count: Ext::from_f32(value("random selection count", 0x4120_0000))
                .to_i32_truncate(),
            distance_attenuation: value("functional distance attenuation", 0x4040_0000),
            friendship_threshold: Ext::from_f32(value("friendship threshold", 0x41c8_0000))
                .to_i32_truncate(),
        }
    }

    /// A person's score curves: the curves Find Best Interaction weighs
    /// its motives with. They are the entries of a string table, one per
    /// motive in [`crate::autonomy::SCORE_MOTIVES`] order, each holding up
    /// to four points ([`sims_sim::motives::Curve::parse`]). `global` and
    /// `table` say which table ([`crate::curve_table`]); a table that is
    /// not the global file's is the person's own file's, `resource`.
    ///
    /// The order and the four points are the old prototype's
    /// (`Tuning::from_program` in legacy/crates/sims-vm/src/motives.rs).
    /// The curves read here were compared, point for point, with people's
    /// curves in a memory image of the original (`docs/decisions/`).
    pub fn score_curves(
        &self,
        global: bool,
        resource: usize,
        table: u16,
    ) -> std::result::Result<Vec<sims_sim::motives::Curve>, String> {
        use crate::autonomy::{SCORE_MOTIVES, SCORE_POINTS};
        let file = if global { self.global } else { resource };
        let strings = self
            .libraries
            .get(&file)
            .and_then(|library| library.strings.get(&table))
            .ok_or_else(|| format!("resource {file} has no string table {table}"))?;
        SCORE_MOTIVES
            .iter()
            .enumerate()
            .map(|(i, &motive)| {
                let text = strings
                    .get(i)
                    .ok_or_else(|| format!("string table {table} has no entry {i}"))?;
                Ok(sims_sim::motives::Curve::parse(motive, text, SCORE_POINTS))
            })
            .collect()
    }

    /// Resolves a tree ID for code running on behalf of the object whose
    /// file is `owner`: 256..4096 are global, 4096..8192 the object's own,
    /// and 8192 up its semi-global file's. Calls made from inside a global
    /// or semi-global tree still resolve from the object's file.
    pub fn tree(&self, owner: usize, id: u16) -> Tree<'_> {
        let resource = match id {
            0..=255 => return Tree::Primitive(id),
            256..=4095 => self.global,
            4096..=8191 => owner,
            _ => match self.semi_globals.get(&owner).copied().flatten() {
                Some(resource) => resource,
                None => return Tree::Missing,
            },
        };
        match self
            .libraries
            .get(&resource)
            .and_then(|l| l.behaviors.get(&id))
        {
            Some(behavior) => Tree::Behavior(resource, behavior),
            None => Tree::Missing,
        }
    }

    /// A tuning constant, addressed as BCON table (high 9 bits) and entry
    /// (low 7 bits): tables 0..64 are the object's own BCONs from 4096,
    /// 64..128 its semi-global's from 8192, and 128..192 the global ones
    /// from 256.
    pub fn constant(&self, owner: usize, address: u16) -> Option<i16> {
        let table = address >> 7;
        let entry = usize::from(address & 127);
        let (resource, id) = match table {
            0..=63 => (owner, table + 4096),
            64..=127 => (
                self.semi_globals.get(&owner).copied().flatten()?,
                table - 64 + 8192,
            ),
            128..=191 => (self.global, table - 128 + 256),
            _ => return None,
        };
        self.libraries
            .get(&resource)?
            .constants
            .get(&id)?
            .get(entry)
            .copied()
    }

    /// An object definition's words, by the resource and OBJD chunk ID that
    /// hold it.
    pub fn definition(&self, resource: usize, chunk: u16) -> Option<&[i16; DEFINITION_WORDS]> {
        self.libraries.get(&resource)?.definitions.get(&chunk)
    }

    /// The interaction table an object definition uses: the TTAB its word 7
    /// names, in its own file or else its semi-global file. A tile of a
    /// multi-tile object whose word 7 is -1 uses its group's catalog
    /// definition's table (0x45726a, 0x43ae0a). Ok(None) is no table.
    ///
    /// Ported from the old prototype (legacy/crates/sims-content/src/vm.rs:448-490).
    pub fn interactions(
        &self,
        resource: usize,
        chunk: u16,
    ) -> std::result::Result<Option<&InteractionTable>, String> {
        let Some(library) = self.libraries.get(&resource) else {
            return Err(format!("resource {resource} is not linked"));
        };
        let Some(words) = library.definitions.get(&chunk) else {
            return Err(format!("resource {resource} has no OBJD #{chunk}"));
        };
        let mut table = words[7];
        if table == -1 {
            table = (words[10] != 0)
                .then(|| group_master(library, words))
                .flatten()
                .map_or(0, |parent| parent[7]);
        }
        if table <= 0 {
            return Ok(None);
        }
        let id = table as u16;
        let semi_global = self.semi_globals.get(&resource).copied().flatten();
        let found = library.interactions.get(&id).or_else(|| {
            self.libraries
                .get(&semi_global?)
                .and_then(|l| l.interactions.get(&id))
        });
        match found {
            None => Ok(None),
            Some(Some(table)) => Ok(Some(table)),
            Some(None) => Err(format!(
                "TTAB #{id} for OBJD #{chunk} of resource {resource} has no reader"
            )),
        }
    }

    /// The definition an object's definition stands for (0x45728d, through
    /// 0x43ae0a): itself when its word 10 (the group) is 0; otherwise the
    /// first definition of its file, in the file's order, of the same
    /// group whose word 11 is -1 (0x43adf7), the group's master. Ok(None)
    /// is a tile whose group has no master, for which the original's
    /// callers read through a null pointer.
    pub fn master_definition(
        &self,
        resource: usize,
        chunk: u16,
    ) -> std::result::Result<Option<&[i16; DEFINITION_WORDS]>, String> {
        let Some(library) = self.libraries.get(&resource) else {
            return Err(format!("resource {resource} is not linked"));
        };
        let Some(words) = library.definitions.get(&chunk) else {
            return Err(format!("resource {resource} has no OBJD #{chunk}"));
        };
        if words[10] == 0 {
            return Ok(Some(words));
        }
        Ok(group_master(library, words))
    }

    /// How many objects an object definition's slots can hold: the kind 0
    /// entries of the SLOT chunk its word 20 names, or none when the file
    /// has no such chunk (0x431aaf, 0x46e2d1).
    pub fn containers(&self, resource: usize, chunk: u16) -> usize {
        let Some(library) = self.libraries.get(&resource) else {
            return 0;
        };
        library
            .definitions
            .get(&chunk)
            .and_then(|words| library.containers.get(&(words[20] as u16)))
            .copied()
            .unwrap_or(0)
    }

    /// The SLOT entries an object definition's word 20 names, or none when
    /// the file has no such chunk.
    pub fn slots(&self, resource: usize, chunk: u16) -> &[sims_formats::execution::Slot] {
        self.libraries
            .get(&resource)
            .and_then(|library| {
                let words = library.definitions.get(&chunk)?;
                library.slots.get(&(words[20] as u16))
            })
            .map_or(&[], Vec::as_slice)
    }

    /// Routing slot `index` of an object definition, with its index in the
    /// SLOT chunk: an object's routing slot table (+0x13c) holds the
    /// chunk's entries of kind 3, in order, each with its index plus one
    /// as its number (the SLOT parse at construction, 0x46e2d1). None
    /// when there is no such entry.
    pub fn routing_slot(
        &self,
        resource: usize,
        chunk: u16,
        index: usize,
    ) -> Option<(usize, &sims_formats::execution::Slot)> {
        self.slots(resource, chunk)
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.kind == ROUTING_SLOT)
            .nth(index)
    }

    /// The lot's global routing slot `index`, with its index in the SLOT
    /// chunk: the lot's table (+0x9c) holds the kind 3 entries of SLOT 100
    /// of the global file, in order, as an object's table holds its type's
    /// (made as the lot loads, 0x440635 and 0x4430e9). None when there is
    /// no such entry.
    pub fn global_routing_slot(
        &self,
        index: usize,
    ) -> Option<(usize, &sims_formats::execution::Slot)> {
        self.libraries
            .get(&self.global)?
            .slots
            .get(&GLOBAL_SLOT_CHUNK)?
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.kind == ROUTING_SLOT)
            .nth(index)
    }

    /// The type attribute records the catalog makes, all zero: one for
    /// every definition whose word 90 asks for a number of words, in
    /// catalog order, two definitions with one GUID included (0x43ba52,
    /// 0x455f09). A count outside 0..32000 makes a record with no words.
    /// Every catalog definition must be linked.
    ///
    /// Ported from the old prototype (`resource_storage` in
    /// legacy/crates/sims-content/src/object_catalog/household.rs). A definition
    /// whose word 91 has bit 1 set wants its words cleared for each lot.
    /// When the original does that is not known, so one is an error; the
    /// Complete Collection has none.
    pub fn type_attributes(&self, catalog: &Catalog) -> Result<Vec<TypeAttributes>> {
        let mut records = Vec::new();
        for object in &catalog.objects {
            let words = self
                .definition(object.resource, object.chunk)
                .ok_or(Error::MissingDefinition(object.guid))?;
            let count = words[90];
            if count == 0 {
                continue;
            }
            if words[91] & 2 != 0 {
                return Err(Error::Unsupported(format!(
                    "GUID 0x{:08X} resets its type attributes for each lot",
                    object.guid
                )));
            }
            let count = usize::try_from(count).ok().filter(|&n| n < 32000);
            records.push(TypeAttributes {
                guid: object.guid,
                values: vec![0; count.unwrap_or(0)],
            });
        }
        Ok(records)
    }

    /// The tree with a name, looked up for an object whose file is
    /// `owner` (0x4021f2): in that file, then its semi-global file, then
    /// the global file, first by the exact name and then ignoring case.
    /// Returns the tree's ID, or None. Two trees with the name in one file
    /// are an error: which one the original takes is not known.
    pub fn named_tree(&self, owner: usize, name: &str) -> std::result::Result<Option<u16>, String> {
        let files = [
            Some(owner),
            self.semi_globals.get(&owner).copied().flatten(),
            Some(self.global),
        ];
        for exact in [true, false] {
            for file in files.into_iter().flatten() {
                let Some(library) = self.libraries.get(&file) else {
                    continue;
                };
                let mut found = library.names.iter().filter(|(_, label)| {
                    if exact {
                        label.as_str() == name
                    } else {
                        label.eq_ignore_ascii_case(name)
                    }
                });
                if let Some((&id, _)) = found.next() {
                    if found.next().is_some() {
                        return Err(format!("two trees are named {name:?} in one file"));
                    }
                    return Ok(Some(id));
                }
            }
        }
        Ok(None)
    }

    /// The tree an object's callback names, or 0 when it names none.
    pub fn callback(&self, object: &Object, callback: Callback) -> u16 {
        self.libraries
            .get(&object.resource)
            .and_then(|l| l.callbacks.get(&object.chunk))
            .and_then(|table| table.get(callback as usize))
            .map_or(0, |f| f.action)
    }

    /// The original's behavior signature (0x4026ed): the root tree's
    /// revision shifted left 16, plus the revision of every distinct tree
    /// reachable from the root through calls, the root included. Missing
    /// trees add nothing, and the sum wraps.
    pub fn signature(&self, owner: usize, root: u16) -> u32 {
        let revision = |id| match self.tree(owner, id) {
            Tree::Behavior(_, behavior) => behavior.revision,
            _ => 0,
        };
        let mut value = revision(root).wrapping_shl(16);
        let mut seen = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            value = value.wrapping_add(revision(id));
            if let Tree::Behavior(_, behavior) = self.tree(owner, id) {
                for instruction in &behavior.instructions {
                    let target = instruction.opcode & 0x7fff;
                    if target >= 256 {
                        pending.push(target);
                    }
                }
            }
        }
        value
    }
}

/// A career as the original holds it (a record of 0x74 bytes in the list
/// at 0x6de848).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Career {
    /// The CARR chunk's ID, sign-extended (0x403eff).
    pub id: i32,
    /// Its levels, as the chunk holds them (0x4035e7; records of 0x3c0
    /// bytes, the 21 numbers first, then the strings from +0x54).
    pub levels: Vec<Level>,
}

/// The careers of `work.iff`, or None when the table cannot be had from
/// it. See [`Program::careers`].
/// The numbers of the `User%05d.iff` files directly in `characters`, the
/// neighborhood's `Characters` folder as the index lists it
/// ([`sims_content::Index::add_characters`]), matching the name without
/// regard to ASCII case, as the file system does. These are the files
/// Make New Character's copy finds there (0x4722b8 returning -0x2b).
pub fn character_files(index: &sims_content::Index, characters: &Path) -> BTreeSet<i32> {
    let Some(mount) = index
        .mounts
        .iter()
        .position(|m| m.path == sims_content::Index::CHARACTERS)
    else {
        return BTreeSet::new();
    };
    index
        .resources
        .iter()
        .filter(|r| r.mount == mount && r.member.is_none())
        .filter(|r| r.path.parent() == Some(characters))
        .filter_map(|r| {
            let name = r.name();
            let name = name.to_ascii_lowercase();
            let digits = name.strip_prefix("user")?.strip_suffix(".iff")?;
            let plain = digits.len() == 5 && digits.bytes().all(|b| b.is_ascii_digit());
            plain.then(|| digits.parse().ok()).flatten()
        })
        .collect()
}

/// The file the headlines' sprite cache reads: the game data directory
/// (0x415336 entry 0) and "sprites" (entry 1), joined at 0x4ee8fe.
pub const ICON_FILE: &str = "GameData/Sprites.iff";

/// Reads the frame count the headlines' sprite cache gives each sprite of
/// [`ICON_FILE`] (0x4c8deb), by sprite ID. None when the install has no
/// such file.
pub fn icon_frames(install: &sims_content::Install) -> Result<Option<BTreeMap<u16, i32>>> {
    let path = match install.file(ICON_FILE) {
        Ok(path) => path,
        Err(sims_content::Error::Missing { .. }) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let bytes = std::fs::read(&path).map_err(|source| sims_content::Error::Io {
        path: path.clone(),
        source,
    })?;
    let location = path.display().to_string();
    let iff = Iff::parse(bytes).map_err(|e| sims_content::Error::format(&location, e))?;
    sprite_frames(&iff).map(Some)
}

/// The frame count the sprite cache (0x4c8e0d) gives each sprite of
/// `iff`, by sprite ID, as 0x4c8deb hands it on: 0 unless the header's
/// version (sprite +0x20) is at least 500.
///
/// The cache looks up the resources by ID ([`Iff::get`]: live chunks of
/// the default variant, through the resource map when there is one), an
/// SPR# of the ID first and an SPR2 only when there is none. For an SPR#
/// the sprite (0x4c154e) takes the version and the count from the
/// header's first two words, then halves the count for versions 501 and
/// 503 and divides it by five for 506 (0x4c1604 to 0x4c1631). An SPR2's
/// are its first two words as they stand (0x4c1757).
///
/// The resource layer hands an SPR# to a callback that swaps the header's
/// bytes (0x4c8f63) unless the resource's conversion flag, 0x10, is set
/// (0x41d3a7): the header is big-endian without the flag and
/// little-endian with it. The flag is read from the chunk's header; with
/// a resource map, whether the map's entry can carry different flags is
/// **unverified**. The install's SPR# in `Sprites.iff` all have flags 0
/// and version 502 written big-endian.
pub fn sprite_frames(iff: &Iff) -> Result<BTreeMap<u16, i32>> {
    let header = |data: &[u8]| -> Option<[[u8; 4]; 2]> {
        Some([
            *data.first_chunk::<4>()?,
            *data.get(4..8)?.first_chunk::<4>()?,
        ])
    };
    let resources = |tag: &'static [u8; 4]| {
        let ids: std::collections::BTreeSet<u16> = iff.all(tag).map(|c| c.id).collect();
        ids.into_iter().filter_map(move |id| iff.get(tag, id))
    };
    let mut frames = BTreeMap::new();
    for chunk in resources(b"SPR#") {
        let [version, count] = header(iff.data(chunk))
            .ok_or_else(|| Error::Unsupported(format!("SPR# #{} has no header", chunk.id)))?;
        let (version, count) = if chunk.flags & 0x10 != 0 {
            (i32::from_le_bytes(version), i32::from_le_bytes(count))
        } else {
            (i32::from_be_bytes(version), i32::from_be_bytes(count))
        };
        let count = match version {
            0x1f5 | 0x1f7 => count / 2,
            0x1fa => count / 5,
            _ => count,
        };
        frames.insert(chunk.id, if version >= 500 { count } else { 0 });
    }
    for chunk in resources(b"SPR2") {
        if frames.contains_key(&chunk.id) {
            continue;
        }
        let [version, count] = header(iff.data(chunk))
            .ok_or_else(|| Error::Unsupported(format!("SPR2 #{} has no header", chunk.id)))?;
        let (version, count) = (i32::from_le_bytes(version), i32::from_le_bytes(count));
        frames.insert(chunk.id, if version >= 500 { count } else { 0 });
    }
    Ok(frames)
}

/// The master of a tile's group in its file (0x43ae0a): the first
/// definition, in the file's order, of the same group (word 10) whose word
/// 11 is -1 (0x43adf7).
fn group_master<'a>(
    library: &'a Library,
    words: &[i16; DEFINITION_WORDS],
) -> Option<&'a [i16; DEFINITION_WORDS]> {
    library
        .definition_order
        .iter()
        .filter_map(|id| library.definitions.get(id))
        .find(|parent| parent[10] == words[10] && parent[11] == -1)
}

fn careers(index: &Index) -> Result<Option<Vec<Career>>> {
    if index.named("Careers.txt").next().is_some() {
        return Ok(None);
    }
    let Some((_, file)) = index.named("work.iff").next() else {
        return Ok(None);
    };
    let location = file.location(&index.root);
    let iff = Iff::parse(file.read()?).map_err(|e| sims_content::Error::format(&location, e))?;
    let chunks = iff
        .resource_order(b"CARR")
        .map_err(|e| sims_content::Error::format(&location, e))?;
    chunks
        .iter()
        .map(|chunk| {
            let levels = sims_formats::career::Career::parse(iff.data(chunk))
                .map_err(|e| {
                    sims_content::Error::format(&format!("{location} CARR #{}", chunk.id), e)
                })?
                .levels;
            Ok(Career {
                id: i32::from(chunk.id as i16),
                levels,
            })
        })
        .collect::<Result<_>>()
        .map(Some)
}

/// Why the string loader (0x46feac, called with argument 3 zero) reads a
/// STR# resource with flags `flags` ([`Iff::resource_flags`]: the resource map's
/// when it has one) and bytes `data` in a way not ported, or None when it reads it as [`loaded_order`] does: a table of
/// format -1, -2 or -3 (0x46ff37, 0x46ff7f, 0x46ffcc) with a count read
/// as a signed word (`jle` at 0x46ff50, 0x46ff92) up to 0x7fff, whose
/// header the resource reader leaves as it is. It byte-swaps the header of
/// a resource without flag 0x10 (0x41d3a7 to 0x41d3d4, through 0x470351;
/// the flags are the map's, 0x41b7fa to 0x41b813, by Sol's reading,
/// **unverified**; every STR# 301 of the recorded install has the flag), and reads any other format with its
/// leading word as the count and bytes 0xa1 and 0xd5 rewritten
/// (0x4700d7 to 0x47014c), so a version-0 table has no entries.
pub fn unported_strings(flags: u16, data: &[u8]) -> Option<&'static str> {
    if flags & 0x10 == 0 {
        return Some("is a chunk without flag 0x10, whose header the loader byte-swaps (0x41d3a7)");
    }
    let word = |at: usize| {
        data.get(at..at + 2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
    };
    match word(0) {
        Some(-3..=-1) => {}
        _ => {
            return Some(
                "has a format other than -1, -2 and -3, which the loader reads with its leading \
                 word as the count (0x4700d7 to 0x47014c)",
            );
        }
    }
    match word(2) {
        Some(count) if count >= 0 => None,
        Some(_) => Some("has a count above 0x7fff, which the loader reads as none (0x46ff50)"),
        None => Some("has no count, and the loader reads one past its end (0x46ff42)"),
    }
}

/// Whether the string loader (0x46feac) reads a table with entries
/// `entries` again for English, a way not ported: it has entries, and none
/// is in the wanted language, US English (language 1, which the reader
/// also gives an entry of language 0). With its fallback on, as for the
/// dialog's default labels (0x435fee) and the careers' strings (0x403f37),
/// the loader then scans the table again (0x47006b to 0x4700d0) in a way
/// that can read a string's bytes as a language byte.
fn english_fallback(entries: &[sims_formats::strings::TextEntry]) -> bool {
    !entries.is_empty() && entries.iter().all(|entry| entry.language != 1)
}

/// The first-language entries of a string table, in the order the
/// original's loader (0x46feac, for a table of entries with a language,
/// 0x46ffcc to 0x470069) puts them in: it counts the entries of each run of
/// one language from 1 (the count starts again where the language
/// changes, 0x470002 to 0x47000b), and inserts each entry of the wanted
/// language at its place in its run (0x470482, which clamps the place to
/// the entries so far plus one and moves the rest along). A later run
/// therefore goes in front of the earlier ones. An entry with language 0
/// counts as the wanted language (0x46fffc), as the reader gives it 1
/// ([`sims_formats::strings::StringTable`]). The wanted language is US
/// English (see [`Library::strings`]); its fallback to the English entries
/// when it has none (0x47006b to 0x4700d0) adds nothing for it.
pub fn loaded_order(entries: &[sims_formats::strings::TextEntry]) -> Vec<String> {
    let mut loaded: Vec<String> = Vec::new();
    let mut run = None;
    let mut place = 0;
    for entry in entries {
        if run != Some(entry.language) {
            run = Some(entry.language);
            place = 0;
        }
        if entry.language == 1 {
            loaded.insert(place.min(loaded.len()), entry.text.clone());
            place += 1;
        }
    }
    loaded
}

/// Whether no text the career passes may write holds a career token past
/// its first character ([`Program::career_texts_end`]). The careers'
/// strings come from the `Work` resource set of `ExpansionShared`
/// (0x403da2 to 0x403dd9, the directory found at 0x415336), which the
/// composite opener makes of every component it finds: `.iff`, `.fam`,
/// `.wll`, `.flr`, loose or in an archive there, and `.dir`, a filesystem
/// directory, which the directory reader (selected at 0x46f50f and
/// 0x46b5d9) lists with a `*.*` wildcard (0x5c3099 to 0x5c3102)
/// (0x46f50f to 0x46f542, opened at 0x46b627 to 0x46b699, searched at
/// 0x406a29). Only one component is ported, so this is false unless that
/// directory holds exactly one entry or archive member named `work` with
/// any extension ([`resource_set`]), `work.iff`, and it is the index's
/// only `work.iff`, which [`careers`] reads. (A `.dir` that is a file or
/// an archive member, which the original does not open as one, makes it
/// false too: a check stricter than need be, not a port.) So do
/// a `Careers.txt` (0x403e31). Every string table of it (STR#, and CST,
/// which the loader falls back to, 0x4702c7) must hold no `$` byte at all,
/// deleted chunks included: the loader's ways not ported
/// ([`unported_strings`]) may read a string's bytes as entries. Every
/// career's name and level strings must hold no career token
/// ([`crate::primitives::dialog::may_hold_career_token`]): its suit names
/// hold `$`. A career that does not parse makes it false here, though
/// [`careers`] fails the link first.
fn career_texts_end(index: &Index) -> Result<bool> {
    if index.named("Careers.txt").next().is_some() {
        return Ok(false);
    }
    let Some(directory) = find(&index.root, "ExpansionShared") else {
        return Ok(false);
    };
    match resource_set(&directory, "work")? {
        Some(set) if set.len() == 1 && set[0].eq_ignore_ascii_case("work.iff") => {}
        _ => return Ok(false),
    }
    let mut works = index.named("work.iff");
    let (Some((_, file)), None) = (works.next(), works.next()) else {
        return Ok(false);
    };
    // A member whose name holds a path is in a subdirectory of its
    // archive's (0x5b9dbc), whose path is `file.path`.
    if file
        .member
        .as_ref()
        .is_some_and(|member| member.name.contains(['\\', '/']))
    {
        return Ok(false);
    }
    let parent = file.path.parent().map(|parent| parent.to_string_lossy());
    if !parent.is_some_and(|parent| parent.eq_ignore_ascii_case(&directory.to_string_lossy())) {
        return Ok(false);
    }
    let location = file.location(&index.root);
    let iff = Iff::parse(file.read()?).map_err(|e| sims_content::Error::format(&location, e))?;
    if !strings_read_as_ported(&iff, None) {
        return Ok(false);
    }
    let token = crate::primitives::dialog::may_hold_career_token;
    for chunk in &iff.chunks {
        let data = iff.data(chunk);
        match &chunk.tag {
            b"STR#" | b"\0CST" => {
                if data.contains(&b'$') {
                    return Ok(false);
                }
            }
            b"CARR" => {
                let Ok(career) = sims_formats::career::Career::parse(data) else {
                    return Ok(false);
                };
                if token(&career.name)
                    || career
                        .levels
                        .iter()
                        .any(|level| level.strings.iter().any(|text| token(text)))
                {
                    return Ok(false);
                }
            }
            _ => {}
        }
    }
    Ok(true)
}

/// The file the default button labels come from (0x435fee, 0x43600a):
/// its STR# 142.
pub const LABEL_FILE: &str = "GameData/UIText.iff";

/// Whether the default button labels a dialog may show hold no `$`
/// ([`Program::labels_hold_no_token`]), for the install at `root`. They
/// come from the `UIText` resource set of `GameData`, which the composite
/// opener makes as it makes `Work`'s ([`career_texts_end`]), so this is
/// false unless that directory holds exactly one entry or archive member
/// named `UIText` with any extension (a `.dir` that is not a directory
/// included, as there), the file [`LABEL_FILE`], and every
/// string table of ID 142 in it (STR#, and CST, which the loader falls
/// back to, 0x4702c7) holds no `$` byte, deleted chunks included.
pub fn labels_hold_no_token(root: &Path) -> Result<bool> {
    const LABELS: u16 = 142;
    let Some(directory) = find(root, "GameData") else {
        return Ok(false);
    };
    match resource_set(&directory, "UIText")? {
        Some(set) if set.len() == 1 => {}
        _ => return Ok(false),
    }
    let path = match find(root, LABEL_FILE) {
        Some(path) if path.is_file() => path,
        _ => return Ok(false),
    };
    let bytes = std::fs::read(&path).map_err(|source| sims_content::Error::Io {
        path: path.clone(),
        source,
    })?;
    let location = path.display().to_string();
    let iff = Iff::parse(bytes).map_err(|e| sims_content::Error::format(&location, e))?;
    if !strings_read_as_ported(&iff, Some(LABELS)) {
        return Ok(false);
    }
    Ok(!iff.chunks.iter().any(|chunk| {
        matches!(&chunk.tag, b"STR#" | b"\0CST")
            && chunk.id == LABELS
            && iff.data(chunk).contains(&b'$')
    }))
}

/// Whether the loader reads every string table (STR#) resource of `iff`
/// (or only that of ID `only`) within its bytes, so that a check of them
/// for a `$` byte holds: the resource map matches the chunks (0x41bfdd to
/// 0x41c032), and each table is of a layout the loader reads as ported
/// ([`unported_strings`], with the flags the map states), parses, and is
/// not one the loader reads again for English ([`english_fallback`]). A
/// table it reads otherwise may be read past its end, or have a string's
/// bytes read as entries.
fn strings_read_as_ported(iff: &Iff, only: Option<u16>) -> bool {
    iff.map_error().is_none()
        && iff
            .all(b"STR#")
            .filter(|chunk| only.is_none_or(|id| chunk.id == id))
            .all(|chunk| {
                let data = iff.data(chunk);
                unported_strings(iff.resource_flags(chunk), data).is_none()
                    && sims_formats::strings::StringTable::parse(data)
                        .is_ok_and(|table| !english_fallback(&table.entries))
            })
}

/// `relative` (`/`-separated) under `root`, each component matched
/// exactly or else ignoring ASCII case, as Windows does (the first match in
/// name order); None when one is absent.
fn find(root: &Path, relative: &str) -> Option<std::path::PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        let exact = path.join(component);
        if exact.symlink_metadata().is_ok() {
            path = exact;
            continue;
        }
        // Sorted below.
        #[allow(clippy::disallowed_methods)]
        let mut names: Vec<_> = std::fs::read_dir(&path)
            .ok()?
            .filter_map(|entry| entry.ok().map(|entry| entry.file_name()))
            .collect();
        names.sort();
        let name = names.into_iter().find(|name| {
            name.to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(component))
        })?;
        path.push(name);
    }
    Some(path)
}

/// The names of the entries of `directory`, and of the members the
/// archives (FAR: an entry not a directory whose name ends in `far`) in it
/// place there, whose name up to
/// its first `.` is `stem`, ignoring ASCII case: whatever a resource set
/// of that name there could be made of. None when an archive there cannot
/// be read or has members the index leaves out
/// ([`sims_content::far_members`]), which may be any.
fn resource_set(directory: &Path, stem: &str) -> Result<Option<Vec<String>>> {
    let named = |name: &str| {
        name.split('.')
            .next()
            .is_some_and(|first| first.eq_ignore_ascii_case(stem))
    };
    let io = |source| sims_content::Error::Io {
        path: directory.to_path_buf(),
        source,
    };
    // Sorted below.
    #[allow(clippy::disallowed_methods)]
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .map_err(io)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()
        .map_err(io)?;
    entries.sort();
    let mut set = Vec::new();
    for path in entries {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if named(&name) {
            set.push(name.clone());
        }
        // The original takes an entry that is not a directory (0x5ba7d4
        // to 0x5ba7d9) whose name ends in the letters `far`, with or
        // without a dot, as an archive (0x5ba86d to 0x5ba8a7), as the
        // index does, and tries to mount it (0x5ba8d6). A metadata failure
        // (a dangling link, say) or a non-regular entry (a named pipe,
        // whose open would block) fails closed without opening it; a
        // regular file that `far_members` cannot open, read, or parse
        // fails closed too.
        let archive = name.len() >= 3
            && name.is_char_boundary(name.len() - 3)
            && name[name.len() - 3..].eq_ignore_ascii_case("far");
        if archive {
            let Ok(meta) = std::fs::metadata(&path) else {
                return Ok(None);
            };
            if meta.is_dir() {
                continue;
            }
            if !meta.is_file() {
                return Ok(None);
            }
            let Ok((members, skipped)) = sims_content::far_members(&path) else {
                return Ok(None);
            };
            if !skipped.is_empty() {
                return Ok(None);
            }
            // A member whose name holds a path is placed in a
            // subdirectory (0x5b9dbc, `/` read as `\\`, 0x5b96c8), not here.
            for member in members {
                if !member.name.contains(['\\', '/']) && named(&member.name) {
                    set.push(member.name);
                }
            }
        }
    }
    Ok(Some(set))
}

/// An OBJD's words as the original holds them. Registration (0x43ba52)
/// normalizes the resource with 0x431759 before reading any field
/// (legacy/crates/sims-sim/src/object_catalog/definition.rs:9): versions
/// 136 and 137 become 138, and if such a record is shorter than 216 bytes
/// and odd, its last byte is cleared. Every version is then cut or
/// zero-padded to 216 bytes, so all 108 words read, whatever the payload's
/// length.
pub(crate) fn definition_words(data: &[u8]) -> sims_formats::Result<[i16; DEFINITION_WORDS]> {
    let version = data
        .first_chunk::<4>()
        .map(|&b| u32::from_le_bytes(b))
        .ok_or_else(|| sims_formats::Error::new(0, "OBJD version absent"))?;
    let mut bytes = data.to_vec();
    if matches!(version, 136 | 137) {
        bytes[..4].copy_from_slice(&138u32.to_le_bytes());
        if bytes.len() < DEFINITION_WORDS * 2
            && !bytes.len().is_multiple_of(2)
            && let Some(last) = bytes.last_mut()
        {
            *last = 0;
        }
    }
    bytes.resize(DEFINITION_WORDS * 2, 0);
    let mut words = [0; DEFINITION_WORDS];
    for (word, pair) in words.iter_mut().zip(bytes.as_chunks::<2>().0) {
        *word = i16::from_le_bytes(*pair);
    }
    Ok(words)
}

/// A library's string table. One the reader here refuses is read as the
/// original's string loader reads it (0x46feac), in its resource's byte
/// order (`flags`): a big-endian version -1 table, say. Tables the reader
/// takes are kept as read, so the linked strings do not change.
fn string_table(
    data: &[u8],
    flags: u16,
) -> sims_formats::Result<sims_formats::strings::StringTable> {
    use sims_formats::strings::StringTable;
    StringTable::parse(data).or_else(|_| StringTable::parse_chunk(data, flags))
}

/// Whether a raw `BMP_` 2002 resource decodes cleanly through the original's
/// portrait bitmap reader (`0x4dece6` → `0x4ded4a..0x4df13d`).
///
/// Unlike `sims_formats::bitmap::bmp`, the original decoder does not reject
/// uncompressed 32-bit BMPs: `0x4ded62..0x4dedc9` checks only `"BM"`, non-zero
/// dimensions, and an optional `<= 256`-entry palette (where `used == 0` computes
/// `1u32.wrapping_shl((planes * bpp) as u32)`, matching x86 `shl eax, cl` at
/// `0x4dedb7`), and `0x4def5b..0x4df122` decodes RLE8 (`compression == 1`),
/// 8-bit plain (`bpp == 8`), or 3-bytes-per-pixel rows padded by `width & 3`
/// (`0x4df06a..0x4df11c`, which accepts 24-bit and 32-bit uncompressed BMPs).
fn is_native_portrait_bmp(data: &[u8]) -> bool {
    const MAX_PIXELS: u64 = 4096 * 4096;
    if data.len() < 54 || &data[0..2] != b"BM" {
        return false;
    }
    let offset = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    let width = i32::from_le_bytes(data[18..22].try_into().unwrap());
    let raw_height = i32::from_le_bytes(data[22..26].try_into().unwrap());
    if width <= 0 || raw_height == 0 || raw_height == i32::MIN {
        return false;
    }
    let w = width as usize;
    let h = raw_height.unsigned_abs() as usize;
    if (w as u64).saturating_mul(h as u64) > MAX_PIXELS {
        return false;
    }
    let planes = u16::from_le_bytes(data[26..28].try_into().unwrap());
    let bpp = u16::from_le_bytes(data[28..30].try_into().unwrap());
    let compression = u32::from_le_bytes(data[30..34].try_into().unwrap());
    let used_raw = u32::from_le_bytes(data[46..50].try_into().unwrap());
    let used = if used_raw != 0 {
        used_raw
    } else {
        1u32.wrapping_shl(u32::from(planes.wrapping_mul(bpp)))
    };
    if used <= 256 {
        let Some(pal_end) = (used as usize)
            .checked_mul(4)
            .and_then(|n| 54usize.checked_add(n))
        else {
            return false;
        };
        if pal_end > data.len() {
            return false;
        }
    }
    let Some(body) = data.get(offset..) else {
        return false;
    };
    if compression == 1 {
        let (mut pos, mut x, mut y) = (0usize, 0usize, 0usize);
        while pos + 2 <= body.len() {
            let count = usize::from(body[pos]);
            let val = body[pos + 1];
            pos += 2;
            if count > 0 {
                if y >= h || x.checked_add(count).is_none_or(|nx| nx > w) {
                    return false;
                }
                x += count;
            } else {
                match val {
                    0 => {
                        if y >= h {
                            return false;
                        }
                        x = 0;
                        y += 1;
                    }
                    1 => return true,
                    2 => {
                        if pos + 2 > body.len() {
                            return false;
                        }
                        x = x.saturating_add(usize::from(body[pos]));
                        y = y.saturating_add(usize::from(body[pos + 1]));
                        pos += 2;
                        if y >= h || x > w {
                            return false;
                        }
                    }
                    n => {
                        let n = usize::from(n);
                        if y >= h || x.checked_add(n).is_none_or(|nx| nx > w) {
                            return false;
                        }
                        let advance = n + (n & 1);
                        if pos.checked_add(advance).is_none_or(|end| end > body.len()) {
                            return false;
                        }
                        pos += advance;
                        x += n;
                    }
                }
            }
        }
        false
    } else if bpp == 8 {
        // 0x4df050..0x4df063 advances `esi` past the final row's padding and
        // immediately exits without dereferencing those padding bytes.
        let Some(stride) = w.checked_add(3).map(|n| n & !3) else {
            return false;
        };
        stride
            .checked_mul(h - 1)
            .and_then(|n| n.checked_add(w))
            .is_some_and(|need| need <= body.len())
    } else {
        // 0x4df10e..0x4df122 advances `esi` past `width & 3` on the final row
        // and immediately exits without reading the unused trailing padding.
        let Some(pixel_bytes) = w.checked_mul(3) else {
            return false;
        };
        let Some(row_bytes) = pixel_bytes.checked_add(w & 3) else {
            return false;
        };
        row_bytes
            .checked_mul(h - 1)
            .and_then(|n| n.checked_add(pixel_bytes))
            .is_some_and(|need| need <= body.len())
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFINITION_WORDS, definition_words, loaded_order, string_table};

    #[test]
    fn a_later_run_of_the_language_goes_in_front() {
        // 0x46ffcc to 0x470069: each entry of the wanted language goes in
        // at its place in its run of one language, from the front.
        let entry = |language, text: &str| sims_formats::strings::TextEntry {
            language,
            text: text.to_owned(),
            comment: String::new(),
            raw_text: text.as_bytes().to_vec(),
            raw_comment: Vec::new(),
        };
        let entries = [
            entry(1, "a"),
            entry(1, "b"),
            entry(2, "x"),
            entry(1, "c"),
            entry(1, "d"),
            entry(3, "y"),
            entry(3, "z"),
            entry(1, "e"),
        ];
        assert_eq!(loaded_order(&entries), ["e", "c", "d", "a", "b"]);
        // One run keeps the file's order, and other languages are left out.
        assert_eq!(
            loaded_order(&[entry(2, "x"), entry(1, "a"), entry(1, "b")]),
            ["a", "b"]
        );
    }

    #[test]
    fn short_definitions_are_padded() {
        let mut data = 138u32.to_le_bytes().to_vec();
        data.extend_from_slice(&[0x34, 0x12, 0x78]);
        let words = definition_words(&data).unwrap();
        assert_eq!(words[..4], [138, 0, 0x1234, 0x78]);
        assert!(words[4..].iter().all(|&w| w == 0));
    }

    #[test]
    fn versions_136_and_137_are_upgraded() {
        for version in [136u32, 137] {
            let mut data = version.to_le_bytes().to_vec();
            data.extend_from_slice(&[0x34, 0x12, 0x78]);
            // The odd trailing byte is cleared.
            assert_eq!(definition_words(&data).unwrap()[..4], [138, 0, 0x1234, 0]);
        }
    }

    #[test]
    fn a_big_endian_table_the_reader_refuses_is_read_in_its_order() {
        // Version -1 with a big-endian count of 2, in a resource without
        // flag 0x10.
        let data = b"\xff\xff\x00\x02a\0b\0";
        assert!(sims_formats::strings::StringTable::parse(data).is_err());
        assert_eq!(string_table(data, 0).unwrap().entries.len(), 2);
        // A table the reader takes is kept as read.
        let pascal = b"\x00\x00\x01\x00\x02hi";
        assert_eq!(string_table(pascal, 0x10).unwrap().entries.len(), 1);
    }

    #[test]
    fn long_definitions_are_cut() {
        let mut data = 138u32.to_le_bytes().to_vec();
        data.resize(300, 0xff);
        let words = definition_words(&data).unwrap();
        assert_eq!(words.len(), DEFINITION_WORDS);
        assert_eq!(words[DEFINITION_WORDS - 1], -1);
    }

    #[test]
    fn a_definition_needs_a_version() {
        assert!(definition_words(&[1, 2, 3]).is_err());
    }
}
