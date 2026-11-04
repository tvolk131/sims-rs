use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};
use sims_content::{Index, neighborhood_input::INSTALLED_HOUSES_ASCENDING_V1};
use sims_runtime::original_session::{
    Options, Session,
    neighborhood_input::{Status, replacement::Replacement},
};
use sims_vm::Trace;
use std::{fs, path::PathBuf};

mod neighborhood_continuation_support;
use neighborhood_continuation_support::{
    Budget, MAX_TRACE_ROWS, advance, calendar, cursor, digest, faults, full_state, mode11,
    note_failure, read_saved, rng, summary, write_new, write_report,
};

const ORIGINAL_FAULT_TICK: u64 = 180_828;

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
enum Routing {
    Strict,
    InferredRectangles,
}

#[derive(Parser, Serialize)]
struct Args {
    output: PathBuf,
    #[arg(long)]
    policy: String,
    #[arg(long)]
    start: Option<PathBuf>,
    #[arg(long, default_value = "UserData2/Houses/House09.iff")]
    source: String,
    #[arg(long, value_enum, default_value = "inferred-rectangles")]
    routing: Routing,
    #[arg(long)]
    headless_live_input: bool,
    #[arg(long, default_value_t = 180_820)]
    search_from_tick: u64,
    #[arg(long, default_value_t = 180_844)]
    search_until_tick: u64,
    #[arg(long, default_value_t = 181_084)]
    stop_after_tick: u64,
    #[arg(long, default_value_t = 6000)]
    max_ticks: u64,
    #[arg(long, default_value_t = 30_000)]
    max_steps: usize,
    #[arg(long, default_value_t = 64)]
    search_steps: usize,
    #[arg(long, default_value_t = 50_000)]
    probe_steps: usize,
    #[arg(long, default_value_t = 8)]
    compare_steps: usize,
    #[arg(long, default_value_t = 90.0)]
    setup_seconds: f64,
    #[arg(long, default_value_t = 30.0)]
    sustain_seconds: f64,
    #[arg(long, default_value_t = 20.0)]
    search_seconds: f64,
    #[arg(long, default_value_t = 15.0)]
    probe_seconds: f64,
    #[arg(long, default_value_t = 90.0)]
    restore_seconds: f64,
    #[arg(long, default_value_t = 30.0)]
    compare_seconds: f64,
    #[arg(long, default_value_t = 240.0)]
    total_seconds: f64,
}

struct Located {
    owner: Session,
    trace: Vec<Trace>,
    boundary: Value,
    state: Vec<u8>,
}

fn replacement(session: &Session) -> Result<&Replacement> {
    match session.neighborhood_input_status()? {
        Status::Replacement(replacement) => Ok(replacement),
        status => bail!("explicit replacement provenance absent: {status:?}"),
    }
}

fn begin(args: &Args, report: &mut Value, budget: &Budget) -> Result<()> {
    report["stage"] = json!(budget.label);
    report["stages"][budget.label] = json!({"status":"running",
        "max_calls":budget.max_calls,"limit_seconds":budget.seconds});
    write_report(&args.output, report)
}

fn end(report: &mut Value, budget: &Budget, result: &Result<()>) {
    report["stages"][budget.label] = budget.receipt();
    report["stages"][budget.label]["status"] = json!(if result.is_ok() {
        "completed"
    } else {
        "stopped"
    });
}

fn checked_advance(
    session: &mut Session,
    trace: Option<&mut Vec<Trace>>,
    owner: &str,
    args: &Args,
    report: &mut Value,
) -> Result<Value> {
    match advance(session, trace) {
        Ok(boundary) => {
            if faults(session) != json!([]) {
                note_failure(
                    session,
                    owner,
                    "faulted owner returned success",
                    &args.output,
                    report,
                )?;
                bail!("faulted owner returned success");
            }
            if !boundary["input"].is_null() {
                report["inputs"]
                    .as_array_mut()
                    .unwrap()
                    .push(boundary["input"].clone());
            }
            Ok(boundary)
        }
        Err(error) => {
            note_failure(session, owner, &format!("{error:#}"), &args.output, report)?;
            Err(error)
        }
    }
}

fn sustain(
    session: &mut Session,
    target: u64,
    label: &'static str,
    args: &Args,
    total: &Budget,
    report: &mut Value,
) -> Result<()> {
    let mut budget = Budget::new(label, args.sustain_seconds, args.max_steps);
    begin(args, report, &budget)?;
    let result = (|| {
        while session.simulation.vm.tick < target {
            budget.take(total)?;
            ensure!(
                report["ordinary_steps"].as_u64().unwrap() < args.max_steps as u64,
                "total ordinary Session call limit reached"
            );
            report["ordinary_steps"] = json!(report["ordinary_steps"].as_u64().unwrap() + 1);
            checked_advance(session, None, label, args, report)?;
        }
        budget.check()?;
        total.check()
    })();
    end(report, &budget, &result);
    result
}

fn locate(
    session: &mut Session,
    args: &Args,
    total: &Budget,
    report: &mut Value,
) -> Result<Option<(Located, Trace, Vec<Trace>)>> {
    let mut budget = Budget::new("mode11_search", args.search_seconds, args.search_steps);
    let mut probe_budget = None;
    begin(args, report, &budget)?;
    let result = (|| {
        while session.simulation.vm.tick <= args.search_until_tick {
            budget.take(total)?;
            ensure!(
                report["ordinary_steps"].as_u64().unwrap() < args.max_steps as u64,
                "total ordinary Session call limit reached"
            );
            let owner = session.fork_for_exploration()?;
            let mut trace = Vec::new();
            report["ordinary_steps"] = json!(report["ordinary_steps"].as_u64().unwrap() + 1);
            let boundary = checked_advance(session, Some(&mut trace), "search", args, report);
            if let Some(instruction) = trace.iter().find(|instruction| mode11(instruction)) {
                report["gates"]["mode11_encountered"] = json!(true);
                if report["first_mode11"].is_null() {
                    report["first_mode11"] = json!(instruction);
                }
            }
            let boundary = boundary?;
            ensure!(
                trace.len() <= MAX_TRACE_ROWS,
                "search trace row limit exceeded"
            );
            budget.check()?;
            total.check()?;
            if trace.iter().any(mode11) {
                let mut located = Located {
                    owner,
                    trace,
                    boundary,
                    state: full_state(session)?,
                };
                budget.check()?;
                total.check()?;
                let probe_budget = probe_budget.get_or_insert_with(|| {
                    Budget::new("small_budget_cut", args.probe_seconds, args.probe_steps)
                });
                let probe = midscan(&mut located, args, total, report, probe_budget);
                let attempt = json!({
                    "boundary_tick": session.simulation.vm.tick,
                    "probe": report["stages"]["small_budget_cut"].clone(),
                    "stop": report["probe_stop"],
                    "interior_cut": probe.as_ref().is_ok_and(Option::is_some),
                    "error": probe.as_ref().err().map(|error| format!("{error:#}"))
                });
                report["probe_attempts"]
                    .as_array_mut()
                    .unwrap()
                    .push(attempt);
                match probe {
                    Ok(Some((instruction, prefix))) => {
                        budget.check()?;
                        total.check()?;
                        return Ok(Some((located, instruction, prefix)));
                    }
                    Ok(None) => {
                        budget.check()?;
                        total.check()?;
                    }
                    Err(error) => {
                        write_new(
                            &args.output.join("cut-owner-final.json"),
                            &serde_json::to_vec(&located.owner.save())?,
                        )?;
                        return Err(error);
                    }
                }
            }
        }
        budget.check()?;
        total.check()?;
        Ok(None)
    })();
    let status = result
        .as_ref()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error:#}"));
    end(report, &budget, &status);
    result
}

fn midscan(
    located: &mut Located,
    args: &Args,
    total: &Budget,
    report: &mut Value,
    budget: &mut Budget,
) -> Result<Option<(Trace, Vec<Trace>)>> {
    begin(args, report, budget)?;
    report["probe_stop"] = Value::Null;
    let result = (|| {
        let owner = &mut located.owner;
        ensure!(
            owner.simulation.vm.world.dialogs.modal.is_none(),
            "probe starts at a modal"
        );
        owner.prepare_headline_catalog()?;
        owner.host.update_contexts(&mut owner.simulation)?;
        let mut prefix = Vec::new();
        loop {
            budget.take(total)?;
            let mut polls = Vec::new();
            let step = owner
                .simulation
                .run_current_tick_with_host_polls(&owner.program, 1, &mut polls)
                .map_err(anyhow::Error::msg)?;
            if step.stop == sims_sim::StepStop::Faulted || faults(owner) != json!([]) {
                note_failure(
                    owner,
                    "small-budget probe",
                    "natural VM fault",
                    &args.output,
                    report,
                )?;
                bail!("probe encountered a natural VM fault");
            }
            let mut slice = Vec::new();
            let mut offset = 0;
            for poll in polls {
                slice.extend_from_slice(&step.trace[offset..poll.before_instruction]);
                slice.push(poll.instruction);
                offset = poll.before_instruction;
            }
            slice.extend_from_slice(&step.trace[offset..]);
            if slice.is_empty() {
                report["probe_stop"] = json!(
                    "Session-owned service needed before an interior cut; no service bypassed"
                );
                budget.check()?;
                total.check()?;
                return Ok(None);
            }
            ensure!(
                prefix.len() + slice.len() <= MAX_TRACE_ROWS,
                "probe trace row limit exceeded"
            );
            prefix.extend(slice);
            ensure!(
                located.trace.starts_with(&prefix),
                "diagnostic budget trace diverged from naturally executed Session prefix"
            );
            let instruction = prefix.last().unwrap();
            if mode11(instruction) && instruction.effect.starts_with("Set To Next 11: ") {
                let position = cursor(owner, instruction)?;
                let value = position["value"].as_i64().context("cursor word absent")?;
                let order = replacement(owner)?.nearby_houses();
                if let Some(index) = order
                    .iter()
                    .position(|candidate| i64::from(*candidate) == value)
                {
                    if value != 0 && index > 0 && index + 1 < order.len() {
                        report["gates"]["midscan_obtained"] = json!(true);
                        report["midscan"] = json!({"instruction":instruction,
                            "cursor":position,"resolved_index":index,"remaining":&order[index+1..],
                            "state":summary(owner),"prefix_instructions":prefix.len()});
                        budget.check()?;
                        total.check()?;
                        return Ok(Some((instruction.clone(), prefix)));
                    }
                }
            }
            if step.stop != sims_sim::StepStop::Budget {
                report["probe_stop"] = json!(format!(
                    "probe reached {:?} before a retained nonterminal scan cursor",
                    step.stop
                ));
                budget.check()?;
                total.check()?;
                return Ok(None);
            }
        }
    })();
    let status = result
        .as_ref()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error:#}"));
    end(report, budget, &status);
    result
}

fn cold_compare(
    located: &mut Located,
    instruction: &Trace,
    prefix: Vec<Trace>,
    index: &Index,
    args: &Args,
    total: &Budget,
    report: &mut Value,
) -> Result<()> {
    let mut budget = Budget::new("managed_save_restore", args.restore_seconds, 4);
    begin(args, report, &budget)?;
    budget.take(total)?;
    let owner = &mut located.owner;
    let retained = replacement(owner)?.clone();
    let retained_cursor = cursor(owner, instruction)?;
    let value = retained_cursor["value"]
        .as_i64()
        .context("cursor word absent")?;
    let current = retained
        .nearby_houses()
        .iter()
        .position(|candidate| i64::from(*candidate) == value)
        .context("retained cursor absent from order")?;
    let successor = retained.nearby_houses()[current + 1];
    let removed = retained
        .manifest()
        .candidates()
        .iter()
        .find(|candidate| i32::from(candidate.lot()) == successor)
        .context("successor manifest row absent")?
        .locator()
        .to_owned();
    let mut changed = index.clone();
    changed
        .resources
        .retain(|resource| !resource.locator.eq_ignore_ascii_case(&removed));
    let rediscovered = Replacement::from_index(&changed, &owner.house)?;
    ensure!(
        changed.resources.len() < index.resources.len()
            && rediscovered.manifest() != retained.manifest()
            && rediscovered.nearby_houses() != retained.nearby_houses(),
        "changed starting Index did not change rediscovery"
    );
    report["gates"]["changed_inventory_proven"] = json!(true);
    report["inventory"] = json!({"method":"clone Index metadata; filter next candidate only",
        "removed_locator":removed,"removed_next_lot":successor,"original_resources":index.resources.len(),
        "changed_resources":changed.resources.len(),"retained":retained,"rediscovered":rediscovered,
        "assets_copied":false,"original_assets_mutated":false});
    let restored = (|| -> Result<Session> {
        budget.take(total)?;
        let before = full_state(owner)?;
        let managed = owner.save_managed()?;
        ensure!(
            managed.version == 2 && managed.managed.is_some(),
            "managed save absent"
        );
        ensure!(
            full_state(owner)? == before,
            "managed capture mutated its owner"
        );
        let bytes = serde_json::to_vec(&managed)?;
        write_new(&args.output.join("midscan-managed.json"), &bytes)?;
        report["gates"]["managed_save_completed"] = json!(true);
        report["managed_save"] = json!({"path":"midscan-managed.json","bytes":bytes.len(),
            "sha256":digest(&bytes),"version":managed.version,
            "content_schema":managed.managed.as_ref().map(|content|content.schema)});
        drop(bytes);
        drop(managed);
        budget.take(total)?;
        write_report(&args.output, report)?;
        let (saved, disk_digest) = read_saved(&args.output.join("midscan-managed.json"))?;
        ensure!(
            report["managed_save"]["sha256"] == disk_digest,
            "managed artifact digest changed"
        );
        let restored = Session::restore(&changed, saved)?;
        report["gates"]["managed_restore_completed"] = json!(true);
        budget.check()?;
        total.check()?;
        ensure!(
            replacement(&restored)? == &retained,
            "retained manifest/order/provenance changed"
        );
        ensure!(
            cursor(&restored, instruction)? == retained_cursor,
            "retained cursor/frame changed"
        );
        report["gates"]["retained_manifest_vector_cursor_provenance_equal"] = json!(true);
        let after = full_state(&restored)?;
        ensure!(
            after == before && rng(&restored) == rng(owner),
            "cold full state/RNG/continuation changed"
        );
        report["cold_comparison"] = json!({"complete_state_sha256":digest(&before),
            "restored_state_sha256":digest(&after),"rng":rng(&restored),"cursor":retained_cursor,
            "method":"exact postcard bytes of (Session::save(), imported House); all float bits retained",
            "program_reused":false,"restore":"public Session::restore(changed Index, disk-decoded managed Saved)"});
        budget.check()?;
        total.check()?;
        report["gates"]["cold_state_rng_continuation_equal"] = json!(true);
        Ok(restored)
    })();
    let status = restored
        .as_ref()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error:#}"));
    end(report, &budget, &status);
    let mut restored = restored?;
    let mut budget = Budget::new(
        "next_boundaries",
        args.compare_seconds,
        args.compare_steps * 2,
    );
    begin(args, report, &budget)?;
    let result = (|| {
        let mut joined_trace = prefix;
        for boundary_index in 0..args.compare_steps {
            budget.take(total)?;
            let mut uninterrupted_trace = Vec::new();
            let uninterrupted = checked_advance(
                owner,
                Some(&mut uninterrupted_trace),
                "uninterrupted cut owner",
                args,
                report,
            )?;
            budget.take(total)?;
            let mut restored_trace = Vec::new();
            let resumed = checked_advance(
                &mut restored,
                Some(&mut restored_trace),
                "cold restored owner",
                args,
                report,
            )?;
            ensure!(
                uninterrupted_trace.len() <= MAX_TRACE_ROWS
                    && restored_trace.len() <= MAX_TRACE_ROWS,
                "comparison trace row limit exceeded"
            );
            let uninterrupted_state = full_state(owner)?;
            let restored_state = full_state(&restored)?;
            let equal = uninterrupted == resumed
                && uninterrupted_trace == restored_trace
                && uninterrupted_state == restored_state
                && rng(owner) == rng(&restored);
            report["boundaries"].as_array_mut().unwrap().push(json!({"index":boundary_index,
                "equal":equal,"boundary":uninterrupted,"restored_boundary":resumed,
                "uninterrupted_sha256":digest(&uninterrupted_state),"restored_sha256":digest(&restored_state),
                "trace_sha256":digest(&serde_json::to_vec(&uninterrupted_trace)?),
                "restored_trace_sha256":digest(&serde_json::to_vec(&restored_trace)?),
                "rng":rng(owner),"restored_rng":rng(&restored),"tick":owner.simulation.vm.tick}));
            ensure!(equal, "cold next boundary {boundary_index} diverged");
            if boundary_index == 0 {
                joined_trace.extend(uninterrupted_trace);
                ensure!(
                    joined_trace == located.trace
                        && uninterrupted == located.boundary
                        && uninterrupted_state == located.state,
                    "small-budget cut did not rejoin the original full Session boundary"
                );
                report["gates"]["small_budget_rejoins_session_boundary"] = json!(true);
                joined_trace.clear();
            }
            budget.check()?;
            total.check()?;
        }
        report["gates"]["next_boundaries_equal"] = json!(true);
        Ok(())
    })();
    end(report, &budget, &result);
    result
}

fn validate(args: &Args) -> Result<()> {
    ensure!(
        args.policy == INSTALLED_HOUSES_ASCENDING_V1,
        "explicit supported --policy required"
    );
    ensure!(
        args.search_from_tick < args.search_until_tick,
        "empty search window"
    );
    ensure!(
        args.search_until_tick < args.stop_after_tick,
        "search must precede final target"
    );
    ensure!(
        args.stop_after_tick > ORIGINAL_FAULT_TICK,
        "target must exceed original fault tick"
    );
    ensure!(
        (1..=10_000).contains(&args.max_ticks),
        "max-ticks must be 1..10000"
    );
    ensure!(
        (1..=100_000).contains(&args.max_steps),
        "max-steps must be 1..100000"
    );
    ensure!(
        (1..=128).contains(&args.search_steps),
        "search-steps must be 1..128"
    );
    ensure!(
        (1..=200_000).contains(&args.probe_steps),
        "probe-steps must be 1..200000"
    );
    ensure!(
        (2..=16).contains(&args.compare_steps),
        "compare-steps must be 2..16"
    );
    ensure!(
        args.start.is_none() || !args.headless_live_input,
        "restored inputs must remain untouched; omit --headless-live-input"
    );
    for seconds in [
        args.setup_seconds,
        args.sustain_seconds,
        args.search_seconds,
        args.probe_seconds,
        args.restore_seconds,
        args.compare_seconds,
        args.total_seconds,
    ] {
        ensure!(
            seconds.is_finite() && (0.1..=600.0).contains(&seconds),
            "wall budgets must be 0.1..600 seconds"
        );
    }
    Ok(())
}

fn run(args: &Args, total: &Budget, report: &mut Value) -> Result<()> {
    let mut budget = Budget::new("setup", args.setup_seconds, 3);
    begin(args, report, &budget)?;
    let loaded = (|| -> Result<(Index, Session)> {
        budget.take(total)?;
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game-data/cabinet-extracted");
        let index = Index::scan(&root)?;
        budget.take(total)?;
        let session = if let Some(start) = &args.start {
            let (saved, input_sha256) = read_saved(start)?;
            report["start"] = json!({"path":start,"version":saved.version,
                "sha256":input_sha256,"tick":saved.simulation.state.vm.tick,"source":saved.source});
            let session = Session::restore(&index, saved)?;
            report["gates"]["validated_start_restore"] = json!(true);
            session
        } else {
            let session = Session::load(
                &index,
                &args.source,
                Options {
                    routing: match args.routing {
                        Routing::Strict => sims_sim::lot_route::path::Policy::Strict,
                        Routing::InferredRectangles => {
                            sims_sim::lot_route::path::Policy::InferredRectangles
                        }
                    },
                    ..Default::default()
                },
            )?;
            report["gates"]["public_session_load"] = json!(true);
            session
        };
        budget.check()?;
        total.check()?;
        Ok((index, session))
    })();
    let status = loaded
        .as_ref()
        .map(|_| ())
        .map_err(|error| anyhow::anyhow!("{error:#}"));
    end(report, &budget, &status);
    let (index, mut session) = loaded?;
    report["initial_state"] = summary(&session);
    report["actual_options"] = json!(session.options);
    let initial_tick = session.simulation.vm.tick;
    let initial_calendar = calendar(&session);
    let result = (|| {
        if faults(&session) != json!([]) {
            note_failure(
                &session,
                "unmodified start",
                "faulted start rejected before binding",
                &args.output,
                report,
            )?;
            bail!("faulted start cannot receive replacement input");
        }
        ensure!(
            session.house.source.eq_ignore_ascii_case(&args.source),
            "start house differs from --source"
        );
        ensure!(
            initial_tick < ORIGINAL_FAULT_TICK,
            "start must precede the preserved missing-input fault tick"
        );
        ensure!(
            initial_tick <= args.search_from_tick,
            "start is later than search window"
        );
        ensure!(
            args.stop_after_tick - initial_tick <= args.max_ticks,
            "requested span exceeds max-ticks"
        );
        ensure!(
            args.search_until_tick - args.search_from_tick <= 64,
            "trace search window exceeds 64 ticks"
        );
        report["stage"] = json!("legal_policy_binding");
        write_report(&args.output, report)?;
        match session.neighborhood_input_status()? {
            Status::Missing => {
                session.bind_installed_houses_ascending_v1()?;
                report["gates"]["policy_bound_legally"] = json!(true);
            }
            Status::Replacement(retained) => {
                ensure!(
                    retained.policy_id() == args.policy,
                    "retained policy conflict"
                );
                report["gates"]["retained_policy_reused"] = json!(true);
            }
            status => bail!("cannot upgrade conflicting neighborhood input: {status:?}"),
        }
        report["binding"] = json!({"tick":session.simulation.vm.tick,
            "replacement":replacement(&session)?,"faults":faults(&session),
            "safe_guard":"public binder accepted, or retained replacement validated without rebinding"});
        if args.headless_live_input {
            let mode = session
                .simulation
                .vm
                .world
                .globals
                .get_mut(&19)
                .context("mode global absent")?;
            let previous = *mode;
            *mode = 0;
            report["inputs"].as_array_mut().unwrap().push(json!({"global":19,"previous":previous,"value":0,
                "scope":"explicit Session/headless Live input; NOT fresh GUI Play/window success"}));
        }
        report["bound_state"] = summary(&session);
        sustain(
            &mut session,
            args.search_from_tick,
            "prelude",
            args,
            total,
            report,
        )?;
        if let Some((mut located, instruction, prefix)) = locate(&mut session, args, total, report)?
        {
            let gate_result = cold_compare(
                &mut located,
                &instruction,
                prefix,
                &index,
                args,
                total,
                report,
            );
            if let Err(error) = gate_result {
                if faults(&located.owner) != json!([]) {
                    note_failure(
                        &located.owner,
                        "cut owner",
                        &format!("{error:#}"),
                        &args.output,
                        report,
                    )?;
                }
                let failure = json!({"stage":report["stage"],"error":format!("{error:#}"),
                    "owner_state":summary(&located.owner)});
                report["unsupported_gates"]
                    .as_array_mut()
                    .unwrap()
                    .push(failure);
                write_new(
                    &args.output.join("cut-owner-final.json"),
                    &serde_json::to_vec(&located.owner.save())?,
                )?;
            }
        } else {
            report["unsupported_gates"].as_array_mut().unwrap().push(json!({
                "stage":"mode11_search","error":"no qualifying interior mode11 cut within bounded natural window; no midscan/restore pass"}));
        }
        sustain(
            &mut session,
            args.stop_after_tick,
            "postlude",
            args,
            total,
            report,
        )?;
        Ok(())
    })();
    report["final_state"] = summary(&session);
    let progressed = session.simulation.vm.tick > ORIGINAL_FAULT_TICK
        && session.simulation.vm.tick > initial_tick
        && initial_calendar
            .zip(calendar(&session))
            .is_some_and(|(before, after)| after > before);
    report["gates"]["sustained_past_original_fault_tick"] = json!(progressed);
    report["completed_ticks"] = json!(session.simulation.vm.tick.saturating_sub(initial_tick));
    let bytes = serde_json::to_vec(&session.save())?;
    report["final_session_sha256"] = json!(digest(&bytes));
    write_report(&args.output, report)?;
    write_new(&args.output.join("final-session.json"), &bytes)?;
    result
}

fn main() -> Result<()> {
    let args = Args::parse();
    validate(&args)?;
    fs::create_dir(&args.output)
        .context("output must be a new directory with an existing parent")?;
    let total = Budget::new("total", args.total_seconds, usize::MAX);
    let mut report = json!({"schema":1,"status":"running","success":false,"arguments":args,
        "execution_scope":"Session/headless; public budgeted Simulation diagnostic only at traced cut",
        "claims":{"gui_play":false,"native_window":false,"responsiveness":false,"parity":false,
            "native_original_equality_admissible":false,"speedup":false},
        "original_strict_missing_input_fault_tick":ORIGINAL_FAULT_TICK,
        "first_natural_failure":null,"fatal_error":null,"unsupported_gates":[],
        "ordinary_steps":0,"stages":{},"inputs":[],"boundaries":[],"probe_attempts":[],
        "gates":{"public_session_load":false,"validated_start_restore":false,
            "policy_bound_legally":false,"retained_policy_reused":false,
            "sustained_past_original_fault_tick":false,"mode11_encountered":false,"midscan_obtained":false,
            "changed_inventory_proven":false,"managed_save_completed":false,"managed_restore_completed":false,
            "retained_manifest_vector_cursor_provenance_equal":false,"cold_state_rng_continuation_equal":false,
            "small_budget_rejoins_session_boundary":false,"next_boundaries_equal":false}});
    write_report(&args.output, &report)?;
    let result = run(&args, &total, &mut report).and_then(|()| total.check());
    if let Err(error) = &result {
        report["fatal_error"] = json!({"stage":report["stage"],"error":format!("{error:#}")});
    }
    let required = [
        "sustained_past_original_fault_tick",
        "mode11_encountered",
        "midscan_obtained",
        "changed_inventory_proven",
        "managed_save_completed",
        "managed_restore_completed",
        "retained_manifest_vector_cursor_provenance_equal",
        "cold_state_rng_continuation_equal",
        "small_budget_rejoins_session_boundary",
        "next_boundaries_equal",
    ];
    let success = result.is_ok()
        && report["first_natural_failure"].is_null()
        && report["unsupported_gates"].as_array().unwrap().is_empty()
        && required.iter().all(|gate| report["gates"][*gate] == true);
    report["success"] = json!(success);
    report["status"] = json!("terminal");
    report["total"] = total.receipt();
    write_report(&args.output, &report)?;
    println!(
        "{}",
        json!({"success":success,"report":args.output.join("report.json"),
        "gates":report["gates"],"fatal_error":report["fatal_error"]})
    );
    result?;
    ensure!(
        success,
        "one or more continuation gates remain unqualified; see report.json"
    );
    Ok(())
}
