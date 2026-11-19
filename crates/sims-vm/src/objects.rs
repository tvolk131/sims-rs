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
//! original taken when the read returns; `docs/decisions/` says what was
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
pub(crate) const PERSON_SLOTS: usize = 5;

/// The words of a person's data. Saves before ObjM version 0x47 hold the
/// first 80.
pub(crate) const PERSON_WORDS: usize = 256;

/// The person data words that hold interests, which decide whether the
/// constructor makes defaults (0x45962b).
pub(crate) const INTERESTS: std::ops::RangeInclusive<usize> = 46..=53;

/// A registry word with countdown -1 and no flags: what registration
/// gives a new object (0x4434cd).
const REGISTERED: i32 = 0xffff;

/// The person data words a new person's default interests and
/// personality go into, in the order they are made, with how many of each
/// category (low, middle, high) the words of a group may take (0x459a9c).
const TRAITS: [(&[usize], [u8; 3]); 2] = [
    (&[46, 47, 48, 49, 50, 51, 52, 53, 54, 55], [4, 3, 3]),
    (&[13, 14, 16, 20, 26], [1, 2, 2]),
];

/// A counting view of the C runtime's random stream.
pub(crate) struct Draws<'a> {
    pub stream: &'a mut CrtRng,
    pub count: u32,
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
    /// interest words, then five personality words ([`TRAITS`]). Each
    /// takes a category (low, middle or high) that still has room,
    /// drawing again while the drawn one is full, and then one more draw
    /// for its value: 0 to 3 for low, 4 to 6 for middle, 7 to 10 for high,
    /// times 100. Returns each word with its value, in order.
    ///
    /// Ported from the old prototype's `default_traits`
    /// (legacy/crates/sims-vm/src/person_factory.rs), which compared it
    /// with the original's code run alone.
    pub(crate) fn default_traits(&mut self) -> Vec<(usize, i16)> {
        let mut traits = Vec::new();
        for (fields, mut room) in TRAITS {
            for &field in fields {
                let category = loop {
                    let category = usize::from(self.next() % 3);
                    if room[category] > 0 {
                        room[category] -= 1;
                        break category;
                    }
                };
                let draw = self.next();
                let value = match category {
                    0 => draw % 4,
                    1 => draw % 3 + 4,
                    _ => draw % 4 + 7,
                };
                traits.push((field, (value * 100) as i16));
            }
        }
        traits
    }
}

/// The types whose saved attributes may outnumber what their constructor
/// allocates (0x43df98, 0x43dfa3, 0x43dfae and 0x43dfb9, read at
/// 0x43fe49): the house read lets each of these grow its attribute array to
/// the saved count. The last two only when the configuration's expansions
/// byte (`[0x6e48bc]+0x66`, the mask global 20 is set from) has Unleashed's
/// bit, 0x20; without it both slots hold the first type (0x43fe9d).
const GROWN: [u32; 4] = [0x3C08_965F, 0x7FD9_6B54, 0x4A70_DF92, 0x7BEA_0977];

/// Whether the house read lets type `guid` grow its attribute array.
fn grows(guid: u32, expansions: i16) -> bool {
    let types = if expansions & 0x20 != 0 { 4 } else { 2 };
    GROWN[..types].contains(&guid)
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
    /// attributes beyond what the constructor allocates, for a type the
    /// house read does not let grow (`GROWN`); a person type with
    /// no skeleton in `people`; a person with no neighborhood record,
    /// for which the original makes one; and a person whose saved
    /// animator the restore does not port (`crate::restore`), among them
    /// every person whose type's scripts did not change since the save,
    /// which load step 13 does not reset.
    ///
    /// Each person's animator is restored as far as the simulation reads
    /// it (`crate::restore`): what differs from
    /// [`sims_sim::animator::LOADED`] goes into [`World::animators`], and
    /// the pose its saved layers left in the skin into
    /// [`World::held_poses`].
    ///
    /// A person saved part way through a route keeps only the number of
    /// its route contexts; the load makes that many, each as its default
    /// constructor makes it
    /// ([`crate::routing::slot::loaded_context`], 0x460443), and the
    /// first run of the route's steps pops it (0x45ae71). A negative
    /// count fails.
    ///
    /// Not modeled: object flags (the constructor sets 0x200 and the read
    /// 0x80), the count
    /// of queued interactions the game keeps, and graphics for objects
    /// that have their own (definition byte 0xcc bit 0), which fail.
    ///
    /// **Unverified**: an object whose record has no scheduler word keeps
    /// the one registration gave it; and that the game has read no
    /// skeleton file before its first load but those of
    /// [`READ_BEFORE_THE_FIRST_LOAD`].
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
        let expansions = world.globals.get(20).copied().unwrap_or(0);
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
                .construct_saved(
                    instance,
                    people,
                    &world.neighbors,
                    &mut draws,
                    &mut files,
                    expansions,
                )
                .map_err(|trap| fail(id, trap))?;
            if objects.insert(id, object).is_some() {
                return Err(fail(id, Trap::Invalid("a saved ID is listed twice".into())));
            }
            order.push(id);
        }
        let next_parts = link_parts(&objects, &order).map_err(|(id, trap)| fail(id, trap))?;
        // Each person's animator, as its read restores it (0x5918ba) and
        // the person's reset in load step 13 leaves it.
        let mut animators = BTreeMap::new();
        let mut held_poses = BTreeMap::new();
        let mut base_layers = BTreeMap::new();
        let mut moved = BTreeMap::new();
        for &id in &order {
            let restored = self
                .restore_animator(
                    id,
                    &objects[&id],
                    world.ticks,
                    world.changed_types.contains(&objects[&id].guid),
                )
                .map_err(|trap| fail(id, trap))?;
            if let Some(a) = restored.animator {
                animators.insert(id, a);
            }
            if let Some(held) = restored.held {
                held_poses.insert(id, held);
            }
            if let Some(base) = restored.base {
                base_layers.insert(id, base);
            }
            moved.insert(id, restored.moved);
        }
        // Each person's route contexts: as many as the save counts, each
        // as the load makes it (0x460443). A negative count, which the
        // original's resize would take as erasing before the start, fails.
        let mut contexts = BTreeMap::new();
        for &id in &order {
            let Some(person) = objects[&id].person.as_ref() else {
                continue;
            };
            let frames = usize::try_from(person.routing_frames).map_err(|_| {
                fail(
                    id,
                    Trap::Unsupported(format!(
                        "the person's saved route context count is {} (0x460443)",
                        person.routing_frames
                    )),
                )
            })?;
            if frames != 0 {
                contexts.insert(id, vec![crate::routing::slot::loaded_context(); frames]);
            }
        }
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
        world.people = world.registered_people();
        world.next_parts = next_parts;
        world.animators.extend(animators);
        world.held_poses.extend(held_poses);
        world.base_layers.extend(base_layers);
        for (id, bones) in moved {
            Self::note_moved(world, id, bones);
        }
        for (id, contexts) in contexts {
            world.routing.routes.entry(id).or_default().contexts = contexts;
        }
        // 0x41f320: reading an interaction (a person's running one, its
        // last one and each queued one: 0x45d012, 0x45d01f, 0x45d08b)
        // raises the action serial to its number when that is larger,
        // compared signed. Nothing reads the serial between the reads, so
        // the largest number counts.
        let largest = world
            .objects
            .values()
            .filter_map(|o| o.person.as_ref())
            .flat_map(|p| [&p.active, &p.last].into_iter().chain(&p.queue))
            .map(|action| action.uid)
            .max();
        if let Some(uid) = largest
            && uid > world.action_serial
        {
            world.action_serial = uid;
        }
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
        expansions: i16,
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
        // The read (the object's function at vtable offset 0x7c, 0x4342ae).
        // From version 0x18 the saved attribute count comes first; when it
        // is larger than the array, the array is made that long, zeroed and
        // read whole if the house read asks for it (0x434345), and the
        // house read (0x4400a7) asks only for the types in `GROWN`.
        let attributes = if saved.attributes.len() > attributes {
            if !grows(saved.guid, expansions) {
                return Err(Trap::Unsupported(format!(
                    "{} saved attributes of type 0x{:08X}, more than the {attributes} the \
                     constructor allocates: the original reads them past the array's end \
                     (0x43437b)",
                    saved.attributes.len(),
                    saved.guid
                )));
            }
            saved.attributes.len()
        } else {
            attributes
        };
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
    // The house's people are of the types the catalog registered, never
    // of one made while the game ran.
    let skeleton = people.skeletons.get(&guid).ok_or_else(|| {
        Trap::Unsupported(format!(
            "no skeleton was supplied for the person type 0x{guid:08X}"
        ))
    })?;
    build_skeleton(skeleton, draws, files)?;
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

/// The skeleton files the game has read before it loads its first house,
/// so that a person of theirs made later builds only its own copy.
///
/// The Goth day (`goth-day-20261115`) makes a cat ("kat" skeleton) at
/// updates 6756, 10358, 17562 and 24719, and each time the C runtime's
/// stream moves by 1,920 draws: one skeleton's worth, its own copy
/// (0x598da3). Had the first cat read its file there, that update would
/// take two (0x598af7 builds the file's one skeleton, then the copy). The
/// stream at the entry of the first lot update is 7,212 draws on from its
/// initial state 1, which the load's own work accounts for (the adult and
/// child files, the three Goths' copies and default interests, and the two
/// indicators). So the file was read without drawing from the stream the
/// simulation draws from (the C runtime keeps one per thread, 0x63e2b3
/// +0x14). That it was read before the load, and not between the load and
/// the first cat, is **unverified**; so is where (the avatar registry's
/// scan at startup, 0x59f844, reads files through 0x59f087, whose
/// skeleton records go through 0x598af7), and so is the rest of the
/// list: the same folder of `ExpansionShared.far`
/// holds the "dog" and "effects1" skeletons, which no capture has made.
pub const READ_BEFORE_THE_FIRST_LOAD: &[&str] =
    &["ExpansionShared/ExpansionShared.far/Skeletons\\kat_skeleton.cmx.bcf"];

/// The files of [`READ_BEFORE_THE_FIRST_LOAD`] as `people` names them.
/// A skeleton's file is its location in the install, spelled as it is on
/// disk (`sims_content::Resource::location`), so a listed file is matched
/// without regard to case. A listed file no person type's skeleton comes
/// from is left out: no person made later would read it.
pub fn read_before_the_first_load(people: &People) -> BTreeSet<String> {
    people
        .skeletons
        .values()
        .map(|skeleton| &skeleton.file)
        .filter(|file| {
            READ_BEFORE_THE_FIRST_LOAD
                .iter()
                .any(|listed| listed.eq_ignore_ascii_case(file))
        })
        .cloned()
        .collect()
}

/// A person's skeleton, as far as building it draws random numbers (in
/// 0x588dcb): the skeleton's file is read if the game has not read it yet
/// (`files`), which builds every skeleton in the file (0x598af7), and then
/// the person's own copy is built (0x598da3). A person type with no
/// skeleton in `people` fails.
pub(crate) fn build_skeleton(
    skeleton: &Skeleton,
    draws: &mut Draws<'_>,
    files: &mut BTreeSet<String>,
) -> Result<(), Trap> {
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
