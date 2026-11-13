//! Differential base construction/reset and every executable chair Init frame.
use serde_json::{Value, json};
use sims_content::{Index, vm::load_program};
use sims_vm::{construction::ObjectDate, refresh::DrawState, *};
use std::{collections::BTreeMap, path::Path};
fn frames(t: &Thread) -> Value {
    let mut frames = vec![];
    for (i, f) in t.stack.iter().enumerate() {
        if t.callback.as_ref().is_some_and(|b| b.base_depth == i) {
            frames.push(json!({"bhav":65535,"pc":0,"stack_object":0,"args":[],"locals":[],"offset":f.storage_offset-8}));
        }
        frames.push(json!({"bhav":f.location.bhav,"pc":f.location.instruction,"stack_object":f.stack_object,"args":f.args,"locals":f.locals,"offset":f.storage_offset}));
    }
    json!(frames)
}
fn storage(v: &Vm) -> Value {
    let o = &v.world.objects[&1];
    let d = &v.world.refresh.as_ref().unwrap().draw[&1];
    json!({"data":(0..72).map(|i|o.data[&i]).collect::<Vec<_>>(),"attributes":o.attributes,"temps":o.temps,"draw":[u32::from(d.dynamic),d.expiry as u32,d.refreshed_at]})
}
fn main() -> anyhow::Result<()> {
    let report: Value = serde_json::from_slice(&std::fs::read(
        "cache/research/sims-exe/oracle/food-construction.json",
    )?)?;
    let index = Index::scan(Path::new("game-data/cabinet-extracted"))?;
    let p = load_program(&index, "Food.iff", "Global.iff")?;
    let owner = p
        .modules
        .keys()
        .find(|s| s.ends_with("::Food.iff"))
        .unwrap()
        .clone();
    let mut comparisons = 0;
    for row in report["cases"].as_array().unwrap() {
        let c = &row["input"];
        let expected = &row["original"];
        let mut o = Object::construct(
            Definition {
                source: owner.clone(),
                object: 16808,
                guid: 0xf301e375,
                slots: 128,
            },
            &p,
            1,
        )
        .unwrap();
        o.data.extend([
            (1, c["direction"].as_i64().unwrap() as i16),
            (11, 1),
            (14, -1),
            (29, 1),
            (45, 1997),
            (46, 1),
            (47, 1),
        ]);
        let mut w = World::default();
        w.objects.insert(1, o);
        w.refresh = Some(refresh::RefreshState {
            visible_level: 1,
            render_clock: 1234,
            draw: BTreeMap::from([(1, DrawState::constructed(1))]),
            rooms: BTreeMap::from([(1, Default::default())]),
            ..Default::default()
        });
        let mut v = Vm::new(w, 1);
        let id = v
            .reset_object_lifecycle(
                &p,
                1,
                ObjectDate {
                    year: 1997,
                    month: 1,
                    day: 1,
                },
            )
            .unwrap();
        for checkpoint in expected["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["pc"].as_u64().unwrap() < 254)
        {
            assert_eq!(
                frames(&v.threads[&id]),
                checkpoint["frames"],
                "{c}; {checkpoint}"
            );
            let mut resumed = Snapshot::capture(&p, &v).restore(&p).unwrap();
            assert_eq!(
                v.run_slice(&p, id, 1).unwrap(),
                resumed.run_slice(&p, id, 1).unwrap()
            );
            assert_eq!(v, resumed);
            comparisons += 1;
        }
        if c["update"] == 1 {
            v.finish_object_update(1, true).unwrap();
        }
        assert_eq!(storage(&v), expected["storage"], "{c}");
        assert_eq!(frames(&v.threads[&id]), expected["frames"], "{c}");
        Snapshot::capture(&p, &v).restore(&p).unwrap();
    }
    println!("8 original Food Init/reset(1) cases and {comparisons} frame/save checkpoints match");
    Ok(())
}
