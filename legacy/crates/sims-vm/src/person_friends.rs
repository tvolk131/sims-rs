//! 425b5d field5 ->427148. Inspection computes the same result without
//! publishing the cache; execution publishes it during memory resolution.
use crate::person::WorldState;
use std::collections::BTreeMap;

pub(crate) struct Read {
    pub value: i16,
    pub cache: Option<(i16, i32)>,
}

impl WorldState {
    pub(crate) fn neighbor_friend_count(
        &self,
        resources: Option<&BTreeMap<u32, u32>>,
        slot: i16,
        constants: Option<&crate::autonomy_constants::Owner>,
    ) -> Result<Read, String> {
        let id = if let Some(slots) = &self.neighbor_slots {
            usize::try_from(slot)
                .ok()
                .and_then(|i| i.checked_sub(1))
                .and_then(|i| slots.get(i))
                .copied()
                .flatten()
        } else {
            self.neighbors.contains_key(&slot).then_some(slot)
        };
        let Some(id) = id else {
            return Ok(Read {
                value: 0,
                cache: None,
            });
        };
        let source = self
            .neighbors
            .get(&id)
            .ok_or("neighbor slot record absent")?;
        if !source.dirty {
            let value = source
                .friend_count
                .ok_or("native neighbor friend cache uninitialized")?;
            return Ok(Read {
                value: value as i16,
                cache: None,
            });
        }
        let resources = resources.ok_or("neighbor linked-resource catalog unavailable")?;
        let order: Box<dyn Iterator<Item = i16> + '_> = match &self.neighbor_slots {
            Some(slots) => Box::new(slots.iter().flatten().copied()),
            None => Box::new(self.neighbors.keys().copied()),
        };
        let mut count = 0i32;
        for other_id in order {
            if other_id == id {
                continue;
            }
            let other = self
                .neighbors
                .get(&other_id)
                .ok_or("friend scan slot record absent")?;
            // 423f30: null linked resource is false. Species and OBJD kind are
            // not substitutes for the resource's registration-category bits.
            if resources
                .get(&other.guid)
                .is_none_or(|flags| flags & 0x0c != 4)
            {
                continue;
            }
            if *other
                .person
                .get(&61)
                .ok_or("friend scan family word absent")?
                == 0
            {
                continue;
            }
            if self.mutual_friend_read(
                other.relationships.get(&id).map_or(&[], Vec::as_slice),
                source
                    .relationships
                    .get(&other_id)
                    .map_or(&[], Vec::as_slice),
                constants,
            )? {
                count = count.wrapping_add(1);
            }
        }
        // Original leaves dirty set, so later reads can recompute again.
        Ok(Read {
            value: count as i16,
            cache: Some((id, count)),
        })
    }
}
