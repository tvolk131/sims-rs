//! Recovered spatial contracts. Native coordinates put north on negative X.
use serde::{Deserialize, Serialize};

/// Original 0x433d16 / 0x43519f, before placement-specific collision exceptions.
/// Bounds are [max X, max Y, min X, min Y], including degenerate rectangles.
pub fn footprint(
    position: [i32; 2],
    person: bool,
    zero_size: bool,
    size: i16,
    mask: u16,
    extension: u16,
    facing: u8,
) -> [i32; 4] {
    let width = if person {
        if zero_size { 0 } else { 6 }
    } else if size == 0 {
        16
    } else {
        i32::from(size)
    };
    let half = width / 2;
    let [x, y] = position;
    let mut bounds = [x + half, y + half, x - half, y - half];
    let extension = if person && facing & 1 == 0 {
        extension
    } else {
        0
    };
    let inset: [i32; 4] = std::array::from_fn(|i| {
        i32::from((mask >> (i * 4)) & 15) - i32::from((extension >> (i * 4)) & 15)
    });
    let i = usize::from((8 - facing) / 2) % 4;
    bounds[2] += inset[i];
    bounds[1] -= inset[(i + 1) % 4];
    bounds[0] -= inset[(i + 2) % 4];
    bounds[3] += inset[(i + 3) % 4];
    bounds
}

/// Original 0x4359ea: architectural wall bits to compass mask, then relative rotation.
pub fn wall_mask(tile_bits: u8, object_mask: u8, rotation: u8) -> u8 {
    let mut mask = object_mask;
    for (bit, direction) in [(1, 64), (2, 1), (4, 4), (8, 16)] {
        if tile_bits & bit != 0 {
            mask |= direction;
        }
    }
    mask.rotate_right(u32::from(rotation))
}

/// Original 0x4359ce. Preserve the signed word and x86 shift-count semantics;
/// the native fold only considers the first two bytes of the shifted value.
pub fn object_wall_mask(mask: i16, facing: i16) -> u8 {
    let shifted = (i32::from(mask) as u32).wrapping_shl(u32::from(facing as u16 & 31));
    (shifted | shifted >> 8) as u8
}

/// Original 0x46876f cache fill. Room membership and tile flags come from architecture.
pub fn room_outdoors(room: i16, tiles: &[(i16, u8)]) -> bool {
    tiles
        .iter()
        .find(|(id, _)| *id == room)
        .map_or(tiles.len() == 1, |(_, flags)| flags & 4 != 0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlacementResult {
    pub allowed: bool,
    pub error: i32,
    pub blocker: Option<(i16, i16)>,
}
/// Original 0x481439. Placement itself remains a separate query; a person
/// obstruction is classification 5, which native route/exit selection may admit.
pub fn classify(
    outside: bool,
    flags: u32,
    same_room: bool,
    direction: i32,
    relative_walls: u8,
    height: u32,
    target_height: u32,
    placement: PlacementResult,
    ignored: Option<i16>,
) -> (u8, Option<i16>) {
    if outside {
        return (0, None);
    }
    if flags & 0x800 == 0 {
        if !same_room {
            return (2, None);
        }
        if direction != -1
            && (relative_walls & 1 != 0 || direction & 1 != 0 && relative_walls & 0x82 != 0)
        {
            return (3, None);
        }
    }
    if flags & 0x8000 == 0
        && (height != target_height || height.to_le_bytes().iter().any(|b| *b != height as u8))
    {
        return (4, None);
    }
    if !placement.allowed {
        if placement.error == 7 {
            return (3, None);
        }
        if placement.error != 11 {
            return (0, None);
        }
        if let Some((id, kind)) = placement.blocker.filter(|(id, _)| Some(*id) != ignored) {
            return (if kind == 2 { 5 } else { 1 }, Some(id));
        }
    }
    (6, None)
}
