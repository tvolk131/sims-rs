//! Borrowed43ae0a master-definition lookup,45728d cache and487ae6 selected-
//! definition classification. The original return register is significant to
//! 4ceff9 even though Ghidra exported487ae6 with a void prototype.
use super::super::definition::Catalog;
use super::{Fields, Groups, entry, guid, identity, word};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Masters(pub BTreeMap<u32, u32>);
///43ae0a itself does not read or populate45728d's per-definition cache.
///Snapshot restoration calls this entry directly even after a cached selection.
pub fn lookup_master(catalog: &Catalog, id: u32) -> Result<Option<u32>, String> {
    let source = entry(catalog, id)?;
    let group = word(source, 0x14)?;
    if group == 0 {
        return Ok(Some(id));
    }
    for (row, e) in catalog.entries.iter().enumerate() {
        if e.key.resource == source.key.resource && word(e, 0x14)? == group && word(e, 0x16)? == -1
        {
            return Ok(Some(identity(row)?));
        }
    }
    Ok(None)
}
impl Masters {
    /// Null results are not cached: native+8c remains zero and is retried.
    /// Resource identity and signed multipart group must both match. First
    /// registered master wins, independently of the latest-by-GUID lookup.
    pub fn resolve(&mut self, catalog: &Catalog, id: u32) -> Result<Option<u32>, String> {
        entry(catalog, id)?;
        if let Some(&cached) = self.0.get(&id) {
            if cached != 0 {
                return Ok(Some(cached));
            }
        }
        let resolved = lookup_master(catalog, id)?;
        if let Some(master) = resolved {
            self.0.insert(id, master);
        }
        Ok(resolved)
    }
}
impl Groups {
    ///48692b: compare the supplied definition's GUID to each retained list
    ///word, then return first+ordinal with native wrapping arithmetic.
    pub fn index_of(&self, group: usize, catalog: &Catalog, id: u32) -> Result<i32, String> {
        let id = guid(entry(catalog, id)?)?;
        let g = self.0.get(group).ok_or("move-tool group absent")?;
        Ok(match g.guids.iter().position(|&v| v == id) {
            Some(n) => g.first.wrapping_add(n as i32),
            None => 0,
        })
    }
    pub fn select_definition(
        &self,
        f: &mut Fields,
        masters: &mut Masters,
        catalog: &Catalog,
        id: u32,
    ) -> Result<i32, String> {
        f.definition = masters.resolve(catalog, id)?;
        f.subtype = 0;
        f.cursor = if let Some(master) = f.definition {
            if word(entry(catalog, master)?, 0x8a)? != 0 {
                7
            } else {
                6
            }
        } else {
            6
        };
        f.special = 0;
        // Even a failed master lookup reaches the mandatory GUID getter in
        //48692b. Report its invalid receiver after the original prefix writes.
        let master = f
            .definition
            .ok_or("selected move-tool definition has no master")?;
        for group in 0..7 {
            let index = self.index_of(group, catalog, master)?;
            if index != 0 {
                f.mode = group as i32 + 3;
                f.flags = [0, 0, u8::from(group != 6), u8::from(group != 3), 0];
                f.special = u8::from(group < 2);
                f.subtype = u8::from(group == 3);
                return Ok(index);
            }
        }
        f.mode = 1;
        f.flags = [0, 0, 0, 1, 0];
        Ok(0)
    }
}
