//! State written by the five neighborhood pool fills. See
//! docs/decisions/20261120-neighborhood-open-replays-five-pools.md.

use crate::boot::BootError;
use crate::objects::{Draws, PERSON_WORDS};
use crate::people::{SAVED_REVISION, field, reload_profile};
use sims_content::family_import::reserve_character_file;
use sims_content::{Neighborhood, Profile};
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
    /// The skeletons, by name, already loaded when the pools run. A
    /// temporary actor's skeleton not among them would be loaded on first
    /// use (0x5a03cd), whose draws are not ported, so it fails. Both UD2
    /// captures fit `adult` and `child` loaded; that is fitted, not traced.
    pub loaded_skeletons: BTreeSet<String>,
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
    /// Each skeleton's own bone scales, by name: the templates' and a
    /// second actor's (`adult` or `child`).
    pub scales: BTreeMap<String, Result<Vec<i32>, String>>,
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
        loaded_skeletons: &input.loaded_skeletons,
        files,
        made: Vec::new(),
    };
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
    loaded_skeletons: &'a BTreeSet<String>,
    files: BTreeSet<i32>,
    made: Vec<(i32, &'a Iff)>,
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
        let families = &mut self.neighborhood.families;
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
        }
        Ok(())
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
        let mut members = Vec::with_capacity(people.len());
        let mut adults = Vec::new();
        let mut children = 0;
        for person in people {
            let (guid, age, gender) = self.make_person(pool, person)?;
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

    fn make_person(
        &mut self,
        pool: &Pool,
        person: &'a PoolPerson,
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
        let townie_specialty = pool.done == TOWNIES.done
            && self
                .context
                .registry
                .iter()
                .any(|entry| entry.guid == source && entry.kind == 0x400 && entry.resolves);
        if !matches_generic && !townie_specialty {
            return Err(format!(
                "{}: unexpected captured template {source:#010x}",
                pool.worker
            )
            .into());
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
            let scales = match skeletons.scales.get(name) {
                Some(Ok(scales)) => scales.as_slice(),
                Some(Err(reason)) => {
                    return Err(format!(
                        "the skeleton {name:?} for User{file:05}: {reason}"
                    )
                    .into());
                }
                None => {
                    return Err(format!(
                        "0x5a03c0: no skeleton {name:?} looked up for User{file:05}"
                    )
                    .into());
                }
            };
            if !self.loaded_skeletons.contains(name) {
                return Err(format!(
                    "0x5a03cd: the skeleton {name:?} for User{file:05} is not loaded yet; \
                     loading it on first use is unported"
                )
                .into());
            }
            copies.push(scales);
        }
        let (age, gender) = (data[field::AGE], data[field::GENDER]);
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
        for scales in copies {
            self.draws.skeleton(scales);
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
