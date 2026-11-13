//! Authored single-floor household fixtures. Object storage/Init/Main and behavior
//! resources are original; placement, registry membership and collision proxies
//! are supplied explicitly until native lot loading and placement are implemented.
use crate::{Index, vm::*};
use anyhow::{Context, Result, bail};
use sims_sim::{Body, Point, Simulation, StepStop, clock::LotClock};
use sims_vm::{Definition, Object, ObjectId, Program};

/// Add an authored adult using recovered person storage and the original Init/Main.
/// Identity, age, family membership and initial needs are scenario inputs.
pub fn add_resident(
    program: &Program,
    sim: &mut Simulation,
    id: ObjectId,
    neighbor: i16,
    body: Body,
) -> Result<()> {
    if id <= 0 || sim.vm.world.objects.contains_key(&id) {
        bail!("invalid or occupied resident ID {id}");
    }
    body.validate(&sim.room).map_err(anyhow::Error::msg)?;
    let level = body.level;
    let definition = sim.vm.world.objects[&1].definition.clone().unwrap();
    let mut actor = Object::construct_person(
        definition,
        program,
        sims_vm::person::BUILTIN_SLOT_HEIGHTS.len(),
    )
    .map_err(anyhow::Error::msg)?;
    for index in 0..72 {
        actor.data.entry(index).or_insert(0);
    }
    for index in 0..256 {
        actor.person.as_mut().unwrap().entry(index).or_insert(0);
    }
    actor.data.extend([
        (1, body.facing.trailing_zeros() as i16),
        (11, id),
        (29, 1),
        (67, 5),
    ]);
    actor
        .person
        .as_mut()
        .unwrap()
        .extend([(31, neighbor), (58, 30), (61, 1)]);
    actor.attributes = vec![0; 8];
    actor.headline = Some(Default::default());
    let world = sim
        .vm
        .world
        .person
        .as_mut()
        .context("person world missing")?;
    if world.neighbors.contains_key(&neighbor) {
        bail!("neighbor already exists");
    }
    world.neighbors.insert(
        neighbor,
        sims_vm::person::Neighbor {
            guid: actor.definition.as_ref().unwrap().guid,
            person: actor.person.clone().unwrap(),
            ..Default::default()
        },
    );
    world.families.entry(1).or_default().push(neighbor);
    world.object_order.push(id);
    world.person_order.push(id);
    if let Some(order) = world
        .resource_instances
        .get_mut(&actor.definition.as_ref().unwrap().guid)
    {
        order.push(id);
    }
    if let Some(context) = &mut world.autonomy {
        context.placements.insert(
            id,
            sims_vm::autonomy::Placement {
                x: body.position.y,
                y: body.position.x,
                level: body.level,
            },
        );
        context.full_scan.as_mut().unwrap().insert(id);
    }
    sim.vm
        .world
        .actions
        .as_mut()
        .context("action world missing")?
        .actors
        .insert(id, Default::default());
    sim.vm
        .world
        .refresh
        .as_mut()
        .context("refresh world missing")?
        .draw
        .insert(id, sims_vm::refresh::DrawState::constructed(level));
    sim.vm.world.objects.insert(id, actor);
    sim.bodies.insert(id, body);
    let thread = sim
        .vm
        .start_object_lifecycle(program, id)
        .map_err(anyhow::Error::msg)?;
    sim.interactions.insert(
        id,
        sims_sim::interaction::ActorQueue {
            main: Some(thread),
            ..Default::default()
        },
    );
    loop {
        let r = sim
            .run_fixture_tick(program, 2000)
            .map_err(anyhow::Error::msg)?;
        if matches!(r.stop, StepStop::Faulted | StepStop::Animation) {
            bail!("resident initialization failed: {:?}", sim.vm.threads);
        }
        if r.stop != StepStop::Budget {
            break;
        }
    }
    let actor = sim.vm.world.objects.get_mut(&id).unwrap();
    let motives = actor.motives.as_mut().context("resident motives missing")?;
    for index in [5, 6, 7, 8, 9, 13, 14, 15] {
        motives[index] = 50f32.to_bits();
    }
    if let Some(household) = &mut sim.household {
        actor.motive_update = Some(sims_vm::motives::UpdateState {
            previous: *motives,
            last_decay: sim.vm.tick as i32,
        });
        household.people.insert(
            id,
            sims_sim::household::PersonUpdate {
                started: None,
                finished: None,
            },
        );
    }
    Ok(())
}

pub fn add_object(
    index: &Index,
    program: &mut Program,
    sim: &mut Simulation,
    selector: &str,
    definition: u16,
    id: ObjectId,
    body: Body,
) -> Result<()> {
    for (source, module) in load_program(index, selector, "Global.iff")?.modules {
        program.modules.entry(source).or_insert(module);
    }
    let source = index.resources[index.find(selector)?].locator.clone();
    let module = &program.modules[&source];
    let words = module
        .object_definition_words
        .get(&definition)
        .context("missing authored definition")?;
    if words[10] == 0 {
        return add_part(index, program, sim, selector, definition, id, body, None);
    }
    if body.facing.trailing_zeros() % 2 != 0 {
        bail!("authored multi-tile placement requires a cardinal direction");
    }
    let plan =
        sims_vm::multitile::construction_plan(module, definition).map_err(anyhow::Error::msg)?;
    let price = words[0x24 / 2];
    let primary = sims_vm::multitile::origin(&module.object_definition_words[&plan[0]])
        .map_err(anyhow::Error::msg)?;
    let mut parts = vec![];
    for (i, &definition) in plan.iter().enumerate() {
        let part_id = id
            .checked_add(i.try_into()?)
            .context("multi-tile ID overflow")?;
        if part_id <= 0 || sim.vm.world.objects.contains_key(&part_id) {
            bail!("occupied group ID {part_id}");
        }
        let origin = sims_vm::multitile::origin(&module.object_definition_words[&definition])
            .map_err(anyhow::Error::msg)?;
        let offset = sims_vm::multitile::rotate(
            std::array::from_fn(|i| origin[i] - primary[i]),
            body.facing.trailing_zeros() as i16,
        );
        let mut placed = body.clone();
        let ([native_x, native_y], level) = sims_vm::multitile::placement(
            [body.position.y, body.position.x],
            body.level,
            [0; 3],
            offset,
        );
        placed.level = level;
        placed.position = Point::new(native_y, native_x);
        placed.validate(&sim.room).map_err(anyhow::Error::msg)?;
        parts.push((
            part_id,
            definition,
            placed,
            sims_vm::multitile::Part {
                head: (i != 0).then_some(id),
                next: (i + 1 < plan.len()).then(|| part_id + 1),
                origin,
                offset,
            },
        ));
    }
    for (id, definition, body, part) in parts {
        add_part(
            index,
            program,
            sim,
            selector,
            definition,
            id,
            body,
            Some(part),
        )?;
    }
    sim.vm
        .world
        .objects
        .get_mut(&id)
        .unwrap()
        .data
        .insert(41, price);
    let order = sim
        .vm
        .prepare_multitile_reset(program, id, Default::default())
        .map_err(anyhow::Error::msg)?;
    // Authored construction is synchronous and finishes before this fixture is
    // returned. Original reset(1) runs Init and one update per part in chain order.
    for part in order {
        sim.vm
            .begin_multitile_init(program, part)
            .map_err(anyhow::Error::msg)?;
        let thread = sim.vm.object_threads[&part];
        loop {
            let run = sim
                .vm
                .run_slice(program, thread, 1000)
                .map_err(anyhow::Error::msg)?;
            match run.stop {
                sims_vm::Stop::Budget => {
                    if run.trace.len() == 1000 {
                        bail!("group Init/Main exceeded authored construction budget");
                    }
                }
                sims_vm::Stop::Sleeping | sims_vm::Stop::Returned(_) => {
                    sim.vm
                        .finish_object_update(part, true)
                        .map_err(anyhow::Error::msg)?;
                    break;
                }
                _ => bail!(
                    "group Init/Main requires an unsupported construction service: {:?}",
                    sim.vm.threads[&thread]
                ),
            }
        }
    }
    Ok(())
}

fn add_part(
    index: &Index,
    program: &mut Program,
    sim: &mut Simulation,
    selector: &str,
    definition: u16,
    id: ObjectId,
    body: Body,
    multitile: Option<sims_vm::multitile::Part>,
) -> Result<()> {
    if id <= 0 || sim.vm.world.objects.contains_key(&id) {
        bail!("invalid or occupied authored object ID {id}");
    }
    body.validate(&sim.room).map_err(anyhow::Error::msg)?;
    for (source, module) in load_program(index, selector, "Global.iff")?.modules {
        program.modules.entry(source).or_insert(module);
    }
    let doc = index.document(index.find(selector)?)?;
    let def = &doc
        .objects
        .iter()
        .find(|o| o.id == definition)
        .context("missing object definition")?
        .value;
    let slots = doc
        .iff
        .get(b"SLOT", def.slots)
        .map(|c| sims_formats::execution::Slots::parse(doc.iff.data(c)))
        .transpose()?
        .map_or(0, |s| s.entries.iter().filter(|s| s.kind == 0).count());
    let mut object = Object::construct(
        Definition {
            source: doc.source.clone(),
            object: definition,
            guid: def.guid,
            slots: def.slots,
        },
        program,
        slots,
    )
    .map_err(anyhow::Error::msg)?;
    let group = multitile.is_some();
    object.multitile = multitile;
    // Native post-construction allocates OBJD dynamic-sprite storage before Init.
    let sprites = program.modules[&doc.source].object_definition_words[&definition][0x82 / 2];
    object.dynamic_sprite_flags =
        vec![0; usize::try_from(sprites).context("negative sprite count")?];
    object
        .data
        .extend([(1, body.facing.trailing_zeros() as i16), (11, id), (29, 1)]);
    if let Some(order) = sim.vm.world.person.as_mut().and_then(|p| {
        p.resource_instances
            .get_mut(&object.definition.as_ref().unwrap().guid)
    }) {
        order.push(id);
    }
    sim.vm.world.objects.insert(id, object);
    let level = body.level;
    sim.bodies.insert(id, body);
    sim.vm
        .world
        .person
        .as_mut()
        .context("missing person world")?
        .object_order
        .push(id);
    sim.vm
        .world
        .refresh
        .as_mut()
        .context("missing drawing world")?
        .draw
        .insert(id, sims_vm::refresh::DrawState::constructed(level));
    if !group {
        sim.vm
            .reset_object_lifecycle(program, id, Default::default())
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

pub fn needs_room(
    index: &Index,
    selector: &str,
    definition: u16,
    motive: usize,
) -> Result<(Program, Simulation)> {
    if ![5, 6, 7, 8, 9, 13, 14, 15].contains(&motive) {
        bail!("invalid household motive index");
    }
    let mut p = load_program(index, "DiningChairs.iff", "Global.iff")?;
    p.accessories = crate::avatar::load_accessories(index)?
        .into_iter()
        .map(|(name, asset)| (name, asset.sources))
        .collect();
    load_household_animations(index, &mut p)?;
    if selector == "Fridges.iff" {
        for resource in ["Food.iff", "trashpile.iff"] {
            for (source, module) in load_program(index, resource, "Global.iff")?.modules {
                p.modules.entry(source).or_insert(module);
            }
        }
    }
    for (source, module) in load_program(index, "flood.iff", "Global.iff")?.modules {
        p.modules.entry(source).or_insert(module);
    }
    let fixture = ChairFixture::load(index, "DiningChairs.iff", 16807, ChairScenario::Empty)?;
    let mut s = initialized_two_chair_room(&p, fixture, 1)?;
    // Authored household starting funds; neighborhood/family import is separate.
    s.vm.world.budget = Some(sims_vm::budget::State {
        balance: 20000,
        ..Default::default()
    });
    // Original global 25 holds debug-display flags (Global 0x017b).
    s.vm.world
        .globals
        .extend([(13, 0), (14, 2), (25, 0), (32, 0)]);
    add_object(
        index,
        &mut p,
        &mut s,
        selector,
        definition,
        4,
        Body {
            level: 1,
            position: Point::new(88, 24),
            facing: 1,
            radius: 6,
            elevation: 0,
        },
    )?;
    crate::assets::load_linked_animations(index, &mut p)?;
    p.accessories = crate::avatar::load_linked_accessories(index, &p)?
        .into_iter()
        .map(|(n, a)| (n, a.sources))
        .collect();
    loop {
        let step = s.run_fixture_tick(&p, 1000).map_err(anyhow::Error::msg)?;
        if step.stop == StepStop::Faulted {
            bail!("needs object Init failed: {:?}", s.vm.threads);
        }
        if step.stop != StepStop::Budget {
            break;
        }
    }
    s.enable_household(
        &p,
        LotClock {
            hour: 8,
            minute: 0,
            second: 0,
            day: 1,
            month: 1,
            year: 1997,
            period: 0,
            elapsed_days: 0,
            month_days: 30,
            ticks: 0,
            statistics: [[0; 8]; 6],
        },
    )
    .map_err(anyhow::Error::msg)?;
    s.enable_residential_autonomy(
        s.vm.world
            .objects
            .keys()
            .copied()
            .filter(|id| *id != 1)
            .collect::<Vec<_>>(),
    )
    .map_err(anyhow::Error::msg)?;
    let motives =
        s.vm.world
            .objects
            .get_mut(&1)
            .unwrap()
            .motives
            .as_mut()
            .unwrap();
    for i in [5, 6, 7, 8, 9, 13, 14, 15] {
        motives[i] = 50f32.to_bits();
    }
    motives[motive] = (-30f32).to_bits();
    Ok((p, s))
}

/// Combined authored household. The object scripts and motive tuning are
/// original; layout and initial funds/motives are explicit scenario inputs.
pub fn autonomous_household(index: &Index) -> Result<(Program, Simulation)> {
    let (mut p, mut s) = needs_room(index, "Fridges.iff", 16807, 7)?;
    // Two initial deficits exercise competing original advertisements. The
    // remaining needs start at 50 and decay normally through the day.
    s.vm.world
        .objects
        .get_mut(&1)
        .unwrap()
        .motives
        .as_mut()
        .unwrap()[8] = (-30f32).to_bits();
    s.room.width = 192;
    s.room.height = 176;
    s.room.obstacles.clear();
    for (id, position) in [
        (2, Point::new(48, 64)),
        (3, Point::new(80, 64)),
        (4, Point::new(24, 24)),
    ] {
        s.bodies.get_mut(&id).unwrap().position = position;
    }
    for (selector, definition, id, position) in [
        ("Sinks.iff", 16807, 5, Point::new(72, 24)),
        ("Toilets.iff", 16807, 6, Point::new(120, 24)),
        ("Recliners.iff", 16807, 7, Point::new(120, 104)),
        ("ShowerC.iff", 16807, 9, Point::new(168, 24)),
        ("Bookcases.iff", 16811, 10, Point::new(24, 112)),
        ("trash1.iff", 16807, 11, Point::new(24, 152)),
        ("TrashOutside.iff", 16807, 12, Point::new(168, 152)),
    ] {
        add_object(
            index,
            &mut p,
            &mut s,
            selector,
            definition,
            id,
            Body {
                level: 1,
                position,
                facing: 1,
                radius: 6,
                elevation: 0,
            },
        )?;
    }
    // Rebuild the authored advertisement registry after constructing all objects,
    // before the first person update or autonomy query in this scenario.
    crate::assets::load_linked_animations(index, &mut p)?;
    p.accessories = crate::avatar::load_linked_accessories(index, &p)?
        .into_iter()
        .map(|(n, a)| (n, a.sources))
        .collect();
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    s.enable_residential_autonomy(2..=12)
        .map_err(anyhow::Error::msg)?;
    loop {
        let step = s.run_fixture_tick(&p, 1000).map_err(anyhow::Error::msg)?;
        if matches!(step.stop, StepStop::Faulted | StepStop::Animation) {
            bail!("household initialization stopped: {:?}", s.vm.threads);
        }
        if step.stop != StepStop::Budget {
            break;
        }
    }
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}

/// Two original adult Main loops in an authored, divided single-story home.
pub fn two_sim_home(index: &Index) -> Result<(Program, Simulation)> {
    let (mut p, mut s) = autonomous_household(index)?;
    s.room.width = 256;
    s.room.height = 224;
    for (id, x, y) in [
        (1, 88, 112),
        (2, 88, 88),
        (3, 120, 88),
        (4, 56, 56),
        (5, 200, 56),
        (6, 168, 56),
        (7, 168, 144),
        (8, 168, 160),
        (9, 200, 88),
        (10, 56, 136),
        (11, 56, 168),
        (12, 136, 208),
    ] {
        s.bodies.get_mut(&id).unwrap().position = Point::new(x, y);
    }
    add_resident(
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
    for (id, x, y, facing) in [(14, 136, 88, 64), (16, 136, 152, 64), (18, 104, 184, 1)] {
        add_object(
            index,
            &mut p,
            &mut s,
            "Doors.iff",
            16815,
            id,
            Body {
                level: 1,
                position: Point::new(x, y),
                facing,
                radius: 0,
                elevation: 0,
            },
        )?;
    }
    add_object(
        index,
        &mut p,
        &mut s,
        "Recliners.iff",
        16807,
        20,
        Body {
            level: 1,
            position: Point::new(200, 144),
            facing: 1,
            radius: 6,
            elevation: 0,
        },
    )?;
    let mut walls = vec![];
    for x in (32..224).step_by(16) {
        for y in [32, 192] {
            walls.push(sims_sim::architecture::Wall {
                a: Point::new(x, y),
                b: Point::new(x + 16, y),
                door: if x == 96 && y == 192 {
                    Some([18, 19])
                } else {
                    None
                },
            });
        }
    }
    for y in (32..192).step_by(16) {
        for x in [32, 144, 224] {
            walls.push(sims_sim::architecture::Wall {
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
        walls.push(sims_sim::architecture::Wall {
            a: Point::new(x, 112),
            b: Point::new(x + 16, 112),
            door: None,
        });
    }
    for resource in [
        "HDSI_Talk.iff",
        "SocialDRT.iff",
        "Destination.iff",
        "HDSI.iff",
        "dropdest.iff",
    ] {
        for (source, module) in load_program(index, resource, "Global.iff")?.modules {
            p.modules.entry(source).or_insert(module);
        }
    }
    s.set_architecture(&p, walls).map_err(anyhow::Error::msg)?;
    s.vm.world.person.as_mut().unwrap().autonomy = None;
    s.enable_residential_autonomy((2..=12).chain([20]))
        .map_err(anyhow::Error::msg)?;
    crate::assets::load_linked_animations(index, &mut p)?;
    p.accessories = crate::avatar::load_linked_accessories(index, &p)?
        .into_iter()
        .map(|(n, a)| (n, a.sources))
        .collect();
    sims_sim::Snapshot::capture(&p, &s)
        .restore(&p)
        .map_err(anyhow::Error::msg)?;
    Ok((p, s))
}
