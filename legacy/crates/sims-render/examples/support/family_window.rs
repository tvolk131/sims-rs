//! Explicit source-bound callback-body events, never a renderer getter/cadence.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sims_runtime::original_session::{
    Session,
    window_update::{ApplicationFamily, Binding, Progress, State},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub serial: u64,
    pub observation: String,
    pub raw_argument: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HashedFile {
    path: PathBuf,
    sha256: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    capture: HashedFile,
    verification: HashedFile,
    entry_line: usize,
    return_line: usize,
    next_poll_line: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u32,
    binding: Binding,
    observations: BTreeMap<String, Source>,
}
#[derive(Clone, Debug)]
struct Observation {
    source: Source,
    raw_argument: u32,
    tick: u64,
    lot: i16,
    family: i16,
    count: i32,
    dirty: bool,
    final_count: i32,
    final_dirty: bool,
    initial_text: Vec<u8>,
    final_text: Vec<u8>,
    initial_label: Vec<u8>,
    final_label: Vec<u8>,
    words: [u32; 12],
    loaded: u8,
    return_al: u8,
    numeric_getters: usize,
}
pub struct Replay {
    binding: Binding,
    observations: BTreeMap<String, Observation>,
    used: BTreeSet<String>,
    next_serial: u64,
    pub journal: Vec<Value>,
}
/// Pure retained ownership export; missing state stays unknown.
pub fn retained(state: Option<&State>) -> Value {
    json!({"schema":1,"state":state})
}
pub fn startup_events(value: Option<&Value>, bound: bool) -> Result<Vec<Event>> {
    let events: Vec<Event> = value
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    ensure!(
        events.is_empty() || bound,
        "startup callback events are unbound"
    );
    Ok(events)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn checked(bytes: &[u8], expected: &str) -> Result<()> {
    ensure!(
        expected.len() == 64
            && expected
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "source digest domain"
    );
    ensure!(hash(bytes) == expected, "source digest mismatch");
    Ok(())
}
fn read(file: &HashedFile) -> Result<Vec<u8>> {
    ensure!(
        file.path.is_relative()
            && file
                .path
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_))),
        "source path must be a plain relative path"
    );
    ensure!(
        file.path.starts_with("cache"),
        "private original evidence required"
    );
    ensure!(
        fs::metadata(&file.path)?.len() <= 8 * 1024 * 1024,
        "source evidence observer bound"
    );
    let bytes = fs::read(&file.path)?;
    checked(&bytes, &file.sha256)?;
    Ok(bytes)
}
fn uint(v: &Value) -> Result<u64> {
    v.as_u64()
        .context("unsigned source field missing or out of domain")
}
fn hex(v: &Value) -> Result<Vec<u8>> {
    let s = v.as_str().context("raw hex field absent")?;
    ensure!(s.len() % 2 == 0 && s.is_ascii(), "raw hex domain");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(Into::into))
        .collect()
}
fn word(bytes: &[u8], offset: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .context("raw owner truncated")?
            .try_into()?,
    ))
}
fn counted(row: &Value) -> Result<Vec<u8>> {
    let mut bytes = hex(&row["cpstate"]["text_with_nul"])?;
    ensure!(
        bytes.pop() == Some(0) && !bytes.contains(&0),
        "counted string is not one terminated CString"
    );
    ensure!(
        uint(&row["cpstate"]["length"])? == bytes.len() as u64,
        "counted length mismatch"
    );
    Ok(bytes)
}
fn observe(source: &Source, capture: &[u8], verification: &[u8]) -> Result<Observation> {
    checked(capture, &source.capture.sha256)?;
    checked(verification, &source.verification.sha256)?;
    ensure!(
        source.entry_line > 0
            && source.entry_line < source.return_line
            && source.return_line < source.next_poll_line,
        "source call order"
    );
    let proof: Value = serde_json::from_slice(verification)?;
    ensure!(
        proof["capture_sha256"] == source.capture.sha256,
        "verifier capture identity"
    );
    let rows: Vec<_> = capture.split(|b| *b == b'\n').collect();
    let get = |line: usize, kind: &str| -> Result<Value> {
        let raw = rows.get(line - 1).context("source row missing")?;
        let row: Value = serde_json::from_slice(raw)?;
        ensure!(row["kind"] == kind, "source row kind mismatch");
        ensure!(
            proof["evidence_rows"]
                .as_array()
                .context("verifier rows missing")?
                .iter()
                .any(|r| r["line"] == line && r["kind"] == kind && r["sha256"] == hash(raw)),
            "source row is not verified"
        );
        Ok(row)
    };
    let entry = get(source.entry_line, "window_entry")?;
    let returned = get(source.return_line, "window_return")?;
    let poll = get(source.next_poll_line, "ui_poll_entry")?;
    let stack = hex(&entry["stack"])?;
    ensure!(
        uint(&entry["registers"]["eip"])? == 0x53a01f,
        "wrong callback entry"
    );
    let raw_argument = word(&stack, 4)?;
    let tick = uint(&entry["tick"])?;
    let lot = i16::try_from(uint(&entry["lot"])?)?;
    ensure!(
        returned["tick"] == tick
            && poll["tick"] == tick
            && returned["lot"] == lot
            && poll["lot"] == lot,
        "observed callback crossed tick/lot"
    );
    ensure!(
        entry["rng"] == returned["rng"] && entry["process"] == returned["process"],
        "source callback changed RNG/process"
    );
    let before = hex(&entry["application_family"]["storage"])?;
    let after = hex(&returned["application_family"]["storage"])?;
    let family = i16::try_from(word(&before, 0x10c)? as i32)?;
    ensure!(
        word(&after, 0x10c)? as i32 == i32::from(family),
        "source family identity changed"
    );
    let raw = hex(&entry["cpstate"]["raw"])?;
    ensure!(
        raw.get(0x23e..0x240) == Some(&256u16.to_le_bytes()),
        "unsupported captured capacity"
    );
    ensure!(
        hex(&returned["cpstate"]["raw"])?.get(0x23e..0x240) == Some(&256u16.to_le_bytes()),
        "captured return capacity changed"
    );
    ensure!(
        entry["cpstate"]["buffer"] == returned["cpstate"]["buffer"],
        "source buffer identity changed"
    );
    let process = &entry["process"];
    let words = process["words"]
        .as_array()
        .context("source process words absent")?;
    ensure!(words.len() == 12, "source process shape");
    let words: Vec<_> = words
        .iter()
        .zip(sims_vm::autonomy_constants::ORDER)
        .map(|(w, field)| {
            ensure!(
                uint(&w["address"])? == u64::from(field.address()),
                "source process address/order differs"
            );
            let raw = hex(&w["raw"])?;
            ensure!(raw.len() == 4, "source process word extent");
            word(&raw, 0)
        })
        .collect::<Result<_>>()?;
    let result = Observation {
        source: source.clone(),
        raw_argument,
        tick,
        lot,
        family,
        count: word(&before, 0x120)? as i32,
        dirty: *before.get(0x124).context("source dirty missing")? != 0,
        final_count: word(&after, 0x120)? as i32,
        final_dirty: *after.get(0x124).context("source final dirty missing")? != 0,
        initial_text: counted(&entry)?,
        final_text: counted(&returned)?,
        initial_label: hex(&entry["label"]["text"]["bytes"])?,
        final_label: hex(&returned["label"]["text"]["bytes"])?,
        words: words.try_into().unwrap(),
        loaded: u8::try_from(uint(&process["loaded"])?)?,
        return_al: u32::try_from(uint(&returned["registers"]["eax"])?)? as u8,
        numeric_getters: usize::try_from(uint(&proof["summary"]["numeric_getters"])?)?,
    };
    ensure!(
        proof["summary"]["raw_argument"] == result.raw_argument
            && proof["summary"]["family_id"] == result.family
            && proof["summary"]["cpstate_capacity"] == 256
            && proof["summary"]["callback_return_al"] == result.return_al,
        "source verifier summary mismatch"
    );
    Ok(result)
}
fn logical_text(state: &State, owner: u32) -> Result<(Vec<u8>, u32)> {
    let value = serde_json::to_value(state)?;
    let counted = &value["counted"][owner.to_string()];
    let buffer = u32::try_from(uint(&counted["buffer"])?)?;
    let bytes: Vec<u8> =
        serde_json::from_value(value["buffers"][buffer.to_string()]["bytes"].clone())?;
    ensure!(
        uint(&counted["length"])? == bytes.len() as u64,
        "logical counted length mismatch"
    );
    Ok((bytes, buffer))
}
fn process_guard(world: &sims_vm::World, o: &Observation) -> Result<()> {
    let p = &world
        .autonomy_constants
        .as_ref()
        .context("retained process is unknown; callback input cannot repair it")?
        .process;
    ensure!(
        p.loaded == Some(o.loaded) && p.words == o.words.map(Some),
        "callback source process words differ or are unknown"
    );
    Ok(())
}
fn neutral(s: &Session) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(
        s.simulation.vm.tick,
        s.simulation.vm.rng_state,
        s.simulation.vm.lot_rng_state,
        s.simulation.vm.crt_rng_state,
        &s.simulation.vm.world.autonomy_constants,
        &s.host.autonomy_resources,
    ))?)
}
fn owner_guard(session: &Session, o: &Observation, after: bool) -> Result<()> {
    ensure!(
        session.simulation.vm.tick == o.tick
            && session.simulation.vm.world.globals.get(&10) == Some(&o.lot),
        "callback source tick/lot mismatch"
    );
    process_guard(&session.simulation.vm.world, o)?;
    let people = session
        .simulation
        .vm
        .world
        .person
        .as_ref()
        .context("family owner absent")?;
    ensure!(
        people.controlled_family == o.family
            && session.house.household.as_ref().map(|h| i32::from(h.id))
                == Some(i32::from(o.family)),
        "callback application household differs"
    );
    let meta = people
        .family_metadata
        .get(&o.family)
        .context("family metadata absent")?;
    ensure!(
        meta.friend_count == Some(if after { o.final_count } else { o.count })
            && meta.dirty == if after { o.final_dirty } else { o.dirty },
        "callback source cache pre/postcondition differs"
    );
    Ok(())
}
impl Replay {
    pub fn load(value: Option<&Value>) -> Result<Option<Self>> {
        let Some(value) = value else { return Ok(None) };
        let input: Input = serde_json::from_value(value.clone())?;
        ensure!(
            input.schema == 1 && !input.observations.is_empty(),
            "family callback schema/observations missing"
        );
        let mut observations = BTreeMap::new();
        for (name, source) in input.observations {
            ensure!(!name.is_empty(), "empty observation identity");
            let observation = observe(
                &source,
                &read(&source.capture)?,
                &read(&source.verification)?,
            )?;
            observations.insert(name, observation);
        }
        Ok(Some(Self::from_verified(input.binding, observations)?))
    }
    fn from_verified(
        binding: Binding,
        observations: BTreeMap<String, Observation>,
    ) -> Result<Self> {
        State::bind(binding.clone())?;
        let state = binding
            .fields
            .state
            .context("observed counted-text owner unknown")?;
        let label = binding
            .fields
            .label
            .context("observed label owner unknown")?;
        let text = binding
            .counted_text
            .get(&state)
            .context("binding counted owner absent")?;
        ensure!(text.capacity==Some(256) && text.profile==Some(sims_runtime::original_session::window_update::StorageProfile::CompleteCollection191Inline256Observed20261025),"binding capacity/source profile mismatch");
        ensure!(
            binding.label_text.contains_key(&label),
            "binding label owner absent"
        );
        for observation in observations.values() {
            ensure!(
                binding.application_family == ApplicationFamily::Family(observation.family),
                "binding source family mismatch"
            );
        }
        // Initial bytes belong to binding only. Every observation compares its
        // actual current bytes when invoked, after earlier callbacks may change them.
        Ok(Self {
            binding,
            observations,
            used: BTreeSet::new(),
            next_serial: 1,
            journal: vec![],
        })
    }
    fn logical_guard(&self, state: &State, o: &Observation) -> Result<u32> {
        let state_id = self.binding.fields.state.context("counted owner unknown")?;
        let label_id = self.binding.fields.label.context("label owner unknown")?;
        let (bytes, buffer) = logical_text(state, state_id)?;
        ensure!(
            state.fields() == self.binding.fields
                && bytes == o.initial_text
                && state.label(label_id) == Some(o.initial_label.as_slice()),
            "callback logical prestate differs"
        );
        Ok(buffer)
    }
    fn event_observation(&self, event: &Event) -> Result<Observation> {
        ensure!(
            event.serial == self.next_serial && !self.used.contains(&event.observation),
            "missing, reordered or reused callback event"
        );
        let o = self
            .observations
            .get(&event.observation)
            .context("undeclared callback observation")?
            .clone();
        ensure!(
            event.raw_argument == o.raw_argument,
            "callback argument differs from actual source row"
        );
        Ok(o)
    }
    pub fn bind(&self, session: &mut Session) -> Result<()> {
        session.bind_family_window(self.binding.clone())
    }
    pub fn complete(&self) -> bool {
        self.used.len() == self.observations.len()
    }
    pub fn apply(
        &mut self,
        session: &mut Session,
        resumed: &mut Session,
        event: &Event,
        mut restore: impl FnMut(&Session) -> Result<Session>,
        out: &Path,
    ) -> Result<()> {
        let o = self.event_observation(event)?;
        owner_guard(session, &o, false)?;
        let state_id = self.binding.fields.state.context("counted owner unknown")?;
        let label_id = self.binding.fields.label.context("label owner unknown")?;
        let backend = session
            .host
            .presentation
            .family_window
            .as_ref()
            .context("family window is unbound")?;
        let buffer = self.logical_guard(backend, &o)?;
        ensure!(
            serde_json::to_vec(&session.save())? == serde_json::to_vec(&resumed.save())?,
            "callback initial warm/cold state differs"
        );
        let neutral_before = neutral(session)?;
        let mut expected = session.simulation.clone();
        let reference = expected
            .vm
            .world
            .person
            .as_mut()
            .unwrap()
            .resolve_family_friend_count(o.family, expected.vm.world.autonomy_constants.as_ref());
        let expected = match reference {
            Ok(_) => Ok(serde_json::to_vec(&expected)?),
            Err(e) => Err(e),
        };
        let journal = self.journal.len();
        self.journal.push(json!({"event":event,"source":serde_json::to_value(&json!({"capture":o.source.capture.path,"capture_sha256":o.source.capture.sha256,"verification":o.source.verification.path,"verification_sha256":o.source.verification.sha256,"entry_line":o.source.entry_line,"return_line":o.source.return_line,"next_poll_line":o.source.next_poll_line}))?,"status":"begun","cuts":[],"scope":"callback-body, no native prefix/cadence/tick parity claim"}));
        super::write(
            &out.join(format!(
                "family-window-{}-before-session.json",
                event.serial
            )),
            &session.save_managed()?,
        )?;
        session.begin_family_window_update(event.raw_argument)?;
        resumed.begin_family_window_update(event.raw_argument)?;
        let start = Instant::now();
        let mut restores = 0usize;
        let mut services = 0usize;
        let mut friends = 0usize;
        let mut temporary_buffers = vec![];
        for _ in 0..160 {
            ensure!(
                start.elapsed().as_secs() < 900,
                "callback event observer budget"
            );
            let p = session.advance_family_window_update(1)?;
            ensure!(
                p == resumed.advance_family_window_update(1)?,
                "callback restored advance differs"
            );
            self.cut(
                session,
                resumed,
                &mut restore,
                journal,
                "advance",
                &mut restores,
                &neutral_before,
            )?;
            match p {
                Progress::Complete => break,
                Progress::Pending => (),
                Progress::Service(request) => {
                    self.journal[journal]["service_attempt"] = serde_json::to_value(&request)?;
                    if matches!(
                        request,
                        sims_vm::family_label::Request::Numeric(
                            sims_vm::family_label::NumericRequest::Friends { .. }
                        )
                    ) {
                        friends += 1;
                    }
                    if let sims_vm::family_label::Request::Window(
                        sims_vm::family_label::WindowRequest::Temporary { source },
                    ) = &request
                    {
                        ensure!(
                            source.buffer == buffer,
                            "callback returned buffer identity changed"
                        );
                        temporary_buffers.push(source.buffer);
                    }
                    session.service_family_window_request(&request)?;
                    resumed.service_family_window_request(&request)?;
                    services += 1;
                    self.cut(
                        session,
                        resumed,
                        &mut restore,
                        journal,
                        "service_ready",
                        &mut restores,
                        &neutral_before,
                    )?;
                    let before = serde_json::to_vec(&session.save())?;
                    ensure!(
                        session.service_family_window_request(&request).is_err()
                            && before == serde_json::to_vec(&session.save())?,
                        "callback duplicate service changed state"
                    );
                }
            }
        }
        ensure!(
            !session.family_window_pending()
                && friends == o.numeric_getters
                && temporary_buffers.len() == o.numeric_getters,
            "callback event did not complete the observed getter/temporary count"
        );
        owner_guard(session, &o, true)?;
        let backend = session.host.presentation.family_window.as_ref().unwrap();
        ensure!(
            logical_text(backend, state_id)? == (o.final_text, buffer)
                && backend.label(label_id) == Some(o.final_label.as_slice())
                && backend.result() == Some(o.return_al),
            "callback logical source postcondition differs"
        );
        ensure!(
            expected.map_err(anyhow::Error::msg)? == serde_json::to_vec(&session.simulation)?,
            "callback changed simulation beyond ordinary family getter"
        );
        ensure!(
            neutral_before == neutral(session)?,
            "callback changed tick/RNG/process/provider"
        );
        self.journal[journal]["status"] = json!("complete");
        self.journal[journal]["services"] = json!(services);
        self.journal[journal]["cold_cuts"] = json!(restores);
        self.journal[journal]["friends_services"] = json!(friends);
        self.journal[journal]["temporary_buffers"] = json!(temporary_buffers);
        self.used.insert(event.observation.clone());
        self.next_serial = self
            .next_serial
            .checked_add(1)
            .context("callback serial overflow")?;
        Ok(())
    }
    fn cut(
        &mut self,
        s: &Session,
        resumed: &mut Session,
        restore: &mut impl FnMut(&Session) -> Result<Session>,
        journal: usize,
        phase: &str,
        count: &mut usize,
        neutral_before: &[u8],
    ) -> Result<()> {
        ensure!(
            neutral(s)? == neutral_before,
            "callback cut changed tick/RNG/process/provider"
        );
        let before = serde_json::to_vec(&s.save())?;
        ensure!(
            before == serde_json::to_vec(&resumed.save())?,
            "callback warm/cold execution differs"
        );
        *resumed = restore(s)?;
        ensure!(
            before == serde_json::to_vec(&resumed.save())?
                && before == serde_json::to_vec(&s.save())?,
            "callback cold restore or passive save differs"
        );
        *count += 1;
        self.journal[journal]["cuts"]
            .as_array_mut()
            .unwrap()
            .push(json!({"phase":phase,"state_sha256":hash(&before)}));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Source, Vec<u8>, Vec<u8>) {
        // Synthetic byte/parser fixture only; not an original observation.
        let mut before = vec![0u8; 348];
        before[0x10c..0x110].copy_from_slice(&5u32.to_le_bytes());
        before[0x124] = 1;
        let mut after = before.clone();
        after[0x124] = 0;
        let mut cp = vec![0u8; 580];
        cp[0x23e..0x240].copy_from_slice(&256u16.to_le_bytes());
        let encode = |v: &[u8]| v.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut stack = vec![0u8; 12];
        stack[4..8].copy_from_slice(&1u32.to_le_bytes());
        let entry = json!({"kind":"window_entry","tick":88444,"lot":5,"rng":[1,2,3],"registers":{"eip":0x53a01f},"stack":encode(&stack),
            "process":{"loaded":1,"words":sims_vm::autonomy_constants::ORDER.iter().map(|field|json!({"address":field.address(),"raw":"00000000"})).collect::<Vec<_>>()},
            "application_family":{"storage":encode(&before)},"cpstate":{"raw":encode(&cp),"buffer":1000,"length":0,"text_with_nul":"00"},"label":{"text":{"bytes":""}}});
        let mut returned = entry.clone();
        returned["kind"] = json!("window_return");
        returned["registers"] = json!({"eax":0});
        returned["application_family"]["storage"] = json!(encode(&after));
        returned["cpstate"]["length"] = json!(1);
        returned["cpstate"]["text_with_nul"] = json!("3000");
        returned["label"]["text"]["bytes"] = json!("30");
        let poll = json!({"kind":"ui_poll_entry","tick":88444,"lot":5});
        let rows = [entry, returned, poll];
        let lines = rows
            .iter()
            .map(|r| serde_json::to_vec(r).unwrap())
            .collect::<Vec<_>>();
        let mut capture = vec![];
        for line in &lines {
            capture.extend(line);
            capture.push(b'\n');
        }
        let proof = json!({"capture_sha256":hash(&capture),"summary":{"raw_argument":1,"family_id":5,"cpstate_capacity":256,"callback_return_al":0,"numeric_getters":2},"evidence_rows":rows.iter().enumerate().map(|(i,r)|json!({"line":i+1,"kind":r["kind"],"sha256":hash(&lines[i])})).collect::<Vec<_>>()});
        let proof = serde_json::to_vec(&proof).unwrap();
        (
            Source {
                capture: HashedFile {
                    path: "cache/synthetic-never-read.jsonl".into(),
                    sha256: hash(&capture),
                },
                verification: HashedFile {
                    path: "cache/synthetic-never-read-proof.json".into(),
                    sha256: hash(&proof),
                },
                entry_line: 1,
                return_line: 2,
                next_poll_line: 3,
            },
            capture,
            proof,
        )
    }
    fn binding() -> Binding {
        Binding{fields:sims_vm::family_label::Fields{owner:100,state:Some(101),label:Some(102)},application_family:ApplicationFamily::Family(5),counted_text:[(101,sims_runtime::original_session::window_update::CountedBinding{bytes:vec![],capacity:Some(256),profile:Some(sims_runtime::original_session::window_update::StorageProfile::CompleteCollection191Inline256Observed20261025)})].into(),label_text:[(102,vec![])].into()}
    }
    fn replay() -> Replay {
        let (source, capture, proof) = fixture();
        Replay {
            binding: binding(),
            observations: [(
                "observed".into(),
                observe(&source, &capture, &proof).unwrap(),
            )]
            .into(),
            used: BTreeSet::new(),
            next_serial: 1,
            journal: vec![],
        }
    }
    #[test]
    fn retained_export_keeps_historical_unknown_and_preserves_owned_bytes() {
        assert!(retained(None)["state"].is_null());
        let state = State::bind(binding()).unwrap();
        let before = serde_json::to_vec(&state).unwrap();
        assert_eq!(
            retained(Some(&state))["state"],
            serde_json::to_value(&state).unwrap()
        );
        assert_eq!(before, serde_json::to_vec(&state).unwrap());
    }
    #[test]
    #[ignore = "private sealed native source; no guest or gameplay execution"]
    fn actual_hashed_callback_source_and_explicit_binding_load() -> Result<()> {
        let path = std::env::var("SIMS_FAMILY_EVENTS_INPUT")
            .context("SIMS_FAMILY_EVENTS_INPUT required")?;
        let input: Value = serde_json::from_slice(&fs::read(path)?)?;
        let replay =
            Replay::load(input.get("family_window"))?.context("declared binding absent")?;
        assert!(!replay.complete() && replay.journal.is_empty());
        Ok(())
    }
    #[test]
    fn synthetic_parser_preserves_raw_source_and_does_not_infer_cadence() {
        let (source, capture, proof) = fixture();
        let o = observe(&source, &capture, &proof).unwrap();
        assert_eq!(
            (o.raw_argument, o.tick, o.family, o.return_al),
            (1, 88444, 5, 0)
        );
        assert!(o.initial_text.is_empty());
        assert_eq!(o.final_text, b"0");
        let r = replay();
        assert!(!r.complete());
        assert!(r.journal.is_empty());
        assert_eq!(r.next_serial, 1);
    }
    #[test]
    fn absent_input_and_missing_event_do_not_create_callback_work() {
        assert!(Replay::load(None).unwrap().is_none());
        assert!(startup_events(None, false).unwrap().is_empty());
        assert!(
            startup_events(
                Some(&json!([{"serial":1,"observation":"observed","raw_argument":1}])),
                false
            )
            .is_err()
        );
        let r = replay();
        assert!(!r.complete() && r.used.is_empty() && r.journal.is_empty());
        assert!(
            r.event_observation(&Event {
                serial: 1,
                observation: "missing".into(),
                raw_argument: 1
            })
            .is_err()
        );
    }
    #[test]
    fn later_observation_checks_current_bytes_instead_of_the_initial_binding_template() {
        let first = replay().observations["observed"].clone();
        let mut later = first.clone();
        later.initial_text = b"0".to_vec();
        later.initial_label = b"0".to_vec();
        later.dirty = false;
        later.numeric_getters = 1;
        let replay = Replay::from_verified(
            binding(),
            [("first".into(), first), ("later".into(), later.clone())].into(),
        )
        .unwrap();
        assert!(
            replay
                .logical_guard(&State::bind(binding()).unwrap(), &later)
                .is_err()
        );
        let mut current = binding();
        current.counted_text.get_mut(&101).unwrap().bytes = b"0".to_vec();
        current.label_text.insert(102, b"0".to_vec());
        assert_eq!(
            replay
                .logical_guard(&State::bind(current).unwrap(), &later)
                .unwrap(),
            1
        );
        assert!(replay.used.is_empty() && replay.journal.is_empty());
    }
    #[test]
    fn argument_is_required_and_not_defaulted_to_one() {
        for v in [
            json!({"serial":1,"observation":"observed"}),
            json!({"serial":1,"observation":"observed","raw_argument":-1}),
            json!({"serial":1,"observation":"observed","raw_argument":4294967296u64}),
            json!({"serial":1,"observation":"observed","raw_argument":1,"extra":true}),
        ] {
            assert!(serde_json::from_value::<Event>(v).is_err());
        }
        let r = replay();
        assert!(
            r.event_observation(&Event {
                serial: 1,
                observation: "observed".into(),
                raw_argument: 0
            })
            .is_err()
        );
        assert!(r.journal.is_empty());
    }
    #[test]
    fn reordered_and_reused_events_fail_before_any_work() {
        let mut r = replay();
        let e = Event {
            serial: 2,
            observation: "observed".into(),
            raw_argument: 1,
        };
        assert!(r.event_observation(&e).is_err());
        r.used.insert("observed".into());
        assert!(r.event_observation(&Event { serial: 1, ..e }).is_err());
        assert!(r.journal.is_empty());
    }
    #[test]
    fn changed_capture_file_digest_and_verified_row_digest_are_rejected() {
        let (mut source, mut capture, proof) = fixture();
        let changed = capture.windows(5).position(|b| b == b"88444").unwrap() + 4;
        capture[changed] = b'5';
        assert!(observe(&source, &capture, &proof).is_err());
        let (mut source2, capture2, proof2) = fixture();
        source2.capture.sha256 = "0".repeat(64);
        assert!(observe(&source2, &capture2, &proof2).is_err());
        source.capture.sha256 = hash(&capture);
        let mut p: Value = serde_json::from_slice(&proof).unwrap();
        p["capture_sha256"] = json!(hash(&capture));
        let p = serde_json::to_vec(&p).unwrap();
        source.verification.sha256 = hash(&p);
        assert!(observe(&source, &capture, &p).is_err());
    }
    #[test]
    fn missing_or_reordered_source_boundaries_are_rejected() {
        let (mut source, capture, proof) = fixture();
        for (entry, returned, poll) in [(0, 2, 3), (2, 1, 3), (1, 3, 2), (1, 2, 40)] {
            source.entry_line = entry;
            source.return_line = returned;
            source.next_poll_line = poll;
            assert!(observe(&source, &capture, &proof).is_err());
        }
    }
    #[test]
    fn unknown_partial_process_stays_unknown_and_is_not_repaired() {
        let r = replay();
        let o = &r.observations["observed"];
        let mut world = sims_vm::World::default();
        let before = serde_json::to_vec(&world).unwrap();
        assert!(process_guard(&world, o).is_err());
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
        world.autonomy_constants = Some(sims_vm::autonomy_constants::Owner {
            input_evidence: None,
            observation: None,
            process: sims_vm::autonomy_constants::Process {
                loaded: Some(1),
                words: [None; 12],
            },
            configuration: Default::default(),
        });
        let before = serde_json::to_vec(&world).unwrap();
        assert!(process_guard(&world, o).is_err());
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
        world.autonomy_constants.as_mut().unwrap().process.words = [Some(0); 12];
        let before = serde_json::to_vec(&world).unwrap();
        process_guard(&world, o).unwrap();
        assert_eq!(before, serde_json::to_vec(&world).unwrap());
        assert!(
            world
                .autonomy_constants
                .as_ref()
                .unwrap()
                .configuration
                .fp_control
                .is_none()
        );
    }
    #[test]
    fn logical_text_projection_is_read_only_and_missing_owners_fail() {
        let state = State::bind(binding()).unwrap();
        let before = serde_json::to_vec(&state).unwrap();
        assert_eq!(logical_text(&state, 101).unwrap(), (vec![], 1));
        assert!(logical_text(&state, 0).is_err());
        assert_eq!(before, serde_json::to_vec(&state).unwrap());
    }
}
