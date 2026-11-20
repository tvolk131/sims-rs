use sims_content::{Catalog, Index, Install, Neighborhood};
use sims_formats::family_file::FamilyFile;
use sims_formats::iff::Iff;
use sims_sim::neighbors::registry;
use sims_sim::patch_import::{Context, Patch, open};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
#[expect(clippy::disallowed_methods, reason = "sort paths before using them")]
fn sorted(directory: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

fn file_number(path: &Path) -> Option<i32> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    name.strip_prefix("user")?
        .strip_suffix(".iff")?
        .parse()
        .ok()
}

/// The GUID of each person type in the first neighborhood's
/// `Characters/User%05d.iff`, by file number.
fn character_guids(directory: &Path) -> BTreeMap<i32, u32> {
    let mut guids = BTreeMap::new();
    for path in sorted(&directory.join("Characters")) {
        let Some(number) = file_number(&path) else {
            continue;
        };
        let iff = Iff::parse(fs::read(&path).unwrap()).unwrap();
        let mut objds = iff.all(b"OBJD");
        let objd = iff.data(objds.next().unwrap());
        assert!(objds.next().is_none(), "{}", path.display());
        guids.insert(
            number,
            u32::from_le_bytes(objd[0x1c..0x20].try_into().unwrap()),
        );
    }
    guids
}

const AFTER: &str = "/Users/tommyvolk/Desktop/sims-rs/cache/ud2-import-state-20261120";
const POOLS: [u16; 15] = [
    2000, 3000, 3001, 3002, 3003, 3004, 3005, 3006, 3007, 3008, 3009, 5000, 5500, 6000, 0,
];

#[test]
fn probe() {
    let root =
        PathBuf::from("/Users/tommyvolk/Desktop/sims-rs/cache/installs/cd-complete-collection");
    let install = Install::detect(&root).unwrap();
    let directory = root.join("UserData2");
    let after = Path::new(AFTER);
    let mut index = Index::build(&install).unwrap();
    index.add_characters(&directory.join("Characters")).unwrap();
    let catalog = Catalog::build(&index).unwrap();
    let registry = registry(&index, &catalog);
    let character_mount = index
        .mounts
        .iter()
        .position(|m| m.path == Index::CHARACTERS)
        .unwrap();
    let mut neighborhood = Neighborhood::open(&directory).unwrap();
    let context = Context {
        catalog: &catalog,
        registry: &registry,
        character_files: sorted(&directory.join("Characters"))
            .iter()
            .filter_map(|p| file_number(p))
            .collect(),
        character_guids: catalog
            .objects
            .iter()
            .filter(|o| index.resources[o.resource].mount == character_mount)
            .map(|o| o.guid)
            .collect(),
        house_files: neighborhood.houses().unwrap().into_keys().collect(),
        zoning: neighborhood
            .zoning(u16::from(install.expansions.0))
            .unwrap(),
        lots: neighborhood.lot_rows().unwrap(),
        expansions: install.expansions.0,
        language: 1,
    };
    let names: Vec<String> = sorted(&directory.join("Patch"))
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_owned())
        .collect();
    println!("names {names:?}");
    let patches: Vec<Patch> = names
        .iter()
        .map(|name| Patch {
            name: name.clone(),
            file: FamilyFile::parse(&fs::read(directory.join("Patch").join(name)).unwrap())
                .unwrap(),
        })
        .collect();
    let theirs_guids = character_guids(after);
    let mut draws = (2..30).map(|n| theirs_guids[&n]);
    let opened = open(&mut neighborhood, &patches, &context, &mut || {
        draws.next().expect("more draws")
    })
    .unwrap();
    println!("left draws {}", draws.count());
    println!(
        "next_file {} made {} renumbered {:?}",
        opened.next_file,
        opened.made_records.len(),
        opened.renumbered
    );
    for (name, import) in &opened.imports {
        let files: Vec<i32> = import.made.values().map(|c| c.file).collect();
        println!(
            "I {name} family {} files {files:?} lot {:?}",
            import.family_id,
            import.lot.as_ref().map(|l| l.house)
        );
    }
    let theirs = Neighborhood::open(after).unwrap();
    // families
    for (id, f) in &theirs.families {
        if POOLS.contains(id) && *id != 0 {
            continue;
        }
        match neighborhood.families.get(id) {
            Some(o) if format!("{o:?}") == format!("{f:?}") => {}
            o => println!("FAM {id}: theirs {f:?}\n   ours {o:?}"),
        }
    }
    for id in neighborhood.families.keys() {
        if !theirs.families.contains_key(id) {
            println!("FAM {id} only ours");
        }
    }
    println!(
        "slots ours {} theirs {}",
        neighborhood.neighbors.len(),
        theirs.neighbors.len()
    );
    let mut diffs = 0;
    for (i, o) in neighborhood.neighbors.iter().enumerate() {
        let t = &theirs.neighbors[i];
        if format!("{o:?}") != format!("{t:?}") {
            diffs += 1;
            if diffs <= 6 {
                match (o, t) {
                    (Some(o), Some(t)) => {
                        let words: Vec<_> = o
                            .person_data
                            .iter()
                            .zip(&t.person_data)
                            .enumerate()
                            .filter(|(_, (a, b))| a != b)
                            .map(|(i, (a, b))| (i, *a, *b))
                            .collect();
                        println!(
                            "SLOT {i}: id {} {} guid {:x} {:x} name {} {} mode {} {} words {:?} rel {} {} ver {} {} extra {:?} {:?}",
                            o.id,
                            t.id,
                            o.guid,
                            t.guid,
                            o.name,
                            t.name,
                            o.person_mode,
                            t.person_mode,
                            words,
                            o.relationships.len(),
                            t.relationships.len(),
                            o.version,
                            t.version,
                            o.version_extra,
                            t.version_extra
                        );
                        if words.is_empty() && o.relationships.len() == t.relationships.len() {
                            println!(
                                "  rel ours {:?}\n  rel theirs {:?}",
                                o.relationships, t.relationships
                            );
                        }
                    }
                    _ => println!(
                        "SLOT {i}: ours {:?} theirs {:?}",
                        o.as_ref().map(|n| n.guid),
                        t.as_ref().map(|n| n.guid)
                    ),
                }
            }
        }
    }
    println!("slot diffs {diffs}");
    println!(
        "data eq {} options ours {:?} theirs {:?}",
        neighborhood.data == theirs.data,
        neighborhood.options,
        theirs.options
    );
    println!(
        "inventories eq {} ({} vs {})",
        format!("{:?}", neighborhood.inventories) == format!("{:?}", theirs.inventories),
        neighborhood.inventories.len(),
        theirs.inventories.len()
    );
    println!(
        "tatt eq {}",
        format!("{:?}", neighborhood.type_attributes) == format!("{:?}", theirs.type_attributes)
    );
}
