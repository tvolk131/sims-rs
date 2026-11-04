//!Original Cursors.iff bytecode on newly registered cXCursorObject storage.
//!This exercises the isolated constructor/reset projection, not4215e6's full
//!pickup/permission/placement factory or an original native process.
use anyhow::{Result, ensure};
use sims_content::{Index, vm::load_program};
use sims_vm::{
    cursor::reset::{Borrowed, Call, Request},
    *,
};
use std::path::PathBuf;

fn cold(p: &Program, v: &Vm) -> Result<Vm> {
    Snapshot::from_json(&serde_json::to_vec(&Snapshot::capture(p, v))?, p)
        .map_err(anyhow::Error::msg)
}
fn init(
    p: &Program,
    v: &mut Vm,
    t: ThreadId,
    resume: bool,
    instructions: &mut usize,
) -> Result<()> {
    for _ in 0..1000 {
        if v.threads[&t].callback.is_none() {
            return Ok(());
        }
        ensure!(v.threads[&t].status == Status::Ready);
        *instructions += v
            .run_slice(p, t, 1)
            .map_err(anyhow::Error::msg)?
            .trace
            .len();
        if resume {
            *v = cold(p, v)?;
        }
    }
    anyhow::bail!("original cursor Init exceeded diagnostic budget")
}
fn update(p: &Program, v: &mut Vm, id: i16, instructions: &mut usize) -> Result<()> {
    v.begin_scheduled_object_visit(id)
        .map_err(anyhow::Error::msg)?;
    let t = v.object_threads[&id];
    let r = v.run_slice(p, t, 1000).map_err(anyhow::Error::msg)?;
    ensure!(r.stop != Stop::Budget);
    *instructions += r.trace.len();
    v.finish_object_update(id, true)
        .map_err(anyhow::Error::msg)?;
    v.end_scheduled_object_visit().map_err(anyhow::Error::msg)?;
    Ok(())
}
#[test]
#[ignore = "requires private original Cursors.iff and Global.iff"]
fn original_cursor_constructor_and_two_init_passes_cold_resume() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let p = load_program(&index, "Cursors.iff", "Global.iff")?;
    let m = &p.modules[&index.resources[index.find("Cursors.iff")?].locator];
    let words = &m.object_definition_words[&128];
    ensure!(words[10] == 0);
    let d = Definition {
        source: m.source.clone(),
        object: 128,
        guid: words[14] as u16 as u32 | ((words[15] as u16 as u32) << 16),
        slots: words[20] as u16,
    };
    let mut reports = vec![];
    for (count, selected, updates) in [(1, 1, 0), (2, 1, 1), (2, 2, 1)] {
        let mut initial = Vm::new(
            World {
                person: Some(Default::default()),
                refresh: Some(Default::default()),
                scheduler_storage: Some(Default::default()),
                ..Default::default()
            },
            1,
        );
        for id in 1..=count {
            initial
                .register_cursor_storage(&p, id, d.clone(), None, (id != 1).then_some(1), None)
                .map_err(anyhow::Error::msg)?;
            initial = cold(&p, &initial)?;
            initial
                .finish_cursor_postconstruction(&p, id)
                .map_err(anyhow::Error::msg)?;
            initial = cold(&p, &initial)?;
        }
        let mut runs = vec![];
        for resume in [false, true] {
            let mut v = initial.clone();
            let mut call = Call::new(
                v.world
                    .scheduler_storage
                    .as_ref()
                    .unwrap()
                    .handle(selected)
                    .unwrap(),
                updates,
            )
            .map_err(anyhow::Error::msg)?;
            let mut instructions = 0;
            let mut init_calls = 0;
            let mut updates_run = 0;
            let mut requests = vec![];
            for _ in 0..100 {
                let Some(r) = call
                    .advance(&mut Borrowed {
                        world: &mut v.world,
                    })
                    .map_err(anyhow::Error::msg)?
                else {
                    break;
                };
                requests.push(r);
                if resume {
                    v = cold(&p, &v)?;
                    call = serde_json::from_slice(&serde_json::to_vec(&call)?)?;
                }
                let id = |v: &Vm, h| {
                    v.world
                        .scheduler_storage
                        .as_ref()
                        .unwrap()
                        .object_for_handle(h)
                        .unwrap()
                };
                match r {
                    Request::BaseReset {
                        object,
                        update: run_update,
                    } => {
                        let object = id(&v, object);
                        let t = v
                            .begin_cursor_base_reset(&p, object, Default::default())
                            .map_err(anyhow::Error::msg)?;
                        ensure!(v.lifecycles[&object].run_init);
                        init(&p, &mut v, t, resume, &mut instructions)?;
                        init_calls += 1;
                        if run_update != 0 {
                            update(&p, &mut v, object, &mut instructions)?;
                            updates_run += 1;
                        }
                        v.refresh_registry_flags(&p, object)
                            .map_err(anyhow::Error::msg)?;
                    }
                    Request::Offsets(h) => {
                        let id = id(&v, h);
                        cursor::refresh_offsets(&mut v.world, &p, id)
                            .map_err(anyhow::Error::msg)?;
                    }
                    Request::Init(h) => {
                        let id = id(&v, h);
                        let t = v
                            .begin_loaded_function(&p, id, 0)
                            .map_err(anyhow::Error::msg)?
                            .unwrap();
                        init(&p, &mut v, t, resume, &mut instructions)?;
                        init_calls += 1;
                    }
                    Request::Update(h) => {
                        let id = id(&v, h);
                        update(&p, &mut v, id, &mut instructions)?;
                        updates_run += 1;
                    }
                    Request::RegistryFlags(h) => {
                        let id = id(&v, h);
                        v.refresh_registry_flags(&p, id)
                            .map_err(anyhow::Error::msg)?;
                    }
                    _ => anyhow::bail!(
                        "empty cursor fixture unexpectedly requested furniture services"
                    ),
                }
                call.acknowledge(r).map_err(anyhow::Error::msg)?;
                if resume {
                    v = cold(&p, &v)?;
                    call = serde_json::from_slice(&serde_json::to_vec(&call)?)?;
                }
            }
            ensure!(call.complete() && v.tick == 0 && init_calls == 2 * count);
            ensure!(updates_run == if updates == 0 { 0 } else { 2 * count });
            for id in 1..=count {
                ensure!(v.world.objects[&id].data[&23] == 2);
            }
            runs.push((v, instructions, requests, init_calls, updates_run));
        }
        ensure!(runs[0] == runs[1]);
        reports.push(serde_json::json!({"cursor_parts":count,"selected":selected,"update_argument":updates,"init_calls":runs[0].3,"updates":runs[0].4,"bytecode_instructions":runs[0].1,"services":runs[0].2,"cold_restore_equal":true,"tick":0}));
    }
    if let Ok(output) = std::env::var("SIMS_CURSOR_REGISTRATION_OUTPUT") {
        let out = root.join(output);
        ensure!(out.starts_with(root.join("cache")) && !out.exists());
        std::fs::write(
            out,
            serde_json::to_vec_pretty(
                &serde_json::json!({"native_execution":false,"original_bytecode":true,"resource":d,"cases":reports,"scope":"Isolated registered cursors with null furniture. Original Init/Main bytecode; no complete Buy factory, live lot host or physical placement."}),
            )?,
        )?;
    }
    Ok(())
}
