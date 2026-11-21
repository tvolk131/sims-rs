//! The house view's sprite pick (0x4aa713): which object a point on the
//! screen lands on, from the tile under the point and each object's
//! screen rectangle.

use crate::boot::Lot;
use crate::placement::{Cursor, off_lot};

/// How many steps the walk takes from its first tile, at most: it looks
/// at 21 tiles (0x4aa91d).
const STEPS: usize = 20;

/// Object data word 40 (+0x9a), the object's flags: an object with either
/// of these bits set is never the fallback (0x4aa8da).
const FLAGS: usize = 40;
const NOT_FALLBACK: u16 = 0xc000;

/// Definition word 91 (+0xb6): with bit 0 set, the object may be the
/// fallback in live mode (0x4aa8ec), as it may be listed with the other
/// objects on its tile (`crate::orders`).
const MENU_WORD: usize = 91;

/// Global 19, the simulator's mode: in any mode but live mode (0), an
/// object without the flags is a fallback whatever its definition says
/// (0x4aa8e1).
const MODE: usize = 19;

/// A lot position's span, in sixteenths of a tile, that the view turns
/// positions within: 0x47bc1f is passed 0x400 and reflects about one
/// less.
const SPAN: i32 = 64 * 16;

impl Lot {
    /// The object a point on the screen lands on, by the house view's
    /// sprite pick as the pointer's move calls it (0x4aa713 in mode 1,
    /// from 0x476af0).
    ///
    /// 1. The tile the walk starts on is the one under the point (the
    ///    view's point to tile, 0x4a97c3). `depth` is where the depth
    ///    buffer puts the pixel: a lot position (sixteenths of a tile, the
    ///    lot's own axes) and the level its height is on, None where
    ///    nothing was drawn. It is taken when its level is `level` and it
    ///    is on the lot's inner tiles (0x422632); otherwise `ground`, the
    ///    lot position under the point on `level`'s floor, is.
    /// 2. The view turns the position by `rotation`, the view's rotation
    ///    the frame was drawn with (0x47bc1f), and the walk starts on its
    ///    tile. Outside the lot's inner tiles it finds nothing.
    /// 3. Each tile, turned back to the lot's (0x434f58), gives its
    ///    objects on `level` in the order the lot links them (0x43f331,
    ///    0x43f379). An object whose screen rectangle (`rect`: left, top,
    ///    right and bottom, the right and bottom edges outside) holds
    ///    `point` is found; the last found on a tile wins, and the walk
    ///    ends with that tile.
    /// 4. On the first tile only, an object whose rectangle does not hold
    ///    the point is the fallback (the last such) when its data word 40
    ///    has neither bit 0xc000, and either the mode (global 19) is not
    ///    live mode or its definition word 91 has bit 0.
    /// 5. Otherwise the walk steps toward the viewer, by turns one column
    ///    on, one column back and one row on, and one column on, for at
    ///    most 20 steps and while it stays on the lot's inner tiles.
    /// 6. The answer is the found object, else the fallback.
    ///
    /// The view's tile bounds (the scene's +0x70 to +0x7c, tested on the
    /// view's tiles) are taken as the lot's inner tiles turned into the
    /// view, which is **unverified**.
    pub fn sprite_pick(
        &self,
        depth: Option<([i32; 2], i32)>,
        ground: [i32; 2],
        level: i8,
        rotation: u8,
        point: [i32; 2],
        rect: impl Fn(u16) -> Option<[i32; 4]>,
    ) -> Option<u16> {
        let world = self.world();
        let start = match depth {
            Some((place, on)) if on == i32::from(level) && !off_lot(world, place) => place,
            _ => ground,
        };
        let size = i32::from(world.size);
        let inside = |tile: [i32; 2]| {
            turn_tile_back(rotation, tile)
                .iter()
                .all(|&n| (1..size - 1).contains(&n))
        };
        let mut tile = turn_fine(rotation, start).map(|n| n >> 4);
        if !inside(tile) {
            return None;
        }
        let live = world.globals.get(MODE).copied() == Some(0);
        let mut fallback = None;
        for step in 0..=STEPS {
            let [column, row] = turn_tile_back(rotation, tile);
            let mut found = None;
            let mut cursor = Cursor::on_tile(world, [column as i8, row as i8, level]);
            while let Some(id) = cursor.current() {
                let holds = rect(id).is_some_and(|[left, top, right, bottom]| {
                    (left..right).contains(&point[0]) && (top..bottom).contains(&point[1])
                });
                if holds {
                    found = Some(id);
                } else if step == 0 && self.may_fall_back(id, live) {
                    fallback = Some(id);
                }
                if cursor.advance(world).is_err() {
                    break;
                }
            }
            if found.is_some() {
                return found;
            }
            if step == STEPS {
                break;
            }
            match step % 3 {
                1 => tile = [tile[0] - 1, tile[1] + 1],
                _ => tile[0] += 1,
            }
            if !inside(tile) {
                break;
            }
        }
        fallback
    }

    /// Whether object `id` may be the sprite pick's fallback.
    fn may_fall_back(&self, id: u16, live: bool) -> bool {
        let world = self.world();
        let Some(object) = world.objects.get(&id) else {
            return false;
        };
        let flags = object.data.get(FLAGS).copied().unwrap_or(0) as u16;
        if flags & NOT_FALLBACK != 0 {
            return false;
        }
        if !live {
            return true;
        }
        crate::interpreter::object_definition(self.catalog(), world, object.guid)
            .and_then(|d| self.program().definition(d.resource, d.chunk))
            .is_some_and(|words| words[MENU_WORD] & 1 != 0)
    }
}

/// A lot position turned into the view by `rotation` (0x47bc1f).
fn turn_fine(rotation: u8, [x, y]: [i32; 2]) -> [i32; 2] {
    let m = SPAN - 1;
    match rotation & 3 {
        1 => [m - y, x],
        2 => [m - x, m - y],
        3 => [y, m - x],
        _ => [x, y],
    }
}

/// A view tile turned back to the lot's (0x434f58 by `rotation`
/// negated).
fn turn_tile_back(rotation: u8, [x, y]: [i32; 2]) -> [i32; 2] {
    let m = SPAN / 16 - 1;
    match rotation & 3 {
        1 => [y, m - x],
        2 => [m - x, m - y],
        3 => [m - y, x],
        _ => [x, y],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turned_tile_turns_back() {
        for rotation in 0..4 {
            for fine in [[0, 0], [328, 496], [1023, 17], [512, 1000]] {
                let tile = turn_fine(rotation, fine).map(|n| n >> 4);
                assert_eq!(
                    turn_tile_back(rotation, tile),
                    fine.map(|n| n >> 4),
                    "{rotation} {fine:?}"
                );
            }
        }
        // The view reflects fine positions about 1023 (0x47bc1f).
        assert_eq!(turn_fine(1, [328, 496]), [527, 328]);
        assert_eq!(turn_fine(1, [328, 496]).map(|n| n >> 4), [32, 20]);
    }
}
