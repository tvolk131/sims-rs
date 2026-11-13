//! Neighborhood reload operations performed before original house loading.
use crate::person::WorldState;
use std::collections::{BTreeMap, BTreeSet};

/// Fields consumed by neighborhood reload before registering new characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub id: i16,
    pub number: i32,
    pub house: i16,
    pub flags: u32,
    pub friends: i32,
    pub members: Vec<u32>,
}

/// 0x425209 compares the saved family number, then its chunk identity, both
/// descending and signed. Distinct resource IDs make this a total order; the
/// native sort's equal-key permutations are outside this resource profile.
pub fn family_order(families: &[(i16, i32)]) -> Result<Vec<usize>, String> {
    if families.len() > 32767
        || families.iter().map(|f| f.0).collect::<BTreeSet<_>>().len() != families.len()
    {
        return Err("invalid neighborhood family resource identities".into());
    }
    let mut order: Vec<_> = (0..families.len()).collect();
    order.sort_unstable_by_key(|&i| std::cmp::Reverse((families[i].1, families[i].0)));
    Ok(order)
}

impl WorldState {
    /// 0x424b9f..0x424d3d: remove missing member GUIDs, insert the native
    /// empty default family if absent, then order families. Registration of
    /// newly discovered character resources follows this phase. Rebuilding
    /// family words must use these cleaned lists, so a newly registered GUID
    /// cannot resurrect membership that the earlier cleanup removed.
    pub fn prepare_family_reload(&mut self, families: &[Family]) -> Result<Vec<Family>, String> {
        let first = self.first_neighbor_guids()?;
        let mut families = families.to_vec();
        for family in &mut families {
            family.members.retain(|guid| first.contains_key(guid));
        }
        if !families.iter().any(|family| family.id == 0) {
            families.push(Family {
                id: 0,
                number: -1,
                house: 0,
                flags: 0,
                friends: 0,
                members: vec![],
            });
        }
        let keys: Vec<_> = families.iter().map(|f| (f.id, f.number)).collect();
        let order = family_order(&keys)?;
        let ordered: Vec<_> = order.into_iter().map(|i| families[i].clone()).collect();
        self.families = ordered
            .iter()
            .map(|f| (f.id, f.members.iter().map(|guid| first[guid]).collect()))
            .collect();
        self.family_homes = ordered.iter().map(|f| (f.id, f.house)).collect();
        // 41003c starts dirty; the v7..9 4104ce reader loads flags without
        // clearing that byte. The inserted default family retains flags zero.
        self.family_metadata = ordered
            .iter()
            .map(|f| {
                (
                    f.id,
                    crate::family::Metadata {
                        flags: f.flags,
                        dirty: true,
                        friend_count: Some(f.friends),
                        member_guids: Some(f.members.clone()),
                    },
                )
            })
            .collect();
        self.family_order = Some(ordered.iter().map(|f| f.id).collect());
        Ok(ordered)
    }

    pub fn validate_family_order(&self) -> Result<(), String> {
        if let Some(order) = &self.family_order {
            let ids: BTreeSet<_> = order.iter().copied().collect();
            if ids.len() != order.len() || ids != self.families.keys().copied().collect() {
                return Err("invalid native neighborhood family order".into());
            }
        }
        Ok(())
    }

    /// Neighbor scope 24/6 searches family GUID lists in native vector order;
    /// it does not use the person's last-written family word.
    pub fn neighbor_home(&self, neighbor: i16) -> Result<i16, String> {
        let Some(neighbor) = self.neighbors.get(&neighbor) else {
            return Ok(0);
        };
        self.validate_family_order()?;
        let contains = |family: &i16| {
            self.families[family].iter().any(|id| {
                self.neighbors
                    .get(id)
                    .is_some_and(|n| n.guid == neighbor.guid)
            })
        };
        let family = if let Some(order) = &self.family_order {
            order.iter().find(|id| contains(id))
        } else {
            let mut matches = self.families.keys().filter(|id| contains(id));
            let first = matches.next();
            if matches.next().is_some() {
                return Err("native family order unavailable for overlapping membership".into());
            }
            first
        };
        Ok(family
            .and_then(|id| self.family_homes.get(id))
            .copied()
            .unwrap_or(0))
    }

    fn first_neighbor_guids(&self) -> Result<BTreeMap<u32, i16>, String> {
        self.validate_neighbor_slots()?;
        let slots = self
            .neighbor_slots
            .as_ref()
            .ok_or("native neighborhood slots not supplied")?;
        let mut first = BTreeMap::new();
        for &id in slots.iter().flatten() {
            first.entry(self.neighbors[&id].guid).or_insert(id);
        }
        Ok(first)
    }

    /// 0x424c10..0x424c56 removes GUIDs with no neighbor; remaining GUIDs
    /// resolve through the first native slot. Valid duplicates retain their order.
    pub fn resolve_family_members(&self, members: &[u32]) -> Result<Vec<i16>, String> {
        let first = self.first_neighbor_guids()?;
        Ok(members
            .iter()
            .filter_map(|guid| first.get(guid).copied())
            .collect())
    }

    /// 0x425244 clears the NBRS family word before assigning each ordered FAMI
    /// member to its first matching neighbor slot. Missing GUIDs are skipped;
    /// repeated membership overwrites the word without marking the record dirty.
    /// The caller supplies native family order, not map/neighbor-ID order.
    pub fn rebuild_family_links<'a>(
        &mut self,
        families: impl IntoIterator<Item = (i16, &'a [u32])>,
    ) -> Result<(), String> {
        let first = self.first_neighbor_guids()?;
        for neighbor in self.neighbors.values_mut() {
            neighbor.person.insert(61, 0);
        }
        for (family, members) in families {
            for guid in members {
                if let Some(id) = first.get(guid) {
                    self.neighbors
                        .get_mut(id)
                        .unwrap()
                        .person
                        .insert(61, family);
                }
            }
        }
        Ok(())
    }
}
