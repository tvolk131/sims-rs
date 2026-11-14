//! The people to draw: those a saved house holds, or those a simulation
//! snapshot holds now, each with its skeleton and what it wears.

use crate::appearance::Outfit;
use sims_content::{Catalog, House, Index, Install, Neighborhood, People};
use sims_formats::iff::Iff;
use sims_formats::saved_objects::Person;
use sims_formats::strings::StringTable;
use sims_sim::World;
use std::path::Path;

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

/// The person types of an install and of one neighborhood's characters:
/// what is needed to turn a person object into a [`Resident`].
#[derive(Debug)]
pub struct Cast {
    install: Install,
    index: Index,
    catalog: Catalog,
}

/// The word of a person type's definition that names its body strings
/// (0x457f60).
const BODY_STRINGS: usize = 19 * 2;

impl Cast {
    /// The person types of the install at `root`, with the characters of
    /// the neighborhood in `neighborhood` (the directory that holds its
    /// `Houses` directory).
    pub fn open(root: &Path, neighborhood: &Path) -> Result<Self, String> {
        let fail = |e: &dyn std::fmt::Display| e.to_string();
        let install = Install::detect(root).map_err(|e| fail(&e))?;
        let neighborhood = Neighborhood::open(neighborhood).map_err(|e| fail(&e))?;
        let mut index = Index::build(&install).map_err(|e| fail(&e))?;
        if let Some(characters) = neighborhood.characters().map_err(|e| fail(&e))? {
            index.add_characters(&characters).map_err(|e| fail(&e))?;
        }
        let catalog = Catalog::build(&index).map_err(|e| fail(&e))?;
        Ok(Self {
            install,
            index,
            catalog,
        })
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
    /// [`Outfit::from_body_strings`]). A type that is not a person, or
    /// whose skeleton cannot be chosen, is an error.
    pub fn residents<'a>(
        &self,
        people: impl IntoIterator<Item = (u16, u32, &'a Person)>,
    ) -> Result<Vec<Resident>, String> {
        let people: Vec<_> = people.into_iter().collect();
        let (types, unresolved) = People::load_each(
            &self.install,
            &self.index,
            &self.catalog,
            people.iter().map(|&(_, guid, _)| guid),
        )
        .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for (id, guid, person) in people {
            let skeleton =
                types
                    .skeletons
                    .get(&guid)
                    .ok_or_else(|| match unresolved.get(&guid) {
                        Some(reason) => format!("person {id}: {reason}"),
                        None => format!("person {id} has no skeleton (type {guid:#010x})"),
                    })?;
            // A record whose body is empty has had no clothes chosen.
            let outfit = if person.appearance[0].trim().is_empty() {
                Outfit::from_body_strings(&self.body_strings(guid)?)
            } else {
                Outfit::from_person(person)
            };
            out.push(Resident {
                id,
                guid,
                skeleton: skeleton.name.clone(),
                outfit,
                person: person.clone(),
            });
        }
        Ok(out)
    }

    /// The body strings of the person type `guid`, in their order
    /// (language 1, the first a string table gives).
    pub fn body_strings(&self, guid: u32) -> Result<Vec<String>, String> {
        let object = self
            .catalog
            .object(guid)
            .ok_or_else(|| format!("no object type has the GUID {guid:#010x}"))?;
        let resource = &self.index.resources[object.resource];
        let location = resource.location(&self.index.root);
        let iff = Iff::parse(resource.read().map_err(|e| e.to_string())?)
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
        Ok(strings
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect())
    }
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
/// after the load.
pub fn residents_of(cast: &Cast, world: &World) -> Result<Vec<Resident>, String> {
    cast.residents(
        world
            .objects
            .iter()
            .filter_map(|(&id, o)| o.person.as_ref().map(|p| (id, o.guid, p))),
    )
}
