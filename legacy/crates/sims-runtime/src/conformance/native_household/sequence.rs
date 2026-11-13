//! Bounded exploration beyond the qualified prefix. No new native admission.
//! Only the already retained Clock caller can drive this diagnostic. A missing
//! application/input continuation is an explicit stop, never a direct tick.
use super::*;
use crate::original_session::clock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineInput {
    pub after_tick: u64,
    pub source: String,
    pub routine: u16,
    pub content_sha256: String,
    pub payload_sha256: String,
}

pub struct GlobalInputs<'a> {
    pub after_tick: u64,
    pub animation_cache: Option<&'a Value>,
    pub tuning_cache: Option<&'a Value>,
    pub score_strings: Option<&'a Value>,
}

fn add_global_constants(
    prepared: &Prepared,
    program: &mut sims_vm::Program,
    tuning: &Value,
) -> Result<Value> {
    add_global_resource(prepared, program, tuning, false)
}

fn add_default_score_strings(
    prepared: &Prepared,
    program: &mut sims_vm::Program,
    banks: &Value,
) -> Result<Value> {
    add_global_resource(prepared, program, banks, true)
}

fn add_global_resource(
    prepared: &Prepared,
    program: &mut sims_vm::Program,
    input: &Value,
    scores: bool,
) -> Result<Value> {
    let store = Store::open(&prepared.source_root)?;
    let config = raw_json(
        &store,
        prepared
            .request
            .program_files
            .get(&prepared.request.program_config)
            .context("qualified Program configuration absent")?,
    )?;
    let key = config["resident_global_capture"]
        .as_str()
        .context("retained Global owner absent")?;
    let global = raw_json(
        &store,
        prepared
            .request
            .program_files
            .get(key)
            .context("retained Global source closure absent")?,
    )?;
    let locator = prepared
        .initial
        .view
        .resource_aliases
        .as_ref()
        .and_then(|aliases| aliases.get(&program.globals))
        .context("qualified Global alias absent")?;
    if scores {
        crate::original_session::completed_assets::import_available_default_score_strings(
            &global,
            input,
            &prepared.index,
            locator,
            program,
        )
    } else {
        crate::original_session::completed_assets::import_available_global_constants(
            &global,
            input,
            &prepared.index,
            locator,
            program,
        )
    }
}

fn validate_inputs(initial: u64, target: u64, inputs: &[RoutineInput]) -> Result<()> {
    ensure!(
        inputs.len() <= 64
            && inputs.iter().all(|i| i.after_tick > initial
                && i.after_tick < target
                && (4096..=8191).contains(&i.routine)
                && !i.source.is_empty()
                && [&i.content_sha256, &i.payload_sha256]
                    .iter()
                    .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))),
        "routine inputs exceed the diagnostic window or identity contract"
    );
    let unique: std::collections::BTreeSet<_> = inputs
        .iter()
        .map(|i| (i.after_tick, &i.source, i.routine))
        .collect();
    ensure!(unique.len() == inputs.len(), "duplicate routine input");
    Ok(())
}

fn add_routine(prepared: &Prepared, session: &mut Session, input: &RoutineInput) -> Result<Value> {
    ensure!(
        (4096..=8191).contains(&input.routine),
        "diagnostic input is not a private BHAV"
    );
    let locator = prepared
        .initial
        .view
        .resource_aliases
        .as_ref()
        .and_then(|a| a.get(&input.source))
        .context("routine input has no qualified provider alias")?;
    let index = prepared
        .initial
        .managed
        .as_ref()
        .context("owned source absent")?
        .owned
        .mount(&prepared.index)?;
    let bytes = index.resources[index.find(locator)?].read()?;
    ensure!(
        sims_fixtures::digest(&bytes) == input.content_sha256,
        "routine input whole source version differs"
    );
    let iff = sims_formats::iff::Iff::parse(bytes)?;
    ensure!(
        iff.chunks
            .iter()
            .filter(|c| c.tag == *b"BHAV" && c.id == input.routine)
            .count()
            == 1,
        "routine input BHAV absent or ambiguous"
    );
    let chunk = iff
        .get(b"BHAV", input.routine)
        .context("routine input BHAV absent")?;
    let payload = iff.data(chunk);
    ensure!(
        sims_fixtures::digest(payload) == input.payload_sha256 && payload.len() >= 12,
        "routine input payload identity differs"
    );
    let body = sims_formats::behavior::Behavior::parse(payload)?;
    let revision = u32::from_le_bytes(payload[8..12].try_into()?);
    let module = session
        .program
        .modules
        .get_mut(&input.source)
        .context("routine provider absent from linked Program")?;
    ensure!(
        module.content_hash == input.content_sha256,
        "routine input changes retained provider version"
    );
    ensure!(
        !module.routines.contains_key(&input.routine)
            && !module.routine_versions.contains_key(&input.routine),
        "routine input would replace retained code"
    );
    module.routines.insert(
        input.routine,
        sims_vm::Routine {
            name: chunk.label.clone(),
            body,
        },
    );
    module.routine_versions.insert(input.routine, revision);
    Ok(
        json!({"input":input,"locator":locator,"revision":revision,"program_fingerprint":session.program.fingerprint(),"native_cache_order_qualified":false}),
    )
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stop {
    TargetReached,
    ClockInactive,
    ClockReturned { updates: u32 },
    InputPoll,
    HouseholdService { progress: String },
    WorkBound,
    RuntimeFailure { error: String },
    ReferenceDivergence,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub initial_tick: u64,
    pub target_tick: u64,
    pub completed_ticks: u64,
    pub known_reference_ticks_matched: u64,
    /// Probe output is diagnostic, including any matching qualified prefix.
    pub native_parity_ticks: u64,
    pub stop: Stop,
    pub difference: Option<Difference>,
    pub boundaries: Vec<Value>,
    pub final_state: BlobRef,
    pub managed_capture: bool,
    pub trace: BlobRef,
    pub qualification: BlobRef,
    pub code_inputs: Vec<Value>,
    pub timings_ms: BTreeMap<String, f64>,
}

fn completed(before: u64, after: u64, progress: Progress) -> Result<bool> {
    match progress {
        Progress::Tick => {
            ensure!(
                after == before.checked_add(1).context("tick overflow")?,
                "completed Clock child did not advance exactly one tick"
            );
            Ok(true)
        }
        Progress::PartialPass => {
            ensure!(after == before, "partial Clock child advanced the lot tick");
            Ok(false)
        }
        _ => {
            ensure!(
                after == before,
                "suspended Clock child advanced the lot tick"
            );
            Ok(false)
        }
    }
}

impl Prepared {
    /// Describe an immutable asset dependency without installing or executing it.
    pub fn routine_input(
        &self,
        after_tick: u64,
        source: &str,
        routine: u16,
    ) -> Result<RoutineInput> {
        ensure!((4096..=8191).contains(&routine), "expected a private BHAV");
        let locator = self
            .initial
            .view
            .resource_aliases
            .as_ref()
            .and_then(|a| a.get(source))
            .context("qualified provider alias absent")?;
        let index = self
            .initial
            .managed
            .as_ref()
            .context("owned source absent")?
            .owned
            .mount(&self.index)?;
        let bytes = index.resources[index.find(locator)?].read()?;
        let hash = sims_fixtures::digest(&bytes);
        ensure!(
            self.program
                .modules
                .get(source)
                .is_some_and(|m| m.content_hash == hash),
            "routine source differs from retained module version"
        );
        let iff = sims_formats::iff::Iff::parse(bytes)?;
        let chunk = iff.get(b"BHAV", routine).context("requested BHAV absent")?;
        Ok(RoutineInput {
            after_tick,
            source: source.into(),
            routine,
            content_sha256: hash,
            payload_sha256: sims_fixtures::digest(iff.data(chunk)),
        })
    }

    /// Restore once, observe at most32 completed ticks, retain selected binary
    /// checkpoints and compact boundary records. This cannot earn native credit.
    /// Output must be new and separate from immutable assets.
    pub fn probe_sequence(&self, ticks: u64, output: &Path) -> Result<Report> {
        self.probe_sequence_with_inputs(ticks, &[], output)
    }

    /// Authored execution-closure additions for diagnosis only. They are bound
    /// to whole source/payload hashes and applied after a compared boundary;
    /// they do not establish the native lookup/cache/OS-service ordering.
    pub fn probe_sequence_with_inputs(
        &self,
        ticks: u64,
        inputs: &[RoutineInput],
        output: &Path,
    ) -> Result<Report> {
        self.probe_sequence_inner(ticks, inputs, None, output)
    }

    /// Supply observed effective Global name vectors as an explicit diagnostic
    /// configuration after the compared prefix. This does not qualify their
    /// native cache lifetime, private-table absence or a new completed tick.
    pub fn probe_sequence_with_animation_cache(
        &self,
        ticks: u64,
        inputs: &[RoutineInput],
        after_tick: u64,
        cache: &Value,
        output: &Path,
    ) -> Result<Report> {
        self.probe_sequence_with_global_inputs(
            ticks,
            inputs,
            GlobalInputs {
                after_tick,
                animation_cache: Some(cache),
                tuning_cache: None,
                score_strings: None,
            },
            output,
        )
    }

    pub fn probe_sequence_with_global_inputs(
        &self,
        ticks: u64,
        inputs: &[RoutineInput],
        globals: GlobalInputs<'_>,
        output: &Path,
    ) -> Result<Report> {
        ensure!(
            globals.animation_cache.is_some()
                || globals.tuning_cache.is_some()
                || globals.score_strings.is_some(),
            "empty Global input"
        );
        let profile = globals
            .animation_cache
            .map(crate::original_session::completed_animation_cache::GlobalTables::read)
            .transpose()?;
        ensure!(
            profile
                .as_ref()
                .is_none_or(|p| p.tick == self.initial.simulation.state.vm.tick),
            "animation input does not come from the baseline epoch"
        );
        self.probe_sequence_inner(
            ticks,
            inputs,
            Some((
                globals.after_tick,
                profile.as_ref(),
                globals.tuning_cache,
                globals.score_strings,
            )),
            output,
        )
    }

    fn probe_sequence_inner(
        &self,
        ticks: u64,
        inputs: &[RoutineInput],
        global_inputs: Option<(
            u64,
            Option<&crate::original_session::completed_animation_cache::GlobalTables>,
            Option<&Value>,
            Option<&Value>,
        )>,
        output: &Path,
    ) -> Result<Report> {
        ensure!(
            (1..=32).contains(&ticks),
            "sequence probe bound must be 1..32 ticks"
        );
        let initial_tick = self.initial.simulation.state.vm.tick;
        let target_tick = initial_tick
            .checked_add(ticks)
            .context("target tick overflow")?;
        validate_inputs(initial_tick, target_tick, inputs)?;
        ensure!(
            global_inputs.is_none_or(|(tick, _, _, _)| tick > initial_tick && tick < target_tick),
            "Global input would change the baseline or exceed the window"
        );
        let parent = output
            .parent()
            .context("probe output parent absent")?
            .canonicalize()?;
        ensure!(
            !parent.starts_with(&self.source_root) && !parent.starts_with(&self.immutable_root),
            "probe output overlaps fixture or immutable source"
        );
        let destination = parent.join(output.file_name().context("probe directory name absent")?);
        // Reserve the directory atomically; Store also supports empty roots.
        fs::create_dir(&destination)?;
        let store = Store::create(destination)?;
        let mut timings = BTreeMap::new();
        let mut session = measured(&mut timings, "baseline_restore", || {
            self.restore(self.initial.clone())
        })?;
        let mut saved = measured(&mut timings, "capture", || session.save_managed())?;
        let mut difference = compare_saved(&self.initial, &saved, "/baseline")?;
        let mut stop = if difference.is_some() {
            Stop::ReferenceDivergence
        } else {
            Stop::WorkBound
        };
        let mut boundaries = vec![];
        let mut trace = vec![];
        let mut tick_trace = vec![];
        let mut matched = 0u64;
        let mut observed = 0u64;
        let mut work = 0usize;
        let mut code_inputs = vec![];
        let mut applied = std::collections::BTreeSet::new();
        let mut animation_applied = false;
        while difference.is_none() && session.simulation.vm.tick < target_tick {
            if let Some((after_tick, profile, tuning, scores)) = global_inputs {
                if !animation_applied && after_tick == session.simulation.vm.tick {
                    ensure!(
                        matched > 0,
                        "Global input precedes the accepted comparison prefix"
                    );
                    if let Some(tuning) = tuning {
                        let mut receipt = add_global_constants(self, &mut session.program, tuning)?;
                        receipt["applied_after_tick"] = json!(after_tick);
                        code_inputs.push(receipt);
                    }
                    if let Some(profile) = profile {
                        let mut receipt = profile.install_diagnostic(&mut session.program)?;
                        receipt["applied_after_tick"] = json!(after_tick);
                        code_inputs.push(receipt);
                    }
                    if let Some(scores) = scores {
                        let mut receipt =
                            add_default_score_strings(self, &mut session.program, scores)?;
                        receipt["applied_after_tick"] = json!(after_tick);
                        code_inputs.push(receipt);
                    }
                    animation_applied = true;
                }
            }
            for (position, input) in inputs.iter().enumerate() {
                if input.after_tick == session.simulation.vm.tick && applied.insert(position) {
                    code_inputs.push(add_routine(self, &mut session, input)?);
                }
            }
            if !session.clock_pending() {
                stop = Stop::ClockInactive;
                break;
            }
            if work == 4096 * ticks as usize || trace.len() + tick_trace.len() >= 1_000_000 {
                stop = Stop::WorkBound;
                break;
            }
            work += 1;
            let before = session.simulation.vm.tick;
            let progress = measured(&mut timings, "simulation", || {
                session.advance_clock_with_trace(32, &mut tick_trace)
            });
            let progress = match progress {
                Ok(p) => p,
                Err(error) => {
                    stop = Stop::RuntimeFailure {
                        error: format!("{error:#}"),
                    };
                    break;
                }
            };
            match progress {
                clock::Progress::Pending => continue,
                clock::Progress::PollInput => {
                    stop = Stop::InputPoll;
                    break;
                }
                clock::Progress::Complete { updates } => {
                    stop = Stop::ClockReturned { updates };
                    break;
                }
                clock::Progress::Household(progress) => {
                    match completed(before, session.simulation.vm.tick, progress) {
                        Err(error) => {
                            stop = Stop::RuntimeFailure {
                                error: format!("{error:#}"),
                            };
                            break;
                        }
                        Ok(false) if progress == Progress::PartialPass => continue,
                        Ok(false) => {
                            stop = Stop::HouseholdService {
                                progress: format!("{progress:?}"),
                            };
                            break;
                        }
                        Ok(true) => (),
                    }
                }
            }
            observed += 1;
            saved = measured(&mut timings, "capture", || session.save_managed())?;
            let tick = saved.simulation.state.vm.tick;
            let reference = self.request.frames.iter().position(|f| f.tick == tick);
            if let Some(i) = reference {
                difference = measured(&mut timings, "state_compare", || {
                    compare_saved(&self.expected[i], &saved, &format!("/boundaries/{tick}"))
                })?;
                if difference.is_none() {
                    if let Err(error) =
                        completed_replay::trace::compare(&self.native[i], &tick_trace, tick - 1)
                    {
                        difference = Some(Difference {
                            path: format!("/boundaries/{tick}/native_trace"),
                            expected: Some(self.native[i].clone()),
                            actual: Some(json!({"error":error.to_string(),"trace":tick_trace})),
                        });
                    } else if tick_trace != self.traces[i] {
                        difference = Some(Difference {
                            path: format!("/boundaries/{tick}/qualified_rust_trace"),
                            expected: Some(serde_json::to_value(&self.traces[i])?),
                            actual: Some(serde_json::to_value(&tick_trace)?),
                        });
                    }
                }
                if difference.is_none() {
                    matched += 1;
                }
            }
            let checkpoint = if [1, 2, 8, 32, ticks].contains(&observed) || difference.is_some() {
                Some(store.put_value(&saved)?)
            } else {
                None
            };
            boundaries.push(json!({"tick":tick,"mapped_state_sha256":sims_fixtures::digest(&postcard::to_allocvec(&saved)?),"snapshot":checkpoint,"trace_events":tick_trace.len(),"qualified_reference_available":reference.is_some(),"qualified_reference_matched":reference.map(|_|difference.is_none()),"clock_pending":session.clock_pending(),"rng":{"global":saved.simulation.state.vm.rng_state,"lot":saved.simulation.state.vm.lot_rng_state,"CRT":saved.simulation.state.vm.crt_rng_state}}));
            trace.append(&mut tick_trace);
            if difference.is_some() {
                stop = Stop::ReferenceDivergence;
                break;
            }
            if tick == target_tick {
                stop = Stop::TargetReached;
            }
        }
        // A failed/returned parent may have changed since the last completed cut.
        let final_capture = measured(&mut timings, "capture", || session.save_managed());
        let managed_capture = final_capture.is_ok();
        saved = final_capture.unwrap_or_else(|_| session.save());
        trace.append(&mut tick_trace);
        let report = Report {
            schema: 1,
            initial_tick,
            target_tick,
            completed_ticks: observed,
            known_reference_ticks_matched: matched,
            native_parity_ticks: 0,
            stop,
            difference,
            boundaries,
            final_state: store.put_value(&saved)?,
            managed_capture,
            trace: store.put_value(&trace)?,
            qualification: self.request.qualification.clone(),
            code_inputs,
            timings_ms: timings,
        };
        fs::write(
            store.root().join("sequence-probe.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        fs::write(store.root().join("trace.json"), serde_json::to_vec(&trace)?)?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_a_real_completed_child_advances_one_tick() {
        assert!(completed(4, 5, Progress::Tick).unwrap());
        assert!(!completed(4, 4, Progress::PartialPass).unwrap());
        assert!(!completed(4, 4, Progress::Dialog).unwrap());
        assert!(completed(4, 4, Progress::Tick).is_err());
        assert!(completed(4, 6, Progress::Tick).is_err());
        assert!(completed(4, 5, Progress::PartialPass).is_err());
        assert!(completed(4, 5, Progress::Dialog).is_err());
    }

    #[test]
    fn code_inputs_cannot_change_the_baseline_or_replace_each_other() {
        let input = RoutineInput {
            after_tick: 5,
            source: "provider".into(),
            routine: 4098,
            content_sha256: "a".repeat(64),
            payload_sha256: "b".repeat(64),
        };
        assert!(validate_inputs(4, 8, std::slice::from_ref(&input)).is_ok());
        assert!(validate_inputs(5, 8, std::slice::from_ref(&input)).is_err());
        assert!(validate_inputs(4, 5, std::slice::from_ref(&input)).is_err());
        assert!(validate_inputs(4, 8, &[input.clone(), input.clone()]).is_err());
        let mut bad = input.clone();
        bad.routine = 44;
        assert!(validate_inputs(4, 8, &[bad]).is_err());
        let mut bad = input;
        bad.payload_sha256 = "unknown".into();
        assert!(validate_inputs(4, 8, &[bad]).is_err());
    }

    #[test]
    #[ignore = "private qualified fixture, immutable Global catalog and same-cut override owner"]
    fn global_constant_catalog_preserves_residents_and_rejects_changed_owners() -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let store = Store::open(root.join("fixtures/private/player-action-regressions-20261025"))?;
        let case = store.load_case("native-goth-qualified-replay-20261029-v1")?;
        let prepared = Prepared::load(
            &store,
            &case,
            &root.join("cache/managed-independent-20261022-third/immutable-assets"),
            "UserData/Houses/House05.iff",
        )?
        .unwrap();
        let tuning: Value = serde_json::from_slice(&fs::read(
            root.join("fixtures/private/goth-tuning-cache-20261029-first/capture-first.json"),
        )?)?;
        let mut program = prepared.program.clone();
        let receipt = add_global_constants(&prepared, &mut program, &tuning)?;
        assert_eq!(receipt["tables_available"], 18);
        assert_eq!(receipt["resident_payloads_preserved"], 5);
        assert_eq!(program.modules[&program.globals].constants[&257][4], 1);
        let hash = program.fingerprint();
        for corrupt in 0..3 {
            let mut changed = tuning.clone();
            match corrupt {
                0 => changed["tick"] = json!(88449),
                1 => changed["otf"] = json!([{}]),
                _ => changed["source_pin"]["observation_sha256"] = json!("wrong"),
            }
            assert!(add_global_constants(&prepared, &mut program, &changed).is_err());
            assert_eq!(
                program.fingerprint(),
                hash,
                "failed input changed code configuration"
            );
        }
        program
            .modules
            .get_mut(&program.globals)
            .unwrap()
            .constants
            .insert(257, vec![999]);
        let hash = program.fingerprint();
        assert!(add_global_constants(&prepared, &mut program, &tuning).is_err());
        assert_eq!(
            program.fingerprint(),
            hash,
            "conflicting captured table was overwritten"
        );
        Ok(())
    }

    #[test]
    #[ignore = "private qualified baseline and immutable assets"]
    fn ordinary_scores_preserve_effective_animation_views_and_reject_conflicts() -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let store = Store::open(root.join("fixtures/private/player-action-regressions-20261025"))?;
        let prepared = Prepared::load(
            &store,
            &store.load_case("native-goth-qualified-replay-20261029-v1")?,
            &root.join("cache/managed-independent-20261022-third/immutable-assets"),
            "UserData/Houses/House05.iff",
        )?
        .unwrap();
        let cache: Value = serde_json::from_slice(&fs::read(
            root.join("fixtures/private/goth-animation-cache-20261029-first/capture-first.json"),
        )?)?;
        let banks: Value = serde_json::from_slice(&fs::read(
            root.join("fixtures/private/goth-tuning-cache-20261029-first/string-banks-first.json"),
        )?)?;
        let mut program = prepared.program.clone();
        crate::original_session::completed_animation_cache::GlobalTables::read(&cache)?
            .install_diagnostic(&mut program)?;
        let previous = program.modules[&program.globals].strings.clone();
        let receipt = add_default_score_strings(&prepared, &mut program, &banks)?;
        assert_eq!(receipt["tables_added"], 2);
        assert_eq!(receipt["raw_string_catalog_tables"], 20);
        for (id, values) in previous {
            assert_eq!(program.modules[&program.globals].strings[&id], values);
        }
        for id in [501, 503] {
            assert_eq!(program.modules[&program.globals].strings[&id].len(), 10);
        }
        let fingerprint = program.fingerprint();
        for corrupt in 0..4 {
            let mut bad = banks.clone();
            match corrupt {
                0 => bad["tick"] = json!(88449),
                1 => bad["file_language_byte"] = json!(2),
                2 => bad["provider"] = json!(0),
                _ => bad["source_pin"]["physical"]["compressed_sha256"] = json!("wrong"),
            }
            assert!(add_default_score_strings(&prepared, &mut program, &bad).is_err());
            assert_eq!(program.fingerprint(), fingerprint);
        }
        program
            .modules
            .get_mut(&program.globals)
            .unwrap()
            .strings
            .insert(503, vec!["changed".into()]);
        let fingerprint = program.fingerprint();
        assert!(add_default_score_strings(&prepared, &mut program, &banks).is_err());
        assert_eq!(program.fingerprint(), fingerprint);
        Ok(())
    }

    #[test]
    #[ignore = "private qualified baseline and immutable assets"]
    fn host_poll_observation_preserves_complete_subject_state_and_code_inputs_are_explicit()
    -> Result<()> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let store = Store::open(root.join("fixtures/private/player-action-regressions-20261025"))?;
        let case = store.load_case("native-goth-qualified-replay-20261029-v1")?;
        let p = Prepared::load(
            &store,
            &case,
            &root.join("cache/managed-independent-20261022-third/immutable-assets"),
            "UserData/Houses/House05.iff",
        )?
        .unwrap();
        let mut a = p.restore(p.initial.clone())?;
        let mut b = p.restore(p.initial.clone())?;
        let mut trace = Vec::new();
        let mut outcomes = Vec::new();
        for (session, observed) in [(&mut a, false), (&mut b, true)] {
            let mut outcome = String::new();
            for _ in 0..128 {
                let result = if observed {
                    session.advance_clock_with_trace(32, &mut trace)
                } else {
                    session.advance_clock(32)
                };
                outcome = format!("{result:?}");
                if result.is_err()
                    || !session.clock_pending()
                    || session.simulation.vm.tick >= 88450
                {
                    break;
                }
            }
            outcomes.push(outcome);
        }
        assert_eq!(outcomes[0], outcomes[1]);
        assert!(compare_saved(&a.save_managed()?, &b.save_managed()?, "/observed")?.is_none());
        assert_eq!(
            trace[8..11]
                .iter()
                .map(|t| (t.thread, t.opcode, t.effect.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (16, 44, "await animation service response"),
                (18, 44, "await animation service response"),
                (21, 44, "await animation service response")
            ]
        );
        // Consume the immutable new original-data case rather than treating a
        // matching authored trace or this Rust-produced snapshot as an oracle.
        let native_case = store.load_case("native-goth-sequence-frontier-20261029-v1")?;
        ensure!(
            native_case.source.blob.sha256
                == "ea46a36f9348c59f3417e478c8cdf92a82e2f00539aec7422e906fcf4039d68e",
            "native frontier source changed"
        );
        let inventory: sims_fixtures::archive::ArchiveInventory =
            store.get_value(&native_case.source.blob)?;
        inventory.validate_artifact(&native_case.source)?;
        let file = inventory
            .files
            .iter()
            .find(|f| {
                f.relative_path
                    .ends_with("goth-process-sequence-20261029-fourth/native-bytecode.json")
            })
            .context("original partial trace absent")?;
        let native: Vec<Value> = serde_json::from_slice(&store.read_bytes(&file.blob, 1 << 20)?)?;
        ensure!(
            native.len() == 40 && trace.len() >= 40,
            "partial native frontier length changed"
        );
        completed_replay::trace::compare(&json!(&native[..8]), &trace[..8], 88448)?;
        completed_replay::trace::compare(&json!(&native[8..39]), &trace[8..39], 88449)?;
        let last = &trace[39];
        assert_eq!(
            json!({"thread":last.thread,"depth":last.depth,"source":last.location.source,"tree":last.location.bhav,"node":last.location.instruction,"opcode":last.opcode,"operands":last.operands}),
            json!({"thread":native[39]["object"],"depth":native[39]["depth"],"source":native[39]["source"],"tree":native[39]["tree"],"node":native[39]["node"],"opcode":native[39]["opcode"],"operands":native[39]["operands"]})
        );
        ensure!(
            native[39].get("call_result_raw").is_none(),
            "native call unexpectedly has a completed return"
        );
        let fingerprint = b.program.fingerprint();
        let mut input = p.routine_input(88449, "native-provider:09e04fa0", 4098)?;
        input.payload_sha256 = "0".repeat(64);
        assert!(add_routine(&p, &mut b, &input).is_err());
        assert_eq!(b.program.fingerprint(), fingerprint);
        let retained = p.routine_input(88449, "native-provider:09e04fa0", 4096)?;
        assert!(add_routine(&p, &mut b, &retained).is_err());
        assert_eq!(b.program.fingerprint(), fingerprint);
        Ok(())
    }
}
