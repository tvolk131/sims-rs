//! Compact continuation from real retained native-owned player boundaries.
//! No starting-house load, world repair, routing fallback or RNG injection.
use super::*;
use clap::Parser;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
};

const BASE: &str = "cache/definition-link-normalization-20261024-first/furniture-after-first";
const INSTALL: &str = "cache/native-parity-guest-20261023-second/assets-immutable";
const RESERVE: u64 = 3 * 1024 * 1024 * 1024;

/// Frozen research input policy: exact observed dialogs, never a generic
/// dismissal. The proof file binds native response/source joins and the
/// unchanged resource's execution location separately from original timing.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Notice {
    source: String,
    source_sha256: String,
    caller_guid: u32,
    bhav: u16,
    instruction: usize,
    title: String,
    body_sha256: String,
    buttons: Vec<sims_vm::dialog::Button>,
    operands: [u8; 8],
    bitmap_icon: Option<u16>,
    result: u32,
    evidence: Value,
}
impl Notice {
    fn matches(&self, record: &sims_vm::dialog::Record, guid: u32, source_hash: &str) -> bool {
        self.source == record.source
            && self.source == record.origin.source
            && self.source_sha256 == source_hash
            && self.caller_guid == guid
            && self.bhav == record.origin.bhav
            && self.instruction == record.origin.instruction
            && self.title == record.title
            && self.body_sha256 == engine::digest(record.body.as_bytes())
            && self.buttons == record.buttons
            && self.operands == record.operands
            && self.bitmap_icon == record.bitmap_icon
            && record.status == 1
            && record.buttons.iter().any(|b| b.result == self.result)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoticePolicy {
    schema: u32,
    scope: String,
    captured_schedule: String,
    captured_schedule_sha256: String,
    source_strings: String,
    source_strings_sha256: String,
    notices: Vec<Notice>,
}
fn notice_policy() -> Result<NoticePolicy> {
    let path = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_NOTICE_POLICY")?);
    let bytes = std::fs::read(&path)?;
    let policy: NoticePolicy = serde_json::from_slice(&bytes)?;
    ensure!(
        policy.schema == 1 && policy.notices.len() == 6,
        "not the six-response captured input policy"
    );
    let sources: BTreeSet<_> = policy.notices.iter().map(|n| &n.source).collect();
    ensure!(sources.len() == 6, "duplicate notice source in policy");
    Ok(policy)
}

fn returned_boundary(before: u64, after: u64, trace: &[Observation]) -> Result<bool> {
    ensure!(
        after >= before && after - before <= 1 && trace.len() <= 1,
        "ordinary frame crossed multiple ticks: {before}->{after}, trace={}",
        trace.len()
    );
    ensure!(
        trace.len() as u64 == after - before,
        "completed tick missing from native trace: {before}->{after}, trace={}",
        trace.len()
    );
    if let Some(row) = trace.first() {
        ensure!(row.tick == after, "trace boundary differs");
    }
    Ok(after != before)
}
fn available_bytes(text: &str) -> Result<u64> {
    let value = text
        .lines()
        .last()
        .context("df output absent")?
        .split_whitespace()
        .nth(3)
        .context("df available column absent")?
        .parse::<u64>()?;
    value.checked_mul(1024).context("df size overflow")
}
fn space(out: &Path, extra: u64) -> Result<()> {
    let result = std::process::Command::new("/bin/df")
        .args(["-k", out.to_str().context("nonUTF8output")?])
        .output()?;
    ensure!(result.status.success(), "disk reserve query failed");
    let bytes = available_bytes(std::str::from_utf8(&result.stdout)?)?;
    ensure!(
        bytes >= RESERVE.saturating_add(extra),
        "disk reserve reached: {bytes} available, {extra} proposed"
    );
    Ok(())
}
fn append(file: &mut BufWriter<File>, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *file, value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}
struct Control {
    out: PathBuf,
    paused: Duration,
    last: Option<(String, u64)>,
}
impl Control {
    fn gate(&mut self, phase: &str, tick: u64) -> Result<()> {
        let start = Instant::now();
        let paused = self.out.join("PAUSE");
        if paused.exists() {
            write_json(
                &self.out.join("control.json"),
                &json!({"pid":std::process::id(),"phase":phase,
                "tick":tick,"status":"paused","pause_file":paused,"stop_file":self.out.join("STOP")}),
            )?;
            while paused.exists() && !self.out.join("STOP").exists() {
                std::thread::sleep(Duration::from_millis(100));
            }
            self.paused += start.elapsed();
        }
        ensure!(
            !self.out.join("STOP").exists(),
            "operator STOP requested; no engine failure asserted"
        );
        if self
            .last
            .as_ref()
            .is_none_or(|(previous, old_tick)| previous != phase || *old_tick != tick)
        {
            write_json(
                &self.out.join("control.json"),
                &json!({"pid":std::process::id(),"phase":phase,"tick":tick,"status":"running",
                    "pause_file":paused,"stop_file":self.out.join("STOP")}),
            )?;
            self.last = Some((phase.into(), tick));
        }
        Ok(())
    }
}

/// Full ordinary persisted state, streamed without a giant JSON tree/output
/// buffer. This includes Host model/geometry/continuations, mutable character
/// resource overlay and DialogClient owners, not just Simulation+View.
fn full(app: &Application) -> Result<Observation> {
    #[derive(Serialize)]
    struct Capture<'a> {
        schema: u32,
        session: Saved,
        dialog_client: sims_render::dialog::client::Saved,
        camera_device: Option<sims_render::camera_device::Saved>,
        application: Local,
        observer: &'a str,
    }
    let s = app.session()?;
    let snapshot = Capture {
        schema: 1,
        session: s.save(),
        dialog_client: app
            .viewer
            .client
            .as_ref()
            .context("client absent")?
            .save(s)?,
        camera_device: app.viewer.device.as_ref().map(|d| d.save(s)).transpose()?,
        application: app.local(),
        observer: "full-native-owned-continuation-v1",
    };
    Ok(Observation {
        tick: s.simulation.vm.tick,
        rng: [
            Some(s.simulation.vm.rng_state),
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        ],
        state_sha256: engine::digest_serialized(&snapshot)?,
    })
}
fn safe(app: &Application, mode: i32) -> Result<()> {
    let s = app.session()?;
    ensure!(
        !app.viewer.playing
            && !app.viewer.presentation_pending()
            && !s.player_input_pending()
            && s.simulation.vm.world.dialogs.modal.is_none(),
        "not a safe paused player boundary"
    );
    ensure!(
        s.host
            .presentation
            .transport
            .as_ref()
            .context("transport absent")?
            .controls
            .mode
            == mode,
        "wrong actual player mode"
    );
    ensure!(
        s.options.routing == sims_sim::lot_route::path::Policy::Strict
            && s.simulation
                .vm
                .world
                .map_size
                .as_ref()
                .is_some_and(|m| m.active_size == 48)
            && s.simulation.vm.world.registry_order.is_some(),
        "actual Strict map/registry owner lost"
    );
    ensure!(
        s.simulation
            .architecture
            .as_ref()
            .and_then(|a| a.original.as_ref())
            .is_some_and(|lot| lot.size == 64)
            && s.simulation.native_scheduler.is_some()
            && s.simulation.fixture_driver.is_none(),
        "physical stride64/native scheduler owner lost"
    );
    Ok(())
}
fn identities(app: &Application) -> Result<Value> {
    let s = app.session()?;
    Ok(json!({"source_hashes":s.house.sources,
        "owned":sims_content::owned_resources::Neighborhood::capture(s.index(), &s.house.neighborhood_source)?.hashes(),
        "program_hash":s.program.fingerprint(),"normalized_sources":s.save().normalized_object_definitions,
        "map_size":s.simulation.vm.world.map_size,
        "physical_stride":s.simulation.architecture.as_ref().and_then(|a|a.original.as_ref()).map(|lot|lot.size),
        "registry_order_present":s.simulation.vm.world.registry_order.is_some(),
        "routing":s.options.routing,"tick":s.simulation.vm.tick}))
}
fn checkpoint(app: &Application, out: &Path, label: &str, mode: i32) -> Result<Vec<u8>> {
    safe(app, mode)?;
    let before = full(app)?;
    // Enforce the real player Client's managed-preview boundary as well as
    // the Session's managed capture; research-only previews cannot be promoted.
    app.viewer
        .client
        .as_ref()
        .context("client absent")?
        .save_managed(app.session()?)?;
    // The real application checkpoint uses Session::save_managed and actual
    // Client/Device save recipes, then the actual JSON encode/decode path.
    let bytes = app.checkpoint()?;
    ensure!(full(app)? == before, "managed capture changed state");
    let restored = cold(app, &bytes)?;
    ensure!(
        full(&restored)? == before && identities(&restored)? == identities(app)?,
        "safe managed encode/decode changed full state/RNG or owned source namespace"
    );
    drop(restored);
    space(out, bytes.len() as u64)?;
    let path = out.join(format!("{label}.json"));
    std::fs::write(&path, &bytes)?;
    write_json(
        &out.join(format!("{label}-identity.json")),
        &json!({"sha256":engine::digest(&bytes),
        "bytes":bytes.len(),"owners":identities(app)?,"full_observation":before,
        "safe_player_boundary":true,"managed_decode_full_equal":true,"strict_native_parity_ticks":0}),
    )?;
    Ok(bytes)
}
fn cold(app: &Application, bytes: &[u8]) -> Result<Application> {
    let index = Index::scan(&app.viewer.args.root)?;
    Application::restore_checkpoint(
        &app.viewer.args,
        &index,
        app.viewer.icons.as_ref().context("icons absent")?,
        app.epoch,
        bytes,
    )
}
fn restore(repo: &Path, source: &Path, out: &Path) -> Result<Application> {
    let root = repo.join(INSTALL);
    let index = Index::scan(&root)?;
    let bytes = std::fs::read(source)?;
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House07.iff",
        "--root",
        root.to_str().unwrap(),
        "--save",
        out.join("unused-player-save.json").to_str().unwrap(),
    ]);
    let icons = Arc::new(dialog_view::Icons::load(&index)?);
    Application::restore_checkpoint(&args, &index, &icons, Instant::now(), &bytes)
}

#[derive(Default)]
struct Changes {
    previous: BTreeMap<i16, sims_vm::person::Neighbor>,
    off_lot_neighbors: BTreeSet<i16>,
    neighbor_changes: u64,
    off_lot_changes: u64,
}
impl Changes {
    fn new(app: &Application) -> Result<Self> {
        let previous = app
            .session()?
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("person registry absent")?
            .neighbors
            .clone();
        Ok(Self {
            previous,
            ..Default::default()
        })
    }
    fn observe(&mut self, app: &Application) -> Result<Vec<Value>> {
        let world = &app.session()?.simulation.vm.world;
        let now = &world
            .person
            .as_ref()
            .context("person registry absent")?
            .neighbors;
        let live: BTreeSet<_> = world
            .objects
            .values()
            .filter(|o| o.person.is_some())
            .filter_map(|o| o.definition.as_ref().map(|d| d.guid))
            .collect();
        let mut rows = vec![];
        for (&id, current) in now {
            if self.previous.get(&id) == Some(current) {
                continue;
            }
            let old = self.previous.get(&id);
            let off_lot = !live.contains(&current.guid);
            let fields: Vec<_> = current
                .person
                .iter()
                .filter(|(k, v)| old.and_then(|o| o.person.get(*k)) != Some(*v))
                .map(
                    |(k, v)| json!({"field":k,"before":old.and_then(|o|o.person.get(k)),"after":v}),
                )
                .collect();
            rows.push(
                json!({"neighbor":id,"guid":current.guid,"off_lot_at_boundary":off_lot,
                "before_sha256":old.map(engine::digest_serialized).transpose()?,
                "after_sha256":engine::digest_serialized(current)?,"person_fields":fields,
                "dirty":current.dirty,"friend_count":current.friend_count,
                "relationships_changed":old.is_none_or(|o|o.relationships!=current.relationships),
                "inventory_changed":old.is_none_or(|o|o.inventory!=current.inventory)}),
            );
            self.neighbor_changes += 1;
            if off_lot {
                self.off_lot_changes += 1;
                self.off_lot_neighbors.insert(id);
            }
        }
        for (&id, old) in &self.previous {
            if !now.contains_key(&id) {
                rows.push(json!({"neighbor":id,"removed":true,"guid":old.guid}));
            }
        }
        self.previous = now.clone();
        Ok(rows)
    }
}
struct Pair {
    warm: Application,
    cold: Application,
    commands: BufWriter<File>,
    ticks: BufWriter<File>,
    changes: Changes,
    count: u64,
    frames: u64,
    help_answered: bool,
    notice_policy: NoticePolicy,
    answered_notices: BTreeSet<String>,
    out: PathBuf,
}
impl Pair {
    fn new(
        warm: Application,
        bytes: &[u8],
        out: PathBuf,
        notice_policy: &NoticePolicy,
    ) -> Result<Self> {
        std::fs::create_dir(&out)?;
        let cold = cold(&warm, bytes)?;
        ensure!(
            identities(&warm)? == identities(&cold)?,
            "owned immutable namespace/catalog differs after cold restore"
        );
        ensure!(
            full(&warm)? == full(&cold)?,
            "full initial managed decode differs"
        );
        let changes = Changes::new(&warm)?;
        let commands = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(out.join("commands.jsonl"))?;
        let ticks = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(out.join("ticks.jsonl"))?;
        Ok(Self {
            warm,
            cold,
            commands: BufWriter::new(commands),
            ticks: BufWriter::new(ticks),
            changes,
            count: 0,
            frames: 0,
            help_answered: false,
            notice_policy: notice_policy.clone(),
            answered_notices: BTreeSet::new(),
            out,
        })
    }
    fn execute(&mut self, command: Command) -> Result<()> {
        let before = self.warm.session()?.simulation.vm.tick;
        ensure!(
            self.cold.session()?.simulation.vm.tick == before,
            "branch tick already differs"
        );
        self.warm.begin_tick_trace();
        self.cold.begin_tick_trace();
        let limits = Limits {
            service_polls: 10_000,
            wall_seconds: 180.,
            ..Default::default()
        };
        let a = self.warm.execute(&command, &limits);
        let b = self.cold.execute(&command, &limits);
        let ta = self.warm.take_tick_trace();
        let tb = self.cold.take_tick_trace();
        append(
            &mut self.commands,
            &json!({"command":command,"before":before,
            "after":[self.warm.session()?.simulation.vm.tick,self.cold.session()?.simulation.vm.tick],
            "warm_error":a.as_ref().err().map(|e|format!("{e:#}")),
            "cold_error":b.as_ref().err().map(|e|format!("{e:#}")),
            "native_tick_trace":ta,"cold_native_tick_trace":tb}),
        )?;
        a?;
        b?;
        ensure!(ta == tb, "native completed-tick trace diverges");
        let after = self.warm.session()?.simulation.vm.tick;
        let completed = returned_boundary(before, after, &ta)?;
        returned_boundary(before, self.cold.session()?.simulation.vm.tick, &tb)?;
        if completed {
            let warm = full(&self.warm)?;
            let cold = full(&self.cold)?;
            ensure!(
                warm == cold,
                "full mutable gameplay/host/client/device state diverges at tick{after}"
            );
            self.count += 1;
            append(
                &mut self.ticks,
                &json!({"ordinal":self.count,"tick":after,"full":warm,
                "native_boundary":ta.first(),"neighbor_changes":self.changes.observe(&self.warm)?,
                "owned_index_identity_checked_at_managed_checkpoints":true}),
            )?;
        } else if !matches!(command, Command::Advance { .. }) {
            ensure!(
                full(&self.warm)? == full(&self.cold)?,
                "ordinary input state diverges before tick"
            );
        }
        self.frames += u64::from(matches!(command, Command::Advance { .. }));
        Ok(())
    }
    fn notice(&mut self) -> Result<bool> {
        let vm = &self.warm.session()?.simulation.vm;
        let Some(id) = vm.world.dialogs.modal else {
            return Ok(false);
        };
        let record = vm
            .world
            .dialogs
            .records
            .iter()
            .find(|r| r.id == id)
            .context("modal record absent")?;
        write_json(&self.out.join(format!("modal-{id}.json")), record)?;
        let definition = vm
            .world
            .objects
            .get(&record.caller)
            .and_then(|o| o.definition.as_ref())
            .context("actual dialog caller definition absent")?;
        let module = self
            .warm
            .session()?
            .program
            .modules
            .get(&record.source)
            .context("actual dialog source module absent")?;
        let allowed: Vec<_> = self
            .notice_policy
            .notices
            .iter()
            .filter(|n| n.matches(record, definition.guid, &module.content_hash))
            .collect();
        ensure!(
            allowed.len() == 1,
            "unknown modal; exact recorded input required"
        );
        let allowed = allowed[0];
        ensure!(
            !self.answered_notices.contains(&allowed.source),
            "repeated notice lifetime; recorded input exhausted"
        );
        let result = allowed.result;
        let source = allowed.source.clone();
        let hash = engine::digest(record.body.as_bytes());
        self.execute(Command::Dialog {
            id,
            result,
            body_sha256: hash,
        })?;
        self.help_answered |= source.ends_with("::HelpSystem.iff");
        self.answered_notices.insert(source);
        // Viewer::Dialog intentionally asks the player to press Play. Native
        // transport pause can already be clear while this local hold remains.
        // Record the same ordinary resume used by the earlier furniture replay.
        self.execute(Command::Playing { value: true })?;
        Ok(true)
    }
    fn summary(&self) -> Value {
        json!({"completed_ticks":self.count,"ordinary_frames":self.frames,
        "tick":self.warm.session().map(|s|s.simulation.vm.tick).map_err(|e|format!("{e:#}")),
        "neighbor_changes":self.changes.neighbor_changes,"off_lot_neighbor_changes":self.changes.off_lot_changes,
        "off_lot_neighbors":self.changes.off_lot_neighbors,"help_answered":self.help_answered,
        "answered_notices":self.answered_notices,
        "strict_native_parity_ticks":0})
    }
    fn preserve_failure(&self, error: &anyhow::Error) -> Result<()> {
        write_json(
            &self.out.join("first-failure.json"),
            &json!({"error":format!("{error:#}"),"progress":self.summary()}),
        )?;
        for (label, app) in [("warm", &self.warm), ("cold", &self.cold)] {
            match app.checkpoint() {
                Ok(bytes) => {
                    space(&self.out, bytes.len() as u64)?;
                    std::fs::write(self.out.join(format!("first-failure-{label}.json")), bytes)?;
                }
                Err(capture) => {
                    write_json(
                        &self
                            .out
                            .join(format!("first-failure-{label}-capture-error.json")),
                        &format!("{capture:#}"),
                    )?;
                    space(&self.out, 128 * 1024 * 1024)?;
                    write_json(
                        &self
                            .out
                            .join(format!("first-failure-{label}-research.json")),
                        &app.capture(false)?,
                    )?;
                }
            }
        }
        Ok(())
    }
    fn progress(
        &mut self,
        control: &mut Control,
        ticks: u64,
        active_budget: Duration,
    ) -> Result<()> {
        let start = self.count;
        let wall = Instant::now();
        let paused = control.paused;
        self.execute(Command::Playing { value: true })?;
        while self.count - start < ticks {
            control.gate(
                self.out.file_name().unwrap().to_str().unwrap(),
                self.warm.session()?.simulation.vm.tick,
            )?;
            ensure!(
                wall.elapsed().saturating_sub(control.paused - paused) < active_budget,
                "continuation active wall budget reached"
            );
            if self.frames % 1024 == 0 {
                space(&self.out, 0)?;
            }
            if self.notice()? {
                continue;
            }
            ensure!(
                self.warm.viewer.playing,
                "game stopped before required completed ticks"
            );
            self.execute(Command::Advance { millis: 1 })?;
        }
        Ok(())
    }
}

fn directed_records(app: &Application, actor: &Identity) -> Result<Vec<sims_vm::action::Action>> {
    actor.check(app.session()?)?;
    let owner = app
        .session()?
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .context("actions absent")?
        .actors
        .get(&actor.object)
        .context("directed actor absent")?;
    Ok(owner
        .active
        .iter()
        .chain(owner.queue.pending.iter())
        .cloned()
        .collect())
}
fn resolved_offered_label(
    action: &sims_vm::action::Action,
    static_label: impl FnOnce() -> Result<String>,
) -> Result<String> {
    match &action.label {
        Some(label) => Ok(label.clone()),
        None => static_label(),
    }
}
fn choose_offered_label(pair: &mut Pair, label: &str) -> Result<()> {
    let session = pair.warm.session()?;
    let mut matches = vec![];
    let mut labels = vec![];
    for (index, action) in pair
        .warm
        .menu()
        .context("actual menu absent")?
        .actions()
        .iter()
        .enumerate()
    {
        let resolved = resolved_offered_label(action, || {
            let table = session
                .simulation
                .vm
                .action_menu(&session.program, action.target)
                .map_err(anyhow::Error::msg)?;
            table
                .iter()
                .find(|entry| entry.id == action.menu_id)
                .map(|entry| entry.label.clone())
                .context("actual static menu entry identity absent")
        })?;
        let enabled = action.flags & 16 == 0;
        labels.push(
            json!({"index":index,"action":action,"resolved_label":resolved,
            "dynamic_override_present":action.label.is_some(),"enabled":enabled}),
        );
        if enabled && resolved == label {
            matches.push((index, action.clone()));
        }
    }
    write_json(&pair.out.join("actual-offered-labels.json"), &labels)?;
    ensure!(
        matches.len() == 1,
        "expected one actual enabled resolved {label:?} entry, got{}",
        matches.len()
    );
    let (index, expected) = matches.pop().unwrap();
    pair.execute(Command::Choice { index, expected })
}
fn new_directed_serial(
    before: &BTreeSet<u32>,
    records: &[sims_vm::action::Action],
    target: i16,
) -> Result<u32> {
    let candidates: Vec<_> = records
        .iter()
        .filter(|a| !before.contains(&a.id) && a.target == target)
        .collect();
    ensure!(
        candidates.len() == 1,
        "offered action did not create exactly one fresh directed serial"
    );
    Ok(candidates[0].id)
}
fn active_sit(
    app: &Application,
    actor: &Identity,
    chair: &Identity,
    directed_serial: u32,
) -> Result<bool> {
    let owner = app
        .session()?
        .simulation
        .vm
        .world
        .actions
        .as_ref()
        .context("actions absent")?
        .actors
        .get(&actor.object)
        .context("directed actor absent")?;
    if !owner
        .active
        .as_ref()
        .is_some_and(|a| a.id == directed_serial && a.target == chair.object)
    {
        return Ok(false);
    }
    actor.check(app.session()?)?;
    chair.check(app.session()?)?;
    let conditions = Milestone::All {
        conditions: vec![
            Milestone::ActionActive {
                actor: actor.clone(),
                target: chair.object,
            },
            Milestone::SolePersonContained {
                actor: actor.clone(),
                target: chair.clone(),
            },
            Milestone::ObjectUsesAccounted {
                target: chair.clone(),
            },
        ],
    };
    if !conditions.reached(app, 0)? {
        return Ok(false);
    }
    Ok(app
        .session()?
        .simulation
        .characters
        .get(&actor.object)
        .is_some_and(|r| {
            r.character
                .skin
                .animator
                .layers()
                .iter()
                .any(|l| !l.layer.clock.ended && l.layer.name.starts_with("a2o-sit-"))
        }))
}
#[test]
fn tick_guard_rejects_missing_skipped_and_multiple_boundaries() -> Result<()> {
    let row = |tick| Observation {
        tick,
        rng: [Some(1), Some(2), Some(3)],
        state_sha256: "x".into(),
    };
    ensure!(!returned_boundary(5, 5, &[])?);
    ensure!(returned_boundary(5, 6, &[row(6)])?);
    ensure!(returned_boundary(5, 7, &[row(6), row(7)]).is_err());
    ensure!(returned_boundary(5, 6, &[]).is_err());
    ensure!(returned_boundary(5, 5, &[row(5)]).is_err());
    ensure!(returned_boundary(5, 6, &[row(7)]).is_err());
    Ok(())
}
#[test]
fn disk_reserve_parser_uses_available_column() -> Result<()> {
    ensure!(
        available_bytes(
            "Filesystem 1024-blocks Used Available Capacity Mounted\n/dev/test 50 20 30 40% /test\n"
        )? == 30 * 1024
    );
    ensure!(available_bytes("invalid").is_err());
    Ok(())
}
#[test]
fn notice_identity_rejects_changed_source_location_text_buttons_and_owner() -> Result<()> {
    let record: sims_vm::dialog::Record = serde_json::from_value(json!({
        "id":12,"caller":298,"stack_object":298,"source":"test::Cab.iff",
        "origin":{"source":"test::Cab.iff","bhav":4119,"instruction":34},
        "operands":[0,3,70,0,0,0,72,6],"title":" ","body":"cab notice",
        "buttons":[{"label":"OK","result":2}],"bitmap_icon":5003,
        "job_icon":null,"icon_object":null,"status":1
    }))?;
    let allowed = Notice {
        source: record.source.clone(),
        source_sha256: "source-hash".into(),
        caller_guid: 77,
        bhav: 4119,
        instruction: 34,
        title: record.title.clone(),
        body_sha256: engine::digest(record.body.as_bytes()),
        buttons: record.buttons.clone(),
        operands: record.operands,
        bitmap_icon: record.bitmap_icon,
        result: 2,
        evidence: Value::Null,
    };
    ensure!(allowed.matches(&record, 77, "source-hash"));
    ensure!(!allowed.matches(&record, 78, "source-hash"));
    ensure!(!allowed.matches(&record, 77, "changed-source-hash"));
    let mut changed = record.clone();
    changed.origin.instruction += 1;
    ensure!(!allowed.matches(&changed, 77, "source-hash"));
    changed = record.clone();
    changed.source = "different::Cab.iff".into();
    ensure!(!allowed.matches(&changed, 77, "source-hash"));
    changed = record.clone();
    changed.body.push('!');
    ensure!(!allowed.matches(&changed, 77, "source-hash"));
    changed = record.clone();
    changed.buttons[0].result = 3;
    ensure!(!allowed.matches(&changed, 77, "source-hash"));
    changed = record.clone();
    changed.status = 2;
    ensure!(!allowed.matches(&changed, 77, "source-hash"));
    Ok(())
}
#[test]
fn directed_identity_rejects_old_or_ambiguous_later_actions() -> Result<()> {
    use sims_vm::action::Action;
    let before = BTreeSet::from([7]);
    let old = Action {
        id: 7,
        target: 147,
        ..Default::default()
    };
    let directed = Action {
        id: 9,
        target: 147,
        ..Default::default()
    };
    let later = Action {
        id: 10,
        target: 147,
        ..Default::default()
    };
    ensure!(new_directed_serial(&before, &[old.clone(), directed.clone()], 147)? == 9);
    ensure!(new_directed_serial(&before, &[old], 147).is_err());
    ensure!(new_directed_serial(&before, &[directed, later], 147).is_err());
    Ok(())
}
#[test]
#[ignore = "private native-owned Strict Newbie managed fixtures;32pilot before4096"]
fn real_newbie_checkpoint_continuation_compares_every_completed_tick() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("native continuation panicked")
}
fn requested_cases(scope: &str) -> Result<Vec<&'static str>> {
    match scope {
        "all" => Ok(vec!["paused-buy", "post-live", "active-sit"]),
        "safe" => Ok(vec!["paused-buy", "post-live"]),
        "active" => Ok(vec!["active-sit"]),
        _ => anyhow::bail!("unknown continuation case scope {scope:?}"),
    }
}
fn complete(out: &Path, ticks: u64, cases: &[&str]) -> Result<()> {
    write_json(
        &out.join("complete.json"),
        &json!({
        "required_ticks_per_checkpoint":ticks,"checkpoints":cases.len(),"completed_cases":cases,
        "active_interaction_case_included":cases.contains(&"active-sit"),
        "strict_native_parity_ticks":0,
        "off_lot_change_requirement":"see actual per-neighbor delta counts; unchanged state is not positive mutation coverage"}),
    )?;
    write_json(
        &out.join("control.json"),
        &json!({"pid":std::process::id(),"status":"terminal_success"}),
    )?;
    Ok(())
}
#[test]
fn offered_label_uses_real_static_fallback_and_preserves_dynamic_override() -> Result<()> {
    let mut action = sims_vm::action::Action::default();
    ensure!(resolved_offered_label(&action, || Ok("Sit".into()))? == "Sit");
    ensure!(resolved_offered_label(&action, || anyhow::bail!("source absent")).is_err());
    action.label = Some("Rest".into());
    ensure!(resolved_offered_label(&action, || anyhow::bail!("must not read fallback"))? == "Rest");
    action.label = Some(String::new());
    ensure!(
        resolved_offered_label(&action, || anyhow::bail!("must preserve blank override"))? == ""
    );
    Ok(())
}
#[test]
fn case_scope_preserves_all_default_and_does_not_credit_omitted_branches() -> Result<()> {
    ensure!(requested_cases("all")? == ["paused-buy", "post-live", "active-sit"]);
    ensure!(requested_cases("safe")? == ["paused-buy", "post-live"]);
    ensure!(requested_cases("active")? == ["active-sit"]);
    ensure!(requested_cases("unknown").is_err());
    Ok(())
}
fn run() -> Result<()> {
    let repo = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_REPO")?).canonicalize()?;
    let out = repo.join(std::env::var("SIMS_NATIVE_CONTINUATION_OUTPUT")?);
    ensure!(
        out.starts_with(repo.join("cache")),
        "private cache output required"
    );
    std::fs::create_dir(&out)?;
    let ticks = std::env::var("SIMS_NATIVE_CONTINUATION_TICKS")
        .unwrap_or_else(|_| "32".into())
        .parse::<u64>()?;
    ensure!(
        matches!(ticks, 32 | 4096),
        "run32pilot or4096qualified continuation only"
    );
    let scope = std::env::var("SIMS_NATIVE_CONTINUATION_CASES").unwrap_or_else(|_| "all".into());
    let requested = requested_cases(&scope)?;
    write_json(
        &out.join("case-selection.json"),
        &json!({"scope":scope,"requested_cases":requested,
        "required_ticks_per_checkpoint":ticks,"native_parity_credit":0}),
    )?;
    let mut control = Control {
        out: out.clone(),
        paused: Duration::ZERO,
        last: None,
    };
    control.gate("initializing", 0)?;
    let policy = notice_policy()?;
    let policy_path = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_NOTICE_POLICY")?);
    let policy_bytes = std::fs::read(&policy_path)?;
    write_json(
        &out.join("notice-policy-proof.json"),
        &json!({"source":policy_path,
        "sha256":engine::digest(&policy_bytes),"policy":policy}),
    )?;
    space(&out, 1024 * 1024 * 1024)?;
    let source_root = repo.join(BASE);
    let cases = [
        ("paused-buy", "07-escape-unpaid-repeat/final.json", 1),
        ("post-live", "08-return-live/final.json", 2),
    ];
    for (label, file, mode) in cases
        .into_iter()
        .filter(|_| requested.contains(&"paused-buy"))
    {
        let app = restore(&repo, &source_root.join(file), &out)?;
        safe(&app, mode)?;
        let bytes = app.checkpoint()?;
        let mut pair = Pair::new(app, &bytes, out.join(label), &policy)?;
        drop(bytes);
        write_json(
            &pair.out.join("input.json"),
            &json!({"source":source_root.join(file),"owners":identities(&pair.warm)?,
            "actual_safe_mode":mode,"ticks_required":ticks,"observer":"full persisted Session/assets+Client/Device/local, streamedJSON; no omitted mutable fields"}),
        )?;
        let result = (|| -> Result<()> {
            if mode == 1 {
                pair.execute(Command::Live)?;
            }
            pair.progress(
                &mut control,
                ticks,
                Duration::from_secs(if ticks == 32 { 600 } else { 7200 }),
            )?;
            pair.execute(Command::Playing { value: false })?;
            safe(&pair.warm, 2)?;
            ensure!(pair.count >= ticks, "insufficient per-tick comparisons");
            checkpoint(&pair.warm, &pair.out, "completed-safe", 2)?;
            write_json(&pair.out.join("complete.json"), &pair.summary())?;
            Ok(())
        })();
        if let Err(error) = result {
            pair.preserve_failure(&error)?;
            return Err(error);
        }
    }
    if !requested.contains(&"active-sit") {
        return complete(&out, ticks, &requested);
    }
    let app = restore(&repo, &source_root.join("08-return-live/final.json"), &out)?;
    safe(&app, 2)?;
    let bytes = app.checkpoint()?;
    let pair = Pair::new(app, &bytes, out.join("reach-active-Sit"), &policy)?;
    drop(bytes);
    // Keep preparation failure evidence even when the actual original action
    // has not reached an eligible active checkpoint.
    let mut pair = pair;
    write_json(
        &pair.out.join("input.json"),
        &json!({
        "source":source_root.join("08-return-live/final.json"),"owners":identities(&pair.warm)?,
        "actual_safe_mode":2,"ticks_required_after_observed_active_checkpoint":ticks,
        "scope":scope,"observer":"full persisted Session/assets+Client/Device/local; exact directed action identity required"}),
    )?;
    let result = (|| -> Result<()> {
        // Follow the exact ordinary player-directed serial through actual
        // ownership and animation; a queued or later autonomous action cannot qualify.
        let actor = Identity::get(pair.warm.session()?, 105)?;
        let chair = Identity::get(pair.warm.session()?, 147)?;
        pair.execute(Command::Resident {
            identity: actor.clone(),
        })?;
        let owner = &pair
            .warm
            .session()?
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .context("actions absent")?
            .actors[&actor.object];
        if owner.active.is_some() || !owner.queue.pending.is_empty() {
            pair.execute(Command::CancelAll {
                actor: actor.clone(),
            })?;
        }
        pair.execute(Command::Object {
            identity: chair.clone(),
        })?;
        write_json(
            &pair.out.join("actual-offered-actions.json"),
            &pair.warm.menu().context("actual menu absent")?.actions(),
        )?;
        let before: BTreeSet<_> = directed_records(&pair.warm, &actor)?
            .iter()
            .map(|a| a.id)
            .collect();
        choose_offered_label(&mut pair, "Sit")?;
        let serial = new_directed_serial(
            &before,
            &directed_records(&pair.warm, &actor)?,
            chair.object,
        )?;
        write_json(
            &pair.out.join("directed-action-identity.json"),
            &json!({"actor":actor,
            "chair":chair,"serial":serial,"records_after_choice":directed_records(&pair.warm,&actor)?}),
        )?;
        pair.execute(Command::Playing { value: true })?;
        let start = pair.count;
        let wall = Instant::now();
        let paused = control.paused;
        while !active_sit(&pair.warm, &actor, &chair, serial)? {
            ensure!(
                directed_records(&pair.warm, &actor)?
                    .iter()
                    .any(|a| a.id == serial),
                "original directed Sit serial{serial} released before its live animation/containment milestone; later autonomy cannot qualify"
            );
            control.gate(
                "reaching-original-Sit",
                pair.warm.session()?.simulation.vm.tick,
            )?;
            ensure!(
                pair.count - start < 512
                    && wall.elapsed().saturating_sub(control.paused - paused)
                        < Duration::from_secs(900),
                "actual offered Sit did not reach contained/use-owned live animation within512ticks/900activeSeconds"
            );
            if pair.notice()? {
                continue;
            }
            pair.execute(Command::Advance { millis: 1 })?;
        }
        write_json(
            &pair.out.join("actual-active-Sit.json"),
            &json!({"actor":actor,"chair":chair,
            "tick":pair.warm.session()?.simulation.vm.tick,"paired_preparation_ticks":pair.count,"directed_serial":serial,
            "milestone":"active target+exclusive bidirectional containment+accounted use owners+non-ended original a2o-sit animation"}),
        )?;
        pair.execute(Command::Playing { value: false })?;
        let bytes = checkpoint(&pair.warm, &pair.out, "active-safe", 2)?;
        // Original preparation branch continues uninterrupted; only the other
        // branch is replaced by the real managed encode/decode checkpoint.
        pair.cold = cold(&pair.warm, &bytes)?;
        drop(bytes);
        ensure!(
            identities(&pair.warm)? == identities(&pair.cold)?
                && full(&pair.warm)? == full(&pair.cold)?,
            "active managed decode changed owned state"
        );
        let start = pair.count;
        pair.progress(
            &mut control,
            ticks,
            Duration::from_secs(if ticks == 32 { 600 } else { 7200 }),
        )?;
        ensure!(
            pair.count - start >= ticks,
            "active continuation insufficient ticks"
        );
        pair.execute(Command::Playing { value: false })?;
        checkpoint(&pair.warm, &pair.out, "active-completed-safe", 2)?;
        write_json(&pair.out.join("complete.json"), &pair.summary())?;
        Ok(())
    })();
    if let Err(error) = result {
        pair.preserve_failure(&error)?;
        return Err(error);
    }
    complete(&out, ticks, &requested)
}

fn payload_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            if a.keys().ne(b.keys()) {
                return Some(format!("{path}: object keys differ"));
            }
            a.iter().find_map(|(key, value)| {
                payload_difference(value, &b[key], &format!("{path}/{key}"))
            })
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: array lengths {} != {}", a.len(), b.len()));
            }
            a.iter()
                .zip(b)
                .enumerate()
                .find_map(|(index, (a, b))| payload_difference(a, b, &format!("{path}/{index}")))
        }
        _ => (a != b).then(|| format!("{path}: scalar differs")),
    }
}
#[test]
fn full_failure_payload_comparison_reports_missing_owner_and_nested_changes() {
    let state = json!({"vm":{"rng":[1,2,3],"pending":{"request":289}}});
    assert_eq!(payload_difference(&state, &state, ""), None);
    assert_eq!(
        payload_difference(
            &state,
            &json!({"vm":{"rng":[1,2,4],"pending":{"request":289}}}),
            ""
        ),
        Some("/vm/rng/2: scalar differs".into())
    );
    assert!(
        payload_difference(&state, &json!({"vm":{"rng":[1,2,3]}}), "")
            .is_some_and(|path| path.starts_with("/vm: object keys"))
    );
}
#[test]
fn persisted_payload_comparison_uses_identical_number_decode_paths() {
    let scalar = json!(0.9145724177360535_f32);
    let encoded = serde_json::to_vec(&scalar).unwrap();
    let decoded: Value = serde_json::from_slice(&encoded).unwrap();
    // serde_json's configured decimal parser can differ by one f64 ULP from
    // an in-memory f32-to-f64 Value. Persisted-byte comparison must decode
    // both operands alike; the underlying f32 bits must still be identical.
    assert_eq!(
        serde_json::to_vec(&decoded).unwrap(),
        serde_json::to_vec(&serde_json::from_slice::<Value>(&encoded).unwrap()).unwrap()
    );
    assert_eq!(
        (decoded.as_f64().unwrap() as f32).to_bits(),
        0.9145724177360535_f32.to_bits()
    );
}
#[test]
#[ignore = "private suspended failure diagnostics; no stepping or safe player-save credit"]
fn suspended_failure_checkpoint_preserves_full_payload_through_two_cold_restores() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(failure_roundtrip)?
        .join()
        .expect("failure roundtrip panicked")
}
fn failure_roundtrip() -> Result<()> {
    let repo = PathBuf::from(std::env::var("SIMS_NATIVE_CONTINUATION_REPO")?).canonicalize()?;
    let source = repo
        .join(std::env::var("SIMS_NATIVE_CONTINUATION_FAILURE_SOURCE")?)
        .canonicalize()?;
    let out = repo.join(std::env::var("SIMS_NATIVE_CONTINUATION_OUTPUT")?);
    ensure!(
        source.starts_with(repo.join("cache")) && out.starts_with(repo.join("cache")),
        "private failure evidence required"
    );
    std::fs::create_dir(&out)?;
    space(&out, 0)?;
    let original_bytes = std::fs::read(&source)?;
    let original_sha = engine::digest(&original_bytes);
    let original: Value = serde_json::from_slice(&original_bytes)?;
    drop(original_bytes);
    let source_payload_sha = engine::digest_serialized(&original)?;
    let mut app = restore(&repo, &source, &out)?;
    let mut rounds = vec![];
    for round in 0..=2 {
        // Both complete persisted payloads pass through exactly the same JSON
        // decoder. No numeric tolerance or field exclusion is applied.
        let encoded = serde_json::to_vec(&app.capture(true)?)?;
        if round == 0 {
            space(&out, encoded.len() as u64)?;
            std::fs::write(out.join("recaptured-complete-payload.json"), &encoded)?;
        }
        let actual: Value = serde_json::from_slice(&encoded)?;
        if let Some(path) = payload_difference(&original, &actual, "") {
            write_json(
                &out.join("first-difference.json"),
                &json!({"round":round,"path":path,
                "source":source,"source_sha256":original_sha,"safe_player_boundary":false}),
            )?;
            anyhow::bail!("suspended diagnostic cold payload differs at round{round}: {path}");
        }
        ensure!(
            engine::digest_serialized(&actual)? == source_payload_sha,
            "complete canonical persisted payload encoding changed"
        );
        let session = app.session()?;
        let expected_bounds_bits = original
            .pointer("/session/assets/characters/104/geometry/state/bindings/2/bounds/maximum/2")
            .and_then(Value::as_f64)
            .map(|n| (n as f32).to_bits());
        let actual_bounds_bits = session
            .host
            .characters
            .get(&104)
            .and_then(|c| c.geometry.state.bindings.get(2))
            .map(|b| b.bounds.maximum[2].to_bits());
        ensure!(
            expected_bounds_bits == actual_bounds_bits,
            "original diagnostic bounds f32 bits changed"
        );
        let neighbors = session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("neighbor owner absent")?;
        let mut explicit_neighbors = BTreeMap::new();
        for id in [32, 34] {
            let actual = neighbors
                .neighbors
                .get(&id)
                .context("explicit neighbor absent")?;
            let serialized = serde_json::to_value(actual)?;
            ensure!(
                original["session"]["simulation"]["state"]["vm"]["world"]["person"]["neighbors"]
                    [id.to_string()]
                    == serialized,
                "explicit neighbor{id} differs"
            );
            explicit_neighbors.insert(
                id,
                json!({"guid":actual.guid,"dirty":actual.dirty,
                "relationships":actual.relationships,"sha256":engine::digest_serialized(actual)?}),
            );
        }
        rounds.push(json!({"round":round,"full_payload_sha256":engine::digest_serialized(&actual)?,
            "actual_complete_encoded_payload_sha256":engine::digest(&encoded),
            "original_character104_binding2_maximum2_f32_bits":expected_bounds_bits,
            "actual_character104_binding2_maximum2_f32_bits":actual_bounds_bits,
            "full_observation":full(&app)?,"identities":identities(&app)?,"explicit_neighbors":explicit_neighbors,
            "lot_rng_scope":session.simulation.vm.lot_rng_scope,"native_scheduler":session.simulation.native_scheduler}));
        if round < 2 {
            let bytes = app.checkpoint()?;
            let restored = cold(&app, &bytes)?;
            app = restored;
        }
    }
    ensure!(
        engine::digest(&std::fs::read(&source)?) == original_sha,
        "source evidence changed"
    );
    write_json(
        &out.join("complete.json"),
        &json!({"source":source,"source_sha256":original_sha,
        "source_canonical_payload_sha256":source_payload_sha,"rounds":rounds,"strict_native_parity_ticks":0,
        "completed_gameplay_ticks":0,"safe_player_boundary":false,"stepped_or_repaired":false,
        "scope":"full canonical managed diagnostic payload, including suspended owners/owned content/client/device/local; no player-save qualification"}),
    )
}

#[cfg(test)]
#[path = "rejected_person_regressions.rs"]
mod rejected_person_regressions;
