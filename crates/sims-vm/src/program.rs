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
    /// The IDs of the file's legacy catalog string tables (CATS), which
    /// are not read: the catalog's name lookup (0x4061e3) checks for one
    /// before the CTSS table (**unverified**, from a reading of the
    /// disassembly not checked here).
    pub legacy_catalog_strings: BTreeSet<u16>,
    /// The sound event each FWAV chunk names, by chunk ID: what Play
    /// Sound Event plays (0x582c57). The name is the chunk's bytes up to
    /// the first zero, read as Latin-1.
    pub sounds: BTreeMap<u16, String>,
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
            zoning: BTreeMap::new(),
            animations: Animations::default(),
            people: People::default(),
            icon_frames: None,
            wardrobe: None,
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
                    let table = sims_formats::strings::StringTable::parse(data)
                        .map_err(|e| format("STR#", chunk.id, e))?;
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

#[cfg(test)]
mod tests {
    use super::{DEFINITION_WORDS, definition_words};

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
