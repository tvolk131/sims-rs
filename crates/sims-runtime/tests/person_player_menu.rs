use anyhow::{Context, Result, ensure};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};

#[test]
#[ignore = "private actual tutorial person-menu boundary; no frontend or gameplay credit"]
fn actual_person_menu_without_join_restores_selected_query_boundaries() -> Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = root
        .join(std::env::var("SIMS_PERSON_MENU_INPUT")?)
        .canonicalize()?;
    ensure!(input.starts_with(root.join("cache")));
    let output = root.join(std::env::var("SIMS_PERSON_MENU_OUTPUT")?);
    ensure!(output.starts_with(root.join("cache")));
    std::fs::create_dir(&output)?;
    let checkpoint: serde_json::Value = serde_json::from_slice(&std::fs::read(&input)?)?;
    let saved: Saved = serde_json::from_value(checkpoint["session"].clone())?;
    let index = Index::scan(&root.join("game-data/cabinet-extracted"))?;
    let mut reference = None;
    let mut reports = Vec::new();
    let wall = std::time::Instant::now();
    for (budget, cold) in [(1, false), (1, true), (64, false), (64, true)] {
        let mut session = Session::restore(&index, saved.clone())?;
        ensure!(
            session
                .simulation
                .vm
                .autonomy_join_target(&session.program, 105)
                .map_err(anyhow::Error::msg)?
                .is_none()
        );
        ensure!(
            session
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(134)
        );
        let tick = session.simulation.vm.tick;
        let mut boundaries = 0;
        let mut restores = 0;
        let actions = loop {
            ensure!(
                wall.elapsed().as_secs() < 180,
                "person menu diagnostic wall limit"
            );
            let state = session
                .host
                .presentation
                .object_input
                .as_ref()
                .context("person menu absent")?;
            if state.ready() {
                break state.actions().to_vec();
            }
            ensure!(boundaries < 20000, "person menu service budget exhausted");
            if cold && boundaries < 3 {
                let bytes = serde_json::to_vec(&session.save())?;
                session = Session::restore(&index, serde_json::from_slice(&bytes)?)?;
                ensure!(
                    serde_json::to_vec(&session.save())? == bytes,
                    "person query restore changed state"
                );
                restores += 1;
            }
            session.advance_object_input(budget)?;
            ensure!(
                session.simulation.vm.tick == tick,
                "person menu advanced household time"
            );
            boundaries += 1;
        };
        ensure!(
            !actions.is_empty(),
            "actual person menu has no eligible original entries"
        );
        ensure!(actions.iter().all(|action| action.target == 105
            && action.icon_owner == 105
            && action.priority == 50
            && action.flags & 16 == 0));
        let final_state = serde_json::to_value(session.save())?;
        if cold {
            session = Session::restore(&index, serde_json::from_value(final_state.clone())?)?;
            ensure!(
                serde_json::to_value(session.save())? == final_state,
                "ready person menu restore changed state"
            );
            restores += 1;
        }
        if let Some(reference) = &reference {
            ensure!(
                reference == &final_state,
                "person menu budget/cold-restore state differs"
            );
        } else {
            reference = Some(final_state);
        }
        reports.push(
            serde_json::json!({"budget":budget,"cold":cold,"boundaries":boundaries,"restores":restores,
            "tick":tick,"actions":actions,"full_session_equal":true}),
        );
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&reports)?,
    )?;
    Ok(())
}
