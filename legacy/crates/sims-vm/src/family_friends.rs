//! 427219/427bd7 family count cache. Native GUID/slot owners are independent
//! from converted family member IDs. Inspection plans; execution publishes.
use crate::{autonomy_constants::Owner, person::WorldState};

struct Read {
    count: i32,
    cache: Option<(i16, i32)>,
}

impl WorldState {
    pub(crate) fn mutual_friend_read(
        &self,
        a: &[i32],
        b: &[i32],
        constants: Option<&Owner>,
    ) -> Result<bool, String> {
        // 4271c5 checks both directed row lengths before reading the threshold.
        if a.is_empty() || b.is_empty() {
            return Ok(false);
        }
        let threshold = constants
            .ok_or("native friendship threshold unavailable")?
            .friendship_threshold()?;
        Ok(crate::person::mutual_friend(a, b, threshold))
    }

    fn first_friend_guid(&self, guid: u32) -> Result<Option<i16>, String> {
        if guid == 0 {
            return Ok(None);
        }
        let slots = self
            .neighbor_slots
            .as_ref()
            .ok_or("native friend scan slots unavailable")?;
        let mut found = None;
        for &id in slots.iter().flatten() {
            let n = self
                .neighbors
                .get(&id)
                .ok_or("friend GUID slot record absent")?;
            // 42571f keeps the first result but still reads all later slots.
            if n.guid == guid && found.is_none() {
                found = Some(id);
            }
        }
        Ok(found)
    }

    fn family_friend_read(
        &self,
        family: i16,
        force: bool,
        constants: Option<&Owner>,
    ) -> Result<Read, String> {
        let metadata = self
            .family_metadata
            .get(&family)
            .ok_or("native family friend cache owner unavailable")?;
        if !metadata.dirty && !force {
            return Ok(Read {
                count: metadata
                    .friend_count
                    .ok_or("native family friend cache uninitialized")?,
                cache: None,
            });
        }
        let slots = self
            .neighbor_slots
            .as_ref()
            .ok_or("native friend scan slots unavailable")?;
        let mut count = 0i32;
        for &id in slots.iter().flatten() {
            let candidate = self
                .neighbors
                .get(&id)
                .ok_or("family friend scan slot record absent")?;
            let members = metadata
                .member_guids
                .as_ref()
                .ok_or("native family friend GUID owner unavailable")?;
            if members.contains(&candidate.guid) {
                continue;
            }
            if *candidate
                .person
                .get(&61)
                .ok_or("family friend scan family word absent")?
                == 0
            {
                continue;
            }
            for &guid in members {
                let Some(member) = self.first_friend_guid(guid)? else {
                    continue;
                };
                let owner = &self.neighbors[&member];
                if self.mutual_friend_read(
                    candidate
                        .relationships
                        .get(&member)
                        .map_or(&[], Vec::as_slice),
                    owner.relationships.get(&id).map_or(&[], Vec::as_slice),
                    constants,
                )? && *candidate
                    .person
                    .get(&65)
                    .ok_or("family friend scan species word absent")?
                    <= 1
                {
                    count = count.wrapping_add(1);
                    break;
                }
            }
        }
        Ok(Read {
            count,
            cache: Some((family, count)),
        })
    }

    fn family_friends_read(&self, slot: i16, constants: Option<&Owner>) -> Result<Read, String> {
        if slot <= 0 {
            return Ok(Read {
                count: 0,
                cache: None,
            });
        }
        let id = self
            .neighbor_slots
            .as_ref()
            .ok_or("native neighbor slots unavailable for family friends")?
            .get(
                usize::try_from(slot)
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .unwrap_or(usize::MAX),
            )
            .copied()
            .flatten();
        let Some(id) = id else {
            return Ok(Read {
                count: 0,
                cache: None,
            });
        };
        let neighbor = self
            .neighbors
            .get(&id)
            .ok_or("family friend neighbor slot record absent")?;
        let family = *neighbor
            .person
            .get(&61)
            .ok_or("neighbor family field unavailable")?;
        if family == 0 || !self.families.contains_key(&family) {
            return Ok(Read {
                count: 0,
                cache: None,
            });
        }
        self.family_friend_read(family, false, constants)
    }

    fn publish_family_friend_read(&mut self, read: &Read) {
        if let Some((family, count)) = read.cache {
            let metadata = self.family_metadata.get_mut(&family).unwrap();
            // 427ca0 clears first, 427ca7 then publishes the full signed count.
            metadata.dirty = false;
            metadata.friend_count = Some(count);
        }
    }

    /// Full-width family getter for immutable host inspection. No publication.
    pub fn family_friend_count(
        &self,
        family: i16,
        constants: Option<&Owner>,
    ) -> Result<i32, String> {
        Ok(self.family_friend_read(family, false, constants)?.count)
    }

    /// Synchronous native getter for an executing host; no plan escapes a call.
    pub fn resolve_family_friend_count(
        &mut self,
        family: i16,
        constants: Option<&Owner>,
    ) -> Result<i32, String> {
        let read = self.family_friend_read(family, false, constants)?;
        self.publish_family_friend_read(&read);
        Ok(read.count)
    }

    /// Direct427bd7 caller, which recomputes even when the getter is clean.
    pub fn recompute_family_friend_count(
        &mut self,
        family: i16,
        constants: Option<&Owner>,
    ) -> Result<i32, String> {
        let read = self.family_friend_read(family, true, constants)?;
        self.publish_family_friend_read(&read);
        Ok(read.count)
    }

    /// Scope24/9 read-only projection; the native value has no writable pointer.
    pub fn family_friends(&self, neighbor: i16, constants: Option<&Owner>) -> Result<i16, String> {
        Ok(self.family_friends_read(neighbor, constants)?.count as i16)
    }

    pub(crate) fn resolve_family_friends(
        &mut self,
        neighbor: i16,
        constants: Option<&Owner>,
    ) -> Result<i16, String> {
        let read = self.family_friends_read(neighbor, constants)?;
        self.publish_family_friend_read(&read);
        Ok(read.count as i16)
    }

    /// 4246b9: neighbor first, then only the first family-header identity match.
    pub fn mark_relationship_dirty(&mut self, neighbor: i16) -> Result<(), String> {
        let n = self
            .neighbors
            .get_mut(&neighbor)
            .ok_or("relationship notification neighbor absent")?;
        n.dirty = true;
        let order = match &self.family_order {
            Some(order) => order,
            None if self.families.is_empty() => return Ok(()),
            None => return Err("native family notification order unavailable".into()),
        };
        if order.is_empty() {
            return Ok(());
        }
        let family = *n
            .person
            .get(&61)
            .ok_or("relationship notification family word absent")?;
        for &id in order {
            if !self.families.contains_key(&id) {
                return Err("relationship notification family header absent".into());
            }
            if id == family {
                self.family_metadata
                    .get_mut(&id)
                    .ok_or("relationship notification family cache owner absent")?
                    .dirty = true;
                break;
            }
        }
        Ok(())
    }
}
