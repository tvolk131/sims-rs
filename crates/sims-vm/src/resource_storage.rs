//! Resource-global variables shared across instances and stored in TATT.
//! Native catalog construction allocates by OBJD +0xb4; access may alias the
//! owning GUID through +0xb8. Original registry order remains significant.
use crate::{ObjectId, Program, World};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub guid: u32,
    pub count: i16,
    pub reset_per_lot: bool,
    pub source: String,
    pub object: u16,
    pub content_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub guid: u32,
    pub values: Vec<i16>,
}
impl Record {
    fn length(count: i32) -> usize {
        if (0..32000).contains(&count) {
            count as usize
        } else {
            0
        }
    }
    /// 0x455f09, including the rejected allocation range.
    pub fn new(guid: u32, count: i32) -> Self {
        Self {
            guid,
            values: vec![0; Self::length(count)],
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub records: Vec<Record>,
}
impl State {
    pub fn construct(catalog: &[Definition]) -> Self {
        Self {
            records: catalog
                .iter()
                .map(|d| Record::new(d.guid, i32::from(d.count)))
                .collect(),
        }
    }
    /// Native reads into the first matching record; an unknown GUID consumes
    /// up to 128 scratch words. An oversized saved count stops this entire
    /// load, retaining earlier rows and leaving this and later rows unchanged.
    pub fn load(&mut self, saved: &sims_formats::tatt::Tatt) -> usize {
        let mut consumed = saved.count_end;
        for row in &saved.entries {
            let destination = self.records.iter_mut().find(|r| r.guid == row.guid);
            let capacity = destination.as_ref().map_or(128, |r| r.values.len());
            if row.values.len() > capacity {
                return row.count_end;
            }
            if let Some(destination) = destination {
                destination.values[..row.values.len()].copy_from_slice(&row.values);
                destination.values[row.values.len()..].fill(0);
            }
            consumed = row.end;
        }
        consumed
    }
    pub fn validate(&self, p: &Program) -> Result<(), String> {
        let Some(catalog) = &p.resource_storage else {
            return if self.records.is_empty() {
                Ok(())
            } else {
                Err("saved resource-global catalog absent".into())
            };
        };
        if self.records.len() != catalog.len()
            || self.records.iter().zip(catalog).any(|(r, d)| {
                r.guid != d.guid || r.values.len() != Record::length(i32::from(d.count))
            })
        {
            return Err("saved resource-global layout differs from catalog".into());
        }
        Ok(())
    }
}
/// Native 0x457e95/0x457eba choose this object's actual resource, rather than
/// the currently executing global/semiglobal frame's resource context.
pub fn key(world: &World, p: &Program, id: ObjectId) -> Result<u32, String> {
    let definition = world
        .objects
        .get(&id)
        .and_then(|o| o.definition.as_ref())
        .ok_or("resource-global object resource absent")?;
    let words = p
        .modules
        .get(&definition.source)
        .and_then(|m| m.object_definition_words.get(&definition.object))
        .ok_or("resource-global OBJD absent")?;
    // Older OBJD promotion is outside this access path's verified profile.
    let alias_words = words
        .get(92..94)
        .ok_or("resource-global OBJD alias fields absent")?;
    let alias = alias_words[0] as u16 as u32 | ((alias_words[1] as u16 as u32) << 16);
    Ok(if alias == 0 { definition.guid } else { alias })
}
