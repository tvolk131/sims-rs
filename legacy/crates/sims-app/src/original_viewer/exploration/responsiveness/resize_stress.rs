//! Continuous native-owned HUD lifetimes, rather than another isolated focus
//! or modal matrix. Timings are diagnostics; no original UI/parity/FPS claim.
use super::*;
use modal_lifetimes::pointer;

#[path = "resize_stress/mode_costs.rs"]
mod mode_costs;
#[path = "resize_stress/dpi.rs"]
mod dpi;

const SOURCE: &str = "cache/definition-link-normalization-20261024-first/furniture-after-first/23-gameplay-paused/final.json";
const SOURCE_SHA256: &str = "cac43efc2f929139222ccca70a46d41b6e22581b8c214c635bd43933952ac262";
const INSTALL: &str = "cache/native-parity-guest-20261023-second/assets-immutable";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Event {
    Move([u32; 2]),
    Down,
    Up,
    Focus(bool),
    Resize([u32; 2]),
    Wheel(i16),
}
impl Event {
    fn moved(point: [f32; 2]) -> Self {
        Self::Move(point.map(f32::to_bits))
    }
    fn iced(&self) -> iced::Event {
        match *self {
            Self::Move(p) => iced::Event::Mouse(iced::mouse::Event::CursorMoved {
                position: iced::Point::new(f32::from_bits(p[0]), f32::from_bits(p[1])),
            }),
            Self::Down => pointer(true),
            Self::Up => pointer(false),
            Self::Focus(value) => iced::Event::Window(if value {
                window::Event::Focused
            } else {
                window::Event::Unfocused
            }),
            Self::Resize(s) => {
                iced::Event::Window(window::Event::Resized(Size::new(s[0] as f32, s[1] as f32)))
            }
            Self::Wheel(y) => iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: y as f32 },
            }),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Control {
    Options,
    ObjectCancel,
    Buy,
    Live,
    Resident(i16),
}
impl Control {
    fn message(self) -> Message {
        match self {
            Self::Options => Message::Options,
            Self::ObjectCancel => Message::ObjectCancel,
            Self::Buy => Message::Buy,
            Self::Live => Message::Live,
            Self::Resident(id) => Message::Resident(id),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Intent {
    None,
    Control(Control),
    Object { object: i16, pixel: [u32; 2] },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Step {
    label: String,
    events: Vec<Event>,
    intent: Intent,
    flush: bool,
}
fn burst(final_size: [u32; 2]) -> Vec<Vec<Event>> {
    vec![
        vec![
            Event::Resize([800, 600]),
            Event::Resize([1000, 740]),
            Event::Resize([1320, 900]),
            Event::Focus(false),
        ],
        vec![
            Event::Resize([1173, 837]),
            Event::Focus(true),
            Event::Resize([821, 613]),
            Event::Resize(final_size),
            Event::Up,
        ],
    ]
}
#[test]
fn continuous_resize_plan_materializes_every_barrier_and_terminal_size() -> Result<()> {
    for size in [[800, 600], [1000, 740], [1320, 900]] {
        let plan = burst(size);
        ensure!(serde_json::from_slice::<Vec<Vec<Event>>>(&serde_json::to_vec(&plan)?)? == plan);
        let flattened: Vec<_> = plan.into_iter().flatten().collect();
        ensure!(
            flattened
                .iter()
                .filter(|e| matches!(e, Event::Resize(_)))
                .count()
                == 6
        );
        ensure!(flattened.iter().filter(|e| matches!(e, Event::Up)).count() == 1);
        ensure!(
            flattened.iter().rev().find_map(|e| match e {
                Event::Resize(s) => Some(*s),
                _ => None,
            }) == Some(size)
        );
        ensure!(flattened.last() == Some(&Event::Up));
        ensure!(!flattened.contains(&Event::Down));
    }
    Ok(())
}

/// Same complete persisted owners as Application::capture, streamed to avoid a
/// giant intermediate JSON tree for every event batch. No fields are omitted.
fn full(app: &Application) -> Result<Observation> {
    #[derive(Serialize)]
    struct Full {
        schema: u32,
        session: Saved,
        dialog_client: sims_render::dialog::client::Saved,
        camera_device: Option<sims_render::camera_device::Saved>,
        application: Local,
        observer: &'static str,
    }
    let s = app.session()?;
    let value = Full {
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
        observer: "complete-native-owned-hud-resize-v1",
    };
    Ok(Observation {
        tick: s.simulation.vm.tick,
        rng: [
            Some(s.simulation.vm.rng_state),
            s.simulation.vm.lot_rng_state,
            s.simulation.vm.crt_rng_state,
        ],
        state_sha256: engine::digest_serialized(&value)?,
    })
}
fn mode(app: &Application) -> Result<i32> {
    Ok(app
        .session()?
        .host
        .presentation
        .transport
        .as_ref()
        .context("native transport absent")?
        .controls
        .mode)
}
fn safe(app: &Application) -> Result<()> {
    let s = app.session()?;
    ensure!(
        !app.viewer.playing
            && !app.viewer.presentation_pending()
            && !s.player_input_pending()
            && s.simulation.vm.world.dialogs.modal.is_none(),
        "fixture is not a paused native player boundary"
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
        "actual native owner/Strict map absent"
    );
    Ok(())
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
fn load_native(out: &Path) -> Result<Application> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let source = repo.join(SOURCE);
    let bytes = std::fs::read(&source)?;
    ensure!(
        engine::digest(&bytes) == SOURCE_SHA256,
        "private native-owned fixture changed"
    );
    let root = repo.join(INSTALL);
    let index = Index::scan(&root)?;
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
    let app = Application::restore_checkpoint(&args, &index, &icons, Instant::now(), &bytes)?;
    safe(&app)?;
    ensure!(mode(&app)? == 2, "source is not Live mode");
    write_json(
        &out.join("input.json"),
        &json!({"source":source,"sha256":engine::digest(&bytes),
        "bytes":bytes.len(),"installation":root,"source_sources":app.session()?.house.sources,
        "native_owned":true,"active_size":48,"routing":"Strict","initial":full(&app)?,
        "scope":"private native-owned adapter output; internal HUD invariants, no new original parity credit"}),
    )?;
    Ok(app)
}
fn ambient(m: &Message) -> bool {
    matches!(
        m,
        Message::ViewportEnter | Message::ViewportLeave | Message::ViewportMotion { .. }
    )
}
fn ignored_release(m: &Message) -> bool {
    matches!(m, Message::ViewportButton { down: false, .. })
}
fn label(m: &Message) -> String {
    format!("{m:?}")
}

struct Run {
    app: Application,
    reference: Application,
    ui: Input,
    out: PathBuf,
    steps: Vec<Step>,
    rows: Vec<Value>,
    pending: Vec<Message>,
    intended: Vec<Message>,
    initial: Observation,
}
impl Run {
    fn journal(&self) -> Result<()> {
        write_json(
            &self.out.join("sequence.json"),
            &json!({"schema":1,"initial":self.initial,"steps":self.steps}),
        )?;
        write_json(&self.out.join("journal.json"), &json!({"rows":self.rows}))
    }
    fn step(&mut self, step: Step) -> Result<()> {
        self.steps.push(step.clone());
        self.journal()?;
        let before = full(&self.app)?;
        let events: Vec<_> = step.events.iter().map(Event::iced).collect();
        let (batch, costs) = self.ui.profiled_batch(&self.app.viewer, &events);
        let unchanged = full(&self.app)? == before;
        let mut expected = match step.intent {
            Intent::None => vec![],
            Intent::Control(c) => vec![c.message()],
            Intent::Object { object, pixel } => [true, false]
                .into_iter()
                .map(|down| Message::ViewportButton {
                    down,
                    pixel: pixel.map(f32::from_bits),
                    picked: Some(object),
                })
                .collect(),
        };
        for e in &step.events {
            match *e {
                Event::Resize(s) => {
                    expected.push(Message::DisplaySize(Size::new(s[0] as f32, s[1] as f32)))
                }
                Event::Focus(false) => expected.push(Message::FocusLost),
                _ => {}
            }
        }
        let actual: Vec<_> = batch
            .messages
            .iter()
            .filter(|m| {
                !ambient(m) && (!ignored_release(m) || matches!(step.intent, Intent::Object { .. }))
            })
            .map(label)
            .collect();
        let wanted: Vec<_> = expected.iter().map(label).collect();
        self.rows.push(json!({"batch":self.steps.len()-1,"step":step,"before":before,
            "messages":batch.messages.iter().map(label).collect::<Vec<_>>(),"expected":wanted,
            "statuses":batch.statuses.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>(),
            "widget_owners":batch.owners,"widget_costs":costs,"widget_update_state_neutral":unchanged}));
        self.journal()?;
        ensure!(unchanged, "widget processing changed application state");
        ensure!(
            actual == wanted,
            "unexpected activation {actual:?}, expected {wanted:?}"
        );
        // Preserve real native Enter/Leave/Motion ordering in the ordinary
        // reference. These deliberate cursor inputs can change move-tool state;
        // they are not labeled passive resize/draw. An unsolicited lot release
        // is deliberately absent from the reference: equality must prove its
        // actual handler was completely neutral before it can be tolerated.
        let mut next = expected.into_iter();
        for m in &batch.messages {
            if ambient(m) {
                self.intended.push(m.clone());
            } else if ignored_release(m) && !matches!(step.intent, Intent::Object { .. }) {
            } else {
                self.intended
                    .push(next.next().context("expected message absent")?);
            }
        }
        ensure!(next.next().is_none());
        self.pending.extend(batch.messages);
        if step.flush {
            self.flush()?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.intended.is_empty() {
            return Ok(());
        }
        engine_marker("actual-begin", self.steps.len());
        let started = Instant::now();
        for m in self.pending.drain(..) {
            self.app.dispatch(m)?;
        }
        let dispatch_us = started.elapsed().as_micros();
        let started = Instant::now();
        self.app.drain(&Limits::default())?;
        let drain_us = started.elapsed().as_micros();
        engine_marker("reference-begin", self.steps.len());
        for m in self.intended.drain(..) {
            self.reference.dispatch(m)?;
        }
        self.reference.drain(&Limits::default())?;
        engine_marker("reference-end", self.steps.len());
        let actual = full(&self.app)?;
        let expected = full(&self.reference)?;
        let viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        let reference_viewport = self.ui.viewport_bounds(&self.reference.viewer)?;
        self.rows.push(json!({"flush_after_batches":self.steps.len(),"app_dispatch_us":dispatch_us,
            "app_drain_refresh_us":drain_us,"actual":actual,"reference":expected,
            "viewport":viewport,"reference_viewport":reference_viewport,
            "size":self.app.viewer.display_size,"mode":mode(&self.app)?,"resident":self.app.viewer.resident,
            "native_selected_person":self.app.session()?.simulation.native_selected_person().map_err(anyhow::Error::msg)?}));
        self.journal()?;
        ensure!(
            actual == expected,
            "complete ordinary-handler reference diverged"
        );
        ensure!(
            viewport == reference_viewport,
            "viewport differs from intended final layout"
        );
        Ok(())
    }
    fn events(&mut self, name: &str, events: Vec<Event>, flush: bool) -> Result<()> {
        self.step(Step {
            label: name.into(),
            events,
            intent: Intent::None,
            flush,
        })
    }
    fn hud(&mut self) -> Result<()> {
        let s = self.app.viewer.display_size;
        self.events(
            "real cursor returns to current HUD",
            vec![Event::moved([s[0] as f32 * 0.5, s[1] as f32 - 2.])],
            true,
        )
    }
    fn point(&mut self, control: Control) -> Result<[f32; 2]> {
        self.hud()?;
        let before = full(&self.app)?;
        let cursor = self.ui.cursor_event();
        let cursor_debug = format!("{cursor:?}");
        let owners = self.ui.application_batch(&self.app.viewer, &[]).owners;
        let mut found = None;
        let mut probes = vec![];
        for point in self.ui.batch_centers(&self.app.viewer) {
            let b = self.ui.application_batch(
                &self.app.viewer,
                &[Event::moved(point).iced(), pointer(true), pointer(false)],
            );
            let names: Vec<_> = b
                .messages
                .iter()
                .filter(|m| !ambient(m) && !ignored_release(m))
                .map(label)
                .collect();
            probes.push(json!({"events":[Event::moved(point),Event::Down,Event::Up],
                "messages":b.messages.iter().map(label).collect::<Vec<_>>(),
                "statuses":b.statuses.iter().map(|s|format!("{s:?}")).collect::<Vec<_>>() }));
            if names == [label(&control.message())] {
                found = Some(point);
                break;
            }
        }
        let restored = self.ui.application_batch(&self.app.viewer, &[cursor]);
        self.rows.push(json!({"discovery":control,"probes":probes,
            "restore_cursor_event":cursor_debug,"restore_messages":restored.messages.iter().map(label).collect::<Vec<_>>(),
            "widget_owners_before":owners,"widget_owners_after":restored.owners,"before":before,
            "application_messages_applied":false}));
        self.journal()?;
        ensure!(
            restored.messages.iter().all(ambient)
                && restored.owners == owners
                && full(&self.app)? == before,
            "geometry discovery changed owners or app state"
        );
        found.with_context(|| format!("actual runtime control absent: {control:?}"))
    }
    fn click(&mut self, control: Control) -> Result<()> {
        let point = self.point(control)?;
        self.step(Step {
            label: format!("intentional fresh {control:?}"),
            events: vec![Event::moved(point), Event::Down, Event::Up],
            intent: Intent::Control(control),
            flush: true,
        })
    }
    fn interrupted(&mut self, control: Control, final_size: [u32; 2]) -> Result<()> {
        let point = self.point(control)?;
        self.events(
            &format!("held {control:?}"),
            vec![Event::moved(point), Event::Down],
            false,
        )?;
        for (i, events) in burst(final_size).into_iter().enumerate() {
            self.events(
                &format!("continuous resize burst {i} with delayed app messages"),
                events,
                i == 1,
            )?;
        }
        ensure!(self.app.viewer.display_size == final_size);
        self.hud()?;
        Ok(())
    }
    fn frame(&mut self, name: &str) -> Result<()> {
        self.flush()?;
        let before = full(&self.app)?;
        let viewport = self.ui.viewport_bounds(&self.app.viewer)?;
        let (pixels, costs) = self.ui.profiled_pixels(&self.app.viewer)?;
        let after = full(&self.app)?;
        ensure!(
            before == after && self.ui.viewport_bounds(&self.app.viewer)? == viewport,
            "passive draw changed full Session/client/device/local or geometry"
        );
        image::save_buffer(
            self.out.join(format!("{name}.png")),
            &pixels,
            self.app.viewer.display_size[0],
            self.app.viewer.display_size[1],
            image::ColorType::Rgba8,
        )?;
        self.rows.push(
            json!({"frame":name,"costs":costs,"pixels_sha256":engine::digest(&pixels),
            "before":before,"after":after,"viewport":viewport,"state_neutral":true}),
        );
        self.journal()
    }
    fn checkpoint(&mut self, name: &str) -> Result<()> {
        self.flush()?;
        let before = full(&self.app)?;
        let bytes = self.app.checkpoint()?;
        std::fs::write(self.out.join(format!("{name}.json")), &bytes)?;
        let restored = cold(&self.app, &bytes)?;
        ensure!(
            full(&restored)? == before && full(&self.app)? == before,
            "complete warm/cold checkpoint divergence"
        );
        self.rows.push(
            json!({"checkpoint":name,"bytes":bytes.len(),"sha256":engine::digest(&bytes),
            "warm":before,"cold":full(&restored)?,"independently_scanned_index":true}),
        );
        self.journal()
    }
    fn pending_query_guards(&mut self, object: i16) -> Result<()> {
        // Exercise genuine pending input, without inventing simultaneous
        // owners. This API-guard check is separate from the actual Iced click.
        let mut query = self.app.fork()?;
        ensure!(
            query
                .viewer
                .session
                .as_mut()
                .unwrap()
                .begin_object_input(object)?
        );
        ensure!(
            query.session()?.player_input_pending()
                && query.session()?.player_input_blocks_ticks()
                && !query.session()?.running_menu()
        );
        let before = full(&query)?;
        let camera = query.session()?.retained_camera_input()?;
        let selection_error = query
            .viewer
            .session
            .as_mut()
            .unwrap()
            .advance_selection(camera, 1)
            .expect_err("independent selection accepted pending query");
        ensure!(
            full(&query)? == before,
            "public selection rejection mutated pending input"
        );
        let frame_error = query
            .viewer
            .session
            .as_mut()
            .unwrap()
            .begin_normal_view_frame()
            .expect_err("normal frame accepted pending query");
        ensure!(
            full(&query)? == before,
            "normal-frame rejection mutated query/environment/selection"
        );
        self.rows.push(
            json!({"guard":"genuine-pending-object-query", "object":object,
            "before":before,"after":full(&query)?,"selection_error":format!("{selection_error:#}"),
            "normal_frame_error":format!("{frame_error:#}"),"rejection_atomic":true}),
        );
        self.journal()
    }
    fn ready_menu_guards_and_cold(&mut self) -> Result<()> {
        ensure!(
            self.app.session()?.player_input_pending()
                && !self.app.session()?.player_input_blocks_ticks()
                && self.app.session()?.running_menu()
        );
        let before = full(&self.app)?;
        let camera = self.app.session()?.retained_camera_input()?;
        let error = self
            .app
            .viewer
            .session
            .as_mut()
            .unwrap()
            .advance_selection(camera, 1)
            .expect_err("independent selection accepted running menu");
        ensure!(
            full(&self.app)? == before,
            "public selection rejection mutated ready menu"
        );
        let bytes = self.app.checkpoint()?;
        let restored = cold(&self.app, &bytes)?;
        ensure!(
            full(&restored)? == before && full(&self.app)? == before,
            "ready menu cold restore mutated full retained owners/RNG"
        );
        self.rows.push(
            json!({"guard":"ready-running-menu-independent-selection", "before":before,
            "after":full(&self.app)?,"error":format!("{error:#}"),"rejection_atomic":true,
            "cold":full(&restored)?,"checkpoint_sha256":engine::digest(&bytes),
            "checkpoint_bytes":bytes.len(),"checkpoint_persisted":false}),
        );
        self.journal()
    }
    fn object_point(&mut self) -> Result<(i16, [f32; 2], [f32; 2])> {
        let bounds = self.ui.viewport_bounds(&self.app.viewer)?;
        let scene = self.app.viewer.scene.as_ref().context("scene absent")?;
        let eligible: std::collections::BTreeSet<_> = self
            .app
            .drawn_targets()?
            .into_iter()
            .filter(|id| {
                self.app
                    .session()
                    .ok()
                    .and_then(|s| s.simulation.vm.world.objects.get(&id.object))
                    .and_then(|o| o.definition.as_ref())
                    .is_some_and(|d| d.source.ends_with("::DiningChairs.iff"))
            })
            .map(|id| id.object)
            .collect();
        let mut found = None;
        'objects: for draw in &scene.draws {
            let Some(object) = draw.object.filter(|id| eligible.contains(id)) else {
                continue;
            };
            let vertices =
                &scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize];
            for tri in vertices.chunks_exact(3) {
                for weights in [
                    [1. / 3.; 3],
                    [0.6, 0.2, 0.2],
                    [0.2, 0.6, 0.2],
                    [0.2, 0.2, 0.6],
                ] {
                    let pixel = [0, 1].map(|axis| {
                        tri.iter()
                            .zip(weights)
                            .map(|(v, w)| {
                                if axis == 0 {
                                    (v.position[0] + 1.) * scene.width as f32 / 2. * w
                                } else {
                                    (1. - v.position[1]) * scene.height as f32 / 2. * w
                                }
                            })
                            .sum::<f32>()
                    });
                    if scene.pick(pixel) != Some(object) {
                        continue;
                    }
                    let scale =
                        (bounds[2] / scene.width as f32).min(bounds[3] / scene.height as f32);
                    let point = [
                        bounds[0]
                            + (bounds[2] - scene.width as f32 * scale) / 2.
                            + pixel[0] * scale,
                        bounds[1]
                            + (bounds[3] - scene.height as f32 * scale) / 2.
                            + pixel[1] * scale,
                    ];
                    let actual = scene_cursor(
                        scene,
                        iced::Rectangle::new(
                            iced::Point::new(bounds[0], bounds[1]),
                            Size::new(bounds[2], bounds[3]),
                        ),
                        iced::mouse::Cursor::Available(iced::Point::new(point[0], point[1])),
                    )
                    .context("projected actual object outside viewport")?;
                    if scene.pick(actual) == Some(object) {
                        found = Some((object, point, actual));
                        break 'objects;
                    }
                }
            }
        }
        found.context("no actually pickable rendered dining chair")
    }
    fn object(&mut self) -> Result<()> {
        let (object, point, pixel) = self.object_point()?;
        self.pending_query_guards(object)?;
        self.step(Step {
            label: format!("actual Iced object click {object}"),
            events: vec![Event::moved(point), Event::Down, Event::Up],
            intent: Intent::Object {
                object,
                pixel: pixel.map(f32::to_bits),
            },
            flush: true,
        })?;
        ensure!(
            self.app
                .menu()
                .is_some_and(|m| m.target() == object && !m.actions().is_empty()),
            "actual object click did not open ready native menu"
        );
        self.ready_menu_guards_and_cold()?;
        Ok(())
    }
    fn scroll_if_available(&mut self) -> Result<()> {
        let owners = self.ui.application_batch(&self.app.viewer, &[]).owners;
        let Some(s) = owners
            .scrolls
            .iter()
            .find(|s| s.content[3] > s.bounds[3] + 1.)
        else {
            self.rows
                .push(json!({"catalog_scroll":"not overflowing at current actual geometry"}));
            return self.journal();
        };
        let point = [
            s.bounds[0] + s.bounds[2] * 0.75,
            s.bounds[1] + s.bounds[3] * 0.5,
        ];
        // Pointer entry can flush the native Move tool's deferred enter call.
        // Complete that ordinary handler before asserting wheel-only neutrality.
        self.events(
            "real cursor entry into overflowing catalogue",
            vec![Event::moved(point)],
            true,
        )?;
        let before = full(&self.app)?;
        self.events(
            "real overflowing catalogue wheel only",
            vec![Event::Wheel(-3)],
            true,
        )?;
        let after = self.ui.application_batch(&self.app.viewer, &[]).owners;
        ensure!(
            owners
                .scrolls
                .iter()
                .zip(&after.scrolls)
                .any(|(a, b)| a.translation != b.translation),
            "actual wheel did not scroll catalogue"
        );
        ensure!(
            full(&self.app)? == before,
            "catalogue widget scroll changed persisted app/Session"
        );
        Ok(())
    }
}

/// Private diagnostic logs from copied renderer sources use these markers to
/// distinguish actual Iced dispatch from the ordinary-handler reference.
fn engine_marker(interval: &str, batch: usize) {
    if std::env::var_os("SIMS_PROFILE_UI_ENGINE").is_some() {
        eprintln!(
            "UI_ENGINE_MARKER {}",
            json!({"interval":interval,"batch":batch})
        );
    }
}

#[test]
#[ignore = "private verified native-owned Newbie checkpoint; continuous actual-Iced batches and headless GPU"]
fn native_owned_continuous_resize_popup_and_live_buy_cycles() -> Result<()> {
    run_worker(campaign)
}
fn campaign() -> Result<()> {
    let out = output()?;
    let app = load_native(&out)?;
    let initial = full(&app)?;
    let reference = app.fork()?;
    let mut run = Run {
        app,
        reference,
        ui: Input::new()?,
        out,
        steps: vec![],
        rows: vec![],
        pending: vec![],
        intended: vec![],
        initial,
    };
    let result = (|| -> Result<()> {
        run.hud()?;
        run.checkpoint("initial-safe")?;
        run.interrupted(Control::Options, [1000, 740])?;
        ensure!(!run.app.viewer.options_open, "stale held Options activated");
        run.click(Control::Options)?;
        ensure!(run.app.viewer.options_open);
        run.interrupted(Control::Options, [1320, 900])?;
        ensure!(
            run.app.viewer.options_open,
            "stale Options release dismissed popup"
        );
        run.frame("resized-options")?;
        run.click(Control::Options)?;
        ensure!(!run.app.viewer.options_open);
        // A completed native Buy/Live edit cycle can retain no selected person.
        // Viewer.resident is merely presentation state. Establish the actual
        // native input precondition through its published Iced control.
        let resident = run.app.viewer.resident;
        run.click(Control::Resident(resident))?;
        ensure!(
            run.app
                .session()?
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?
                == Some(resident),
            "actual resident control did not establish native selection"
        );
        run.object()?;
        let target = run.app.menu().context("native menu absent")?.target();
        let actions = run.app.menu().unwrap().actions().to_vec();
        run.interrupted(Control::ObjectCancel, [1000, 740])?;
        ensure!(
            run.app
                .menu()
                .is_some_and(|m| m.target() == target && m.actions() == actions),
            "resize altered/dismissed menu"
        );
        run.frame("resized-object-menu")?;
        run.click(Control::ObjectCancel)?;
        ensure!(run.app.menu().is_none());
        for (cycle, size) in [[800, 600], [1320, 900]].into_iter().enumerate() {
            run.click(Control::Buy)?;
            ensure!(
                mode(&run.app)? == 1,
                "Buy did not complete ordinary transition"
            );
            run.scroll_if_available()?;
            run.interrupted(Control::Live, size)?;
            ensure!(
                mode(&run.app)? == 1,
                "stale held Live activated in cycle{cycle}"
            );
            run.click(Control::Live)?;
            ensure!(
                mode(&run.app)? == 2,
                "Live did not complete ordinary transition"
            );
        }
        run.click(Control::Options)?;
        ensure!(run.app.viewer.options_open, "fresh final click lost");
        run.click(Control::Options)?;
        safe(&run.app)?;
        run.checkpoint("final-safe")?;
        run.frame("final-live")?;
        let final_state = full(&run.app)?;
        write_json(
            &run.out.join("result.json"),
            &json!({"passed":true,"batches":run.steps.len(),
            "resizes":run.steps.iter().flat_map(|s|&s.events).filter(|e|matches!(e,Event::Resize(_))).count(),
            "initial":run.initial,"final":final_state,"completed_ticks":final_state.tick-run.initial.tick,
            "mode":mode(&run.app)?,"display_size":run.app.viewer.display_size,"cold_equal":true,
            "scope":"native-owned replacement HUD internal reference; no original widget/pixel/tick-parity claim",
            "timings":"diagnostic wall time during parallel work; hash/save/PNG excluded; GPU+readback inseparable"}),
        )?;
        Ok(())
    })();
    if let Err(error) = &result {
        run.journal()?;
        write_json(
            &run.out.join("first-failure.json"),
            &json!({"error":format!("{error:#}"),
            "actual":full(&run.app)?,"reference":full(&run.reference)?,"completed_batches":run.steps.len()}),
        )?;
        std::fs::write(run.out.join("failure.json"), run.app.checkpoint()?)?;
        std::fs::write(
            run.out.join("failure-reference.json"),
            run.reference.checkpoint()?,
        )?;
    }
    result
}
