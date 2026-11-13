//! Restore scheduling belongs to the experiment, never to the game scheduler.
use super::*;

pub(super) fn schedule(
    mut plan: Plan,
    commands: Option<&[Command]>,
    after_milestones: &[usize],
) -> Result<Plan> {
    if after_milestones.is_empty() {
        return Ok(plan);
    }
    let commands = commands.context("milestone restores require an explicit command script")?;
    for &boundary in after_milestones {
        ensure!(
            boundary > 0 && boundary < commands.len(),
            "milestone restore requires a reached milestone and a following command"
        );
        ensure!(
            matches!(
                commands[boundary - 1],
                Command::Wait { .. } | Command::Check { .. }
            ),
            "cold restore boundary {boundary} is not a Wait/Check milestone"
        );
        plan.cold_restore_at_commands.push(boundary);
    }
    plan.cold_restore_at_commands.sort_unstable();
    plan.cold_restore_at_commands.dedup();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn milestones_schedule_only_the_following_command_boundaries() -> Result<()> {
        let commands: Vec<Command> = serde_json::from_value(json!([
            {"kind":"check", "milestone":{"kind":"completed_ticks", "count":0}},
            {"kind":"wait", "milestone":{"kind":"completed_ticks", "count":1},
             "max_ticks":2,"max_frames":10},
            {"kind":"poll"}
        ]))?;
        let scheduled = schedule(Plan::default(), Some(&commands), &[1, 2, 1])?;
        ensure!(scheduled.cold_restore_at_commands == [1, 2]);
        ensure!(schedule(Plan::default(), None, &[1]).is_err());
        ensure!(schedule(Plan::default(), Some(&commands), &[0]).is_err());
        ensure!(schedule(Plan::default(), Some(&commands), &[3]).is_err());
        let non_milestone = [Command::Poll, Command::Poll];
        ensure!(schedule(Plan::default(), Some(&non_milestone), &[1]).is_err());
        Ok(())
    }
}
