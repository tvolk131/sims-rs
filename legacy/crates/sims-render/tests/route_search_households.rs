//! Pure software frames of the independently restored post-routing households.
use anyhow::{Result, ensure};
use sims_content::Index;
use sims_render::{original_house::OriginalHouse, software};
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private queued-routing continuation outputs; fresh SIMS_ROUTE_POLICY_FRAMES_OUTPUT"]
fn routed_households_render_without_advancing_state_and_match_after_cold_restore() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("routing frame check panicked")
}
fn run() -> Result<()> {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let output = repo.join(std::env::var("SIMS_ROUTE_POLICY_FRAMES_OUTPUT")?);
    ensure!(output.starts_with(repo.join("cache")));
    std::fs::create_dir(&output)?;
    let index =
        Index::scan(&repo.join("cache/managed-independent-20261022-third/immutable-assets"))?;
    ensure!(
        index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata"))
    );
    let mut reports = vec![];
    let sources = repo.join(
        std::env::var("SIMS_ROUTE_POLICY_FRAME_SOURCES")
            .unwrap_or_else(|_| "cache/native-route-policy-homes-20261022-final".into()),
    );
    ensure!(sources.starts_with(repo.join("cache")));
    for name in ["newbie", "goth", "mashuga"] {
        let source = sources.join(name).join("tick-512.json");
        let saved: Saved = serde_json::from_slice(&std::fs::read(&source)?)?;
        let mut session = Session::restore(&index, saved)?;
        let tick = session.simulation.vm.tick;
        // Resource preparation is explicit application work; plain drawing
        // remains an observer. Use the already retained draw memberships.
        ensure!(session.person_draw_preparation_ready());
        session.begin_person_draw_preparation(session.person_draw_objects()?)?;
        while !session.advance_person_draw_preparation(16)? {}
        ensure!(session.simulation.vm.tick == tick);
        let prepared = session.save_managed()?;
        std::fs::write(
            output.join(format!("{name}-prepared.json")),
            serde_json::to_vec(&prepared)?,
        )?;
        let frozen = serde_json::to_vec(&session.save())?;
        let camera = OriginalHouse::native_camera(&session)?;
        let scene = OriginalHouse::default().scene(&session, camera)?;
        let frame = software::render(&scene);
        frame.save(output.join(format!("{name}.png")))?;
        ensure!(
            frozen == serde_json::to_vec(&session.save())?,
            "{name}: drawing changed retained state"
        );
        let cold = Session::restore(
            &index,
            serde_json::from_slice(&serde_json::to_vec(&prepared)?)?,
        )?;
        ensure!(
            frozen == serde_json::to_vec(&cold.save())?,
            "{name}: cold state differs"
        );
        let cold_scene =
            OriginalHouse::default().scene(&cold, OriginalHouse::native_camera(&cold)?)?;
        ensure!(
            frame == software::render(&cold_scene),
            "{name}: cold pixels differ"
        );
        ensure!(
            frozen == serde_json::to_vec(&cold.save())?,
            "{name}: cold draw changed state"
        );
        reports.push(
            serde_json::json!({"household":name,"source":source,"tick":tick,
            "drawing_preserves_full_save":true,"cold_pixels_equal":true,
            "native_pixel_parity":false,"gui_launched":false}),
        );
        println!("{name}: tick{tick}, pure drawing and cold pixels agree");
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
