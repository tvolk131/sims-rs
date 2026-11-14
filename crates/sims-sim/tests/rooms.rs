//! The lot's rooms on made-up lots: the partition, the tile flags, the
//! room of a position, and the room update with its light and scores. No
//! game data.

use sims_core::F32;
use sims_formats::lot::Array;
use sims_sim::rooms::{Half, NO_ROOM, Room, RoomError, Standing, Tuning, UNSEEN};
use sims_sim::{Object, World};

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

/// The room update for every room of a lot nothing stands on, with the
/// original's default constants.
fn count(world: &mut World) -> Result<Vec<i16>, RoomError> {
    world.update_rooms(&Tuning::default(), 0, &mut |_, _| Ok(Vec::new()))
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
    assert_eq!(count(&mut world), Ok(vec![0, 1024]));
    let room = &world.rooms.rooms[&0];
    assert_eq!(
        (room.half_tiles, room.covered_half_tiles, room.walls),
        (14, 0, 0)
    );
    assert_eq!(world.globals[27], 0);
    // A new room has no light. The count gave these two the daylight, so
    // both were announced as changed and are marked again.
    assert_eq!(room.light, F32::from_f32(1.0));
    assert!(room.dirty);
    // The second count changes no light and leaves them counted.
    assert_eq!(count(&mut world), Ok(vec![0, 1024]));
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    // A counted room is not counted again.
    assert_eq!(count(&mut world), Ok(vec![]));
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
    count(&mut world).unwrap();
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
    count(&mut world).unwrap();
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
    count(&mut world).unwrap();
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
    count(&mut world).unwrap();
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
    assert_eq!(count(&mut world), Err(RoomError::NoContext));
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

/// A lot of 6 tiles with one enclosed room of four tiles, room 1, counted
/// until nothing waits.
fn counted_box() -> World {
    let mut world = lot(6);
    boxed(&mut world, 2);
    world.rebuild_rooms().unwrap();
    world.globals = vec![0; 64];
    count(&mut world).unwrap();
    count(&mut world).unwrap();
    assert!(world.rooms.rooms.values().all(|room| !room.dirty));
    world
}

#[test]
fn marking_a_room_makes_its_level_wait() {
    let mut world = counted_box();
    assert_eq!(world.rooms.pending, 0);
    world.mark_room(1024);
    assert_eq!(world.rooms.pending, 2);
    assert!(world.rooms.rooms[&1024].dirty && !world.rooms.rooms[&1].dirty);
    world.mark_room(1);
    assert_eq!(world.rooms.pending, 3);
    // A word that names no room marks nothing.
    world.mark_room(77);
    world.mark_room(NO_ROOM);
    assert_eq!(world.rooms.pending, 3);
    // Counting one level takes that level's rooms and its bit only.
    let nothing = &mut |_: &mut World, _| Ok(Vec::new());
    let tuning = Tuning::default();
    assert_eq!(world.update_rooms(&tuning, 2, nothing), Ok(vec![1024]));
    assert_eq!(world.rooms.pending, 1);
    assert!(world.rooms.rooms[&1].dirty);
    // The update of what waits goes by the waiting levels.
    assert_eq!(world.update_pending_rooms(&tuning, nothing), Ok(vec![1]));
    assert_eq!(world.rooms.pending, 0);
    assert_eq!(world.update_pending_rooms(&tuning, nothing), Ok(vec![]));
    // A room that is marked while no level waits is not counted by it.
    world.rooms.rooms.get_mut(&1).unwrap().dirty = true;
    assert_eq!(world.update_pending_rooms(&tuning, nothing), Ok(vec![]));
    // Marking every room makes both levels wait.
    world.mark_all_rooms();
    assert_eq!(world.rooms.pending, 3);
    assert!(world.rooms.rooms.values().all(|room| room.dirty));
    assert_eq!(
        world.update_pending_rooms(&tuning, nothing),
        Ok(vec![0, 1, 1024])
    );
    assert_eq!(
        world.update_rooms(&tuning, 3, nothing),
        Err(RoomError::Level(3))
    );
}

/// An object with nothing set but its ID.
fn thing(id: u16) -> Object {
    Object {
        id,
        guid: 1,
        footprint: [0; 4],
        position: [0; 3],
        unknown_short: 0,
        attributes: Vec::new(),
        temps: vec![0; 8],
        data: vec![0; 72],
        obsolete_word: 0,
        stack_capacity_bytes: 0,
        stack: Vec::new(),
        relationship_flag: -1,
        relationships: Vec::new(),
        slots: Vec::new(),
        dynamic_sprites: Vec::new(),
        multipart: None,
        portal: None,
        person: None,
        registry_word: None,
        last_result: false,
        removal_skips_resets: false,
        selected: false,
    }
}

#[test]
fn the_objects_on_a_rooms_tiles_are_counted() {
    let mut world = counted_box();
    // (data word, value) pairs for each object, and its type.
    type Thing = (u16, i16, &'static [(usize, i16)]);
    let things: [Thing; 9] = [
        // A lamp: a light source with 40 of light and 5 of impact.
        (1, 4, &[(40, 0x400), (51, 40), (7, 5), (39, 3)]),
        // A window: its light is window light, and it is no light.
        (2, 8, &[(40, 0x400), (51, 150)]),
        // A light source that is off the lot adds no light.
        (3, 4, &[(40, 0x400), (51, 99)]),
        // A person. Trash: negative impact, so not a good thing.
        (4, 2, &[]),
        (5, 4, &[(7, -30)]),
        // Categories 4 and 5 to 7.
        (6, 4, &[(59, 4)]),
        (7, 4, &[(59, 6)]),
        // Skipped: data word 34 set, or a negative room.
        (8, 4, &[(34, 1), (7, 100)]),
        (9, 4, &[(29, -5), (7, 100)]),
    ];
    for (id, _, words) in things {
        let mut object = thing(id);
        for &(index, value) in words {
            object.data[index] = value;
        }
        world.objects.insert(id, object);
    }
    // Everything stands on tile 2, 2 of level 1, which the room lists
    // twice, and on tile 0, 0 outdoors.
    let mut standing = |_: &mut World, tile: [u8; 3]| {
        Ok(match tile {
            [2, 2, 1] | [0, 0, 1] => things
                .iter()
                .map(|&(id, kind, _)| Standing {
                    id,
                    kind,
                    on_lot: id != 3,
                })
                .collect(),
            _ => Vec::new(),
        })
    };
    world.mark_all_rooms();
    let tuning = Tuning::default();
    world.update_rooms(&tuning, 0, &mut standing).unwrap();
    let room = &world.rooms.rooms[&1];
    // Seven objects count, twice each.
    assert_eq!(room.objects, 14);
    assert_eq!((room.lights, room.people), (2, 2));
    assert_eq!((room.window_light, room.lamp_light), (300, 80));
    assert_eq!((room.category_4, room.category_5_to_7), (2, 2));
    assert_eq!(room.word_39, 6);
    assert_eq!(room.impact, 2 * (5 - 30));
    assert_eq!(room.good, 12);
    // Light: lamp 80 hundredths over 7 tiles times 30, plus the window
    // light, which is no more than the daylight: 1 by day. Above 0.99 is 1.
    assert_eq!(room.light, F32::from_f32(1.0));
    // Outdoors only what takes away adds to the impact, and the score is
    // the good things plus twice the impact. The light is the daylight.
    // The outdoor room lists the corner tile three times.
    let outside = &world.rooms.rooms[&0];
    let times = outside.tiles.iter().filter(|t| **t == [0, 0, 1]).count();
    assert_eq!(times, 3);
    assert_eq!(
        (outside.objects, outside.impact, outside.good),
        (21, -90, 18)
    );
    assert_eq!(outside.score, F32::from_f32(18.0 - 180.0));
    assert_eq!(outside.light, F32::from_f32(1.0));
    // The lot's outdoor sum adds the upper level's empty room, and the
    // outdoor score is a third of it, within -100 and 100.
    assert_eq!(world.rooms.outdoor_sum, F32::from_f32(-162.0));
    assert_eq!(world.rooms.outdoor_score, F32::from_f32(-54.0));
    // An object the lot lacks stops the update.
    let mut ghost = |_: &mut World, tile: [u8; 3]| {
        Ok(match tile {
            [2, 2, 1] => vec![Standing {
                id: 77,
                kind: 4,
                on_lot: true,
            }],
            _ => Vec::new(),
        })
    };
    world.mark_all_rooms();
    assert_eq!(
        world.update_rooms(&tuning, 0, &mut ghost),
        Err(RoomError::Standing(77))
    );
}

#[test]
fn a_rooms_light_follows_the_time_of_day_and_its_change_marks_the_room() {
    let mut world = counted_box();
    let mut window = thing(1);
    (window.data[40], window.data[51]) = (0x400, 10);
    world.objects.insert(1, window);
    let mut standing = |_: &mut World, tile: [u8; 3]| {
        Ok(match tile {
            [3, 2, 1] => vec![Standing {
                id: 1,
                kind: 8,
                on_lot: true,
            }],
            _ => Vec::new(),
        })
    };
    let tuning = Tuning::default();
    let mut light = |world: &mut World, period: i16| {
        world.globals[4] = period;
        world.mark_all_rooms();
        world.update_rooms(&tuning, 0, &mut standing).unwrap();
        let room = &world.rooms.rooms[&1];
        (room.light, room.dirty, world.rooms.pending)
    };
    // Tile 3, 2 is listed once: 10 hundredths over 7 tiles times 30, by
    // day. The room had no light, so it is marked again, but its level
    // does not wait: the update clears that last.
    let by_day = F32::from_bits(0x3edb_6db6);
    assert_eq!(light(&mut world, 0), (by_day, true, 0));
    // The same light again leaves the room counted.
    assert_eq!(light(&mut world, 0), (by_day, false, 0));
    // Half the daylight at dawn and dusk, none at night.
    let at_dawn = F32::from_bits(0x3e5b_6db6);
    assert_eq!(light(&mut world, 1), (at_dawn, true, 0));
    assert_eq!(light(&mut world, 3), (at_dawn, false, 0));
    assert_eq!(light(&mut world, 2), (F32::ZERO, true, 0));
    // The outdoor rooms take the daylight itself, and the lot's outdoor
    // score the bonus of the time of day: -50 at night, -20 at dusk.
    assert_eq!(world.rooms.rooms[&0].light, F32::ZERO);
    assert_eq!(world.rooms.outdoor_score, F32::from_f32(-50.0));
    light(&mut world, 3);
    assert_eq!(world.rooms.rooms[&0].light, F32::from_f32(0.5));
    assert_eq!(world.rooms.outdoor_score, F32::from_f32(-20.0));
}

#[test]
fn a_rooms_score_is_rounded_after_every_operation() {
    let tuning = Tuning::default();
    let day = Tuning::daylight(0);
    // 13 tiles, every floor and wall covered, one point of impact, and
    // enough window light for a light of 1. The parts are 0 for the
    // walls, 0 for the floor, 10 / 13 for the impact, 20 for the light
    // and -17 for the area. Adding 10 / 13 to 20 rounds at the size of
    // 20, which leaves the sum four units in the last place below the
    // sum rounded once.
    let room = Room {
        half_tiles: 26,
        covered_half_tiles: 26,
        walls: 10,
        covered_walls: 10,
        impact: 1,
        window_light: 1000,
        ..Room::default()
    };
    let (light, score) = tuning.score(&room, day);
    assert_eq!(light, F32::from_f32(1.0));
    assert_eq!(score, F32::from_bits(SCORE_13));
    assert_ne!(score, F32::from_f32(3.0 + 10.0 / 13.0));
    // Lamp light: 30 hundredths over 45 tiles times 30. No walls at all
    // divides zero by zero, which is not a number and scores as -100.
    let room = Room {
        half_tiles: 91,
        lamp_light: 30,
        ..Room::default()
    };
    let (light, score) = tuning.score(&room, day);
    assert_eq!(light, F32::from_bits(0x3e4c_cccc));
    assert_eq!(score, F32::from_f32(-100.0));
    // Light below 0.01 is none.
    let room = Room {
        half_tiles: 91,
        lamp_light: 1,
        ..Room::default()
    };
    assert_eq!(tuning.score(&room, day).0, F32::ZERO);
    // A room of no tiles divides by zero: its light is not a number and
    // reads as none.
    assert_eq!(tuning.score(&Room::default(), day).0, F32::ZERO);
    // The score stays within -100 and 100.
    let room = Room {
        half_tiles: 20,
        covered_half_tiles: 20,
        walls: 4,
        covered_walls: 4,
        impact: 1000,
        ..Room::default()
    };
    assert_eq!(tuning.score(&room, day).1, F32::from_f32(100.0));
    // The constants are read by name, and a slope is the difference of
    // two of them.
    let tuned = Tuning::from_lookup(|name| match name {
        "light score for 1.0" => Some(50.0),
        "outdoor object divisor" => Some(4.0),
        _ => None,
    });
    assert_eq!(tuned.light_slope, F32::from_f32(70.0));
    assert_eq!(tuned.outdoor_divisor, F32::from_f32(4.0));
    assert_eq!(tuned.area_slope, tuning.area_slope);
    assert_eq!(
        tuned.outdoor_score(F32::from_f32(10.0), 7),
        F32::from_f32(2.5)
    );
    assert_eq!(
        tuning.outdoor_score(F32::from_f32(900.0), 0),
        F32::from_f32(100.0)
    );
}

/// The score of the 13-tile room above, worked out by hand with every
/// operation rounded to a float.
const SCORE_13: u32 = 0x4071_3b10;

#[test]
fn a_dark_room_with_people_and_no_lights_is_not_ported() {
    let dark = Room {
        people: 1,
        ..Room::default()
    };
    assert_eq!(dark.shown_light(4), Err(RoomError::Occupancy(4)));
    // A light, no people, enough light or a pool settles it.
    let lit = F32::from_f32(0.8);
    let cases = [
        (
            Room {
                lights: 1,
                ..dark.clone()
            },
            F32::ZERO,
        ),
        (
            Room {
                people: 0,
                ..dark.clone()
            },
            F32::ZERO,
        ),
        (
            Room {
                light: lit,
                ..dark.clone()
            },
            lit,
        ),
        (
            Room {
                pool: true,
                ..dark.clone()
            },
            lit,
        ),
        // An outdoor room never knows who is in it.
        (
            Room {
                outdoors: true,
                ..dark.clone()
            },
            F32::ZERO,
        ),
    ];
    for (room, shown) in cases {
        assert_eq!(room.shown_light(4), Ok(shown));
    }
}
