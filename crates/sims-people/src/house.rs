//! The people to draw: those a saved house holds, or those a simulation
//! snapshot holds now, each with its skeleton and what it wears.

use crate::appearance::Outfit;
use crate::suit::Careers;
use sims_content::{
    ArchivePool, AvatarRegistry, Catalog, House, Index, Install, Neighborhood, People,
};
use sims_formats::career::Career;
use sims_formats::iff::Iff;
use sims_formats::saved_objects::Person;
use sims_formats::strings::StringTable;
use sims_sim::World;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A person to draw.
#[derive(Debug, Clone)]
pub struct Resident {
    /// The person's object ID.
    pub id: u16,
    /// The person type's GUID.
    pub guid: u32,
    /// The skeleton's name (`adult`, `child`), from the type's body
    /// strings.
    pub skeleton: String,
    pub outfit: Outfit,
    /// The person record.
    pub person: Person,
}

/// Pre-resolved person skeleton names shared from a booted lot's catalog scan
/// so [`Cast::residents`] does not rebuild [`AvatarRegistry`] on refresh.
#[derive(Debug, Clone)]
pub struct PreloadedCast {
    pub skeleton_names: BTreeMap<u32, String>,
    pub unresolved: BTreeMap<u32, String>,
}

/// The person types of an install and of one neighborhood's characters:
/// what is needed to turn a person object into a [`Resident`].
pub struct Cast {
    install: Install,
    index: Arc<Index>,
    catalog: Arc<Catalog>,
    pool: Arc<ArchivePool>,
    /// Pre-resolved person types (skeletons + unresolved diagnostics), when
    /// supplied by a caller that already loaded them (e.g. `Game::open`).
    preloaded: Option<PreloadedCast>,
    /// Lazily initialized avatar registry reused across [`Cast::residents`]
    /// calls when `preloaded` is absent or misses a GUID.
    registry: Mutex<Option<AvatarRegistry>>,
    /// Cached language-1 body strings by GUID.
    body_strings_cache: Mutex<BTreeMap<u32, Vec<String>>>,
    /// Cached VitaboyFlags words (OBJD word 102) by GUID.
    vitaboy_flags_cache: Mutex<BTreeMap<u32, u16>>,
    /// The careers of `work.iff`, or None when the install has a
    /// `Careers.txt`, which the original reads in their place (not
    /// ported, as in `sims_vm::program::Program::careers`).
    careers: Option<Careers>,
}

impl fmt::Debug for Cast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cast")
            .field("install", &self.install)
            .field("index", &self.index)
            .field("catalog", &self.catalog)
            .field("careers", &self.careers)
            .finish_non_exhaustive()
    }
}

/// The word of a person type's definition that names its body strings
/// (0x457f60).
const BODY_STRINGS: usize = 19 * 2;

/// The byte of a type's definition that holds its VitaboyFlags word (OBJD
/// word 102; byte +0xcc of the raw words, tested at 0x4376f6).
const VITABOY_FLAGS: usize = 102 * 2;

impl Cast {
    /// The person types of the install at `root`, with the characters of
    /// the neighborhood in `neighborhood` (the directory that holds its
    /// `Houses` directory).
    pub fn open(root: &Path, neighborhood: &Path) -> Result<Self, String> {
        let fail = |e: &dyn std::fmt::Display| e.to_string();
        let install = Install::detect(root).map_err(|e| fail(&e))?;
        let neighborhood = Neighborhood::open(neighborhood).map_err(|e| fail(&e))?;
        let pool = Arc::new(ArchivePool::new());
        let mut index = Index::build(&install).map_err(|e| fail(&e))?;
        if let Some(characters) = neighborhood.characters().map_err(|e| fail(&e))? {
            index.add_characters(&characters).map_err(|e| fail(&e))?;
        }
        let catalog = Catalog::build_with_pool(&index, &pool).map_err(|e| fail(&e))?;
        Self::from_shared(install, Arc::new(index), Arc::new(catalog), None, pool)
    }

    /// Builds a [`Cast`] from already-loaded shared content (`install`,
    /// `index`, `catalog`, optional `preloaded` person types, and `pool`),
    /// avoiding duplicate `Index`/`Catalog`/`AvatarRegistry` construction.
    pub fn from_shared(
        install: Install,
        index: Arc<Index>,
        catalog: Arc<Catalog>,
        preloaded: Option<PreloadedCast>,
        pool: Arc<ArchivePool>,
    ) -> Result<Self, String> {
        let careers = careers_with_pool(&index, &pool)?;
        Ok(Self {
            install,
            index,
            catalog,
            pool,
            preloaded,
            registry: Mutex::new(None),
            body_strings_cache: Mutex::new(BTreeMap::new()),
            vitaboy_flags_cache: Mutex::new(BTreeMap::new()),
            careers,
        })
    }

    /// The careers of the install's `work.iff`, by CARR chunk ID; None
    /// when the install has a `Careers.txt`.
    pub fn careers(&self) -> Option<&Careers> {
        self.careers.as_ref()
    }

    /// The cast of the house file `house`, which must be in a
    /// neighborhood's `Houses` directory.
    pub fn of_house(root: &Path, house: &Path) -> Result<Self, String> {
        let neighborhood = house
            .parent()
            .and_then(Path::parent)
            .ok_or("the house file must be in a neighborhood's Houses directory")?;
        Self::open(root, neighborhood)
    }

    /// The person objects `people` (ID, type GUID, record) as residents,
    /// in the order given. Each one's skeleton comes from its type's body
    /// strings; what it wears comes from its record, or, when the record
    /// names no body, from the type's default outfit (see
    /// [`Outfit::from_body_strings`]). The suit a running simulation
    /// dresses a person in is not applied here; [`crate::People`] applies
    /// it ([`crate::suit`]). A type that is not a person, or whose
    /// skeleton cannot be chosen, is an error.
    pub fn residents<'a>(
        &self,
        people: impl IntoIterator<Item = (u16, u32, &'a Person)>,
    ) -> Result<Vec<Resident>, String> {
        let people: Vec<_> = people.into_iter().collect();
        let all_in_preloaded = self.preloaded.as_ref().is_some_and(|pre| {
            people.iter().all(|&(_, guid, _)| {
                pre.skeleton_names.contains_key(&guid)
                    || pre.unresolved.contains_key(&guid)
                    || self.catalog.object(guid).is_none_or(|o| o.kind != 2)
            })
        });
        let loaded_fallback = if all_in_preloaded {
            None
        } else {
            let mut guard = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let cached = guard.take();
            let result = match cached {
                Some(registry) => {
                    let loaded = People::load_each_with_registry(
                        &self.index,
                        &self.catalog,
                        people.iter().map(|&(_, guid, _)| guid),
                        &registry,
                        &self.pool,
                    )
                    .map_err(|e| e.to_string());
                    // `People::load_each_with_registry` reads `registry`
                    // immutably via `export_skeletons`, so retaining it across
                    // refreshes is always order-independent.
                    *guard = Some(registry);
                    loaded?
                }
                None => {
                    let registry_res = AvatarRegistry::load_with_pool(&self.install, &self.pool);
                    let (loaded, registry) = People::load_each_with_registry_cached(
                        &self.index,
                        &self.catalog,
                        people.iter().map(|&(_, guid, _)| guid),
                        registry_res,
                        &self.pool,
                    )
                    .map_err(|e| e.to_string())?;
                    *guard = Some(registry);
                    loaded
                }
            };
            Some(result)
        };
        let mut out = Vec::new();
        for (id, guid, person) in people {
            let skeleton_name: &str = match (&self.preloaded, &loaded_fallback) {
                (Some(pre), None) => pre
                    .skeleton_names
                    .get(&guid)
                    .map(String::as_str)
                    .ok_or_else(|| match pre.unresolved.get(&guid) {
                        Some(reason) => format!("person {id}: {reason}"),
                        None => format!("person {id} has no skeleton (type {guid:#010x})"),
                    })?,
                (_, Some((types, unresolved))) => types
                    .skeletons
                    .get(&guid)
                    .map(|s| s.name.as_str())
                    .ok_or_else(|| match unresolved.get(&guid) {
                        Some(reason) => format!("person {id}: {reason}"),
                        None => format!("person {id} has no skeleton (type {guid:#010x})"),
                    })?,
                (None, None) => unreachable!("fallback populated when preloaded is absent"),
            };
            // A record whose body is empty has had no clothes chosen.
            let outfit = if person.appearance[0].trim().is_empty() {
                Outfit::from_body_strings(&self.body_strings(guid)?)
            } else {
                Outfit::from_person(person)
            };
            out.push(Resident {
                id,
                guid,
                skeleton: skeleton_name.to_owned(),
                outfit,
                person: person.clone(),
            });
        }
        Ok(out)
    }

    /// The VitaboyFlags word of the object type `guid` (OBJD word 102):
    /// bit 0 ([`crate::bob::ANIMATED`]) gives its objects a 3D graphic
    /// (0x4376f6). A definition too short to have the word has none.
    pub fn vitaboy_flags(&self, guid: u32) -> Result<u16, String> {
        if let Ok(cache) = self.vitaboy_flags_cache.lock()
            && let Some(&cached) = cache.get(&guid)
        {
            return Ok(cached);
        }
        let object = self
            .catalog
            .object(guid)
            .ok_or_else(|| format!("no object type has the GUID {guid:#010x}"))?;
        let resource = &self.index.resources[object.resource];
        let location = resource.location(&self.index.root);
        let iff = Iff::parse(resource.read_with(&self.pool).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{location}: {e}"))?;
        let definition = iff
            .get(b"OBJD", object.chunk)
            .ok_or_else(|| format!("{location}: the type has no definition"))?;
        let flags = iff
            .data(definition)
            .get(VITABOY_FLAGS..VITABOY_FLAGS + 2)
            .map_or(0, |b| u16::from_le_bytes([b[0], b[1]]));
        if let Ok(mut cache) = self.vitaboy_flags_cache.lock() {
            cache.insert(guid, flags);
        }
        Ok(flags)
    }

    /// The suit the graphic of an object of type `guid` wears, if the
    /// type has one ([`crate::bob::animated_suit`]); None for a type
    /// without the bit, or one we cannot read.
    pub fn animated_suit(&self, guid: u32) -> Option<String> {
        let flags = self.vitaboy_flags(guid).ok()?;
        if flags & crate::bob::ANIMATED == 0 {
            return None;
        }
        let rows = self.body_strings(guid).ok()?;
        crate::bob::animated_suit(flags, &rows).map(str::to_owned)
    }

    /// The body strings of the type `guid` as the simulation has them
    /// now: its definition's ([`sims_sim::characters::Characters::definition`],
    /// a made type's template) with the rows the simulation wrote put over
    /// them ([`written_rows`]).
    pub fn rows_of(&self, world: &World, guid: u32) -> Result<Vec<String>, String> {
        let rows = self.body_strings(world.characters.definition(guid))?;
        Ok(with_rows(rows, &written_rows(world, guid)))
    }

    /// The people `people` names (object ID, type GUID, record) as they
    /// are in `world`, in their order. A type is read as its definition
    /// (a made type as its template), and the rows the simulation wrote
    /// ([`written_rows`]) override its skeleton (row 0) and, for a record
    /// that names no body, its default outfit. Each resident keeps the
    /// type GUID it was given.
    pub fn residents_in<'a>(
        &self,
        world: &World,
        people: impl IntoIterator<Item = (u16, u32, &'a Person)>,
    ) -> Result<Vec<Resident>, String> {
        let people: Vec<_> = people.into_iter().collect();
        let mut found = self.residents(
            people
                .iter()
                .map(|&(id, guid, person)| (id, world.characters.definition(guid), person)),
        )?;
        for (resident, &(_, guid, person)) in found.iter_mut().zip(&people) {
            resident.guid = guid;
            let written = written_rows(world, guid);
            if written.is_empty() {
                continue;
            }
            let rows = with_rows(
                self.body_strings(world.characters.definition(guid))?,
                &written,
            );
            if let Some(skeleton) = rows.first().filter(|s| !s.trim().is_empty()) {
                resident.skeleton = skeleton.clone();
            }
            if person.appearance[0].trim().is_empty() {
                resident.outfit = Outfit::from_body_strings(&rows);
            }
        }
        Ok(found)
    }

    /// The body strings of the person type `guid`, in their order
    /// (language 1, the first a string table gives).
    pub fn body_strings(&self, guid: u32) -> Result<Vec<String>, String> {
        if let Ok(cache) = self.body_strings_cache.lock()
            && let Some(cached) = cache.get(&guid)
        {
            return Ok(cached.clone());
        }
        let object = self
            .catalog
            .object(guid)
            .ok_or_else(|| format!("no object type has the GUID {guid:#010x}"))?;
        let resource = &self.index.resources[object.resource];
        let location = resource.location(&self.index.root);
        let iff = Iff::parse(resource.read_with(&self.pool).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{location}: {e}"))?;
        let table = iff
            .get(b"OBJD", object.chunk)
            .and_then(|d| iff.data(d).get(BODY_STRINGS..BODY_STRINGS + 2))
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .ok_or_else(|| format!("{location}: the type names no body strings"))?;
        let strings = iff
            .get(b"STR#", table)
            .ok_or_else(|| format!("{location}: body strings {table} are missing"))?;
        let strings =
            StringTable::parse(iff.data(strings)).map_err(|e| format!("{location}: {e}"))?;
        let rows: Vec<String> = strings
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect();
        if let Ok(mut cache) = self.body_strings_cache.lock() {
            cache.insert(guid, rows.clone());
        }
        Ok(rows)
    }
}

/// The body-string rows the simulation wrote for the type `guid`, by
/// index counting from 0, in the order sims-vm reads them
/// (`read_character_row`): what Make New Character wrote
/// ([`sims_sim::characters::Character::strings`]) wins over the outfit rows
/// it and the outfit choice wrote (`World::made_outfits`). Empty for a
/// type nothing wrote.
pub fn written_rows(world: &World, guid: u32) -> BTreeMap<usize, String> {
    let mut rows = world.made_outfits.get(&guid).cloned().unwrap_or_default();
    if let Some(made) = world.characters.made.get(&guid) {
        rows.extend(made.strings.iter().map(|(&i, s)| (i, s.clone())));
    }
    rows
}

/// `rows` with each of `written` (by index counting from 0) put in its
/// place, growing them with empty rows to reach it.
fn with_rows(mut rows: Vec<String>, written: &BTreeMap<usize, String>) -> Vec<String> {
    for (&index, text) in written {
        if rows.len() <= index {
            rows.resize(index + 1, String::new());
        }
        rows[index] = text.clone();
    }
    rows
}

/// Every CARR of the install's `work.iff` (none without the file), or
/// None when the install has a `Careers.txt`.
fn careers_with_pool(index: &Index, pool: &ArchivePool) -> Result<Option<Careers>, String> {
    if index.named("Careers.txt").next().is_some() {
        return Ok(None);
    }
    let mut careers = Careers::new();
    let Some((_, file)) = index.named("work.iff").next() else {
        return Ok(Some(careers));
    };
    let location = file.location(&index.root);
    let iff = Iff::parse(file.read_with(pool).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{location}: {e}"))?;
    for chunk in iff
        .resource_order(b"CARR")
        .map_err(|e| format!("{location}: {e}"))?
    {
        let career = Career::parse(iff.data(chunk))
            .map_err(|e| format!("{location} CARR #{}: {e}", chunk.id))?;
        careers.insert(chunk.id as i16, career.levels);
    }
    Ok(Some(careers))
}

/// Reads the people of the house file `house`, which must be in a
/// neighborhood's `Houses` directory of the install at `root`, in object
/// ID order.
pub fn residents(root: &Path, house: &Path) -> Result<Vec<Resident>, String> {
    let cast = Cast::of_house(root, house)?;
    let saved = House::load(house, &cast.catalog).map_err(|e| e.to_string())?;
    cast.residents(
        saved
            .objects
            .instances
            .iter()
            .filter_map(|(&id, o)| o.person.as_ref().map(|p| (id, o.guid, p))),
    )
}

/// The people a simulation snapshot holds now, in object ID order:
/// every object with a person record, including people a script made
/// after the load ([`Cast::residents_in`]).
pub fn residents_of(cast: &Cast, world: &World) -> Result<Vec<Resident>, String> {
    cast.residents_in(
        world,
        world
            .objects
            .iter()
            .filter_map(|(&id, o)| o.person.as_ref().map(|p| (id, o.guid, p))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A made type's rows are its template's with the rows Make New
    /// Character wrote put over them.
    #[test]
    fn a_made_types_rows_are_its_templates_with_its_own_written_over() {
        let template = vec!["bones".to_owned(), "coat".to_owned(), "head".to_owned()];
        let written = BTreeMap::from([(1, "fur".to_owned()), (4, "far".to_owned())]);
        assert_eq!(
            with_rows(template.clone(), &written),
            ["bones", "fur", "head", "", "far"]
        );
        assert_eq!(with_rows(template.clone(), &BTreeMap::new()), template);
    }

    /// What Make New Character wrote wins over the outfit rows written for
    /// the same type, as sims-vm reads them; a type nothing wrote has no
    /// written rows.
    #[test]
    fn a_types_written_rows_put_its_own_over_its_outfit_rows() {
        let mut world = World::empty();
        world.characters.made.insert(
            7,
            sims_sim::characters::Character {
                template: 3,
                file: 0,
                strings: BTreeMap::from([(0, "small".to_owned()), (2, "own".to_owned())]),
            },
        );
        world.made_outfits.insert(
            7,
            BTreeMap::from([(1, "coat".to_owned()), (2, "picked".to_owned())]),
        );
        world
            .made_outfits
            .insert(9, BTreeMap::from([(30, "formal".to_owned())]));
        assert_eq!(
            written_rows(&world, 7),
            BTreeMap::from([
                (0, "small".to_owned()),
                (1, "coat".to_owned()),
                (2, "own".to_owned()),
            ])
        );
        assert_eq!(
            written_rows(&world, 9),
            BTreeMap::from([(30, "formal".to_owned())])
        );
        assert!(written_rows(&world, 3).is_empty());
    }
}
