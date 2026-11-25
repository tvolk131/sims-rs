//! State written by the five neighborhood pool fills. See
//! docs/decisions/20261120-neighborhood-open-replays-five-pools.md.

use crate::boot::BootError;
use crate::objects::{Draws, PERSON_WORDS};
use crate::people::{SAVED_REVISION, field, reload_profile};
use sims_content::family_import::reserve_character_file;
use sims_content::{Neighborhood, Profile, Skeleton};
use sims_core::CrtRng;
use sims_formats::behavior::ObjectDefinition;
use sims_formats::iff::Iff;
use sims_formats::neighborhood::{Family, Neighbor};
use sims_formats::strings::StringTable;
use sims_sim::characters;
use sims_sim::neighbors::FAMILY;
use sims_sim::patch_import::{self, Context, allocate_record, rebuild_family_words};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Captured outcomes of the pool RNG (0x5bc82b, 0x5bc882), clock reads
/// (0x4725ff), and editor scripts. Only families that gain people need an
/// entry, listing exactly the new people in creation order. In a missing
/// tourist family the people also encode the composition rolls at
/// 0x42b4fe and 0x42b53c: adults first (man before woman), then children.
#[derive(Debug, Default)]
pub struct PoolInput {
    pub people: BTreeMap<u16, Vec<PoolPerson>>,
    /// The later lot or toolbar dispatch. A view open alone stops after
    /// Magic Town (0x5580e9, 0x55fb4c).
    pub transition: Option<LotTransition>,
    /// The `.ndx` skeletons, by name, that earlier work loaded before the
    /// pools run, in the order it loaded them (the first catalog to
    /// register a name keeps it, 0x59f18c). A temporary actor's skeleton that neither the startup
    /// scan nor that work loaded is loaded on first use (0x5a03cd), with
    /// its draws. `adult` and `child` come from `Animation.ndx`; both UD2
    /// captures fit them loaded already. That is fitted, not traced.
    pub loaded_skeletons: Vec<String>,
}

#[derive(Debug)]
pub struct PoolPerson {
    /// The captured person's file, carried through to the output. Its
    /// OBJD supplies the GUID high word and the recorded template
    /// choice. Its body strings supply age, sex and tone. The original's
    /// names, appearance and other resource edits are kept in this file.
    pub character: Iff,
    /// Editor outcomes in the saved record (0x425a69). Celebrities use
    /// words 10–12, 15, 17–18 and 80–82. Tourist children use words 9–12,
    /// 15, 17–18 and 56–57. Other words are constructed here.
    pub editor_words: BTreeMap<usize, i16>,
}

/// Observed effects between the neighborhood update and the dispatch
/// that calls the Townie and tourist fills.
#[derive(Debug, Default)]
pub struct LotTransition {
    /// Rebuild person word 61 and reset the file cursor, as the end of
    /// reload does (0x424e89, 0x424e8e). Both UD2 captures show the rebuild
    /// in their earlier pools' records. Its exact call site is untraced.
    pub reload: bool,
    /// Additional CRT draws before the Townie worker. Both UD2 captures
    /// have 64, whose caller is untraced.
    pub crt_draws: u32,
}

#[derive(Debug)]
pub struct OpenedPools {
    pub update_draws: u32,
    pub townie_draws: u32,
    pub tourist_draws: u32,
    /// Neighborhood +0x160 after the last attempt.
    pub next_file: i32,
    /// The `.ndx` skeletons the pools loaded on first use (0x5a03cd), each
    /// with its catalog, as the World keeps them
    /// (`World::loaded_indexed_skeletons`), and the skeleton files that
    /// read (`World::skeleton_files`). Both are empty for the UD2
    /// captures, whose `adult` and `child` were loaded before the pools.
    pub loaded_skeletons: BTreeMap<String, Option<String>>,
    pub skeleton_files: BTreeSet<String>,
    /// Each family the Townie and tourist wrappers moved off a reserved
    /// ID (0x42b05e to 0x42b09f), as its old and new ID, in order. The
    /// family keeps its file and record in memory, so its save inputs go
    /// with it ([`sims_sim::neighborhood_save::SaveInputs::renumber_family`]).
    pub renumbered: Vec<(u16, u16)>,
}

const CELEBRITY_WORDS: &[usize] = &[10, 11, 12, 15, 17, 18, 80, 81, 82];
const TOURIST_CHILD_WORDS: &[usize] = &[9, 10, 11, 12, 15, 17, 18, 56, 57];

struct Pool {
    families: Range<u16>,
    /// The tourist worker skips existing families regardless of size.
    target: Option<usize>,
    expansion: u8,
    /// The pending/done pair's done byte in NGBH #1 (pending follows it).
    done: usize,
    /// Whether the one-time wrapper tests done (the first three) rather
    /// than pending (the Townies and tourists).
    tests_done: bool,
    template_kind: u16,
    actors: usize,
    worker: &'static str,
    collision: &'static str,
}

const CELEBRITIES: Pool = Pool {
    families: 5000..5001,
    target: Some(12),
    expansion: 0x40,
    done: 4,
    tests_done: true,
    template_kind: 0x800,
    actors: 2,
    worker: "0x42c1ea",
    collision: "0x42c5e9",
};
const FANS: Pool = Pool {
    families: 5500..5501,
    target: Some(18),
    expansion: 0x40,
    done: 6,
    tests_done: true,
    template_kind: 0x200,
    actors: 2,
    worker: "0x42d193",
    collision: "0x42d592",
};
const MAGIC_TOWN: Pool = Pool {
    families: 6000..6001,
    target: Some(30),
    expansion: 0x80,
    done: 8,
    tests_done: true,
    template_kind: 0x1000,
    actors: 2,
    worker: "0x42d5f2",
    collision: "0x42d9e2",
};
const TOWNIES: Pool = Pool {
    families: 2000..2001,
    target: Some(30),
    expansion: 0x04,
    done: 0,
    tests_done: false,
    template_kind: 0x200,
    actors: 1,
    worker: "0x42a98f",
    collision: "0x42b09f",
};
const TOURISTS: Pool = Pool {
    families: 3000..3010,
    target: None,
    expansion: 0x08,
    done: 2,
    tests_done: false,
    template_kind: 0x200,
    actors: 2,
    worker: "0x42b0fd",
    collision: "0x42b975",
};

/// The skeletons the pool people's temporary actors copy, or why a
/// lookup failed (reported only when a person needs it).
#[derive(Debug, Default)]
pub(super) struct Skeletons {
    /// The first actor's: each template's skeleton name, by template GUID.
    pub templates: BTreeMap<u32, Result<String, String>>,
    /// The skeletons the startup scan loaded (0x59f844 → 0x59f132), each
    /// with its own bone scales.
    pub eager: BTreeMap<String, Vec<i32>>,
    /// The skeletons an `.ndx` names, each with its catalog or why it
    /// cannot load (0x584562 → 0x59fc48), and each catalog's skeletons.
    pub indexed: BTreeMap<String, Result<String, String>>,
    pub catalogs: BTreeMap<String, (Vec<Skeleton>, Option<String>)>,
}

pub(super) struct FilledPools<'a> {
    pub report: OpenedPools,
    pub files: Vec<(i32, &'a Iff)>,
}

pub(super) fn fill<'a>(
    neighborhood: &mut Neighborhood,
    context: &Context<'_>,
    imported: &patch_import::Opened,
    input: &'a PoolInput,
    skeletons: &Skeletons,
    crt: &mut CrtRng,
) -> Result<FilledPools<'a>, BootError> {
    let mut files = context.character_files.clone();
    for (_, import) in &imported.imports {
        files.extend(import.made.values().map(|made| made.file));
    }
    let mut fills = Fills {
        neighborhood,
        context,
        people: input
            .people
            .iter()
            .map(|(&id, people)| (id, people.as_slice()))
            .collect(),
        draws: Draws {
            stream: crt,
            count: 0,
        },
        next_file: imported.next_file,
        skeletons,
        lazy: LazySkeletons::default(),
        files,
        made: Vec::new(),
        renumbered: Vec::new(),
    };
    // What earlier work loaded registers the same way, without draws
    // here: they were made before the pool boundary `crt` starts at.
    // A fill with no people looks nothing up.
    if !input.people.is_empty() {
        fills.lazy = LazySkeletons::loaded_before(skeletons, &input.loaded_skeletons)?;
    }
    // The setup update (0x5580a2–0x5580e9, also 0x55fb05–0x55fb4c).
    for pool in [CELEBRITIES, FANS, MAGIC_TOWN] {
        fills.run(pool)?;
    }
    let update_draws = fills.draws.count;
    let (mut townie_draws, mut tourist_draws) = (0, 0);
    if let Some(transition) = &input.transition {
        if transition.reload {
            rebuild_family_words(fills.neighborhood)?;
            fills.next_file = 0;
        }
        for _ in 0..transition.crt_draws {
            fills.draws.next();
        }
        // The lot and toolbar callbacks, e.g. 0x55855d–0x558588.
        let before = fills.draws.count;
        fills.run(TOWNIES)?;
        townie_draws = fills.draws.count - before;
        let before = fills.draws.count;
        fills.run(TOURISTS)?;
        tourist_draws = fills.draws.count - before;
    }
    if let Some(id) = fills.people.keys().next() {
        return Err(format!("0x5580a2 / 0x55855d: unused recorded people for family {id}").into());
    }
    let report = OpenedPools {
        update_draws,
        townie_draws,
        tourist_draws,
        next_file: fills.next_file,
        loaded_skeletons: fills.lazy.loaded,
        skeleton_files: fills.lazy.files,
        renumbered: fills.renumbered,
    };
    Ok(FilledPools {
        report,
        files: fills.made,
    })
}

struct Fills<'a, 'b> {
    neighborhood: &'b mut Neighborhood,
    context: &'b Context<'b>,
    people: BTreeMap<u16, &'a [PoolPerson]>,
    draws: Draws<'b>,
    next_file: i32,
    skeletons: &'b Skeletons,
    lazy: LazySkeletons,
    files: BTreeSet<i32>,
    made: Vec<(i32, &'a Iff)>,
    renumbered: Vec<(u16, u16)>,
}

impl<'a, 'b> Fills<'a, 'b> {
    fn run(&mut self, pool: Pool) -> Result<(), BootError> {
        if self.context.expansions & pool.expansion == 0 {
            return Ok(());
        }
        let options = self.neighborhood.options.ok_or_else(|| {
            format!(
                "{}: no NGBH #1; option defaults from 0x4244bc are unported",
                pool.worker
            )
        })?;
        let pending = pool.done + 1;
        // The first three wrappers test done; the last two test pending
        // (0x42c548, 0x42d4f1, 0x42d941, 0x42b007, 0x42b8d7).
        let converting = if pool.tests_done {
            !options[pool.done]
        } else {
            options[pending]
        };
        if converting {
            self.free_reserved_ids(&pool)?;
        }
        // The callback always calls the worker, too. If the wrapper just
        // filled it, the second call exits at the member/family count.
        for id in pool.families.clone() {
            self.fill_family(&pool, id)?;
        }
        if converting {
            let options = self.neighborhood.options.as_mut().expect("NGBH #1 checked");
            options[pending] = false;
            options[pool.done] = true;
        }
        Ok(())
    }

    fn free_reserved_ids(&mut self, pool: &Pool) -> Result<(), BootError> {
        free_reserved_ids(&mut self.neighborhood.families, pool, &mut self.renumbered)
    }

    fn fill_family(&mut self, pool: &Pool, id: u16) -> Result<(), BootError> {
        let family = self.neighborhood.families.get(&id);
        let count = pool
            .target
            .map(|target| target.saturating_sub(family.map_or(0, |family| family.members.len())));
        if count == Some(0) || (count.is_none() && family.is_some()) {
            return Ok(());
        }
        let people = self
            .people
            .remove(&id)
            .ok_or_else(|| format!("{}: missing recorded people for family {id}", pool.worker))?;
        match count {
            Some(count) if people.len() == count => {}
            None if (2..=4).contains(&people.len()) => {}
            _ => return Err(format!(
                "{}: {} recorded people cannot fill family {id} (expected {count:?}, or 2–4 for tourists)",
                pool.worker, people.len()
            ).into()),
        }
        let existing = family.map_or(0, |family| family.members.len());
        // The Townie worker's specialty templates: every registered
        // definition of kind 0x400, in the registry's order, duplicates
        // included (0x42a9e8 to 0x42aa2f). A new family takes them first
        // (0x42ac74 to 0x42acc1).
        let mut townies = (pool.done == TOWNIES.done).then(|| Specialties {
            remaining: self
                .context
                .registry
                .iter()
                .filter(|entry| entry.kind == 0x400)
                .map(|entry| entry.guid)
                .collect(),
            new_family: family.is_none(),
        });
        let mut members = Vec::with_capacity(people.len());
        let mut adults = Vec::new();
        let mut children = 0;
        for (index, person) in people.iter().enumerate() {
            let ordinal = existing + index;
            let (guid, age, gender) = self.make_person(pool, person, ordinal, townies.as_mut())?;
            match age {
                27 if children == 0 => adults.push(gender),
                9 if count.is_none() => children += 1,
                _ => {
                    return Err(
                        format!("{}: unexpected age or order in family {id}", pool.worker).into(),
                    );
                }
            }
            members.push(guid);
        }
        if count.is_none()
            && (!matches!(adults.as_slice(), [0] | [1] | [0, 1]) || !(1..=2).contains(&children))
        {
            return Err(format!("0x42b4fe / 0x42b53c: invalid composition in family {id}").into());
        }
        // 0x41003c, 0x410696, 0x410849: saved fields of a new family and
        // appended GUIDs. Existing families keep their fields.
        self.neighborhood
            .families
            .entry(id)
            .or_insert_with(|| Family {
                version: 9,
                house: 0,
                number: -1,
                budget: 0,
                architecture_value: 0,
                friends: 0,
                flags: 0,
                members: Vec::new(),
                tail: vec![0; 16],
            })
            .members
            .extend(members);
        Ok(())
    }

    /// Makes the pool person `person`, the family's member number
    /// `ordinal` (counting from 0). `townies` is the Townie worker's
    /// specialty templates not yet used.
    fn make_person(
        &mut self,
        pool: &Pool,
        person: &'a PoolPerson,
        ordinal: usize,
        townies: Option<&mut Specialties>,
    ) -> Result<(u32, i16, i16), BootError> {
        let iff = &person.character;
        let mut objects = iff.all(b"OBJD");
        let chunk = objects
            .next()
            .ok_or("0x47249d: captured person has no OBJD")?;
        if objects.next().is_some() {
            return Err("0x47249d: multiple OBJDs in a pool character are unported".into());
        }
        let objd = iff.data(chunk);
        let definition = ObjectDefinition::parse(objd)?;
        if definition.object_type != 2 || definition.master_id != 0 {
            return Err(
                "0x42cf09: captured pool character is not a single person definition".into(),
            );
        }
        let offset = characters::ORIGINAL_GUID_WORDS * 2;
        let source = objd
            .get(offset..offset + 4)
            .ok_or("0x4725e4: captured OBJD has no source GUID")?;
        let source = u32::from_le_bytes(source.try_into()?);
        let generic = self
            .context
            .registry
            .iter()
            .find(|entry| entry.kind == pool.template_kind);
        let matches_generic = generic.is_some_and(|entry| entry.guid == source && entry.resolves);
        match townies {
            Some(townies) => townies.take(
                source,
                ordinal,
                matches_generic,
                self.context
                    .registry
                    .iter()
                    .any(|entry| entry.guid == source && entry.resolves),
            )?,
            None if matches_generic => {}
            None => {
                return Err(format!(
                    "{}: unexpected captured template {source:#010x}",
                    pool.worker
                )
                .into());
            }
        }

        let file = reserve_character_file(&mut self.next_file, &mut self.files)
            .map_err(|e| format!("{}: character file allocation (0x4722b8): {e}", pool.worker))?;
        // Clock supplies the high word, CRT rand() supplies the low word.
        let guid = (definition.guid & 0xffff_0000) | u32::from(self.draws.next());
        if guid != definition.guid {
            return Err(format!(
                "0x4725ff: User{file:05} CRT GUID {guid:#010x}, captured {:#010x}",
                definition.guid
            )
            .into());
        }
        if guid == 0
            || self.context.registry.iter().any(|entry| entry.guid == guid)
            || self
                .neighborhood
                .neighbors
                .iter()
                .flatten()
                .any(|record| record.guid == guid)
        {
            return Err(format!(
                "0x43c33d: registering a colliding or zero GUID {guid:#010x} is unported"
            )
            .into());
        }

        let mut data = vec![0; PERSON_WORDS];
        let skeleton = read_profile(
            iff,
            objd,
            self.context.language,
            pool.actors == 2,
            &mut data,
        )?;
        let skeletons: &'b Skeletons = self.skeletons;
        let template = match skeletons.templates.get(&source) {
            Some(Ok(name)) => name.as_str(),
            Some(Err(reason)) => {
                return Err(format!(
                    "0x5a0384: template {source:#010x}'s skeleton for User{file:05}: {reason}"
                )
                .into());
            }
            None => {
                return Err(format!(
                    "0x5a0384: no skeleton looked up for template {source:#010x} of User{file:05}"
                )
                .into());
            }
        };
        // The first actor is made from the template (0x426e8e), a second
        // after the age and skeleton are set (0x426e97 via 0x40cc2a). A
        // Townie makes only the first (0x426646, 0x4269c5).
        let mut copies = Vec::with_capacity(pool.actors);
        for name in [template, skeleton].into_iter().take(pool.actors) {
            // A name the registry lacks takes the miss branch (0x5a03c0);
            // one it holds but has not loaded is loaded now (0x5a03cd).
            let (load, scales) = self
                .lazy
                .load(self.skeletons, name)
                .map_err(|error| format!("{error} (User{file:05})"))?;
            copies.push((load, scales));
        }
        let (age, gender) = (data[field::AGE], data[field::GENDER]);
        if pool.done == CELEBRITIES.done {
            check_celebrity_sex(ordinal, gender)
                .map_err(|error| format!("{error} (User{file:05})"))?;
        }
        let editor_words = match pool.done {
            done if done == CELEBRITIES.done => CELEBRITY_WORDS,
            done if done == TOURISTS.done && age == 9 => TOURIST_CHILD_WORDS,
            _ => &[],
        };
        if person.editor_words.len() != editor_words.len() {
            return Err(format!("0x425a69: wrong number of editor words on User{file:05}").into());
        }
        for &word in editor_words {
            data[word] = *person
                .editor_words
                .get(&word)
                .ok_or_else(|| format!("0x425a69: missing editor word {word} on User{file:05}"))?;
        }

        // Each temporary actor copies its skeleton and draws two passes.
        // The skeleton's file is taken as already read (0x598af7), so
        // only the actor's own copy draws (0x598da3). The final pass
        // survives 0x4ea538 → 0x425a69.
        for (load, scales) in copies {
            // The lookup's load, if any, comes before the actor's copy.
            for scales in load {
                self.draws.skeleton(&scales);
            }
            self.draws.skeleton(&scales);
            self.draws.default_traits();
            for (word, value) in self.draws.default_traits() {
                data[word] = value;
            }
        }
        data[FAMILY] = 0;
        allocate_record(
            &mut self.neighborhood.neighbors,
            Neighbor {
                version: 10,
                version_extra: Some(9),
                name: format!("user{file:05}"),
                unknown_zero: 0,
                person_mode: SAVED_REVISION,
                person_data: data,
                id: 0,
                guid,
                unknown_negative: -1,
                relationships: Vec::new(),
            },
        )?;
        self.made.push((file, iff));
        Ok((guid, age, gender))
    }
}

/// The `.ndx` skeletons loaded after the startup scan.
#[derive(Debug, Default)]
struct LazySkeletons {
    /// Those the pools loaded on first use, as [`OpenedPools`] reports:
    /// each with its catalog, or `None` for a load that failed.
    loaded: BTreeMap<String, Option<String>>,
    /// The same, for the loads of [`PoolInput::loaded_skeletons`].
    before: BTreeMap<String, Option<String>>,
    /// The skeleton files the pools' loads read.
    files: BTreeSet<String>,
}

impl LazySkeletons {
    /// Registers `names`, loaded by earlier work, as their loads would,
    /// leaving their draws out: they were made before the pool boundary.
    fn loaded_before(skeletons: &Skeletons, names: &[String]) -> Result<Self, BootError> {
        let mut lazy = Self::default();
        for name in names {
            lazy.load(skeletons, name)
                .map_err(|error| format!("{error} (loaded before the pools)"))?;
        }
        Ok(Self {
            before: lazy.loaded,
            ..Self::default()
        })
    }

    /// Looks up the skeleton `name` (0x5a0384) and, if the startup scan
    /// did not load it and no earlier lookup has, loads it now (0x5a03cd
    /// → 0x5a3810). Returns the bone scales whose builds the load draws,
    /// in order, to come before the actor's copy, and the scales of the
    /// skeleton the registry now holds under `name`, which the copy uses.
    ///
    /// The load reads the skeleton's `.ndx` catalog (0x59e60e → 0x59f087),
    /// which builds every skeleton in it, each drawing (0x59f107 →
    /// 0x598af7), and registers those not already registered (0x59f18c);
    /// this is `people::load_indexed_bcf_catalog_into`, the World's lazy
    /// load. A lookup that fails marks the skeleton (0x5a389b).
    fn load(
        &mut self,
        skeletons: &Skeletons,
        name: &str,
    ) -> Result<(Vec<Vec<i32>>, Vec<i32>), String> {
        if let Some(scales) = skeletons.eager.get(name) {
            return Ok((Vec::new(), scales.clone()));
        }
        let builds = match self.before.get(name).or(self.loaded.get(name)) {
            Some(Some(_)) => Vec::new(),
            Some(None) => {
                return Err(format!(
                    "0x5a389b: an earlier load of the skeleton {name:?} failed"
                ));
            }
            None => self.read_catalog(skeletons, name)?,
        };
        let Some(Some(location)) = self.before.get(name).or(self.loaded.get(name)) else {
            return Err(format!("0x5a03cd: the load of {name:?} registered nothing"));
        };
        // The registry's entry points into the catalog that registered it
        // (as `people::resolve_skeleton_by_name_into` finds it).
        skeletons
            .catalogs
            .get(location)
            .and_then(|(items, _)| items.iter().find(|item| item.name == name))
            .and_then(|item| item.scales.get(item.place).cloned())
            .map(|scales| (builds, scales))
            .ok_or_else(|| {
                format!("0x5a03c0: loaded catalog {location} does not define skeleton {name:?}")
            })
    }

    /// The load itself: draws nothing here, but returns the builds and
    /// registers the catalog's skeletons, or marks `name` failed.
    fn read_catalog(&mut self, skeletons: &Skeletons, name: &str) -> Result<Vec<Vec<i32>>, String> {
        let failed = |loaded: &mut BTreeMap<String, Option<String>>, reason: String| {
            loaded.insert(name.to_owned(), None);
            Err(reason)
        };
        let location = match skeletons.indexed.get(name) {
            Some(Ok(location)) => location,
            Some(Err(reason)) => return failed(&mut self.loaded, reason.clone()),
            None => return Err(format!("0x5a03c0: no skeleton {name:?} is registered")),
        };
        let Some((items, error)) = skeletons.catalogs.get(location) else {
            return failed(
                &mut self.loaded,
                format!("0x5a3810: the catalog {location} for skeleton {name:?} is unavailable"),
            );
        };
        if let Some(first) = items.first() {
            self.files.insert(first.file.clone());
        }
        let builds = items
            .iter()
            .filter_map(|item| item.scales.get(item.place).cloned())
            .collect();
        for item in items {
            if !skeletons.eager.contains_key(&item.name) && !self.before.contains_key(&item.name) {
                self.loaded
                    .entry(item.name.clone())
                    .or_insert_with(|| Some(location.clone()));
            }
        }
        if matches!(self.loaded.get(name), Some(Some(_))) {
            return Ok(builds);
        }
        let reason = match error {
            Some(error) => format!("0x5a3810: the skeletons of {location} do not decode ({error})"),
            None => format!("0x5a03c0: {location} does not define skeleton {name:?}"),
        };
        failed(&mut self.loaded, reason)
    }
}

/// A celebrity's sex follows its place in the family (0x42c6d7): the
/// first seven are women, named from table 0xa6 ("Nora" by default), the
/// rest men, from table 0xa5 ("Nomad"); the editor sets it (0x42cf5d →
/// 0x4eaaba). Sex 1 is female.
fn check_celebrity_sex(ordinal: usize, gender: i16) -> Result<(), String> {
    let female = i16::from(ordinal <= 6);
    if gender == female {
        Ok(())
    } else {
        Err(format!(
            "0x42c6d7: celebrity {ordinal} has sex {gender}; its place makes it {female}"
        ))
    }
}

/// The Townie worker's specialty templates (kind 0x400) not yet used.
struct Specialties {
    remaining: Vec<u32>,
    /// Whether the worker made the family (0x42aab1): only then does it
    /// take specialties before anything else.
    new_family: bool,
}

impl Specialties {
    /// Checks that a Townie, the family's member `ordinal`, could be made
    /// from `source`, and uses up that specialty.
    ///
    /// While a new family has fewer than ten members and specialties
    /// remain, each person is a specialty the bounded draw picks
    /// (0x42ac96), which leaves the list once made (0x42acc1). Then, up
    /// to the target, a person is a remaining specialty when a draw of
    /// five gives 3 (0x42ae75, 0x42ae8f, 0x42aebe) and the generic
    /// template otherwise (0x42aecf). The draws are on the game stream,
    /// which is not ported, so only whether the template could be the
    /// pick is checked. A GUID registered twice is two candidates, each
    /// used up once; replaying a pick of one fails earlier, since which
    /// definition its skeleton name is read from is unported.
    fn take(
        &mut self,
        source: u32,
        ordinal: usize,
        generic: bool,
        resolves: bool,
    ) -> Result<(), BootError> {
        let first = self.new_family && ordinal < 10 && !self.remaining.is_empty();
        if let Some(at) = self.remaining.iter().position(|&guid| guid == source)
            && resolves
        {
            self.remaining.remove(at);
            return Ok(());
        }
        if generic && !first {
            return Ok(());
        }
        let address = if first { "0x42ac96" } else { "0x42ae8f" };
        Err(format!(
            "{address}: Townie {ordinal}'s template {source:#010x} is not a specialty the draw \
             could still pick{}",
            if first {
                " (a new family takes specialties first)"
            } else {
                " nor the generic one"
            }
        )
        .into())
    }
}

/// The recorded STR# body strings named by OBJD word 19 (0x457f60),
/// decoded as the actor's profile (0x5888d3). The human semi-global tone
/// table (STR# 8192) at the supported Complete Collection layout indexes
/// light, dark and medium this way.
/// Returns the skeleton 0x40cc2a sets for the profile's age, `adult` or
/// `child`. With `check_skeleton` (a pool that makes a second actor from
/// it), the body strings' skeleton must be that one; a one-actor pool
/// copies only the template's.
fn read_profile(
    iff: &Iff,
    objd: &[u8],
    language: u8,
    check_skeleton: bool,
    data: &mut [i16],
) -> Result<&'static str, BootError> {
    let id = u16::from_le_bytes(objd[38..40].try_into()?);
    let body = iff
        .get(b"STR#", id)
        .ok_or("0x457f60: captured pool character has no body strings")?;
    let strings = StringTable::parse(iff.data(body))?;
    let string = |index| strings.get(index, language);
    let gender = string(characters::GENDER);
    let age = string(characters::AGE);
    let skin = string(characters::SKIN);
    let tones = ["", "lgt", "drk", "med"];
    let skeleton = match age {
        Some("27") => "adult",
        Some("9") => "child",
        _ => return Err(format!("0x40cc2a: unsupported age in pool profile {age:?}").into()),
    };
    let matches = |value: Option<&str>, choices: &[&str]| {
        value.is_some_and(|value| {
            choices
                .iter()
                .any(|choice| value.eq_ignore_ascii_case(choice))
        })
    };
    if !matches(gender, &["male", "female"])
        || !matches(skin, &tones[1..])
        || (check_skeleton && !matches(string(characters::SKELETON), &[skeleton]))
    {
        return Err("0x5888d3: unsupported pool character profile".into());
    }
    let profile = Profile {
        gender: gender.map(str::to_owned),
        age: age.map(str::to_owned),
        skin: skin.map(str::to_owned),
        tones: tones.map(str::to_owned).to_vec(),
        ..Profile::default()
    };
    reload_profile(data, &profile).map_err(|error| format!("0x5888d3: {error}"))?;
    Ok(skeleton)
}

/// The Townie and tourist wrappers' collision step (0x42b05e to 0x42b09f):
/// each family at one of `pool`'s reserved IDs moves to the largest ID
/// plus one, noted in `renumbered`.
fn free_reserved_ids(
    families: &mut BTreeMap<u16, Family>,
    pool: &Pool,
    renumbered: &mut Vec<(u16, u16)>,
) -> Result<(), BootError> {
    for id in pool.families.clone() {
        if !families.contains_key(&id) {
            continue;
        }
        if pool.tests_done {
            // These wrappers use the loop's terminal index on a hit.
            return Err(format!(
                "{}: reserved family {id} sends the wrapper past the end of its family vector",
                pool.collision
            )
            .into());
        }
        // The Townie and tourist wrappers renumber the first match to
        // the largest family ID plus one (0x42b05e to 0x42b09f).
        let replacement = families.last_key_value().unwrap().0 + 1;
        if replacement > i16::MAX as u16 {
            return Err(format!(
                "{}: renumbered family {id} would exceed the signed FAMI ID range",
                pool.collision
            )
            .into());
        }
        let family = families.remove(&id).expect("reserved family");
        families.insert(replacement, family);
        renumbered.push((id, replacement));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(house: i32) -> Family {
        Family {
            version: 9,
            house,
            number: 0,
            budget: 0,
            architecture_value: 0,
            friends: 0,
            flags: 0,
            members: Vec::new(),
            tail: vec![0; 16],
        }
    }

    #[test]
    fn a_household_at_a_reserved_id_is_moved_and_noted() {
        // The Townie wrapper moves family 2000 to the largest ID plus one,
        // and notes the move so its save inputs follow it.
        let mut families = BTreeMap::from([(5, family(1)), (2000, family(2)), (4001, family(3))]);
        let mut renumbered = Vec::new();
        free_reserved_ids(&mut families, &TOWNIES, &mut renumbered).unwrap();
        assert_eq!(renumbered, [(2000, 4002)]);
        assert_eq!(
            families.keys().copied().collect::<Vec<_>>(),
            [5, 4001, 4002]
        );
        assert_eq!(families[&4002].house, 2);
        // With the reserved IDs free, nothing moves.
        free_reserved_ids(&mut families, &TOURISTS, &mut renumbered).unwrap();
        assert_eq!(renumbered, [(2000, 4002)]);
    }

    #[test]
    fn the_first_seven_celebrities_are_women() {
        for ordinal in 0..=6 {
            assert!(check_celebrity_sex(ordinal, 1).is_ok());
            assert!(
                check_celebrity_sex(ordinal, 0)
                    .unwrap_err()
                    .starts_with("0x42c6d7")
            );
        }
        for ordinal in [7, 8, 20] {
            assert!(check_celebrity_sex(ordinal, 0).is_ok());
            assert!(check_celebrity_sex(ordinal, 1).is_err());
        }
    }

    const A: u32 = 0x0a;
    const B: u32 = 0x0b;
    const GENERIC: u32 = 0x99;

    fn specialties(new_family: bool) -> Specialties {
        Specialties {
            remaining: vec![A, B, A],
            new_family,
        }
    }

    #[test]
    fn a_new_townie_family_takes_specialties_first() {
        let mut townies = specialties(true);
        let error = townies
            .take(GENERIC, 0, true, true)
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("0x42ac96"), "{error}");
        // Duplicates are separate candidates, each used up once.
        townies.take(A, 0, false, true).unwrap();
        townies.take(A, 1, false, true).unwrap();
        townies.take(B, 2, false, true).unwrap();
        // With none left, the generic template fills the rest.
        townies.take(GENERIC, 3, true, true).unwrap();
        let error = townies.take(A, 4, false, true).unwrap_err().to_string();
        assert!(error.starts_with("0x42ae8f"), "{error}");
    }

    #[test]
    fn a_used_or_unresolved_specialty_is_not_picked_again() {
        let mut townies = specialties(false);
        // An existing family runs only the second phase: generic or a
        // remaining specialty.
        townies.take(GENERIC, 0, true, true).unwrap();
        townies.take(B, 1, false, true).unwrap();
        let error = townies.take(B, 2, false, true).unwrap_err().to_string();
        assert!(error.starts_with("0x42ae8f"), "{error}");
        assert!(townies.take(A, 3, false, false).is_err());
    }

    fn skeleton(name: &str, file: &str, place: usize, scales: &[Vec<i32>]) -> Skeleton {
        Skeleton {
            name: name.to_owned(),
            file: file.to_owned(),
            place,
            scales: scales.to_vec(),
            bones: Vec::new(),
        }
    }

    /// `kat` and `dog` share an `.ndx` catalog; `adult` is eager.
    fn registry() -> Skeletons {
        let scales = [vec![1, 2], vec![1]];
        let pets = vec![
            skeleton("kat", "pets.bcf", 0, &scales),
            skeleton("dog", "pets.bcf", 1, &scales),
        ];
        Skeletons {
            eager: [("adult".to_owned(), vec![1, 1])].into(),
            indexed: [
                ("kat".to_owned(), Ok("pets".to_owned())),
                ("dog".to_owned(), Ok("pets".to_owned())),
                ("bad".to_owned(), Err("blocked".to_owned())),
            ]
            .into(),
            catalogs: [("pets".to_owned(), (pets, None))].into(),
            ..Skeletons::default()
        }
    }

    #[test]
    fn a_skeleton_is_loaded_on_first_use_with_its_catalogs_draws() {
        let skeletons = registry();
        let mut lazy = LazySkeletons::default();
        assert_eq!(
            lazy.load(&skeletons, "adult").unwrap(),
            (vec![], vec![1, 1])
        );
        // The first use builds every skeleton in the catalog, in order,
        // and the copy takes the looked-up skeleton's own scales.
        assert_eq!(
            lazy.load(&skeletons, "dog").unwrap(),
            (vec![vec![1, 2], vec![1]], vec![1])
        );
        // Its sibling came with it, and neither loads again.
        assert_eq!(lazy.load(&skeletons, "kat").unwrap(), (vec![], vec![1, 2]));
        assert_eq!(lazy.load(&skeletons, "dog").unwrap(), (vec![], vec![1]));
        assert_eq!(lazy.files, BTreeSet::from(["pets.bcf".to_owned()]));
        assert_eq!(lazy.loaded["kat"].as_deref(), Some("pets"));
        // A blocked one fails, and is marked so it fails the same way after.
        assert_eq!(lazy.load(&skeletons, "bad").unwrap_err(), "blocked");
        assert!(
            lazy.load(&skeletons, "bad")
                .unwrap_err()
                .starts_with("0x5a389b")
        );
        assert!(
            lazy.load(&skeletons, "nobody")
                .unwrap_err()
                .starts_with("0x5a03c0")
        );
    }

    #[test]
    fn the_first_catalog_to_register_a_skeleton_keeps_it() {
        // `bigkat` is indexed to its own catalog, which also defines `kat`
        // with other scales; `kat` itself is indexed to the pets catalog.
        let mut skeletons = registry();
        let big = vec![
            skeleton("bigkat", "big.bcf", 0, &[vec![2], vec![3, 3]]),
            skeleton("kat", "big.bcf", 1, &[vec![2], vec![3, 3]]),
        ];
        skeletons
            .indexed
            .insert("bigkat".to_owned(), Ok("big".to_owned()));
        skeletons.catalogs.insert("big".to_owned(), (big, None));
        let mut lazy = LazySkeletons::default();
        lazy.load(&skeletons, "bigkat").unwrap();
        // The copy takes the big catalog's `kat`, which registered first.
        assert_eq!(lazy.load(&skeletons, "kat").unwrap(), (vec![], vec![3, 3]));
        // Loaded before the pools in the other order, the pets' one wins.
        let names = ["kat".to_owned(), "bigkat".to_owned()];
        let mut lazy = LazySkeletons::loaded_before(&skeletons, &names).unwrap();
        assert_eq!(lazy.load(&skeletons, "kat").unwrap(), (vec![], vec![1, 2]));
    }

    #[test]
    fn a_skeleton_loaded_before_the_pools_draws_nothing_there() {
        let skeletons = registry();
        let mut lazy = LazySkeletons::loaded_before(&skeletons, &["kat".to_owned()]).unwrap();
        assert_eq!(lazy.load(&skeletons, "dog").unwrap(), (vec![], vec![1]));
        assert!(lazy.loaded.is_empty() && lazy.files.is_empty());
    }

    #[test]
    fn a_new_family_past_ten_members_may_use_the_generic_template() {
        let mut townies = specialties(true);
        townies.take(GENERIC, 10, true, true).unwrap();
    }
}
