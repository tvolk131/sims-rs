//! Run original-resource Init/Main in Rust and compare to original x86 execution.
use serde_json::{Value, json};
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, lifecycle_fixture, load_program},
};
use sims_formats::behavior::{Behavior, Instruction};
use sims_vm::{refresh::RoomState, *};
use std::path::Path;
fn main() -> anyhow::Result<()> {
    let report: Value = serde_json::from_slice(&std::fs::read(
        std::env::args().nth(1).expect("private report"),
    )?)?;
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let mut p = load_program(&index, "DiningChairs.iff", "Global.iff")?;
    for (s, m) in load_program(&index, r"People\TemplatePerson.iff", "Global.iff")?.modules {
        p.modules.entry(s).or_insert(m);
    }
    p.modules.insert(
        "harness".into(),
        Module {
            source: "harness".into(),
            routines: std::collections::BTreeMap::from([(
                4096,
                Routine {
                    name: "explicit external Notify".into(),
                    body: Behavior {
                        format: 0x8002,
                        kind: 0,
                        args: 0,
                        locals: 0,
                        instructions: vec![Instruction {
                            opcode: 49,
                            on_true: 254,
                            on_false: 253,
                            operands: [0; 8],
                        }],
                    },
                },
            )]),
            ..Module::default()
        },
    );
    let actor = ChairFixture::load(&index, "DiningChairs.iff", 16807, ChairScenario::Empty)?
        .world
        .objects[&1]
        .clone();
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let (_, mut w) = lifecycle_fixture(&index, "DiningChairs.iff", 16807)?;
        w.objects.insert(2, actor.clone());
        let o = w.objects.get_mut(&1).unwrap();
        o.data.insert(14, -1);
        o.data.insert(29, 1);
        w.objects.get_mut(&2).unwrap().motives.as_mut().unwrap()[6] =
            (c["initial"].as_f64().unwrap() as f32).to_bits();
        let r = w.refresh.as_mut().unwrap();
        r.visible_level = 1;
        r.render_clock = 1234;
        r.rooms.insert(1, RoomState::default());
        let mut v = Vm::new(w, 1);
        let id = v.start_object_lifecycle(&p, 1).unwrap();
        let run = v.run_slice(&p, id, 1000).unwrap();
        assert_eq!(run.stop, Stop::Budget);
        let data: Vec<i16> = (0..72)
            .map(|i| v.inspect_memory(&p, id, 3, i, false).unwrap().value)
            .collect();
        let r = v.world.refresh.as_ref().unwrap();
        let d = &r.draw[&1];
        let actual = json!({"data":data,"attributes":v.world.objects[&1].attributes,"draw":[i32::from(d.dynamic),d.expiry,d.refreshed_at],"room_dirty":u8::from(r.rooms[&1].dirty)});
        assert_eq!(actual, row["original"]["init"], "Init {c}");
        let mut replay = Snapshot::capture(&p, &v).restore(&p).unwrap();
        let mut changes = vec![];
        let mut executed = vec![];
        let mut previous = v.world.objects[&2].motives.unwrap()[6];
        for tick in 0..=260 {
            for state in [&mut v, &mut replay] {
                if tick > 0 {
                    state.advance_tick().unwrap();
                }
                if tick == 3 {
                    if c["occupied"] == true {
                        state.world.objects.get_mut(&1).unwrap().slots[0] = Some(2);
                        state.world.objects.get_mut(&2).unwrap().container = Some((1, 0));
                    }
                    let notify = state
                        .spawn(
                            &p,
                            Start {
                                owner: "harness",
                                bhav: 4096,
                                caller: 2,
                                callee: 2,
                                stack_object: 1,
                                args: vec![],
                                check: false,
                            },
                        )
                        .unwrap();
                    assert_eq!(
                        state.run_slice(&p, notify, 100).unwrap().stop,
                        Stop::Returned(true)
                    );
                }
                if tick == 190 {
                    state.world.objects.get_mut(&1).unwrap().slots[0] = None;
                    state.world.objects.get_mut(&2).unwrap().container = None;
                }
            }
            let a = v.run_slice(&p, id, 1000).unwrap();
            let b = replay.run_slice(&p, id, 1000).unwrap();
            assert_eq!(a, b);
            assert_eq!(a.stop, Stop::Sleeping, "{c} tick {tick}");
            assert_eq!(v, replay);
            if !a.trace.is_empty() {
                executed.push(tick);
            }
            let bits = v.world.objects[&2].motives.unwrap()[6];
            if bits != previous {
                changes.push(json!([tick, bits]));
                previous = bits;
            }
            if [1, 3, 62, 100, 189, 190, 243].contains(&tick) {
                replay = Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(&p, &v))?, &p)
                    .unwrap();
            }
        }
        assert_eq!(
            json!(changes),
            row["original"]["comfort_changes"],
            "comfort {c}"
        );
        assert_eq!(
            json!(executed),
            row["original"]["executed_ticks"],
            "scheduling {c}"
        );
        assert_eq!(json!(previous), row["original"]["final_bits"]);
    }
    println!(
        "{} original chair Init/Main timelines match; seven save/resume checkpoints per timeline",
        report["cases"].as_array().unwrap().len()
    );
    Ok(())
}
