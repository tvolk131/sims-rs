//! Deliberate real CarPool acknowledgement across generated event/restore
//! histories. No native widget/golden or whole-engine parity qualification.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Plan {
    seed: u64,
    key: Option<Key>,
    under_hud: bool,
    size: [u32; 2],
    split: bool,
    deferred_ack: bool,
}
fn plan(seed: u64) -> Plan {
    let mut rng = engine::Random::new(seed ^ 0x434152504f4f4c);
    Plan {
        seed,
        key: match seed % 4 {
            0 => None,
            1 => Some(Key::Enter),
            2 => Some(Key::Escape),
            _ => Some(Key::Space),
        },
        under_hud: seed & 4 != 0,
        size: SIZES[(rng.next_u64() % 3) as usize],
        split: rng.next_u64() & 1 == 0,
        deferred_ack: rng.next_u64() & 1 == 0,
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum MarkerKind {
    ColdRestore,
    Frame,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Marker {
    after_batches: usize,
    name: String,
    kind: MarkerKind,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Script {
    schema: u32,
    seed: u64,
    initial: Observation,
    steps: Vec<Step>,
    markers: Vec<Marker>,
}

pub(super) fn expected_answer(app: &Application, messages: &[Message]) -> Result<Option<u64>> {
    let world = &app.session()?.simulation.vm.world;
    let Some(id) = world.dialogs.modal else {
        return Ok(None);
    };
    let record = world
        .dialogs
        .records
        .iter()
        .find(|r| r.id == id)
        .context("active modal record absent")?;
    Ok(messages
        .iter()
        .any(|m| match m {
            Message::Dialog(answer, result) => {
                *answer == id
                    && record.status == 1
                    && record.buttons.iter().any(|b| b.result == *result)
            }
            Message::DialogKey(key) => record.key_result(*key).is_some(),
            _ => false,
        })
        .then_some(id))
}
pub(super) fn continuation(app: &Application, id: u64) -> Result<Value> {
    let value = app.capture(false)?;
    let state = &value["session"]["simulation"]["state"];
    let vm = &state["vm"];
    let record = vm["world"]["dialogs"]["records"]
        .as_array()
        .context("saved dialog records absent")?
        .iter()
        .find(|r| r["id"] == id)
        .cloned();
    Ok(json!({"observation":app.observe()?,"record":record,
        "dialogs":vm["world"]["dialogs"],"dialog_owners":vm["world"]["ui_owner"]["dialog_owners"],
        "clock":value["session"]["view"]["clock"],"native_scheduler":state["native_scheduler"],
        "object_threads":vm["object_threads"],"all_threads_sha256":engine::digest(&serde_json::to_vec(&vm["threads"])?),
        "effects":vm["effects"]}))
}
fn protocol(app: &Application) -> Result<()> {
    let d = &app.session()?.simulation.vm.world.dialogs;
    let r = d
        .records
        .iter()
        .find(|r| r.id == 149)
        .context("retained CarPool record absent")?;
    ensure!(
        d.modal == Some(149)
            && r.status == 1
            && r.caller == 8
            && r.stack_object == 21
            && r.source == "GameData_ALL_Recursive/Objects/Objects.far::CarPortal.iff"
            && r.origin.bhav == 4125
            && r.origin.instruction == 11
            && [13, 27, 32].into_iter().all(|k| r.key_result(k) == Some(2))
            && r.buttons.len() == 1
            && r.buttons[0].result == 2,
        "CarPool protocol changed"
    );
    Ok(())
}
struct Transition<'a> {
    run: Run<'a>,
    markers: Vec<Marker>,
    snapshots: Vec<Value>,
}
impl Transition<'_> {
    fn script(&self) -> Script {
        Script {
            schema: 1,
            seed: self.run.seed,
            initial: self.run.initial.clone(),
            steps: self.run.steps.clone(),
            markers: self.markers.clone(),
        }
    }
    fn journal(&self) -> Result<()> {
        write_json(
            &self.run.out.join("transition-replay.json"),
            &serde_json::to_value(self.script())?,
        )
    }
    fn marker(&mut self, marker: Marker) -> Result<()> {
        ensure!(
            marker.after_batches == self.run.steps.len(),
            "marker batch boundary mismatch"
        );
        self.markers.push(marker.clone());
        self.journal()?;
        let before = self.run.app.observe()?;
        match marker.kind {
            MarkerKind::ColdRestore => {
                ensure!(
                    self.run.pending.is_empty() && self.run.intended.is_empty(),
                    "restore at unflushed boundary"
                );
                let bytes = self.run.app.checkpoint()?;
                std::fs::write(self.run.out.join(format!("{}.json", marker.name)), &bytes)?;
                let cold = self.run.app.restore(&bytes)?;
                ensure!(
                    cold.observe()? == before && cold.observe()? == self.run.reference.observe()?,
                    "modal_cold_restore_divergence"
                );
                self.run.app = cold;
                self.run.ui.reset();
            }
            MarkerKind::Frame => {
                self.run.ui.batch_save(
                    &self.run.app.viewer,
                    &self.run.out.join(format!("{}.png", marker.name)),
                )?;
                ensure!(
                    self.run.app.observe()? == before,
                    "passive_modal_transition_frame_mutated_vm"
                );
            }
        }
        let summary = continuation(&self.run.app, 149)?;
        ensure!(
            summary == continuation(&self.run.reference, 149)?,
            "modal_marker_continuation_divergence"
        );
        self.snapshots.push(json!({"marker":marker,"summary":summary,"passive_state_neutral":before==self.run.app.observe()?}));
        write_json(
            &self.run.out.join("markers.json"),
            &json!({"markers":self.snapshots}),
        )?;
        Ok(())
    }
    fn mark(&mut self, name: &str, kind: MarkerKind) -> Result<()> {
        self.run.flush()?;
        self.marker(Marker {
            after_batches: self.run.steps.len(),
            name: name.into(),
            kind,
        })
    }
    fn replay(&mut self, script: Script) -> Result<()> {
        ensure!(
            script.schema == 1
                && script.seed == self.run.seed
                && script.initial == self.run.initial,
            "transition replay initial mismatch"
        );
        let mut markers = script.markers.into_iter().peekable();
        for index in 0..=script.steps.len() {
            while markers.peek().is_some_and(|m| m.after_batches == index) {
                self.marker(markers.next().unwrap())?;
            }
            if let Some(step) = script.steps.get(index) {
                self.run.step(step.clone())?;
                self.journal()?;
            }
        }
        ensure!(markers.next().is_none(), "replay marker outside trace");
        Ok(())
    }
}

fn normal_hud_point(run: &Run<'_>, probe: &mut Input) -> Result<[f32; 2]> {
    let before = run.app.observe()?;
    let mut app = run.app.fork()?;
    app.dispatch(Message::Dialog(149, 2))?;
    app.drain(&Limits::default())?;
    ensure!(
        app.session()?.simulation.vm.world.dialogs.modal.is_none(),
        "ordinary ack probe has another modal"
    );
    probe.reset();
    let p = control(probe, &app, &Action::Options)?;
    ensure!(
        before == run.app.observe()?,
        "geometry probe changed live continuation"
    );
    Ok(p)
}
fn sequence(t: &mut Transition<'_>, p: &Plan, probe: &mut Input) -> Result<()> {
    protocol(&t.run.app)?;
    t.mark("modal-open", MarkerKind::Frame)?;
    t.mark("before-ack-cold", MarkerKind::ColdRestore)?;
    // First hold is interrupted across multiple real update batches. The
    // modal must remain open; a fresh acknowledgement is deliberate later.
    let point = t.run.locate(&Action::Dialog(149, 2))?;
    t.run.events(
        vec![Event::moved(point), Event::Down],
        false,
        "hold original modal button",
    )?;
    let first = vec![Event::Focus(false), Event::Resize(p.size)];
    if p.split {
        t.run
            .events(vec![first[0].clone()], false, "delayed focus batch")?;
        t.run.events(
            vec![first[1].clone(), Event::Focus(true), Event::Up],
            false,
            "delayed resize and stale modal release",
        )?;
    } else {
        t.run.events(
            vec![
                first[0].clone(),
                first[1].clone(),
                Event::Focus(true),
                Event::Up,
            ],
            false,
            "batched interrupted modal release",
        )?;
    }
    t.run.flush()?;
    protocol(&t.run.app)?;
    t.mark("modal-resized", MarkerKind::Frame)?;
    if let Some(key) = p.key {
        // Hold either the old dialog button or the actual underlying Options
        // location while modal input is active. The underlying point comes
        // from a separate ordinary-ack fork, not synthetic tree coordinates.
        let point = if p.under_hud {
            normal_hud_point(&t.run, probe)?
        } else {
            t.run.locate(&Action::Dialog(149, 2))?
        };
        t.run.events(
            vec![Event::moved(point), Event::Down],
            false,
            "hold through deliberate key acknowledgement",
        )?;
        t.run.events(
            vec![Event::Key { key, down: true }],
            !p.deferred_ack,
            "deliberate protocol key answer",
        )?;
        if p.deferred_ack {
            t.run.events(
                vec![Event::Focus(false), Event::Focus(true)],
                false,
                "focus before delayed answer dispatch",
            )?;
            t.run.flush()?;
        }
        ensure!(
            t.run
                .app
                .session()?
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none(),
            "CarPool answer did not return to HUD"
        );
        t.run.events(
            vec![Event::Up, Event::Key { key, down: false }],
            true,
            "old held releases after modal closure",
        )?;
    } else {
        t.run.click(
            Action::Dialog(149, 2),
            "fresh deliberate pointer acknowledgement",
        )?;
        ensure!(
            t.run
                .app
                .session()?
                .simulation
                .vm
                .world
                .dialogs
                .modal
                .is_none(),
            "CarPool pointer answer did not return to HUD"
        );
        t.run.events(
            vec![Event::Up],
            true,
            "duplicate old release after pointer acknowledgement",
        )?;
    }
    t.mark("after-ack-cold", MarkerKind::ColdRestore)?;
    // Cold restore begins a new widget lifetime with no tracked cursor. A
    // guessed bottom-center point is real lot-edge input in this legacy Goth
    // profile, whose lot shader fills the window; never call it neutral HUD.
    t.run.events(
        vec![Event::Up],
        true,
        "post-restore stale release without cursor",
    )?;
    t.mark("acknowledged-restored", MarkerKind::Frame)?;
    let point = control(t.run.ui, &t.run.app, &Action::Options)?;
    t.run.step(Step {
        events: vec![Event::moved(point), Event::Down, Event::Up],
        actions: vec![Action::Options],
        flush: true,
        scroll_preserved: false,
        reason: "fresh actual Options widget after acknowledgement".into(),
    })?;
    ensure!(t.run.app.viewer.options_open, "fresh Options did not open");
    t.journal()?;
    Ok(())
}
#[test]
fn acknowledgement_plans_and_typed_markers_are_replayable() -> Result<()> {
    let mut keys = std::collections::BTreeSet::new();
    let mut sizes = std::collections::BTreeSet::new();
    for seed in 0..8 {
        let p = plan(seed);
        ensure!(p == plan(seed));
        let bytes = serde_json::to_vec(&p)?;
        ensure!(serde_json::from_slice::<Plan>(&bytes)? == p);
        keys.insert(p.key.map(Key::code));
        sizes.insert(p.size);
        for key in [Key::Enter, Key::Escape, Key::Space] {
            let step = Step {
                events: vec![
                    Event::Key { key, down: true },
                    Event::Key { key, down: false },
                ],
                actions: vec![],
                flush: false,
                scroll_preserved: false,
                reason: "protocol".into(),
            };
            ensure!(labels(&expected(&step)) == [format!("DialogKey({})", key.code())]);
            ensure!(serde_json::from_slice::<Step>(&serde_json::to_vec(&step)?)? == step);
        }
    }
    ensure!(keys.len() == 4 && sizes.len() == 3);
    Ok(())
}
#[test]
#[ignore = "private genuine CarPool continuation; generated acknowledgement/restore histories; offscreen Iced"]
fn generated_acknowledgement_and_normal_hud_transitions() -> Result<()> {
    run_worker(campaign)
}
fn campaign() -> Result<()> {
    let out = output()?;
    let base = modal_lifetimes::load_car_pool(&out)?;
    let replay = std::env::var("SIMS_MODAL_TRANSITION_REPLAY")
        .ok()
        .map(|p| -> Result<Script> { Ok(serde_json::from_slice(&std::fs::read(p)?)?) })
        .transpose()?;
    let seeds = if let Some(s) = &replay {
        vec![s.seed]
    } else if let Ok(seed) = std::env::var("SIMS_MODAL_TRANSITION_SEED") {
        vec![seed.parse()?]
    } else {
        (0..8).collect::<Vec<_>>()
    };
    let mut ui = Input::new()?;
    let mut probe = Input::new()?;
    let mut rows = vec![];
    for seed in seeds {
        let run = Run::new(
            base.fork()?,
            &mut ui,
            out.join(format!("seed-{seed}")),
            seed,
            "modal-transition",
        )?;
        let mut t = Transition {
            run,
            markers: vec![],
            snapshots: vec![],
        };
        write_json(
            &t.run.out.join("plan.json"),
            &serde_json::to_value(plan(seed))?,
        )?;
        let result = (|| -> Result<()> {
            if let Some(script) = &replay {
                t.replay(script.clone())?;
            } else {
                sequence(&mut t, &plan(seed), &mut probe)?;
            }
            t.run.finish()?;
            t.journal()?;
            ensure!(
                t.run.acknowledgements.len() == 2,
                "answer protocol executed more/less than once"
            );
            ensure!(
                t.run.acknowledgements[0]["actual"]["record"]["status"] == 2
                    && t.run.acknowledgements[0]["actual"]["record"]["origin"]["bhav"] == 4125
                    && t.run.acknowledgements[0]["actual"]["record"]["origin"]["instruction"] == 11,
                "ordinary answer did not retain native result2 at original CarPortal continuation"
            );
            write_json(
                &t.run.out.join("transition-result.json"),
                &json!({"passed":true,"initial":t.run.initial,"final":t.run.app.observe()?,"batches":t.run.steps.len(),"markers":t.markers,"acknowledgements":t.run.acknowledgements,"complete_warm_cold_equal":true,"scope":"ordinary Rust handler/continuation agreement, no original widget/tick-parity qualification"}),
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            let evidence = t.run.preserve_error(&error).err().map(|e| e.to_string());
            t.journal()?;
            write_json(
                &out.join("first-failure.json"),
                &json!({"seed":seed,"error":format!("{error:#}"),"evidence_error":evidence}),
            )?;
            return Err(error);
        }
        rows.push(json!({"seed":seed,"passed":true,"batches":t.run.steps.len()}));
    }
    write_json(
        &out.join("report.json"),
        &json!({"seeds":rows,"strict_original_parity_qualified":false,"production_changes":false}),
    )?;
    Ok(())
}
