//! Load step 6e: reading a house's saved objects onto the cleared lot
//! (0x449c5e, 0x43fe0a).
//!
//! The original does not copy saved objects into place. It first
//! constructs every object the file lists, as the factory would, and
//! registers it under its saved ID. Then it reads each object's saved
//! fields into what the constructor made. So what the lot holds afterwards
//! is partly the constructor's doing:
//!
//! - An object has as many attributes as its definition asks for, and never
//!   fewer than 8 (0x43189d). The saved ones fill the front and the rest
//!   stay zero.
//! - It has one slot for its tile and one for each object its definition
//!   says it can hold (0x431aaf). A person has five more (0x45962b). A save
//!   with fewer slots does not shrink that (0x4344d3).
//! - A person's constructor builds its skeleton, attaches the person to
//!   its neighborhood record, and may give it default interests and
//!   personality. The skeleton and the defaults draw from the C runtime's
//!   random stream. The saved person data then replaces the defaults, so
//!   the draws are what is left of them.
//!
//! Ported from the old prototype's `import_saved_storage_inner`
//! (legacy/crates/sims-content/src/save_storage.rs), `Object::construct`
//! and `Object::construct_person` (legacy/crates/sims-vm/src/construction.rs),
//! `initialize_with_constants`
//! (legacy/crates/sims-content/src/character_loading.rs), `WiggleState::new`
//! (legacy/crates/sims-sim/src/pose/skeleton.rs), and `default_traits`,
//! `attach_person_neighbor` and `persistent_person_fields`
//! (legacy/crates/sims-vm/src/person_factory.rs and loading.rs).
//!
//! For the Goth house the result was compared with a capture of the
//! original taken when the read returns; `docs/decisions.md` says what was
//! equal.

use crate::fault::{Fault, Trap};
use crate::interpreter::Interpreter;
use sims_content::{People, Skeleton};
use sims_core::CrtRng;
use sims_formats::saved_objects::{Instance, Objects, Slot};
use sims_sim::neighbors::Neighbors;
use sims_sim::{Object, World};
use std::collections::{BTreeMap, BTreeSet};

/// What [`Interpreter::read_objects`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedRead {
    /// The IDs read, in the file's order.
    pub objects: Vec<u16>,
    /// How many numbers the people's constructors drew from the C
    /// runtime's random stream.
    pub draws: u32,
}

/// Definition words the constructors read.
mod word {
    pub const TYPE: usize = 9;
    pub const ATTRIBUTES: usize = 58;
    pub const DYNAMIC_SPRITES: usize = 65;
}

/// The slots a person has besides its tile slot and its definition's own:
/// a carried object, the head, and three more (0x45962b).
const PERSON_SLOTS: usize = 5;

/// The words of a person's data. Saves before ObjM version 0x47 hold the
/// first 80.
const PERSON_WORDS: usize = 256;

/// The person data words that hold interests, which decide whether the
/// constructor makes defaults (0x45962b).
const INTERESTS: std::ops::RangeInclusive<usize> = 46..=53;

/// A registry word with countdown -1 and no flags: what registration
/// gives a new object (0x4434cd).
const REGISTERED: i32 = 0xffff;

/// A counting view of the C runtime's random stream.
struct Draws<'a> {
    stream: &'a mut CrtRng,
    count: u32,
}

impl Draws<'_> {
    fn next(&mut self) -> u16 {
        self.count += 1;
        self.stream.next_u15()
    }

    /// Building one skeleton's bones (0x598a6e, 0x595d65). Each bone gets
    /// four noise records of two random words and a random gradient, which
    /// is 32 draws, since a word is two draws and a gradient two words. A
    /// bone whose wiggle scale is not 1 then has its four gradients made
    /// again at that scale: 16 more. The records are for animation, which
    /// is not modeled, so only the draws are kept.
    fn skeleton(&mut self, scales: &[i32]) {
        for &scale in scales {
            for _ in 0..if scale == 1 { 32 } else { 48 } {
                self.next();
            }
        }
    }

    /// A new person's default interests and personality (0x459a9c): ten
    /// interest words, then five personality words. Each takes a category
    /// (low, middle or high) that still has room, drawing again while the
    /// drawn one is full, and then one more draw for its value. The saved
    /// person data replaces the values, so they are not kept.
    fn default_traits(&mut self) {
        for (fields, mut room) in [(10, [4, 3, 3]), (5, [1, 2, 2])] {
            for _ in 0..fields {
                loop {
                    let category = usize::from(self.next() % 3);
                    if room[category] > 0 {
                        room[category] -= 1;
                        break;
                    }
                }
                self.next();
            }
        }
    }
}

impl Interpreter<'_> {
    /// Load step 6e: reads a house's saved objects onto the lot (0x449c5e).
    /// The lot must hold no objects: the original clears it first.
    ///
    /// Every object is constructed and registered under its saved ID, at
    /// the end of the registration list, in the file's order (0x43fe0a,
    /// 0x4434cd). Then it takes its saved fields: position, attributes,
    /// temps, data, thread, relationships, slots, sprite flags, and the
    /// scheduler word. See the module documentation for what the
    /// constructor leaves. An object whose type already has an instance
    /// list is appended to it. A tile of a multi-tile object is appended
    /// to its lead tile's list in [`World::next_parts`] (0x423223,
    /// 0x4228a9).
    ///
    /// `people` supplies what constructing a person needs: its skeleton
    /// and its neighborhood record. Constructing a person:
    ///
    /// 1. Reads the skeleton's file if the game has not read it yet
    ///    ([`World::skeleton_files`]), which builds every skeleton in the
    ///    file, and then builds the person's own copy.
    /// 2. Attaches the person to the neighborhood record of its type. The
    ///    record's interests are copied to the person if the record holds
    ///    person data.
    /// 3. If every interest word is then below 11, makes default interests
    ///    and personality.
    ///
    /// Steps 1 and 3 draw from [`World::crt_rng`]. Nothing else of the
    /// person's graphics is modeled.
    ///
    /// On an error the lot is unchanged. These fail: a lot that holds
    /// objects; an object or a multi-tile lead without a definition; saved
    /// attributes beyond what the constructor allocates; a person type with
    /// no skeleton in `people`; and a person with no neighborhood record,
    /// for which the original makes one.
    ///
    /// Not modeled: object flags (the constructor sets 0x200 and the read
    /// 0x80), each person's animator and its saved animations, the count
    /// of queued interactions the game keeps, and graphics for objects
    /// that have their own (definition byte 0xcc bit 0), which fail.
    ///
    /// **Unverified**: an object whose record has no scheduler word keeps
    /// the one registration gave it; and that the game has read no
    /// skeleton file before its first load.
    pub fn read_objects(
        &self,
        world: &mut World,
        saved: &Objects,
        people: &People,
    ) -> Result<SavedRead, Fault> {
        let fail = |object, trap| Fault {
            object,
            at: None,
            trap,
        };
        if let Some((&id, _)) = world.objects.first_key_value() {
            return Err(fail(
                id,
                Trap::Unsupported("reading saved objects onto a lot that holds objects".into()),
            ));
        }
        let mut crt = world.crt_rng;
        let mut draws = Draws {
            stream: &mut crt,
            count: 0,
        };
        let mut files = world.skeleton_files.clone();
        let mut objects = BTreeMap::new();
        let mut order = Vec::new();
        // First every object is constructed, then every object is read.
        // Only construction draws, so one pass in the file's order gives
        // the same stream.
        for &id in &saved.order {
            let Some(instance) = saved.instances.get(&id) else {
                continue;
            };
            let object = self
                .construct_saved(instance, people, &world.neighbors, &mut draws, &mut files)
                .map_err(|trap| fail(id, trap))?;
            if objects.insert(id, object).is_some() {
                return Err(fail(id, Trap::Invalid("a saved ID is listed twice".into())));
            }
            order.push(id);
        }
        let next_parts = link_parts(&objects, &order).map_err(|(id, trap)| fail(id, trap))?;
        let count = draws.count;
        world.crt_rng = crt;
        world.skeleton_files = files;
        for &id in &order {
            if let Some(instances) = world.instances.get_mut(&objects[&id].guid) {
                instances.push(id);
            }
        }
        world.objects = objects;
        world.order = order.clone();
        world.next_parts = next_parts;
        Ok(SavedRead {
            objects: order,
            draws: count,
        })
    }

    /// Constructs one saved object and reads its saved fields into it.
    fn construct_saved(
        &self,
        saved: &Instance,
        people: &People,
        neighbors: &Neighbors,
        draws: &mut Draws<'_>,
        files: &mut BTreeSet<String>,
    ) -> Result<Object, Trap> {
        let definition = self
            .catalog
            .object(saved.guid)
            .ok_or_else(|| Trap::Invalid(format!("no definition for GUID 0x{:08X}", saved.guid)))?;
        let words = self
            .program
            .definition(definition.resource, definition.chunk)
            .ok_or_else(|| {
                Trap::Invalid(format!(
                    "the program does not hold the definition of GUID 0x{:08X}",
                    saved.guid
                ))
            })?;
        let person = words[word::TYPE] == 2;
        if person != saved.person.is_some() {
            return Err(Trap::Invalid(
                "the save and the definition disagree on whether this is a person".into(),
            ));
        }
        if words[102] & 1 != 0 && !person {
            return Err(Trap::Unsupported(
                "an object with graphics of its own (definition byte 0xcc bit 0); its \
                 constructor is not ported"
                    .into(),
            ));
        }
        // The base constructor (0x43189d).
        let attributes = usize::try_from(words[word::ATTRIBUTES]).unwrap_or(0).max(8);
        let sprites = usize::try_from(words[word::DYNAMIC_SPRITES]).map_err(|_| {
            Trap::Invalid(format!(
                "negative dynamic sprite count {}",
                words[word::DYNAMIC_SPRITES]
            ))
        })?;
        let containers = self
            .program
            .containers(definition.resource, definition.chunk);
        let slots = 1 + containers + if person { PERSON_SLOTS } else { 0 };
        if person {
            construct_person(saved.guid, people, neighbors, draws, files)?;
        }
        // The read (the object's function at vtable offset 0x7c).
        if saved.attributes.len() > attributes {
            return Err(Trap::Unsupported(format!(
                "{} saved attributes, more than the {attributes} the constructor allocates",
                saved.attributes.len()
            )));
        }
        let mut object = Object {
            id: saved.id,
            guid: saved.guid,
            footprint: saved.footprint,
            position: saved.position,
            unknown_short: saved.unknown_short,
            attributes: saved.attributes.clone(),
            temps: saved.temps.clone(),
            data: saved.data.clone(),
            obsolete_word: saved.obsolete_word,
            stack_capacity_bytes: saved.stack_capacity_bytes,
            stack: saved.stack.clone(),
            relationship_flag: saved.relationship_flag,
            relationships: saved.relationships.clone(),
            slots: saved.slots.clone(),
            dynamic_sprites: saved.dynamic_sprites.clone(),
            multipart: saved.multipart.clone(),
            portal: saved.portal.clone(),
            person: saved.person.clone(),
            registry_word: Some(saved.registry_word.unwrap_or(REGISTERED)),
            last_result: false,
            removal_skips_resets: false,
            selected: false,
        };
        object.attributes.resize(attributes, 0);
        if object.slots.len() < slots {
            object.slots.resize(
                slots,
                Slot {
                    unknown: 0,
                    object: 0,
                },
            );
        }
        if object.dynamic_sprites.len() < sprites {
            object.dynamic_sprites.resize(sprites, 0);
        }
        if let Some(person) = &mut object.person
            && person.data.len() < PERSON_WORDS
        {
            person.data.resize(PERSON_WORDS, 0);
        }
        Ok(object)
    }
}

/// A person's constructor, as far as it draws random numbers (0x45962b).
fn construct_person(
    guid: u32,
    people: &People,
    neighbors: &Neighbors,
    draws: &mut Draws<'_>,
    files: &mut BTreeSet<String>,
) -> Result<(), Trap> {
    let skeleton: &Skeleton = people.skeletons.get(&guid).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    })?;
    let own = skeleton.scales.get(skeleton.place).ok_or_else(|| {
        Trap::Invalid(format!(
            "the skeleton {:?} is not in its file",
            skeleton.name
        ))
    })?;
    // Reading a skeleton file builds every skeleton in it (0x598af7).
    if files.insert(skeleton.file.clone()) {
        for scales in &skeleton.scales {
            draws.skeleton(scales);
        }
    }
    // The person's own copy (0x598da3).
    draws.skeleton(own);
    // The first record of the person's type, in slot order.
    let record = neighbors.first_of(guid).ok_or_else(|| {
        Trap::Unsupported(format!(
            "the person type 0x{guid:08X} has no neighborhood record; making one during a \
             load is not ported"
        ))
    })?;
    // A record with person data gives the new person its interests (the
    // table at 0x423827 lists them from revision 1). Without, they stay 0.
    let mut defaults = true;
    if record.revision >= 1 {
        for field in INTERESTS {
            let value = record.data.get(field).ok_or_else(|| {
                Trap::Invalid(format!(
                    "neighborhood record {} has no person data word {field}",
                    record.id
                ))
            })?;
            defaults &= *value < 11;
        }
    }
    if defaults {
        draws.default_traits();
    }
    Ok(())
}

/// Appends each tile of a multi-tile object to its lead tile's list, in
/// the file's order (0x423223, 0x4228a9). A tile names its lead by ID; the
/// lead names none.
fn link_parts(
    objects: &BTreeMap<u16, Object>,
    order: &[u16],
) -> Result<BTreeMap<u16, u16>, (u16, Trap)> {
    let mut next = BTreeMap::new();
    for &id in order {
        let Some(part) = &objects[&id].multipart else {
            continue;
        };
        if part.parent == 0 {
            continue;
        }
        let lead = part.parent as u16;
        if objects.get(&lead).is_none_or(|o| o.multipart.is_none()) {
            return Err((
                id,
                Trap::Invalid(format!("the lead tile {lead} is not a multi-tile object")),
            ));
        }
        let mut last = lead;
        let mut steps = 0;
        while let Some(&following) = next.get(&last) {
            last = following;
            steps += 1;
            if last == id || steps > order.len() {
                return Err((id, Trap::Invalid("the tiles of a group form a loop".into())));
            }
        }
        if last == id {
            return Err((id, Trap::Invalid("a tile names itself as its lead".into())));
        }
        next.insert(last, id);
    }
    Ok(next)
}
