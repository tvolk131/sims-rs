use super::*;
use serde::{Deserialize, Serialize};
use sims_vm::observations::{ActionTerminalEvent, ActionTerminalReason, NativeRouteTerminalEvent};
use std::collections::{BTreeMap, BTreeSet};
#[path = "alpha_activities/fire_recovery.rs"]
mod fire_recovery;

const DAY_SECONDS: i64 = 86_400;
const REQUIRED_DAYS: i64 = 3;
const FRAME_MILLIS: u32 = 250;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    schema: u32,
    household: String,
    input: PathBuf,
    input_sha256: String,
    steps: Vec<Step>,
    #[serde(default)]
    notices: Vec<Notice>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
enum Step {
    Activity(Activity),
    PlayUntil { calendar: i64, max_ticks: u64 },
    SaveResume { compare_frames: u32 },
    CancelActive(Cancellation),
    FurnitureCycle(FurnitureCycle),
    CookingFireRecovery(fire_recovery::CookingFireRecovery),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Cancellation {
    actor: Identity,
    target: Identity,
    routine: u16,
    after_ticks: u64,
    max_ticks: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FurnitureCycle {
    purchased: Identity,
    purchase_price: i32,
    sale_price: i32,
    definition: u32,
    guid: u32,
    stages: Vec<FurnitureStage>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct FurnitureStage {
    kind: FurnitureStageKind,
    commands: Vec<Command>,
    expected: super::super::buy_edit_cycle::Expected,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FurnitureStageKind {
    Purchase,
    Move,
    Rotate,
    MoveRotate,
    Sell,
    Live,
}

#[derive(Default, Serialize)]
struct Coverage {
    autonomous_activities: u32,
    active_cancellations: u32,
    furniture_cycles: u32,
    active_restores: u32,
}

impl Coverage {
    fn complete(&self) -> bool {
        self.autonomous_activities > 0
            && self.active_cancellations > 0
            && self.furniture_cycles > 0
            && self.active_restores > 0
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Activity {
    actor: Identity,
    kind: Kind,
    target: Identity,
    routine: u16,
    max_ticks: u64,
    #[serde(default)]
    resume: Option<Resume>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Resume {
    after_ticks: u64,
    compare_frames: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Meal,
    Hygiene,
    Sleep,
}

impl Kind {
    const ALL: [Self; 3] = [Self::Meal, Self::Hygiene, Self::Sleep];
    fn motive(self) -> usize {
        match self {
            Self::Meal => 7,
            Self::Hygiene => 8,
            Self::Sleep => 5,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Notice {
    source: String,
    source_sha256: String,
    #[serde(default)]
    origin_sha256: Option<String>,
    caller: Identity,
    origin: sims_vm::Location,
    operands: [u8; 8],
    title: String,
    body_sha256: String,
    buttons: Vec<sims_vm::dialog::Button>,
    result: u32,
    max_occurrences: u32,
}

impl Notice {
    fn matches_sources(
        &self,
        source: &str,
        origin: &sims_vm::Location,
        source_hash: Option<&str>,
        origin_hash: Option<&str>,
    ) -> bool {
        let expected_origin = self
            .origin_sha256
            .as_deref()
            .or_else(|| (self.source == self.origin.source).then_some(self.source_sha256.as_str()));
        self.source == source
            && &self.origin == origin
            && !self.source_sha256.is_empty()
            && source_hash == Some(self.source_sha256.as_str())
            && expected_origin
                .is_some_and(|expected| !expected.is_empty() && origin_hash == Some(expected))
    }
}

pub(super) fn household_source(household: &str) -> Result<&'static str> {
    Ok(match household {
        "newbie" => "UserData/Houses/House07.iff",
        "goth" => "UserData/Houses/House05.iff",
        "mashuga" => "UserData2/Houses/House09.iff",
        _ => anyhow::bail!("unknown alpha household"),
    })
}

fn candidate_rule(session: &Session, record: &sims_vm::dialog::Record) -> Value {
    json!({"source":record.source,
        "source_sha256":session.program.modules.get(&record.source).map(|module|&module.content_hash),
        "origin_sha256":session.program.modules.get(&record.origin.source).map(|module|&module.content_hash),
        "caller":Identity::get(session,record.caller).ok(),"origin":record.origin,
        "operands":record.operands,"title":record.title,
        "body_sha256":engine::digest(record.body.as_bytes()),"buttons":record.buttons})
}

struct Progress<'action> {
    notices: &'action [Notice],
    uses: Vec<u32>,
    evidence: Vec<Value>,
}

impl Progress<'_> {
    fn respond(&mut self, app: &mut Application, limits: &Limits) -> Result<u32> {
        let mut responses = 0;
        while let Some(id) = app.session()?.simulation.vm.world.dialogs.modal {
            let session = app.session()?;
            let record = session
                .simulation
                .vm
                .world
                .dialogs
                .records
                .iter()
                .find(|record| record.id == id)
                .context("alpha modal record absent")?
                .clone();
            self.evidence
                .push(json!({"kind":"activity_modal","record":record}));
            let matches: Vec<_> = self
                .notices
                .iter()
                .enumerate()
                .filter(|(_, rule)| {
                    rule.matches_sources(
                        &record.source,
                        &record.origin,
                        session
                            .program
                            .modules
                            .get(&record.source)
                            .map(|module| module.content_hash.as_str()),
                        session
                            .program
                            .modules
                            .get(&record.origin.source)
                            .map(|module| module.content_hash.as_str()),
                    ) && rule.caller.object == record.caller
                        && rule.caller.check(session).is_ok()
                        && rule.operands == record.operands
                        && rule.title == record.title
                        && rule.body_sha256 == engine::digest(record.body.as_bytes())
                        && rule.buttons == record.buttons
                        && record.status == 1
                        && record.modal()
                        && record
                            .buttons
                            .iter()
                            .any(|button| button.result == rule.result)
                })
                .collect();
            if matches.len() != 1 {
                self.evidence.push(json!({"kind":"unplanned_modal",
                    "tick":session.simulation.vm.tick,"calendar":calendar(app)?,
                    "candidate_rule":candidate_rule(session,&record)}));
                anyhow::bail!(
                    "activity reached an unplanned modal; freeze its actual offered response from the retained checkpoint"
                );
            }
            let (index, rule) = matches[0];
            ensure!(
                (1..=16).contains(&rule.max_occurrences) && self.uses[index] < rule.max_occurrences,
                "frozen notice response budget exhausted"
            );
            let command = Command::Dialog {
                id,
                result: rule.result,
                body_sha256: rule.body_sha256.clone(),
            };
            app.execute(&command, limits)?;
            self.uses[index] += 1;
            self.evidence
                .push(json!({"kind":"activity_modal_response","command":command,
                "rule":index,"occurrence":self.uses[index]}));
            responses += 1;
        }
        if responses > 0 {
            app.execute(&Command::Playing { value: true }, limits)?;
        }
        Ok(responses)
    }
}

#[derive(Default)]
struct Frame {
    routes: Vec<NativeRouteTerminalEvent>,
    terminals: Vec<ActionTerminalEvent>,
}

impl Frame {
    fn extend(&mut self, other: Self) {
        self.routes.extend(other.routes);
        self.terminals.extend(other.terminals);
    }
}

struct Shadow<'action> {
    app: Application,
    progress: Progress<'action>,
    remaining: u32,
    label: String,
    start_tick: u64,
}

struct Play<'action> {
    app: Application,
    limits: Limits,
    wall: Instant,
    progress: Progress<'action>,
    shadow: Option<Shadow<'action>>,
    normal_returns: BTreeMap<i16, u64>,
    compared: u32,
    frames: u64,
    coverage: Coverage,
}

fn tick(app: &Application) -> Result<u64> {
    Ok(app.session()?.simulation.vm.tick)
}

fn continuation_components(app: &Application) -> Result<Value> {
    let session = app.session()?;
    Ok(json!({"session":session.save(),
        "client":app.viewer.client.as_ref().context("dialog client absent")?.save(session)?,
        "device":app.viewer.device.as_ref().map(|device| device.save(session)).transpose()?}))
}

fn continuation_differences(
    saved: Option<&Value>,
    loaded: Option<&Value>,
    path: &str,
    differences: &mut Vec<Value>,
) {
    if saved == loaded {
        return;
    }
    match (saved, loaded) {
        (Some(Value::Object(saved)), Some(Value::Object(loaded))) => {
            let keys: BTreeSet<_> = saved.keys().chain(loaded.keys()).collect();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                continuation_differences(
                    saved.get(key),
                    loaded.get(key),
                    &format!("{path}/{escaped}"),
                    differences,
                );
            }
        }
        (Some(Value::Array(saved)), Some(Value::Array(loaded))) => {
            for index in 0..saved.len().max(loaded.len()) {
                continuation_differences(
                    saved.get(index),
                    loaded.get(index),
                    &format!("{path}/{index}"),
                    differences,
                );
            }
        }
        _ => differences.push(json!({"path":path,
            "saved_present":saved.is_some(),"saved":saved,
            "loaded_present":loaded.is_some(),"loaded":loaded})),
    }
}

fn write_continuation_diagnostic(path: &Path, value: &Value) -> Result<Value> {
    let bytes = serde_json::to_vec(value)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    std::io::Write::write_all(&mut file, &bytes)?;
    Ok(json!({"path":path,"sha256":engine::digest(&bytes),"bytes":bytes.len()}))
}

fn retain_continuation_mismatch(
    save: &Path,
    start_tick: u64,
    compared: u32,
    saved: &Value,
    loaded: &Value,
) -> Result<Value> {
    let output = save.parent().context("private save output absent")?;
    let prefix = format!("save-resume-mismatch-{start_tick}-{compared}");
    let saved_file =
        write_continuation_diagnostic(&output.join(format!("{prefix}-saved.json")), saved)?;
    let loaded_file =
        write_continuation_diagnostic(&output.join(format!("{prefix}-loaded.json")), loaded)?;
    let mut components = BTreeMap::new();
    for name in ["session", "client", "device"] {
        let saved_component = saved.get(name).context("saved component absent")?;
        let loaded_component = loaded.get(name).context("loaded component absent")?;
        components.insert(
            name,
            json!({"saved_sha256":engine::digest_serialized(saved_component)?,
                "loaded_sha256":engine::digest_serialized(loaded_component)?,
                "equal":saved_component==loaded_component}),
        );
    }
    let mut differences = Vec::new();
    continuation_differences(Some(saved), Some(loaded), "", &mut differences);
    let differences_file = write_continuation_diagnostic(
        &output.join(format!("{prefix}-differences.json")),
        &json!({"components":components,"difference_count":differences.len(),
            "differences":differences}),
    )?;
    Ok(json!({"saved":saved_file,"loaded":loaded_file,
        "components":components,"difference_count":differences.len(),"differences":differences_file}))
}

fn residents(app: &Application) -> Result<Value> {
    let session = app.session()?;
    let mut residents = Vec::new();
    for identity in app.residents()? {
        let motives = session.simulation.vm.world.objects[&identity.object]
            .motives
            .context("resident motives absent")?;
        residents.push(json!({"identity":identity,
            "energy":f32::from_bits(motives[5]),"hunger":f32::from_bits(motives[7]),
            "hygiene":f32::from_bits(motives[8]),"bladder":f32::from_bits(motives[9]),
            "social":f32::from_bits(motives[14]),"fun":f32::from_bits(motives[15]),
            "position":session.simulation.vm.world.positions.get(&identity.object)}));
    }
    Ok(Value::Array(residents))
}

fn branch_tick_rng(app: &Application) -> Result<(u64, [Option<u32>; 3])> {
    let vm = &app.session()?.simulation.vm;
    Ok((
        vm.tick,
        [Some(vm.rng_state), vm.lot_rng_state, vm.crt_rng_state],
    ))
}

fn save_and_drain(app: &mut Application, limits: &Limits) -> Result<()> {
    let saved_tick_rng = branch_tick_rng(app)?;
    app.dispatch(Message::Save)?;
    app.drain(limits)?;
    ensure!(
        branch_tick_rng(app)? == saved_tick_rng,
        "production Save/drain advanced the native tick or RNG"
    );
    Ok(())
}

impl<'action> Play<'action> {
    fn session(&self) -> Result<&Session> {
        self.app.session()
    }
    fn tick(&self) -> Result<u64> {
        tick(&self.app)
    }
    fn execute(&mut self, command: Command) -> Result<()> {
        ensure!(
            self.shadow.is_none(),
            "player command issued during action restored-continuation comparison"
        );
        self.progress
            .evidence
            .push(json!({"kind":"player_command_attempted",
            "tick":tick(&self.app)?,"calendar":calendar(&self.app)?,"command":command}));
        self.app.execute(&command, &self.limits)?;
        self.progress
            .evidence
            .push(json!({"kind":"player_command_returned",
            "tick":tick(&self.app)?,"command":command}));
        Ok(())
    }
    fn observe_events(&mut self) -> Result<Frame> {
        let simulation = &mut self
            .app
            .viewer
            .session
            .as_mut()
            .context("session absent")?
            .simulation;
        ensure!(
            simulation.vm.terminal_observations().enabled()
                && simulation.vm.terminal_observations().dropped() == 0
                && simulation.route_terminal_observations().dropped() == 0,
            "alpha observer lost evidence"
        );
        let frame = Frame {
            routes: simulation.drain_route_terminal_observations(),
            terminals: simulation.vm.drain_terminal_observations(),
        };
        for event in &frame.terminals {
            if event.reason == ActionTerminalReason::NormalReturn {
                *self.normal_returns.entry(event.actor).or_default() += 1;
            }
        }
        Ok(frame)
    }
    fn frame(&mut self) -> Result<Frame> {
        ensure!(
            self.wall.elapsed().as_secs_f64() < self.limits.wall_seconds,
            "alpha scenario wall limit"
        );
        self.app.execute(
            &Command::Advance {
                millis: FRAME_MILLIS,
            },
            &self.limits,
        )?;
        self.progress.respond(&mut self.app, &self.limits)?;
        ensure!(
            self.app.viewer.playing,
            "uncommanded pause during alpha scenario"
        );
        self.frames += 1;
        if self.frames % 512 == 0 {
            println!(
                "{}",
                json!({"phase":"alpha_scenario","tick":self.tick()?,"calendar":calendar(&self.app)?,
                "frames":self.frames,"wall_seconds":self.wall.elapsed().as_secs_f64()})
            );
        }
        let frame = self.observe_events()?;
        if let Some(mut shadow) = self.shadow.take() {
            shadow.app.execute(
                &Command::Advance {
                    millis: FRAME_MILLIS,
                },
                &self.limits,
            )?;
            shadow.progress.respond(&mut shadow.app, &self.limits)?;
            ensure!(
                shadow.app.viewer.playing,
                "restored continuation paused without action command"
            );
            shadow.remaining -= 1;
            if shadow.remaining > 0 {
                self.shadow = Some(shadow);
            } else {
                self.compare(shadow)?;
            }
        }
        Ok(frame)
    }
    fn compare(&mut self, shadow: Shadow<'_>) -> Result<()> {
        let uninterrupted = continuation(&self.app)?;
        let restored = continuation(&shadow.app)?;
        let main = branch_tick_rng(&self.app)?;
        let other = branch_tick_rng(&shadow.app)?;
        let same = uninterrupted == restored
            && main == other
            && self.progress.uses == shadow.progress.uses;
        self.progress
            .evidence
            .push(json!({"kind":"save_resume_compared",
            "label":shadow.label,"start_tick":shadow.start_tick,"tick":main.0,
            "uninterrupted_sha256":uninterrupted,"restored_sha256":restored,
            "uninterrupted_tick_rng":main,"restored_tick_rng":other,
            "restored_modal_responses":shadow.progress.evidence.iter()
                .filter(|event|event["kind"]=="activity_modal_response").count(),
            "restored_evidence":shadow.progress.evidence,
            "consistent":same}));
        ensure!(
            same,
            "uninterrupted and production-saved/restored continuations diverged"
        );
        self.compared += 1;
        Ok(())
    }
    fn begin_shadow(&mut self, frames: u32, label: String) -> Result<Frame> {
        ensure!(
            (1..=4096).contains(&frames) && self.shadow.is_none(),
            "invalid restored-continuation comparison"
        );
        self.app
            .execute(&Command::Playing { value: false }, &self.limits)?;
        self.app.execute(
            &Command::Advance {
                millis: FRAME_MILLIS,
            },
            &self.limits,
        )?;
        let paused = self.observe_events()?;
        ensure!(
            self.session()?.simulation.vm.world.dialogs.modal.is_none() && !self.app.viewer.playing,
            "restored-continuation comparison needs action paused, modal-free boundary"
        );
        save_and_drain(&mut self.app, &self.limits)?;
        ensure!(
            self.app.viewer.args.save.is_file(),
            "production Save handler produced no file"
        );
        let saved_components = continuation_components(&self.app)?;
        let expected = engine::digest_serialized(&saved_components)?;
        let mut restored = self.app.fork()?;
        restored.dispatch(Message::Load)?;
        restored.drain(&self.limits)?;
        let loaded_components = continuation_components(&restored)?;
        let actual = engine::digest_serialized(&loaded_components)?;
        let start_tick = self.tick()?;
        self.progress.evidence.push(json!({"kind":"save_resume_loaded","label":label,
            "tick":start_tick,"calendar":calendar(&self.app)?,"saved_sha256":expected,
            "loaded_sha256":actual,"save":self.app.viewer.args.save,
            "save_file_sha256":engine::digest(&std::fs::read(&self.app.viewer.args.save)?),
            "saved_tick_rng":branch_tick_rng(&self.app)?,"loaded_tick_rng":branch_tick_rng(&restored)?}));
        if expected != actual {
            let diagnostics = retain_continuation_mismatch(
                &self.app.viewer.args.save,
                start_tick,
                self.compared,
                &saved_components,
                &loaded_components,
            );
            self.progress.evidence.push(match diagnostics {
                Ok(files) => json!({"kind":"save_resume_mismatch_diagnostics",
                    "label":label,"tick":start_tick,"files":files}),
                Err(error) => json!({"kind":"save_resume_mismatch_diagnostics",
                    "label":label,"tick":start_tick,"diagnostic_error":format!("{error:#}")}),
            });
        }
        ensure!(
            expected == actual && branch_tick_rng(&self.app)? == branch_tick_rng(&restored)?,
            "production Save/Load did not restore the saved continuation"
        );
        self.app
            .execute(&Command::Playing { value: true }, &self.limits)?;
        restored.execute(&Command::Playing { value: true }, &self.limits)?;
        self.shadow = Some(Shadow {
            app: restored,
            progress: Progress {
                notices: self.progress.notices,
                uses: self.progress.uses.clone(),
                evidence: Vec::new(),
            },
            remaining: frames,
            label,
            start_tick,
        });
        Ok(paused)
    }
    fn finish_shadow(&mut self) -> Result<()> {
        while self.shadow.is_some() {
            self.frame()?;
        }
        Ok(())
    }
    fn clear_actions(&mut self, actor: &Identity) -> Result<()> {
        let original = action_ids(&self.app, actor.object)?;
        if original.is_empty() {
            return Ok(());
        }
        let start = self.tick()?;
        self.execute(Command::CancelAll {
            actor: actor.clone(),
        })?;
        self.execute(Command::Playing { value: true })?;
        while action_ids(&self.app, actor.object)?
            .iter()
            .any(|serial| original.contains(serial))
        {
            ensure!(
                self.tick()? - start < 3_000,
                "prelude cancellation tick limit"
            );
            self.frame()?;
        }
        self.progress
            .evidence
            .push(json!({"kind":"prelude_cancelled","actor":actor,
            "actions":original,"ticks":tick(&self.app)?-start}));
        self.execute(Command::Playing { value: false })
    }
}

fn private_existing(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = root.join(path).canonicalize()?;
    ensure!(
        path.starts_with(root.join("cache")),
        "alpha inputs must stay in cache"
    );
    Ok(path)
}

fn motive(app: &Application, actor: &Identity, kind: Kind) -> Result<f32> {
    actor.check(app.session()?)?;
    let value = f32::from_bits(
        app.session()?.simulation.vm.world.objects[&actor.object]
            .motives
            .context("resident motives absent")?[kind.motive()],
    );
    ensure!(value.is_finite(), "non-finite resident motive");
    Ok(value)
}

fn content_targets(
    app: &Application,
    actor: &Identity,
    kind: Kind,
) -> Result<BTreeSet<(u32, u32)>> {
    Ok(app.session()?.simulation.vm.world.objects[&actor.object]
        .motive_changes
        .iter()
        .filter(|change| usize::from(change.motive) == kind.motive())
        .map(|change| (change.delta, change.target))
        .collect())
}

fn describe(targets: &BTreeSet<(u32, u32)>) -> Value {
    targets
        .iter()
        .map(|&(delta, target)| {
            json!({"delta_per_tick":f32::from_bits(delta),"target":f32::from_bits(target)})
        })
        .collect()
}

fn improvement(before: f32, peak: f32, targets: &BTreeSet<(u32, u32)>) -> Result<()> {
    ensure!(
        peak > before,
        "completed activity did not improve its motive: start {before}, peak {peak}, observed content motive changes {}",
        describe(targets)
    );
    Ok(())
}

fn positive_content_effect(
    kind: Kind,
    live_use: bool,
    before: f32,
    after: f32,
    changes: &BTreeSet<(u32, u32)>,
) -> bool {
    live_use
        && after > before
        && (kind == Kind::Meal
            || changes.iter().any(|&(delta, limit)| {
                f32::from_bits(delta) > 0. && f32::from_bits(limit) > before
            }))
}

fn three_days(
    completed: &BTreeMap<Kind, u32>,
    start_calendar: i64,
    end_calendar: i64,
    compared: u32,
) -> bool {
    Kind::ALL
        .iter()
        .all(|kind| completed.get(kind).is_some_and(|&count| count > 0))
        && end_calendar - start_calendar >= REQUIRED_DAYS * DAY_SECONDS
        && compared > 0
}

fn carried_by(vm: &sims_vm::Vm, object: i16, actor: i16) -> bool {
    let mut current = object;
    let mut seen = BTreeSet::new();
    while seen.insert(current) {
        if current == actor {
            return true;
        }
        let Some((parent, _)) = vm
            .world
            .objects
            .get(&current)
            .and_then(|object| object.container)
        else {
            return false;
        };
        current = parent;
    }
    false
}

fn released_target(vm: &sims_vm::Vm, actor: i16, target: i16) -> Result<bool> {
    if vm.world.objects.contains_key(&target) {
        return super::super::milestone::interaction_released(vm, actor, target);
    }
    let owner = vm
        .world
        .actions
        .as_ref()
        .and_then(|action| action.actors.get(&actor))
        .context("actor action owner absent during destroyed-food cleanup")?;
    Ok(owner
        .active
        .iter()
        .chain(owner.queue.pending.iter())
        .all(|action| action.target != target)
        && !vm.world.objects[&actor]
            .container
            .is_some_and(|(id, _)| id == target)
        && vm.threads.values().all(|thread| {
            !thread
                .stack
                .iter()
                .any(|frame| frame.caller == actor && frame.action_target == Some(target))
                && !thread
                    .completed_actions
                    .iter()
                    .any(|(action, action_target, ..)| *action == actor && *action_target == target)
        }))
}

fn activity(
    play: &mut Play<'_>,
    spec: &Activity,
    index: usize,
    expected_failure: Option<&fire_recovery::FailedMeal>,
) -> Result<()> {
    ensure!(
        expected_failure.is_none()
            || (spec.kind == Kind::Meal && spec.resume.is_none() && play.shadow.is_none()),
        "failed-meal observation cannot overlap restored-continuation work"
    );
    ensure!(
        (1..=50_000).contains(&spec.max_ticks),
        "invalid activity tick budget"
    );
    ensure!(
        spec.resume
            .as_ref()
            .is_none_or(|record| record.after_ticks < spec.max_ticks
                && (1..=4096).contains(&record.compare_frames)),
        "invalid in-action restored-continuation point"
    );
    let actor = &spec.actor;
    actor.check(play.session()?)?;
    spec.target.check(play.session()?)?;
    play.execute(Command::Resident {
        identity: actor.clone(),
    })?;
    play.clear_actions(actor)?;
    let before_motive = motive(&play.app, actor, spec.kind)?;
    let before_calendar = calendar(&play.app)?;
    let initial_objects: BTreeSet<_> = {
        let session = play.session()?;
        session
            .simulation
            .vm
            .world
            .objects
            .keys()
            .map(|&id| {
                Identity::get(session, id).map(|identity| (identity.object, identity.handle))
            })
            .collect::<Result<_>>()?
    };
    play.execute(Command::Object {
        identity: spec.target.clone(),
    })?;
    let offered: Vec<_> = play
        .app
        .menu()
        .context("activity menu absent")?
        .actions()
        .iter()
        .enumerate()
        .filter(|(_, action)| action.flags & 16 == 0 && action.routine == spec.routine)
        .map(|(index, action)| (index, action.clone()))
        .collect();
    ensure!(
        offered.len() == 1,
        "frozen routine is not action unique enabled offered action"
    );
    let (choice, expected) = offered.into_iter().next().unwrap();
    let previous = action_ids(&play.app, actor.object)?;
    let target = Identity::get(play.session()?, expected.target)?;
    play.progress.evidence.push(
        json!({"kind":"activity_started","step":index,"specification":spec,
        "actual_offered_action":expected,"before_motive":before_motive,
        "calendar":before_calendar,"tick":tick(&play.app)?}),
    );
    play.execute(Command::Choice {
        index: choice,
        expected,
    })?;
    let serials: Vec<_> = action_ids(&play.app, actor.object)?
        .into_iter()
        .filter(|serial| !previous.contains(serial))
        .collect();
    ensure!(
        serials.len() == 1,
        "activity choice did not enqueue exactly one new action"
    );
    let serial = serials[0];
    let start = play.tick()?;
    let mut peak = before_motive;
    let mut targets = BTreeSet::new();
    let mut created_and_carried_food = false;
    let mut contained_in_target = false;
    let mut routes = Vec::new();
    let mut parent_returned = false;
    let mut meal_food = BTreeMap::new();
    let mut eating_actions = BTreeMap::new();
    let mut eating_return = None;
    let mut resumed = false;
    let mut previous_motive = before_motive;
    let mut content_effect = false;
    let mut carried = Frame::default();
    play.execute(Command::Playing { value: true })?;
    loop {
        ensure!(
            play.tick()? - start < spec.max_ticks,
            "activity completion tick limit"
        );
        let mut frame = std::mem::take(&mut carried);
        frame.extend(play.frame()?);
        let current_motive = motive(&play.app, actor, spec.kind)?;
        peak = peak.max(current_motive);
        let changes = content_targets(&play.app, actor, spec.kind)?;
        targets.extend(changes.iter().copied());
        let session = play.app.session()?;
        contained_in_target |= session.simulation.vm.world.objects[&actor.object]
            .container
            .is_some_and(|(parent, _)| parent == target.object);
        if spec.kind == Kind::Meal {
            for (&id, object) in &session.simulation.vm.world.objects {
                if object
                    .definition
                    .as_ref()
                    .is_some_and(|play| play.source.to_ascii_lowercase().contains("food"))
                    && carried_by(&session.simulation.vm, id, actor.object)
                {
                    let identity = Identity::get(session, id)?;
                    created_and_carried_food |= !initial_objects.contains(&(id, identity.handle));
                    if !parent_returned && !initial_objects.contains(&(id, identity.handle)) {
                        meal_food.insert(id, identity.handle);
                    }
                }
            }
            if let Some(action) = session
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|actions| actions.actors.get(&actor.object))
                .and_then(|owner| owner.active.as_ref())
                .filter(|action| meal_food.contains_key(&action.target))
            {
                let identity = Identity::get(session, action.target)?;
                ensure!(
                    meal_food[&action.target] == identity.handle,
                    "meal food lifetime changed"
                );
                let menu = session
                    .simulation
                    .vm
                    .action_menu(&session.program, action.target)
                    .map_err(anyhow::Error::msg)?;
                if let Some(entry) = menu.iter().find(|entry| {
                    entry.id == action.menu_id
                        && entry.action == action.routine
                        && entry.label.eq_ignore_ascii_case("Eat")
                }) && eating_actions.insert(action.id, action.target).is_none()
                {
                    play.progress
                        .evidence
                        .push(json!({"kind":"meal_eating_started","action":action,
                        "food":identity,"actual_menu_entry":entry}));
                }
            }
        }
        let active = session
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|actions| actions.actors.get(&actor.object))
            .and_then(|owner| owner.active.as_ref());
        let using_required_object = active.is_some_and(|action| {
            let required = if spec.kind == Kind::Meal {
                eating_actions.get(&action.id) == Some(&action.target)
            } else {
                action.id == serial && action.target == target.object
            };
            required
                && session.simulation.vm.threads.values().any(|thread| {
                    thread.stack.iter().any(|frame| {
                        frame.caller == actor.object && frame.action_target == Some(action.target)
                    })
                })
        });
        let gained = positive_content_effect(
            spec.kind,
            using_required_object,
            previous_motive,
            current_motive,
            &changes,
        );
        if gained && !content_effect {
            let definition = active
                .and_then(|action| session.simulation.vm.world.objects.get(&action.target))
                .and_then(|object| object.definition.as_ref());
            play.progress.evidence.push(json!({"kind":"motive_effect_observed","step":index,
                "actor":actor,"motive":spec.kind.motive(),"before":previous_motive,"after":current_motive,
                "action":active,"source_definition":definition,"tick":session.simulation.vm.tick,
                "source_sha256":definition.and_then(|definition|session.program.modules.get(&definition.source))
                    .map(|module|&module.content_hash),"scheduled_changes":describe(&changes)}));
        }
        content_effect |= gained;
        previous_motive = current_motive;
        routes.extend(frame.routes);
        if let Some(terminal) = frame
            .terminals
            .iter()
            .find(|event| event.actor == actor.object && event.serial == serial)
        {
            play.progress.evidence.push(json!({"kind":"activity_retired","step":index,
                "specification":spec,"terminal":terminal,"routes":routes,"start_tick":start,
                "final_tick":tick(&play.app)?,"calendar":calendar(&play.app)?,"before_motive":before_motive,
                "peak_motive":peak,"content_motive_changes":describe(&targets),
                "created_and_carried_food":created_and_carried_food,
                "contained_in_target":contained_in_target,"restored_comparison_started":resumed}));
            ensure!(
                terminal.reason == ActionTerminalReason::NormalReturn
                    && terminal.result == Some(true)
                    && !terminal.cancelled
                    && terminal.target == target.object
                    && terminal.invocation.is_some(),
                "activity lacks its own successful qualified normal return"
            );
            ensure!(
                routes.iter().any(|record| record.actor == actor.object
                    && record.action_serial == Some(serial)
                    && record.action_invocation == terminal.invocation
                    && record.success),
                "activity lacks action correlated successful route"
            );
            ensure!(
                spec.resume.is_none() || resumed,
                "in-action restored-continuation point was not reached during the action"
            );
            if spec.kind == Kind::Meal {
                ensure!(
                    created_and_carried_food,
                    "meal did not create food carried by its actor"
                );
            }
            if spec.kind == Kind::Sleep {
                ensure!(contained_in_target, "sleep never occupied its bed");
            }
            parent_returned = true;
        }
        for terminal in frame.terminals.iter().filter(|event| {
            event.actor == actor.object && eating_actions.get(&event.serial) == Some(&event.target)
        }) {
            play.progress
                .evidence
                .push(json!({"kind":"meal_eating_retired","terminal":terminal,
                "before_motive":before_motive,"peak_motive":peak}));
            if let Some(expected) = expected_failure {
                ensure!(
                    parent_returned
                        && created_and_carried_food
                        && expected.matches(terminal)
                        && meal_food.get(&expected.food.object) == Some(&expected.food.handle),
                    "cooking-fire recovery did not observe its exact qualified failed meal"
                );
                let session = play.session()?;
                ensure!(
                    !session.simulation.vm.world.objects.contains_key(&expected.food.object)
                        || Identity::get(session, expected.food.object)?.handle != expected.food.handle,
                    "expected destroyed meal food lifetime still exists"
                );
                play.progress.evidence.push(json!({"kind":"expected_failed_meal",
                    "step":index,"terminal":terminal,"food":expected.food,
                    "meal_credit":false,"before_motive":before_motive,"peak_motive":peak}));
                return play.execute(Command::Playing { value: false });
            }
            ensure!(
                terminal.reason == ActionTerminalReason::NormalReturn
                    && terminal.result == Some(true)
                    && !terminal.cancelled
                    && terminal.invocation.is_some(),
                "meal eating lacks its own successful qualified normal return"
            );
            eating_return = Some(terminal.target);
        }
        if parent_returned && (spec.kind != Kind::Meal || eating_return.is_some()) {
            improvement(before_motive, peak, &targets)?;
            ensure!(
                content_effect,
                "activity lacks an observed positive content motive effect during use"
            );
            if let Some(food) = eating_return {
                let session = play.session()?;
                ensure!(
                    session.simulation.vm.world.objects.get(&food).is_none()
                        || Identity::get(session, food)?.handle != meal_food[&food],
                    "meal eating returned without consuming its created food lifetime"
                );
                play.progress
                    .evidence
                    .push(json!({"kind":"meal_food_consumed",
                    "food":{"object":food,"handle":meal_food[&food]},"tick":tick(&play.app)?}));
            }
            loop {
                let vm = &play.session()?.simulation.vm;
                let parent_released = released_target(vm, actor.object, target.object)?;
                let food_released = match eating_return {
                    Some(food) => released_target(vm, actor.object, food)?,
                    None => true,
                };
                if parent_released && food_released {
                    break;
                }
                ensure!(
                    play.tick()? - start < spec.max_ticks,
                    "activity cleanup tick limit"
                );
                play.frame()?;
            }
            break;
        }
        if let Some(resume) = &spec.resume
            && !resumed
            && play.tick()? - start >= resume.after_ticks
        {
            let session = play.session()?;
            let active = session
                .simulation
                .vm
                .world
                .actions
                .as_ref()
                .and_then(|actions| actions.actors.get(&actor.object))
                .and_then(|owner| owner.active.as_ref());
            ensure!(
                !parent_returned
                    && active
                        .is_some_and(|action| action.id == serial && action.target == target.object)
                    && session.simulation.vm.threads.values().any(|thread| thread
                        .stack
                        .iter()
                        .any(|frame| frame.caller == actor.object
                            && frame.action_target == Some(target.object))),
                "in-action save point is not inside the selected live action invocation"
            );
            play.progress
                .evidence
                .push(json!({"kind":"active_save_boundary",
                "actor":actor,"target":target,"action":active,"tick":tick(&play.app)?}));
            carried =
                play.begin_shadow(resume.compare_frames, format!("step {index} active action"))?;
            resumed = true;
        }
    }
    ensure!(
        expected_failure.is_none(),
        "expected failed meal unexpectedly completed"
    );
    play.finish_shadow()?;
    if resumed {
        play.coverage.active_restores += 1;
    }
    play.progress
        .evidence
        .push(json!({"kind":"activity_completed","step":index,
        "kind_completed":spec.kind,"actor":actor,"tick":tick(&play.app)?,"calendar":calendar(&play.app)?,
        "after_motive":motive(&play.app, actor, spec.kind)?,"content_effect_observed":content_effect,
        "released":true,"food_lifetime_consumed":eating_return.is_some()}));
    play.execute(Command::Playing { value: false })
}

fn cancel_active(play: &mut Play<'_>, spec: &Cancellation, index: usize) -> Result<()> {
    ensure!(
        (1..=30_000).contains(&spec.max_ticks) && spec.after_ticks < spec.max_ticks,
        "invalid active cancellation bounds"
    );
    spec.actor.check(play.session()?)?;
    spec.target.check(play.session()?)?;
    play.execute(Command::Resident {
        identity: spec.actor.clone(),
    })?;
    play.clear_actions(&spec.actor)?;
    play.execute(Command::Object {
        identity: spec.target.clone(),
    })?;
    let offered: Vec<_> = play
        .app
        .menu()
        .context("cancellation menu absent")?
        .actions()
        .iter()
        .enumerate()
        .filter(|(_, action)| action.flags & 16 == 0 && action.routine == spec.routine)
        .map(|(choice, action)| (choice, action.clone()))
        .collect();
    ensure!(
        offered.len() == 1,
        "frozen cancellation action is not uniquely offered"
    );
    let (choice, expected) = offered.into_iter().next().unwrap();
    let target = Identity::get(play.session()?, expected.target)?;
    let previous = action_ids(&play.app, spec.actor.object)?;
    play.execute(Command::Choice {
        index: choice,
        expected,
    })?;
    let added: Vec<_> = action_ids(&play.app, spec.actor.object)?
        .into_iter()
        .filter(|serial| !previous.contains(serial))
        .collect();
    ensure!(
        added.len() == 1,
        "cancellation choice did not add one action"
    );
    let serial = added[0];
    let start = play.tick()?;
    play.execute(Command::Playing { value: true })?;
    loop {
        ensure!(
            play.tick()? - start < spec.max_ticks,
            "cancellation activation tick limit"
        );
        let frame = play.frame()?;
        ensure!(
            !frame
                .terminals
                .iter()
                .any(|event| event.actor == spec.actor.object && event.serial == serial),
            "selected cancellation action retired before cancellation"
        );
        let session = play.session()?;
        let active = session
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|actions| actions.actors.get(&spec.actor.object))
            .and_then(|owner| owner.active.as_ref());
        if play.tick()? - start >= spec.after_ticks
            && active.is_some_and(|action| action.id == serial)
            && session.simulation.vm.threads.values().any(|thread| {
                thread.stack.iter().any(|frame| {
                    frame.caller == spec.actor.object && frame.action_target == Some(target.object)
                })
            })
        {
            play.progress
                .evidence
                .push(json!({"kind":"active_cancellation_started","step":index,
                "actor":spec.actor,"target":target,"action":active,"tick":tick(&play.app)?}));
            break;
        }
    }
    play.execute(Command::CancelAction {
        actor: spec.actor.clone(),
        serial,
    })?;
    let mut retired = false;
    loop {
        ensure!(
            play.tick()? - start < spec.max_ticks,
            "active cancellation completion tick limit"
        );
        let frame = play.frame()?;
        for terminal in frame
            .terminals
            .iter()
            .filter(|event| event.actor == spec.actor.object && event.serial == serial)
        {
            play.progress
                .evidence
                .push(json!({"kind":"active_cancellation_retired","terminal":terminal}));
            ensure!(
                terminal.target == target.object
                    && terminal.invocation.is_some()
                    && terminal.reason == ActionTerminalReason::NormalReturn
                    && terminal.cancelled
                    && terminal.result.is_some(),
                "active cancellation lacks qualified cancelled normal unwind"
            );
            retired = true;
        }
        if retired
            && released_target(
                &play.session()?.simulation.vm,
                spec.actor.object,
                target.object,
            )?
        {
            play.progress
                .evidence
                .push(json!({"kind":"active_cancellation_completed",
                "step":index,"actor":spec.actor,"target":target,"serial":serial,
                "tick":tick(&play.app)?,"released":true}));
            play.coverage.active_cancellations += 1;
            break;
        }
    }
    play.execute(Command::Playing { value: false })
}

#[derive(Clone, Deserialize)]
struct FurnitureSnapshot {
    mode: i32,
    funds: i32,
    ready_paused: bool,
    identities: Vec<Identity>,
    furniture: Vec<FurniturePlacement>,
    sales: Vec<FurnitureSale>,
    history_handles: Vec<u32>,
}

#[derive(Clone, Deserialize)]
struct FurniturePlacement {
    identity: Identity,
    position: [i32; 2],
    level: i32,
    direction: i16,
}

#[derive(Clone, Deserialize)]
struct FurnitureSale {
    identity: Identity,
    cost: i32,
    reference_count: u32,
    undo_owned: bool,
    off_lot: bool,
    rendered: bool,
}

impl FurnitureSnapshot {
    fn sale_is_undo_owned(&self, identity: &Identity, price: i32) -> bool {
        self.identities.contains(identity)
            && self.history_handles.contains(&identity.handle)
            && self.furniture.iter().any(|placement| {
                placement.identity == *identity
                    && placement.position.iter().all(|coordinate| *coordinate < 0)
            })
            && self.sales.iter().any(|sale| {
                sale.identity == *identity
                    && sale.cost == price
                    && sale.reference_count > 0
                    && sale.undo_owned
                    && sale.off_lot
                    && !sale.rendered
            })
    }

    fn lifetime_released(&self, identity: &Identity) -> bool {
        !self.identities.contains(identity)
            && !self.history_handles.contains(&identity.handle)
            && self
                .furniture
                .iter()
                .all(|placement| placement.identity != *identity)
            && self.sales.iter().all(|sale| sale.identity != *identity)
    }
}

fn furniture_snapshot(app: &Application) -> Result<FurnitureSnapshot> {
    Ok(serde_json::from_value(serde_json::to_value(
        super::super::buy_edit_cycle::capture(app)?,
    )?)?)
}

fn furniture_cycle(play: &mut Play<'_>, spec: &FurnitureCycle, index: usize) -> Result<()> {
    let required = [
        FurnitureStageKind::Purchase,
        FurnitureStageKind::Move,
        FurnitureStageKind::Rotate,
        FurnitureStageKind::Sell,
        FurnitureStageKind::Live,
    ];
    let combined = [
        FurnitureStageKind::Purchase,
        FurnitureStageKind::MoveRotate,
        FurnitureStageKind::Sell,
        FurnitureStageKind::Live,
    ];
    ensure!(
        (spec.stages.iter().map(|stage| stage.kind).eq(required)
            || spec.stages.iter().map(|stage| stage.kind).eq(combined))
            && spec.purchase_price > 0
            && spec.sale_price > 0
            && spec.sale_price <= spec.purchase_price,
        "furniture cycle needs purchase, move, rotate, sell and Live with positive frozen prices"
    );
    play.execute(Command::Playing { value: false })?;
    let initial = furniture_snapshot(&play.app)?;
    ensure!(
        !initial.identities.contains(&spec.purchased),
        "purchase identity existed before cycle"
    );
    let mut before = initial;
    let mut purchases = 0;
    let mut purchased_definition = None;
    for stage in &spec.stages {
        ensure!(
            !stage.commands.is_empty() && stage.commands.len() <= 64,
            "invalid frozen furniture command count"
        );
        for command in &stage.commands {
            ensure!(
                matches!(
                    command,
                    Command::Buy
                        | Command::Live
                        | Command::ExpandPanel
                        | Command::BuyNavigate { .. }
                        | Command::BuyProduct { .. }
                        | Command::ViewportEnter
                        | Command::ViewportLeave
                        | Command::ViewportMotion { .. }
                        | Command::ViewportButton { .. }
                        | Command::Escape
                        | Command::SellFurniture { .. }
                        | Command::Resize { .. }
                        | Command::Rotate
                        | Command::ZoomIn
                        | Command::ZoomOut
                        | Command::Floor { .. }
                        | Command::Check { .. }
                ),
                "non-furniture command inside frozen cycle"
            );
            if let Command::BuyProduct {
                index,
                expected_definition,
                expected_guid,
                ..
            } = command
            {
                ensure!(
                    stage.kind == FurnitureStageKind::Purchase
                        && *expected_definition == spec.definition
                        && *expected_guid == spec.guid,
                    "purchase differs from frozen product"
                );
                purchases += 1;
                let catalog = super::super::buy_edit_cycle::catalog(&play.app)?;
                let product = catalog
                    .product(*index)
                    .context("frozen catalog product absent")?;
                ensure!(
                    product.definition == *expected_definition
                        && product.source.guid == *expected_guid,
                    "frozen catalog product identity differs"
                );
                purchased_definition = Some(product.source.clone());
                play.progress
                    .evidence
                    .push(json!({"kind":"furniture_product_selected",
                    "registry_definition":product.definition,"source_definition":product.source}));
            }
            play.execute(command.clone())?;
        }
        let actual = super::super::buy_edit_cycle::capture(&play.app)?;
        play.progress
            .evidence
            .push(json!({"kind":"furniture_stage","step":index,
            "stage":stage.kind,"expected":stage.expected,"actual":actual}));
        ensure!(
            stage.expected.matches(&actual),
            "frozen furniture stage state differs"
        );
        let after = furniture_snapshot(&play.app)?;
        ensure!(
            after.ready_paused,
            "furniture stage is not settled and paused"
        );
        let prior_placement = before
            .furniture
            .iter()
            .find(|entry| entry.identity == spec.purchased);
        let placement = after
            .furniture
            .iter()
            .find(|entry| entry.identity == spec.purchased);
        match stage.kind {
            FurnitureStageKind::Purchase => {
                ensure!(
                    purchases == 1
                        && after.mode == 1
                        && placement.is_some()
                        && before.funds - after.funds == spec.purchase_price,
                    "purchase lacks placement or exact debit"
                );
                spec.purchased.check(play.session()?)?;
                ensure!(
                    play.session()?.simulation.vm.world.objects[&spec.purchased.object]
                        .definition
                        .as_ref()
                        == purchased_definition.as_ref(),
                    "purchased lifetime differs from catalog product"
                );
            }
            FurnitureStageKind::Move
            | FurnitureStageKind::Rotate
            | FurnitureStageKind::MoveRotate => {
                let prior = prior_placement.context("furniture absent before edit")?;
                let placed = placement.context("furniture absent after edit")?;
                ensure!(
                    after.mode == 1 && after.funds == before.funds,
                    "furniture edit changed funds or mode"
                );
                if stage.kind != FurnitureStageKind::Rotate {
                    ensure!(
                        (prior.position, prior.level) != (placed.position, placed.level),
                        "furniture move did not change placement"
                    );
                }
                if stage.kind != FurnitureStageKind::Move {
                    ensure!(
                        prior.direction != placed.direction,
                        "furniture rotation did not change direction"
                    );
                }
            }
            FurnitureStageKind::Sell => ensure!(
                after.mode == 1
                    && prior_placement.is_some()
                    && after.sale_is_undo_owned(&spec.purchased, spec.sale_price)
                    && after.funds - before.funds == spec.sale_price,
                "furniture sale lacks off-lot undo ownership or exact credit"
            ),
            FurnitureStageKind::Live => ensure!(
                after.mode == 2
                    && after.lifetime_released(&spec.purchased)
                    && after.funds == before.funds,
                "furniture cycle did not release its sold lifetime and history at unchanged Live funds"
            ),
        }
        before = after;
    }
    let start = play.tick()?;
    play.execute(Command::Playing { value: true })?;
    for _ in 0..16 {
        play.frame()?;
    }
    ensure!(
        play.tick()? > start,
        "furniture cycle prevented continued Live ticks"
    );
    ensure!(
        furniture_snapshot(&play.app)?.lifetime_released(&spec.purchased),
        "sold furniture lifetime or history returned during continued Live play"
    );
    play.progress
        .evidence
        .push(json!({"kind":"furniture_cycle_completed","step":index,
        "purchased":spec.purchased,"continued_live_ticks":tick(&play.app)?-start}));
    play.coverage.furniture_cycles += 1;
    play.execute(Command::Playing { value: false })
}

fn play_until(play: &mut Play<'_>, target: i64, max_ticks: u64, index: usize) -> Result<()> {
    ensure!(
        (1..=100_000).contains(&max_ticks),
        "invalid play-until tick budget"
    );
    let start_tick = play.tick()?;
    let start_calendar = calendar(&play.app)?;
    let returns = play.normal_returns.clone();
    let residents = play.app.residents()?;
    let mut initial = BTreeSet::new();
    for actor in &residents {
        initial.extend(
            action_ids(&play.app, actor.object)?
                .into_iter()
                .map(|serial| (actor.object, serial)),
        );
    }
    let mut observed = BTreeMap::new();
    let mut retired = BTreeMap::new();
    let mut completed_autonomous = Vec::new();
    if start_calendar < target {
        play.execute(Command::Playing { value: true })?;
        while calendar(&play.app)? < target {
            ensure!(
                play.tick()? - start_tick < max_ticks,
                "play-until tick limit"
            );
            let session = play.session()?;
            for actor in &residents {
                actor.check(session)?;
                let active = session
                    .simulation
                    .vm
                    .world
                    .actions
                    .as_ref()
                    .and_then(|actions| actions.actors.get(&actor.object))
                    .and_then(|owner| owner.active.as_ref());
                if let Some(action) = active {
                    let key = (actor.object, action.id);
                    if !initial.contains(&key)
                        && !observed.contains_key(&key)
                        && action.target > 0
                        && action.flags & sims_vm::action::CANCELLED == 0
                        && session.simulation.vm.threads.values().any(|thread| {
                            thread.stack.iter().any(|frame| {
                                frame.caller == actor.object
                                    && frame.action_target == Some(action.target)
                            })
                        })
                    {
                        observed.insert(
                            key,
                            (
                                actor.clone(),
                                action.clone(),
                                Identity::get(session, action.target)?,
                            ),
                        );
                    }
                }
            }
            let frame = play.frame()?;
            for terminal in frame.terminals {
                let key = (terminal.actor, terminal.serial);
                if let Some((_, action, _)) = observed.get(&key)
                    && terminal.target == action.target
                    && terminal.reason == ActionTerminalReason::NormalReturn
                    && terminal.result == Some(true)
                    && !terminal.cancelled
                    && terminal.invocation.is_some()
                {
                    retired.insert(key, terminal);
                }
            }
            let keys: Vec<_> = retired.keys().copied().collect();
            for key in keys {
                let (actor, action, object) = &observed[&key];
                if released_target(&play.session()?.simulation.vm, actor.object, object.object)? {
                    let terminal = retired.remove(&key).unwrap();
                    completed_autonomous.push(json!({"actor":actor,"action":action,"target":object,
                        "terminal":terminal,"released":true,"no_player_action_commands_in_interval":true}));
                    play.coverage.autonomous_activities += 1;
                    observed.remove(&key);
                    initial.insert(key);
                }
            }
        }
        play.finish_shadow()?;
        play.execute(Command::Playing { value: false })?;
    }
    let autonomous: BTreeMap<_, _> = play
        .normal_returns
        .iter()
        .map(|(actor, count)| (*actor, count - returns.get(actor).copied().unwrap_or(0)))
        .filter(|(_, count)| *count > 0)
        .collect();
    play.progress
        .evidence
        .push(json!({"kind":"play_until","step":index,"target":target,
        "already_reached":start_calendar >= target,"start_tick":start_tick,"tick":tick(&play.app)?,
        "start_calendar":start_calendar,"calendar":calendar(&play.app)?,
        "normal_returns_by_actor":autonomous,"completed_autonomous_activities":completed_autonomous,
        "residents":self::residents(&play.app)?}));
    Ok(())
}

#[test]
#[ignore = "private frozen SIMS_ALPHA_SCENARIO and fresh SIMS_ALPHA_OUTPUT required"]
fn complete_frozen_domestic_activities() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("alpha activity worker panicked")
}

fn run() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let scenario_path = private_existing(&root, Path::new(&std::env::var("SIMS_ALPHA_SCENARIO")?))?;
    let bytes = std::fs::read(&scenario_path)?;
    let scenario: Scenario = serde_json::from_slice(&bytes)?;
    ensure!(
        scenario.schema == 2 && !scenario.steps.is_empty() && scenario.steps.len() <= 64,
        "unsupported alpha household scenario"
    );
    let input = private_existing(&root, &scenario.input)?;
    let input_bytes = std::fs::read(&input)?;
    ensure!(
        engine::digest(&input_bytes) == scenario.input_sha256,
        "frozen activity input changed"
    );
    let saved: Value = serde_json::from_slice(&input_bytes)?;
    let source = saved["session"]["source"]
        .as_str()
        .context("managed input source absent")?;
    ensure!(
        source == household_source(&scenario.household)?,
        "scenario input belongs to action different household"
    );
    let relative = PathBuf::from(std::env::var("SIMS_ALPHA_OUTPUT")?);
    ensure!(
        relative.starts_with("cache")
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "alpha activity output must stay beneath cache"
    );
    let output = root.join(relative);
    std::fs::create_dir(&output)?;
    let game = root.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        source,
        "--paused",
        "--root",
        game.to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
        "--save",
        output.join("player-save.json").to_str().unwrap(),
    ]);
    let limits = Limits {
        commands: 100_000,
        completed_ticks: 400_000,
        service_polls: 20_000,
        wall_seconds: 7_200.,
    };
    let wall = Instant::now();
    let mut driver = None;
    let mut early_evidence = Vec::new();
    let mut completed = BTreeMap::new();
    let mut bounds = None;
    let result = (|| -> Result<()> {
        let ready = Arc::new(Ready::load(&args)?);
        let mut app = Application::install_ready(&args, ready, Instant::now(), &limits)?;
        app.execute(&Command::Live, &limits)?;
        app.viewer
            .session
            .as_mut()
            .context("session absent")?
            .simulation
            .enable_terminal_observations(4096);
        let start = (app.session()?.simulation.vm.tick, calendar(&app)?);
        early_evidence.push(
            json!({"kind":"scenario_started","household":scenario.household,
            "tick":start.0,"calendar":start.1}),
        );
        let play = driver.insert(Play {
            app,
            limits: limits.clone(),
            wall,
            progress: Progress {
                notices: &scenario.notices,
                uses: vec![0; scenario.notices.len()],
                evidence: Vec::new(),
            },
            shadow: None,
            normal_returns: BTreeMap::new(),
            compared: 0,
            frames: 0,
            coverage: Coverage::default(),
        });
        for (index, step) in scenario.steps.iter().enumerate() {
            match step {
                Step::Activity(spec) => {
                    activity(play, spec, index, None)?;
                    *completed.entry(spec.kind).or_insert(0u32) += 1;
                }
                Step::CookingFireRecovery(spec) => {
                    fire_recovery::run(play, spec, index)?;
                    *completed.entry(Kind::Meal).or_insert(0u32) += 1;
                }
                Step::PlayUntil {
                    calendar: target,
                    max_ticks,
                } => play_until(play, *target, *max_ticks, index)?,
                Step::CancelActive(spec) => cancel_active(play, spec, index)?,
                Step::FurnitureCycle(spec) => furniture_cycle(play, spec, index)?,
                Step::SaveResume { compare_frames } => {
                    let paused =
                        play.begin_shadow(*compare_frames, format!("step {index} boundary"))?;
                    ensure!(
                        paused.terminals.is_empty() && paused.routes.is_empty(),
                        "boundary comparison paused frame retired evidence"
                    );
                    play.finish_shadow()?;
                    play.execute(Command::Playing { value: false })?;
                }
            }
        }
        bounds = Some((start, (play.tick()?, calendar(&play.app)?)));
        save_and_drain(&mut play.app, &limits)?;
        Ok(())
    })();
    let mut resume = Value::Null;
    let resume_result = match (&result, &driver) {
        (Ok(()), Some(play)) => (|| -> Result<()> {
            let resumed_args = Args::parse_from([
                "sims-room",
                "--original",
                source,
                "--paused",
                "--root",
                game.to_str().unwrap(),
                "--load",
                output.join("player-save.json").to_str().unwrap(),
                "--save",
                output.join("resumed-player-save.json").to_str().unwrap(),
            ]);
            let ready = Arc::new(Ready::load(&resumed_args)?);
            let resumed =
                Application::install_ready(&resumed_args, ready, Instant::now(), &limits)?;
            let saved_components = continuation_components(&play.app)?;
            let loaded_components = continuation_components(&resumed)?;
            let expected = engine::digest_serialized(&saved_components)?;
            let actual = engine::digest_serialized(&loaded_components)?;
            resume = json!({"save_file_sha256":engine::digest(&std::fs::read(output.join("player-save.json"))?),
                "saved_sha256":expected,"loaded_sha256":actual,
                "saved_tick_rng":branch_tick_rng(&play.app)?,"loaded_tick_rng":branch_tick_rng(&resumed)?});
            if expected != actual {
                resume["diagnostics"] = match retain_continuation_mismatch(
                    &output.join("player-save.json"),
                    play.tick()?,
                    play.compared,
                    &saved_components,
                    &loaded_components,
                ) {
                    Ok(files) => files,
                    Err(error) => json!({"diagnostic_error":format!("{error:#}")}),
                };
            }
            ensure!(
                expected == actual && branch_tick_rng(&play.app)? == branch_tick_rng(&resumed)?,
                "explicit Load of the final managed save differs from the saved continuation"
            );
            Ok(())
        })(),
        _ => Ok(()),
    };
    let mut evidence = early_evidence;
    let mut compared = 0;
    if let Some(play) = &driver {
        evidence.extend(play.progress.evidence.iter().cloned());
        compared = play.compared;
        if result.is_err() || resume_result.is_err() {
            match play.app.checkpoint() {
                Ok(bytes) => std::fs::write(output.join("failure-checkpoint.json"), bytes)?,
                Err(error) => {
                    evidence.push(
                        json!({"kind":"failure_checkpoint_unavailable","error":format!("{error:#}")}),
                    );
                    if let Some(session) = play.app.viewer.session.as_ref() {
                        let path = output.join("failure-session-state.json");
                        let diagnostic = json!({
                            "schema":1,"restorable_application_checkpoint":false,
                            "scope":"Raw failure diagnostics; no camera/client validation or restore acceptance",
                            "error":result.as_ref().err().map(|error|format!("{error:#}")),
                            "session":session.save(),
                        });
                        match write_json(&path, &diagnostic) {
                            Ok(()) => evidence.push(json!({"kind":"failure_session_state","path":path})),
                            Err(error) => evidence.push(json!({"kind":"failure_session_state_unavailable","error":format!("{error:#}")})),
                        }
                    }
                }
            }
        }
    }
    let complete = result.is_ok()
        && resume_result.is_ok()
        && driver.as_ref().is_some_and(|play| play.coverage.complete())
        && bounds
            .is_some_and(|((_, start), (_, end))| three_days(&completed, start, end, compared));
    write_json(
        &output.join("activity-report.json"),
        &json!({"schema":2,"household":scenario.household,"activities_passed":result.is_ok(),
        "completed_activities":completed,"restored_continuations_compared":compared,
        "coverage":driver.as_ref().map(|play| &play.coverage),
        "bounds":bounds.map(|((t0,c0),(t1,c1))|json!({"start_tick":t0,"start_calendar":c0,
            "end_tick":t1,"end_calendar":c1,"completed_ticks":t1-t0,"calendar_seconds":c1-c0})),
        "three_day_acceptance_complete":complete,"explicit_load_resume":resume,
        "explicit_load_resume_passed":result.is_ok() && resume_result.is_ok(),
        "scenario_sha256":engine::digest(&bytes),"input_sha256":scenario.input_sha256,
        "wall_seconds":wall.elapsed().as_secs_f64(),
        "error":result.as_ref().err().map(|event|format!("{event:#}")),
        "resume_error":resume_result.as_ref().err().map(|event|format!("{event:#}")),"evidence":evidence}),
    )?;
    result?;
    resume_result?;
    ensure!(
        complete,
        "strict three-day acceptance incomplete; inspect retained activity report and coverage"
    );
    Ok(())
}

#[test]
#[ignore = "private final managed save and fresh SIMS_ALPHA_FINAL_LOAD_OUTPUT required"]
fn final_managed_load_uses_symmetric_quiescent_save_boundary() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(final_managed_load_roundtrip)?
        .join()
        .expect("final managed load worker panicked")
}

fn final_managed_load_roundtrip() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = private_existing(
        &root,
        Path::new(&std::env::var("SIMS_ALPHA_FINAL_LOAD_INPUT")?),
    )?;
    let input_bytes = std::fs::read(&input)?;
    let saved: Value = serde_json::from_slice(&input_bytes)?;
    ensure!(saved["session"]["version"] == 2);
    let source = saved["session"]["source"]
        .as_str()
        .context("managed input source absent")?;
    let relative = PathBuf::from(std::env::var("SIMS_ALPHA_FINAL_LOAD_OUTPUT")?);
    ensure!(
        relative.starts_with("cache/playable-alpha")
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
    );
    let output = root.join(relative);
    std::fs::create_dir(&output)?;
    ensure!(
        output
            .canonicalize()?
            .starts_with(root.join("cache").canonicalize()?)
    );
    let game = root.join("game-data/cabinet-extracted");
    let save = output.join("player-save.json");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        source,
        "--paused",
        "--root",
        game.to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
        "--save",
        save.to_str().unwrap(),
    ]);
    let limits = Limits {
        commands: 32,
        completed_ticks: 1,
        service_polls: 20_000,
        wall_seconds: 180.,
    };
    let ready = Arc::new(Ready::load(&args)?);
    let mut app = Application::install_ready(&args, ready, Instant::now(), &limits)?;
    let initial_tick_rng = branch_tick_rng(&app)?;
    save_and_drain(&mut app, &limits)?;
    let mut resumed_args = args.clone();
    resumed_args.load = Some(save.clone());
    resumed_args.save = output.join("resumed-player-save.json");
    let ready = Arc::new(Ready::load(&resumed_args)?);
    let resumed = Application::install_ready(&resumed_args, ready, Instant::now(), &limits)?;
    let saved_components = continuation_components(&app)?;
    let loaded_components = continuation_components(&resumed)?;
    let expected = engine::digest_serialized(&saved_components)?;
    let actual = engine::digest_serialized(&loaded_components)?;
    let equal = saved_components == loaded_components
        && expected == actual
        && branch_tick_rng(&app)? == initial_tick_rng
        && branch_tick_rng(&resumed)? == initial_tick_rng;
    let diagnostics = if equal {
        Value::Null
    } else {
        retain_continuation_mismatch(
            &save,
            initial_tick_rng.0,
            0,
            &saved_components,
            &loaded_components,
        )?
    };
    write_json(
        &output.join("roundtrip.json"),
        &json!({"input":input,"input_sha256":engine::digest(&input_bytes),
            "save_file_sha256":engine::digest(&std::fs::read(&save)?),
            "saved_sha256":expected,"loaded_sha256":actual,
            "saved_tick_rng":branch_tick_rng(&app)?,"loaded_tick_rng":branch_tick_rng(&resumed)?,
            "full_equality":equal,"diagnostics":diagnostics,"qualification_complete":false}),
    )?;
    ensure!(
        equal,
        "final managed cold Load changed the full continuation"
    );
    Ok(())
}

#[test]
#[ignore = "private failed-meal checkpoint; diagnostic recovery only, no qualification credit"]
fn actual_fire_menu_and_extinguish_checkpoint_diagnostic() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(fire_checkpoint_diagnostic)?
        .join()
        .expect("fire diagnostic worker panicked")
}

fn fire_checkpoint_diagnostic() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let input = private_existing(&root, Path::new(&std::env::var("SIMS_FIRE_INPUT")?))?;
    let scenario_path = private_existing(&root, Path::new(&std::env::var("SIMS_FIRE_SCENARIO")?))?;
    let scenario: Scenario = serde_json::from_slice(&std::fs::read(scenario_path)?)?;
    ensure!(scenario.household == "goth");
    let relative = PathBuf::from(std::env::var("SIMS_FIRE_OUTPUT")?);
    ensure!(
        relative.starts_with("cache/playable-alpha")
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
    );
    let output = root.join(relative);
    std::fs::create_dir(&output)?;
    ensure!(
        output
            .canonicalize()?
            .starts_with(root.join("cache").canonicalize()?)
    );
    let game = root.join("game-data/cabinet-extracted");
    let args = Args::parse_from([
        "sims-room",
        "--original",
        "UserData/Houses/House05.iff",
        "--paused",
        "--root",
        game.to_str().unwrap(),
        "--load",
        input.to_str().unwrap(),
    ]);
    let ready = Ready::load(&args)?;
    let bytes = std::fs::read(&input)?;
    let limits = Limits::default();
    let mut app = Application::restore_checkpoint(
        &args,
        ready
            .restore_index
            .as_ref()
            .context("fire restore index absent")?,
        &ready.icons,
        Instant::now(),
        &bytes,
    )?;
    app.check_errors()?;
    app.viewer
        .session
        .as_mut()
        .unwrap()
        .simulation
        .enable_terminal_observations(4096);
    let mut play = Play {
        app,
        limits,
        wall: Instant::now(),
        progress: Progress {
            notices: &scenario.notices,
            uses: vec![0; scenario.notices.len()],
            evidence: Vec::new(),
        },
        shadow: None,
        normal_returns: BTreeMap::new(),
        compared: 0,
        frames: 0,
        coverage: Coverage::default(),
    };
    let actor: Identity = match std::env::var("SIMS_FIRE_ACTOR") {
        Ok(value) => serde_json::from_str(&value)?,
        Err(std::env::VarError::NotPresent) => Identity {
            object: 21,
            handle: 65,
        },
        Err(error) => return Err(error.into()),
    };
    ensure!(matches!((actor.object, actor.handle), (21, 65) | (18, 66)));
    let fire = Identity {
        object: 320,
        handle: 585,
    };
    let result = (|| -> Result<()> {
        actor.check(play.session()?)?;
        fire.check(play.session()?)?;
        let vm = &play.session()?.simulation.vm;
        let definition = vm.world.objects[&fire.object]
            .definition
            .as_ref()
            .context("fire definition absent")?;
        ensure!(definition.guid == 617176985 && definition.source.ends_with("::Fire.iff"));
        ensure!(vm.world.fires.registered == [fire.object]);
        play.execute(Command::Resident {
            identity: actor.clone(),
        })?;
        play.execute(Command::Object {
            identity: fire.clone(),
        })?;
        let offers = play
            .app
            .menu()
            .context("fire menu absent")?
            .actions()
            .to_vec();
        let source_menu = play
            .session()?
            .simulation
            .vm
            .action_menu(&play.session()?.program, fire.object)
            .map_err(anyhow::Error::msg)?
            .to_vec();
        play.progress
            .evidence
            .push(json!({"kind":"actual_fire_menu","actor":actor,"fire":fire,
            "offered_actions":offers,"source_menu":source_menu,"tick":play.tick()?}));
        let choices: Vec<_> = offers
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                action.flags & 16 == 0
                    && action.routine == 4107
                    && source_menu.iter().any(|entry| {
                        entry.id == action.menu_id
                            && entry.action == action.routine
                            && entry.label.eq_ignore_ascii_case("Extinguish")
                    })
            })
            .map(|(index, action)| (index, action.clone()))
            .collect();
        ensure!(
            choices.len() == 1,
            "fire lacks one actual enabled Extinguish offer"
        );
        let (index, expected) = choices.into_iter().next().unwrap();
        let previous = action_ids(&play.app, actor.object)?;
        let prior_active = play
            .session()?
            .simulation
            .vm
            .world
            .actions
            .as_ref()
            .and_then(|state| state.actors.get(&actor.object))
            .and_then(|owner| owner.active.as_ref())
            .cloned();
        play.execute(Command::Choice {
            index,
            expected: expected.clone(),
        })?;
        let serials: Vec<_> = action_ids(&play.app, actor.object)?
            .into_iter()
            .filter(|serial| !previous.contains(serial))
            .collect();
        ensure!(
            serials.len() == 1,
            "Extinguish did not enqueue one new action"
        );
        let serial = serials[0];
        if let Some(active) = prior_active {
            play.progress.evidence.push(json!({"kind":"cancel_prior_active_for_extinguish",
                "actor":actor,"action":active,"queued_extinguish_serial":serial,"tick":play.tick()?}));
            play.execute(Command::CancelAction {
                actor: actor.clone(),
                serial: active.id,
            })?;
        }
        let start = play.tick()?;
        let mut returned = false;
        play.execute(Command::Playing { value: true })?;
        loop {
            ensure!(
                play.tick()? - start < 6000,
                "fire diagnostic resolution tick limit"
            );
            let frame = play.frame()?;
            play.progress.evidence.push(json!({"kind":"fire_resolution_frame","tick":play.tick()?,
                "registered":play.session()?.simulation.vm.world.fires.registered,"terminals":frame.terminals}));
            for terminal in &frame.terminals {
                if terminal.actor == actor.object && terminal.serial == serial {
                    ensure!(
                        terminal.target == fire.object
                            && terminal.reason == ActionTerminalReason::NormalReturn
                            && terminal.result == Some(true)
                            && !terminal.cancelled
                            && terminal.invocation.is_some(),
                        "Extinguish lacks a successful qualified normal return"
                    );
                    returned = true;
                }
            }
            let vm = &play.session()?.simulation.vm;
            ensure!(
                vm.world
                    .fires
                    .registered
                    .iter()
                    .all(|object| *object == fire.object),
                "fire spread during diagnostic"
            );
            actor.check(play.session()?)?;
            if returned
                && vm.world.fires.registered.is_empty()
                && !vm.world.objects.contains_key(&fire.object)
                && released_target(vm, actor.object, fire.object)?
            {
                play.progress
                    .evidence
                    .push(json!({"kind":"fire_resolved","actor":actor,"fire":fire,
                    "serial":serial,"tick":play.tick()?,"released":true}));
                return Ok(());
            }
        }
    })();
    if let Ok(checkpoint) = play.app.checkpoint() {
        std::fs::write(output.join("checkpoint.json"), checkpoint)?;
    }
    write_json(
        &output.join("report.json"),
        &json!({"schema":1,"input_sha256":engine::digest(&bytes),
        "diagnostic_only":true,"qualification_credit":false,"error":result.as_ref().err().map(|error|format!("{error:#}")),
        "evidence":play.progress.evidence,"frames":play.frames,"state":play.app.capture(false).ok()}),
    )?;
    result
}

#[test]
fn household_notice_pins_both_subject_and_semiglobal_sources() {
    let subject_hash = "a".repeat(64);
    let origin_hash = "b".repeat(64);
    let mut notice: Notice = serde_json::from_value(json!({
        "source":"Objects.far::phones.iff","source_sha256":subject_hash,
        "origin_sha256":origin_hash,
        "caller":{"object":195,"handle":12},
        "origin":{"source":"Global.far::PhoneGlobals.iff","bhav":8216,"instruction":53},
        "operands":[0,1,136,0,0,0,0,6],"title":"Sylvia Marie",
        "body_sha256":"reward","buttons":[{"label":"OK","result":2}],
        "result":2,"max_occurrences":1
    }))
    .unwrap();
    let source = notice.source.clone();
    let origin = notice.origin.clone();
    assert!(notice.matches_sources(&source, &origin, Some(&subject_hash), Some(&origin_hash)));
    for (actual_subject, actual_origin) in [
        (None, Some(origin_hash.as_str())),
        (Some(subject_hash.as_str()), None),
        (Some(origin_hash.as_str()), Some(origin_hash.as_str())),
        (Some(subject_hash.as_str()), Some(subject_hash.as_str())),
    ] {
        assert!(!notice.matches_sources(&source, &origin, actual_subject, actual_origin));
    }
    assert!(!notice.matches_sources(
        "Objects.far::other.iff",
        &origin,
        Some(&subject_hash),
        Some(&origin_hash)
    ));
    let mut different_origin = origin.clone();
    different_origin.instruction += 1;
    assert!(!notice.matches_sources(
        &source,
        &different_origin,
        Some(&subject_hash),
        Some(&origin_hash)
    ));
    different_origin = origin.clone();
    different_origin.source = source.clone();
    assert!(!notice.matches_sources(
        &source,
        &different_origin,
        Some(&subject_hash),
        Some(&origin_hash)
    ));
    notice.origin_sha256 = None;
    assert!(!notice.matches_sources(&source, &origin, Some(&subject_hash), Some(&origin_hash)));
    notice.origin_sha256 = Some(String::new());
    assert!(!notice.matches_sources(&source, &origin, Some(&subject_hash), Some("")));
}

#[test]
fn household_notice_same_module_pins_preserve_exact_ownership() {
    let hash = "a".repeat(64);
    let mut notice: Notice = serde_json::from_value(json!({
        "source":"Objects.far::CarPortal.iff","source_sha256":hash,
        "caller":{"object":196,"handle":11},
        "origin":{"source":"Objects.far::CarPortal.iff","bhav":4103,"instruction":14},
        "operands":[0,46,5,0,16,0,4,8],"title":"Frankie",
        "body_sha256":"earnings","buttons":[{"label":"OK","result":2}],
        "result":2,"max_occurrences":2
    }))
    .unwrap();
    let source = notice.source.clone();
    let origin = notice.origin.clone();
    assert!(notice.matches_sources(&source, &origin, Some(&hash), Some(&hash)));
    assert!(!notice.matches_sources(&source, &origin, Some(&hash), None));
    assert!(!notice.matches_sources(&source, &origin, None, Some(&hash)));
    notice.origin_sha256 = Some("b".repeat(64));
    assert!(!notice.matches_sources(&source, &origin, Some(&hash), Some(&hash)));
    notice.origin_sha256 = Some(hash.clone());
    assert!(notice.matches_sources(&source, &origin, Some(&hash), Some(&hash)));
    notice.source_sha256.clear();
    assert!(!notice.matches_sources(&source, &origin, Some(""), Some(&hash)));
}

#[test]
fn restore_mismatch_diagnostics_preserve_all_components_and_exact_differences() {
    let saved = json!({"session":{"tick":123,"active_action":134},
        "client":{"selected":196},
        "device":{"generation":4709,"published_generation":4709,
            "literal/key~":null,"array":[1,null]}});
    let bytes = serde_json::to_vec(&saved).unwrap();
    let mut loaded: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved, loaded);
    assert_eq!(
        engine::digest(&bytes),
        engine::digest_serialized(&loaded).unwrap()
    );
    let mut differences = Vec::new();
    continuation_differences(Some(&saved), Some(&loaded), "", &mut differences);
    assert!(differences.is_empty());
    loaded["device"]["generation"] = json!(4710);
    loaded["device"]["published_generation"] = json!(4710);
    assert_ne!(
        engine::digest_serialized(&saved).unwrap(),
        engine::digest_serialized(&loaded).unwrap()
    );
    loaded["device"]
        .as_object_mut()
        .unwrap()
        .remove("literal/key~");
    loaded["device"]["array"][1] = json!(false);
    loaded["device"]["array"]
        .as_array_mut()
        .unwrap()
        .push(Value::Null);
    loaded["client"]["selected"] = json!(195);
    loaded["session"]["tick"] = json!(124);
    continuation_differences(Some(&saved), Some(&loaded), "", &mut differences);
    assert_eq!(
        differences,
        vec![
            json!({"path":"/client/selected","saved_present":true,"saved":196,
                "loaded_present":true,"loaded":195}),
            json!({"path":"/device/array/1","saved_present":true,"saved":null,
                "loaded_present":true,"loaded":false}),
            json!({"path":"/device/array/2","saved_present":false,"saved":null,
                "loaded_present":true,"loaded":null}),
            json!({"path":"/device/generation","saved_present":true,"saved":4709,
                "loaded_present":true,"loaded":4710}),
            json!({"path":"/device/literal~1key~0","saved_present":true,"saved":null,
                "loaded_present":false,"loaded":null}),
            json!({"path":"/device/published_generation","saved_present":true,"saved":4709,
                "loaded_present":true,"loaded":4710}),
            json!({"path":"/session/tick","saved_present":true,"saved":123,
                "loaded_present":true,"loaded":124}),
        ]
    );
}

#[test]
fn household_scenario_steps_parse_exact_fields() {
    let steps: Vec<Step> = serde_json::from_value(json!([
        {"step":"activity","actor":{"object":18,"handle":66},"kind":"hygiene",
         "target":{"object":112,"handle":18},"routine":4102,"max_ticks":6000},
        {"step":"activity","actor":{"object":18,"handle":66},"kind":"sleep",
         "target":{"object":124,"handle":55},"routine":4106,"max_ticks":30000,
         "resume":{"after_ticks":3000,"compare_frames":240}},
        {"step":"play_until","calendar":50400,"max_ticks":20000},
        {"step":"save_resume","compare_frames":240}
    ]))
    .unwrap();
    assert_eq!(steps.len(), 4);
    assert!(
        matches!(&steps[0], Step::Activity(action) if action.kind == Kind::Hygiene && action.resume.is_none())
    );
    assert!(
        matches!(&steps[1], Step::Activity(Activity { resume: Some(record), .. }) if record.after_ticks == 3000)
    );
    assert_eq!(
        steps[2],
        Step::PlayUntil {
            calendar: 50400,
            max_ticks: 20000
        }
    );
    for rejected in [
        json!({"step":"play_until","calendar":1,"max_ticks":1,"speed":3}),
        json!({"step":"activity","actor":{"object":18,"handle":66},"kind":"hygiene",
            "target":{"object":112,"handle":18},"routine":4102,"max_ticks":1,"skip":true}),
        json!({"step":"teleport"}),
    ] {
        assert!(serde_json::from_value::<Step>(rejected).is_err());
    }
}

#[test]
fn moderate_tub_hygiene_target_cannot_raise_an_already_cleaner_sim() {
    let change = sims_vm::motives::Change::new(8, 203, 80).unwrap();
    let mut clean = [0u32; 16];
    clean[8] = 89.99f32.to_bits();
    let before = clean;
    for _ in 0..1800 {
        sims_vm::motives::apply_changes(&mut clean, std::slice::from_ref(&change));
    }
    assert_eq!(clean, before);
    let targets = BTreeSet::from([(change.delta, change.target)]);
    let error = improvement(89.99, 89.99, &targets).unwrap_err().to_string();
    assert!(error.contains("\"target\":80.0"), "{error}");
    let mut dirty = [0u32; 16];
    dirty[8] = 60f32.to_bits();
    for _ in 0..1800 {
        sims_vm::motives::apply_changes(&mut dirty, std::slice::from_ref(&change));
    }
    assert_eq!(f32::from_bits(dirty[8]), 80.);
    improvement(60., f32::from_bits(dirty[8]), &targets).unwrap();
}

#[test]
fn three_day_acceptance_needs_every_activity_full_days_and_a_restore() {
    let all = BTreeMap::from([(Kind::Meal, 3), (Kind::Hygiene, 1), (Kind::Sleep, 2)]);
    let start = 23_504;
    let end = start + REQUIRED_DAYS * DAY_SECONDS;
    assert!(three_days(&all, start, end, 1));
    assert!(!three_days(&all, start, end - 1, 1));
    assert!(!three_days(&all, start, end, 0));
    let mut missing = all.clone();
    missing.insert(Kind::Hygiene, 0);
    assert!(!three_days(&missing, start, end, 1));
    missing.remove(&Kind::Hygiene);
    assert!(!three_days(&missing, start, end, 1));
}

#[test]
fn strict_coverage_needs_autonomy_completed_active_cancellation_furniture_and_active_restore() {
    let complete = Coverage {
        autonomous_activities: 1,
        active_cancellations: 1,
        furniture_cycles: 1,
        active_restores: 1,
    };
    assert!(complete.complete());
    assert!(!Coverage::default().complete());
    for missing in 0..4 {
        let mut coverage = Coverage {
            autonomous_activities: 1,
            active_cancellations: 1,
            furniture_cycles: 1,
            active_restores: 1,
        };
        match missing {
            0 => coverage.autonomous_activities = 0,
            1 => coverage.active_cancellations = 0,
            2 => coverage.furniture_cycles = 0,
            _ => coverage.active_restores = 0,
        }
        assert!(!coverage.complete());
    }
}

#[test]
fn cancellation_and_furniture_steps_reject_unfrozen_fields() {
    let cancellation = json!({"step":"cancel_active","actor":{"object":18,"handle":66},
        "target":{"object":124,"handle":55},"routine":4106,"after_ticks":1000,"max_ticks":6000});
    assert!(serde_json::from_value::<Step>(cancellation.clone()).is_ok());
    let mut unexpected = cancellation;
    unexpected["force_success"] = json!(true);
    assert!(serde_json::from_value::<Step>(unexpected).is_err());
    assert!(
        serde_json::from_value::<Step>(json!({"step":"furniture_cycle",
        "purchased":{"object":264,"handle":400},"purchase_price":80,"sale_price":64,
        "definition":16808,"guid":1,"stages":[],"refill_funds":true}))
        .is_err()
    );
}

#[test]
fn direct_food_hunger_writes_still_need_a_real_positive_effect_during_live_use() {
    let empty = BTreeSet::new();
    assert!(positive_content_effect(Kind::Meal, true, 75., 80., &empty));
    assert!(!positive_content_effect(
        Kind::Meal,
        false,
        75.,
        80.,
        &empty
    ));
    assert!(!positive_content_effect(Kind::Meal, true, 80., 80., &empty));
    assert!(!positive_content_effect(Kind::Meal, true, 80., 75., &empty));
    assert!(!positive_content_effect(
        Kind::Hygiene,
        true,
        75.,
        80.,
        &empty
    ));
    assert!(!positive_content_effect(
        Kind::Sleep,
        true,
        75.,
        80.,
        &empty
    ));
}

#[test]
fn furniture_sale_requires_native_undo_ownership_until_actual_live_release() {
    let identity = Identity {
        object: 260,
        handle: 390,
    };
    let sold = FurnitureSnapshot {
        mode: 1,
        funds: 2542,
        ready_paused: true,
        identities: vec![identity.clone()],
        furniture: vec![FurniturePlacement {
            identity: identity.clone(),
            position: [-16, -16],
            level: 1,
            direction: 4,
        }],
        sales: vec![FurnitureSale {
            identity: identity.clone(),
            cost: 80,
            reference_count: 3,
            undo_owned: true,
            off_lot: true,
            rendered: false,
        }],
        history_handles: vec![identity.handle],
    };
    assert!(sold.sale_is_undo_owned(&identity, 80));
    assert!(!sold.lifetime_released(&identity));
    for mutate in [
        (|snapshot: &mut FurnitureSnapshot| snapshot.sales[0].cost = 79)
            as fn(&mut FurnitureSnapshot),
        |snapshot| snapshot.sales[0].reference_count = 0,
        |snapshot| snapshot.sales[0].undo_owned = false,
        |snapshot| snapshot.sales[0].off_lot = false,
        |snapshot| snapshot.sales[0].rendered = true,
        |snapshot| snapshot.sales[0].identity.handle += 1,
        |snapshot| snapshot.furniture[0].position[0] = 16,
        |snapshot| snapshot.identities.clear(),
        |snapshot| snapshot.history_handles.clear(),
        |snapshot| snapshot.sales.clear(),
    ] {
        let mut invalid = sold.clone();
        mutate(&mut invalid);
        assert!(!invalid.sale_is_undo_owned(&identity, 80));
    }
    let live = FurnitureSnapshot {
        mode: 2,
        funds: sold.funds,
        ready_paused: true,
        identities: vec![],
        furniture: vec![],
        sales: vec![],
        history_handles: vec![],
    };
    assert!(live.lifetime_released(&identity));
    assert!(!live.sale_is_undo_owned(&identity, 80));
    for mutate in [
        (|snapshot: &mut FurnitureSnapshot| {
            snapshot.identities.push(Identity {
                object: 260,
                handle: 390,
            })
        }) as fn(&mut FurnitureSnapshot),
        |snapshot| snapshot.history_handles.push(390),
        |snapshot| {
            snapshot.furniture.push(FurniturePlacement {
                identity: Identity {
                    object: 260,
                    handle: 390,
                },
                position: [-16, -16],
                level: 1,
                direction: 4,
            })
        },
        |snapshot| {
            snapshot.sales.push(FurnitureSale {
                identity: Identity {
                    object: 260,
                    handle: 390,
                },
                cost: 80,
                reference_count: 1,
                undo_owned: true,
                off_lot: true,
                rendered: false,
            })
        },
    ] {
        let mut invalid = live.clone();
        mutate(&mut invalid);
        assert!(!invalid.lifetime_released(&identity));
    }
}
