use anyhow::{Result, ensure};
use sims_content::{Index, saves::House};

#[test]
#[ignore = "requires private original neighborhoods and character resources"]
fn original_neighbors_preserve_default_constructor_cache() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    for source in [
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        let program = house.link_program(&index)?;
        let storage = house.prepare_storage(&program)?;
        let people = storage.world.person.as_ref().unwrap();
        ensure!(people.neighbors.len() == house.neighbors.entries.iter().flatten().count());
        ensure!(
            people
                .neighbors
                .values()
                .all(|n| n.friend_count == Some(0) && n.dirty)
        );
        let catalog = sims_content::object_catalog::Catalog::household(
            std::sync::Arc::new(index.clone()),
            &house.neighborhood_source,
            Default::default(),
        )?;
        for resident in &house.household.as_ref().unwrap().residents {
            let neighbor = &people.neighbors[&resident.neighbor];
            let entry = catalog.definitions.guids[&neighbor.guid];
            ensure!(
                catalog.definitions.entries[entry].flags & 0x0c == 4,
                "resident {} lacks original person resource category",
                resident.neighbor
            );
        }
        println!(
            "{source}: {} original neighbors start dirty with cache zero; resident categories verified",
            people.neighbors.len()
        );
    }
    Ok(())
}
