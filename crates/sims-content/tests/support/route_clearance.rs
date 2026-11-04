//! Read-only experiment on a clone of the imported household at a real route
//! boundary. Supplied process diagnostics and ignored-object binding are
//! explicit; this does not activate the route or substitute a successful path.
use serde_json::{Value, json};
use sims_sim::{
    Simulation,
    lot_acceptance::{self, person},
    lot_route::candidate::{Check, Input, Reply, Request},
    routing::PointCandidate,
};
use sims_vm::{Program, RouteRequest, Status};

pub fn inspect(p: &Program, s: &Simulation, r: &RouteRequest) -> Result<Value, String> {
    let native = s.native_lot.as_ref().ok_or("native state absent")?;
    let architecture = s.architecture.as_ref().ok_or("architecture absent")?;
    let lot = architecture
        .original
        .as_ref()
        .ok_or("original arrays absent")?;
    let (slot, version) = r.destination.slot()?;
    if !(4..=10).contains(&version) || slot.kind != 3 || slot.offset[2] != 0. {
        return Err("unsupported diagnostic point SLOT".into());
    }
    let target = s.vm.world.positions[&r.target];
    let actor = s.vm.world.positions[&r.caller];
    let point = PointCandidate::prepare(
        [target.x, target.y],
        [slot.offset[0], slot.offset[1]],
        s.vm.world.objects[&r.target].data[&1] as u8,
        slot.flags,
    )?;
    let input = Input {
        size: lot.size,
        position: point.position,
        target: [target.x, target.y],
        level: actor.level,
        room: s.vm.world.objects[&r.caller].data[&29],
        flags: slot.flags,
        direction: point.direction,
        ignored: None,
    };
    let mut vm = s.vm.clone();
    vm.world
        .objects
        .get_mut(&r.caller)
        .unwrap()
        .data
        .insert(1, i16::from(point.actor_facing));
    let mut check = Check::new(
        input.clone(),
        person::Diagnostics {
            error: 91,
            blocker: Some(r.target),
        },
        77,
    )?;
    let mut services = vec![];
    let mut callback_instructions = 0;
    while let Some(request) = check.next() {
        let checkpoint = serde_json::to_vec(&check).map_err(|e| e.to_string())?;
        let restored: Check = serde_json::from_slice(&checkpoint).map_err(|e| e.to_string())?;
        if restored != check {
            return Err("clearance checkpoint mismatch".into());
        }
        if let Some(reply) = Check::world_reply(request, &vm.world, &native.placement.links, lot)? {
            services.push(json!({"request":request,"reply":reply}));
            check.resume(request, reply)?;
            continue;
        }
        let Request::Placement { position, level } = request else {
            unreachable!()
        };
        let input = lot_acceptance::Input::for_person(
            &vm.world,
            &native.placement,
            architecture,
            r.caller,
            position,
            level,
        )?;
        let mut diagnostics = check.diagnostics();
        let mut floor = lot_acceptance::Floor::new(input, diagnostics.error)?;
        let mut collision_events = vec![];
        while let Some(service) = floor.next() {
            if service != lot_acceptance::Request::Collision {
                return Err(format!(
                    "route clearance floor service required: {service:?}"
                ));
            }
            let guid = vm.world.objects[&r.caller]
                .definition
                .as_ref()
                .unwrap()
                .guid;
            let insets = architecture.footprints[&guid].insets;
            let mut collision = person::Collision::begin(
                &mut vm.world,
                &native.placement.links,
                r.caller,
                position,
                level,
                insets,
                diagnostics,
            )?;
            while let Some(service) =
                collision.next(&vm.world, &native.placement.links, &native.placement.bounds)?
            {
                let reply = match service {
                    person::Request::WallObjects { .. } => {
                        return Err(
                            "person clearance unexpectedly requested wall-object comparison".into(),
                        );
                    }
                    person::Request::Walls {
                        bounds: [max_x, max_y, min_x, min_y],
                        level,
                    } => person::Reply::Walls(lot.wall_grid(level)?.blocks_bounds(
                        sims_sim::Rect {
                            min: sims_sim::Point::new(min_y, min_x),
                            max: sims_sim::Point::new(max_y, max_x),
                        },
                    )?),
                    person::Request::Function { object, other } => {
                        let thread = vm.object_threads[&object];
                        let ordinary = vm.threads[&thread].clone();
                        let started = vm.begin_object_function(p, object, 5, other)?;
                        if let Some(thread) = started {
                            while vm.threads[&thread].callback.is_some() {
                                if callback_instructions >= 10000 {
                                    return Err(
                                        "clearance callback diagnostic budget exhausted".into()
                                    );
                                }
                                if vm.threads[&thread].status != Status::Ready {
                                    return Err(format!(
                                        "clearance callback host required: {:?}",
                                        vm.threads[&thread].status
                                    ));
                                }
                                let mut resumed = sims_vm::Snapshot::capture(p, &vm).restore(p)?;
                                let step = vm.run_slice(p, thread, 1)?;
                                if resumed.run_slice(p, thread, 1)? != step || resumed != vm {
                                    return Err("clearance callback resume divergence".into());
                                }
                                callback_instructions += step.trace.len();
                            }
                        }
                        let after = &vm.threads[&thread];
                        if after.status != ordinary.status
                            || after.stack != ordinary.stack
                            || after.last_result != ordinary.last_result
                        {
                            return Err("collision callback changed ordinary execution".into());
                        }
                        person::Reply::Function {
                            allowed: started.is_some() && after.callback_result == Some(true),
                            diagnostics,
                        }
                    }
                };
                collision_events.push(json!({"request":service,"reply":reply}));
                collision.resume(service, reply)?;
            }
            let (accepted, d) = collision.outcome().ok_or("collision result absent")?;
            diagnostics = d;
            floor.resume(
                service,
                lot_acceptance::Reply::Geometry(lot_acceptance::Outcome {
                    accepted,
                    error: d.error,
                }),
            )?;
        }
        let outcome = floor.outcome().ok_or("floor outcome absent")?;
        let blocker = diagnostics.blocker.map(|id| {
            let d = vm.world.objects[&id].definition.as_ref().unwrap();
            (
                id,
                p.modules[&d.source].object_definition_words[&d.object][9],
            )
        });
        let reply = Reply::Placement(sims_sim::spatial::PlacementResult {
            allowed: outcome.accepted,
            error: outcome.error,
            blocker,
        });
        services.push(json!({"request":request,"reply":reply,"collision":collision_events}));
        check.resume(request, reply)?;
    }
    let outcome = check.outcome().ok_or("route classification absent")?;
    Ok(
        json!({"request":r,"point":point,"input":input,"services":services,
        "outcome":outcome,"candidates":point.finish(outcome.classification, 0)?,
        "callback_instructions":callback_instructions,
        "boundary":"cloned prefix; initial error 91/blocker target/context blocker 77 and ignored=None supplied; no path search or movement"}),
    )
}
