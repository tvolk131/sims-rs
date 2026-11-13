//! Compare original adult Main instruction prefixes through its first engine service.
use serde_json::{Value, json};
use sims_content::{Index, vm::*};
use sims_vm::*;
fn main() -> anyhow::Result<()> {
    let report: Value = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).expect("private report"),
    )?)?;
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let index = Index::scan(std::path::Path::new("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    p.modules
        .extend(load_program(&index, r"People\TemplatePerson.iff", "Global.iff")?.modules);
    let world: World = serde_json::from_slice(&std::fs::read(
        "cache/research/sims-exe/oracle/person-world.json",
    )?)?;
    let mut count = 0;
    let mut instructions = 0;
    for c in report["cases"].as_array().unwrap() {
        let mut v = Vm::new(world.clone(), c["seed"].as_u64().unwrap());
        v.world.objects.get_mut(&1).unwrap().idle = Some(Default::default());
        let id = v
            .start_supplied_person_main(&p, 1)
            .map_err(anyhow::Error::msg)?;
        if c["queued"] == true {
            let e = v.action_menu(&p, 2).map_err(anyhow::Error::msg)?[0].clone();
            v.enqueue_action(
                &p,
                1,
                action::Action {
                    target: 2,
                    icon_owner: 2,
                    menu_id: e.id,
                    routine: e.action,
                    priority: 100,
                    ..Default::default()
                },
            )
            .map_err(anyhow::Error::msg)?;
        }
        let mut trace = vec![];
        for _ in 0..1000 {
            // Rust exposes the synchronous eligibility call at diagnostic budget
            // boundaries. Resuming that already-entered Idle is not a second
            // bytecode entry in the original interpreter.
            let resumes_check =
                v.threads[&id].dispatch.is_some() && v.threads[&id].callback_result.is_some();
            let run = v.run_slice(&p, id, 1).map_err(anyhow::Error::msg)?;
            for t in run.trace.iter().filter(|_| !resumes_check) {
                let module = t.location.source.split("::").last().unwrap();
                trace.push(json!([
                    t.tick,
                    module,
                    t.location.bhav,
                    t.location.instruction,
                    t.depth
                ]));
            }
            match run.stop {
                Stop::Sleeping => v.advance_tick().map_err(anyhow::Error::msg)?,
                Stop::Budget => (),
                Stop::AwaitingAnimation | Stop::AwaitingRoute => break,
                other => panic!("{other:?}: {:?}", run.trace),
            }
        }
        let original = c["original"]["trace"].as_array().unwrap();
        for (i, (a, b)) in trace.iter().zip(original).enumerate() {
            assert_eq!(
                a, b,
                "instruction {i}, seed {}, queued {}",
                c["seed"], c["queued"]
            );
        }
        assert_eq!(trace.len(), original.len(), "{c}");
        assert_eq!(
            json!(v.world.objects[&1].attributes),
            c["original"]["attributes"]
        );
        assert_eq!(
            json!(
                (0..72)
                    .map(|i| v.world.objects[&1].data[&i])
                    .collect::<Vec<_>>()
            ),
            c["original"]["data"]
        );
        assert_eq!(json!(v.rng_state), c["original"]["rng"]);
        assert_eq!(json!(v.world.objects[&1].temps), c["original"]["temps"]);
        assert_eq!(
            json!(
                (0..256)
                    .map(|i| v.world.objects[&1].person.as_ref().unwrap()[&i])
                    .collect::<Vec<_>>()
            ),
            c["original"]["person"]
        );
        instructions += trace.len();
        count += 1;
    }
    let mut init_count = 0;
    for c in report["inits"].as_array().into_iter().flatten() {
        let mut v = Vm::new(world.clone(), c["seed"].as_u64().unwrap());
        v.world
            .objects
            .get_mut(&1)
            .unwrap()
            .person
            .as_mut()
            .unwrap()
            .insert(56, c["field56"].as_i64().unwrap() as i16);
        let id = v
            .start_object_lifecycle(&p, 1)
            .map_err(anyhow::Error::msg)?;
        let mut trace = vec![];
        while v.threads[&id].callback.is_some() {
            let run = v.run_slice(&p, id, 1).map_err(anyhow::Error::msg)?;
            assert_eq!(run.stop, Stop::Budget, "{:?}", run.trace);
            for t in run.trace {
                trace.push(json!([
                    t.tick,
                    t.location.source.split("::").last().unwrap(),
                    t.location.bhav,
                    t.location.instruction,
                    t.depth
                ]));
            }
            assert!(trace.len() < 1000);
            v = Snapshot::capture(&p, &v)
                .restore(&p)
                .map_err(anyhow::Error::msg)?;
        }
        let o = &c["original"];
        assert_eq!(
            json!(trace),
            o["trace"],
            "Init seed {} field56 {}",
            c["seed"],
            c["field56"]
        );
        let a = &v.world.objects[&1];
        assert_eq!(
            json!((0..72).map(|i| a.data[&i]).collect::<Vec<_>>()),
            o["data"]
        );
        assert_eq!(
            json!(
                (0..256)
                    .map(|i| a.person.as_ref().unwrap()[&i])
                    .collect::<Vec<_>>()
            ),
            o["person"]
        );
        assert_eq!(json!(a.motives.unwrap()), o["motives"]);
        assert_eq!(json!(a.attributes), o["attributes"]);
        assert_eq!(json!(a.temps), o["temps"]);
        assert_eq!(json!(v.rng_state), o["rng"]);
        init_count += 1;
    }
    println!("{init_count} original person Init traces and complete storage/RNG match");
    println!(
        "{count} original adult Main prefixes match ({instructions} instruction positions, person state, temporaries and RNG)"
    );
    Ok(())
}
