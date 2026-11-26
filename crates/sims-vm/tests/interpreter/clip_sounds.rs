//! The sounds a clip's timed events play (0x5912fd at 0x591505 to
//! 0x59175e; `footsteps.rs`): "sound", "selectedsound",
//! "deselectedsound" and "footstep", with the floor surface under the
//! animator (0x58900f, 0x5890a8). Synthetic; no game data.

use crate::animate::{CLIP, HUNDRED, SKILL, standing, track, tuning};
use crate::support::*;
use sims_content::Clip;
use sims_core::F32;
use sims_formats::lot::Array;
use sims_sim::PersonOutfit;
use sims_sim::animator::{Animator, LOADED, Layer};
use sims_vm::{Interpreter, SoundEvent};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// The lot arrays read: the level-1 object grid, floors and pool.
const GRID: u16 = 3;
const FLOORS: u16 = 1;
const POOL: u16 = 9;
/// The Sim's tile: (100, 200) is column 6, row 12.
const COLUMN: usize = 6;
const ROW: usize = 12;

/// Floor pattern keys and their categories (0x49fd95): 'C' 3, 'M' 1,
/// 'H' or other 0, 'S' 2.
const CERAMIC: u16 = 40;
const METAL: u16 = 41;
const HARD: u16 = 42;
const SOFT: u16 = 43;
const UNREAD: u16 = 44;

fn categories() -> BTreeMap<u16, u8> {
    BTreeMap::from([(CERAMIC, 3), (METAL, 1), (HARD, 0), (SOFT, 2)])
}

fn array(stride: usize, fill: u8) -> Array {
    Array {
        width: 64,
        height: 64,
        stride,
        encoding: 0,
        data: vec![fill; 64 * 64 * stride],
    }
}

/// The standing Sim, its foreground clip with one event of `key` and
/// `value` at time 0, playing forward or backward, on a 64-tile lot whose
/// Sim's tile has floor pattern `floor`, no pool and no objects. Forward
/// the event is at time 0, due at the first tick; backward at 50, due
/// within the first few.
fn sounding(key: &str, value: &str, backward: bool, floor: u16) -> Fixture {
    let mut fx = standing();
    fx.program.animations.insert(
        "test.far",
        Clip {
            name: CLIP.into(),
            skill: SKILL.into(),
            duration: HUNDRED,
            distance: 0,
            moving: false,
            frames: 4,
            motions: 1,
            tracks: vec![track(&[(if backward { 50 } else { 0 }, &[(key, value)])])],
            channels: Vec::new(),
            samples: [0, 0],
        },
    );
    let mut foreground = Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, 1, backward, 500);
    foreground.priority = 10;
    fx.world.animators.insert(
        SIM,
        Animator {
            foreground: Some(foreground),
            ..LOADED
        },
    );
    // Nothing silences a footstep (words 84, 68 and 74 are 252, 204 and
    // 222 in the fixture; 222 has bit 3).
    fx.sim_mut().person.as_mut().unwrap().data[74] = 0;
    fx.world.size = 64;
    fx.world.arrays.insert(GRID, array(2, 0));
    fx.world.arrays.insert(POOL, array(1, 255));
    let mut floors = array(2, 0);
    let at = (COLUMN * 64 + ROW) * 2;
    floors.data[at..at + 2].copy_from_slice(&floor.to_le_bytes());
    fx.world.arrays.insert(FLOORS, floors);
    fx
}

fn footstep(value: &str, floor: u16) -> Fixture {
    sounding("footstep", value, false, floor)
}

/// One tick, noting the sounds, with the floor categories given.
fn hear_with(fx: &mut Fixture, floors: Option<&BTreeMap<u16, u8>>) -> Vec<SoundEvent> {
    let sink = RefCell::new(Vec::new());
    let mut interpreter = Interpreter::new(&fx.program, &fx.catalog, fx.stream);
    interpreter.sounds = Some(&sink);
    interpreter.floor_categories = floors;
    let ticks = fx.world.ticks as i32;
    interpreter
        .tick_person(&mut fx.world, SIM, ticks, &tuning())
        .unwrap();
    fx.world.ticks += 1;
    sink.into_inner()
}

fn hear(fx: &mut Fixture) -> Vec<SoundEvent> {
    hear_with(fx, Some(&categories()))
}

fn played(object: i16, name: &str) -> Vec<SoundEvent> {
    vec![SoundEvent::Play {
        object,
        name: name.into(),
    }]
}

fn dressed(fx: &mut Fixture, suit: i32) {
    fx.world.person_outfits.insert(
        SIM,
        PersonOutfit {
            last_dressed: [suit, -1, -1, -1],
            ..PersonOutfit::new()
        },
    );
}

#[test]
fn a_footstep_on_a_floor_sounds_of_the_patterns_category() {
    // 0x5891d5: no pattern is the ground (0, "terrain"), 255 is 2 (hard);
    // a pattern by its category: 3 -> 1, 1 -> 2 (hard), 0 -> 3 (medium),
    // 2 -> 4 (soft).
    for (floor, name) in [
        (0, "footstep_terrain"),
        (255, "footstep_hard"),
        (CERAMIC, "footstep_hard"),
        (METAL, "footstep_hard"),
        (HARD, "footstep_medium"),
        (SOFT, "footstep_soft"),
    ] {
        let mut fx = footstep("1", floor);
        assert_eq!(hear(&mut fx), played(SIM as i16, name), "floor {floor}");
    }
    // A pattern whose category was not read plays nothing, nor does any
    // pattern when no categories are given; the ground needs none.
    let mut fx = footstep("1", UNREAD);
    assert_eq!(hear(&mut fx), []);
    let mut fx = footstep("1", CERAMIC);
    assert_eq!(hear_with(&mut fx, None), []);
    let mut fx = footstep("1", 0);
    assert_eq!(
        hear_with(&mut fx, None),
        played(SIM as i16, "footstep_terrain")
    );
}

#[test]
fn bare_feet_and_the_ground_pick_their_own_footsteps() {
    // Suits 1, 2, 5 and 14 (animator +0x1b0) are bare-footed: "_noshoe"
    // for hard, medium, soft and the ground's soft or terrain.
    for suit in [1, 2, 5, 14] {
        for (floor, name) in [
            (CERAMIC, "footstep_hard_noshoe"),
            (HARD, "footstep_medium_noshoe"),
            (SOFT, "footstep_soft_noshoe"),
            (0, "footstep_terrain_noshoe"),
        ] {
            let mut fx = footstep("2", floor);
            dressed(&mut fx, suit);
            assert_eq!(hear(&mut fx), played(SIM as i16, name), "suit {suit}");
        }
    }
    for suit in [-1, 0, 3, 4, 6, 13, 15] {
        let mut fx = footstep("2", CERAMIC);
        dressed(&mut fx, suit);
        assert_eq!(hear(&mut fx), played(SIM as i16, "footstep_hard"));
    }
    // The ground (0x5916b0): snow on houses 40 to 42 (0x4c471b), shod or
    // not; soft on house 99; terrain elsewhere.
    for (house, bare, name) in [
        (39, false, "footstep_terrain"),
        (40, false, "footstep_snow"),
        (42, true, "footstep_snow"),
        (43, false, "footstep_terrain"),
        (99, false, "footstep_soft"),
        (99, true, "footstep_soft_noshoe"),
    ] {
        let mut fx = footstep("1", 0);
        fx.world.globals[10] = house;
        if bare {
            dressed(&mut fx, 5);
        }
        assert_eq!(hear(&mut fx), played(SIM as i16, name), "house {house}");
    }
    // A floor pattern is not the ground, so snow does not reach it.
    let mut fx = footstep("1", SOFT);
    fx.world.globals[10] = 41;
    assert_eq!(hear(&mut fx), played(SIM as i16, "footstep_soft"));
}

#[test]
fn a_pool_tile_comes_before_the_floor() {
    // Level 1: pool below 16 is 11 (0x478f76), its edge 16 is 1 (hard,
    // 0x478fa3); above, the floor.
    for (pool, name) in [
        (0, "footstep_pool_swim_stroke"),
        (15, "footstep_pool_swim_stroke"),
        (16, "footstep_hard"),
        (17, "footstep_soft"),
    ] {
        let mut fx = footstep("1", SOFT);
        fx.world.arrays.get_mut(&POOL).unwrap().data[COLUMN * 64 + ROW] = pool;
        assert_eq!(hear(&mut fx), played(SIM as i16, name), "pool {pool}");
    }
    // Bare feet in the pool still swim.
    let mut fx = footstep("1", SOFT);
    fx.world.arrays.get_mut(&POOL).unwrap().data[COLUMN * 64 + ROW] = 3;
    dressed(&mut fx, 1);
    assert_eq!(
        hear(&mut fx),
        played(SIM as i16, "footstep_pool_swim_stroke")
    );
}

/// Puts objects of GUIDs `first`, then `next`, on the Sim's tile (the
/// grid names the first; its slot 0 the next), and one of GUID `held` in
/// the first's slot 1.
fn on_the_tile(fx: &mut Fixture, first: u32, next: u32, held: u32) {
    const A: u16 = 30;
    const B: u16 = 31;
    const C: u16 = 32;
    for (id, guid) in [(A, first), (B, next), (C, held)] {
        let o = fx.add_object(id);
        o.guid = guid;
        for slot in &mut o.slots {
            slot.object = 0;
        }
        o.position = [100, 200, 1];
    }
    let set = |fx: &mut Fixture, id: u16, slot: i16, before: u16| {
        let o = fx.world.objects.get_mut(&id).unwrap();
        o.data[14] = slot;
        o.data[26] = before as i16;
    };
    set(fx, A, -1, 0);
    set(fx, B, 0, A);
    set(fx, C, 2, A);
    let a = fx.world.objects.get_mut(&A).unwrap();
    a.slots[0].object = B as i16;
    a.slots[1].object = C as i16;
    let at = (COLUMN * 64 + ROW) * 2;
    fx.world.arrays.get_mut(&GRID).unwrap().data[at..at + 2].copy_from_slice(&A.to_le_bytes());
}

#[test]
fn the_highest_object_on_the_tile_wins_over_pool_and_floor() {
    // 0x5890f4: of the objects on the tile not in a container, the
    // highest rank; one held in a slot is passed over. Before the pool.
    let plant = 0x96e3_398d;
    let trash = 0x7f90_7075;
    let ash = 0x6341_6ba1;
    let roach = 99_999_999;
    let puddle = 0x5c67_fc8f;
    let lamp = LAMP_GUID;
    for (first, next, held, name) in [
        (plant, lamp, roach, "footstep_plant"),
        (lamp, trash, puddle, "footstep_trash"),
        (ash, plant, puddle, "footstep_ash"),
        (ash, roach, lamp, "footstep_roach"),
        (puddle, roach, lamp, "footstep_puddle"),
    ] {
        let mut fx = footstep("1", SOFT);
        fx.world.arrays.get_mut(&POOL).unwrap().data[COLUMN * 64 + ROW] = 3;
        on_the_tile(&mut fx, first, next, held);
        assert_eq!(hear(&mut fx), played(SIM as i16, name), "{name}");
    }
    // Only a held object of a rank: the pool decides.
    let mut fx = footstep("1", SOFT);
    fx.world.arrays.get_mut(&POOL).unwrap().data[COLUMN * 64 + ROW] = 3;
    on_the_tile(&mut fx, lamp, lamp, roach);
    assert_eq!(
        hear(&mut fx),
        played(SIM as i16, "footstep_pool_swim_stroke")
    );
}

#[test]
fn a_footstep_plays_only_the_way_the_clip_goes_and_unsilenced() {
    // Forward, 1 and 2; backward, -1 and -2 (0x59172a).
    for (value, backward, heard) in [
        ("1", false, true),
        ("2", false, true),
        ("-1", false, false),
        ("3", false, false),
        ("0", false, false),
        ("x", false, false),
        ("-1", true, true),
        ("-2", true, true),
        ("1", true, false),
        ("-3", true, false),
    ] {
        let mut fx = sounding("footstep", value, backward, SOFT);
        let sounds: Vec<_> = (0..6).flat_map(|_| hear(&mut fx)).collect();
        assert_eq!(sounds.len(), usize::from(heard), "{value} {backward}");
    }
    // Person word 84 bit 0, word 68 bit 0, word 74 bit 3 silence it; other
    // bits do not.
    for (word, bits, heard) in [
        (84, 1, false),
        (84, 2, true),
        (68, 1, false),
        (68, -2, true),
        (74, 8, false),
        (74, 7, true),
    ] {
        let mut fx = footstep("1", SOFT);
        fx.sim_mut().person.as_mut().unwrap().data[word] = bits;
        assert_eq!(hear(&mut fx).len(), usize::from(heard), "word {word}");
    }
}

#[test]
fn a_footstep_needs_the_animator_on_the_lots_arrays() {
    // A coordinate below 0 is off the arrays (0x58902c, -1; NaN too).
    // (An x of -999, an animator not yet placed, is passed over before
    // that, 0x5915f6; being below 0 it would play nothing anyway.)
    for (position, heard) in [
        ([-999.000_1, 200.0], false),
        ([-0.5, 200.0], false),
        ([100.0, -1.0], false),
        ([-0.0, 0.0], true),
        ([100.0, 200.0], true),
    ] {
        let mut fx = footstep("1", 0);
        let anim = fx.world.animators.get_mut(&SIM).unwrap();
        let mut foreground = anim.foreground.take();
        hear(&mut fx);
        // Play the clip from the next tick, the animator where wanted.
        let anim = fx.world.animators.get_mut(&SIM).unwrap();
        anim.position[0] = F32::from_f32(position[0]);
        anim.position[1] = F32::from_f32(position[1]);
        anim.destination = anim.position;
        let layer = foreground.as_mut().unwrap();
        *layer = Layer::start(CLIP, F32::from_bits(HUNDRED), 4, true, 1, false, anim.clock);
        layer.priority = 10;
        anim.foreground = foreground;
        let sounds = hear(&mut fx);
        assert_eq!(!sounds.is_empty(), heard, "{position:?}: {sounds:?}");
    }
}

#[test]
fn sound_plays_for_a_person_on_the_lot_or_a_pet_off_it() {
    // 0x432595: on the lot (tile inside the border), for the person.
    let mut fx = sounding("sound", "bark", false, 0);
    assert_eq!(hear(&mut fx), played(SIM as i16, "bark"));
    // Off it: for object -4 when kind (word 65) is above 1.
    for (kind, heard) in [(0, false), (1, false), (2, true), (3, true)] {
        let mut fx = sounding("sound", "bark", false, 0);
        fx.world.size = 10;
        fx.sim_mut().person.as_mut().unwrap().data[65] = kind;
        let expected = if heard { played(-4, "bark") } else { vec![] };
        assert_eq!(hear(&mut fx), expected, "kind {kind}");
    }
}

#[test]
fn selected_and_deselected_sounds_follow_the_selected_mark() {
    for (selected, key, heard) in [
        (true, "selectedsound", true),
        (false, "selectedsound", false),
        (true, "deselectedsound", false),
        (false, "deselectedsound", true),
    ] {
        let mut fx = sounding(key, "hi", false, 0);
        fx.sim_mut().selected = selected;
        let expected = if heard {
            played(SIM as i16, "hi")
        } else {
            vec![]
        };
        assert_eq!(hear(&mut fx), expected, "{key} {selected}");
    }
}

#[test]
fn the_sounds_leave_the_world_as_it_would_be_without_them() {
    // Noting the sounds or not, with categories or none, the world comes
    // out the same: nothing is written, no number drawn. The tile's first
    // object has slot word 0 and nothing before it, which 0x4325b5 would
    // put right; the footstep's read leaves it.
    let build = |key: &str| {
        let mut fx = sounding(key, "1", false, CERAMIC);
        on_the_tile(&mut fx, LAMP_GUID, 0x96e3_398d, 99_999_999);
        fx.world.objects.get_mut(&30).unwrap().data[14] = 0;
        fx
    };
    for key in ["footstep", "sound", "selectedsound", "deselectedsound"] {
        let mut heard = build(key);
        let mut deaf = build(key);
        let sounds = hear(&mut heard);
        assert!(!sounds.is_empty() || key == "selectedsound", "{key}");
        hear_with(&mut deaf, None);
        let mut silent = build(key);
        let t = tuning();
        let now = silent.world.ticks as i32;
        Interpreter::new(&silent.program, &silent.catalog, silent.stream)
            .tick_person(&mut silent.world, SIM, now, &t)
            .unwrap();
        silent.world.ticks += 1;
        assert_eq!(heard.world.hash(), deaf.world.hash(), "{key}");
        assert_eq!(heard.world.hash(), silent.world.hash(), "{key}");
        assert_eq!(heard.world.objects[&30].data[14], 0, "{key}");
    }
}
