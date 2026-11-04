//! Career identity and field records from the original work.iff CARR resources.
//! Optional Careers.txt table overrides require their separate native loader.
use crate::Index;
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use sims_formats::iff::Iff;
use sims_vm::program::CareerCatalog;
pub fn load(index: &Index) -> Result<CareerCatalog> {
    // Current original-house sessions use the Complete Collection profile.
    load_with_expansions(index, 0xff)
}
pub fn load_with_expansions(index: &Index, expansions: i16) -> Result<CareerCatalog> {
    ensure!(
        !index.resources.iter().any(|r| {
            let name = r.name.replace('\\', "/").to_ascii_lowercase();
            name.rsplit('/').next().is_some_and(|n| n == "careers.txt")
        }),
        "Careers.txt override requires its native table loader"
    );
    let source = &index.resources[index.find("work.iff")?];
    let iff = Iff::parse(source.read()?)?;
    let resources = iff.resource_order(b"CARR")?;
    let ids: Vec<_> = resources.iter().map(|c| i32::from(c.id as i16)).collect();
    let records = resources
        .iter()
        .map(|chunk| {
            let bytes = iff.data(chunk);
            let record = sims_formats::career::Career::parse(bytes)?;
            ensure!(
                record.consumed == bytes.len(),
                "CARR {} has unread data",
                chunk.id
            );
            Ok(record)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut text: Vec<_> = records
        .iter()
        .map(sims_vm::career_text::Text::from_carr)
        .collect();
    let mut tables = std::collections::BTreeMap::new();
    for chunk in iff.chunks.iter().filter(|c| &c.tag == b"STR#") {
        let table = sims_formats::strings::StringTable::parse(iff.data(chunk))?;
        let entries = table
            .entries
            .iter()
            .filter(|e| e.language == 1)
            .enumerate()
            .map(|(i, e)| (i + 1, e.text.clone()))
            .collect();
        tables.insert(chunk.id, entries);
    }
    sims_vm::career_text::localize(&ids, &mut text, &tables, expansions)
        .map_err(anyhow::Error::msg)?;
    Ok(CareerCatalog {
        source: source.locator.clone(),
        content_hash: format!("{:x}", Sha256::digest(&iff.bytes)),
        ids,
        records: Some(records),
        text: Some(text),
    })
}
