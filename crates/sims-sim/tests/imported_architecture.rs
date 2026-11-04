use sims_formats::lot::WallTile;
use sims_sim::{
    Point, Rect, Room, Simulation, Snapshot,
    lot_geometry::{FloorSource, Lot},
};
use sims_vm::{Program, Vm};

fn lot() -> Lot {
    let ground = FloorSource {
        coverings: vec![0; 16],
        walls: vec![WallTile::default(); 16],
        flags: vec![0; 16],
        object_heads: vec![0; 16],
    };
    let mut upper = ground.clone();
    upper.coverings[5] = 1;
    upper.walls[5].segments = 16;
    upper.walls[5].styles = [1, 1];
    let mut water = vec![255; 16];
    water[5] = 16;
    Lot::rebuild(
        4,
        vec![[0; 4]; 16],
        vec![255; 16],
        water,
        vec![ground, upper],
    )
    .unwrap()
}

#[test]
fn imported_room_queries_diagonals_and_upper_surfaces_survive_snapshot() {
    let p = Program::default();
    let mut s = Simulation::new(
        Vm::new(Default::default(), 1),
        Room {
            width: 64,
            height: 64,
            obstacles: vec![],
        },
        Default::default(),
    )
    .unwrap();
    s.set_original_lot(&p, lot()).unwrap();
    let a = s.architecture.as_ref().unwrap();
    assert_eq!(a.original.as_ref().unwrap().water[5], 16);
    assert_eq!(a.original.as_ref().unwrap().pool[5], 255);
    let pos = Point::new(24, 24);
    assert_eq!(a.primary_room_at(2, pos), -5);
    assert_eq!(
        a.room_at(2, pos),
        1024,
        "native membership samples neighboring cells"
    );
    assert!(a.supported(2, pos));
    assert!(!a.supported(2, Point::new(40, 24)));
    assert_eq!(
        a.room_at(1, Point::new(8, 24)),
        -5,
        "native room lookup excludes border"
    );
    assert!(!a.walls_clear(
        2,
        Rect {
            min: Point::new(22, 22),
            max: Point::new(26, 26)
        },
        &[]
    ));
    assert!(a.walls_clear(
        2,
        Rect {
            min: Point::new(17, 17),
            max: Point::new(21, 21)
        },
        &[]
    ));
    assert!(!s.vm.world.room_half_tiles.contains_key(&-5));
    let snapshot = Snapshot::capture(&p, &s);
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    let restored: Snapshot = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored.restore(&p).unwrap(), s);
    let mut changed = snapshot;
    changed
        .state
        .architecture
        .as_mut()
        .unwrap()
        .original
        .as_mut()
        .unwrap()
        .floors[1]
        .flags
        .flags[5] ^= 4;
    assert!(
        changed
            .restore(&p)
            .unwrap_err()
            .contains("derived geometry mismatch")
    );
}

#[test]
fn imported_surface_is_not_inferred_from_structural_support() {
    let l = lot();
    assert_eq!(l.floors[1].flags.flags[5] & 1, 0);
    assert_ne!(l.floors[1].source.coverings[5], 0);
    let mut bad = l;
    bad.heights.pop();
    assert!(bad.validate().is_err());
}

#[test]
fn imported_room_directory_precedes_individual_object_room_publication() {
    let p = Program::default();
    let a = sims_sim::architecture::Architecture::from_original(lot(), &p).unwrap();
    let mut vm = Vm::new(Default::default(), 1);
    vm.world.refresh = Some(Default::default());
    vm.world.objects.insert(
        1,
        sims_vm::Object {
            data: std::collections::BTreeMap::from([(29, 1234)]),
            ..Default::default()
        },
    );
    vm.world.positions.insert(
        1,
        sims_vm::autonomy::Placement {
            x: 24,
            y: 24,
            level: 2,
        },
    );
    let before = (vm.world.objects.clone(), vm.world.positions.clone());
    a.publish_room_context(&mut vm.world);
    assert_eq!(
        before,
        (vm.world.objects.clone(), vm.world.positions.clone())
    );
    assert_eq!(
        vm.world.room_half_tiles[&1024],
        a.original.as_ref().unwrap().floors[1].metrics[&1024].half_tiles
    );
    // Once the ordered visit has assigned this room, original Refresh must
    // invalidate the upper-floor category, not the ground-floor category.
    vm.world.objects.get_mut(&1).unwrap().data.insert(29, 1024);
    vm.refresh_object(1, 0, 0, 2).unwrap();
    a.publish_room_context(&mut vm.world);
    let r = vm.world.refresh.as_ref().unwrap();
    assert!(r.rooms[&1024].dirty);
    assert_eq!(r.dirty_categories, 2);
}
