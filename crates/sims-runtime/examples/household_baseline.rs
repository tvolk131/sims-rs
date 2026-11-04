//! Bounded, reproducible timings. Original assets and outputs remain private.
use anyhow::{Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Options, Progress, Session};
use std::{path::PathBuf, time::Instant};

fn run() -> Result<()> {
    let output = PathBuf::from(std::env::args().nth(1).expect("output directory"));
    std::fs::create_dir(&output)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
    let start = Instant::now();
    let index = Index::scan(&root)?;
    let index_seconds = start.elapsed().as_secs_f64();
    assets(&index, &output)?;
    let start = Instant::now();
    let mut session = Session::load(
        &index,
        "UserData2/Houses/House09.iff",
        Options {
            routing: sims_sim::lot_route::path::Policy::InferredRectangles,
            ..Default::default()
        },
    )?;
    let load_seconds = start.elapsed().as_secs_f64();
    // Explicit benchmark Live input, after loading the original saved mode.
    session.simulation.vm.world.globals.insert(19, 0);
    let start_tick = session.simulation.vm.tick;
    let start = Instant::now();
    let mut boundary = None;
    let mut answers = vec![];
    while session.simulation.vm.tick - start_tick < 1000 {
        let progress = session.step()?;
        if progress == Progress::Dialog {
            let record = session
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|r| Some(r.id) == session.simulation.vm.world.dialogs.modal)
                .filter(|r| {
                    r.origin.bhav == 4119
                        && [
                            "HDHelpSystem.iff",
                            "HelpSystemMagic.iff",
                            "NeighborhoodHelpSystem.iff",
                            "HelpSystemStudioTown.iff",
                            "VacationHelpSystem.iff",
                        ]
                        .iter()
                        .any(|name| r.origin.source.ends_with(name))
                        && r.buttons.len() == 1
                        && r.buttons[0].result == 2
                });
            if let Some(record) = record {
                let id = record.id;
                answers.push(serde_json::json!({"tick":session.simulation.vm.tick,"id":id,"result":2,"body":record.body}));
                session
                    .simulation
                    .vm
                    .answer_dialog(id, 2)
                    .map_err(anyhow::Error::msg)?;
                continue;
            }
        }
        if progress != Progress::Tick {
            boundary = Some(format!("{progress:?}"));
            break;
        }
    }
    let simulation_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let bytes = serde_json::to_vec(&session.save())?;
    let serialize_seconds = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let restored = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
    let restore_seconds = start.elapsed().as_secs_f64();
    ensure!(
        serde_json::to_vec(&restored.save())? == bytes,
        "restored state differs"
    );
    let report = serde_json::json!({
        "source": session.house.source, "profile": "original registry scheduler, explicit Live input and inferred rectangle routing",
        "ticks": session.simulation.vm.tick-start_tick, "index_seconds": index_seconds,
        "load_seconds": load_seconds, "simulation_seconds": simulation_seconds,
        "ticks_per_second": (session.simulation.vm.tick-start_tick) as f64/simulation_seconds, "serialize_seconds": serialize_seconds,
        "restore_seconds": restore_seconds, "saved_bytes": bytes.len(), "restore_identical": true,
        "rendering_included": false, "native_execution": false, "boundary": boundary,
        "dialogs": session.simulation.vm.world.dialogs.records, "scripted_inputs":answers
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    // A contact sheet can use these exact installed resources without ever
    // constructing objects or running a simulation in order to draw UI art.
    Ok(())
}
fn assets(index: &Index, output: &std::path::Path) -> Result<()> {
    let bitmaps = sims_content::dialog::ui_bitmaps::Bitmaps::load(index)?;
    let mut assets = vec![];
    for id in [
        48, 100, 101, 102, 108, 150, 151, 152, 153, 154, 155, 156, 157, 2008, 2009, 2010, 2011,
        2012, 2013, 2014, 2023, 2024, 2026, 4500, 4506, 4510, 4600, 4702, 4705, 4706, 4707, 4803,
        4804, 4910, 4911,
    ] {
        let image = match bitmaps.image(
            index,
            sims_vm::dialog::GlobalBitmap {
                id,
                button_cell: false,
            },
            1,
            [255, 0, 255],
        ) {
            Ok(image) => image,
            Err(error) => {
                assets.push(serde_json::json!({"id":id,"error":error.to_string()}));
                continue;
            }
        };
        if let Some(image) = image {
            assets.push(serde_json::json!({"id":id,"width":image.width(),"height":image.height()}));
            image.save(output.join(format!("ui-{id}.png")))?;
        }
    }
    std::fs::write(
        output.join("ui-assets.json"),
        serde_json::to_vec_pretty(&assets)?,
    )?;
    Ok(())
}
fn main() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("baseline panicked")
}
