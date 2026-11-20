//! The house view's sprite pick (0x4aa713): which object a point on the
//! screen lands on, from the tiles behind the point and each object's
//! screen rectangle.

use crate::boot::Lot;
use crate::placement::Cursor;

/// How many tiles the walk looks at, at most (0x4aa91d).
const STEPS: usize = 20;

/// Object data word 40 (+0x9a), the object's flags: an object with either
/// of these bits set is not the fallback (0x4aa8da).
const FLAGS: usize = 40;
const NOT_FALLBACK: u16 = 0xc000;

/// Definition word 91 (+0xb6): bit 0 lets the object be the fallback in
/// live mode (0x4aa8ec), as it lets the object be listed with the tile's
/// others (`crate::orders`).
const MENU_WORD: usize = 91;

/// Global 19, the simulator's mode: in any mode but live mode (0), every
/// object without the flags is a fallback (0x4aa8e1).
const MODE: usize = 19;

impl Lot {
    /// The object a point on the screen lands on, by the house view's
    /// sprite pick in its mode 1 (0x4aa713 as 0x476af0 calls it):
    ///
    /// 1. The walk starts at view tile `start` (column and row as the
    ///    view shows them) on `level`; outside the lot's size, inside its
    ///    edge row, it finds nothing.
    /// 2. Each tile, turned back by the view's rotation (0x434f58), gives
    ///    its objects in the order the lot links them (0x43f331,
    ///    0x43f379). An object whose screen rectangle (`rect`: left, top,
    ///    right, bottom) holds `point`, left and top edges in, is found;
    ///    the last found on a tile wins, and the walk ends after it.
    /// 3. On the first tile only, an object whose rectangle does not hold
    ///    the point is the fallback (the last such) when its data word 40
    ///    has neither bit 0xc000, and either the mode (global 19) is not
    ///    live mode or its definition word 91 has bit 0.
    /// 4. Otherwise the walk steps toward the viewer, by turns one column
    ///    on, one column back and one row on, then one column on again,
    ///    for at most 20 tiles and while it stays inside the lot's edge.
    /// 5. The answer is the found object, else the fallback.
    ///
    /// The lot's size stands for the view's tile bounds (the scene's
    /// +0x70 to +0x7c), which is **unverified**.
    pub fn sprite_pick(
        &self,
        start: [i32; 2],
        level: i8,
        point: [i32; 2],
        rect: impl Fn(u16) -> Option<[i32; 4]>,
    ) -> Option<u16> {
        let world = self.world();
        let size = i32::from(world.size);
        let inside = |t: [i32; 2]| t.iter().all(|&n| (1..size - 1).contains(&n));
        let mut tile = start;
        if !inside(tile) {
            return None;
        }
        let live = world.globals.get(MODE).copied() == Some(0);
        let mut fallback = None;
        for step in 0..STEPS {
            let [column, row] = self.unrotate_tile(tile);
            let mut found = None;
            let mut cursor = Cursor::on_tile(world, [column as i8, row as i8, level]);
            while let Some(id) = cursor.current() {
                let holds = rect(id).is_some_and(|[left, top, right, bottom]| {
                    (left..right).contains(&point[0]) && (top..bottom).contains(&point[1])
                });
                if std::env::var_os("SPIKE_DEBUG").is_some() {
                    eprintln!("  walk {step} tile {tile:?} obj {id} rect {:?} holds {holds} fb {}", rect(id), self.may_fall_back(id, live));
                }
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

    /// A view tile turned back to the lot's (0x434f58 with the view's
    /// rotation negated).
    fn unrotate_tile(&self, [x, y]: [i32; 2]) -> [i32; 2] {
        let m = 63;
        match self.view_rotation() & 3 {
            1 => [y, m - x],
            2 => [m - x, m - y],
            3 => [m - y, x],
            _ => [x, y],
        }
    }
}
