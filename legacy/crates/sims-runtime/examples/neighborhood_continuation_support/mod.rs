use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_runtime::original_session::{Progress, Saved, Session};
use sims_vm::{Status, Trace};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::Instant,
};

pub const MAX_SAVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_TRACE_ROWS: usize = 250_000;

pub struct Budget {
    pub label: &'static str,
    pub started: Instant,
    pub seconds: f64,
    pub calls: usize,
    pub max_calls: usize,
}

impl Budget {
    pub fn new(label: &'static str, seconds: f64, max_calls: usize) -> Self {
        Self {
            label,
            started: Instant::now(),
            seconds,
            calls: 0,
            max_calls,
        }
    }

    pub fn check(&self) -> Result<()> {
        ensure!(
            self.started.elapsed().as_secs_f64() < self.seconds,
            "{} cooperative wall budget exhausted",
            self.label
        );
        Ok(())
    }

    pub fn take(&mut self, total: &Self) -> Result<()> {
        total.check()?;
        self.check()?;
        ensure!(
            self.calls < self.max_calls,
            "{} call budget exhausted",
            self.label
        );
        self.calls += 1;
        Ok(())
    }

    pub fn receipt(&self) -> Value {
        json!({"calls":self.calls,"max_calls":self.max_calls,
            "wall_seconds":self.started.elapsed().as_secs_f64(),"limit_seconds":self.seconds})
    }
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn read_saved(path: &Path) -> Result<(Saved, String)> {
    ensure!(
        fs::metadata(path)?.len() <= MAX_SAVE_BYTES,
        "save input exceeds 512 MiB"
    );
    let bytes = fs::read(path)?;
    ensure!(bytes.len() as u64 <= MAX_SAVE_BYTES, "save input grew");
    let saved = serde_json::from_slice(&bytes).context("decode unmodified Session save")?;
    Ok((saved, digest(&bytes)))
}

pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(
        bytes.len() as u64 <= MAX_SAVE_BYTES,
        "artifact exceeds 512 MiB"
    );
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

pub fn write_report(output: &Path, report: &Value) -> Result<()> {
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

pub fn full_state(session: &Session) -> Result<Vec<u8>> {
    postcard::to_allocvec(&(session.save(), &session.house))
        .map_err(|error| anyhow::anyhow!("encode complete continuation: {error}"))
}

pub fn calendar(session: &Session) -> Option<i64> {
    session.simulation.household.as_ref().map(|household| {
        let clock = &household.clock;
        i64::from(clock.elapsed_days) * 86_400
            + i64::from(clock.hour) * 3600
            + i64::from(clock.minute) * 60
            + i64::from(clock.second)
    })
}

pub fn rng(session: &Session) -> Value {
    let vm = &session.simulation.vm;
    json!({"global":vm.rng_state,"lot":vm.lot_rng_state,"crt":vm.crt_rng_state,
        "lot_scope":vm.lot_rng_scope})
}

pub fn faults(session: &Session) -> Value {
    let rows: Vec<_> = session
        .simulation
        .vm
        .threads
        .iter()
        .filter_map(|(thread, state)| match &state.status {
            Status::Faulted(fault) => Some(json!({"thread":thread,"fault":fault})),
            _ => None,
        })
        .collect();
    json!(rows)
}

pub fn summary(session: &Session) -> Value {
    let vm = &session.simulation.vm;
    json!({"tick":vm.tick,"calendar_seconds":calendar(session),"rng":rng(session),
        "global17":vm.world.globals.get(&17),"global19":vm.world.globals.get(&19),
        "faults":faults(session),"modal":vm.world.dialogs.modal,
        "registry_scheduler_retained":session.simulation.native_scheduler.is_some(),
        "visit_retained":session.simulation.visit.is_some(),
        "scheduled_object_visit_retained":vm.scheduled_object_visit.is_some()})
}

fn answer_startup_dialog(session: &mut Session) -> Result<Value> {
    let record = session
        .simulation
        .vm
        .world
        .dialogs
        .records
        .iter()
        .find(|record| Some(record.id) == session.simulation.vm.world.dialogs.modal)
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
    let button = record
        .buttons
        .iter()
        .find(|button| notice || (help && button.label.eq_ignore_ascii_case("no")))
        .with_context(|| format!("unsupported natural modal: {record:?}"))?;
    let input = json!({"id":record.id,"origin":record.origin,
        "label":button.label,"result":button.result,
        "scope":"Session/headless pinned startup response; not GUI input"});
    session
        .simulation
        .vm
        .answer_dialog(record.id, button.result)
        .map_err(anyhow::Error::msg)?;
    Ok(input)
}

pub fn advance(session: &mut Session, trace: Option<&mut Vec<Trace>>) -> Result<Value> {
    let progress = match trace {
        Some(trace) => session.step_with_trace(trace)?,
        None => session.step()?,
    };
    let input = if progress == Progress::Dialog {
        Some(answer_startup_dialog(session)?)
    } else {
        ensure!(
            matches!(progress, Progress::Tick | Progress::PartialPass),
            "unsupported natural Session boundary: {progress:?}"
        );
        None
    };
    Ok(json!({"progress":format!("{progress:?}"),"input":input}))
}

pub fn mode11(trace: &Trace) -> bool {
    trace.opcode == 0x1f && trace.operands[4] & 0x7f == 11
}

pub fn cursor(session: &Session, trace: &Trace) -> Result<Value> {
    let thread = session
        .simulation
        .vm
        .threads
        .get(&trace.thread)
        .context("mode11 thread no longer retained")?;
    let frame = thread.stack.last().context("mode11 frame returned")?;
    ensure!(
        thread.stack.len() == trace.depth
            && frame.location.source == trace.location.source
            && frame.location.bhav == trace.location.bhav,
        "mode11 destination frame no longer current"
    );
    let (scope, index) = if trace.operands[4] & 0x80 == 0 {
        (10, 0)
    } else {
        (trace.operands[5], i16::from(trace.operands[7]))
    };
    let probe = session
        .simulation
        .vm
        .inspect_memory(&session.program, trace.thread, scope, index, true)
        .map_err(anyhow::Error::msg)?;
    ensure!(
        probe.writable && !probe.float_pointer,
        "cursor is not a word pointer"
    );
    Ok(
        json!({"thread":trace.thread,"scope":scope,"index":index,"value":probe.value,
        "frame":frame,"status":thread.status}),
    )
}

pub fn note_failure(
    session: &Session,
    owner: &str,
    error: &str,
    output: &Path,
    report: &mut Value,
) -> Result<()> {
    if report["first_natural_failure"].is_null() {
        let bytes = serde_json::to_vec(&session.save())?;
        report["first_natural_failure"] = json!({"owner":owner,"error":error,
            "state":summary(session),"artifact":"first-natural-failure.json",
            "sha256":digest(&bytes)});
        write_report(output, report)?;
        write_new(&output.join("first-natural-failure.json"), &bytes)?;
    }
    Ok(())
}
