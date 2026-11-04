//! Replayable Buy furniture edits through actual Viewer input handlers.
//! Observations are read-only. Historical inferred-map imports keep that profile.
use super::*;

pub(super) fn pixel(bits: [u32; 2]) -> Result<[f32; 2]> {
    let value = bits.map(f32::from_bits);
    ensure!(
        value.iter().all(|v| v.is_finite() && *v >= 0.),
        "invalid Move viewport coordinate"
    );
    Ok(value)
}
pub(super) fn catalog(
    app: &Application,
) -> Result<&sims_render::camera_device::panels::catalog::Catalog> {
    app.viewer
        .device
        .as_ref()
        .and_then(|d| d.panel_widgets())
        .and_then(|w| w.catalog())
        .context("actual Buy catalog absent")
}
fn undo_commands(store: &sims_sim::edit_history::Store, clipboard: u32) -> Result<&[Option<u32>]> {
    let vector = store
        .vector(sims_sim::edit_history::List::Undo(clipboard))
        .map_err(anyhow::Error::msg)?;
    if vector.len == 0 {
        return Ok(&[]);
    }
    store
        .allocations
        .get(&vector.allocation)
        .and_then(|allocation| allocation.get(..vector.len))
        .context("nonempty Undo vector lost its retained allocation")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Furniture {
    identity: Identity,
    position: [i32; 2],
    level: i32,
    direction: i16,
    room: i16,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Actual {
    tick: u64,
    mode: i32,
    funds: i32,
    ready_paused: bool,
    cursors: usize,
    held: Option<Identity>,
    held_furniture: Option<Identity>,
    identities: Vec<Identity>,
    furniture: Vec<Furniture>,
    definitions: Vec<(Identity, String)>,
    sales: Vec<HistorySale>,
    history_handles: Vec<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HistorySale {
    identity: Identity,
    command: u32,
    cost: i32,
    reference_count: u32,
    undo_owned: bool,
    off_lot: bool,
    rendered: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Held {
    Any,
    Absent,
    Furniture { identity: Identity },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Expected {
    tick: u64,
    mode: i32,
    funds: i32,
    ready_paused: bool,
    held: Held,
    #[serde(default)]
    cursors: Option<usize>,
    #[serde(default)]
    furniture: Vec<Furniture>,
    #[serde(default)]
    alive: Vec<Identity>,
    #[serde(default)]
    absent: Vec<Identity>,
    /// Each required source must belong to a new lifetime absent from this set.
    #[serde(default)]
    original_identities: Vec<Identity>,
    #[serde(default)]
    new_definition_suffixes: Vec<String>,
    #[serde(default)]
    owned_sales: Vec<(Identity, i32)>,
    #[serde(default)]
    released_history: Vec<Identity>,
}
impl Expected {
    pub(super) fn matches(&self, actual: &Actual) -> bool {
        self.tick == actual.tick
            && self.mode == actual.mode
            && self.funds == actual.funds
            && (!self.ready_paused || actual.ready_paused)
            && self.cursors.is_none_or(|n| n == actual.cursors)
            && match &self.held {
                Held::Any => true,
                Held::Absent => actual.held.is_none(),
                Held::Furniture { identity } => actual.held_furniture.as_ref() == Some(identity),
            }
            && self.furniture.iter().all(|f| actual.furniture.contains(f))
            && self.alive.iter().all(|id| actual.identities.contains(id))
            && self.absent.iter().all(|id| !actual.identities.contains(id))
            && self.owned_sales.iter().all(|(id, cost)| {
                actual.sales.iter().any(|sale| {
                    sale.identity == *id
                        && sale.cost == *cost
                        && sale.reference_count > 0
                        && sale.undo_owned
                        && sale.off_lot
                        && !sale.rendered
                })
            })
            && self
                .released_history
                .iter()
                .all(|id| !actual.history_handles.contains(&id.handle))
            && self.new_definition_suffixes.iter().all(|suffix| {
                actual.definitions.iter().any(|(id, source)| {
                    !self.original_identities.contains(id)
                        && source
                            .to_ascii_lowercase()
                            .ends_with(&suffix.to_ascii_lowercase())
                })
            })
    }
}
pub(super) fn capture(app: &Application) -> Result<Actual> {
    let s = app.session()?;
    let world = &s.simulation.vm.world;
    let storage = world
        .scheduler_storage
        .as_ref()
        .context("registry absent")?;
    let identity = |handle| -> Result<Identity> {
        Identity::get(
            s,
            storage
                .object_for_handle(handle)
                .context("held lifetime absent")?,
        )
    };
    let held = s
        .host
        .presentation
        .move_tool
        .as_ref()
        .and_then(|m| m.tool.held)
        .map(identity)
        .transpose()?;
    let held_furniture = held
        .as_ref()
        .and_then(|id| world.objects[&id.object].cursor.as_ref())
        .and_then(|c| c.underlying)
        .map(identity)
        .transpose()?;
    let identities = world
        .objects
        .keys()
        .map(|&id| Identity::get(s, id))
        .collect::<Result<Vec<_>>>()?;
    let mut furniture = vec![];
    let mut definitions = vec![];
    let mut sales = vec![];
    let mut history_handles = vec![];
    if let Some(m) = &s.host.presentation.move_tool {
        history_handles.extend(
            m.history
                .references
                .records
                .values()
                .filter_map(|r| r.object),
        );
        let undo_commands = undo_commands(&m.history.store, m.tool.clipboard)?;
        for (&command, object) in &m.history.objects.0 {
            if object.kind != 3
                || object.snapshots[0].active != 1
                || object.snapshots[1].active != 0
                || object.snapshots[1].key != 0
            {
                continue;
            }
            let Some(reference) = m
                .history
                .references
                .records
                .values()
                .find(|r| r.key == object.snapshots[0].key)
            else {
                continue;
            };
            let (Some(handle), Some(cost)) = (reference.object, object.cost) else {
                continue;
            };
            let id = reference.script_id;
            sales.push(HistorySale {
                identity: Identity { object: id, handle },
                command,
                cost,
                reference_count: reference.count,
                undo_owned: undo_commands.contains(&Some(command))
                    && m.history
                        .store
                        .commands
                        .get(&command)
                        .is_some_and(|c| c.references > 0),
                off_lot: world.positions.get(&id).is_some_and(|p| p.x < 0 && p.y < 0)
                    && world
                        .objects
                        .get(&id)
                        .and_then(|o| o.native_flags)
                        .is_some_and(|f| f & 1 == 0),
                rendered: app
                    .viewer
                    .scene
                    .as_ref()
                    .is_some_and(|scene| scene.draws.iter().any(|d| d.object == Some(id))),
            });
        }
    }
    for id in &identities {
        let object = &world.objects[&id.object];
        if let Some(definition) = &object.definition {
            definitions.push((id.clone(), definition.source.clone()));
        }
        if object.person.is_none()
            && object.cursor.is_none()
            && let Some(p) = world.positions.get(&id.object)
            && let (Some(&direction), Some(&room)) = (object.data.get(&1), object.data.get(&29))
        {
            furniture.push(Furniture {
                identity: id.clone(),
                position: [p.x, p.y],
                level: p.level,
                direction,
                room,
            });
        }
    }
    Ok(Actual {
        tick: s.simulation.vm.tick,
        mode: s
            .host
            .presentation
            .transport
            .as_ref()
            .context("transport absent")?
            .controls
            .mode,
        funds: world.budget.as_ref().context("budget absent")?.balance,
        ready_paused: !app.viewer.playing
            && !app.viewer.presentation_pending()
            && !s.player_input_pending()
            && world.dialogs.modal.is_none(),
        cursors: world
            .objects
            .values()
            .filter(|o| o.cursor.is_some())
            .count(),
        held,
        held_furniture,
        identities,
        furniture,
        definitions,
        sales,
        history_handles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn id(object: i16) -> Identity {
        Identity {
            object,
            handle: object as u32 + 100,
        }
    }
    fn sample() -> (Expected, Actual) {
        let f = Furniture {
            identity: id(5),
            position: [488, 296],
            level: 1,
            direction: 2,
            room: 0,
        };
        let actual = Actual {
            tick: 7,
            mode: 1,
            funds: 585,
            ready_paused: true,
            cursors: 0,
            held: None,
            held_furniture: None,
            identities: vec![id(5), id(6)],
            furniture: vec![f.clone()],
            definitions: vec![(id(6), "GameData/Objects/ComeAndSee.iff".into())],
            sales: vec![],
            history_handles: vec![],
        };
        let expected = Expected {
            tick: 7,
            mode: 1,
            funds: 585,
            ready_paused: true,
            held: Held::Absent,
            cursors: Some(0),
            furniture: vec![f],
            alive: vec![id(5)],
            absent: vec![id(4)],
            original_identities: vec![id(5)],
            new_definition_suffixes: vec!["ComeAndSee.iff".into()],
            owned_sales: vec![],
            released_history: vec![],
        };
        (expected, actual)
    }
    #[test]
    fn furniture_predicate_rejects_wrong_cost_identity_room_orientation_and_lingering_cursor() {
        let (expected, actual) = sample();
        assert!(expected.matches(&actual));
        for mutate in [
            (|a: &mut Actual| a.funds += 80) as fn(&mut Actual),
            |a| a.furniture[0].identity.handle += 1,
            |a| a.furniture[0].position[0] += 16,
            |a| a.furniture[0].room += 1,
            |a| a.furniture[0].direction = 4,
            |a| a.cursors = 1,
            |a| a.held = Some(id(9)),
            |a| a.ready_paused = false,
            |a| a.mode = 2,
            |a| a.identities.push(id(4)),
            |a| a.definitions.clear(),
        ] {
            let mut bad = actual.clone();
            mutate(&mut bad);
            assert!(!expected.matches(&bad), "accepted {bad:?}");
        }
    }
    #[test]
    fn furniture_predicate_requires_new_controller_lifetime_and_exact_held_furniture() {
        let (mut expected, mut actual) = sample();
        expected.original_identities.push(id(6));
        assert!(!expected.matches(&actual));
        expected.new_definition_suffixes.clear();
        expected.cursors = None;
        expected.held = Held::Furniture { identity: id(5) };
        actual.held = Some(id(9));
        assert!(!expected.matches(&actual));
        actual.held_furniture = Some(id(5));
        assert!(expected.matches(&actual));
        actual.held_furniture = Some(Identity {
            object: 5,
            handle: 999,
        });
        assert!(!expected.matches(&actual));
    }
    #[test]
    fn sale_predicate_requires_undo_ownership_until_actual_live_release() {
        let (mut expected, mut actual) = sample();
        expected.owned_sales.push((id(5), 80));
        assert!(!expected.matches(&actual));
        actual.sales.push(HistorySale {
            identity: id(5),
            command: 10,
            cost: 80,
            reference_count: 3,
            undo_owned: true,
            off_lot: true,
            rendered: false,
        });
        assert!(expected.matches(&actual));
        for mutate in [
            (|s: &mut HistorySale| s.undo_owned = false) as fn(&mut HistorySale),
            |s| s.reference_count = 0,
            |s| s.off_lot = false,
            |s| s.rendered = true,
        ] {
            let mut bad = actual.clone();
            mutate(&mut bad.sales[0]);
            assert!(!expected.matches(&bad));
        }
        expected.owned_sales.clear();
        expected.released_history.push(id(5));
        actual.history_handles.push(id(5).handle);
        assert!(!expected.matches(&actual));
        actual.history_handles.clear();
        assert!(expected.matches(&actual));
    }
    #[test]
    fn undo_observation_accepts_constructor_empty_vector_but_rejects_missing_nonempty_storage()
    -> Result<()> {
        let mut store = sims_sim::edit_history::Store::default();
        store.construct_manager(7).map_err(anyhow::Error::msg)?;
        ensure!(undo_commands(&store, 7)?.is_empty());
        store
            .lists
            .get_mut(&sims_sim::edit_history::List::Undo(7))
            .unwrap()
            .len = 1;
        ensure!(undo_commands(&store, 7).is_err());
        Ok(())
    }
    #[test]
    fn furniture_commands_round_trip_without_changing_historical_command_serialization()
    -> Result<()> {
        let old: Command = serde_json::from_value(json!({"kind":"buy"}))?;
        ensure!(serde_json::to_value(old)? == json!({"kind":"buy"}));
        let (expected, _) = sample();
        let commands = vec![
            Command::Check {
                milestone: Milestone::FurnitureEdit {
                    expected: Box::new(expected),
                },
            },
            Command::BuyNavigate {
                revision: 3,
                event: sims_vm::interface::buy::input::Event::Category(0),
            },
            Command::BuyProduct {
                revision: 4,
                index: 5,
                expected_definition: 6,
                expected_guid: 7,
            },
            Command::ViewportEnter,
            Command::ViewportMotion {
                pixel: [10f32.to_bits(), 20f32.to_bits()],
            },
            Command::ViewportButton {
                down: true,
                pixel: [10f32.to_bits(), 20f32.to_bits()],
                expected: None,
            },
            Command::Escape,
            Command::SellFurniture { held: 10 },
            Command::ViewportLeave,
            Command::Poll,
        ];
        ensure!(
            serde_json::from_value::<Vec<Command>>(serde_json::to_value(&commands)?)? == commands
        );
        let plan = checkpoint::schedule(Plan::default(), Some(&commands), &[1])?;
        ensure!(plan.cold_restore_at_commands == [1]);
        let progress = vec![
            Command::Playing { value: true },
            interaction_sequences::wait(
                Milestone::Any {
                    conditions: vec![Milestone::CompletedTicks { count: 16 }, help_prompt()],
                },
                32,
                40,
            ),
            Command::Poll,
        ];
        let plan = checkpoint::schedule(Plan::default(), Some(&progress), &[2])?;
        ensure!(plan.cold_restore_at_commands == [2]);
        ensure!(pixel([f32::NAN.to_bits(), 0]).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "private household Buy tools/assets; fresh SIMS_BUY_EDIT_OUTPUT, no GUI or original guest"]
    fn actual_live_buy_furniture_cycle_preserves_identity_funds_and_cold_continuation() -> Result<()>
    {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(private_cycle)?
            .join()
            .expect("Buy edit worker panicked")
    }

    fn private_cycle() -> Result<()> {
        let repo = std::env::var_os("SIMS_BUY_EDIT_REPO")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
            .canonicalize()?;
        let out = repo.join(std::env::var("SIMS_BUY_EDIT_OUTPUT")?);
        ensure!(
            out.starts_with(repo.join("cache")),
            "output must stay in private cache"
        );
        std::fs::create_dir(&out)?;
        let source = repo
            .join(std::env::var("SIMS_BUY_EDIT_SOURCE").unwrap_or_else(|_| {
                "cache/sale-20261022-seventh/goth/sold-returned-live.json".into()
            }))
            .canonicalize()?;
        ensure!(
            source.starts_with(repo.join("cache")),
            "source must stay in private cache"
        );
        let bytes = std::fs::read(&source)?;
        let value: Value = serde_json::from_slice(&bytes)?;
        let root = repo.join("cache/managed-independent-20261022-third/immutable-assets");
        let args = Args::parse_from([
            "sims-room",
            "--original",
            value["session"]["source"]
                .as_str()
                .context("house source absent")?,
            "--paused",
            "--root",
            root.to_str().unwrap(),
            "--load",
            source.to_str().unwrap(),
            "--save",
            out.join("unused-player-save.json").to_str().unwrap(),
        ]);
        let ready = Arc::new(Ready::load(&args)?);
        let epoch = Instant::now();
        let mut app = if value.get("schema") == Some(&json!(1)) {
            Application::restore_checkpoint(
                &args,
                ready.restore_index.as_ref().unwrap(),
                &ready.icons,
                epoch,
                &bytes,
            )?
        } else {
            // An ordinary private managed viewer checkpoint gets a real fresh
            // frontend attachment, exactly as exploration::run does. No game
            // namespace, tool owner or application-local state is fabricated.
            let (mut viewer, _) = Viewer::new(args);
            viewer.install(ready)?;
            viewer.last = Some(epoch);
            if let Some(clock) = viewer
                .session
                .as_ref()
                .and_then(|s| s.host.presentation.clock.as_ref())
            {
                viewer.clock_origin = Some((epoch, clock.now));
            }
            Application {
                viewer,
                epoch,
                millis: 0,
                paths: Default::default(),
                progress_burst_ticks: 0,
            }
        };
        app.drain(&limits())?;
        write_json(
            &out.join("profile.json"),
            &json!({"source":source,
            "source_sha256":engine::digest(&bytes),"root":root,
            "routing_policy":app.session()?.simulation.native_lot.as_ref().map(|n|n.routing_policy),
            "map_owner":app.session()?.simulation.vm.world.map_size,
            "strict_native_parity_ticks":0,"original_engine_executed":false,"player_save_accessed":false}),
        )?;
        std::fs::write(out.join("initial.json"), app.checkpoint()?)?;
        let result = if std::env::var_os("SIMS_BUY_EDIT_CONTINUE_POST_LIVE").is_some() {
            continue_post_live(&mut app, &out)
        } else if std::env::var_os("SIMS_BUY_EDIT_CONTINUE_AFTER_MOVE").is_some() {
            continue_after_move(&mut app, &out)
        } else {
            cycle(&mut app, &out)
        };
        if let Err(error) = &result {
            write_json(
                &out.join("first-boundary-failure.json"),
                &json!({"error":format!("{error:#}"),
                "observation":app.observe().ok(),"read_only_edit_state":capture(&app).ok(),
                "strict_native_parity_ticks":0}),
            )?;
            match app.checkpoint() {
                Ok(bytes) => std::fs::write(out.join("first-boundary-failure-state.json"), bytes)?,
                Err(error) => write_json(
                    &out.join("failure-checkpoint-error.json"),
                    &format!("{error:#}"),
                )?,
            }
        }
        result
    }
    fn limits() -> Limits {
        Limits {
            commands: 64,
            completed_ticks: 128,
            service_polls: 10000,
            wall_seconds: 180.,
        }
    }
    fn expected(app: &Application, mode: i32, funds: i32, held: Held) -> Result<Expected> {
        Ok(Expected {
            tick: app.session()?.simulation.vm.tick,
            mode,
            funds,
            ready_paused: true,
            held,
            cursors: None,
            furniture: vec![],
            alive: vec![],
            absent: vec![],
            original_identities: vec![],
            new_definition_suffixes: vec![],
            owned_sales: vec![],
            released_history: vec![],
        })
    }
    fn check(expected: Expected) -> Command {
        Command::Check {
            milestone: Milestone::FurnitureEdit {
                expected: Box::new(expected),
            },
        }
    }
    fn stage(
        app: &mut Application,
        out: &Path,
        name: &str,
        commands: Vec<Command>,
        restore_after: &[usize],
    ) -> Result<()> {
        let path = out.join(name);
        std::fs::create_dir(&path)?;
        std::fs::write(path.join("initial.json"), app.checkpoint()?)?;
        write_json(&path.join("commands.json"), &commands)?;
        let plan = checkpoint::schedule(
            Plan {
                limits: limits(),
                compare_ticks: true,
                cold_restore_at: Some(0),
                ..Plan::default()
            },
            Some(&commands),
            restore_after,
        )?;
        let mut budget = ArtifactBudget {
            screenshots: 0,
            failure_checkpoints: 1,
        };
        let report = run_case(
            app,
            plan,
            Some(&commands),
            &path,
            "run",
            &mut budget,
            0,
            restore_after,
            &[],
        )?;
        preserve(app, &path, "run", &report, &mut budget)?;
        std::fs::write(path.join("final.json"), app.checkpoint()?)?;
        write_json(&path.join("edit-state.json"), &capture(app)?)?;
        let row = json!({"stage":name,"outcome":report.outcome,"failure":report.failure,
            "commands":report.steps.len(),"completed_ticks":report.completed_ticks(),
            "tick_comparisons":report.tick_comparisons,"cold_restores":report.cold_restores,
            "seconds":report.seconds});
        use std::io::Write;
        let mut journal = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(out.join("stages.jsonl"))?;
        writeln!(journal, "{row}")?;
        ensure!(
            report.passed(),
            "{name} failed; preserved exact report and checkpoint"
        );
        Ok(())
    }
    fn navigate(
        app: &Application,
        event: sims_vm::interface::buy::input::Event,
    ) -> Result<Command> {
        Ok(Command::BuyNavigate {
            revision: catalog(app)?.revision(),
            event,
        })
    }
    fn at(app: &Application, position: [i32; 2]) -> Result<[u32; 2]> {
        let s = app.session()?;
        let mut pixel = OriginalHouse::native_camera(s)?
            .screen(OriginalHouse::ground_point(s, position, 1)?)
            .to_array();
        let offset = s
            .host
            .presentation
            .move_tool
            .as_ref()
            .context("Move absent")?
            .tool
            .tool
            .position;
        pixel[0] += offset[1] as f32;
        pixel[1] += offset[0] as f32;
        let scene = app.viewer.scene.as_ref().context("scene absent")?;
        ensure!(
            pixel[0] >= 1.
                && pixel[1] >= 1.
                && pixel[0] < scene.width as f32 - 1.
                && pixel[1] < scene.height as f32 - 1.,
            "target not on actual viewport: {pixel:?}"
        );
        let actual = move_ground(s, pixel)?;
        ensure!(
            actual.map(|v| v.div_euclid(16)) == position.map(|v| v.div_euclid(16)),
            "real viewport projection changed target tile: {position:?} -> {actual:?}"
        );
        Ok(pixel.map(f32::to_bits))
    }
    fn button(app: &Application, down: bool, pixel: [u32; 2]) -> Result<Command> {
        let expected = app
            .viewer
            .scene
            .as_ref()
            .context("scene absent")?
            .pick(super::pixel(pixel)?)
            .map(|id| Identity::get(app.session()?, id))
            .transpose()?;
        Ok(Command::ViewportButton {
            down,
            pixel,
            expected,
        })
    }
    fn target_pixel(app: &Application, target: &Identity) -> Result<[u32; 2]> {
        target.check(app.session()?)?;
        let scene = app.viewer.scene.as_ref().context("scene absent")?;
        for draw in &scene.draws {
            if draw.object != Some(target.object) {
                continue;
            }
            let vertices =
                &scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize];
            let points = vertices
                .iter()
                .map(|v| {
                    [
                        (v.position[0] + 1.) * scene.width as f32 / 2.,
                        (1. - v.position[1]) * scene.height as f32 / 2.,
                    ]
                })
                .collect::<Vec<_>>();
            let min = std::array::from_fn::<_, 2, _>(|axis| {
                points
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::INFINITY, f32::min)
                    .ceil()
                    .max(0.) as u32
            });
            let max = std::array::from_fn::<_, 2, _>(|axis| {
                points
                    .iter()
                    .map(|p| p[axis])
                    .fold(f32::NEG_INFINITY, f32::max)
                    .floor()
                    .max(0.) as u32
            });
            for y in min[1]..=max[1].min(scene.height - 1) {
                for x in min[0]..=max[0].min(scene.width - 1) {
                    let p = [x as f32 + 0.5, y as f32 + 0.5];
                    if scene.pick(p) == Some(target.object) {
                        return Ok(p.map(f32::to_bits));
                    }
                }
            }
        }
        anyhow::bail!("actual furniture has no pickable visible fragment: {target:?}")
    }
    fn placed(
        app: &Application,
        identity: &Identity,
        position: [i32; 2],
        direction: i16,
    ) -> Result<Furniture> {
        Ok(Furniture {
            identity: identity.clone(),
            position,
            level: 1,
            direction,
            room: app
                .session()?
                .simulation
                .architecture
                .as_ref()
                .context("architecture absent")?
                .room_at(1, sims_sim::Point::new(position[1], position[0])),
        })
    }
    fn picture(app: &Application, out: &Path, name: &str) -> Result<()> {
        let before = app.observe()?;
        let scene = app.viewer.device.as_ref().context("camera absent")?.scene();
        let rgba = sims_render::software::render(scene);
        image::save_buffer(
            out.join(format!("{name}.png")),
            &rgba,
            scene.width,
            scene.height,
            image::ColorType::Rgba8,
        )?;
        ensure!(
            before == app.observe()?,
            "software milestone rendering mutated retained state"
        );
        write_json(
            &out.join(format!("{name}-render.json")),
            &json!({"state_neutral":true,
            "observation":before,"rgba_sha256":engine::digest(&rgba),"native_parity_ticks":0}),
        )
    }
    fn click_pickup(
        app: &mut Application,
        out: &Path,
        prefix: &str,
        target: &Identity,
    ) -> Result<()> {
        let p = target_pixel(app, target)?;
        let down = button(app, true, p)?;
        stage(app, out, &format!("{prefix}-down"), vec![down], &[])?;
        let up = button(app, false, p)?;
        stage(app, out, &format!("{prefix}-up"), vec![up], &[])?;
        ensure!(
            capture(app)?.held_furniture.as_ref() == Some(target),
            "actual click did not retain requested furniture"
        );
        Ok(())
    }
    fn observed_startup_notices(app: &mut Application, out: &Path) -> Result<()> {
        // Exact five records published by the first preserved Goth branch.
        // No new title, changed body, repeated lifetime or player choice is
        // answered by this fixture setup.
        let allowed = [
            (
                "HelpSystemMagic.iff",
                "Magic Town",
                "8adc30cd1de26fbbbf34b0e9a2c619161c31903c1c2e2fe5c5b4bf750b798588",
            ),
            (
                "HDHelpSystem.iff",
                " ",
                "74458af6ac718e72b3a2ab6598af7e3bea98e2d04da12a23a4c8794850231d2f",
            ),
            (
                "NeighborhoodHelpSystem.iff",
                "Old Town Shuttle",
                "018722c9a7b117b343d7815d4ce26980a01f01a1f544a691e0b1da621c0365cb",
            ),
            (
                "HelpSystemStudioTown.iff",
                "Studio Town",
                "2fbd7eaa7fe4848c76c02c21954e6f8c8df1f623401370a70d3daa0f2f1ce63a",
            ),
            (
                "VacationHelpSystem.iff",
                "Feeling Down?",
                "5ef4d7566d4762c9e0f57717f8a20f650cdbbced96b82ebcf93899c863b62d4d",
            ),
        ];
        let mut used = std::collections::BTreeSet::new();
        for sequence in 0..=allowed.len() {
            let dialogs = &app.session()?.simulation.vm.world.dialogs;
            let Some(id) = dialogs.modal else {
                return Ok(());
            };
            let record = dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("modal record absent")?;
            let hash = engine::digest(record.body.as_bytes());
            let allowed_index = allowed
                .iter()
                .position(|(source, title, body)| {
                    record.source == format!("ExpansionShared/ExpansionShared.far::{source}")
                        && record.title == *title
                        && hash == *body
                })
                .context("unobserved Buy startup modal; no automatic answer")?;
            ensure!(
                used.insert(allowed_index)
                    && sequence < allowed.len()
                    && record.buttons.len() == 1
                    && record.buttons[0].result == 2,
                "repeated or changed Buy startup notice"
            );
            stage(
                app,
                out,
                &format!("01-startup-notice-{sequence}"),
                vec![Command::Dialog {
                    id,
                    result: 2,
                    body_sha256: hash,
                }],
                &[],
            )?;
        }
        anyhow::bail!("observed startup notice limit exceeded")
    }
    #[derive(Clone, Copy)]
    struct Locations {
        purchase: [i32; 2],
        destination: [i32; 2],
        cancelled: [i32; 2],
    }
    fn locations(app: &Application) -> Result<Locations> {
        // Physical free tile centers from each retained fixture. Actual Move
        // projection, placement and room predicates must still prove delivery.
        Ok(match app.session()?.house.source.as_str() {
            "UserData/Houses/House05.iff" => Locations {
                purchase: [552, 456],
                destination: [456, 392],
                cancelled: [568, 456],
            },
            "UserData/Houses/House07.iff" => Locations {
                purchase: [552, 280],
                destination: [456, 408],
                cancelled: [568, 280],
            },
            "UserData2/Houses/House09.iff" => Locations {
                purchase: [552, 296],
                destination: [424, 312],
                cancelled: [568, 296],
            },
            source => anyhow::bail!("no observed furniture targets for {source}"),
        })
    }
    fn cycle(app: &mut Application, out: &Path) -> Result<()> {
        let locations = locations(app)?;
        let initial = capture(app)?;
        ensure!(
            initial.mode == 2 && initial.ready_paused && initial.cursors == 0,
            "requires safe paused Live source"
        );
        ensure!(
            app.session()?.host.presentation.tool_selection.is_some(),
            "source lacks genuine Buy tool owner"
        );
        let funds = initial.funds;
        let buy = expected(app, 1, funds, Held::Absent)?;
        stage(
            app,
            out,
            "01-enter-buy",
            vec![
                Command::Buy,
                Command::ExpandPanel,
                check(buy),
                Command::Poll,
            ],
            &[3],
        )?;
        observed_startup_notices(app, out)?;
        let command = navigate(app, sims_vm::interface::buy::input::Event::Category(0))?;
        stage(app, out, "02-chairs-category", vec![command], &[])?;
        let c = catalog(app)?;
        let index = c
            .view()
            .context("catalog view absent")?
            .products
            .iter()
            .copied()
            .find(|&i| {
                c.product(i).is_some_and(|p| {
                    p.source.source.ends_with("DiningChairs.iff") && p.fields.price == 80
                })
            })
            .context("actual $80 ordinary DiningChairs product absent")?;
        let product = c.product(index).unwrap().clone();
        write_json(&out.join("selected-product.json"), &product)?;
        let product_command = Command::BuyProduct {
            revision: c.revision(),
            index,
            expected_definition: product.definition,
            expected_guid: product.source.guid,
        };
        stage(
            app,
            out,
            "03-product-preview",
            vec![product_command, Command::ViewportEnter],
            &[],
        )?;
        let chair = capture(app)?
            .held_furniture
            .context("actual product preview did not create underlying furniture")?;
        ensure!(
            app.session()?.simulation.vm.world.objects[&chair.object]
                .definition
                .as_ref()
                == Some(&product.source),
            "preview underlying definition differs from selected product"
        );
        // The preserved second branch used488,296, which is stove119's actual
        // physical position. Its rejection is retained; these actual empty
        // furniture rectangles avoid mistaking a blocked placement for a buy.
        let position = locations.purchase;
        let p = at(app, position)?;
        stage(
            app,
            out,
            "04-place-motion",
            vec![Command::ViewportMotion { pixel: p }],
            &[],
        )?;
        let direction = app.session()?.simulation.vm.world.objects[&chair.object].data[&1];
        let down = button(app, true, p)?;
        stage(app, out, "05-purchase-down", vec![down], &[])?;
        let up = button(app, false, p)?;
        stage(app, out, "06-purchase-up", vec![up], &[])?;
        let mut bought = expected(app, 1, funds - 80, Held::Absent)?;
        bought.cursors = Some(0);
        bought
            .furniture
            .push(placed(app, &chair, position, direction)?);
        // Escape cancels an actual repeated preview when present and otherwise
        // leaves the paid object intact. A separate held-existing cancellation
        // below is mandatory before declaring the whole cycle complete.
        stage(
            app,
            out,
            "07-escape-unpaid-repeat",
            vec![Command::Escape, check(bought), Command::Poll],
            &[2],
        )?;
        picture(app, out, "purchased-safe-buy")?;
        let before_live = capture(app)?.identities;
        let mut live = expected(app, 2, funds - 80, Held::Absent)?;
        live.cursors = Some(0);
        live.furniture
            .push(placed(app, &chair, position, direction)?);
        live.original_identities = before_live;
        live.new_definition_suffixes = vec!["ComeAndSee.iff".into()];
        stage(
            app,
            out,
            "08-return-live",
            vec![Command::Live, check(live), Command::Poll],
            &[2],
        )?;
        picture(app, out, "purchased-post-live")?;
        let buy = expected(app, 1, funds - 80, Held::Absent)?;
        stage(
            app,
            out,
            "09-reenter-buy",
            vec![Command::Buy, check(buy), Command::Poll],
            &[2],
        )?;
        click_pickup(app, out, "10-pickup-to-move", &chair)?;
        let destination = locations.destination;
        let p = at(app, destination)?;
        stage(
            app,
            out,
            "11-move-existing",
            vec![Command::ViewportMotion { pixel: p }],
            &[],
        )?;
        let facing = app.session()?.simulation.vm.world.objects[&chair.object].data[&1];
        let down = button(app, true, p)?;
        stage(app, out, "12-rotation-down", vec![down], &[])?;
        let desired = (facing + 2) & 7;
        let rotation = app
            .session()?
            .host
            .presentation
            .presenter
            .as_ref()
            .context("presenter absent")?
            .view
            .display_rotation as i16;
        let delta = match (desired + rotation * 2) & 7 {
            0 => [-14., 14.],
            2 => [14., 14.],
            4 => [14., -14.],
            6 => [-14., -14.],
            _ => anyhow::bail!("ordinary chair has non-cardinal drag facing"),
        };
        let p = super::pixel(p)?;
        let rotated = [p[0] + delta[0], p[1] + delta[1]].map(f32::to_bits);
        stage(
            app,
            out,
            "13-drag-rotate",
            vec![Command::ViewportMotion { pixel: rotated }],
            &[],
        )?;
        ensure!(
            app.session()?.simulation.vm.world.objects[&chair.object].data[&1] == desired,
            "actual drag did not rotate chair"
        );
        let up = button(app, false, rotated)?;
        stage(app, out, "14-release-rotation", vec![up], &[])?;
        let mut moved = expected(app, 1, funds - 80, Held::Absent)?;
        moved.cursors = Some(0);
        moved
            .furniture
            .push(placed(app, &chair, destination, desired)?);
        stage(
            app,
            out,
            "15-moved-safe-buy",
            vec![Command::Escape, check(moved), Command::Poll],
            &[2],
        )?;
        let credit = finish_cancel_and_sale(
            app,
            out,
            chair.clone(),
            destination,
            locations.cancelled,
            desired,
            funds - 80,
        )?;
        write_json(
            &out.join("complete.json"),
            &json!({"item":product,"furniture":chair,
            "purchase_position":position,"move_position":destination,"cancelled_position":locations.cancelled,
            "direction_before":direction,"direction_after":desired,"funds_before":funds,
            "funds_after":capture(app)?.funds,"sale_credit":credit,
            "ticks":app.session()?.simulation.vm.tick-initial.tick,"strict_native_parity_ticks":0,
            "scope":"internal retained-state robustness; staged exact replay, full state/RNG and cold continuation; no original gameplay comparison",
            "routing_policy":app.session()?.options.routing,
            "retained_map_owner":app.session()?.simulation.vm.world.map_size,
            "retained_registry_order_present":app.session()?.simulation.vm.world.registry_order.is_some(),
            "fixture_provenance":"exact retained input path/hash in profile.json; no routing policy or native-owner promotion"}),
        )?;
        Ok(())
    }
    fn continue_after_move(app: &mut Application, out: &Path) -> Result<()> {
        let actual = capture(app)?;
        ensure!(
            actual.mode == 1 && actual.ready_paused && actual.cursors == 0,
            "continuation requires actual safe paused Buy"
        );
        let matches: Vec<_> = actual
            .furniture
            .iter()
            .filter(|f| {
                f.position == [456, 392]
                    && f.level == 1
                    && f.direction == 4
                    && app
                        .session()
                        .ok()
                        .and_then(|s| s.simulation.vm.world.objects.get(&f.identity.object))
                        .and_then(|o| o.definition.as_ref())
                        .is_some_and(|d| d.guid == 650099497)
            })
            .collect();
        ensure!(
            matches.len() == 1,
            "continuation must retain exactly the purchased/moved original chair"
        );
        let chair = matches[0].identity.clone();
        let mut expected = expected(app, 1, actual.funds, Held::Absent)?;
        expected.cursors = Some(0);
        expected.furniture.push(placed(app, &chair, [456, 392], 4)?);
        stage(
            app,
            out,
            "15-moved-safe-buy",
            vec![Command::Escape, check(expected), Command::Poll],
            &[2],
        )?;
        let credit = finish_cancel_and_sale(
            app,
            out,
            chair.clone(),
            [456, 392],
            [568, 456],
            4,
            actual.funds,
        )?;
        write_json(
            &out.join("continuation-complete.json"),
            &json!({
            "furniture":chair,"funds_at_continuation_start":actual.funds,"sale_credit":credit,
            "funds_after":capture(app)?.funds,"ticks":app.session()?.simulation.vm.tick-actual.tick,
            "strict_native_parity_ticks":0,
            "scope":"linked preserved post-move checkpoint; does not independently replay the purchase prefix"}),
        )?;
        Ok(())
    }
    fn finish_cancel_and_sale(
        app: &mut Application,
        out: &Path,
        chair: Identity,
        destination: [i32; 2],
        displaced: [i32; 2],
        desired: i16,
        balance: i32,
    ) -> Result<i32> {
        click_pickup(app, out, "16-pickup-to-cancel", &chair)?;
        let p = at(app, displaced)?;
        stage(
            app,
            out,
            "17-uncommitted-move",
            vec![Command::ViewportMotion { pixel: p }],
            &[],
        )?;
        ensure!(
            capture(app)?.held_furniture.as_ref() == Some(&chair),
            "cancellation has no held existing furniture"
        );
        let mut cancelled = expected(app, 1, balance, Held::Absent)?;
        cancelled.cursors = Some(0);
        cancelled
            .furniture
            .push(placed(app, &chair, destination, desired)?);
        stage(
            app,
            out,
            "18-escape-existing",
            vec![Command::Escape, check(cancelled), Command::Poll],
            &[2],
        )?;
        click_pickup(app, out, "19-pickup-to-sell", &chair)?;
        let selection = app
            .viewer
            .device
            .as_ref()
            .context("camera absent")?
            .furniture_selection()
            .context("actual selected furniture controls absent")?;
        ensure!(
            selection.sell_allowed && selection.price > 0 && selection.price <= 80,
            "unexpected actual sale eligibility/value"
        );
        let credit = selection.price;
        let held = app
            .session()?
            .host
            .presentation
            .move_tool
            .as_ref()
            .unwrap()
            .tool
            .held
            .context("sale cursor absent")?;
        let mut sold = expected(app, 1, balance + credit, Held::Absent)?;
        sold.cursors = Some(0);
        // Native48bce9 kind3 leaves the actual furniture off-lot and owned by
        // its Undo snapshot.4933dc's normal Live close releases that owner.
        sold.owned_sales.push((chair.clone(), credit));
        stage(
            app,
            out,
            "20-sell",
            vec![Command::SellFurniture { held }, check(sold), Command::Poll],
            &[2],
        )?;
        let mut live = expected(app, 2, balance + credit, Held::Absent)?;
        live.cursors = Some(0);
        live.absent.push(chair.clone());
        live.released_history.push(chair.clone());
        stage(
            app,
            out,
            "21-final-live",
            vec![Command::Live, check(live), Command::Poll],
            &[2],
        )?;
        picture(app, out, "sold-post-live")?;
        post_live_progress(app, out)?;
        ensure!(
            !capture(app)?.identities.contains(&chair),
            "sold lifetime returned after gameplay"
        );
        Ok(credit)
    }
    fn help_prompt() -> Milestone {
        Milestone::Modal {
            source: "GameData_ALL_Recursive/Objects/Objects.far::HelpSystem.iff".into(),
            title: "Help System".into(),
            body_sha256: "52f35949be4524be16205e81c259ff3e338cc6e87ff98c641387cfa7191e766f".into(),
        }
    }
    fn continue_post_live(app: &mut Application, out: &Path) -> Result<()> {
        let initial = capture(app)?;
        ensure!(
            initial.mode == 2 && initial.ready_paused && initial.cursors == 0,
            "post-Live continuation requires safe paused Live, no pending cursor"
        );
        post_live_progress(app, out)?;
        write_json(
            &out.join("continuation-complete.json"),
            &json!({
            "ticks":app.session()?.simulation.vm.tick-initial.tick,"strict_native_parity_ticks":0,
            "scope":"linked exact post-Live checkpoint; purchase/edit prefix not replayed here"}),
        )?;
        Ok(())
    }
    fn post_live_progress(app: &mut Application, out: &Path) -> Result<()> {
        let start = app.session()?.simulation.vm.tick;
        let mut answered = false;
        for part in 0..=1 {
            let elapsed = app.session()?.simulation.vm.tick.saturating_sub(start);
            if elapsed >= 16 {
                break;
            }
            let progress = Milestone::CompletedTicks {
                count: 16 - elapsed,
            };
            let milestone = if answered {
                progress
            } else {
                Milestone::Any {
                    conditions: vec![progress, help_prompt()],
                }
            };
            stage(
                app,
                out,
                &format!("22-progress-{part}"),
                vec![
                    Command::Playing { value: true },
                    interaction_sequences::wait(milestone, 32, 40),
                    Command::Poll,
                ],
                &[2],
            )?;
            if app.session()?.simulation.vm.tick.saturating_sub(start) >= 16 {
                break;
            }
            ensure!(
                !answered && help_prompt().reached(app, start)?,
                "unexpected interrupted gameplay, no automatic response"
            );
            let dialogs = &app.session()?.simulation.vm.world.dialogs;
            let id = dialogs.modal.context("observed Help modal disappeared")?;
            let record = dialogs
                .records
                .iter()
                .find(|r| r.id == id)
                .context("Help record absent")?;
            ensure!(
                record.buttons.len() == 2
                    && record.buttons[0].label == "Yes"
                    && record.buttons[0].result == 2
                    && record.buttons[1].label == "No"
                    && record.buttons[1].result == 3,
                "changed Help choice identity"
            );
            write_json(
                &out.join("observed-help-prompt.json"),
                &json!({
                    "record":record,"clock":app.session()?.host.presentation.clock,
                    "choice":{"label":"No","result":3},"strict_native_parity_ticks":0
                }),
            )?;
            stage(
                app,
                out,
                "22-help-no",
                vec![Command::Dialog {
                    id,
                    result: 3,
                    body_sha256: engine::digest(record.body.as_bytes()),
                }],
                &[],
            )?;
            answered = true;
        }
        ensure!(
            app.session()?.simulation.vm.tick.saturating_sub(start) >= 16,
            "fewer than16 completed post-Live ticks"
        );
        stage(
            app,
            out,
            "23-gameplay-paused",
            vec![
                Command::Playing { value: false },
                Command::Check {
                    milestone: Milestone::Mode { value: 2 },
                },
                Command::Poll,
            ],
            &[2],
        )?;
        Ok(())
    }
}
