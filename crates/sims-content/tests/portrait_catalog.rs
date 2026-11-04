use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::{
    Index,
    avatar_installation::{Binding, Installation, complete_collection_asset_bindings},
    object_catalog::{Catalog, Settings},
    object_resources::Resources,
};
use sims_sim::object_scan::Paths;
use sims_vm::image_cache::State;

#[test]
#[ignore = "requires original installation and Mashuga neighborhood"]
fn original_people_catalog_resolves_deceased_neighbor_independently_of_live_objects() -> Result<()>
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let people = b"./UserData2/Characters/".to_vec();
    let mut bindings = complete_collection_asset_bindings(&index.root);
    bindings.push(Binding {
        directory: people.clone(),
        physical: index.root.join("UserData2/Characters"),
    });
    let mut installation = Installation::new(&index, bindings)?;
    let initial = installation.initial_mounts(b"./", 255, false)?;
    // The active-neighborhood physical binding/mount is an explicit host input,
    // not a claim that the complete Windows startup sequence has been run.
    installation.register(&people, false, true)?;
    let paths = Paths {
        objects: initial[1].clone(),
        user_objects: initial[2].clone(),
        people,
        downloads: initial.last().unwrap().clone(),
        expansions: std::array::from_fn(|n| initial[3 + n].clone()),
        shared: initial[11].clone(),
        npcs: initial[12].clone(),
    };
    let mut resources = Resources::new(
        &index,
        installation.directories()?,
        installation.directory_search(),
    );
    let global = resources
        .acquire(&[initial[0].as_slice(), b"Global"].concat(), 0)
        .map_err(anyhow::Error::msg)?
        .context("global resource absent")?;
    let mut catalog = Catalog::new(
        resources,
        Settings {
            filter_flagged: true,
            global_directory: initial[0].clone(),
            global_people: 0,
            global: Some(global),
        },
    );
    installation.scan_objects(&paths, 255, 0, false, &mut catalog)?;
    let mut cache = State::default();
    catalog
        .populate_portraits(&mut cache, true, false, &mut |_, _| {
            panic!("empty cache has no images")
        })
        .map_err(anyhow::Error::msg)?;
    let house = sims_content::saves::House::load(&index, "UserData2/Houses/House09.iff")?;
    let neighbor = house
        .neighbors
        .entries
        .iter()
        .flatten()
        .find(|n| n.id == 48)
        .context("neighbor48 absent")?;
    ensure!(neighbor.guid == 1480716884);
    let slot = cache
        .entries
        .iter()
        .position(|e| e.guid == neighbor.guid)
        .context("deceased neighbor portrait absent")?;
    let handle = cache
        .resolve_resource(slot, |guid, _| catalog.portrait_resource(guid))
        .map_err(anyhow::Error::msg)?
        .context("neighbor resource absent")?;
    let selected = catalog
        .portrait_definition(handle)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        selected.source == "UserData2/Characters/User00000.iff"
            && selected.object == 128
            && selected.guid == neighbor.guid
            && selected.slots == 128
    );
    ensure!(
        catalog
            .live_dialog_resource(&selected)
            .map_err(anyhow::Error::msg)?
            == handle
    );
    // Deliberately supply a different latest-by-GUID binding. Live lookup must
    // still use the retained source/OBJD/slot identity, not this manager index.
    let original_guid_binding = catalog.definitions.guids[&selected.guid];
    let other_index = (original_guid_binding + 1) % catalog.definitions.entries.len();
    catalog.definitions.guids.insert(selected.guid, other_index);
    ensure!(
        catalog
            .portrait_resource(selected.guid)
            .map_err(anyhow::Error::msg)?
            != Some(handle)
    );
    ensure!(
        catalog
            .live_dialog_resource(&selected)
            .map_err(anyhow::Error::msg)?
            == handle
    );
    catalog
        .definitions
        .guids
        .insert(selected.guid, original_guid_binding);
    for field in 0..4 {
        let mut missing = selected.clone();
        match field {
            0 => missing.source.push_str(".absent"),
            1 => missing.object = u16::MAX,
            2 => missing.guid = 0,
            _ => missing.slots ^= 1,
        }
        ensure!(catalog.live_dialog_resource(&missing).is_err());
    }
    let definition = &catalog.definitions.entries[(handle - 1) as usize];
    let strings = u16::from_le_bytes(definition.data[82..84].try_into().unwrap());
    ensure!(
        catalog
            .portrait_chunk(handle, *b"CATS", strings)
            .map_err(anyhow::Error::msg)?
            .is_none(),
        "native legacy name decoding requires its separate service"
    );
    let raw = catalog
        .portrait_chunk(handle, *b"CTSS", strings)
        .map_err(anyhow::Error::msg)?
        .context("catalog name absent")?;
    let names = sims_formats::strings::StringTable::parse(&raw)?;
    let name = names
        .get(0, 1)
        .context("English neighbor name absent")?
        .to_owned();
    ensure!(name == "Frankie");
    let bitmap = catalog
        .portrait_chunk(handle, *b"BMP_", 2002)
        .map_err(anyhow::Error::msg)?
        .context("saved face absent")?;
    let image = image::load_from_memory_with_format(&bitmap, image::ImageFormat::Bmp)?;
    ensure!((image.width(), image.height()) == (105, 41));
    let filename = String::from_utf8(
        catalog
            .portrait_filename(handle)
            .map_err(anyhow::Error::msg)?,
    )?;
    ensure!(
        // The native composite filename service strips the file extension.
        filename
            .replace('\\', "/")
            .to_ascii_lowercase()
            .ends_with("userdata2/characters/user00000"),
        "unexpected portrait resource filename: {filename:?}"
    );
    let urn =
        &index.resources[index.find("GameData_ALL_Recursive/Objects/Objects.far::UrnStone.iff")?];
    let iff = sims_formats::iff::Iff::parse(urn.read()?)?;
    let strings =
        sims_formats::strings::StringTable::parse(iff.data(iff.get(b"STR#", 301).unwrap()))?;
    let mut selected_resource = 0;
    let mut selected_neighbor = 0;
    let text = sims_vm::dialog::format_neighbor(
        strings.get(0, 1).unwrap(),
        48,
        Some(&mut selected_neighbor),
        Some(&mut selected_resource),
        |id| Ok((id == 48).then_some(handle)),
        |_| Ok(name.clone()),
    )
    .map_err(anyhow::Error::msg)?;
    ensure!(
        selected_neighbor == 48
            && selected_resource == handle
            && text.contains("Frankie")
            && !text.contains("$Neighbor")
    );
    let restored: State = serde_json::from_slice(&serde_json::to_vec(&cache)?)?;
    ensure!(cache == restored);
    catalog
        .populate_portraits(&mut cache, false, true, &mut |_, _| {
            panic!("refresh must retain images")
        })
        .map_err(anyhow::Error::msg)?;
    ensure!(cache.entries[slot].resource.is_none());
    ensure!(
        cache
            .resolve_resource(slot, |guid, _| catalog.portrait_resource(guid))
            .map_err(anyhow::Error::msg)?
            == Some(handle)
    );
    let report = json!({"definitions":catalog.definitions.entries.len(),"portraits":cache.entries.len(),
        "neighbor":48,"guid":neighbor.guid,"slot":slot,"resource":handle,"filename":filename,"name":name,"text":text,
        "restored_cache_identical":true,"refresh_rebound_same_resource":true,
        "scope":"Source catalog with explicit active-neighborhood mount; no live object lookup, full startup or regeneration claimed"});
    std::fs::write(
        root.join("cache/original-neighbor-portrait-catalog.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{report}");
    Ok(())
}
