//! Read-only software views of sessions produced by original_scheduler.rs.
use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::original_house::OriginalHouse;
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "requires private original scheduler sessions and assets; headless software only"]
fn native_scheduler_household_frames_freeze_while_paused() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut report = vec![];
    for name in ["newbie", "goth", "mashuga"] {
        let dir = root
            .join(
                std::env::var("SIMS_SCHEDULER_EVIDENCE")
                    .unwrap_or_else(|_| "cache/native-simulation-scheduler-homes".into()),
            )
            .join(name);
        let mut frames = vec![];
        let mut paused_tick = None;
        for phase in ["paused-before", "paused-after", "live"] {
            let saved: Saved =
                serde_json::from_slice(&std::fs::read(dir.join(format!("{phase}.json")))?)?;
            let session = Session::restore(&index, saved)?;
            ensure!(session.simulation.native_scheduler.is_some());
            ensure!(session.simulation.vm.world.globals[&19] == 0);
            ensure!(
                session.simulation.vm.world.globals[&17] == i16::from(phase != "live"),
                "incorrect pause input in {name}/{phase} artifact"
            );
            let tick = *paused_tick.get_or_insert(session.simulation.vm.tick);
            ensure!(session.simulation.vm.tick == tick + u64::from(phase == "live"));
            let before = serde_json::to_vec(&session.save())?;
            let camera = OriginalHouse::camera(&session, 960, 720, 0, 0.7, 1);
            let scene = OriginalHouse::default().scene(&session, camera)?;
            let image = sims_render::software::render(&scene);
            image.save(dir.join(format!("{phase}.png")))?;
            ensure!(
                serde_json::to_vec(&session.save())? == before,
                "rendering mutated {name}/{phase}"
            );
            frames.push(image);
        }
        ensure!(
            frames[0] == frames[1],
            "paused room pixels changed for {name}"
        );
        report.push(serde_json::json!({"house":name,"paused_pixels_identical":true,"rendering_preserves_full_save":true,"live_pixels_changed":frames[1]!=frames[2],"size":[960,720]}));
        println!("{name}: paused software frames identical; rendering preserves saved state");
    }
    std::fs::write(
        root.join(
            std::env::var("SIMS_SCHEDULER_EVIDENCE")
                .unwrap_or_else(|_| "cache/native-simulation-scheduler-homes".into()),
        )
        .join("frames.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"headless":true,"gui_launched":false,"native_pixel_equivalence":false,"houses":report}),
        )?,
    )?;
    Ok(())
}
