//! Native Refresh invalidation contracts. Screen rectangles are represented by
//! object IDs: the client redraws the entire viewport, without a rectangle cache.
use crate::{ObjectId, Vm};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DrawState {
    pub level: i32,
    pub dynamic: bool,
    pub expiry: i32,
    pub refreshed_at: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RoomState {
    pub category: u8,
    pub dirty: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RefreshState {
    pub visible_level: i32,
    pub render_clock: u32,
    pub draw: BTreeMap<ObjectId, DrawState>,
    pub invalidated: BTreeSet<ObjectId>,
    /// HouseViewer's F1 room-tile invalidation request (0x468113). The client
    /// redraws the whole viewport, without native per-tile rectangle caches.
    pub redraw_rooms: BTreeSet<u16>,
    pub rooms: BTreeMap<u16, RoomState>,
    pub dirty_categories: u32,
}
impl Vm {
    /// Recovered Commander F1/F2/F3 consumers. Other event families are outside
    /// this API. Invalidations are synchronous; environment/light recomputation
    /// belongs to the later room update, not to this broadcast.
    pub fn refresh_room_event(&mut self, event: u16, value: i32) -> Result<(), String> {
        match event {
            0xf1 => {
                let room = value as u16;
                self.dirty_room(room)?;
                let state = self.world.refresh.as_mut().unwrap();
                if state.rooms.contains_key(&room) {
                    state.redraw_rooms.insert(room);
                }
            }
            0xf2 => {
                // ObjectModule resolves a positive registry ID before emitting
                // F1 with the unsigned room word. Missing objects are harmless.
                if let Ok(id) = ObjectId::try_from(value)
                    && id > 0
                    && let Some(object) = self.world.objects.get(&id)
                {
                    let room = *object
                        .data
                        .get(&29)
                        .ok_or("Refresh room field unavailable")?;
                    if room != -5 {
                        self.refresh_room_event(0xf1, i32::from(room as u16))?;
                    }
                }
            }
            0xf3 => {
                // House compares the full payload to zero-extended 0xfffb;
                // the room-tree lookup itself uses its unsigned low word.
                if value != 0xfffb {
                    self.dirty_room(value as u16)?;
                }
            }
            _ => return Err("unsupported room notification".into()),
        }
        Ok(())
    }
    fn dirty_room(&mut self, id: u16) -> Result<(), String> {
        let state = self
            .world
            .refresh
            .as_mut()
            .ok_or("Refresh room context unavailable")?;
        if let Some(room) = state.rooms.get_mut(&id) {
            room.dirty = true;
            match room.category {
                0 => state.dirty_categories |= 1,
                1 => state.dirty_categories |= 2,
                _ => {}
            }
        }
        Ok(())
    }
    pub fn refresh_object(
        &mut self,
        caller: ObjectId,
        stack: ObjectId,
        selector: i16,
        mode: i16,
    ) -> Result<(), String> {
        let object = match selector {
            0 if self.world.objects.contains_key(&caller) => caller,
            1 if self.world.objects.contains_key(&stack) => stack,
            1 => return Err("native error 0x15: missing Refresh stack object".into()),
            _ => return Err("native error 0x03: invalid Refresh selector".into()),
        };
        match mode {
            0 => {
                let mut root = object;
                let mut seen = BTreeSet::new();
                while let Some((parent, _)) = self
                    .world
                    .objects
                    .get(&root)
                    .ok_or("dangling Refresh container")?
                    .container
                {
                    if !seen.insert(root) {
                        return Err("cyclic Refresh containment".into());
                    }
                    root = parent;
                }
                let mut work = vec![root];
                let mut descendants = Vec::new();
                seen.clear();
                while let Some(id) = work.pop() {
                    if !seen.insert(id) {
                        return Err("cyclic or duplicate Refresh containment".into());
                    }
                    let o = self
                        .world
                        .objects
                        .get(&id)
                        .ok_or("dangling Refresh child")?;
                    descendants.push(id);
                    work.extend(o.slots.iter().rev().flatten().copied());
                }
                let state = self
                    .world
                    .refresh
                    .as_mut()
                    .ok_or("Refresh drawing context unavailable")?;
                if descendants.iter().any(|id| !state.draw.contains_key(id)) {
                    return Err("Refresh object drawing state unavailable".into());
                }
                for id in descendants {
                    let draw = state.draw.get_mut(&id).unwrap();
                    if !draw.dynamic && draw.level <= state.visible_level {
                        state.invalidated.insert(id);
                        draw.dynamic = true;
                    }
                    if draw.dynamic {
                        draw.expiry = i32::MAX;
                        draw.refreshed_at = state.render_clock;
                    }
                }
            }
            1 => self.refresh_room_event(0xf2, i32::from(object))?,
            2 => {
                let room = *self.world.objects[&object]
                    .data
                    .get(&29)
                    .ok_or("Refresh room field unavailable")?;
                if room != -5 {
                    self.dirty_room(room as u16)?;
                }
            }
            _ => return Err("native error 0x03: invalid Refresh mode".into()),
        }
        Ok(())
    }
}
