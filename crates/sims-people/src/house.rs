//! The people a saved house holds, with what is needed to draw them.

use crate::appearance::Outfit;
use sims_content::{Catalog, House, Index, Install, Neighborhood, People};
use sims_formats::saved_objects::Person;
use std::path::Path;

/// A person of a saved house.
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
    /// The saved person record.
    pub person: Person,
}

/// Reads the people of the house file `house`, which must be in a
/// neighborhood's `Houses` directory of the install at `root`, in object
/// ID order.
pub fn residents(root: &Path, house: &Path) -> Result<Vec<Resident>, String> {
    let fail = |e: &dyn std::fmt::Display| e.to_string();
    let install = Install::detect(root).map_err(|e| fail(&e))?;
    let directory = house
        .parent()
        .and_then(Path::parent)
        .ok_or("the house file must be in a neighborhood's Houses directory")?;
    let neighborhood = Neighborhood::open(directory).map_err(|e| fail(&e))?;
    let mut index = Index::build(&install).map_err(|e| fail(&e))?;
    if let Some(characters) = neighborhood.characters().map_err(|e| fail(&e))? {
        index.add_characters(&characters).map_err(|e| fail(&e))?;
    }
    let catalog = Catalog::build(&index).map_err(|e| fail(&e))?;
    let house = House::load(house, &catalog).map_err(|e| fail(&e))?;
    let people = People::load(
        &install,
        &index,
        &catalog,
        house.objects.instances.values().map(|o| o.guid),
    )
    .map_err(|e| fail(&e))?;
    let mut out = Vec::new();
    for (&id, object) in &house.objects.instances {
        let Some(person) = &object.person else {
            continue;
        };
        let skeleton = people
            .skeletons
            .get(&object.guid)
            .ok_or_else(|| format!("person {id} has no skeleton"))?;
        out.push(Resident {
            id,
            guid: object.guid,
            skeleton: skeleton.name.clone(),
            outfit: Outfit::from_person(person),
            person: person.clone(),
        });
    }
    Ok(out)
}
