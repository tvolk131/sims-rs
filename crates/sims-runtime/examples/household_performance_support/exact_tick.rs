use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_runtime::original_session::{Progress, Session};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const SOURCE: &str = "UserData2/Houses/House09.iff";

pub struct Capture {
    output: PathBuf,
    nonce: String,
    binary_sha256: String,
    epoch: Instant,
    before: Value,
    before_ns: u128,
    pub error: Option<String>,
    returned: bool,
}

fn boundary(session: &Session) -> Value {
    let vm = &session.simulation.vm;
    json!({"tick":vm.tick,"rng":vm.rng_state,"lot_rng":vm.lot_rng_state,
        "crt_rng":vm.crt_rng_state,"global17":vm.world.globals.get(&17),
        "global19":vm.world.globals.get(&19)})
}

#[inline(never)]
fn profile_target_step(session: &mut Session, epoch: Instant) -> (Result<Progress>, u128, u128) {
    let start_ns = epoch.elapsed().as_nanos();
    let result = session.step();
    let end_ns = epoch.elapsed().as_nanos();
    (result, start_ns, end_ns)
}

impl Capture {
    pub fn new(output: &Path, session: &Session) -> Result<Self> {
        let nonce = std::env::var("SIMS_EXACT_TICK_NONCE")?;
        let binary_sha256 = std::env::var("SIMS_EXACT_TICK_BINARY_SHA256")?;
        ensure!(
            nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "exact-tick nonce must be 32 hex characters"
        );
        ensure!(
            binary_sha256.len() == 64 && binary_sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "exact-tick binary identity must be SHA-256"
        );
        let initial = boundary(session);
        ensure!(
            session.house.source == SOURCE
                && initial["tick"] == 178126
                && initial["global17"] == 0
                && initial["global19"] == 0,
            "exact-tick requires fresh Mashuga tick 178126, global17=0 and explicit headless global19=0"
        );
        Ok(Self {
            output: output.to_owned(),
            nonce,
            binary_sha256,
            epoch: Instant::now(),
            before: Value::Null,
            before_ns: 0,
            error: None,
            returned: false,
        })
    }

    fn message(&self, kind: &str) -> Value {
        json!({"schema":1,"kind":kind,"pid":std::process::id(),"nonce":self.nonce,
            "binary_sha256":self.binary_sha256,"source":SOURCE,"call_1based":11,
            "tick_before":178131})
    }

    #[inline(never)]
    fn profile_publish(&self, name: &str, value: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() <= 65536, "exact-tick message exceeds 64 KiB");
        let temporary = self.output.join(format!(".{name}.tmp"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_data()?;
        fs::rename(temporary, self.output.join(name))?;
        Ok(())
    }

    #[inline(never)]
    fn profile_gate_wait(&self, name: &str, kind: &str, seconds: u64) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let path = self.output.join(name);
        loop {
            if path.try_exists()? {
                ensure!(
                    fs::metadata(&path)?.len() <= 65536,
                    "oversized exact-tick gate"
                );
                let received: Value = serde_json::from_slice(&fs::read(&path)?)?;
                ensure!(
                    received == self.message(kind),
                    "exact-tick gate identity mismatch"
                );
                return Ok(());
            }
            ensure!(Instant::now() < deadline, "exact-tick {kind} timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn fail(&mut self, error: impl std::fmt::Display) {
        if self.error.is_none() {
            self.error = Some(error.to_string());
        }
    }

    pub fn prepare(&mut self, session: &Session, rows: &[Value]) -> Result<bool> {
        ensure!(rows.len() < 32, "exact-tick 32-call cap reached");
        let call = rows.len() + 1;
        let expected_tick = 178126 + call.saturating_sub(6) as u64;
        ensure!(
            call <= 11 && session.simulation.vm.tick == expected_tick,
            "exact-tick prefix/call mismatch before call {call}"
        );
        if call != 11 {
            return Ok(false);
        }
        let notices = [
            "HelpSystemMagic.iff",
            "HDHelpSystem.iff",
            "NeighborhoodHelpSystem.iff",
            "HelpSystemStudioTown.iff",
            "VacationHelpSystem.iff",
        ];
        for (offset, row) in rows.iter().enumerate() {
            if offset < 5 {
                ensure!(
                    row["tick_before"] == 178126
                        && row["tick_after"] == 178126
                        && row["outcome"] == format!("Dialog({},2)", 13 + offset)
                        && row["response"]["bhav"] == 4119
                        && row["response"]["result"] == 2
                        && row["response"]["source"]
                            .as_str()
                            .is_some_and(|source| source.ends_with(notices[offset])),
                    "exact-tick notice prefix mismatch at call {}",
                    offset + 1
                );
            } else {
                ensure!(
                    row["tick_before"] == 178126 + offset as u64 - 5
                        && row["tick_after"] == 178127 + offset as u64 - 5
                        && row["outcome"] == "Tick"
                        && row["response"].is_null(),
                    "exact-tick tick prefix mismatch at call {}",
                    offset + 1
                );
            }
        }
        self.before = boundary(session);
        self.before_ns = self.epoch.elapsed().as_nanos();
        let mut message = self.message("step-ready");
        message["boundary_before"] = self.before.clone();
        message["ready_boundary_ns"] = json!(self.before_ns);
        self.profile_publish("step-ready.json", &message)?;
        self.profile_gate_wait("step-go.json", "step-go", 10)?;
        Ok(true)
    }

    pub fn step(&mut self, session: &mut Session) -> (Result<Progress>, f64) {
        self.before = boundary(session);
        self.before_ns = self.epoch.elapsed().as_nanos();
        let (result, start_ns, end_ns) = profile_target_step(session, self.epoch);
        let after = boundary(session);
        let after_ns = self.epoch.elapsed().as_nanos();
        self.return_step(result, after, start_ns, end_ns, after_ns)
    }

    fn return_step(
        &mut self,
        result: Result<Progress>,
        after: Value,
        start_ns: u128,
        end_ns: u128,
        after_ns: u128,
    ) -> (Result<Progress>, f64) {
        let tick_after = after["tick"].as_u64();
        let original_error = result.as_ref().err().map(|error| format!("{error:#}"));
        let mut message = self.message("step-returned");
        message["boundary_before"] = self.before.clone();
        message["boundary_after"] = after;
        message["boundary_before_ns"] = json!(self.before_ns);
        message["call_start_ns"] = json!(start_ns);
        message["call_end_ns"] = json!(end_ns);
        message["boundary_after_ns"] = json!(after_ns);
        message["outcome"] = json!(match &result {
            Ok(progress) => format!("{progress:?}"),
            Err(_) => "Error".to_owned(),
        });
        message["original_step_error"] = json!(original_error);
        self.returned = true;
        if let Err(error) = self.profile_publish("step-returned.json", &message) {
            self.fail(format!("step-returned: {error:#}"));
            return (result, (end_ns - start_ns) as f64 / 1_000_000_000.);
        }
        if let Err(error) = self.profile_gate_wait("step-finish.json", "step-finish", 14) {
            self.fail(format!("step-finish: {error:#}"));
        }
        if !matches!(result, Ok(Progress::Tick)) || tick_after != Some(178132) {
            self.fail("target did not return Tick at 178132");
        }
        (result, (end_ns - start_ns) as f64 / 1_000_000_000.)
    }

    pub fn flush(&mut self, rows: &[Value], original_error: &Option<String>) {
        let message = json!({"schema":1,"pid":std::process::id(),"nonce":self.nonce,
            "steps":rows.len(),"last_step":rows.last(),"original_step_error":original_error,
            "capture_error":self.error,"target_returned":self.returned,
            "substage_spans_captured":false,"service_counts_captured":false,
            "animation_batches_captured":false});
        if let Err(error) = self.profile_publish("original-step-outcome.json", &message) {
            self.fail(format!("original-step-outcome: {error:#}"));
        }
    }

    pub fn finish(&mut self, rows: &[Value], original_error: &Option<String>) -> Result<()> {
        if rows.len() != 11 || !self.returned {
            self.fail("exact-tick requires eleven calls and one target return");
        }
        self.flush(rows, original_error);
        self.error.as_ref().map_or(Ok(()), |error| {
            Err(anyhow::anyhow!(error.clone())).context("exact-tick capture failed")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_return_publication_preserves_original_error_without_parking() -> Result<()> {
        let epoch = Instant::now();
        let output = std::env::temp_dir().join(format!(
            "sims-exact-tick-return-{}-{}",
            std::process::id(),
            epoch.elapsed().as_nanos()
        ));
        fs::create_dir(&output)?;
        let result = (|| -> Result<()> {
            fs::write(output.join(".step-returned.json.tmp"), b"occupied")?;
            let mut capture = Capture {
                output: output.clone(),
                nonce: "0".repeat(32),
                binary_sha256: "0".repeat(64),
                epoch,
                before: json!({"tick":178131}),
                before_ns: 1,
                error: None,
                returned: false,
            };
            let (returned, seconds) = capture.return_step(
                Err(anyhow::anyhow!("original synthetic step failure")),
                json!({"tick":178131}),
                2,
                22,
                23,
            );
            let original = returned.err().context("original error lost")?;
            ensure!(format!("{original:#}") == "original synthetic step failure");
            ensure!(seconds == 20. / 1_000_000_000.);
            ensure!(capture.returned && capture.error.as_ref().unwrap().contains("step-returned"));
            capture.flush(&[], &Some(format!("{original:#}")));
            let receipt: Value =
                serde_json::from_slice(&fs::read(output.join("original-step-outcome.json"))?)?;
            ensure!(receipt["original_step_error"] == "original synthetic step failure");
            ensure!(receipt["capture_error"].is_string());
            ensure!(!output.join("step-finish.json").exists());
            Ok(())
        })();
        fs::remove_file(output.join(".step-returned.json.tmp"))?;
        if output.join("original-step-outcome.json").exists() {
            fs::remove_file(output.join("original-step-outcome.json"))?;
        }
        fs::remove_dir(&output)?;
        result
    }
}
