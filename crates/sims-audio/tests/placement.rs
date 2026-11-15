//! The 3D placement (0x5730b6) and the sum over a track's objects
//! (0x57473b).

use sims_audio::placement::{CENTRE, FULL, Projection, STALE_VOL, combine, place, project};
use sims_audio::player::Sources;
use std::collections::BTreeMap;

/// Objects at positions, some of them on the screen.
#[derive(Default)]
struct Lot {
    positions: BTreeMap<i32, [i32; 3]>,
    views: BTreeMap<i32, Projection>,
}

impl Sources for Lot {
    fn field(&self, _: i32, _: i32) -> Option<i32> {
        Some(0)
    }

    fn position(&self, object: i32) -> Option<[i32; 3]> {
        self.positions.get(&object).copied()
    }

    fn projection(&self, object: i32) -> Option<Projection> {
        self.views.get(&object).copied()
    }
}

fn lot(objects: &[(i32, [i32; 3])]) -> Lot {
    Lot {
        positions: objects.iter().copied().collect(),
        views: BTreeMap::new(),
    }
}

#[test]
fn an_object_on_the_lot_is_at_full_volume_in_the_centre_when_spl_is_0() {
    let lot = lot(&[(5, [40, 56, 1]), (6, [-16, -16, 2])]);
    assert_eq!(place(5, 0, &lot), Ok(Some((FULL, CENTRE))));
    // An object taken off the lot is still found, at -16, -16.
    assert_eq!(place(6, 0, &lot), Ok(Some((FULL, CENTRE))));
}

#[test]
fn an_id_not_above_0_is_at_full_volume_in_the_centre() {
    let lot = lot(&[]);
    assert_eq!(place(0, 20, &lot), Ok(Some((FULL, CENTRE))));
    assert_eq!(place(-3, 20, &lot), Ok(Some((FULL, CENTRE))));
}

#[test]
fn no_object_or_a_coordinate_of_0_leaves_the_values() {
    let lot = lot(&[(5, [0, 56, 1]), (6, [40, 0, 1])]);
    assert_eq!(place(4, 0, &lot), Ok(None));
    assert_eq!(place(5, 0, &lot), Ok(None));
    assert_eq!(place(6, 0, &lot), Ok(None));
}

#[test]
fn spl_without_a_screen_is_a_fault() {
    let lot = lot(&[(5, [40, 56, 1])]);
    assert!(place(5, 20, &lot).is_err());
}

fn view(at: [i32; 2], zoom: i32) -> Projection {
    Projection {
        at,
        screen: [800, 600],
        zoom,
        shown_level: 1,
    }
}

#[test]
fn spl_places_an_object_by_where_it_is_on_the_screen() {
    // Right of the middle: (600 - 400) * 3 * 1024 / 40000 = 15 right of
    // the centre. Spl 20 at zoom 1 takes (3 - 1) * 20 * 14 = 560 off the
    // volume; inside the screen's middle nothing more.
    assert_eq!(project(20, 1, &view([600, 300], 1)), Ok((464, 527)));
    // Left of the fifth: 60 out, so 464 * 540 / 600; on another level,
    // three fifths of that.
    assert_eq!(project(20, 2, &view([100, 300], 1)), Ok((250, 489)));
    // Spl 10001 keeps at least 800, down to that 600 or more outside;
    // the pan is -1100 * 3 * 1024 / 40000, -84.
    assert_eq!(project(10_001, 1, &view([-700, 300], 3)), Ok((800, 428)));
    // Far right: the pan stops at full right.
    assert_eq!(project(20, 1, &view([8000, 300], 3)).map(|p| p.1), Ok(1024));
    // A screen 0 wide is a division by 0.
    let mut flat = view([1, 1], 1);
    flat.screen = [0, 600];
    assert!(project(20, 1, &flat).is_err());
    // Through the trait.
    let mut lot = lot(&[(5, [40, 56, 1])]);
    lot.views.insert(5, view([600, 300], 1));
    assert_eq!(place(5, 20, &lot), Ok(Some((464, 527))));
}

#[test]
fn the_sum_takes_the_loudest_and_weights_the_pans() {
    let mut lot = lot(&[(5, [40, 56, 1]), (6, [40, 56, 1])]);
    assert_eq!(combine(&[], 0, &lot), Ok((0, CENTRE)));
    assert_eq!(combine(&[5, 6], 0, &lot), Ok((FULL, CENTRE)));
    // 464 at 527 and 250 at 489 (on another level): the pan is
    // (15 * 464 - 23 * 250) / 714 right of the centre, 1.
    lot.views.insert(5, view([600, 300], 1));
    lot.views.insert(6, view([100, 300], 1));
    lot.positions.insert(6, [40, 56, 2]);
    assert_eq!(combine(&[5, 6], 20, &lot), Ok((464, 513)));
}

#[test]
fn an_object_not_placed_takes_the_values_before_it() {
    let lot = lot(&[(5, [40, 56, 1])]);
    // The first object not found: the pan starts at 0 and the volume at
    // the stand-in heap address, the loudest. Its weight, -512 times it,
    // wraps to -108544, which the volume divides to 0: the centre.
    assert_eq!(STALE_VOL, 0x0100_00d4);
    assert_eq!(combine(&[4], 0, &lot), Ok((STALE_VOL, CENTRE)));
    assert_eq!(combine(&[4, 5], 0, &lot), Ok((STALE_VOL, CENTRE)));
    // After an object at the centre, it is at the centre too.
    assert_eq!(combine(&[5, 7], 0, &lot), Ok((FULL, CENTRE)));
}
