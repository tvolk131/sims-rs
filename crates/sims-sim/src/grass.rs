//! The lawn's growth: the grass array (Arry 6) grows toward the target
//! grass array (Arry 7).
//!
//! The architecture loader keeps the two arrays (0x478599: Arry 6 at
//! architecture +0x1c, Arry 7 at +0x20). When the time of day changes, the
//! simulator sends signal 0xf8, and the architecture's handler (0x47b22a,
//! case 0xf8 at 0x47b2e0) grows the grass by 5 when global 36 is not 0
//! (0x47b3c8). That sweep:
//!
//! 1. The step is the argument times 4: 20.
//! 2. For y from 1 to size − 2, and in it x from 1 to size − 2, where size
//!    is the lot's size (architecture +0x10, which the lot's resize
//!    0x415e36 sets): the grass g and target t of tile (x, y), both signed
//!    bytes.
//! 3. When g < t and any diagonal neighbour has grass equal to its target
//!    (0x47b4fe, directions 4 to 7 of the table at 0x6e486c: (−1, −1),
//!    (1, 1), (1, −1), (−1, 1)), the tile's grass becomes g + 20, or t if
//!    that is more. The array is changed in place, so a tile grown earlier
//!    in the sweep counts for the tiles after it.
//! 4. It then sends signal 0x83, and the handler sends 0x86 and 0x83 after
//!    it, which the interface's panels (0x4c3ea5, a refresh flag on 0x86)
//!    and the house viewer (0x4a82db, a redraw on 0x83) take.

use crate::World;

/// The grass array and the target grass array.
pub const GRASS: u16 = 6;
pub const TARGET_GRASS: u16 = 7;

/// The diagonal directions the sweep checks (the table at 0x6e486c,
/// entries 4 to 7).
const DIAGONALS: [(isize, isize); 4] = [(-1, -1), (1, 1), (1, -1), (-1, 1)];

impl World {
    /// Grows the lawn by `amount` (0x47b3c8; see the module's
    /// documentation): each tile inside the lot's border whose grass is
    /// below its target and that has a diagonal neighbour at its target
    /// grows by 4 × `amount`, up to the target.
    ///
    /// The original's lot always has both arrays, as wide as the lot. A
    /// lot here without them, or with arrays that do not hold the lot's
    /// size in one-byte cells, is an error, and the lot is unchanged.
    pub fn grow_grass(&mut self, amount: i32) -> Result<(), String> {
        let size = usize::try_from(self.size).unwrap_or(0);
        let fits = |id| match self.arrays.get(&id) {
            Some(array) => {
                array.stride == 1
                    && array.width >= size
                    && array.height >= size
                    && array.data.len() == array.width * array.height
            }
            None => false,
        };
        if !fits(GRASS) || !fits(TARGET_GRASS) {
            return Err(format!(
                "the lawn grows (0x47b3c8), and the lot has no grass and target grass arrays \
                 (Arry {GRASS} and {TARGET_GRASS}) of its size, {size}"
            ));
        }
        let target = self.arrays[&TARGET_GRASS].clone();
        let grass = self.arrays.get_mut(&GRASS).expect("checked above");
        let height = grass.height;
        let target_height = target.height;
        let at = |data: &[u8], height: usize, x: usize, y: usize| data[x * height + y] as i8;
        let step = amount.wrapping_mul(4);
        for y in 1..size.saturating_sub(1) {
            for x in 1..size.saturating_sub(1) {
                let g = at(&grass.data, height, x, y);
                let t = at(&target.data, target_height, x, y);
                if g >= t {
                    continue;
                }
                let grown = DIAGONALS.iter().any(|&(dx, dy)| {
                    let (nx, ny) = (x.wrapping_add_signed(dx), y.wrapping_add_signed(dy));
                    at(&grass.data, height, nx, ny) == at(&target.data, target_height, nx, ny)
                });
                if grown {
                    // The original adds in 32 bits, which wrap.
                    let next = i32::from(g).wrapping_add(step).min(i32::from(t));
                    grass.data[x * height + y] = next as i8 as u8;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sims_formats::lot::Array;

    fn array(size: usize, cells: &[((usize, usize), i8)], fill: i8) -> Array {
        let mut data = vec![fill as u8; size * size];
        for &((x, y), value) in cells {
            data[x * size + y] = value as u8;
        }
        Array {
            width: size,
            height: size,
            stride: 1,
            encoding: 0,
            data,
        }
    }

    fn lot(size: usize, grass: Array, target: Array) -> World {
        let mut world = World::empty();
        world.size = size as i16;
        world.arrays.insert(GRASS, grass);
        world.arrays.insert(TARGET_GRASS, target);
        world
    }

    fn cell(world: &World, x: usize, y: usize) -> i8 {
        let array = &world.arrays[&GRASS];
        array.data[x * array.height + y] as i8
    }

    #[test]
    fn a_tile_grows_from_a_diagonal_neighbour_at_its_target_up_to_its_own() {
        // Target 100 everywhere; grass 0 except a full tile at (1, 1).
        let mut world = lot(6, array(6, &[((1, 1), 100)], 0), array(6, &[], 100));
        world.grow_grass(5).unwrap();
        // (2, 2) is the full tile's diagonal neighbour and grows by 20.
        assert_eq!(cell(&world, 2, 2), 20);
        // (2, 1) is only beside it, not diagonal, and does not grow.
        assert_eq!(cell(&world, 2, 1), 0);
        // (3, 3) neighbours (2, 2) only after it grew, and 20 is not its
        // target, so it does not grow.
        assert_eq!(cell(&world, 3, 3), 0);

        // Near the target a tile stops at it.
        let mut world = lot(
            6,
            array(6, &[((1, 1), 50), ((2, 2), 40)], 0),
            array(6, &[((1, 1), 50), ((2, 2), 50)], 100),
        );
        world.grow_grass(5).unwrap();
        assert_eq!(cell(&world, 2, 2), 50);
    }

    #[test]
    fn the_sweep_runs_in_place_column_by_column_inside_the_border() {
        // Target 20 everywhere, grass 0 except (1, 1) at its target. The
        // sweep goes along x for each y, so (2, 2) grows on row 2 and
        // reaches its target in time for (3, 3) on row 3, then (4, 4).
        let mut world = lot(6, array(6, &[((1, 1), 20)], 0), array(6, &[], 20));
        world.grow_grass(5).unwrap();
        assert_eq!(cell(&world, 2, 2), 20);
        assert_eq!(cell(&world, 3, 3), 20);
        assert_eq!(cell(&world, 4, 4), 20);
        // (1, 2)'s diagonals are (0, 1), (2, 3), (2, 1) and (0, 3): none
        // is at its target when the sweep reaches it, and (2, 3) grows
        // only later, so it stays.
        assert_eq!(cell(&world, 1, 2), 0);
        // The sweep goes along x within each y. (1, 3)'s diagonal (2, 2)
        // grew on row 2, before it; (3, 1)'s did not grow until after it,
        // on row 2. With the loops the other way round these swap.
        assert_eq!(cell(&world, 1, 3), 20);
        assert_eq!(cell(&world, 3, 1), 0);
        // The border stays as it was.
        assert_eq!(cell(&world, 0, 0), 0);
        assert_eq!(cell(&world, 5, 5), 0);
    }

    #[test]
    fn the_bytes_are_signed_and_the_addition_wraps() {
        // Grass −100 below a target of −50 grows by 20; a grass of 100
        // is above a target of −1 (0xff), so it does not grow.
        let mut world = lot(
            6,
            array(6, &[((1, 1), -50), ((3, 3), 100)], -100),
            array(6, &[((3, 3), -1)], -50),
        );
        world.grow_grass(5).unwrap();
        assert_eq!(cell(&world, 2, 2), -80);
        assert_eq!(cell(&world, 3, 3), 100);
        // An amount whose step wraps past i32::MAX gives the wrapped sum.
        let mut world = lot(
            6,
            array(6, &[((1, 1), 120), ((2, 2), 100)], 0),
            array(6, &[], 120),
        );
        world.grow_grass(536_870_911).unwrap();
        assert_eq!(cell(&world, 2, 2), 96);
    }

    #[test]
    fn the_sweep_stays_inside_the_lots_size_and_needs_both_arrays() {
        // A 64-wide array on a lot of size 4 sweeps only (1..3, 1..3).
        let mut world = lot(4, array(64, &[((1, 1), 20)], 0), array(64, &[], 20));
        world.grow_grass(5).unwrap();
        assert_eq!(cell(&world, 2, 2), 20);
        assert_eq!(cell(&world, 3, 3), 0);

        let mut world = lot(4, array(4, &[], 0), array(4, &[], 20));
        world.arrays.remove(&TARGET_GRASS);
        let before = world.arrays.clone();
        assert!(world.grow_grass(5).is_err());
        assert_eq!(world.arrays, before);
    }
}
