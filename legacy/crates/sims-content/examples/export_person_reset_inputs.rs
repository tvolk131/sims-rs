//! Private original resident profile strings and pre-reset state, not game exports.
use serde_json::json;
use sims_content::{Index, saves::House};
use sims_formats::{iff::Iff, strings::StringTable};
fn main() -> anyhow::Result<()> {
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut tables = std::collections::BTreeMap::new();
    for name in ["PersonGlobals.iff", "Global.iff"] {
        let iff = Iff::parse(index.resources[index.find(name)?].read()?)?;
        if let Some(c) = iff.get(b"STR#", 8192) {
            let table = StringTable::parse(iff.data(c))?
                .entries
                .into_iter()
                .filter(|e| e.language == 1)
                .map(|e| e.text)
                .collect::<Vec<_>>();
            println!("{name} STR#8192 {table:?}");
            tables.insert(name, table);
        }
    }
    std::fs::write(
        "cache/research/sims-exe/oracle/person-profile-tables.json",
        serde_json::to_vec_pretty(&tables)?,
    )?;
    let mut people = vec![];
    for source in [
        "UserData/Houses/House07.iff",
        "UserData/Houses/House05.iff",
        "UserData2/Houses/House09.iff",
    ] {
        let house = House::load(&index, source)?;
        for person in &house.household.as_ref().unwrap().residents {
            let iff = Iff::parse(index.resources[index.find(&person.character_source)?].read()?)?;
            let mut strings = std::collections::BTreeMap::new();
            for c in iff.all(b"STR#").filter(|c| [200, 8192].contains(&c.id)) {
                strings.insert(
                    c.id,
                    StringTable::parse(iff.data(c))?
                        .entries
                        .into_iter()
                        .filter(|e| e.language == 1)
                        .map(|e| e.text)
                        .collect::<Vec<_>>(),
                );
            }
            let object = &house.objects.instances[&person.object];
            people.push(json!({"house":source,"resident":person,"strings":strings,"saved_person":object.person,"position":object.position}));
            println!("{} #{} {:?}", person.name, person.object, strings);
        }
    }
    std::fs::write(
        "cache/research/sims-exe/oracle/person-reset-inputs.json",
        serde_json::to_vec_pretty(&people)?,
    )?;
    Ok(())
}
