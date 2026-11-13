use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use sims_content::{
    Index,
    vm::{ChairFixture, ChairScenario, load_program},
};
use sims_vm::{Program, Run, Snapshot, Start, Status, Stop, Trace, Vm};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Subcommand)]
pub enum Command {
    /// Run Init/Main in explicit single-object laboratory storage.
    Lifecycle {
        #[arg(default_value = "Cursors.iff")]
        resource: String,
        #[arg(long, default_value_t = 128)]
        object: u16,
        #[arg(long, default_value_t = 10)]
        ticks: u32,
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        #[arg(long, default_value = "cache/object-lifecycle")]
        out: PathBuf,
    },
    /// Census all IFF-signature containers, BHAV operands and static dependencies.
    Inventory {
        #[arg(long, default_value = "cache/compatibility-inventory.json")]
        out: PathBuf,
        /// Optional metadata from the original native dispatcher probes.
        #[arg(long)]
        native_map: Option<PathBuf>,
    },
    /// Move a Sim through an authored room using the original chair's Sit bytecode.
    Room {
        #[arg(long, default_value = "empty")]
        scenario: ChairScenario,
        #[arg(long)]
        blocked: bool,
        #[arg(long, default_value_t = 1)]
        facing: u8,
        #[arg(long, default_value_t = 256)]
        ticks: u32,
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        #[arg(long, default_value = "cache/chair-room")]
        out: PathBuf,
    },
    /// Restore a complete room checkpoint, including in-progress movement and host work.
    RoomResume {
        snapshot: PathBuf,
        #[arg(long, default_value_t = 256)]
        ticks: u32,
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        #[arg(long, default_value = "cache/chair-room-resumed")]
        out: PathBuf,
    },
    /// Follow literal calls, retaining the object's private code context.
    Deps {
        #[arg(default_value = "DiningChairs.iff")]
        resource: String,
        #[arg(long, default_value_t = 4097)]
        bhav: u16,
        #[arg(long, default_value = "Global.iff")]
        globals: String,
    },
    /// Run original Sit eligibility/action bytecode in a small, explicit test world.
    Chair {
        #[arg(long, default_value = "DiningChairs.iff")]
        resource: String,
        #[arg(long, default_value_t = 16807)]
        object: u16,
        #[arg(long, default_value = "Global.iff")]
        globals: String,
        #[arg(long, default_value = "empty")]
        scenario: ChairScenario,
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        #[arg(long, default_value = "cache/chair-vm")]
        out: PathBuf,
    },
    /// Reload our native VM snapshot; optionally deliver a routing failure response.
    Resume {
        snapshot: PathBuf,
        #[arg(long, default_value = "DiningChairs.iff")]
        resource: String,
        #[arg(long, default_value = "Global.iff")]
        globals: String,
        #[arg(long)]
        route_failed: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        budget: usize,
        #[arg(long, default_value_t = 0)]
        ticks: u32,
        #[arg(long, default_value = "cache/chair-vm-resumed")]
        out: PathBuf,
    },
}
pub fn run(index: &Index, command: Command) -> Result<()> {
    match command {
        Command::Lifecycle {
            resource,
            object,
            ticks,
            budget,
            out,
        } => {
            if budget == 0 || ticks > 1_000_000 {
                bail!("lifecycle needs a positive budget and at most 1000000 ticks");
            }
            validate_output(index, &out)?;
            let program = load_program(index, &resource, "Global.iff")?;
            let (_, world) = sims_content::vm::lifecycle_fixture(index, &resource, object)?;
            let mut state = Vm::new(world, 1);
            let id = state
                .reset_object_lifecycle(&program, 1, sims_vm::construction::ObjectDate::default())
                .map_err(anyhow::Error::msg)?;
            let mut trace = Vec::new();
            for tick in 0..=ticks {
                let mut remaining = budget;
                loop {
                    let run = state
                        .run_slice(&program, id, remaining)
                        .map_err(anyhow::Error::msg)?;
                    remaining -= run.trace.len();
                    trace.extend(run.trace);
                    if run.stop != Stop::Budget || remaining == 0 {
                        break;
                    }
                }
                if tick == ticks || !matches!(state.threads[&id].status, Status::Sleeping { .. }) {
                    break;
                }
                state.advance_tick().map_err(anyhow::Error::msg)?;
            }
            save(&program, &state, &trace, &out)?;
            println!(
                "Explicit laboratory object #{object}: {:?}, tick {}, {} instructions",
                state.lifecycles[&1].phase,
                state.tick,
                trace.len()
            );
            check_faults(&state)?;
        }
        Command::Inventory { out, native_map } => {
            if out.is_symlink() {
                bail!("refusing to overwrite a symlink inventory output");
            }
            validate_output(index, &out)?;
            let mut report = sims_content::compatibility::inventory(index);
            if let Some(path) = native_map {
                let evidence: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
                if evidence["schema"] != 1
                    || evidence["dispatch"].as_array().is_none_or(|a| {
                        a.len() != 256
                            || a.iter()
                                .enumerate()
                                .any(|(i, row)| row["opcode"].as_u64() != Some(i as u64))
                    })
                {
                    bail!("unsupported native dispatch metadata");
                }
                report.native_evidence = Some(evidence);
            }
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&out, serde_json::to_vec_pretty(&report)?)?;
            println!(
                "{} resources probed; {} containers; {} BHAVs; {} instructions; {} diagnostics\n{}",
                report.probed_resources,
                report.totals.containers,
                report.totals.behaviors,
                report.totals.instructions,
                report.diagnostics.len(),
                out.display()
            );
        }
        Command::Room {
            scenario,
            blocked,
            facing,
            ticks,
            budget,
            out,
        } => {
            validate_output(index, &out)?;
            let mut program = load_program(index, "DiningChairs.iff", "Global.iff")?;
            sims_content::vm::load_chair_animations(index, &mut program)?;
            let fixture = ChairFixture::load(index, "DiningChairs.iff", 16807, scenario)?;
            println!(
                "{} · {scenario:?}\nAuthored single-floor room; one subtile per tick; original Sit and SLOT resources.\n",
                fixture.title
            );
            let mut state = sims_content::vm::chair_room(&program, fixture, blocked, facing)?;
            run_room(&program, &mut state, ticks, budget, &out)?;
        }
        Command::RoomResume {
            snapshot,
            ticks,
            budget,
            out,
        } => {
            validate_output(index, &out)?;
            let mut program = load_program(index, "DiningChairs.iff", "Global.iff")?;
            sims_content::vm::load_chair_animations(index, &mut program)?;
            if snapshot.metadata()?.len() > 16 * 1024 * 1024 {
                bail!("room snapshot exceeds 16 MiB limit");
            }
            let mut state = sims_sim::Snapshot::from_json(&fs::read(snapshot)?, &program)
                .map_err(anyhow::Error::msg)?;
            run_room(&program, &mut state, ticks, budget, &out)?;
        }
        Command::Deps {
            resource,
            bhav,
            globals,
        } => {
            let program = load_program(index, &resource, &globals)?;
            let owner = &index.resources[index.find(&resource)?].locator;
            println!(
                "{}",
                serde_json::to_string_pretty(&program.dependencies(owner, bhav))?
            );
        }
        Command::Chair {
            resource,
            object,
            globals,
            scenario,
            budget,
            out,
        } => {
            validate_output(index, &out)?;
            let program = load_program(index, &resource, &globals)?;
            let fixture = ChairFixture::load(index, &resource, object, scenario)?;
            println!(
                "{} · {scenario:?}\nLaboratory fixture: actor #1, chair #2; no lot, routing service, or animation runtime.\n",
                fixture.title
            );
            let mut vm = Vm::new(fixture.world, 1);
            let check = vm
                .spawn(
                    &program,
                    Start {
                        owner: &fixture.owner,
                        bhav: fixture.check,
                        caller: 1,
                        callee: 2,
                        stack_object: 2,
                        args: fixture.args.clone(),
                        check: true,
                    },
                )
                .map_err(anyhow::Error::msg)?;
            let mut trace = Vec::new();
            let result = vm
                .run_slice(&program, check, budget)
                .map_err(anyhow::Error::msg)?;
            show_run("Eligibility", &result);
            let eligible = result.stop == Stop::Returned(true);
            trace.extend(result.trace);
            if eligible {
                let action = vm
                    .spawn(
                        &program,
                        Start {
                            owner: &fixture.owner,
                            bhav: fixture.action,
                            caller: 1,
                            callee: 2,
                            stack_object: 2,
                            args: fixture.args,
                            check: false,
                        },
                    )
                    .map_err(anyhow::Error::msg)?;
                let result = vm
                    .run_slice(&program, action, budget)
                    .map_err(anyhow::Error::msg)?;
                show_run("Sit action", &result);
                trace.extend(result.trace);
            }
            save(&program, &vm, &trace, &out)?;
            check_faults(&vm)?;
        }
        Command::Resume {
            snapshot,
            resource,
            globals,
            route_failed,
            budget,
            ticks,
            out,
        } => {
            validate_output(index, &out)?;
            let program = load_program(index, &resource, &globals)?;
            if snapshot.metadata()?.len() > 16 * 1024 * 1024 {
                bail!("snapshot exceeds 16 MiB limit");
            }
            let mut vm =
                Snapshot::from_json(&fs::read(snapshot)?, &program).map_err(anyhow::Error::msg)?;
            if let Some(request) = route_failed {
                let thread = vm
                    .threads
                    .iter()
                    .find_map(|(&id, t)| match &t.status {
                        Status::AwaitingRoute(r) if r.id == request => Some(id),
                        _ => None,
                    })
                    .context("pending route request not found")?;
                println!(
                    "Injecting explicit routing failure for request {request}; no pathfinding was run."
                );
                vm.finish_route(&program, thread, request, false)
                    .map_err(anyhow::Error::msg)?;
            }
            let mut trace = Vec::new();
            for turn in 0..=ticks {
                if turn != 0 {
                    vm.advance_tick().map_err(anyhow::Error::msg)?;
                }
                let mut stopped = false;
                for (id, result) in vm.run_round(&program, budget) {
                    stopped |= matches!(result.stop, Stop::Budget | Stop::Faulted);
                    show_run(&format!("Thread {id}"), &result);
                    trace.extend(result.trace);
                }
                if stopped {
                    break;
                }
            }
            save(&program, &vm, &trace, &out)?;
            check_faults(&vm)?;
        }
    }
    Ok(())
}
fn show_run(label: &str, run: &Run) {
    for t in &run.trace {
        println!(
            "tick {} thread {} depth {} · BHAV {:#06x}[{:03}] opcode {:#06x}\n  {}\n  {}",
            t.tick,
            t.thread,
            t.depth,
            t.location.bhav,
            t.location.instruction,
            t.opcode,
            t.location.source,
            t.effect
        );
    }
    println!("{label}: {:?}\n", run.stop);
}
fn save(program: &Program, vm: &Vm, trace: &[Trace], out: &Path) -> Result<()> {
    fs::create_dir_all(out)?;
    fs::write(
        out.join("snapshot.json"),
        serde_json::to_vec_pretty(&Snapshot::capture(program, vm))?,
    )?;
    let jsonl = trace
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    fs::write(
        out.join("trace.jsonl"),
        if jsonl.is_empty() {
            jsonl
        } else {
            format!("{jsonl}\n")
        },
    )?;
    println!("Snapshot and trace: {}", out.display());
    Ok(())
}
fn check_faults(vm: &Vm) -> Result<()> {
    for t in vm.threads.values() {
        if t.status == Status::Ready {
            bail!(
                "instruction budget exhausted; resume the saved state without advancing its tick"
            );
        }
        if let Status::Faulted(f) = &t.status {
            return Err(anyhow!("VM stopped: {}\n{:?}", f.message, f.stack));
        }
    }
    Ok(())
}
fn validate_output(index: &Index, out: &Path) -> Result<()> {
    if out
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        bail!("output paths must not contain '..'");
    }
    // Resolve existing ancestors first so symlinks cannot redirect into the asset tree.
    let absolute = if out.is_absolute() {
        out.to_owned()
    } else {
        std::env::current_dir()?.join(out)
    };
    let mut suffix = Vec::new();
    let mut ancestor = absolute.as_path();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("invalid output directory")?
                .to_owned(),
        );
        ancestor = ancestor
            .parent()
            .context("output has no existing ancestor")?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    if resolved.starts_with(index.root.canonicalize()?) {
        bail!("VM outputs must stay outside the source asset tree");
    }
    for filename in ["snapshot.json", "trace.jsonl", "events.jsonl"] {
        if out.join(filename).is_symlink() {
            bail!("refusing to overwrite a symlink output: {filename}");
        }
    }
    Ok(())
}

fn run_room(
    program: &Program,
    state: &mut sims_sim::Simulation,
    ticks: u32,
    budget: usize,
    out: &Path,
) -> Result<()> {
    let mut traces = Vec::new();
    let mut events = Vec::new();
    let result = (|| -> Result<()> {
        for turn in 0..=ticks {
            if turn != 0 {
                state.advance_fixture_tick().map_err(anyhow::Error::msg)?;
            }
            let step = state
                .run_fixture_tick(program, budget)
                .map_err(anyhow::Error::msg)?;
            for event in &step.events {
                println!(
                    "tick {} thread {} · {}",
                    event.tick, event.thread, event.detail
                );
            }
            for t in &step.trace {
                println!(
                    "tick {} · BHAV {:#06x}[{}] {}",
                    t.tick, t.location.bhav, t.location.instruction, t.effect
                );
            }
            traces.extend(step.trace);
            events.extend(step.events);
            match step.stop {
                sims_sim::StepStop::Waiting => (),
                sims_sim::StepStop::Budget => {
                    bail!("instruction budget exhausted; resume this checkpoint in the same tick")
                }
                sims_sim::StepStop::Faulted => {
                    check_faults(&state.vm)?;
                    bail!("VM faulted");
                }
                stop => {
                    println!("Room stopped: {stop:?} at tick {}", state.vm.tick);
                    return Ok(());
                }
            }
        }
        println!(
            "Tick limit reached at {}; simulation checkpoint saved.",
            state.vm.tick
        );
        Ok(())
    })();
    fs::create_dir_all(out)?;
    fs::write(
        out.join("snapshot.json"),
        serde_json::to_vec_pretty(&sims_sim::Snapshot::capture(program, state))?,
    )?;
    for (name, lines) in [
        (
            "trace.jsonl",
            traces
                .iter()
                .map(serde_json::to_string)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
        (
            "events.jsonl",
            events
                .iter()
                .map(serde_json::to_string)
                .collect::<std::result::Result<Vec<_>, _>>()?,
        ),
    ] {
        let text = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        fs::write(out.join(name), text)?;
    }
    println!(
        "Room checkpoint, bytecode trace and movement/animation events: {}",
        out.display()
    );
    result
}
