//! Authored domestic acceptance layouts; all activities use original scripts.
use crate::{Index, household, vm};
use anyhow::Result;
use sims_sim::{Body, Point, Simulation, StepStop, architecture::Wall};
use sims_vm::Program;

#[derive(Debug, Clone, Copy)]
pub enum Layout {
    StraightKitchen,
    CornerKitchen,
}
pub fn home(index: &Index) -> Result<(Program, Simulation)> {
    home_with(index, 7, Layout::StraightKitchen)
}
/// Busier routing acceptance lot: four original adult Main loops, four beds,
/// four table seats, and shared kitchen/bathroom doors. Still one storey.
pub fn four_residents(index: &Index, seed: u32) -> Result<(Program, Simulation)> {
    let (mut p, mut s) = home_with(index, seed, Layout::StraightKitchen)?;
    s.room.height = 256;
    for id in [18, 19] {
        s.bodies.get_mut(&id).unwrap().position.y += 48;
    }
    s.bodies.get_mut(&12).unwrap().position.y = 248;
    let mut walls = s.architecture.as_ref().unwrap().floors[0]
        .plan
        .walls
        .clone();
    for wall in &mut walls {
        if wall.a.y == 192 && wall.b.y == 192 {
            wall.a.y += 48;
            wall.b.y += 48;
        }
    }
    for y in (192..240).step_by(16) {
        for x in [32, 144, 224] {
            walls.push(Wall {
                a: Point::new(x, y),
                b: Point::new(x, y + 16),
                door: None,
            });
        }
    }
    s.set_architecture(&p, walls).map_err(anyhow::Error::msg)?;
    for (id, neighbor, x) in [(32, 3, 88), (33, 4, 120)] {
        household::add_resident(
            &p,
            &mut s,
            id,
            neighbor,
            Body {
                level: 1,
                position: Point::new(x, 200),
                facing: 1,
                radius: 3,
                elevation: 0,
            },
        )?;
        s.vm.world
            .objects
            .get_mut(&id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(10, 200);
    }
    for (file, definition, id, x, y, facing) in [
        ("Beds.iff", 16816, 34, 168, 216, 4),
        ("Beds.iff", 16816, 37, 200, 216, 4),
        ("DiningChairs.iff", 16807, 40, 88, 88, 16),
        ("DiningChairs.iff", 16807, 41, 88, 120, 1),
    ] {
        household::add_object(
            index,
            &mut p,
            &mut s,
            file,
            definition,
            id,
            Body {
                level: 1,
                position: Point::new(x, y),
                facing,
                radius: 6,
                elevation: 0,
            },
        )?;
    }
    let objects =
        s.vm.world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_none())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    s.enable_residential_autonomy(objects)
        .map_err(anyhow::Error::msg)?;
    loop {
        let r = s.run_fixture_tick(&p, 4000).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
            "four-resident initialization failed"
        );
        if r.stop != StepStop::Budget {
            break;
        }
    }
    s.initialize_seat_adjacency(&p)
        .map_err(anyhow::Error::msg)?;
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}
pub fn home_with(index: &Index, seed: u32, layout: Layout) -> Result<(Program, Simulation)> {
    let (mut p, mut s) = household::needs_room(index, "Fridges.iff", 16807, 7)?;
    s.vm.rng_state = seed;
    s.room.width = 256;
    s.room.height = 224;
    s.room.obstacles.clear();
    for (id, x, y, facing) in [
        (1, 88, 144, 1),
        (2, 72, 104, 4),
        (3, 120, 104, 64),
        (4, 56, 56, 16),
    ] {
        let body = s.bodies.get_mut(&id).unwrap();
        body.position = Point::new(x, y);
        body.facing = facing;
        s.vm.world
            .objects
            .get_mut(&id)
            .unwrap()
            .data
            .insert(1, facing.trailing_zeros() as i16);
    }
    for (file, def, id, x, y, facing) in [
        ("Sinks.iff", 16807, 5, 200, 56, 16),
        ("Toilets.iff", 16807, 6, 168, 56, 16),
        ("ShowerC.iff", 16807, 9, 200, 88, 1),
        ("Bookcases.iff", 16811, 10, 56, 136, 1),
        ("trash1.iff", 16807, 11, 56, 168, 1),
        ("TrashOutside.iff", 16807, 12, 136, 208, 1),
        ("Doors.iff", 16815, 14, 136, 88, 64),
        ("Doors.iff", 16815, 16, 136, 152, 64),
        ("Doors.iff", 16815, 18, 104, 184, 1),
        ("Beds.iff", 16816, 20, 168, 152, 4),
        ("Beds.iff", 16816, 23, 200, 152, 4),
        ("Counters.iff", 16810, 26, 88, 56, 16),
        ("Stoves.iff", 16807, 27, 120, 56, 16),
        ("DiningTables.iff", 16815, 28, 88, 104, 1),
    ] {
        let (x, y, facing) = if id == 26 && matches!(layout, Layout::CornerKitchen) {
            (56, 88, 4)
        } else {
            (x, y, facing)
        };
        household::add_object(
            index,
            &mut p,
            &mut s,
            file,
            def,
            id,
            Body {
                level: 1,
                position: Point::new(x, y),
                facing,
                radius: 6,
                elevation: 0,
            },
        )?;
    }
    household::add_resident(
        &p,
        &mut s,
        13,
        2,
        Body {
            level: 1,
            position: Point::new(120, 144),
            facing: 64,
            radius: 3,
            elevation: 0,
        },
    )?;
    // Authored residents have two cooking skill points. Fires remain a separate
    // compatibility boundary; the original stove's skill gate stays untouched.
    for id in [1, 13] {
        s.vm.world
            .objects
            .get_mut(&id)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(10, 200);
    }
    for file in [
        "HDSI_Talk.iff",
        "SocialDRT.iff",
        "Destination.iff",
        "HDSI.iff",
        "dropdest.iff",
    ] {
        for (name, module) in vm::load_program(index, file, "Global.iff")?.modules {
            p.modules.entry(name).or_insert(module);
        }
    }
    // Original global failure-sleep helper creates its placement object by GUID.
    crate::assets::load_object_guids(index, &mut p, &[0x3929aadc])?;
    let mut walls = vec![];
    for x in (32..224).step_by(16) {
        for y in [32, 192] {
            walls.push(Wall {
                a: Point::new(x, y),
                b: Point::new(x + 16, y),
                door: (x == 96 && y == 192).then_some([18, 19]),
            });
        }
    }
    for y in (32..192).step_by(16) {
        for x in [32, 144, 224] {
            walls.push(Wall {
                a: Point::new(x, y),
                b: Point::new(x, y + 16),
                door: match (x, y) {
                    (144, 80) => Some([14, 15]),
                    (144, 144) => Some([16, 17]),
                    _ => None,
                },
            });
        }
    }
    for x in (144..224).step_by(16) {
        walls.push(Wall {
            a: Point::new(x, 112),
            b: Point::new(x + 16, 112),
            door: None,
        });
    }
    s.set_architecture(&p, walls).map_err(anyhow::Error::msg)?;
    crate::assets::load_linked_animations(index, &mut p)?;
    p.accessories = crate::avatar::load_linked_accessories(index, &p)?
        .into_iter()
        .map(|(n, a)| (n, a.sources))
        .collect();
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    let objects =
        s.vm.world
            .objects
            .iter()
            .filter(|(_, o)| o.person.is_none())
            .map(|(&id, _)| id)
            .collect::<Vec<_>>();
    s.enable_residential_autonomy(objects)
        .map_err(anyhow::Error::msg)?;
    loop {
        let r = s.run_fixture_tick(&p, 4000).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(
            !matches!(r.stop, StepStop::Faulted | StepStop::Animation),
            "domestic initialization: {:?}",
            s.vm.threads
        );
        if r.stop != StepStop::Budget {
            break;
        }
    }
    s.initialize_seat_adjacency(&p)
        .map_err(anyhow::Error::msg)?;
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}
