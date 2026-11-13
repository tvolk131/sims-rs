//! Authored inputs for stock-house scheduler tests, never production defaults.
use anyhow::{Context, Result};
use sims_runtime::original_session::{Progress, Session};

/// Only the five one-OK expansion notices and the exact help activation question.
/// Select No for help so unrelated tutorial observations do not own this test.
pub fn answer_startup_dialog(session: &mut Session) -> Result<()> {
    let record = session
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|r| Some(r.id) == session.simulation.vm.world.dialogs.modal)
        .context("modal record missing")?;
    let notice = record.origin.bhav == 4119
        && [
            "HDHelpSystem.iff",
            "HelpSystemMagic.iff",
            "NeighborhoodHelpSystem.iff",
            "HelpSystemStudioTown.iff",
            "VacationHelpSystem.iff",
        ]
        .iter()
        .any(|name| record.origin.source.ends_with(name))
        && record.buttons.len() == 1
        && record.buttons[0].result == 2;
    let help = record.origin.bhav == 4119
        && record.origin.source.ends_with("::HelpSystem.iff")
        && record
            .body
            .ends_with("Would you like the help system activated?")
        && record.buttons.len() == 2;
    let response = if notice {
        Some(2)
    } else if help {
        record
            .buttons
            .iter()
            .find(|b| b.label.eq_ignore_ascii_case("no"))
            .map(|b| b.result)
    } else {
        None
    };
    let response = response.with_context(|| {
        format!(
            "unexpected dialog at {}: {record:?}",
            session.simulation.vm.tick
        )
    })?;
    println!(
        "{}: explicit startup input {}: BHAV{} {} result{}",
        session.house.source, record.id, record.origin.bhav, record.origin.source, response
    );
    session
        .simulation
        .vm
        .answer_dialog(record.id, response)
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

pub fn step_startup_inputs(session: &mut Session) -> Result<Progress> {
    for _ in 0..20 {
        let progress = session.step()?;
        if progress != Progress::Dialog {
            return Ok(progress);
        }
        answer_startup_dialog(session)?;
    }
    anyhow::bail!("startup input budget exceeded")
}
