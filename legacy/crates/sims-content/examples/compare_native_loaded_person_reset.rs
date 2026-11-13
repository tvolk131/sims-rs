//! Offline reset comparison with the same explicitly bounded animator/lot profile.
use serde_json::{Value, json};
use sims_content::{Index, vm::load_program};
use sims_vm::*;
use std::collections::VecDeque;
fn frames(t: &Thread) -> Value {
    let mut result = vec![];
    for (i, f) in t.stack.iter().enumerate() {
        if t.callback.as_ref().is_some_and(|c| c.base_depth == i) {
            result.push(json!({"bhav":65535,"pc":0,"stack_object":0,"args":[],"locals":[],"offset":f.storage_offset-8}));
        }
        result.push(json!({"bhav":f.location.bhav,"pc":f.location.instruction,"stack_object":f.stack_object,"args":f.args,"locals":f.locals,"offset":f.storage_offset}));
    }
    json!(result)
}
fn words(value: &Value) -> Vec<i16> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as i16)
        .collect()
}
fn main() -> anyhow::Result<()> {
    let root = std::path::Path::new(".");
    let services_report = std::env::args().any(|a| a == "--services");
    let notification_report = std::env::args().any(|a| a == "--notifications");
    let active_report =
        notification_report || services_report || std::env::args().any(|a| a == "--active");
    let report: Value =
        serde_json::from_slice(&std::fs::read(root.join(if notification_report {
            "cache/research/sims-exe/oracle/person-reset-notification.json"
        } else if services_report {
            "cache/research/sims-exe/oracle/person-reset-services.json"
        } else if active_report {
            "cache/research/sims-exe/oracle/active-person-reset.json"
        } else {
            "cache/research/sims-exe/oracle/loaded-person-reset.json"
        }))?)?;
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    p.modules
        .extend(load_program(&index, r"People\TemplatePerson.iff", "Global.iff")?.modules);
    let mut world = World::default();
    world.person = Some(person::WorldState {
        object_order: vec![1, 2],
        person_order: vec![1],
        ..Default::default()
    });
    world.actions = Some(action::WorldState {
        serial: 99,
        ..Default::default()
    });
    for (id, suffix, number, person) in [
        (1, "TemplatePerson.iff", 128, true),
        (2, "DiningChairs.iff", 16807, false),
    ] {
        let source = p
            .modules
            .keys()
            .find(|s| s.ends_with(suffix))
            .unwrap()
            .clone();
        let words = &p.modules[&source].object_definition_words[&number];
        let guid = u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16);
        let definition = Definition {
            source,
            object: number,
            guid,
            slots: 0,
        };
        let object = if person {
            Object::construct_person(definition, &p, 5)
        } else {
            Object::construct(definition, &p, 0)
        }
        .map_err(anyhow::Error::msg)?;
        world.objects.insert(id, object);
    }
    let mut instructions = 0;
    let mut compared = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        if services_report && c["flags"].as_u64().unwrap() & 0x800 != 0 {
            // Guarded entry retains synthetic suspended trees. The VM fixture
            // checks those exactly; this resource comparison checks the actual
            // replacement Main/Reset and their instruction checkpoints.
            continue;
        }
        let original = &row["original"];
        if notification_report && original["before_animator"].is_null() {
            // No Main/Reset execution in these cases. The notification VM
            // comparison checks retained frames and reference cleanup directly.
            continue;
        }
        let before = &original[if active_report {
            "storage_before"
        } else {
            "before"
        }];
        let expected = &original[if active_report {
            "storage_after"
        } else {
            "after"
        }];
        if !active_report {
            assert_eq!(
                before["previous_record"], expected["previous_record"],
                "native previous record changed: {c}"
            );
        }
        // The freshly constructed empty record has native string storage and
        // table-index -1. Rust represents target-null activity as None; it does
        // not preserve those unexposed empty-record bytes.
        assert_eq!(expected["active"], 0, "native action remains active: {c}");
        let mut w = world.clone();
        let o = w.objects.get_mut(&1).unwrap();
        o.person = Some(
            words(&before["person"])
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i as i16, v))
                .collect(),
        );
        o.data = words(&before["data"])
            .into_iter()
            .enumerate()
            .map(|(i, v)| (i as i16, v))
            .collect();
        o.attributes = words(&before["attributes"]);
        o.temps = words(&before["temps"]).try_into().unwrap();
        o.motives = Some(
            before["motives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        );
        o.container = None;
        o.slots = vec![None; 5];
        o.idle = Some(Default::default());
        let previous = action::Action {
            id: 99,
            target: 2,
            icon_owner: 2,
            priority: 77,
            routine: 4097,
            ..Default::default()
        };
        let actor = w.actions.as_mut().unwrap().actors.entry(1).or_default();
        actor.active = None;
        actor.previous = Some(previous.clone());
        actor.queue.pending = (0..c["queued"].as_u64().unwrap())
            .map(|i| action::Action {
                id: i as u32 + 1,
                target: 2,
                icon_owner: 2,
                priority: 90 + i as i32,
                routine: 4097,
                ..Default::default()
            })
            .collect::<VecDeque<_>>();
        let mut v = Vm::new(w, c["seed"].as_u64().unwrap_or(0x51a5cafe));
        let thread = v
            .start_supplied_person_main(&p, 1)
            .map_err(anyhow::Error::msg)?;
        v.threads.get_mut(&thread).unwrap().last_result = c["previous_result"].as_bool().unwrap();
        assert_eq!(frames(&v.threads[&thread]), before["frames"]);
        if active_report {
            let mut third = v.world.objects[&2].clone();
            third.data.insert(62, 20);
            v.world.objects.insert(3, third);
            v.world.objects.get_mut(&2).unwrap().data.insert(62, 10);
            if c["active"] == true {
                let actor = v
                    .world
                    .actions
                    .as_mut()
                    .unwrap()
                    .actors
                    .get_mut(&1)
                    .unwrap();
                actor.active = Some(action::Action {
                    id: 71,
                    priority: 100,
                    flags: 12,
                    ..previous.clone()
                });
            }
            let t = v.threads.get_mut(&thread).unwrap();
            for i in 1..original["before"]["depth"].as_u64().unwrap() as usize {
                let mut f = t.stack[0].clone();
                f.location.bhav = 4097;
                f.location.instruction = 3;
                f.stack_object = 2;
                f.continuation = 1;
                f.storage_offset = i * 32;
                f.args = vec![11 + i as i16, 21 + i as i16];
                f.locals = vec![31 + i as i16, 41 + i as i16];
                if let Some(u) = original["before"]["uses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|u| u[1] == i + 1)
                {
                    f.action_target = Some(u[0].as_i64().unwrap() as i16);
                    f.action_icon = u[2] == 1;
                }
                t.stack.push(f);
            }
        }
        let mut reset = if notification_report {
            let actor = v
                .world
                .actions
                .as_mut()
                .unwrap()
                .actors
                .get_mut(&1)
                .unwrap();
            actor.set_icon_owner(c["icon"].as_i64().unwrap() as i16);
            actor.previous.as_mut().unwrap().target = c["previous_target"].as_i64().unwrap() as i16;
            actor.previous.as_mut().unwrap().icon_owner =
                c["previous_icon"].as_i64().unwrap() as i16;
            for (a, entry) in actor
                .queue
                .pending
                .iter_mut()
                .zip(c["queue"].as_array().unwrap())
            {
                a.target = entry[0].as_i64().unwrap() as i16;
                a.icon_owner = entry[1].as_i64().unwrap() as i16;
            }
            person_reset::Reset::notified(
                1,
                c["flags"].as_u64().unwrap() as u32,
                c["notified"].as_i64().unwrap() as i16,
                c["target_flags"].as_u64().unwrap() as u32,
            )
        } else {
            person_reset::Reset::new(1, c["flags"].as_u64().unwrap() as u32)
        };
        let mut cancelled = vec![];
        let mut events = vec![];
        let mut retargeted = false;
        let mut injected = false;
        while let Some(request) = reset.next(&mut v, &p).map_err(anyhow::Error::msg)? {
            use person_reset::Request;
            match request {
                Request::InterruptAnimation { .. } | Request::InterruptRoute { .. } => {
                    panic!("native fixture has no animation host wait")
                }
                Request::NotifyUsed(id) => events.push(json!(["notify_used", id])),
                Request::ResetUsed(id) => {
                    events.push(json!(["reset_used", id]));
                    if c["retarget"] == true && !retargeted {
                        let f = v
                            .threads
                            .get_mut(&thread)
                            .unwrap()
                            .stack
                            .iter_mut()
                            .rev()
                            .find(|f| f.action_target.is_some())
                            .unwrap();
                        assert_eq!(f.action_target, Some(2));
                        f.action_target = Some(3);
                        *v.world
                            .objects
                            .get_mut(&2)
                            .unwrap()
                            .data
                            .get_mut(&62)
                            .unwrap() -= 1;
                        *v.world
                            .objects
                            .get_mut(&3)
                            .unwrap()
                            .data
                            .get_mut(&62)
                            .unwrap() += 1;
                        retargeted = true;
                    }
                }
                Request::ClearAnimator(id) => events.push(json!([
                    "animator",
                    "0x589220",
                    v.world.objects[&id].person.as_ref().unwrap()[&30]
                ])),
                Request::ReloadAppearance(id) => events.push(json!([
                    "animator",
                    "0x5888d3",
                    v.world.objects[&id].person.as_ref().unwrap()[&30]
                ])),
                Request::ClearRouting(_) | Request::ClearActionIcons(_) => (),
                Request::Placement {
                    marked_for_deletion,
                    ..
                } => {
                    // Both native profiles are uncontained. The off-lot case
                    // is deletion-marked, which must suppress relocation.
                    if c["off_lot"] == true {
                        assert!(marked_for_deletion);
                    }
                }
                Request::Script(id) => {
                    events.extend([json!(["main", 4096]), json!(["callback", 1, 3, 0])]);
                    for checkpoint in
                        original["steps"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|frames| {
                                let f = frames.as_array().unwrap().last().unwrap();
                                f["pc"].as_u64().unwrap() < 254 && f["bhav"] != 65535
                            })
                    {
                        assert_eq!(frames(&v.threads[&id]), *checkpoint, "{c}");
                        v.run_slice(&p, id, 1).map_err(anyhow::Error::msg)?;
                        assert!(
                            !matches!(v.threads[&id].status, Status::Faulted(_)),
                            "{:?}",
                            v.threads[&id].status
                        );
                        instructions += 1;
                    }
                    assert!(v.threads[&id].callback.is_none());
                    events.push(json!(["missing_resource", 0x3929aadc_u32]));
                }
                Request::Cancel(callback) => {
                    events.push(json!([
                        "callback",
                        callback.object,
                        callback.function,
                        callback.stack_object
                    ]));
                    cancelled.push(callback.object);
                    if c["append_during_cancel"] == true && !injected {
                        v.world
                            .actions
                            .as_mut()
                            .unwrap()
                            .actors
                            .get_mut(&1)
                            .unwrap()
                            .queue
                            .pending
                            .push_back(action::Action {
                                id: 777,
                                target: 3,
                                icon_owner: 2,
                                routine: 4097,
                                priority: 123,
                                ..Default::default()
                            });
                        injected = true;
                    }
                }
                other => anyhow::bail!("unexpected native request: {other:?}"),
            }
            // This serializes the host continuation; VM frame snapshots remain
            // separately checked at the end and in the shared callback tests.
            reset = serde_json::from_value(json!(reset))?;
        }
        let native_events: Vec<_> = original["order"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] != "action")
            .cloned()
            .collect();
        assert_eq!(json!(events), json!(native_events), "{c}");
        let o = &v.world.objects[&1];
        let t = &v.threads[&thread];
        for (key, value) in [
            (
                "person",
                json!(
                    (0..256)
                        .map(|i| o.person.as_ref().unwrap()[&i])
                        .collect::<Vec<_>>()
                ),
            ),
            (
                "data",
                json!((0..72).map(|i| o.data[&i]).collect::<Vec<_>>()),
            ),
            ("temps", json!(o.temps)),
            ("attributes", json!(o.attributes)),
            ("motives", json!(o.motives.unwrap())),
            ("frames", frames(t)),
            ("rng", json!(v.rng_state)),
            ("last_result", json!(t.last_result)),
            ("flags", json!(reset.native_flags())),
        ] {
            assert_eq!(value, expected[key], "{c} {key}");
        }
        if active_report {
            let a = &v.world.actions.as_ref().unwrap().actors[&1];
            assert!(a.active.is_none());
            assert_eq!(
                json!(a.previous.as_ref().map_or(0, |a| a.id)),
                original["after"]["previous"],
                "{c}"
            );
            assert_eq!(
                json!(a.previous.as_ref().map_or(0, |a| a.flags)),
                original["after"]["previous_flags"],
                "{c}"
            );
            assert_eq!(
                json!([v.world.objects[&2].data[&62], v.world.objects[&3].data[&62]]),
                original["after"]["counts"],
                "{c}"
            );
        } else {
            assert_eq!(
                v.world.actions.as_ref().unwrap().actors[&1].previous,
                Some(previous)
            );
        }
        assert_eq!(
            v.world.actions.as_ref().unwrap().actors[&1]
                .queue
                .pending
                .len(),
            (expected["queue"][1].as_u64().unwrap() - expected["queue"][0].as_u64().unwrap())
                as usize
        );
        Snapshot::capture(&p, &v)
            .restore(&p)
            .map_err(anyhow::Error::msg)?;
        compared += 1;
    }
    println!(
        "{} original person resets and {instructions} frame checkpoints match",
        compared
    );
    Ok(())
}
