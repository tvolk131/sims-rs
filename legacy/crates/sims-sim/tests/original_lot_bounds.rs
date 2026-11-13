use sims_sim::{
    Body, Point, Room,
    architecture::{Architecture, FloorPlan},
};
use sims_vm::Program;

#[test]
fn full_original_lot_support_and_bounds_cover_both_storeys() {
    let room = Room {
        width: 1024,
        height: 1024,
        obstacles: vec![],
    };
    room.validate().unwrap();
    let mut upper = vec![false; 4096];
    upper[63 * 64 + 63] = true;
    let a = Architecture::with_floors(
        1024,
        1024,
        vec![
            FloorPlan {
                walls: vec![],
                supported: vec![true; 4096],
            },
            FloorPlan {
                walls: vec![],
                supported: upper,
            },
        ],
        &Program::default(),
    )
    .unwrap();
    let mut body = Body {
        level: 2,
        position: Point::new(1016, 1016),
        facing: 1,
        radius: 0,
        elevation: 0,
    };
    body.validate(&room).unwrap();
    assert!(a.supported(1, body.position));
    assert!(a.supported(2, body.position));
    assert!(!a.supported(2, Point::new(1000, 1016)));
    assert_ne!(a.room_at(1, body.position), a.room_at(2, body.position));
    body.position.x = 1024;
    assert!(body.validate(&room).is_err());
    assert!(!a.supported(2, body.position));
    assert!(
        Room {
            width: 1040,
            ..room
        }
        .validate()
        .is_err()
    );
}
