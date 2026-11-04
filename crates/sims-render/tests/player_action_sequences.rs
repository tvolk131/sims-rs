//! Bounded production Session/Device input branches and managed cold cuts.
//! This is internal regression evidence, never original-engine parity credit.
use anyhow::{Context, Result, ensure};
use serde_json::json;
use sims_content::Index;
use sims_runtime::exploration::{self, Driver, Limits, Plan, Report};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[path = "support/player_action_driver.rs"]
mod support;
use support::{Command, Household, Identity};

const SOURCE: &str = "cache/gameplay-gaps-reproducers-20261023/phone/initial.json";
const SOURCE_SHA: &str = "b10a6eebb1f395f4ec2786cd65276f23b476b6a03c208aa38a1150df7c23598f";

#[test]
fn completed_tick_classifier_rejects_slices_and_missing_clock_advance() -> Result<()> {
    use sims_runtime::original_session::Progress;
    use support::{Boundary, classify};
    ensure!(classify(40, 41, Progress::Tick)? == Boundary::Tick);
    ensure!(classify(40, 40, Progress::PartialPass)? == Boundary::PartialPass);
    ensure!(classify(40, 40, Progress::Dialog)? == Boundary::Dialog);
    ensure!(classify(40, 40, Progress::HouseNotification)? == Boundary::HouseNotification);
    ensure!(classify(40, 40, Progress::Tick).is_err());
    ensure!(classify(40, 42, Progress::Tick).is_err());
    ensure!(classify(40, 41, Progress::PartialPass).is_err());
    ensure!(classify(40, 41, Progress::Dialog).is_err());
    ensure!(classify(u64::MAX, 0, Progress::Tick).is_err());
    Ok(())
}

#[test]
fn serialized_action_schedule_keeps_explicit_budget_and_rejects_unknown_inputs() -> Result<()> {
    let input = [
        Command::BeginMode { mode: 1 },
        Command::Poll { budget: 1 },
        Command::GuardPendingInput,
        Command::Drain { budget: 256 },
        Command::CheckMode { mode: 1 },
    ];
    let encoded = serde_json::to_vec(&input)?;
    ensure!(serde_json::from_slice::<Vec<Command>>(&encoded)? == input);
    ensure!(
        serde_json::from_value::<Command>(json!({"kind":"poll","budget":1,"skip_semantics":true}))
            .is_err()
    );
    ensure!(serde_json::from_value::<Command>(json!({"kind":"invent_acknowledgement"})).is_err());
    Ok(())
}

fn repo() -> Result<PathBuf> {
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?)
}
fn worker(f: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)?
        .join()
        .expect("player-action regression worker panicked")
}
fn load() -> Result<(Household, Vec<u8>)> {
    let repo = repo()?;
    let bytes = std::fs::read(repo.join(SOURCE))?;
    ensure!(
        exploration::digest(&bytes) == SOURCE_SHA,
        "private research source changed"
    );
    let index = Arc::new(Index::scan(
        &repo.join("cache/managed-independent-20261022-third/immutable-assets"),
    )?);
    ensure!(
        index
            .resources
            .iter()
            .all(|r| !r.locator.to_ascii_lowercase().starts_with("userdata")),
        "immutable index unexpectedly exposes original starting saves"
    );
    let s = Household::from_bytes(index, &bytes)?;
    ensure!(s.session.simulation.vm.tick == 180346 && s.mode()? == 2);
    Ok((s, bytes))
}
fn output(label: &str) -> Result<PathBuf> {
    let root = repo()?;
    let out = root.join(std::env::var("SIMS_PLAYER_SEQUENCES_OUTPUT")?);
    ensure!(
        out.starts_with(root.join("cache")),
        "private output required"
    );
    std::fs::create_dir_all(&out)?;
    let out = out.join(label);
    std::fs::create_dir(&out).context("refusing to overwrite player-action evidence")?;
    std::fs::write(
        out.join("test-source.rs"),
        include_bytes!("player_action_sequences.rs"),
    )?;
    std::fs::write(
        out.join("adapter-source.rs"),
        include_bytes!("support/player_action_driver.rs"),
    )?;
    std::fs::write(
        out.join("input.json"),
        serde_json::to_vec_pretty(&json!({
            "source":SOURCE,"source_sha256":SOURCE_SHA,
            "starting_tick":180346,"managed_save_namespaces":"original UserData namespaces absent",
            "test_source_sha256":exploration::digest(include_bytes!("player_action_sequences.rs")),
            "adapter_source_sha256":exploration::digest(include_bytes!("support/player_action_driver.rs")),
            "qualification":"production Session/Device robustness; not original-engine parity"
        }))?,
    )?;
    Ok(out)
}
fn plan(cold: Vec<usize>, commands: usize) -> Plan {
    Plan {
        seed: 1,
        compare_fork: true,
        compare_ticks: true,
        cold_restore_at_commands: cold,
        limits: Limits {
            commands: commands + 1,
            service_polls: 10_000,
            completed_ticks: 128,
            wall_seconds: 900.,
        },
        ..Default::default()
    }
}
fn retain(out: &Path, label: &str, report: &Report<Command>) -> Result<()> {
    std::fs::write(
        out.join(format!("{label}.json")),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}
fn reduced_plan(original: &Plan, retained_indices: &[usize]) -> Plan {
    let mut p = original.clone();
    p.cold_restore_at = None;
    p.cold_restore_at_commands = original
        .cold_restore_at_commands
        .iter()
        .copied()
        .chain(original.cold_restore_at)
        .map(|cut| {
            retained_indices
                .iter()
                .position(|index| *index >= cut)
                .unwrap_or(retained_indices.len())
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    p.limits.commands = retained_indices.len() + 1;
    p
}
fn minimize_failure(out: &Path, label: &str, report: &Report<Command>) -> Result<()> {
    if let Some(failure) = &report.failure {
        let expected = (failure.kind.clone(), failure.message.clone());
        let indexed: Vec<_> = report.commands().into_iter().enumerate().collect();
        let remap = |candidate: &[(usize, Command)]| {
            reduced_plan(
                &report.plan,
                &candidate.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            )
        };
        let mut reproduces = |candidate: &[(usize, Command)]| {
            let (mut s, _) = load()?;
            let commands: Vec<_> = candidate.iter().map(|(_, c)| c.clone()).collect();
            let r = exploration::run(&mut s, remap(candidate), Some(&commands))?;
            Ok(r.failure
                .as_ref()
                .is_some_and(|f| f.kind == expected.0 && f.message == expected.1))
        };
        ensure!(
            reproduces(&indexed)?,
            "failure minimizer's positive control did not reproduce"
        );
        let (minimized, attempts) = exploration::minimize(&indexed, 8, &mut reproduces)?;
        let minimized_plan = remap(&minimized);
        let minimized_commands: Vec<_> = minimized.into_iter().map(|(_, c)| c).collect();
        std::fs::write(
            out.join(format!("{label}-minimized.json")),
            serde_json::to_vec_pretty(
                &json!({"commands":minimized_commands,"plan":minimized_plan,"positive_control_reproduced":true,
                "attempts":attempts,"failure":failure,
                "source":SOURCE,"source_sha256":SOURCE_SHA,"qualification":"internal production regression; not native parity"}),
            )?,
        )?;
    }
    Ok(())
}

#[test]
fn minimization_preserves_cold_boundaries_when_adjacent_inputs_are_removed() {
    let mut p = plan(vec![1, 3, 4], 5);
    p.cold_restore_at = Some(1);
    p.seed = 23;
    let reduced = reduced_plan(&p, &[0, 2, 4]);
    assert_eq!(reduced.cold_restore_at_commands, [1, 2]);
    assert_eq!(reduced.cold_restore_at, None);
    assert_eq!(reduced.seed, 23);
    assert_eq!(reduced.limits.commands, 4);
    assert_eq!(reduced_plan(&p, &[0]).cold_restore_at_commands, [1]);
    assert_eq!(reduced_plan(&p, &[]).cold_restore_at_commands, [0]);
}
fn replay_case(out: &Path, label: &str, commands: Vec<Command>, cold: Vec<usize>) -> Result<()> {
    let (mut s, _) = load()?;
    eprintln!(
        "{label}: beginning {} recorded production inputs",
        commands.len()
    );
    std::fs::write(
        out.join(format!("{label}-inputs.json")),
        serde_json::to_vec_pretty(&json!({
            "commands":commands,"plan":plan(cold.clone(),commands.len()),"source":SOURCE,"source_sha256":SOURCE_SHA
        }))?,
    )?;
    let report = exploration::run_observed(
        &mut s,
        plan(cold.clone(), commands.len()),
        Some(&commands),
        |_, event| {
            if let exploration::Event::Input { index, command, .. } = event {
                eprintln!("{label}: input {index} {command:?}");
            }
            Ok(())
        },
    )?;
    retain(out, label, &report)?;
    minimize_failure(out, label, &report)?;
    ensure!(report.passed(), "{label}: {:?}", report.failure);
    let (mut replay, _) = load()?;
    let again = exploration::run(&mut replay, plan(cold, commands.len()), Some(&commands))?;
    exploration::verify_replay(&report, &again)?;
    ensure!(
        report.completed_ticks() == 0,
        "input-only scenario advanced gameplay"
    );
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga; SIMS_PLAYER_SEQUENCES_OUTPUT; no native guest or GPU"]
fn mashuga_queue_cancel_and_mode_branches_survive_managed_cold_cuts() -> Result<()> {
    worker(input_cases)
}
fn input_cases() -> Result<()> {
    let out = output("input-cases")?;
    let (s, _) = load()?;
    let resident = Identity::capture(&s.session, 196)?;
    let roaches = Identity::capture(&s.session, 140)?;
    let menu_result = replay_case(
        &out,
        "menu-queue-cancel",
        vec![
            Command::Select { identity: resident },
            Command::Drain { budget: 256 },
            Command::OpenObject {
                identity: roaches.clone(),
            },
            Command::GuardPendingInput,
            Command::Drain { budget: 256 },
            Command::ChooseFirst,
            Command::CancelLastQueued,
            Command::OpenObject { identity: roaches },
            Command::Drain { budget: 256 },
            Command::CloseMenu,
        ],
        vec![1, 3, 5, 6, 9],
    );
    let mode_result = replay_case(
        &out,
        "buy-live-repeated",
        vec![
            Command::Pause { paused: true },
            Command::Drain { budget: 256 },
            Command::BeginMode { mode: 1 },
            Command::GuardPendingInput,
            Command::Drain { budget: 256 },
            Command::CheckMode { mode: 1 },
            Command::BeginMode { mode: 2 },
            Command::Drain { budget: 256 },
            Command::CheckMode { mode: 2 },
            Command::BeginMode { mode: 1 },
            Command::Drain { budget: 256 },
            Command::BeginMode { mode: 2 },
            Command::Drain { budget: 256 },
            Command::CheckMode { mode: 2 },
        ],
        vec![1, 3, 7, 10, 12],
    );
    let results = json!({"menu_queue_cancel":menu_result.as_ref().err().map(|e|format!("{e:#}")),
        "buy_live_repeated":mode_result.as_ref().err().map(|e|format!("{e:#}")),
        "native_parity_ticks":0,"qualifying_4096_scenarios":0});
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    menu_result?;
    mode_result?;
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga Buy cursor cancellation; SIMS_PLAYER_SEQUENCES_OUTPUT"]
fn mashuga_unpaid_furniture_edit_cancel_survives_pending_cold_cuts() -> Result<()> {
    worker(edit_cases)
}
fn edit_cases() -> Result<()> {
    use sims_runtime::original_session::move_tool::{button, motion};
    let out = output("edit-cases")?;
    let pointer = motion::Input {
        position: [376, 488],
        screen: [230, 300],
        picked: 0,
    };
    replay_case(
        &out,
        "unpaid-edit-cancel",
        vec![
            Command::Pause { paused: true },
            Command::Drain { budget: 256 },
            Command::BeginMode { mode: 1 },
            Command::Drain { budget: 256 },
            Command::BuyCategory {
                label: "Seating".into(),
            },
            Command::Drain { budget: 256 },
            Command::BuyProduct {
                source_contains: "chair".into(),
            },
            Command::Drain { budget: 256 },
            Command::EnterMove,
            Command::Drain { budget: 256 },
            Command::Move { pointer },
            Command::GuardPendingInput,
            Command::Drain { budget: 256 },
            Command::Button {
                input: button::Input {
                    kind: button::Kind::Cancel,
                    pointer,
                    shift: false,
                },
            },
            Command::Drain { budget: 256 },
            Command::CheckNoHeld,
            Command::BeginMode { mode: 2 },
            Command::Drain { budget: 256 },
            Command::CheckMode { mode: 2 },
        ],
        vec![3, 5, 7, 9, 11, 14, 17],
    )?;
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga deterministic seed sweep; SIMS_PLAYER_SEQUENCES_OUTPUT"]
fn mashuga_seeded_player_input_branches_replay_exactly() -> Result<()> {
    worker(seed_cases)
}
fn seed_cases() -> Result<()> {
    let out = output("seed-cases")?;
    let mut outcomes = vec![];
    let mut schedules = std::collections::BTreeSet::new();
    let mut selections = 0;
    let mut object_inputs = 0;
    for seed in [1, 7, 23, 0x5eed] {
        let (mut s, _) = load()?;
        ensure!(
            s.candidates()?
                .iter()
                .any(|c| matches!(c.command, Command::Select { .. })),
            "seed setup offers no visible resident selection; generator coverage gap"
        );
        let mut p = plan(vec![4, 8], 16);
        p.seed = seed;
        p.limits.commands = 16;
        std::fs::write(
            out.join(format!("seed-{seed}-plan.json")),
            serde_json::to_vec_pretty(&p)?,
        )?;
        let mut journal = vec![];
        let report = exploration::run_observed(&mut s, p.clone(), None, |_, event| {
            if let exploration::Event::Input { index, command, .. } = event {
                journal.push(command.clone());
                std::fs::write(
                    out.join(format!("seed-{seed}-inputs.json")),
                    serde_json::to_vec_pretty(&journal)?,
                )?;
                eprintln!("seed {seed}: input {index} {command:?}");
            }
            Ok(())
        })?;
        schedules.insert(exploration::digest_serialized(&report.commands())?);
        selections += report
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Select { .. }))
            .count();
        object_inputs += report
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::OpenObject { .. }))
            .count();
        retain(&out, &format!("seed-{seed}"), &report)?;
        minimize_failure(&out, &format!("seed-{seed}"), &report)?;
        let (mut replay, _) = load()?;
        let repeated = exploration::run(&mut replay, p, Some(&report.commands()))?;
        exploration::verify_replay(&report, &repeated)?;
        outcomes.push(json!({"seed":seed,"outcome":report.outcome,"commands":report.steps.len(),
            "completed_ticks":report.completed_ticks(),"failure":report.failure,"cold_restores":report.cold_restores}));
    }
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&json!({"runs":outcomes,
        "native_parity_ticks":0,"qualifying_4096_scenarios":0,
        "distinct_schedules":schedules.len(),"selection_inputs":selections,"object_inputs":object_inputs,
        "qualification":"deterministic robustness branches; failed seeds are retained and replay checked"}))?,
    )?;
    ensure!(
        schedules.len() > 1 && selections > 0 && object_inputs > 0,
        "seed batch did not cover distinct schedules, resident selection and object input; coverage gap, not a VM divergence"
    );
    ensure!(
        outcomes.iter().all(|r| r["outcome"] == "passed"),
        "seed sweep found a retained failure"
    );
    Ok(())
}

fn leaf_summary(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(s) if s.len() <= 128 => value.clone(),
        _ => {
            json!({"kind":match value {serde_json::Value::Array(_)=>"array",serde_json::Value::Object(_)=>"object",_=>"string"},
            "length":match value {serde_json::Value::Array(a)=>a.len(),serde_json::Value::Object(o)=>o.len(),serde_json::Value::String(s)=>s.len(),_=>0},
            "sha256":exploration::digest_serialized(value).expect("JSON values always serialize")})
        }
    }
}
fn state_differences(
    path: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
    rows: &mut Vec<serde_json::Value>,
) {
    if a == b || rows.len() >= 64 {
        return;
    }
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                state_differences(
                    &format!("{path}/{key}"),
                    a.get(key).unwrap_or(&serde_json::Value::Null),
                    b.get(key).unwrap_or(&serde_json::Value::Null),
                    rows,
                );
            }
        }
        (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
            if a.len() != b.len() {
                rows.push(json!({"path":format!("{path}/length"),"warm":a.len(),"cold":b.len()}));
            }
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                state_differences(&format!("{path}/{i}"), a, b, rows);
            }
        }
        _ => rows.push(json!({"path":path,"warm":leaf_summary(a),"cold":leaf_summary(b)})),
    }
}

#[test]
fn missing_large_leaf_produces_a_bounded_difference_record() -> Result<()> {
    let mut differences = vec![];
    state_differences(
        "/large",
        &serde_json::Value::Null,
        &json!(vec![112u8; 10000]),
        &mut differences,
    );
    let encoded = serde_json::to_vec(&differences)?;
    ensure!(encoded.len() < 1024 && differences.len() == 1);
    ensure!(differences[0]["cold"]["length"] == 10000);
    ensure!(
        differences[0]["cold"]["sha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64)
    );
    Ok(())
}

#[test]
#[ignore = "private managed Mashuga; isolates first resumed-selection full-state divergence"]
fn mashuga_pending_selection_cold_cut_field_differences() -> Result<()> {
    worker(selection_field_diff)
}
fn selection_field_diff() -> Result<()> {
    let out = output("selection-cold-diff")?;
    let (mut warm, _) = load()?;
    let resident = Identity::capture(&warm.session, 196)?;
    warm.execute(&Command::Select { identity: resident }, &Limits::default())?;
    let input = warm.observe()?;
    let mut fork = warm.fork()?;
    ensure!(fork.observe()? == input, "fork changed pending input");
    let mut cold = warm.restore(&warm.checkpoint()?)?;
    ensure!(
        cold.observe()? == input,
        "pending cold restore changed input before continuation"
    );
    for branch in [&mut warm, &mut fork, &mut cold] {
        branch.execute(&Command::Drain { budget: 256 }, &Limits::default())?;
    }
    let mut differences = vec![];
    let mut fork_differences = vec![];
    state_differences(
        "",
        &warm.research_state()?,
        &fork.research_state()?,
        &mut fork_differences,
    );
    state_differences(
        "",
        &warm.research_state()?,
        &cold.research_state()?,
        &mut differences,
    );
    std::fs::write(
        out.join("first-differences.json"),
        serde_json::to_vec_pretty(&json!({
            "source":SOURCE,"source_sha256":SOURCE_SHA,"pending_input":input,
            "warm_after":warm.observe()?,"fork_after":fork.observe()?,"cold_after":cold.observe()?,
            "differences":differences,"fork_differences":fork_differences,
            "qualification":"internal full-state continuation diagnostic; no native parity credit"
        }))?,
    )?;
    ensure!(
        differences.is_empty() && fork_differences.is_empty(),
        "resumed selection differs at {} retained paths (first64)",
        differences.len()
    );
    Ok(())
}
