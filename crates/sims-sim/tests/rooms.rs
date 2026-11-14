//! The lot's rooms on made-up lots: the partition, the tile flags, the
//! room of a position, and the room count. No game data.

use sims_formats::lot::Array;
use sims_sim::World;
use sims_sim::rooms::{Half, NO_ROOM, RoomError, UNSEEN};

const WEST: u8 = 1;
const NORTH: u8 = 2;
const DESCENDING: u8 = 0x10;
const ASCENDING: u8 = 0x20;

/// A wall style that closes a room and can take a covering.
const PLAIN: u16 = 1;

fn array(stride: usize, fill: u8) -> Array {
    Array {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![fill; 64 * 64 * stride],
    }
}

/// A flat, empty lot of `size` tiles: no walls, no floors, no pool.
fn lot(size: i16) -> World {
    let mut world = World::empty();
    world.size = size;
    world.arrays.insert(0, array(4, 20));
    world.arrays.insert(9, array(1, 255));
    for (floors, walls, flags) in [(1, 2, 8), (101, 102, 108)] {
        world.arrays.insert(floors, array(2, 0));
        world.arrays.insert(walls, array(14, 0));
        world.arrays.insert(flags, array(1, 0));
    }
    world
}

fn cell(world: &mut World, array: u16, x: usize, y: usize) -> &mut [u8] {
    let array = world.arrays.get_mut(&array).unwrap();
    let stride = array.stride;
    &mut array.data[(x * 64 + y) * stride..][..stride]
}

/// Sets one of a wall cell's six words: two styles, then four patterns.
fn word(world: &mut World, walls: u16, x: usize, y: usize, word: usize, value: u16) {
    cell(world, walls, x, y)[2 + word * 2..][..2].copy_from_slice(&value.to_le_bytes());
}

/// Puts a plain wall on the west or north edge of a tile, and mirrors it
/// in the neighbour as the original does.
fn wall(world: &mut World, walls: u16, x: usize, y: usize, bit: u8) {
    cell(world, walls, x, y)[0] |= bit;
    if bit == WEST {
        cell(world, walls, x - 1, y)[0] |= 4;
        word(world, walls, x, y, 0, PLAIN);
    } else {
        cell(world, walls, x, y - 1)[0] |= 8;
        word(world, walls, x, y, 1, PLAIN);
    }
}

/// Walls in the tiles 2 and 3 in both directions.
fn boxed(world: &mut World, walls: u16) {
    for n in 2..4 {
        wall(world, walls, 2, n, WEST);
        wall(world, walls, 4, n, WEST);
        wall(world, walls, n, 2, NORTH);
        wall(world, walls, n, 4, NORTH);
    }
}

fn room(world: &World, level: usize, x: usize, y: usize) -> i16 {
    world.rooms.map[level - 1][x * 64 + y]
}

fn flags(world: &World, level: usize, x: usize, y: usize) -> u8 {
    world.arrays[&[8, 108][level - 1]].data[x * 64 + y]
}

fn tiles(world: &World, id: i16) -> Vec<[u8; 2]> {
    world.rooms.rooms[&id]
        .tiles
        .iter()
        .map(|tile| [tile[0], tile[1]])
        .collect()
}

#[test]
fn an_empty_lot_is_one_outdoor_room_on_each_level() {
    let mut world = lot(2);
    // A flag byte outside the lot's size keeps only bits 8 and 0x20.
    cell(&mut world, 8, 40, 40)[0] = 0xff;
    world.rebuild_rooms().unwrap();
    assert_eq!(
        world.rooms.rooms.keys().copied().collect::<Vec<_>>(),
        [0, 1024]
    );
    // The tiles in the order the room reached them. A tile a neighbour
    // reaches again while it waits is listed again.
    let listed = [[0, 0], [0, 1], [1, 0], [1, 1], [0, 0], [0, 1], [1, 1]];
    assert_eq!(tiles(&world, 0), listed);
    assert_eq!(tiles(&world, 1024), listed);
    assert_eq!(world.rooms.rooms[&0].tiles[0], [0, 0, 1]);
    assert_eq!(world.rooms.rooms[&1024].tiles[0], [0, 0, 2]);
    assert!(world.rooms.halves.is_empty());
    for level in 1..=2 {
        assert_eq!(room(&world, level, 1, 1), (level as i16 - 1) * 1024);
        // Outside the size no tile has a room.
        assert_eq!(room(&world, level, 2, 0), UNSEEN);
        assert_eq!(room(&world, level, 63, 63), UNSEEN);
        // Reached from the corner and outside: bits 2 and 4. A lot of two
        // tiles is all border, so nothing can be built on.
        assert_eq!(flags(&world, level, 1, 0), 6);
        assert_eq!(flags(&world, level, 2, 0), 0);
    }
    assert_eq!(flags(&world, 1, 40, 40), 0x28);
    let room = &world.rooms.rooms[&0];
    assert!(room.outdoors && room.dirty);
    assert_eq!(room.half_tiles, 0);
    // Counting: two half tiles for each listed tile, and no indoor area.
    world.globals = vec![7; 64];
    assert_eq!(world.update_rooms(), Ok(vec![0, 1024]));
    let room = &world.rooms.rooms[&0];
    assert!(!room.dirty);
    assert_eq!(
        (room.half_tiles, room.covered_half_tiles, room.walls),
        (14, 0, 0)
    );
    assert_eq!(world.globals[27], 0);
    // A counted room is not counted again.
    assert_eq!(world.update_rooms(), Ok(vec![]));
}

#[test]
fn walls_close_off_an_indoor_room() {
    let mut world = lot(6);
    boxed(&mut world, 2);
    // One tile of the room has a floor and one wall has a covering.
    cell(&mut world, 1, 2, 2)[0] = 9;
    word(&mut world, 2, 2, 2, 2, 5);
    world.rebuild_rooms().unwrap();
    assert_eq!(
        world.rooms.rooms.keys().copied().collect::<Vec<_>>(),
        [0, 1, 1024]
    );
    for (x, y) in [(2, 2), (2, 3), (3, 2), (3, 3)] {
        assert_eq!(room(&world, 1, x, y), 1);
        // Enclosed, and level ground inside the border: buildable only.
        assert_eq!(flags(&world, 1, x, y), 1);
        // The tile above is supported by the room below.
        assert_eq!(flags(&world, 2, x, y), 7);
    }
    assert_eq!(room(&world, 1, 1, 2), 0);
    assert_eq!(room(&world, 1, 4, 3), 0);
    assert_eq!(flags(&world, 1, 1, 2), 7);
    assert_eq!(flags(&world, 1, 0, 2), 6);
    // Upstairs nothing is enclosed, and a tile over open ground with no
    // floor beside it cannot be built on.
    assert_eq!(room(&world, 2, 2, 2), 1024);
    assert_eq!(flags(&world, 2, 1, 2), 6);
    assert_eq!(
        tiles(&world, 1),
        [[2, 2], [2, 3], [3, 2], [3, 3], [2, 2], [2, 3], [3, 3]]
    );
    assert!(!world.rooms.rooms[&1].outdoors);
    assert!(world.rooms.rooms[&0].outdoors);
    world.globals = vec![0; 64];
    world.update_rooms().unwrap();
    let room = &world.rooms.rooms[&1];
    // Seven listed tiles. The floored tile and the covered wall are each
    // listed twice, and so are three of the four corners' two walls.
    assert_eq!((room.half_tiles, room.covered_half_tiles), (14, 4));
    assert_eq!((room.walls, room.covered_walls), (14, 2));
    assert_eq!(world.globals[27], 7);
}

#[test]
fn a_fence_divides_rooms_but_does_not_enclose() {
    let mut world = lot(6);
    boxed(&mut world, 2);
    for n in 2..4 {
        // Style 2 on every wall of the box.
        word(&mut world, 2, 2, n, 0, 2);
        word(&mut world, 2, 4, n, 0, 2);
        word(&mut world, 2, n, 2, 1, 2);
        word(&mut world, 2, n, 4, 1, 2);
    }
    world.rebuild_rooms().unwrap();
    // The partition looks at the wall bits only.
    assert_eq!(room(&world, 1, 2, 2), 1);
    // The flood for the outside crosses the fence.
    assert_eq!(flags(&world, 1, 2, 2), 7);
    assert!(world.rooms.rooms[&1].outdoors);
    world.globals = vec![0; 64];
    world.update_rooms().unwrap();
    let room = &world.rooms.rooms[&1];
    assert_eq!((room.half_tiles, room.walls), (14, 0));
    assert_eq!(world.globals[27], 0);
}

#[test]
fn a_diagonal_wall_splits_its_tile_between_two_rooms() {
    let mut world = lot(6);
    boxed(&mut world, 2);
    // A diagonal in the box's far corner cuts off the triangle behind it.
    cell(&mut world, 2, 3, 3)[0] |= DESCENDING;
    word(&mut world, 2, 3, 3, 1, PLAIN);
    world.rebuild_rooms().unwrap();
    assert_eq!(
        world.rooms.rooms.keys().copied().collect::<Vec<_>>(),
        [0, 1, 2, 1024]
    );
    assert_eq!(room(&world, 1, 3, 3), NO_ROOM);
    assert_eq!(
        world.rooms.halves[&[1, 3, 3]],
        [
            Half {
                triangle: 4,
                room: 1
            },
            Half {
                triangle: 2,
                room: 2
            }
        ]
    );
    assert_eq!(
        tiles(&world, 1),
        [[2, 2], [2, 3], [3, 2], [3, 3], [2, 2], [2, 3]]
    );
    assert_eq!(tiles(&world, 2), [[3, 3]]);
    // A room of triangles only is outdoors if it lists exactly one tile.
    assert!(world.rooms.rooms[&2].outdoors);
    assert!(!world.rooms.rooms[&1].outdoors);
    // The flood for the outside does not enter the tile.
    assert_eq!(flags(&world, 1, 3, 3), 1);
    world.globals = vec![0; 64];
    world.update_rooms().unwrap();
    // Indoors the split tile counts one half tile and its diagonal wall.
    let room = &world.rooms.rooms[&1];
    assert_eq!((room.half_tiles, room.walls), (11, 11));
    assert_eq!(world.rooms.rooms[&2].half_tiles, 2);
    assert_eq!(world.globals[27], 5);
    // An object on the split tile takes the room of the tile beside it on
    // its side of the diagonal, not the room of its triangle.
    assert_eq!(world.room_at([3 * 16 + 2, 3 * 16 + 2, 1]), Ok(1));
    assert_eq!(world.room_at([3 * 16 + 14, 3 * 16 + 14, 1]), Ok(0));
    // The half floor of a split tile is the wall cell's first style word.
    word(&mut world, 2, 3, 3, 0, 4);
    world.rebuild_rooms().unwrap();
    world.update_rooms().unwrap();
    assert_eq!(world.rooms.rooms[&1].covered_half_tiles, 1);
}

#[test]
fn a_diagonal_wall_in_the_open_splits_nothing() {
    let mut world = lot(6);
    cell(&mut world, 2, 2, 2)[0] |= ASCENDING;
    word(&mut world, 2, 2, 2, 1, PLAIN);
    world.rebuild_rooms().unwrap();
    // Both triangles end up in the outdoor room.
    assert_eq!(room(&world, 1, 2, 2), 0);
    assert!(world.rooms.halves.is_empty());
    // The flood for the outside still does not enter the tile.
    assert_eq!(flags(&world, 1, 2, 2), 1);
    assert_eq!(flags(&world, 1, 2, 3), 7);
}

#[test]
fn upstairs_a_floor_and_a_hole_are_different_rooms() {
    let mut world = lot(6);
    cell(&mut world, 101, 2, 2)[0] = 1;
    cell(&mut world, 101, 2, 3)[0] = 1;
    // On the ground a floor changes nothing.
    cell(&mut world, 1, 2, 2)[0] = 1;
    world.rebuild_rooms().unwrap();
    assert_eq!(
        world.rooms.rooms.keys().copied().collect::<Vec<_>>(),
        [0, 1024, 1025]
    );
    assert_eq!(room(&world, 2, 2, 2), 1025);
    assert_eq!(room(&world, 2, 2, 3), 1025);
    assert_eq!(room(&world, 2, 3, 3), 1024);
    assert_eq!(tiles(&world, 1025), [[2, 2], [2, 3], [2, 2]]);
    assert_eq!(room(&world, 1, 2, 2), 0);
}

#[test]
fn the_edge_of_a_pool_divides_the_ground() {
    let mut world = lot(6);
    cell(&mut world, 9, 2, 2)[0] = 3;
    world.rebuild_rooms().unwrap();
    assert_eq!(room(&world, 1, 2, 2), 1);
    assert_eq!(room(&world, 1, 2, 3), 0);
    assert_eq!(tiles(&world, 1), [[2, 2]]);
    // Not upstairs.
    assert_eq!(room(&world, 2, 2, 2), 1024);
}

#[test]
fn sloped_ground_cannot_be_built_on() {
    let mut world = lot(6);
    cell(&mut world, 0, 2, 2)[3] = 21;
    world.rebuild_rooms().unwrap();
    assert_eq!(flags(&world, 1, 2, 2), 6);
    assert_eq!(flags(&world, 1, 2, 3), 7);
}

#[test]
fn an_objects_room_is_its_tiles_unless_it_is_on_the_border() {
    let mut world = lot(6);
    assert_eq!(world.room_at([40, 40, 1]), Err(RoomError::NoRooms));
    boxed(&mut world, 2);
    world.rebuild_rooms().unwrap();
    assert_eq!(world.room_at([2 * 16, 3 * 16 + 15, 1]), Ok(1));
    assert_eq!(world.room_at([2 * 16, 3 * 16 + 15, 2]), Ok(1024));
    assert_eq!(world.room_at([16, 4 * 16, 1]), Ok(0));
    // The outermost ring of tiles, and everything off the lot, is no room.
    for position in [[0, 40, 1], [40, 5 * 16, 1], [-16, -16, 1], [6 * 16, 40, 1]] {
        assert_eq!(world.room_at(position), Ok(NO_ROOM), "{position:?}");
    }
    assert_eq!(world.room_at([40, 40, 0]), Err(RoomError::Level(0)));
    assert_eq!(world.room_at([40, 40, 3]), Err(RoomError::Level(3)));
}

#[test]
fn lots_the_port_does_not_cover_are_refused_and_left_alone() {
    let fresh = lot(6);
    let refused = |change: &dyn Fn(&mut World)| {
        let mut world = fresh.clone();
        change(&mut world);
        let before = world.encode();
        let error = world.rebuild_rooms().unwrap_err();
        assert_eq!(world.encode(), before);
        error
    };
    assert_eq!(
        refused(&|world| cell(world, 102, 2, 2)[0] = DESCENDING | ASCENDING),
        RoomError::CrossingDiagonals {
            level: 2,
            x: 2,
            y: 2
        }
    );
    // A west wall the tile to the west does not know about.
    assert_eq!(
        refused(&|world| cell(world, 2, 2, 2)[0] = WEST),
        RoomError::Asymmetric {
            level: 1,
            x: 2,
            y: 2
        }
    );
    assert_eq!(
        refused(&|world| cell(world, 2, 2, 2)[0] = 0x40),
        RoomError::Segments {
            level: 1,
            x: 2,
            y: 2
        }
    );
    assert_eq!(
        refused(&|world| {
            cell(world, 2, 0, 3)[0] = DESCENDING;
            word(world, 2, 0, 3, 1, PLAIN);
        }),
        RoomError::BorderDiagonal {
            level: 1,
            x: 0,
            y: 3
        }
    );
    assert_eq!(
        refused(&|world| {
            world.arrays.remove(&102);
        }),
        RoomError::MissingArray(102)
    );
    assert_eq!(
        refused(&|world| world.arrays.get_mut(&8).unwrap().stride = 2),
        RoomError::ArrayShape(8)
    );
    assert_eq!(refused(&|world| world.size = 1), RoomError::Size(1));
    // Walls outside the lot's size are not looked at.
    let mut world = fresh.clone();
    cell(&mut world, 2, 20, 20)[0] = 0xff;
    assert_eq!(world.rebuild_rooms(), Ok(()));
    // Counting needs the globals.
    world.globals = vec![0; 32];
    assert_eq!(world.update_rooms(), Err(RoomError::NoContext));
}

#[test]
fn a_lot_without_a_pool_array_has_no_pool_edges() {
    let mut world = lot(4);
    world.arrays.remove(&9);
    world.rebuild_rooms().unwrap();
    assert_eq!(
        world.rooms.rooms.keys().copied().collect::<Vec<_>>(),
        [0, 1024]
    );
}
