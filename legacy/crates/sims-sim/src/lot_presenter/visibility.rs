//! 4ab992: merge supplied visible-room masks and the selected person's mask.
//! Visible-room ownership, application/selection resolution and debug view are
//! explicit caller services. This does not schedule an automatic camera update.
use super::{room_masks::rotate, walls::cutaway::Mask};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub enabled: u8,
    pub dirty: u8,
    pub include_visible: u8,
    pub mask: Mask,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Room {
    pub outdoors: bool,
    pub mask: Mask,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selected {
    pub room: u16,
    pub level: i32,
    pub tile: [i8; 3],
}
pub struct Inputs<'a> {
    pub size: i32,
    pub rotation: u8,
    pub bounds: [i32; 4],
    pub visible_level: i32,
    pub mode: i32,
    pub include_visible: u8,
    pub visible_rooms: &'a [u16],
    pub rooms: &'a BTreeMap<u16, Room>,
    /// Resolved person after the original selected/override lookup.
    pub selected: Option<&'a Selected>,
}
fn valid(mask: &Mask) -> bool {
    mask.first_row == 0 && mask.rows.len() == 64
}
fn merge(mask: &mut Mask, room: &Room) -> Result<(), String> {
    if !valid(&room.mask) {
        return Err("native visibility room mask unavailable".into());
    }
    for (a, b) in mask.rows.iter_mut().zip(&room.mask.rows) {
        *a |= *b;
    }
    Ok(())
}
/// The original keeps its previous mask on the stack until after cursor work.
/// Save this value with the caller when composition suspends for OS services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    old: Mask,
    size: i32,
    bounds: [i32; 4],
    include_visible: u8,
}
impl Pending {
    /// 4abdda..4abf79: clip both ends of the cursor's diagonal, then enumerate
    /// the half-open rectangle between them when a visible room is outdoors.
    pub fn cursor(
        &self,
        state: &mut State,
        mut start: [i8; 3],
        outdoors: bool,
    ) -> Result<[i8; 3], String> {
        self.validate()?;
        if !valid(&state.mask) {
            return Err("native cursor mask unavailable".into());
        }
        let mut end = [start[0].wrapping_sub(4), start[1].wrapping_sub(4), start[2]];
        let inside = |p: [i8; 3]| {
            i32::from(p[0]) >= self.bounds[0]
                && i32::from(p[0]) <= self.bounds[1]
                && i32::from(p[1]) >= self.bounds[2]
                && i32::from(p[1]) <= self.bounds[3]
        };
        while !inside(start) && start != end {
            start[0] = start[0].wrapping_sub(1);
            start[1] = start[1].wrapping_sub(1);
        }
        while !inside(end) && end != start {
            end[0] = end[0].wrapping_add(1);
            end[1] = end[1].wrapping_add(1);
        }
        if outdoors && inside(end) {
            for y in end[1].min(start[1])..end[1].max(start[1]) {
                for x in end[0].min(start[0])..end[0].max(start[0]) {
                    if inside([x, y, end[2]]) {
                        state.mask.rows[y as usize] |= 1u64 << x;
                    }
                }
            }
        }
        Ok(end)
    }
    pub fn validate(&self) -> Result<(), String> {
        if !valid(&self.old)
            || !(0..=64).contains(&self.size)
            || self.bounds.iter().any(|&v| !(0..64).contains(&v))
            || self.bounds[0] > self.bounds[1]
            || self.bounds[2] > self.bounds[3]
            || self.bounds[2] + self.size > 64
        {
            return Err("invalid suspended visibility composition".into());
        }
        Ok(())
    }
    pub fn finish(self, state: &mut State) -> Result<(), String> {
        self.validate()?;
        if !valid(&state.mask) {
            return Err("native visibility mask unavailable".into());
        }
        if state.dirty == 0 {
            let start_x = self.bounds[0] as i8 as i32;
            let start_y = self.bounds[2] as i8 as usize;
            'rows: for y in start_y..start_y + self.size as usize {
                let changed = state.mask.rows[y] ^ self.old.rows[y];
                for x in start_x..start_x + self.size {
                    if (0..64).contains(&x) && changed & (1u64 << x) != 0 {
                        state.dirty = 1;
                        break 'rows;
                    }
                }
            }
        }
        state.include_visible = self.include_visible;
        Ok(())
    }
}
impl State {
    pub fn update(&mut self, input: Inputs<'_>) -> Result<(), String> {
        if let Some(pending) = self.begin(input)? {
            pending.finish(self)?;
        }
        Ok(())
    }
    /// Merge rooms and selection before the native debug-view/cursor branch.
    /// Dirty detection and include-visible publication belong to `finish`.
    pub fn begin(&mut self, input: Inputs<'_>) -> Result<Option<Pending>, String> {
        if self.enabled == 0 {
            return Ok(None);
        }
        if !valid(&self.mask)
            || input.rotation > 3
            || !(0..=64).contains(&input.size)
            || input.bounds.iter().any(|&v| !(0..64).contains(&v))
            || input.bounds[0] > input.bounds[1]
            || input.bounds[2] > input.bounds[3]
            || input.bounds[2] + input.size > 64
        {
            return Err("native visibility mask context unavailable".into());
        }
        let old = self.mask.clone();
        // Calculate into bounded local mask storage; missing host inputs do not
        // partially replace the persistent mask or fabricate an empty room.
        let mut next = old.clone();
        if input.include_visible != 0 {
            next.rows.fill(0);
            for id in input.visible_rooms {
                merge(
                    &mut next,
                    input.rooms.get(id).ok_or("native visible room absent")?,
                )?;
            }
        }
        if let Some(p) = input
            .selected
            .filter(|p| input.mode == 2 && p.level == input.visible_level && p.room < 65531)
        {
            let room = input
                .rooms
                .get(&p.room)
                .ok_or("native selected room absent")?;
            merge(&mut next, room)?;
            if room.outdoors
                && p.tile[..2]
                    .iter()
                    .all(|&v| i32::from(v) > 0 && i32::from(v) < input.size - 1)
            {
                let start = rotate(p.tile, input.rotation);
                let inside = |x: i8, y: i8| {
                    i32::from(x) >= input.bounds[0]
                        && i32::from(x) <= input.bounds[1]
                        && i32::from(y) >= input.bounds[2]
                        && i32::from(y) <= input.bounds[3]
                };
                let mut end = start;
                for distance in (0..=4).rev() {
                    end = [
                        start[0].wrapping_add(distance),
                        start[1].wrapping_add(distance),
                        start[2],
                    ];
                    if inside(end[0], end[1]) {
                        break;
                    }
                }
                //461857/461879/461808 enumerate a half-open RECTANGLE,
                // not a diagonal line. The far corner is selected diagonally.
                for y in start[1]..end[1] {
                    for x in start[0]..end[0] {
                        if inside(x, y) {
                            next.rows[y as usize] |= 1u64 << x;
                        }
                    }
                }
            }
        }
        self.mask = next;
        Ok(Some(Pending {
            old,
            size: input.size,
            bounds: input.bounds,
            include_visible: input.include_visible,
        }))
    }
}
