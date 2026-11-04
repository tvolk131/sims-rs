//! Original stair behavior acceptance fixtures; authored geometry is explicit.
use crate::{Index, assets, household};
use sims_sim::{Body, Point, Simulation, architecture::FloorPlan};
use sims_vm::Program;

pub fn laboratory(index: &Index) -> anyhow::Result<(Program, Simulation)> {
    let (mut p, mut s) = household::needs_room(index, "Fridges.iff", 16807, 7)?;
    s.room.width = 160;
    s.room.height = 160;
    s.room.obstacles.clear();
    for (id, x, y, level) in [
        (1, 56, 136, 1),
        (2, 56, 40, 2),
        (3, 24, 88, 1),
        (4, 24, 24, 1),
    ] {
        let b = s.bodies.get_mut(&id).unwrap();
        b.position = Point::new(x, y);
        b.level = level;
    }
    household::add_object(
        index,
        &mut p,
        &mut s,
        "stair.iff",
        16808,
        50,
        Body {
            level: 1,
            position: Point::new(88, 104),
            facing: 1,
            radius: 6,
            elevation: 0,
        },
    )?;
    assets::load_object_guids(index, &mut p, &[0x7c4])?;
    let ground = FloorPlan {
        walls: vec![],
        supported: vec![true; 100],
    };
    let mut upper = ground.clone();
    for y in 3..7 {
        upper.supported[y * 10 + 5] = false;
    }
    s.set_floors(&p, vec![ground, upper])
        .map_err(anyhow::Error::msg)?;
    assets::load_linked_animations(index, &mut p)?;
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    s.enable_residential_autonomy(
        s.vm.world
            .objects
            .keys()
            .copied()
            .filter(|id| *id != 1)
            .collect::<Vec<_>>(),
    )
    .map_err(anyhow::Error::msg)?;
    s.vm.world.person.as_mut().unwrap().free_will = false;
    s.initialize_seat_adjacency(&p)
        .map_err(anyhow::Error::msg)?;
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}

/// Four adults share a two-storey home: kitchen and bathroom downstairs,
/// bedrooms and reading upstairs. Two original stair groups offer alternate
/// connections. Layout, funds, and initial motives are authored scenario inputs.
pub fn home(index: &Index, seed: u32) -> anyhow::Result<(Program, Simulation)> {
    let (mut p, mut s) = crate::domestic::four_residents(index, seed)?;
    for (&id, b) in &mut s.bodies {
        if [20, 21, 22, 23, 24, 25, 34, 35, 36, 37, 38, 39, 10].contains(&id) {
            b.level = 2;
        }
    }
    for (head, x, y) in [(20, 72, 72), (23, 120, 72), (34, 56, 216), (37, 200, 216)] {
        let old = s.bodies[&head].position;
        for id in head..head + 3 {
            let b = s.bodies.get_mut(&id).unwrap();
            b.position.x += x - old.x;
            b.position.y += y - old.y;
        }
    }
    s.bodies.get_mut(&33).unwrap().position = Point::new(104, 200);
    for (id, x, y) in [(50, 168, 168), (68, 120, 216)] {
        household::add_object(
            index,
            &mut p,
            &mut s,
            "stair.iff",
            16808,
            id,
            Body {
                level: 1,
                position: Point::new(x, y),
                facing: 1,
                radius: 6,
                elevation: 0,
            },
        )?;
    }
    let ground = s.architecture.as_ref().unwrap().floors[0].plan.clone();
    let mut upper = FloorPlan {
        walls: vec![],
        supported: vec![false; 256],
    };
    for y in 2..15 {
        for x in 2..14 {
            upper.supported[y * 16 + x] = true;
        }
    }
    // Upstairs has an enclosed shared landing/bedroom; ground-floor partitions
    // are not implicitly copied onto the second storey.
    for x in (32..224).step_by(16) {
        for y in [32, 240] {
            upper.walls.push(sims_sim::architecture::Wall {
                a: Point::new(x, y),
                b: Point::new(x + 16, y),
                door: None,
            });
        }
    }
    for y in (32..240).step_by(16) {
        for x in [32, 224] {
            upper.walls.push(sims_sim::architecture::Wall {
                a: Point::new(x, y),
                b: Point::new(x, y + 16),
                door: None,
            });
        }
    }
    for id in [54, 55, 56, 72, 73, 74] {
        let b = &s.bodies[&id];
        upper.supported[(b.position.y / 16 * 16 + b.position.x / 16) as usize] = false;
    }
    s.set_floors(&p, vec![ground, upper])
        .map_err(anyhow::Error::msg)?;
    assets::load_linked_animations(index, &mut p)?;
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    let advertised =
        s.vm.world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_none())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
    s.enable_residential_autonomy(advertised)
        .map_err(anyhow::Error::msg)?;
    s.initialize_seat_adjacency(&p)
        .map_err(anyhow::Error::msg)?;
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}
