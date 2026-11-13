//! `sims houses`: load every house in a neighborhood against an install.

use sims_content::{Catalog, House, Index, Install, Neighborhood};
use std::fmt::Write as _;
use std::path::Path;

pub fn report(root: &Path, neighborhood: &Path) -> sims_content::Result<(String, bool)> {
    let install = Install::detect(root)?;
    let neighborhood = Neighborhood::open(neighborhood)?;
    let mut index = Index::build(&install)?;
    if let Some(characters) = neighborhood.characters()? {
        index.add_characters(&characters)?;
    }
    let catalog = Catalog::build(&index)?;
    let people = catalog.objects.iter().filter(|o| o.kind == 2).count();
    let mut out = String::new();
    let _ = writeln!(out, "Neighborhood: {}", neighborhood.directory.display());
    let _ = writeln!(
        out,
        "Families: {}, person definitions: {people}\n",
        neighborhood.families.len()
    );
    let _ = writeln!(
        out,
        "{:>5} {:>6} {:>5} {:>8} {:>7} {:>6} {:>7} {:>9}",
        "house", "family", "SIMI", "ticks", "objects", "people", "arrays", "funds"
    );
    let mut ok = true;
    for (number, path) in neighborhood.houses()? {
        let family = neighborhood
            .family_in(number)
            .map_or_else(|| "-".to_owned(), |(id, _)| id.to_string());
        match House::load(&path, &catalog) {
            Ok(house) => {
                let people = house
                    .objects
                    .instances
                    .values()
                    .filter(|i| catalog.object(i.guid).is_some_and(|o| o.kind == 2))
                    .count();
                let _ = writeln!(
                    out,
                    "{number:>5} {family:>6} {:>5} {:>8} {:>7} {people:>6} {:>7} {:>9}",
                    format!("0x{:x}", house.simulator.version),
                    house.simulator.ticks,
                    house.objects.instances.len(),
                    house.arrays.len(),
                    house.simulator.funds,
                );
            }
            Err(error) => {
                ok = false;
                let _ = writeln!(out, "{number:>5} {family:>6} error: {error}");
            }
        }
    }
    Ok((out, ok))
}
