//! Runtime career strings populated by the tail of native 403d64. Keep these
//! separate from the raw CARR records: STR# overlays do not change that asset.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Level {
    /// Native +54/+e0; the gender selector falls back when the second is empty.
    pub titles: [String; 2],
    /// Retain the separately loaded +16c/+1f8 strings without guessing a use.
    pub secondary: [String; 2],
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Text {
    pub name: String,
    pub offers: [String; 2],
    pub levels: Vec<Level>,
}
impl Text {
    pub fn from_carr(record: &sims_formats::career::Career) -> Self {
        Self {
            name: record.name.clone(),
            offers: Default::default(),
            levels: record
                .levels
                .iter()
                .map(|level| Level {
                    titles: [level.strings[0].clone(), String::new()],
                    secondary: Default::default(),
                    description: String::new(),
                })
                .collect(),
        }
    }
}

/// Entries use the original one-based StringSet indices. Absent base resources
/// retain CARR text. Ordinal 10 is skipped, independently of its career ID.
pub fn localize(
    ids: &[i32],
    text: &mut [Text],
    tables: &BTreeMap<u16, BTreeMap<usize, String>>,
    expansions: i16,
) -> Result<(), String> {
    let count = 11 + usize::from(expansions > 0) * 5 + usize::from(expansions >= 32) * 5;
    if ids.len() != text.len() || text.len() < count {
        return Err("career localization vector shorter than native expansion profile".into());
    }
    let fixed = |value: String, capacity: usize| -> Result<String, String> {
        // The supported English/Windows-1252 path has one byte per scalar.
        // Overlong fixed-buffer truncation is not yet independently compared.
        if value.chars().count() >= capacity {
            return Err("career localization fixed-buffer truncation required".into());
        }
        Ok(value)
    };
    for ordinal in 0..count {
        if ordinal == 10 {
            continue;
        }
        let id = ids[ordinal] as u16;
        let Some(base) = tables.get(&id) else {
            continue;
        };
        let alternate = tables.get(&id.wrapping_add(1000));
        let read = |table: &BTreeMap<usize, String>, entry: usize| -> Result<String, String> {
            table.get(&entry).cloned().ok_or_else(|| format!(
                "career STR#{id} entry {entry} missing; native missing-entry string service required"))
        };
        let record = &mut text[ordinal];
        if record.levels.len() < 10 {
            return Err("career localization requires ten allocated native levels".into());
        }
        record.name = fixed(read(base, 1)?, 64)?;
        record.offers = [read(base, 2)?, read(base, 3)?];
        for (i, level) in record.levels.iter_mut().take(10).enumerate() {
            let title = 4 + i * 2;
            let second = 24 + i;
            level.secondary = [
                fixed(read(base, title)?, 128)?,
                fixed(read(base, second)?, 128)?,
            ];
            let (table, offset) = alternate.map_or((base, 1000), |table| (table, 0));
            level.titles = [
                fixed(read(table, title + offset)?, 128)?,
                fixed(read(table, second + offset)?, 128)?,
            ];
            level.description = read(base, title + 1)?;
        }
    }
    Ok(())
}
