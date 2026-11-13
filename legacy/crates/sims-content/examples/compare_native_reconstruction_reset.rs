//! Base virtual reset with original chair Init/Main. The native fixture supplies
//! virtual detach/reinsert boundaries; their implementations have separate
//! placement corpora. Graphics observers are recorded, not fulfilled here.
use serde_json::{Value, json};
use sims_content::{Index, vm::load_program};
use sims_sim::{
    architecture::Architecture,
    lot_geometry::{FloorSource, Lot},
    lot_links::Links,
    lot_placement::State,
    lot_reset::{Request, Reset},
};
use sims_vm::{
    autonomy::Placement,
    construction::ObjectDate,
    loading::SavedFrame,
    refresh::{DrawState, RefreshState},
    *,
};
use std::{collections::BTreeMap, path::Path};

fn frames(t: &Thread) -> Value {
    json!(
        t.stack
            .iter()
            .map(|f| json!({
                "bhav":f.location.bhav,"pc":f.location.instruction,
                "stack_object":f.stack_object,"args":f.args,"locals":f.locals,
                "offset":f.storage_offset
            }))
            .collect::<Vec<_>>()
    )
}
fn live_reset(
    p: &Program,
    vm: &Vm,
    architecture: &Architecture,
    placement: &State,
    now: ObjectDate,
    deleted: bool,
    budget: usize,
) -> anyhow::Result<(sims_sim::Simulation, usize)> {
    let mut vm = vm.clone();
    placement
        .links
        .publish_tile_chains(&mut vm.world)
        .map_err(anyhow::Error::msg)?;
    let native = sims_sim::lot_runtime::State {
        placement: placement.clone(),
        membership: sims_vm::person_loading::RoomMembership::constructed(&vm.world),
        indicators: Default::default(),
        observers: vec![],
        diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
            error: 0,
            blocker: None,
        },
        route: None,
        shoo: None,
        movement: None,
        completion: None,
        route_paths: Default::default(),
        snap: None,
        find_location: None,
        removal: None,
        snap_offsets: Default::default(),
        selected_person: None,
        room_effects: vec![],
        routing_policy: Default::default(),
    };
    let mut simulation = sims_sim::Simulation::from_loaded_lot(vm, architecture.clone(), native)
        .map_err(anyhow::Error::msg)?;
    let mut reset = sims_sim::lot_reset::live::Execution::new(1);
    let mut checkpoints = 0;
    loop {
        let mut step = sims_sim::Step {
            stop: sims_sim::StepStop::Budget,
            trace: vec![],
            events: vec![],
        };
        let done = reset
            .advance(&mut simulation, p, now, deleted, &mut { budget }, &mut step)
            .map_err(anyhow::Error::msg)?;
        reset.validate(&simulation, p).map_err(anyhow::Error::msg)?;
        // This controller is hosted externally by this component comparison.
        // Check its own callback bindings along with the complete saved state.
        let bytes = serde_json::to_vec(&(&reset, &simulation))?;
        let (restored_reset, restored_sim): (
            sims_sim::lot_reset::live::Execution,
            sims_sim::Simulation,
        ) = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(reset == restored_reset && simulation == restored_sim);
        sims_vm::Snapshot::capture(p, &restored_sim.vm)
            .restore(p)
            .map_err(anyhow::Error::msg)?;
        restored_reset
            .validate(&restored_sim, p)
            .map_err(anyhow::Error::msg)?;
        reset = restored_reset;
        simulation = restored_sim;
        checkpoints += 1;
        anyhow::ensure!(checkpoints < 1000);
        if done {
            break;
        }
    }
    Ok((simulation, checkpoints))
}
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
    let owner = p
        .modules
        .keys()
        .find(|s| s.ends_with("DiningChairs.iff"))
        .unwrap()
        .clone();
    let m = p.modules.get_mut(&owner).unwrap();
    m.object_definition_words.get_mut(&16807).unwrap()[0x62 / 2] = 321;
    let table = m.object_functions.get_mut(&16807).unwrap();
    // Same supplied function table as the native reset fixture: ordinary Init
    // and Main only, so this exercises the absent-function-3 reset branch.
    for (i, f) in table.iter_mut().enumerate() {
        f.action = match i {
            0 => 4108,
            1 => 4096,
            _ => 0,
        };
        f.condition = 0;
    }
    let words = &m.object_definition_words[&16807];
    let guid = u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16);
    let size = 8;
    let n = size * size;
    let source = FloorSource {
        coverings: vec![0; n],
        walls: vec![Default::default(); n],
        object_heads: vec![0; n],
        flags: vec![0; n],
    };
    let a = Architecture::from_original(
        Lot::rebuild(
            size,
            vec![[0; 4]; n],
            vec![255; n],
            vec![255; n],
            vec![source; 2],
        )
        .unwrap(),
        &p,
    )
    .unwrap();
    let mut live_checkpoints = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let e = &row["original"];
        let initial = &e["initial"];
        let mut w = World::default();
        w.person = Some(Default::default());
        w.globals.extend([(8, 15), (7, 3), (1, 10)]);
        let o = Object {
            definition: Some(Definition {
                source: owner.clone(),
                object: 16807,
                guid,
                slots: 0,
            }),
            data: initial["storage"]["data"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, x)| (i as i16, x.as_i64().unwrap() as i16))
                .collect(),
            attributes: serde_json::from_value(initial["storage"]["attributes"].clone())?,
            temps: serde_json::from_value(initial["storage"]["temps"].clone())?,
            idle: Some(idle::IdleState {
                countdown: 51,
                ..Default::default()
            }),
            ..Default::default()
        };
        w.objects.insert(1, o);
        let on_lot = c["on_lot"].as_bool().unwrap();
        let xy = if on_lot { 56 } else { -16 };
        w.positions.insert(
            1,
            Placement {
                x: xy,
                y: xy,
                level: 1,
            },
        );
        w.refresh = Some(RefreshState {
            visible_level: 1,
            render_clock: 1234,
            draw: BTreeMap::from([(1, DrawState::constructed(1))]),
            ..Default::default()
        });
        w.ui_owner.owner = c["owner"].as_bool().unwrap().then_some(1);
        a.publish_room_context(&mut w);
        let mut v = Vm::new(w, 1);
        let id = v
            .restore_object_stack(
                &p,
                1,
                &[SavedFrame {
                    owner: owner.clone(),
                    tree: 4096,
                    node: 0,
                    stack_object: 0,
                    args: vec![0; 4],
                    locals: vec![],
                    primitive_state: 0,
                }],
                4096,
                false,
            )
            .unwrap();
        v.threads.get_mut(&id).unwrap().last_result = c["previous"].as_bool().unwrap();
        assert_eq!(frames(&v.threads[&id]), initial["frames"]);
        let mut heads = vec![vec![0; n]; 2];
        if on_lot {
            heads[0][3 * size + 3] = 1;
        }
        let mut state = State::new(
            Links {
                size,
                heads,
                next: BTreeMap::from([(1, 0)]),
            },
            &a,
        )
        .unwrap();
        let mut reset = Reset::new(1);
        let mut observers = vec![];
        let now = ObjectDate {
            year: 15,
            month: 3,
            day: 10,
        };
        let live_initial = (v.clone(), state.clone());
        while let Some(request) = reset
            .advance(
                &mut v,
                &p,
                &a,
                &mut state,
                now,
                c["deleted"].as_bool().unwrap(),
            )
            .unwrap()
        {
            match request {
                Request::Placement(request) => observers.push(request),
                host => panic!("unexpected host request: {host:?}; {c}"),
            }
            reset = serde_json::from_value(json!(reset))?;
            state = serde_json::from_value(json!(state))?;
            v = Snapshot::capture(&p, &v).restore(&p).unwrap();
            assert!(observers.len() < 100);
        }
        let o = &v.world.objects[&1];
        let d = &v.world.refresh.as_ref().unwrap().draw[&1];
        assert_eq!(
            json!({"data":(0..72).map(|i|o.data[&i]).collect::<Vec<_>>(),
            "attributes":o.attributes,"temps":o.temps,
            "draw":[u32::from(d.dynamic),d.expiry as u32,d.refreshed_at]}),
            e["storage"],
            "{c}"
        );
        assert_eq!(frames(&v.threads[&id]), e["frames"], "{c}");
        assert_eq!(
            json!(o.idle.as_ref().unwrap().countdown as u16),
            e["scheduler"],
            "{c}"
        );
        assert_eq!(json!(v.threads[&id].last_result), e["last_result"], "{c}");
        assert_eq!(json!(v.world.ui_owner.owner.is_some()), e["owner"], "{c}");
        let xy = if on_lot && !c["deleted"].as_bool().unwrap() {
            56
        } else {
            -16
        };
        assert_eq!(
            v.world.positions[&1],
            Placement {
                x: xy,
                y: xy,
                level: 1
            }
        );
        assert_eq!(Snapshot::capture(&p, &v).restore(&p).unwrap(), v);
        let mut live_outcome = None;
        for budget in [1, 3, 100_000] {
            let (live, checkpoints) = live_reset(
                &p,
                &live_initial.0,
                &a,
                &live_initial.1,
                now,
                c["deleted"].as_bool().unwrap(),
                budget,
            )?;
            live_checkpoints += checkpoints;
            let actual = &live.vm.world.objects[&1];
            assert_eq!(actual, o, "live object differs {c}/{budget}");
            assert_eq!(
                live.vm.threads, v.threads,
                "live frames differ {c}/{budget}"
            );
            assert_eq!(
                live.vm.rng_state, v.rng_state,
                "live RNG differs {c}/{budget}"
            );
            assert_eq!(
                live.vm.lot_rng_state, v.lot_rng_state,
                "live lot RNG differs {c}/{budget}"
            );
            assert_eq!(live.vm.world.ui_owner, v.world.ui_owner);
            assert_eq!(live.vm.world.positions, v.world.positions);
            assert_eq!(live.native_lot.as_ref().unwrap().placement, state);
            if let Some(previous) = &live_outcome {
                assert_eq!(&live, previous, "budget changed live Reset {c}");
            }
            live_outcome = Some(live);
        }
    }
    println!(
        "{} native reconstruction reset cases match: original chair Init/Main, storage, frames, scheduler and UI ownership, with saved operation continuations",
        report["cases"].as_array().unwrap().len()
    );
    println!(
        "{live_checkpoints} live reconstruction checkpoints across budgets 1/3/100000 match the native-validated frozen execution"
    );
    Ok(())
}
